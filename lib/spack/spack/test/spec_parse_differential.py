# Copyright Spack Project Developers. See COPYRIGHT file for details.
#
# SPDX-License-Identifier: (Apache-2.0 OR MIT)

"""Differential test of the Rust spec parser against the Python one, with Python as the oracle.

``SPACK_SPEC_IMPL=rust`` routes ``Spec("...")`` and ``spack.spec_parser.parse_one_or_raise``
through the Rust parser, which replays an event stream into the same ``Spec`` construction calls
``SpecParser`` makes. The gated test suite covers the specs that parse; what it barely reaches is
the error surface, where the two parsers have to agree not just on failing but on the exception
class, the message and the column the caret underlines.

This feeds a corpus of spec literals through both implementations and compares the outcome of
each: the canonical string of the parsed spec, or the exception class and its message. The
toggle is read at module scope, so the two implementations cannot coexist in one process; the
default run re-executes this script twice, once per mode, and diffs the records.

    PYTHONPATH=lib/spack python lib/spack/spack/test/spec_parse_differential.py
    PYTHONPATH=lib/spack python lib/spack/spack/test/spec_parse_differential.py --emit

Nothing here is named ``test_*`` or ``Test*``, since ``pytest.ini`` collects every file in this
directory and this is not meant to be collected.
"""

import argparse
import gzip
import os
import shutil
import subprocess
import sys
import tempfile
from typing import List, Optional, Sequence

import spack.paths
import spack.repo

#: Set by the parent process to a spec file both children can read, so filename literals (and the
#: refusal to hang dependencies off the concrete spec they load) are part of the comparison.
SPECFILE_ENV = "SPACK_PARSE_DIFF_SPECFILE"

#: A concrete spec in the test data, decompressed into a temporary directory for that variable.
SPECFILE_SOURCE = "lib/spack/spack/test/data/specfiles/hdf5.v020.json.gz"

#: Literals the parsers are known to be delicate about, beyond what the test tables cover.
EXTRA = [
    # The order in which a Spec mutation error and a parse error are raised. Python interleaves
    # them by construction; the Rust binding has to replay its partial event stream to match.
    "x+debug+debug @1@2",
    "x@1@2",
    "x+debug+debug",
    # The one residual ordering divergence documented in rust/spack-spec-core/src/parse/mod.rs:
    # a direct edge attaches one token later than Python's attach, so a tokenization error on
    # that very token supersedes the attach error Python raises.
    "x %[virtuals=c]gcc %[virtuals=c]clang %&",
    "x %[virtuals=c]gcc %[virtuals=c]clang",
    # Anonymous specs and the '*' name.
    "*",
    "* +debug",
    "^zlib",
    "%gcc",
    # Abstract hashes.
    "/abc123",
    "zlib/abc123",
    "zlib ^/abc123",
    "zlib/abc123/def456",
    # Conditional edges: the canonicalization pass at the end of next_spec.
    "foo %[when='%c'] gcc",
    "foo %[when='+debug'] gcc %[when='~debug'] gcc",
    "foo ^[when='+debug'] bar ^[when='~debug'] bar",
    "foo %[when='@@'] gcc",
    # Edge properties.
    "foo ^[deptypes=build,link] bar",
    "foo ^[deptypes=nosuchtype] bar",
    "foo ^[virtuals=mpi] mpich",
    "foo %[virtuals=c,cxx] gcc",
    # Legacy compiler aliases, rewritten only on direct edges.
    "foo %clang",
    "foo ^clang",
    "foo %clang@15",
    # Propagation of reserved names, an error raised by Spec._add_flag, not by the parser.
    "x arch==linux-rhel9-x86_64",
    "x patches==abcde12345",
    # Git versions, which attach a lookup only when the version list holds one.
    "foo@git.abcd=1.0",
    "foo@abcdefabcdefabcdefabcdefabcdefabcdefabcd=1.0",
    # Trailing text, the parse_one_or_raise rejection.
    "x y",
    "  x y   z",
    "",
    "   ",
]


