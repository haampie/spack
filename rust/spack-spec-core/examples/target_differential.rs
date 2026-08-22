// SPDX-License-Identifier: (Apache-2.0 OR MIT)

//! Differential checker: compares the target algebra against a TSV oracle generated from the
//! Python implementation. Each row is
//! `a<TAB>b<TAB>canonical_a<TAB>satisfies<TAB>intersects<TAB>intersection<TAB>constrain`,
//! where booleans are `True`/`False`, the intersection is `|`-joined, and the constrain field
//! is `UNSAT` or `result|changed`.
//!
//! Python's `_maximal_lower_bounds` iterates a set, so when it has several elements the raw
//! intersection order is hash-seed dependent; rows whose intersection matches as a multiset
//! but not byte-for-byte are counted separately as `order-only`.
//!
//! Usage: `cargo run -p spack-spec-core --example target_differential <oracle.tsv>`

use std::collections::HashSet;

use spack_spec_core::targets::{TargetExpr, TargetGraph};

const JSON_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/spack/spack/vendor/archspec/json/cpu/microarchitectures.json"
);

/// The JSON records in archspec `TARGETS` insertion order (parents recursively first).
fn records() -> Vec<(String, Vec<String>, String)> {
    let text = std::fs::read_to_string(JSON_PATH).expect("vendored microarchitectures.json");
    let data: serde_json::Value = serde_json::from_str(&text).unwrap();
    let map = data["microarchitectures"].as_object().unwrap();
    let mut seen: HashSet<String> = HashSet::new();
    let mut result = Vec::new();
    fn fill(
        name: &str,
        map: &serde_json::Map<String, serde_json::Value>,
        seen: &mut HashSet<String>,
        result: &mut Vec<(String, Vec<String>, String)>,
    ) {
        let value = &map[name];
        let parents: Vec<String> = value["from"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p.as_str().unwrap().to_string())
            .collect();
        for parent in &parents {
            if !seen.contains(parent) {
                fill(parent, map, seen, result);
            }
        }
        seen.insert(name.to_string());
        result.push((
            name.to_string(),
            parents,
            value["vendor"].as_str().unwrap().to_string(),
        ));
    }
    for name in map.keys() {
        if !seen.contains(name) {
            fill(name, map, &mut seen, &mut result);
        }
    }
    result
}

fn py_bool(b: bool) -> &'static str {
    if b {
        "True"
    } else {
        "False"
    }
}

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: target_differential <oracle.tsv>");
    let data = std::fs::read_to_string(&path).expect("failed to read oracle");
    let graph = TargetGraph::from_records(records()).expect("well-formed target table");
    let (mut checked, mut mismatches, mut order_only) = (0u64, 0u64, 0u64);
    for line in data.lines() {
        let fields: Vec<&str> = line.split('\t').collect();
        assert_eq!(fields.len(), 7, "malformed row: {line}");
        let (sa, sb) = (fields[0], fields[1]);
        let a = TargetExpr::parse(&graph, sa);
        let b = TargetExpr::parse(&graph, sb);
        let intersection = graph.target_intersection(&a, &b);
        let constrain = match graph.target_constrain(Some(&a), Some(&b)) {
            Err(_) => "UNSAT".to_string(),
            Ok((result, changed)) => {
                format!("{}|{}", result.unwrap(), py_bool(changed))
            }
        };
        let got = [
            a.as_str().to_string(),
            py_bool(graph.target_satisfies(Some(&a), Some(&b))).to_string(),
            py_bool(graph.target_intersects(Some(&a), Some(&b))).to_string(),
            intersection.join("|"),
            constrain,
        ];
        for (i, (g, e)) in got.iter().zip(&fields[2..]).enumerate() {
            checked += 1;
            if g == e {
                continue;
            }
            if i == 3 {
                // intersection: tolerate order-only differences (Python's order is
                // hash-seed dependent for multi-element maximal lower bounds)
                let mut gs: Vec<&str> = g.split('|').collect();
                let mut es: Vec<&str> = e.split('|').collect();
                gs.sort_unstable();
                es.sort_unstable();
                if gs == es {
                    order_only += 1;
                    continue;
                }
            }
            mismatches += 1;
            if mismatches <= 20 {
                eprintln!("MISMATCH a={sa:?} b={sb:?} field {i}: rust {g:?} python {e:?}");
            }
        }
    }
    println!("checked {checked} fields, {mismatches} mismatches, {order_only} order-only");
    if mismatches > 0 {
        std::process::exit(1);
    }
}
