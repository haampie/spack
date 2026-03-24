#!/usr/bin/env python3
"""Benchmark for FAST_SPEC_REGEX tokenization speed."""

import gc
import time

import spack.spec_parser

gc.disable()

try:
    from spack.spec_parser import FAST_SPEC_REGEX as REGEX
except ImportError:
    from spack.spec_parser import SPEC_TOKENIZER

    REGEX = SPEC_TOKENIZER.regex


# Test spec strings from spec_syntax.py
TEST_SPECS = [
    # Simple package names
    "mvapich",
    "mvapich_foo",
    "_mvapich_foo",
    "3dtk",
    "ns-3-dev",
    # Anonymous specs with versions
    "@2.7",
    "@2.7:",
    "@:2.7",
    "+foo",
    "~foo",
    "-foo",
    "platform=test",
    # Specs with dependencies
    "openmpi ^hwloc",
    "openmpi ^hwloc ^libunwind",
    "openmpi      ^hwloc^libunwind",
    # Complex specs with multiple constraints
    "mvapich_foo ^_openmpi@1.2:1.4,1.6+debug~qt_4 %intel@12.1 ^stackwalker@8.1_1e",
    "mvapich_foo ^_openmpi@1.2:1.4,1.6~qt_4 debug=2 %intel@12.1 ^stackwalker@8.1_1e",
    "mvapich_foo ^_openmpi@1.2:1.4,1.6 cppflags=-O3 +debug~qt_4 %intel@12.1 ^stackwalker@8.1_1e",
    # Specs with YAML/JSON package names
    "yaml-cpp@0.1.8%intel@12.1 ^boost@3.1.4",
    "builtin.yaml-cpp%gcc",
    "testrepo.yaml-cpp%gcc",
    "builtin.yaml-cpp@0.1.8%gcc@7.2.0 ^boost@3.1.4",
    "builtin.yaml-cpp ^testrepo.boost ^zlib",
    # Variant ordering
    "y~f+e~d+c~b+a",
    # Anonymous dependencies with variants
    "^*foo=bar",
    "%*foo=bar",
    "^*+foo",
    "^*~foo",
    "%*+foo",
    "%*~foo",
    # Version ranges and lists
    "@1.6,1.2:1.4",
    # Architecture-related
    "platform=linux",
    # Git versions
    f"develop-branch-version@{'abc12'*8}=develop",
    f"develop-branch-version@git.{'a' * 40}=develop+var1+var2",
    # Compiler version ranges
    "%gcc@10.2.1:",
    "%gcc@:10.2.1",
    "%gcc@10.2.1:12.1.0",
    "%gcc@10.1.0,12.2.1:",
    "%gcc@:8.4.3,10.2.1:12.1.0",
    # Special values
    "dev_path=*",
    "dev_path=none",
    "dev_path=../relpath/work",
    "dev_path=/abspath/work",
    # Flags
    "cflags=a=b=c",
    "cflags=a=b=c+~",
    "cflags=-Wl,a,b,c",
    'cflags=="-O3 -g"',
    # Whitespace handling
    "@1.2:1.4 , 1.6 ",
    "+ debug % intel @ 12.1:12.6",
    "@ 12.1:12.6 + debug - qt_4",
    # Redundant specs
    "x ^y@foo ^y@foo",
    "x ^y@foo ^y+bar",
    "x ^y@foo +bar ^y@foo",
    # Ambiguous variant specifications
    "_openmpi +debug-qt_4",
    "_openmpi +debug -qt_4",
    "_openmpi +debug~qt_4",
    # Key value pairs with special characters
    "target=:broadwell,icelake",
    # Version with compiler
    "foo @2.0 %bar@1.0",
    # Dependency ordering
    "mvapich ^stackwalker ^_openmpi",
]


def tokenize_spec(spec_string: str) -> int:
    """Tokenize a spec string and return the number of tokens."""
    scanner = REGEX.scanner(spec_string.rstrip())
    token_count = 0
    match = scanner.match()

    while match:
        token_count += 1
        match = scanner.match()

    return token_count


def main():
    iterations = 10000

    print(
        f"{'Spec':<100} | {'Tokens':<7} | {'Tokenize (μs)':<15} | {'Spec() (μs)':<13} | "
        "Overhead %"
    )
    print("-" * 100 + "-+-" + "-" * 7 + "-+-" + "-" * 15 + "-+-" + "-" * 13 + "-+-" + "-" * 11)

    total_time = 0.0

    for spec in TEST_SPECS:
        # Count tokens
        tokens = tokenize_spec(spec)
        spack.spec_parser.parse_one_or_raise(spec)

        # Benchmark tokenization
        start = time.perf_counter()
        for _ in range(iterations):
            tokenize_spec(spec)
        end = time.perf_counter()
        tokenize_time_us = (end - start) / iterations * 1e6

        # Benchmark Spec construction
        start = time.perf_counter()
        for _ in range(iterations):
            spack.spec_parser.parse_one_or_raise(spec)
        end = time.perf_counter()
        spec_time_us = (end - start) / iterations * 1e6
        total_time += spec_time_us

        # Calculate overhead percentage
        overhead_pct = (spec_time_us - tokenize_time_us) / spec_time_us * 100

        # Truncate spec if longer than 100 chars
        spec_display = spec if len(spec) <= 100 else spec[:97] + "..."

        print(
            f"{spec_display:<100} | {tokens:<7} | {tokenize_time_us:>15.2f} | "
            f"{spec_time_us:>13.2f} | {overhead_pct:>10.1f}%"
        )

    print("-" * 100 + "-+-" + "-" * 7 + "-+-" + "-" * 15 + "-+-" + "-" * 13 + "-+-" + "-" * 11)
    print(f"{'Total':<100} | {'':<7} | {'':<15} | {total_time:>13.2f} | {'':>11}")


if __name__ == "__main__":
    main()