def _tables() -> List[str]:
    """Every spec literal in the parametrize tables of ``spec_syntax.py``."""
    import spack.test.spec_syntax as syntax

    literals: List[str] = []

    def rows(func_name: str):
        func = getattr(syntax, func_name)
        for mark in getattr(func, "pytestmark", []):
            if mark.name == "parametrize":
                yield from mark.args[1]

    for row in rows("test_parse_single_spec"):
        literals.append(row[0])
    for row in rows("test_parse_multiple_specs"):
        # The text itself is several specs; each expected single spec is a literal of its own.
        literals.append(row[0])
        literals.extend(row[2])
    for row in rows("test_error_reporting"):
        literals.append(row[0])
    for row in rows("test_error_conditions"):
        literals.append(row[0])
    for row in rows("test_specfile_error_conditions_windows"):
        literals.append(row[0])

    return literals


def inputs() -> List[str]:
    """The corpus, in a stable order and without duplicates."""
    from spack.test.spec_algebra_corpus import CORPUS

    literals = _tables() + list(CORPUS) + EXTRA

    specfile = os.environ.get(SPECFILE_ENV)
    if specfile:
        literals += [
            specfile,
            f"{specfile} ^zlib",
            f"{specfile} %clang",  # the error message shows the pre-alias name
            f"{specfile} %gcc@15",
            "./no-such-spec-file.json",
            "./no-such-spec-file.yaml",
        ]

    seen, unique = set(), []
    for literal in literals:
        if literal not in seen:
            seen.add(literal)
            unique.append(literal)
    return unique


def record(literal: str) -> str:
    """What the parser did with ``literal``, as one line: the spec, or the failure.

    Formatting the result is part of the record, so a literal whose canonical string needs a
    fixture this script does not set up (a git version wanting a repository to resolve a ref)
    is captured as the failure it raises, which both implementations have to agree on too.
    """
    from spack.spec import Spec

    try:
        spec = Spec(literal)
        outcome = f"OK\t{str(spec)!r}\t{spec.abstract_hash!r}"
    except Exception as e:  # noqa: BLE001 -- the exception class is part of the comparison
        outcome = f"RAISED\t{type(e).__name__}\t{str(e)!r}"
    return f"{literal!r}\t{outcome}"


def emit() -> int:
    with spack.repo.use_repositories(spack.repo.from_path(spack.paths.mock_packages_path)):
        for literal in inputs():
            print(record(literal), flush=True)
    return 0


def run(impl: Optional[str], specfile: str) -> List[str]:
    """This script under ``--emit``, in a subprocess pinned to one implementation."""
    env = dict(os.environ)
    env.pop("SPACK_SPEC_IMPL", None)
    if impl is not None:
        env["SPACK_SPEC_IMPL"] = impl
    env[SPECFILE_ENV] = specfile
    env["PYTHONPATH"] = os.pathsep.join([spack.paths.lib_path, env.get("PYTHONPATH", "")])

    result = subprocess.run(
        [sys.executable, os.path.abspath(__file__), "--emit"],
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        universal_newlines=True,
    )
    if result.returncode != 0:
        raise RuntimeError(f"the {impl or 'python'} run failed:\n{result.stderr}")
    return result.stdout.splitlines()


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--emit", action="store_true", help="print the records and exit")
    parser.add_argument("--show", type=int, default=20, help="how many differences to print")
    args = parser.parse_args(argv)

    if args.emit:
        return emit()

    with tempfile.TemporaryDirectory() as tmp:
        specfile = os.path.join(tmp, "hdf5.json")
        with gzip.open(os.path.join(spack.paths.prefix, SPECFILE_SOURCE), "rb") as src:
            with open(specfile, "wb") as dst:
                shutil.copyfileobj(src, dst)

        python = run(None, specfile)
        rust = run("rust", specfile)

    if len(python) != len(rust):
        print(f"record count differs: python {len(python)}, rust {len(rust)}")
        return 1

    differences = [(p, r) for p, r in zip(python, rust) if p != r]
    for py_record, rust_record in differences[: args.show]:
        print(f"python: {py_record}\n  rust: {rust_record}\n")
    if len(differences) > args.show:
        print(f"... and {len(differences) - args.show} more\n")

    total = len(python)
    if differences:
        print(f"{len(differences)} difference(s) in {total} spec literal(s)")
        return 1
    print(f"no differences in {total} spec literal(s)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
