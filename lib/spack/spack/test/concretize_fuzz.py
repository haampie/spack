# Copyright Spack Project Developers. See COPYRIGHT file for details.
#
# SPDX-License-Identifier: (Apache-2.0 OR MIT)

"""Randomized end-to-end check of the concretizer against its own inputs.

The spec-algebra property tests sample the *algebra*; this script samples the *solver*. Each
case draws a root package from the configured (real) repo, decorates it with constraints taken
from the package's own metadata (declared versions, variant definitions, dependency names),
optionally injects random hard requirements into the ``packages:`` config section, and
concretizes. The output DAG is then checked against every input the solver saw:

- the input literal (root satisfies the abstract spec),
- the injected ``packages:<name>:require`` constraints,
- every ``depends_on`` whose condition holds on a node,
- every ``conflicts`` and ``requires`` directive,
- variant definitions (presence, value validity, no undefined variants),
- declared versions (a node's version must be one the package lists),
- provider legitimacy (a node reached through a virtual must provide it).

Run standalone, never under pytest::

    python lib/spack/spack/test/concretize_fuzz.py --seeds 0:100 --jsonl /tmp/fuzz.jsonl

A case is reproduced by its seed; the JSONL log records the generated spec and config so a
finding can be replayed with ``spack solve --fresh`` plus the printed requirements.
"""

import argparse
import json
import multiprocessing
import random
import sys
import time
import traceback
from typing import Any, Dict, Iterable, List, Optional, Tuple

import spack.concretize
import spack.config
import spack.deptypes as dt
import spack.error
import spack.repo
import spack.spec
import spack.spec_parser
from spack.version import GitVersion, StandardVersion

#: Variants whose values the solver invents rather than reads from a definition.
SPECIAL_VARIANTS = {"patches", "dev_path", "commit"}

#: Packages that show up in most DAGs; useful targets for config requirements even when they
#: are not direct dependencies of the sampled root.
COMMON_PACKAGES = ["gmake", "cmake", "pkgconf", "python", "zlib-ng", "openssl", "perl", "ninja"]


def load_pkg_class(name: str):
    try:
        return spack.repo.PATH.get_pkg_class(name)
    except Exception:
        return None


def sampleable_versions(pkg) -> List[StandardVersion]:
    """Declared, finite, non-git versions of a package, ascending."""
    result = [
        v
        for v in pkg.versions
        if isinstance(v, StandardVersion) and not v.isdevelop() and v != StandardVersion.typemax()
    ]
    return sorted(result)


def draw_version_constraint(rng: random.Random, pkg) -> Optional[str]:
    versions = sampleable_versions(pkg)
    if not versions:
        return None
    a, b = sorted((rng.choice(versions), rng.choice(versions)))
    return rng.choice([f"@={a}", f"@{a}", f"@{a}:", f"@:{b}", f"@{a}:{b}" if a != b else f"@{a}"])


def draw_variant_constraint(rng: random.Random, pkg, propagate_ok: bool = False) -> Optional[str]:
    names = [n for n in pkg.variant_names() if n not in SPECIAL_VARIANTS and n != "build_system"]
    if not names:
        return None
    name = rng.choice(names)
    definitions = pkg.variant_definitions(name)
    if not definitions:
        return None
    _, vdef = rng.choice(definitions)
    propagate = propagate_ok and rng.random() < 0.15
    if vdef.values == (True, False):
        sigil = rng.choice(["++", "~~"]) if propagate else rng.choice(["+", "~"])
        return f"{sigil}{name}"
    values = [v for v in (vdef.values or ()) if v not in (True, False)]
    if not values:
        return None
    op = "==" if propagate else "="
    if vdef.multi and len(values) > 1 and rng.random() < 0.3:
        picked = rng.sample(values, 2)
        return f"{name}{op}{picked[0]},{picked[1]}"
    return f"{name}{op}{rng.choice(values)}"


FLAG_VALUES = ["-O2", "-g", "-fPIC", "-DNDEBUG"]


def draw_flag_constraint(rng: random.Random) -> str:
    flag = rng.choice(["cflags", "cxxflags", "cppflags", "ldflags"])
    return f"{flag}={rng.choice(FLAG_VALUES)}"


def draw_virtual_constraint(rng: random.Random, virtual: str) -> str:
    """A version constraint for a virtual, sampled from what its providers declare."""
    versions = set()
    try:
        providers = spack.repo.PATH.providers_for(virtual)
    except Exception:
        providers = []
    for p in providers:
        for v in p.versions:
            bounds = [v] if isinstance(v, StandardVersion) else [v.lo, v.hi]
            for bound in bounds:
                if isinstance(bound, StandardVersion) and not bound.isdevelop():
                    versions.add(bound)
    versions = sorted(versions)
    if not versions or rng.random() < 0.4:
        return ""
    v = rng.choice(versions)
    return rng.choice([f"@{v}:", f"@:{v}", f"@{v}"])


