// SPDX-License-Identifier: (Apache-2.0 OR MIT)

//! Reference-model tests for the target algebra, transcribed from
//! `lib/spack/spack/test/spec_algebra_targets.py`, plus golden cases whose expected strings
//! were generated from the Python implementation (`spack.spec._canonical_target_range` and
//! the `ArchSpec` target operations).
//!
//! The model of an expression is a pair (P, T): P the denoted subset of known names, T the set
//! of open lower-bound tails (with `""` for the unbounded tail of `:`), both recomputed here
//! from the vendored archspec JSON independently of the implementation.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::OnceLock;

use crate::targets::{target_concrete, TargetExpr, TargetGraph};

const JSON_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/spack/spack/vendor/archspec/json/cpu/microarchitectures.json"
);

/// Slice of the table the single-element expressions are built from (see the Python module).
const SLICE: [&str; 18] = [
    "x86_64",
    "x86_64_v2",
    "x86_64_v3",
    "x86_64_v4",
    "nocona",
    "core2",
    "nehalem",
    "sandybridge",
    "haswell",
    "skylake",
    "cannonlake",
    "icelake",
    "alderlake",
    "arrowlake",
    "aarch64",
    "neoverse_n1",
    "armv8.6a",
    "zen9",
];

/// Smaller core the two-element unions are built over.
const CORE: [&str; 8] = [
    "x86_64",
    "x86_64_v3",
    "x86_64_v4",
    "sandybridge",
    "icelake",
    "alderlake",
    "arrowlake",
    "zen9",
];

/// Bounded ranges whose bounds are incomparable or outside the table: they denote no target.
const DEGENERATE: [&str; 4] = [
    "icelake:alderlake",
    "nehalem:aarch64",
    "zen9:icelake",
    "icelake:zen9",
];

/// Stride for the union-times-union crossing, as in the Python module.
const UNION_STRIDE: usize = 16;
/// Stride for the pairs on which the greatest-lower-bound law quantifies over all singles.
const GLB_STRIDE: usize = 101;

/// The JSON records in archspec `TARGETS` insertion order: for every top-level name, parents
/// are recursively inserted first (`fill_target_from_dict`).
fn records() -> Vec<(String, Vec<String>, String)> {
    let text = std::fs::read_to_string(JSON_PATH).expect("vendored microarchitectures.json");
    let data: serde_json::Value = serde_json::from_str(&text).unwrap();
    let map = data["microarchitectures"].as_object().unwrap();
    let mut seen: HashSet<String> = HashSet::new();
    let mut result: Vec<(String, Vec<String>, String)> = Vec::new();
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

fn graph() -> &'static TargetGraph {
    static GRAPH: OnceLock<TargetGraph> = OnceLock::new();
    GRAPH.get_or_init(|| TargetGraph::from_records(records()).unwrap())
}

type Down = HashMap<String, BTreeSet<String>>;

/// For every name in the world, the names at or below it in the table order; a name outside
/// the table is its own singleton closure. Computed from the JSON, not from the graph.
fn down() -> &'static Down {
    static DOWN: OnceLock<Down> = OnceLock::new();
    DOWN.get_or_init(|| {
        let mut result: Down = HashMap::new();
        // records are in parents-first order, so closures of parents already exist
        for (name, parents, _) in records() {
            let mut closure: BTreeSet<String> = BTreeSet::new();
            closure.insert(name.clone());
            for parent in &parents {
                closure.extend(result[parent].iter().cloned());
            }
            result.insert(name, closure);
        }
        for name in SLICE {
            result
                .entry(name.to_string())
                .or_insert_with(|| BTreeSet::from([name.to_string()]));
        }
        result
    })
}

/// The world the model quantifies over: the whole table plus the names outside it the
/// expressions mention.
fn world() -> &'static BTreeSet<String> {
    static WORLD: OnceLock<BTreeSet<String>> = OnceLock::new();
    WORLD.get_or_init(|| down().keys().cloned().collect())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Denotation {
    points: BTreeSet<String>,
    tails: BTreeSet<String>,
    degenerate: bool,
}

