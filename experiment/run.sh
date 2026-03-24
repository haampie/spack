#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
PYTHONPATH=$PWD/lib/spack python3 experiment/bench_tokenization.py 2>&1 | tee experiment/out.txt