def dependency_names(pkg) -> List[str]:
    names = set()
    for _, deps in pkg.dependencies.items():
        names.update(deps)
    return sorted(names)


def draw_node_constraints(rng: random.Random, pkg, budget: int) -> str:
    """A constraint string (version and/or variants) for one named node."""
    parts = []
    if rng.random() < 0.7:
        v = draw_version_constraint(rng, pkg)
        if v:
            parts.append(v)
    used_variants = set()
    for _ in range(budget):
        if rng.random() < 0.6:
            c = draw_variant_constraint(rng, pkg)
            if c:
                name = c.lstrip("+~").split("=")[0]
                if name not in used_variants:
                    used_variants.add(name)
                    parts.append(c)
    return "".join(f" {p}" for p in parts)


def draw_root(rng: random.Random, pool: List[str]) -> Optional[Tuple[str, List[str]]]:
    """Draw (root_spec_string, dependency_names) or None if the drawn package is unusable."""
    root_name = rng.choice(pool)
    pkg = load_pkg_class(root_name)
    if pkg is None or getattr(pkg, "virtual", False):
        return None

    spec_str = root_name + draw_node_constraints(rng, pkg, budget=2)
    if rng.random() < 0.05:
        spec_str += f" {draw_flag_constraint(rng)}"
    if rng.random() < 0.1:
        spec_str += f" target={rng.choice(['aarch64', 'm1', 'm2', 'm3', 'm4'])}"

    dep_names = dependency_names(pkg)
    rng.shuffle(dep_names)
    for name in dep_names[: rng.randint(0, 2)]:
        if spack.repo.PATH.is_virtual(name):
            if rng.random() < 0.25:
                providers = [p.name for p in spack.repo.PATH.providers_for(name)]
                if providers:
                    spec_str += f" ^[virtuals={name}] {rng.choice(providers)}"
                    continue
            spec_str += f" ^{name}{draw_virtual_constraint(rng, name)}"
            continue
        dep_pkg = load_pkg_class(name)
        if dep_pkg is None:
            continue
        sigil = "%" if rng.random() < 0.2 else "^"
        spec_str += f" {sigil}{name}{draw_node_constraints(rng, dep_pkg, budget=1)}"

    return spec_str, dep_names


def draw_requirement(rng: random.Random, target_pkg) -> Optional[Any]:
    """One entry for a ``packages:<name>:require`` list: a plain string or a dict form."""
    constraint = draw_node_constraints(rng, target_pkg, budget=1).strip()
    if not constraint:
        return None
    form = rng.random()
    if form < 0.55:
        return constraint
    other = draw_node_constraints(rng, target_pkg, budget=1).strip()
    if form < 0.85:
        if not other or other == constraint:
            return constraint
        policy = "any_of" if form < 0.7 else "one_of"
        return {policy: [constraint, other]}
    when = draw_version_constraint(rng, target_pkg)
    if not when:
        return constraint
    return {"spec": constraint, "when": when}