fn partition(s: &str) -> (&str, bool, &str) {
    match s.find(':') {
        Some(i) => (&s[..i], true, &s[i + 1..]),
        None => (s, false, ""),
    }
}

/// The model denotation of a target expression, computed from the table alone.
fn denote(expr: &str) -> Denotation {
    let down = down();
    let world = world();
    let mut points: BTreeSet<String> = BTreeSet::new();
    let mut tails: BTreeSet<String> = BTreeSet::new();
    let mut degenerate = false;
    for element in expr.split(',') {
        let (lo, sep, hi) = if element == "*" {
            ("", true, "")
        } else {
            partition(element)
        };
        if !sep {
            points.insert(element.to_string());
        } else if lo.is_empty() && hi.is_empty() {
            points.extend(world.iter().cloned());
            tails.insert(String::new());
        } else if hi.is_empty() {
            tails.insert(lo.to_string());
            points.extend(world.iter().filter(|n| down[*n].contains(lo)).cloned());
        } else {
            let members: Vec<&String> = down[hi]
                .iter()
                .filter(|n| lo.is_empty() || down[*n].contains(lo))
                .collect();
            if members.is_empty() {
                degenerate = true;
            }
            points.extend(members.into_iter().cloned());
        }
    }
    let tails: BTreeSet<String> = if tails.contains("") {
        BTreeSet::from([String::new()])
    } else {
        // a tail contained in a lower one is redundant: the pair denotes the lower tail alone
        tails
            .iter()
            .filter(|x| {
                !tails
                    .iter()
                    .any(|y| y != *x && down[x.as_str()].contains(y))
            })
            .cloned()
            .collect()
    };
    Denotation {
        points,
        tails,
        degenerate,
    }
}

/// Whether every open tail on the left starts at or above an open tail on the right; the
/// unbounded tail of `:` starts below everything and only `:` itself covers it.
fn tails_covered(lhs: &BTreeSet<String>, rhs: &BTreeSet<String>) -> bool {
    lhs.iter().all(|x| {
        rhs.contains(x)
            || rhs
                .iter()
                .any(|y| y.is_empty() || (!x.is_empty() && down()[x.as_str()].contains(y)))
    })
}

fn subset(lhs: &Denotation, rhs: &Denotation) -> bool {
    lhs.points.is_subset(&rhs.points) && tails_covered(&lhs.tails, &rhs.tails)
}

#[derive(Debug, Clone)]
struct Value {
    expr: String,
    canon: TargetExpr,
    den: Denotation,
}

fn value(expr: &str) -> Value {
    Value {
        expr: expr.to_string(),
        canon: TargetExpr::parse(graph(), expr),
        den: denote(expr),
    }
}

/// The ordered pairs (lo, hi) with distinct comparable in-table bounds from `names`.
fn comparable_pairs(names: &[&str]) -> Vec<(String, String)> {
    let down = down();
    let mut result = Vec::new();
    for a in names {
        for b in names {
            if a != b && down[*b].contains(*a) {
                result.push((a.to_string(), b.to_string()));
            }
        }
    }
    result
}

/// Every single-element expression over `names`: points, open and bounded ranges.
fn elements(names: &[&str]) -> Vec<String> {
    let mut result: Vec<String> = names.iter().map(|n| n.to_string()).collect();
    result.extend(names.iter().map(|n| format!("{n}:")));
    result.extend(names.iter().map(|n| format!(":{n}")));
    result.extend(
        comparable_pairs(names)
            .into_iter()
            .map(|(lo, hi)| format!("{lo}:{hi}")),
    );
    result
}

struct Corpus {
    singles: Vec<Value>,
    unions: Vec<Value>,
    /// Pairs as (union?, index) references into `singles`/`unions`.
    pairs: Vec<((bool, usize), (bool, usize))>,
}

impl Corpus {
    fn get(&self, key: (bool, usize)) -> &Value {
        if key.0 {
            &self.unions[key.1]
        } else {
            &self.singles[key.1]
        }
    }
}

