# Copyright Spack Project Developers. See COPYRIGHT file for details.
#
# SPDX-License-Identifier: (Apache-2.0 OR MIT)

"""Differential test of the Rust ASP fact builder against the Python one.

``SPACK_SPEC_IMPL=rust`` replaces ``spack.solver.core.AspFunction``/``AspVar`` and
``spack.solver.asp.ProblemInstanceBuilder`` with the extension's, which render atoms without
building the argument strings in Python. Both classes can be imported at once, so this needs no
subprocess: it builds the same terms with both and compares what a solve depends on -- the
rendered string, the ``key_ordering`` comparisons, and the lines a problem instance ends up with.

    PYTHONPATH=lib/spack python lib/spack/spack/test/asp_fact_differential.py

Hash *values* are deliberately not compared. ``AspVar`` defines no ``__hash__`` in either
implementation, so a term holding one hashes by identity and two equal terms built separately
hash differently within Python itself; what has to agree is which terms hash alike.

Nothing here is named ``test_*`` or ``Test*``, since ``pytest.ini`` collects every file in this
directory and this is not meant to be collected.
"""

import argparse
import itertools
import sys
from typing import Any, Callable, List, Optional, Sequence, Tuple

import spack.solver.core as core
from spack.util.spack_yaml import syaml_int, syaml_str


def terms(function: Any, var: Any) -> List[Any]:
    """The same list of terms built with one implementation's classes.

    Every branch of ``AspFunction.__str__``'s type ladder is here: exact ``str``, exact ``int``,
    nested functions, variables, the ``syaml`` subclasses that config values arrive as, ``bool``
    (an ``int`` subclass that ASP still wants quoted), and objects that fall through to a bare
    ``str()``.
    """
    return [
        function("attr", ("version", "zlib")),
        function("node", (0, "zlib")),
        function("empty"),
        function("escapes", ('a"b\\c\nd', "", " ")),
        function("ints", (-3, 0, 2**70, -(2**70))),
        function("bools", (True, False)),
        function("subclasses", (syaml_str("s"), syaml_int(7))),
        function("vars", (var("Hash"), var("NID"))),
        function("nested", ("a", function("inner", (1, var("X"))), 2)),
        function("fallbacks", (None, 1.5, ("t",), ["l"])),
        function("unicode", ("héllo→",)),
        # Calls are additive, and a call on a function that already has arguments extends them.
        function("attr")("version")("foo"),
        function("attr", ("version",))("foo", "bar"),
    ]


def probe(thunk: Callable[[], Any]) -> Tuple[str, Any]:
    """The value a call produces, or the exception class it raises."""
    try:
        return ("ok", thunk())
    except Exception as e:  # noqa: BLE001 -- the exception class is part of the comparison
        return ("raised", type(e).__name__)


def check_rendering(python: List[Any], rust: List[Any]) -> List[str]:
    return [
        f"str({p!r:.60}): python {str(p)!r} != rust {str(r)!r}"
        for p, r in zip(python, rust)
        if str(p) != str(r)
    ]


def check_ordering(python: List[Any], rust: List[Any]) -> List[str]:
    """``key_ordering`` installs six comparisons with an opinion about ``None``; all six have to
    agree with the Python class, including which comparisons raise."""
    ops: Sequence[Tuple[str, Callable[[Any, Any], Any]]] = [
        ("==", lambda a, b: a == b),
        ("!=", lambda a, b: a != b),
        ("<", lambda a, b: a < b),
        ("<=", lambda a, b: a <= b),
        (">", lambda a, b: a > b),
        (">=", lambda a, b: a >= b),
    ]
    failures = []
    for i, j in itertools.product(range(len(python)), repeat=2):
        for name, op in ops:
            p = probe(lambda: op(python[i], python[j]))  # noqa: B023
            r = probe(lambda: op(rust[i], rust[j]))  # noqa: B023
            if p != r:
                failures.append(f"terms[{i}] {name} terms[{j}]: python {p}, rust {r}")
    # Comparing against something that is not a term at all, where key_ordering is peculiar.
    for i in range(len(python)):
        for other in (None, "attr", 42):
            for name, op in ops:
                p = probe(lambda: op(python[i], other))  # noqa: B023
                r = probe(lambda: op(rust[i], other))  # noqa: B023
                if p != r:
                    failures.append(f"terms[{i}] {name} {other!r}: python {p}, rust {r}")
    return failures


def check_hashing(python: List[Any], rust: List[Any]) -> List[str]:
    """Which terms hash alike, and which are unhashable at all."""
    failures = []
    hashable = []
    for i in range(len(python)):
        p, r = probe(lambda: hash(python[i])), probe(lambda: hash(rust[i]))  # noqa: B023
        if p[0] != r[0]:
            failures.append(f"hash(terms[{i}]): python {p[0]}, rust {r[0]}")
        elif p[0] == "ok":
            hashable.append(i)

    for i, j in itertools.product(hashable, repeat=2):
        same_python = hash(python[i]) == hash(python[j])
        same_rust = hash(rust[i]) == hash(rust[j])
        if same_python != same_rust:
            failures.append(
                f"hash(terms[{i}]) == hash(terms[{j}]): python {same_python}, rust {same_rust}"
            )
    return failures


def check_problem_instance(python: List[Any], rust: List[Any]) -> List[str]:
    """The lines two builders accumulate, and the stripped, sorted, joined instance."""
    import spack_spec

    from spack.solver.asp import ProblemInstanceBuilder as PythonBuilder

    def build(builder: Any, facts: List[Any]) -> Any:
        builder.h1("Header")
        for fact in facts:
            builder.fact(fact)
        builder.h2("Section")
        builder.h3("Subsection")
        builder.append("a :- b.")
        builder.newline()
        builder.title("Custom", "*")
        return builder

    py_builder = build(PythonBuilder(), python)
    rust_builder = build(spack_spec.ProblemInstanceBuilder(), rust)

    failures = []
    if list(py_builder.asp_problem) != list(rust_builder.asp_problem):
        failures.append(
            f"asp_problem differs:\n  python {py_builder.asp_problem}\n"
            f"  rust   {rust_builder.asp_problem}"
        )
    if py_builder.stripped_sorted_str() != rust_builder.stripped_sorted_str():
        failures.append("stripped_sorted_str differs")
    return failures


CHECKS = [check_rendering, check_ordering, check_hashing, check_problem_instance]


def main(argv: Optional[List[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.parse_args(argv)

    import spack_spec

    if core.USE_RUST_SPEC:
        # With the toggle set both names would resolve to the extension and every check would
        # compare it against itself. The harness imports both classes directly; it needs no mode.
        print("run this with SPACK_SPEC_IMPL unset", file=sys.stderr)
        return 2

    python = terms(core.AspFunction, core.AspVar)
    rust = terms(spack_spec.AspFunction, spack_spec.AspVar)

    total = 0
    for check in CHECKS:
        name = check.__name__[len("check_") :]
        failures = check(python, rust)
        total += len(failures)
        for failure in failures:
            print(f"{name}: {failure}", flush=True)
        if not failures:
            print(f"{name}: ok", flush=True)

    print(f"\n{total} difference(s)" if total else f"\nno differences in {len(python)} term(s)")
    return 1 if total else 0


if __name__ == "__main__":
    sys.exit(main())