def draw_case(rng: random.Random, pool: List[str]) -> Optional[Dict[str, Any]]:
    """Draw a case dict or None if the drawn package is unusable."""
    drawn = draw_root(rng, pool)
    if drawn is None:
        return None
    spec_str, dep_names = drawn
    roots = [spec_str]
    mode = "one"
    force_mode = globals().get("FORCE_MODE")
    if force_mode or rng.random() < 0.25:
        for _ in range(1 if rng.random() < 0.7 else 2):
            extra = draw_root(rng, pool)
            if extra is not None:
                roots.append(extra[0])
        if len(roots) > 1:
            mode = force_mode or ("when_possible" if rng.random() < 0.3 else "together")

    concretizer_extra: Dict[str, Any] = {}
    if rng.random() < 0.25:
        concretizer_extra["duplicates"] = {"strategy": rng.choice(["none", "minimal"])}
    if rng.random() < 0.15:
        concretizer_extra["targets"] = {"granularity": "generic"}

    reuse_cfg: Any = False
    if rng.random() < 0.3:
        form = rng.random()
        if form < 0.4:
            reuse_cfg = True
        elif form < 0.6:
            reuse_cfg = "dependencies"
        else:
            candidates = [r.split()[0] for r in roots] + COMMON_PACKAGES
            constraint = rng.choice(candidates)
            reuse_cfg = {rng.choice(["include", "exclude"]): [constraint]}

    tests = mode == "one" and rng.random() < 0.1

    toolchains_cfg: Dict[str, str] = {}
    toolchain_used = False
    if rng.random() < 0.15:
        parts = ["%c=apple-clang", "%cxx=apple-clang"]
        if rng.random() < 0.3:
            parts.append("%fortran=gcc")
        toolchains_cfg["fuzztc"] = " ".join(rng.sample(parts, rng.randint(1, len(parts))))
        roots[0] += " %fuzztc"
        toolchain_used = True

    packages_cfg: Dict[str, Any] = {}
    if rng.random() < 0.6:
        root_names = [r.split()[0] for r in roots]
        candidates = root_names + dep_names + COMMON_PACKAGES
        for target in rng.sample(candidates, min(len(candidates), rng.randint(1, 2))):
            if spack.repo.PATH.is_virtual(target):
                providers = [p.name for p in spack.repo.PATH.providers_for(target)]
                if not providers:
                    continue
                provider = rng.choice(providers)
                requirement = provider
                provider_pkg = load_pkg_class(provider)
                if provider_pkg is not None and rng.random() < 0.5:
                    v = draw_version_constraint(rng, provider_pkg)
                    if v:
                        requirement += v
                packages_cfg[target] = {"require": [requirement]}
            else:
                target_pkg = load_pkg_class(target)
                if target_pkg is None:
                    continue
                drawn_requirement = draw_requirement(rng, target_pkg)
                if drawn_requirement is not None:
                    key = "conflict" if rng.random() < 0.25 else "require"
                    if key == "conflict" and isinstance(drawn_requirement, dict):
                        drawn_requirement = drawn_requirement.get("spec") or next(
                            iter(
                                drawn_requirement.get("any_of", [])
                                + drawn_requirement.get("one_of", [])
                            ),
                            None,
                        )
                        if drawn_requirement is None:
                            continue
                        if rng.random() < 0.4:
                            when = draw_version_constraint(rng, target_pkg)
                            if when:
                                drawn_requirement = {"spec": drawn_requirement, "when": when}
                    packages_cfg[target] = {key: [drawn_requirement]}

    if rng.random() < 0.25:
        candidates = [n for n in dep_names + COMMON_PACKAGES if not spack.repo.PATH.is_virtual(n)]
        target = rng.choice(candidates) if candidates else ""
        target_pkg = load_pkg_class(target) if target else None
        if target_pkg is not None and target not in packages_cfg:
            versions = sampleable_versions(target_pkg)
            if versions:
                ext_spec = f"{target}@={rng.choice(versions)}"
                variant = draw_variant_constraint(rng, target_pkg, propagate_ok=False)
                if variant and rng.random() < 0.5:
                    ext_spec += f" {variant}"
                externals = [{"spec": ext_spec, "prefix": "/usr"}]
                if len(versions) > 1 and rng.random() < 0.4:
                    other = rng.choice([v for v in versions if f"@={v}" not in ext_spec])
                    externals.append({"spec": f"{target}@={other}", "prefix": "/opt"})
                entry: Dict[str, Any] = {"externals": externals}
                if rng.random() < 0.5:
                    entry["buildable"] = False
                packages_cfg[target] = entry

    return {
        "roots": roots,
        "packages": packages_cfg,
        "concretizer": concretizer_extra,
        "mode": mode,
        "reuse": reuse_cfg,
        "tests": tests,
        "toolchains": toolchains_cfg,
        "toolchain_used": toolchain_used,
    }


class Finding:
    def __init__(self, kind: str, node: str, detail: str):
        self.kind = kind
        self.node = node
        self.detail = detail

    def to_dict(self) -> Dict[str, str]:
        return {"kind": self.kind, "node": self.node, "detail": self.detail}


def edges_of(node: spack.spec.Spec):
    return node.edges_to_dependencies()


def check_node_versions(node, pkg, findings: List[Finding]) -> None:
    if isinstance(node.version, GitVersion) or not pkg.versions:
        return
    if node.version not in pkg.versions:
        findings.append(
            Finding("undeclared-version", str(node), f"version {node.version} not declared")
        )


def check_deprecated_version(node, pkg, findings: List[Finding]) -> None:
    if isinstance(node.version, GitVersion) or node.version not in pkg.versions:
        return
    if pkg.versions[node.version].get("deprecated", False):
        findings.append(
            Finding("deprecated-version", str(node), f"version {node.version} is deprecated")
        )


def check_node_variants(node, pkg, findings: List[Finding]) -> None:
    for name in node.variants:
        if name in SPECIAL_VARIANTS:
            continue
        definitions = pkg.variant_definitions(name)
        active = [(w, d) for w, d in definitions if node.satisfies(w)]
        if not definitions:
            findings.append(Finding("undefined-variant", str(node), f"variant {name} undefined"))
            continue
        if not active:
            findings.append(
                Finding(
                    "inactive-variant",
                    str(node),
                    f"variant {name} | set, but no definition condition holds",
                )
            )
            continue
        _, vdef = active[-1]
        try:
            vdef.validate_or_raise(node.variants[name], pkg.name)
        except spack.error.SpackError as e:
            findings.append(
                Finding("invalid-variant-value", str(node), f"variant {name} | {str(e).strip()}")
            )
    for name in pkg.variant_names():
        if name in SPECIAL_VARIANTS or name in node.variants:
            continue
        if any(node.satisfies(w) for w, _ in pkg.variant_definitions(name)):
            findings.append(
                Finding("unset-variant", str(node), f"variant {name} | active but unset")
            )