fn corpus() -> &'static Corpus {
    static CORPUS: OnceLock<Corpus> = OnceLock::new();
    CORPUS.get_or_init(|| {
        let mut single_exprs = elements(&SLICE);
        single_exprs.extend(DEGENERATE.iter().map(|s| s.to_string()));
        single_exprs.push(":".to_string());
        single_exprs.push("*".to_string());
        let singles: Vec<Value> = single_exprs.iter().map(|e| value(e)).collect();

        let core_elements = elements(&CORE);
        let unions: Vec<Value> = core_elements
            .iter()
            .enumerate()
            .flat_map(|(i, a)| {
                core_elements[i + 1..]
                    .iter()
                    .map(move |b| format!("{a},{b}"))
            })
            .map(|e| value(&e))
            .collect();

        let core_element_set: HashSet<&str> = core_elements
            .iter()
            .map(|s| s.as_str())
            .chain([":", "*"])
            .collect();
        let core_singles: Vec<usize> = singles
            .iter()
            .enumerate()
            .filter(|(_, v)| core_element_set.contains(v.expr.as_str()))
            .map(|(i, _)| i)
            .collect();

        let mut pairs: Vec<((bool, usize), (bool, usize))> = Vec::new();
        for a in 0..singles.len() {
            for b in 0..singles.len() {
                pairs.push(((false, a), (false, b)));
            }
        }
        for u in 0..unions.len() {
            for &s in &core_singles {
                pairs.push(((true, u), (false, s)));
            }
        }
        for u in 0..unions.len() {
            for &s in &core_singles {
                pairs.push(((false, s), (true, u)));
            }
        }
        for i in 0..unions.len() {
            for j in 0..unions.len() {
                if (i * unions.len() + j) % UNION_STRIDE == 0 {
                    pairs.push(((true, i), (true, j)));
                }
            }
        }
        Corpus {
            singles,
            unions,
            pairs,
        }
    })
}

fn fail(violations: Vec<String>) {
    assert!(
        violations.is_empty(),
        "{} violations, first 20:\n{}",
        violations.len(),
        violations[..violations.len().min(20)].join("\n")
    );
}

/// satisfies is the model subset: P(a) ⊆ P(b) with every open tail covered. The completeness
/// direction is skipped when the left side has an element denoting no target.
#[test]
fn satisfies_matches_the_model() {
    let g = graph();
    let c = corpus();
    let mut violations = Vec::new();
    for &(ka, kb) in &c.pairs {
        let (a, b) = (c.get(ka), c.get(kb));
        let actual = g.target_satisfies(Some(&a.canon), Some(&b.canon));
        let expected = subset(&a.den, &b.den);
        if actual && !expected {
            violations.push(format!(
                "{} satisfies {} but is not a subset",
                a.expr, b.expr
            ));
        } else if expected && !actual && !a.den.degenerate {
            violations.push(format!(
                "{} is a subset of {} but does not satisfy it",
                a.expr, b.expr
            ));
        }
    }
    fail(violations);
}

/// intersects is nonempty overlap of the denoted sets.
#[test]
fn intersects_matches_the_model() {
    let g = graph();
    let c = corpus();
    let mut violations = Vec::new();
    for &(ka, kb) in &c.pairs {
        let (a, b) = (c.get(ka), c.get(kb));
        let actual = g.target_intersects(Some(&a.canon), Some(&b.canon));
        let expected = a.den.points.intersection(&b.den.points).next().is_some();
        if actual != expected {
            violations.push(format!(
                "{} intersects {} is {actual}, overlap {expected}",
                a.expr, b.expr
            ));
        }
    }
    fail(violations);
}

