# Handover: Rust implementation of the spec model

Branch `hs/experiment/rust-spec`, branched from `hs/test/property-based-tests` with
`hs/fix/delete-spec-build-interface` merged in (that branch removes
`SpecBuildInterface(lang.ObjectWrapper, Spec)`, which was the one blocker for a PyO3
base class; it must land upstream before this branch does).

Full plan with exploration findings, architecture decisions, and milestone gates:
`~/.claude-personal/plans/merry-sniffing-candy.md`. Quirks found during the port:
`CURIOSITIES.md` (repo root, keep appending). This file is the operational summary.

## Goal

A drop-in Rust implementation of `Spec` (and everything reachable from its state:
versions, variants, flags, arch/targets, edges) with bug-for-bug parse and
satisfies/intersects/constrain semantics, validated by the existing Python test
suite, and ultimately a 2.5-3x faster `spack solve` setup phase via Rust bulk entry
points in the solver (profiling notes in the plan: naive per-call drop-in does NOT
speed up fact generation; `_spec_clauses` accessor traffic and AspFunction string
building dominate, `Spec.satisfies` is nearly irrelevant during setup).

## How it works

- `rust/` Cargo workspace: `spack-spec-core` (pure Rust, no pyo3 — natively tested
  algebra: versions, variants, targets, lexer/event parser, ASP strings) and
  `spack-spec-py` (cdylib `spack_spec`, pyo3 0.26 — pyclasses + graph algebra over
  Python objects).
- Toggle: `SPACK_SPEC_IMPL=rust` read at module scope in `spack/spec.py`,
  `spack/version/__init__.py`, `spack/variant.py`. Strict: missing extension is a
  hard ImportError, never a silent fallback. Pure Python remains the default and is
  textually unchanged in behavior.
- Layering: Python `class Spec(_SpecBase)` where `_SpecBase = spack_spec.Spec` under
  the toggle. The Rust base owns all node state (name, namespace, abstract_hash,
  `_concrete`, versions/variants/compiler_flags/architecture handles,
  `_provided_virtuals`, `_dependencies`/`_dependents` as real PyDicts) and the ported
  methods; `_RUST_PORTED_NAMES` in spec.py deletes the Python methods under the
  toggle so lookups fall through the MRO (delete-to-expose). Same pattern for
  `DependencySpec`. `FlagMap`/`CompilerFlag` stay pure Python (str subclass; flags
  never make specs disjoint). VariantMap is still Python over Rust values.
- The extension imports nothing from spack: everything is push-registered at import
  time (`spack_spec.register_*` — Spec class, EMPTY_SPEC, exception classes, enums,
  archspec/platform oracles as late-bound callables so test monkeypatching works).
- Pickle formats are byte-identical across modes; Rust classes implement the GC
  protocol (parent<->edge<->child cycles); `_ImmutableSpec` guards keep firing
  because mutations of foreign objects go through the Python attribute protocol.

## Build & test

```sh
# build the extension into .venv (x86_64 rustup under Rosetta -> pass the target).
# --release for any timing: a debug build is ~2x slower than the Python it replaces.
cd rust/spack-spec-py && ../../.venv/bin/python -m maturin develop --release --target aarch64-apple-darwin
# native core tests
cd rust && cargo test -p spack-spec-core
# the gate (identical results expected in both modes; currently 1304 passed / 15 skipped)
source .venv/bin/activate
SPACK_SPEC_IMPL=rust python -m pytest lib/spack/spack/test/spec_semantics.py \
  lib/spack/spack/test/spec_syntax.py lib/spack/spack/test/spec_dag.py \
  lib/spack/spack/test/versions.py lib/spack/spack/test/variant.py \
  lib/spack/spack/test/architecture.py lib/spack/spack/test/spec_algebra.py \
  lib/spack/spack/test/spec_algebra_targets.py lib/spack/spack/test/spec_format.py \
  lib/spack/spack/test/spec_yaml.py lib/spack/spack/test/spec_list.py \
  -q --spack-concretization-cache=/tmp/cache
# property harnesses (byte-identical output across modes is the bar)
SPACK_SPEC_IMPL=rust PYTHONPATH=lib/spack python lib/spack/spack/test/spec_algebra_corpus.py
SPACK_SPEC_IMPL=rust PYTHONPATH=lib/spack python lib/spack/spack/test/spec_algebra_properties.py --seed 0 --iterations 4000
# parse differential: spawns one process per mode itself, Python is the oracle
PYTHONPATH=lib/spack python lib/spack/spack/test/spec_parse_differential.py
# ASP facts: both classes in one process, so run it with the toggle unset
PYTHONPATH=lib/spack python lib/spack/spack/test/asp_fact_differential.py
# the ASP instance itself must not move a byte
for pkg in zlib hdf5+mpi trilinos; do
  diff <(bin/spack --color=never solve --show=asp $pkg) \
       <(SPACK_SPEC_IMPL=rust bin/spack --color=never solve --show=asp $pkg)
done
```