def check_node_dependencies(node, pkg, findings: List[Finding], tests: bool = False) -> None:
    edges = edges_of(node)
    mask = ~0 if tests else ~dt.TEST
    for when, deps in pkg.dependencies.items():
        if not node.satisfies(when):
            continue
        for dep_name, dep in deps.items():
            if not (dep.depflag & mask):
                continue
            candidates = [
                e.spec for e in edges if e.spec.name == dep_name or dep_name in e.virtuals
            ]
            if not candidates:
                findings.append(
                    Finding(
                        "missing-dependency",
                        str(node),
                        f"depends_on({str(dep.spec)!r}, when={str(when)!r}) | no edge",
                    )
                )
            elif not any(c.satisfies(dep.spec) for c in candidates):
                got = ", ".join(c.format("{name}{@version}{variants}") for c in candidates)
                findings.append(
                    Finding(
                        "unsatisfied-dependency",
                        str(node),
                        f"depends_on({str(dep.spec)!r}, when={str(when)!r}) | got {got}",
                    )
                )
            else:
                got_flag = 0
                for e in edges:
                    if e.spec.name == dep_name or dep_name in e.virtuals:
                        got_flag |= e.depflag
                missing = dep.depflag & mask & ~got_flag
                if missing:
                    findings.append(
                        Finding(
                            "missing-deptype",
                            str(node),
                            f"depends_on({str(dep.spec)!r}, when={str(when)!r}) | "
                            f"edge lacks deptype {dt.flag_to_chars(missing)}",
                        )
                    )


def check_node_conflicts(node, pkg, findings: List[Finding]) -> None:
    for when, conflict_list in pkg.conflicts.items():
        if not node.satisfies(when):
            continue
        for conflict_spec, _ in conflict_list:
            if node.satisfies(conflict_spec):
                findings.append(
                    Finding(
                        "conflict-violated",
                        str(node),
                        f"conflicts({str(conflict_spec)!r}, when={str(when)!r})",
                    )
                )


def check_node_requirements(node, pkg, findings: List[Finding]) -> None:
    for when, requirement_list in pkg.requirements.items():
        if not node.satisfies(when):
            continue
        for req_specs, policy, _ in requirement_list:
            n = sum(1 for s in req_specs if node.satisfies(s))
            ok = n == 1 if policy == "one_of" else n >= 1
            if not ok:
                findings.append(
                    Finding(
                        "requires-violated",
                        str(node),
                        f"requires({', '.join(str(s) for s in req_specs)}, "
                        f"policy={policy}, when={str(when)!r}) | {n} satisfied",
                    )
                )


def check_edge_providers(node, findings: List[Finding]) -> None:
    for edge in edges_of(node):
        if not edge.virtuals or edge.spec.external:
            continue
        child = edge.spec
        child_pkg = load_pkg_class(child.name)
        if child_pkg is None:
            continue
        for virtual in edge.virtuals:
            ok = any(
                child.satisfies(when) and any(p.name == virtual for p in provided)
                for when, provided in child_pkg.provided.items()
            )
            if not ok:
                findings.append(
                    Finding(
                        "illegitimate-provider",
                        str(child),
                        f"virtual {virtual} | no active provides() on provider",
                    )
                )


def requirement_holds(node: spack.spec.Spec, requirement: Any) -> bool:
    if isinstance(requirement, str):
        return node.satisfies(requirement)
    if "when" in requirement and "spec" in requirement:
        return not node.satisfies(requirement["when"]) or node.satisfies(requirement["spec"])
    if "any_of" in requirement:
        return any(node.satisfies(s) for s in requirement["any_of"])
    if "one_of" in requirement:
        return sum(1 for s in requirement["one_of"] if node.satisfies(s)) == 1
    return True


LANG_OF_FLAG = {"cflags": "c", "cxxflags": "cxx", "fflags": "fortran"}


def compiler_for(node: spack.spec.Spec, lang: str) -> Optional[Tuple[str, str]]:
    for e in node.edges_to_dependencies():
        if lang in e.virtuals:
            return (e.spec.name, str(e.spec.version))
    return None