/// constrain fails exactly on disjoint targets; otherwise the result denotes the intersection,
/// satisfies both sides, and is the same state in either argument order.
#[test]
fn constrain_matches_the_model() {
    let g = graph();
    let c = corpus();
    let mut violations = Vec::new();
    for &(ka, kb) in &c.pairs {
        let (a, b) = (c.get(ka), c.get(kb));
        let overlap: BTreeSet<&String> = a.den.points.intersection(&b.den.points).collect();
        let forward = g.target_constrain(Some(&a.canon), Some(&b.canon));
        let (result, _changed) = match forward {
            Err(_) => {
                if !overlap.is_empty() {
                    violations.push(format!(
                        "{} constrain {} fails on overlapping targets",
                        a.expr, b.expr
                    ));
                }
                continue;
            }
            Ok(r) => r,
        };
        if overlap.is_empty() {
            violations.push(format!(
                "{} constrain {} does not fail on disjoint targets",
                a.expr, b.expr
            ));
            continue;
        }
        let result = result.unwrap();
        let result_den = denote(result.as_str());
        let result_refs: BTreeSet<&String> = result_den.points.iter().collect();
        if result_refs != overlap {
            violations.push(format!(
                "({}) ∩ ({}) = {} denotes the wrong set",
                a.expr, b.expr, result
            ));
        }
        if !g.target_satisfies(Some(&result), Some(&a.canon))
            || !g.target_satisfies(Some(&result), Some(&b.canon))
        {
            violations.push(format!(
                "({}) ∩ ({}) = {} does not satisfy a side",
                a.expr, b.expr, result
            ));
        }
        let backward = g
            .target_constrain(Some(&b.canon), Some(&a.canon))
            .expect("reverse constrain of overlapping targets")
            .0
            .unwrap();
        if backward != result {
            violations.push(format!(
                "({}) ∩ ({}) = {result}, reversed {backward}",
                a.expr, b.expr
            ));
        }
    }
    fail(violations);
}

/// On a strided sample of pairs, every enumerated value below both sides in the model is below
/// the constrained result in the implementation.
#[test]
fn constrain_computes_a_greatest_lower_bound() {
    let g = graph();
    let c = corpus();
    let mut violations = Vec::new();
    for &(ka, kb) in c.pairs.iter().step_by(GLB_STRIDE) {
        let (a, b) = (c.get(ka), c.get(kb));
        if a.den.points.intersection(&b.den.points).next().is_none() {
            continue;
        }
        let Ok((Some(merged), _)) = g.target_constrain(Some(&a.canon), Some(&b.canon)) else {
            continue; // a failure on overlapping targets is the constrain law's violation
        };
        for v in &c.singles {
            if v.den.degenerate {
                continue;
            }
            if !subset(&v.den, &a.den) || !subset(&v.den, &b.den) {
                continue;
            }
            if !g.target_satisfies(Some(&v.canon), Some(&merged)) {
                violations.push(format!(
                    "{} is below {} and {} but not below their intersection {merged}",
                    v.expr, a.expr, b.expr
                ));
            }
        }
    }
    fail(violations);
}

/// Parsing stores a canonical form; it must denote the same set as the input expression, and
/// printing and reparsing it must reproduce the same state.
#[test]
fn construction_preserves_the_denotation() {
    let g = graph();
    let c = corpus();
    let mut violations = Vec::new();
    for v in c.singles.iter().chain(&c.unions) {
        let stored = denote(v.canon.as_str());
        if stored.points != v.den.points || stored.tails != v.den.tails {
            violations.push(format!(
                "{} is stored as {}, which denotes a different set",
                v.expr, v.canon
            ));
        }
        if TargetExpr::parse(g, v.canon.as_str()) != v.canon {
            violations.push(format!(
                "{} is stored as {}, which reparses to a different state",
                v.expr, v.canon
            ));
        }
    }
    fail(violations);
}

