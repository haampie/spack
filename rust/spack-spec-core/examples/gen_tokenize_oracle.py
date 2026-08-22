# SPDX-License-Identifier: (Apache-2.0 OR MIT)
"""Generate a JSONL oracle of Python spec token streams for the Rust differential test.

Each line: {"input": str, "tokens": [{"kind", "value", "start", "end", "virtuals",
"substitute"}, ...]} where tokens is the raw SPEC_TOKENIZER stream (WS and UNEXPECTED included).

The corpus is the test_parse_single_spec golden table, the spec algebra CORPUS, all
spec_algebra_properties DIMENSIONS fragments, and 500 seeded random mutations of those.

Run from the repo root with the venv active:

    python rust/spack-spec-core/examples/gen_tokenize_oracle.py /tmp/tokenize_oracle.jsonl
    cargo run -p spack-spec-core --example tokenize_differential /tmp/tokenize_oracle.jsonl
"""

import json
import random
import sys

sys.path.insert(0, "lib/spack")

import spack.test.spec_syntax as syntax  # noqa: E402
from spack.spec_parser import SPEC_TOKENIZER  # noqa: E402
from spack.test.spec_algebra_corpus import CORPUS  # noqa: E402
from spack.test.spec_algebra_properties import DIMENSIONS  # noqa: E402

corpus = []

# 1. golden table inputs
marks = [
    mk
    for mk in syntax.test_parse_single_spec.pytestmark
    if mk.name == "parametrize" and mk.args[0].startswith("spec_str")
]
corpus += [row[0] for row in marks[0].args[1]]

# 2. spec algebra corpus
corpus += list(CORPUS)

# 3. dimension fragments
fragments = []
for dim in DIMENSIONS:
    fragments += [c for c in dim.constraints if c]
    fragments += [p for p in dim.points if p]
corpus += fragments

# 4. seeded random mutations and concatenations
rng = random.Random(42)
joiners = ["", " ", "  ", "\t", "^", "%", "%%", ",", "/", "@", "[", "]", ":", "=", "~", "+"]
chars = "@:=+~-^%[]/*.,'\"\\ _abc019"
pool = [c for c in corpus if c]
mutations = []
while len(mutations) < 500:
    op = rng.randrange(3)
    if op == 0:  # concatenation of fragments with random joiners
        n = rng.randint(2, 4)
        parts = [rng.choice(pool) for _ in range(n)]
        text = parts[0]
        for part in parts[1:]:
            text += rng.choice(joiners) + part
    elif op == 1:  # random slice
        base = rng.choice(pool)
        i = rng.randrange(len(base) + 1)
        j = rng.randrange(len(base) + 1)
        text = base[min(i, j) : max(i, j)]
    else:  # random character edits
        text = list(rng.choice(pool))
        for _ in range(rng.randint(1, 3)):
            pos = rng.randrange(len(text) + 1)
            if rng.random() < 0.7:
                text.insert(pos, rng.choice(chars))
            elif text:
                del text[pos % len(text)]
        text = "".join(text)
    mutations.append(text)
corpus += mutations

seen = set()
out = open(sys.argv[1], "w")
count = 0
for text in corpus:
    if text in seen:
        continue
    seen.add(text)
    tokens = []
    for tok in SPEC_TOKENIZER.tokenize(text):
        sub = tok.subvalues or {}
        tokens.append(
            {
                "kind": tok.kind.name,
                "value": tok.value,
                "start": tok.start,
                "end": tok.end,
                "virtuals": sub.get("virtuals"),
                "substitute": sub.get("substitute"),
            }
        )
    out.write(json.dumps({"input": text, "tokens": tokens}) + "\n")
    count += 1
out.close()
print(f"{count} unique inputs written", file=sys.stderr)