def check_flag_propagation(
    root_str: str, concrete: spack.spec.Spec, findings: List[Finding]
) -> None:
    """Propagated input flags (``cflags=='-O2'``) must reach link/run descendants that use
    the same compiler for the flag's language."""
    input_spec = spack.spec.Spec(root_str)
    propagated = [
        (flag_type, flag)
        for flag_type, flag_list in input_spec.compiler_flags.items()
        for flag in flag_list
        if flag.propagate
    ]
    if not propagated:
        return
    for flag_type, flag in propagated:
        lang = LANG_OF_FLAG.get(flag_type, "c")
        source_compiler = compiler_for(concrete, lang)
        if source_compiler is None:
            continue
        for node in concrete.traverse(deptype=("link", "run")):
            if node is concrete or node.external:
                continue
            if compiler_for(node, lang) != source_compiler:
                continue
            if flag not in node.compiler_flags.get(flag_type, []):
                findings.append(
                    Finding(
                        "flag-not-propagated",
                        str(node),
                        f"{flag_type}=='{flag}' from root {concrete.name} | flag missing",
                    )
                )


def variants_mentioned_in_inputs(
    roots: List[str], packages_cfg: Dict[str, Any]
) -> Tuple[set, set]:
    """(pkg, variant) pairs and bare variant names the inputs constrain explicitly."""
    pairs = set()
    bare = set()

    def note(spec: spack.spec.Spec) -> None:
        for node in spec.traverse():
            for vname, value in node.variants.items():
                if node.name:
                    pairs.add((node.name, vname))
                else:
                    bare.add(vname)
                if value.propagate:
                    bare.add(vname)

    for root_str in roots:
        note(spack.spec.Spec(root_str))
    for target, cfg in packages_cfg.items():
        for requirement in cfg.get("require", []):
            members = (
                [requirement]
                if isinstance(requirement, str)
                else requirement.get("any_of", [])
                + requirement.get("one_of", [])
                + ([requirement["spec"]] if "spec" in requirement else [])
            )
            for member in members:
                try:
                    spec = spack.spec.Spec(member)
                except Exception:
                    continue
                for node in spec.traverse():
                    for vname, value in node.variants.items():
                        pairs.add((node.name or target, vname))
                        if value.propagate:
                            bare.add(vname)
    return pairs, bare


def check_sticky_variants(
    node: spack.spec.Spec, pkg, findings: List[Finding], mentioned: Tuple[set, set]
) -> None:
    """A sticky variant must keep its default unless the inputs set it explicitly."""
    pairs, bare = mentioned
    for name in node.variants:
        if name in SPECIAL_VARIANTS or (node.name, name) in pairs or name in bare:
            continue
        active = [
            vdef
            for when, vdef in pkg.variant_definitions(name)
            if vdef.sticky and node.satisfies(when)
        ]
        if not active:
            continue
        vdef = active[-1]
        if isinstance(vdef.default, bool):
            expected = f"+{name}" if vdef.default else f"~{name}"
        else:
            expected = f"{name}={vdef.default}"
        if not node.satisfies(expected):
            findings.append(
                Finding(
                    "sticky-variant-changed",
                    str(node),
                    f"variant {name} | sticky default {expected!r} not kept",
                )
            )


def check_target_granularity(node: spack.spec.Spec, findings: List[Finding]) -> None:
    target = node.architecture.target if node.architecture else None
    if target is None:
        return
    micro = getattr(target, "microarchitecture", target)
    if micro.vendor != "generic":
        findings.append(
            Finding(
                "non-generic-target",
                str(node),
                f"target {target} | granularity=generic requires generic targets",
            )
        )


def check_config_requirements(
    roots: List[spack.spec.Spec], packages_cfg: Dict[str, Any], findings: List[Finding]
) -> None:
    providers: Dict[str, List[spack.spec.Spec]] = {}
    nodes_by_name: Dict[str, List[spack.spec.Spec]] = {}
    for root in roots:
        for edge in root.traverse_edges(cover="edges"):
            for virtual in edge.virtuals:
                providers.setdefault(virtual, []).append(edge.spec)
        for node in root.traverse():
            nodes_by_name.setdefault(node.name, []).append(node)

    for target, cfg in packages_cfg.items():
        if spack.repo.PATH.is_virtual(target):
            nodes = providers.get(target, [])
        else:
            nodes = nodes_by_name.get(target, [])
        for node in nodes:
            if node.external:
                continue
            for requirement in cfg.get("require", []):
                if not requirement_holds(node, requirement):
                    findings.append(
                        Finding(
                            "config-require-violated",
                            str(node),
                            f"packages:{target}:require:{requirement!r}",
                        )
                    )