/// `target=*` and `target=:` denote every target and must behave as one state.
#[test]
fn the_universe_alias_star_is_the_state_colon() {
    let g = graph();
    let c = corpus();
    let star = TargetExpr::parse(g, "*");
    let colon = TargetExpr::parse(g, ":");
    assert!(g.target_satisfies(Some(&star), Some(&colon)));
    assert!(g.target_satisfies(Some(&colon), Some(&star)));
    let mut violations = Vec::new();
    if star.is_concrete() != colon.is_concrete() {
        violations.push("* and : differ in target_concrete".to_string());
    }
    for v in &c.singles {
        if g.target_satisfies(Some(&star), Some(&v.canon))
            != g.target_satisfies(Some(&colon), Some(&v.canon))
        {
            violations.push(format!("satisfies {} differs between * and :", v.expr));
        }
        if g.target_satisfies(Some(&v.canon), Some(&star))
            != g.target_satisfies(Some(&v.canon), Some(&colon))
        {
            violations.push(format!(
                "{} satisfies the universe differently between * and :",
                v.expr
            ));
        }
        if g.target_intersects(Some(&star), Some(&v.canon))
            != g.target_intersects(Some(&colon), Some(&v.canon))
        {
            violations.push(format!("intersects {} differs between * and :", v.expr));
        }
    }
    fail(violations);
}

/// Any target expression parses without panicking, and parsing is idempotent.
#[test]
fn construction_parses_and_is_idempotent() {
    let g = graph();
    for expr in [
        ":zen9,icelake",
        "zen9,icelake",
        ":zen9,:icelake",
        "zen9:,x86_64:",
        "zen9:zen9",
        ":",
        "*",
        "x86_64::icelake",
        ":,:",
        "x86_64:,",
        ",",
        "",
        "a:b:c",
    ] {
        let parsed = TargetExpr::parse(g, expr);
        assert_eq!(
            TargetExpr::parse(g, parsed.as_str()),
            parsed,
            "parsing {expr:?} is not idempotent"
        );
    }
}

// Golden cases: expected strings generated from the Python implementation.

#[test]
fn golden_canonical_strings() {
    let g = graph();
    for (input, expected) in [
        ("x86_64:icelake", ":icelake"),
        ("nocona:nehalem,nehalem:haswell", "nocona:haswell"),
        ("*", ":"),
        (":zen9", "zen9"),
        ("x86_64,aarch64", "aarch64,x86_64"),
        ("icelake:icelake", "icelake"),
        ("x86_64_v3:", "x86_64_v3:"),
        ("nocona:haswell,x86_64_v2", "nocona:haswell,x86_64_v2"),
        (":icelake,cannonlake:", ":cascadelake,cannonlake:"),
        ("zen9:icelake", "zen9:icelake"),
        ("icelake,alderlake", "alderlake,icelake"),
        (",", ""),
        ("x86_64:,", ",x86_64:"),
        ("x86_64::icelake", "x86_64::icelake"),
        (":,x86_64", ":"),
        ("x86_64_v3", "x86_64_v3"),
        (
            "skylake:cascadelake,cannonlake",
            "cannonlake,skylake:cascadelake",
        ),
        (":x86_64", "x86_64"),
        ("aarch64:neoverse_n1", ":neoverse_n1"),
        ("icelake:zen9,zen9:icelake", "icelake:zen9,zen9:icelake"),
    ] {
        assert_eq!(
            TargetExpr::parse(g, input).as_str(),
            expected,
            "canonical form of {input:?}"
        );
    }
}