`requires_python_spec` pytest marker skips a test only under the toggle — so far
**zero tests needed it**. A conftest session fixture fails fast if the toggle is set
but the extension doesn't back `Spec`.

## State (all committed, style-clean, gates green)

| Milestone | Status |
|---|---|
| M0 toggle + subclass + ordering + pickle | done |
| M1 versions (core + bindings) | done — core validated by 145k-case differential fuzz |
| M2 variants (core + bindings) | done |
| M3 targets/ArchSpec (core + bindings) | done — canonical strings byte-identical; ~30% faster than Python |
| M4 lexer/event-parser core | done — all 114 golden token rows, 0/5597 differential |
| M4 binding (`Spec(str)` and `spec_parser.parse_one_or_raise` via Rust) | done — 0/293 on the parse differential |
| M5a state migration, M5b algebra + construction/ordering/format | done |
| ASP fact-string core (`spack-spec-core/src/asp.rs`) | done (groundwork for M7) |
| M6 differential runner (`test/spec_differential.py`) | not started (see plan: two-process design, Python as oracle) |
| M7 bulk fact emission (AspFunction/ProblemBuffer pyclasses + solver wiring) | done — ASP instance byte-identical on zlib/hdf5+mpi/trilinos; trilinos setup 2.02s -> 1.56s |
| M8 bulk clause generation (`spec.clauses()`, interned directive specs, variant-def table in Rust, version-order table) | not started; requires M5 (done) |

Perf acceptance: `spack solve --timers` setup line >=2.5x on trilinos (warm), byte-
identical ASP problem instance vs the Python builder on zlib/hdf5+mpi/trilinos. After M7 the
warm trilinos setup is 2.02s -> 1.56s (1.29x); the rest is M8's, where `_spec_clauses`
accessor traffic lives.

## Working method that has been effective

One background agent per milestone with: exact Python reference file/lines, explicit
bug-for-bug requirement, the file-ownership constraints (never edit
`spack-spec-core` and `spack-spec-py` concurrently from two agents; core and binding
crates can proceed in parallel), the full gate commands with expected baselines, and
"report ambiguities, don't fix Python bugs". Every milestone: run gates in BOTH
modes, `bin/spack style --fix`, `cargo fmt`, zero warnings, commit with `--signoff`
(no Co-Authored-By), one commit per milestone. Append new Python quirks to
CURIOSITIES.md. After any rebase onto develop: rebuild the extension, rerun the gate
+ both harnesses — Python is always the oracle.

## Gotchas for whoever continues

- rustup here is an old x86_64 binary running under Rosetta; the default toolchain is
  `stable-x86_64-apple-darwin` with the `aarch64-apple-darwin` target added. Always
  build the extension with `--target aarch64-apple-darwin` (the venv Python is arm64).
  Components use `-preview` names (`rustfmt-preview`, `clippy-preview`).
- `rust/.cargo/config.toml` carries the macOS `-undefined dynamic_lookup` link args so
  plain `cargo build`/`cargo test` work on the cdylib.
- `bin/spack python` does not see `.venv` site-packages; run harnesses with
  `PYTHONPATH=lib/spack .venv/bin/python ...` instead.
- mypy: the untyped `lazy_lexicographic_ordering` decorator previously made `Spec`
  `Any`; with it applied conditionally, new precise types can surface new mypy errors
  in untouched code (fix at the use site, e.g. `current: Any` in `format`).
- Python-mode and rust-mode pickles do not interchange for the component classes
  (different class identities); each mode round-trips itself; formats are identical.
- Keep `_RUST_PORTED_NAMES` and `algebra.rs`/`graph.rs`/`cmp.rs`/`fmt.rs` in sync —
  that tuple is the single audit point for what Rust serves.