def check_externals(
    concretes: List[spack.spec.Spec], packages_cfg: Dict[str, Any], findings: List[Finding]
) -> None:
    """External nodes must match a configured external spec; buildable:false forbids building."""
    for target, cfg in packages_cfg.items():
        externals = cfg.get("externals")
        if not externals:
            continue
        merged = spack.config.CONFIG.get(f"packages:{target}:externals", externals)
        ext_specs = [spack.spec.Spec(e["spec"]) for e in merged]
        buildable = cfg.get("buildable", True)
        seen = set()
        for concrete in concretes:
            for node in concrete.traverse():
                if node.name != target or id(node) in seen:
                    continue
                seen.add(id(node))
                if node.external:
                    if not any(node.satisfies(es) for es in ext_specs):
                        findings.append(
                            Finding(
                                "external-spec-violated",
                                str(node),
                                f"external node does not match {[str(s) for s in ext_specs]}",
                            )
                        )
                elif not buildable:
                    findings.append(
                        Finding(
                            "nonbuildable-built",
                            str(node),
                            f"packages:{target}:buildable:false | node was built",
                        )
                    )


def check_patches(node, pkg, findings: List[Finding]) -> None:
    """Every patch() directive whose condition holds must appear in the patches variant."""
    got: Tuple[str, ...] = ()
    if "patches" in node.variants:
        got = tuple(str(v) for v in node.variants["patches"].values)
    for when, patch_list in pkg.patches.items():
        if not node.satisfies(when):
            continue
        for patch in patch_list:
            try:
                sha = patch.sha256
            except Exception:
                continue
            if sha is None:
                continue
            if not any(g == sha or sha.startswith(g) for g in got):
                findings.append(
                    Finding(
                        "missing-patch",
                        str(node),
                        f"patch {sha[:10]} when={str(when)!r} | not in patches variant",
                    )
                )


def check_cycles(concrete: spack.spec.Spec, findings: List[Finding]) -> None:
    state: Dict[int, int] = {}
    stack = [(concrete, iter(concrete.edges_to_dependencies()))]
    state[id(concrete)] = 1
    while stack:
        node, it = stack[-1]
        edge = next(it, None)
        if edge is None:
            state[id(node)] = 2
            stack.pop()
            continue
        child = edge.spec
        if state.get(id(child), 0) == 1:
            findings.append(
                Finding("dependency-cycle", str(child), f"cycle via {node.name} -> {child.name}")
            )
            return
        if id(child) not in state:
            state[id(child)] = 1
            stack.append((child, iter(child.edges_to_dependencies())))


def check_provided_together(node, findings: List[Finding]) -> None:
    """A parent taking one virtual of a provided-together group from a provider must not take
    another virtual of that group from a different node."""
    provider_of: Dict[str, spack.spec.Spec] = {}
    for e in node.edges_to_dependencies():
        for v in e.virtuals:
            provider_of[v] = e.spec
    for virtual, provider in provider_of.items():
        provider_pkg = load_pkg_class(provider.name)
        if provider_pkg is None:
            continue
        for when, groups in provider_pkg.provided_together.items():
            if not provider.satisfies(when):
                continue
            for group in groups:
                if virtual not in group:
                    continue
                for other in group:
                    other_provider = provider_of.get(other)
                    if other_provider is not None and other_provider is not provider:
                        findings.append(
                            Finding(
                                "provided-together-split",
                                str(node),
                                f"{virtual} from {provider.name}, {other} from "
                                f"{other_provider.name} | declared provided together",
                            )
                        )


def check_config_conflicts(
    concretes: List[spack.spec.Spec], packages_cfg: Dict[str, Any], findings: List[Finding]
) -> None:
    for target, cfg in packages_cfg.items():
        for entry in cfg.get("conflict", []):
            spec_str = entry if isinstance(entry, str) else entry["spec"]
            when = None if isinstance(entry, str) else entry.get("when")
            for concrete in concretes:
                for node in concrete.traverse():
                    if node.name != target or node.external:
                        continue
                    if when is not None and not node.satisfies(when):
                        continue
                    if node.satisfies(spec_str):
                        findings.append(
                            Finding(
                                "config-conflict-violated",
                                str(node),
                                f"packages:{target}:conflict:{entry!r}",
                            )
                        )


def check_dict_round_trip(concrete: spack.spec.Spec, findings: List[Finding]) -> None:
    try:
        clone = spack.spec.Spec.from_dict(concrete.to_dict())
    except Exception as e:
        findings.append(
            Finding("roundtrip-dict-raises", str(concrete), f"{type(e).__name__}: {e}")
        )
        return
    if clone.dag_hash() != concrete.dag_hash():
        findings.append(
            Finding(
                "roundtrip-dict-hash",
                str(concrete),
                f"dag_hash {concrete.dag_hash()[:10]} -> {clone.dag_hash()[:10]}",
            )
        )