#[test]
fn golden_intersections_and_constrain() {
    let g = graph();
    // (lhs, rhs, intersection elements, constrain result or None, changed)
    let cases: [(&str, &str, &[&str], Option<&str>, bool); 10] = [
        (
            "nocona:haswell",
            "x86_64:icelake",
            &["nocona:haswell"],
            Some("nocona:haswell"),
            false,
        ),
        (
            "x86_64_v4:",
            "nocona:",
            &["skylake_avx512:", "cannonlake:"],
            Some("cannonlake:,skylake_avx512:"),
            true,
        ),
        ("icelake", ":skylake", &[], None, false),
        (
            ":cannonlake",
            ":cascadelake",
            &[":x86_64_v4", ":skylake"],
            Some(":skylake,x86_64_v4"),
            true,
        ),
        (
            "x86_64,aarch64",
            ":",
            &["aarch64", "x86_64"],
            Some("aarch64,x86_64"),
            false,
        ),
        (
            ":haswell",
            "nocona:",
            &["nocona:haswell"],
            Some("nocona:haswell"),
            true,
        ),
        (
            "x86_64_v3",
            "x86_64:",
            &["x86_64_v3"],
            Some("x86_64_v3"),
            false,
        ),
        ("icelake", "aarch64", &[], None, false),
        ("zen9", "zen9:", &["zen9"], Some("zen9"), false),
        ("x86_64_v3:x86_64_v4", "nehalem:", &[], None, false),
    ];
    for (lhs, rhs, intersection, constrained, changed) in cases {
        let l = TargetExpr::parse(g, lhs);
        let r = TargetExpr::parse(g, rhs);
        assert_eq!(
            g.target_intersection(&l, &r),
            intersection,
            "intersection of {lhs:?} and {rhs:?}"
        );
        match constrained {
            None => assert!(
                g.target_constrain(Some(&l), Some(&r)).is_err(),
                "constrain of {lhs:?} by {rhs:?} should fail"
            ),
            Some(expected) => {
                let (result, got_changed) = g.target_constrain(Some(&l), Some(&r)).unwrap();
                assert_eq!(
                    result.unwrap().as_str(),
                    expected,
                    "{lhs:?} constrain {rhs:?}"
                );
                assert_eq!(
                    got_changed, changed,
                    "{lhs:?} constrain {rhs:?} changed flag"
                );
            }
        }
    }
}

#[test]
fn golden_satisfies() {
    let g = graph();
    for (lhs, rhs, expected) in [
        ("haswell", "nocona:icelake", true),
        ("nocona:haswell", "nocona:nehalem,westmere:haswell", true),
        (
            "nocona:haswell",
            "nocona:nehalem,sandybridge:haswell",
            false,
        ),
        ("x86_64:", ":", true),
        (":", "x86_64:", false),
        ("x86_64_v3", "x86_64", false),
        ("zen9", "zen9", true),
        ("zen9", ":", true),
        ("zen9:", ":", true),
        ("icelake,alderlake", ":icelake,alderlake:arrowlake", true),
        ("*", ":", true),
        ("x86_64", "x86_64_v2:", false),
    ] {
        let l = TargetExpr::parse(g, lhs);
        let r = TargetExpr::parse(g, rhs);
        assert_eq!(
            g.target_satisfies(Some(&l), Some(&r)),
            expected,
            "{lhs:?} satisfies {rhs:?}"
        );
    }
}

#[test]
fn none_and_concreteness_semantics() {
    let g = graph();
    let icelake = TargetExpr::parse(g, "icelake");
    let range = TargetExpr::parse(g, "nocona:haswell");
    // rhs None constrains nothing; lhs None satisfies nothing but None
    assert!(g.target_satisfies(Some(&icelake), None));
    assert!(g.target_satisfies(None, None));
    assert!(!g.target_satisfies(None, Some(&icelake)));
    assert!(g.target_intersects(None, Some(&icelake)));
    assert!(g.target_intersects(Some(&icelake), None));
    assert_eq!(
        g.target_constrain(Some(&icelake), None).unwrap(),
        (Some(icelake.clone()), false)
    );
    assert_eq!(
        g.target_constrain(None, Some(&range)).unwrap(),
        (Some(range.clone()), true)
    );
    assert_eq!(g.target_constrain(None, None).unwrap(), (None, false));
    // target_concrete: set, no range, no list
    assert!(target_concrete(Some(&icelake)));
    assert!(!target_concrete(Some(&range)));
    assert!(!target_concrete(Some(&TargetExpr::parse(g, "*"))));
    assert!(!target_concrete(Some(&TargetExpr::parse(
        g,
        "icelake,alderlake"
    ))));
    assert!(!target_concrete(None));
    // unknown names are concrete points
    assert!(target_concrete(Some(&TargetExpr::parse(g, "zen9"))));
}
