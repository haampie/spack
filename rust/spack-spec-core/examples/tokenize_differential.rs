// SPDX-License-Identifier: (Apache-2.0 OR MIT)

//! Differential checker: compares the hand-written lexer against a JSONL oracle generated from
//! Python's `spack.spec_parser.SPEC_TOKENIZER`. Each line is
//! `{"input": str, "tokens": [{"kind", "value", "start", "end", "virtuals", "substitute"}, ...]}`
//! with the raw token stream (WS and UNEXPECTED included).
//!
//! Generate the oracle from the repo root with the venv active:
//!
//! ```sh
//! python rust/spack-spec-core/examples/gen_tokenize_oracle.py /tmp/tokenize_oracle.jsonl
//! ```
//!
//! Usage: `cargo run -p spack-spec-core --example tokenize_differential <oracle.jsonl>`

use serde_json::Value;
use spack_spec_core::parse::lexer::tokenize_all;

fn field(token: &Value, name: &str) -> Option<String> {
    match &token[name] {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    }
}

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: tokenize_differential <oracle.jsonl>");
    let data = std::fs::read_to_string(&path).expect("cannot read oracle");

    let mut cases = 0usize;
    let mut mismatches = 0usize;
    for line in data.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let record: Value = serde_json::from_str(line).expect("invalid oracle line");
        let input = record["input"].as_str().expect("input");
        let expected = record["tokens"].as_array().expect("tokens");
        cases += 1;

        let actual = tokenize_all(input);
        let mut bad = actual.len() != expected.len();
        if !bad {
            for (a, e) in actual.iter().zip(expected) {
                if a.kind.python_name() != e["kind"].as_str().unwrap()
                    || a.value != e["value"].as_str().unwrap()
                    || a.start as u64 != e["start"].as_u64().unwrap()
                    || a.end as u64 != e["end"].as_u64().unwrap()
                    || a.virtuals.map(str::to_string) != field(e, "virtuals")
                    || a.substitute.map(str::to_string) != field(e, "substitute")
                {
                    bad = true;
                    break;
                }
            }
        }
        if bad {
            mismatches += 1;
            eprintln!("MISMATCH on input {input:?}");
            eprintln!("  python: {expected:?}");
            eprintln!("  rust:   {actual:?}");
        }
    }

    println!("{cases} inputs compared, {mismatches} mismatches");
    if mismatches > 0 {
        std::process::exit(1);
    }
}