def check_toolchain(
    concrete: spack.spec.Spec, toolchains_cfg: Dict[str, str], findings: List[Finding]
) -> None:
    for name, tc_spec in toolchains_cfg.items():
        if not concrete.satisfies(tc_spec):
            findings.append(
                Finding("toolchain-violated", str(concrete), f"%{name} = {tc_spec!r} not met")
            )


def check_unification(concretes: List[spack.spec.Spec], findings: List[Finding]) -> None:
    """Under unify:true, the link/run closure of all roots shares one node per package."""
    by_name: Dict[str, set] = {}
    for concrete in concretes:
        for node in concrete.traverse(deptype=("link", "run")):
            by_name.setdefault(node.name, set()).add(node.dag_hash())
    for name, hashes in by_name.items():
        if len(hashes) > 1:
            findings.append(
                Finding(
                    "unify-violated",
                    name,
                    f"{len(hashes)} distinct {name} nodes in link/run closure: "
                    f"{sorted(h[:8] for h in hashes)}",
                )
            )


def verify(case: Dict[str, Any], concretes: List[spack.spec.Spec]) -> List[Finding]:
    findings: List[Finding] = []
    roots, packages_cfg = case["roots"], case["packages"]
    generic_targets = case["concretizer"].get("targets", {}).get("granularity") == "generic"
    reuse_active = bool(case.get("reuse"))
    tests = case.get("tests", False)

    for spec_str, concrete in zip(roots, concretes):
        literal = spec_str.replace(" %fuzztc", "") if case.get("toolchain_used") else spec_str
        if not concrete.satisfies(spack.spec.Spec(literal)):
            findings.append(Finding("input-literal-violated", str(concrete), literal))

    if case.get("toolchain_used") and concretes:
        check_toolchain(concretes[0], case.get("toolchains", {}), findings)

    check_config_requirements(concretes, packages_cfg, findings)
    check_externals(concretes, packages_cfg, findings)
    check_config_conflicts(concretes, packages_cfg, findings)
    if case["mode"] == "together" and len(concretes) > 1:
        check_unification(concretes, findings)
    mentioned = variants_mentioned_in_inputs(roots, packages_cfg)

    for concrete in concretes:
        check_cycles(concrete, findings)
        check_dict_round_trip(concrete, findings)

    seen_nodes = set()
    for concrete in concretes:
        for node in concrete.traverse():
            if node.external or id(node) in seen_nodes:
                continue
            seen_nodes.add(id(node))
            if reuse_active and node.installed:
                continue
            pkg = load_pkg_class(node.name)
            if pkg is None:
                findings.append(Finding("unknown-package", str(node), "cannot load package class"))
                continue
            check_node_versions(node, pkg, findings)
            check_deprecated_version(node, pkg, findings)
            check_node_variants(node, pkg, findings)
            check_sticky_variants(node, pkg, findings, mentioned)
            check_node_dependencies(node, pkg, findings, tests=tests)
            check_node_conflicts(node, pkg, findings)
            check_node_requirements(node, pkg, findings)
            check_edge_providers(node, findings)
            check_patches(node, pkg, findings)
            check_provided_together(node, findings)
            if generic_targets:
                check_target_granularity(node, findings)

    return findings


def _run_case_child(queue, seed: int, pool: List[str], timeout: int) -> None:
    queue.put(run_case(seed, pool, timeout))


def run_case_bounded(seed: int, pool: List[str], timeout: int, wall_limit: int) -> Dict[str, Any]:
    """Run one case in a forked child so a runaway grounding cannot stall the worker."""
    ctx = multiprocessing.get_context("fork")
    queue = ctx.Queue()
    proc = ctx.Process(target=_run_case_child, args=(queue, seed, pool, timeout))
    proc.start()
    proc.join(wall_limit)
    if proc.is_alive():
        proc.terminate()
        proc.join(10)
        if proc.is_alive():
            proc.kill()
            proc.join()
        return {"seed": seed, "status": "wall-timeout", "time": wall_limit}
    try:
        return queue.get(timeout=5)
    except Exception:
        return {"seed": seed, "status": "worker-died", "exitcode": proc.exitcode}


