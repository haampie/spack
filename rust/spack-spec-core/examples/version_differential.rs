// SPDX-License-Identifier: (Apache-2.0 OR MIT)

//! Differential checker: compares the version algebra against a TSV oracle generated from the
//! Python implementation. Each row is
//! `a<TAB>b<TAB>intersects<TAB>a_sat_b<TAB>b_sat_a<TAB>union<TAB>intersection<TAB>cmp`,
//! where boolean results are `True`/`False`, set results are `S:<str(result)>`, comparisons
//! are `LT`/`GT`/`EQ`/`NC`, and Python exceptions are `ERR`.
//!
//! Usage: `cargo run -p spack-spec-core --example version_differential <oracle.tsv>`

use std::cmp::Ordering;

use spack_spec_core::version::{from_string, VersionError, VersionUnion};

fn enc<T: std::fmt::Display>(r: Result<T, VersionError>) -> String {
    match r {
        Ok(v) => format!("S:{v}"),
        Err(_) => "ERR".to_string(),
    }
}

fn enc_bool(r: Result<bool, VersionError>) -> String {
    match r {
        Ok(true) => "True".to_string(),
        Ok(false) => "False".to_string(),
        Err(_) => "ERR".to_string(),
    }
}

fn enc_cmp(a: &VersionUnion, b: &VersionUnion) -> String {
    match a.compare(b) {
        Ok(Ordering::Less) => "LT".to_string(),
        Ok(Ordering::Greater) => "GT".to_string(),
        Ok(Ordering::Equal) => "EQ".to_string(),
        Err(_) => "ERR".to_string(),
    }
}

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: version_differential <oracle.tsv>");
    let data = std::fs::read_to_string(&path).expect("failed to read oracle");
    let (mut checked, mut mismatches) = (0u64, 0u64);
    for line in data.lines() {
        let fields: Vec<&str> = line.split('\t').collect();
        assert_eq!(fields.len(), 8, "malformed row: {line}");
        let (sa, sb) = (fields[0], fields[1]);
        let a = from_string(sa).expect("corpus strings parse");
        let b = from_string(sb).expect("corpus strings parse");
        let got = [
            enc_bool(a.intersects(&b)),
            enc_bool(a.satisfies(&b)),
            enc_bool(b.satisfies(&a)),
            enc(a.union(&b)),
            enc(a.intersection(&b)),
            enc_cmp(&a, &b),
        ];
        for (i, (g, e)) in got.iter().zip(&fields[2..]).enumerate() {
            checked += 1;
            if g != e {
                mismatches += 1;
                let op = [
                    "intersects",
                    "satisfies",
                    "rsatisfies",
                    "union",
                    "intersection",
                    "cmp",
                ][i];
                println!("MISMATCH {op}({sa:?}, {sb:?}): rust={g} python={e}");
            }
        }
    }
    println!("checked {checked} results, {mismatches} mismatches");
    if mismatches > 0 {
        std::process::exit(1);
    }
}