def run_case(seed: int, pool: List[str], timeout: int) -> Dict[str, Any]:
    rng = random.Random(seed)
    drawn = draw_case(rng, pool)
    record: Dict[str, Any] = {"seed": seed}
    if drawn is None:
        record["status"] = "skipped"
        return record
    case = drawn
    roots, packages_cfg = case["roots"], case["packages"]
    record["root"] = " ;; ".join(roots)
    record["packages"] = packages_cfg
    record["concretizer"] = case["concretizer"]
    record["mode"] = case["mode"]
    record["reuse"] = case["reuse"]
    record["tests"] = case["tests"]
    if case["toolchains"]:
        record["toolchains"] = case["toolchains"]

    concretizer_cfg: Dict[str, Any] = {
        "reuse": case["reuse"],
        "timeout": timeout,
        "error_on_timeout": True,
    }
    concretizer_cfg.update(case["concretizer"])
    overrides: Dict[str, Any] = {"concretizer": concretizer_cfg, "packages": packages_cfg}
    if case["toolchains"]:
        overrides["toolchains"] = case["toolchains"]
    scope = spack.config.InternalConfigScope("fuzz", overrides)
    start = time.time()
    try:
        spack.config.CONFIG.push_scope(scope)
        try:
            if case["toolchains"]:
                input_specs = [
                    spack.spec_parser.parse_one_or_raise(r, toolchains=case["toolchains"])
                    for r in roots
                ]
            else:
                input_specs = [spack.spec.Spec(r) for r in roots]
        except Exception as e:
            record["status"] = "unparseable"
            record["error"] = str(e)
            return record
        try:
            if len(input_specs) == 1:
                concretes = [spack.concretize.concretize_one(input_specs[0], tests=case["tests"])]
            else:
                if case["mode"] == "when_possible":
                    pairs = spack.concretize.concretize_together_when_possible(
                        [(s, None) for s in input_specs]
                    )
                else:
                    pairs = spack.concretize.concretize_together([(s, None) for s in input_specs])
                by_abstract = {abstract: concrete for abstract, concrete in pairs}
                concretes = [by_abstract[s] for s in input_specs if s in by_abstract]
        except spack.error.UnsatisfiableSpecError as e:
            record["status"] = "unsat"
            record["error"] = str(e).splitlines()[0][:300]
            return record
        except spack.error.SpackError as e:
            record["status"] = "spack-error"
            record["error"] = f"{type(e).__name__}: {str(e).splitlines()[0][:300]}"
            return record
        except Exception as e:
            record["status"] = "crash"
            record["error"] = f"{type(e).__name__}: {e}"
            record["traceback"] = traceback.format_exc(limit=15)
            return record
        record["status"] = "sat"
        record["concrete"] = " ;; ".join(str(c) for c in concretes)
        try:
            findings = verify(case, concretes)
        except Exception as e:
            record["status"] = "verifier-crash"
            record["error"] = f"{type(e).__name__}: {e}"
            record["traceback"] = traceback.format_exc(limit=15)
            return record
        record["findings"] = [f.to_dict() for f in findings]
    finally:
        record["time"] = round(time.time() - start, 2)
        spack.config.CONFIG.remove_scope("fuzz")
    return record


def parse_seed_range(text: str) -> Iterable[int]:
    if ":" in text:
        a, b = text.split(":")
        return range(int(a), int(b))
    return [int(text)]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--seeds", default="0:50", help="seed or start:stop range")
    parser.add_argument("--pkg", action="append", help="restrict the package pool")
    parser.add_argument("--jsonl", help="append one JSON record per case to this file")
    parser.add_argument("--timeout", type=int, default=60, help="clingo timeout per solve")
    parser.add_argument(
        "--wall-limit", type=int, default=240, help="hard wall-clock seconds per case"
    )
    parser.add_argument("--force-mode", choices=["together", "when_possible"])
    parser.add_argument("-v", "--verbose", action="store_true")
    args = parser.parse_args()

    globals()["FORCE_MODE"] = args.force_mode
    pool = args.pkg or sorted(spack.repo.PATH.all_package_names(include_virtuals=False))
    out = open(args.jsonl, "a") if args.jsonl else None

    seen: set = set()
    totals = {"sat": 0, "unsat": 0, "crash": 0, "skipped": 0, "unparseable": 0, "cases": 0}
    interesting = 0
    for seed in parse_seed_range(args.seeds):
        record = run_case_bounded(seed, pool, args.timeout, args.wall_limit)
        totals["cases"] += 1
        totals[record["status"]] = totals.get(record["status"], 0) + 1
        new_findings = []
        for f in record.get("findings", []):
            sig = (f["kind"], f["node"].split("@")[0], f["detail"].split(" | ")[0])
            if sig not in seen:
                seen.add(sig)
                new_findings.append(f)
        if out:
            print(json.dumps(record), file=out, flush=True)
        status_line = (
            f"[{seed}] {record['status']:>10} {record.get('time', 0):>6}s {record.get('root', '')}"
        )
        if args.verbose or new_findings or record["status"] in ("crash", "verifier-crash"):
            print(status_line, flush=True)
            if record.get("packages"):
                print(f"       packages: {json.dumps(record['packages'])}", flush=True)
            if record["status"] in ("crash", "verifier-crash"):
                print(f"       {record['error']}", flush=True)
            for f in new_findings:
                interesting += 1
                print(f"  !! {f['kind']}: {f['node'][:120]}\n     {f['detail']}", flush=True)
    print(f"done: {totals} unique-findings={len(seen)}", flush=True)
    if out:
        out.close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
