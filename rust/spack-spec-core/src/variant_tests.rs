// SPDX-License-Identifier: (Apache-2.0 OR MIT)

//! Tests transcribed from the pure algebra part of lib/spack/spack/test/variant.py
//! (the abstract/concrete satisfies/intersects/constrain matrix and patches rules).

use crate::variant::{Value, VariantData, VariantError, VariantKind};

fn multi(name: &str, values: &[&str], concrete: bool) -> VariantData {
    VariantData::new(
        VariantKind::Multi,
        name,
        values.iter().map(|v| Value::Str(v.to_string())).collect(),
        false,
        concrete,
    )
    .unwrap()
}

fn abstract_v(values: &[&str]) -> VariantData {
    multi("foo", values, false)
}

fn concrete_v(values: &[&str]) -> VariantData {
    multi("foo", values, true)
}

#[test]
fn satisfies_abstract_abstract() {
    assert!(abstract_v(&["bar"]).satisfies(&abstract_v(&["bar"])));
    assert!(abstract_v(&["bar", "baz"]).satisfies(&abstract_v(&["bar"])));
    assert!(abstract_v(&["bar", "baz"]).satisfies(&abstract_v(&["bar", "baz"])));
    assert!(!abstract_v(&["bar"]).satisfies(&abstract_v(&["baz"])));
    assert!(!abstract_v(&["bar"]).satisfies(&abstract_v(&["bar", "baz"])));
    assert!(abstract_v(&["bar"]).satisfies(&abstract_v(&[])));
    assert!(abstract_v(&[]).satisfies(&abstract_v(&[])));
    assert!(!abstract_v(&[]).satisfies(&abstract_v(&["bar"])));
}

#[test]
fn satisfies_concrete_abstract() {
    assert!(concrete_v(&["bar"]).satisfies(&abstract_v(&["bar"])));
    assert!(concrete_v(&["bar", "baz"]).satisfies(&abstract_v(&["bar"])));
    assert!(concrete_v(&["bar", "baz"]).satisfies(&abstract_v(&["bar", "baz"])));
    assert!(!concrete_v(&["bar"]).satisfies(&abstract_v(&["baz"])));
    assert!(!concrete_v(&["bar"]).satisfies(&abstract_v(&["bar", "baz"])));
    assert!(concrete_v(&["bar"]).satisfies(&abstract_v(&[])));
}

#[test]
fn satisfies_abstract_concrete() {
    assert!(!abstract_v(&["bar"]).satisfies(&concrete_v(&["bar"])));
    assert!(!abstract_v(&["bar", "baz"]).satisfies(&concrete_v(&["bar"])));
    assert!(!abstract_v(&["bar", "baz"]).satisfies(&concrete_v(&["bar", "baz"])));
    assert!(!abstract_v(&["bar"]).satisfies(&concrete_v(&["baz"])));
    assert!(!abstract_v(&["bar"]).satisfies(&concrete_v(&["bar", "baz"])));
    assert!(!abstract_v(&[]).satisfies(&concrete_v(&["bar"])));
}

#[test]
fn satisfies_concrete_concrete() {
    assert!(concrete_v(&["bar"]).satisfies(&concrete_v(&["bar"])));
    assert!(!concrete_v(&["bar", "baz"]).satisfies(&concrete_v(&["bar"])));
    assert!(!concrete_v(&["bar"]).satisfies(&concrete_v(&["bar", "baz"])));
    assert!(concrete_v(&["bar", "baz"]).satisfies(&concrete_v(&["bar", "baz"])));
}

#[test]
fn intersects_abstract_abstract() {
    assert!(abstract_v(&["bar"]).intersects(&abstract_v(&["bar"])));
    assert!(abstract_v(&["bar", "baz"]).intersects(&abstract_v(&["bar"])));
    assert!(abstract_v(&["bar"]).intersects(&abstract_v(&["baz"])));
    assert!(abstract_v(&["bar"]).intersects(&abstract_v(&["bar", "baz"])));
    assert!(abstract_v(&[]).intersects(&abstract_v(&["bar"])));
}

#[test]
fn intersects_concrete_abstract() {
    assert!(concrete_v(&["bar"]).intersects(&abstract_v(&["bar"])));
    assert!(concrete_v(&["bar", "baz"]).intersects(&abstract_v(&["bar"])));
    assert!(concrete_v(&["bar", "baz"]).intersects(&abstract_v(&["bar", "baz"])));
    assert!(!concrete_v(&["bar"]).intersects(&abstract_v(&["baz"])));
    assert!(!concrete_v(&["bar"]).intersects(&abstract_v(&["bar", "baz"])));
    assert!(concrete_v(&["bar"]).intersects(&abstract_v(&[])));
}

#[test]
fn intersects_abstract_concrete() {
    assert!(abstract_v(&["bar"]).intersects(&concrete_v(&["bar"])));
    assert!(!abstract_v(&["bar", "baz"]).intersects(&concrete_v(&["bar"])));
    assert!(abstract_v(&["bar", "baz"]).intersects(&concrete_v(&["bar", "baz"])));
    assert!(!abstract_v(&["bar"]).intersects(&concrete_v(&["baz"])));
    assert!(abstract_v(&["bar"]).intersects(&concrete_v(&["bar", "baz"])));
    assert!(abstract_v(&[]).intersects(&concrete_v(&["bar"])));
}

#[test]
fn intersects_concrete_concrete() {
    assert!(concrete_v(&["bar"]).intersects(&concrete_v(&["bar"])));
    assert!(!concrete_v(&["bar", "baz"]).intersects(&concrete_v(&["bar"])));
    assert!(!concrete_v(&["bar"]).intersects(&concrete_v(&["bar", "baz"])));
    assert!(concrete_v(&["bar", "baz"]).intersects(&concrete_v(&["bar", "baz"])));
}

#[test]
fn constrain_abstract_abstract() {
    let mut s1 = abstract_v(&["bar"]);
    assert!(s1.constrain(&abstract_v(&["baz"])).unwrap());
    assert_eq!(s1, abstract_v(&["bar", "baz"]));

    let mut s2 = abstract_v(&[]);
    assert!(s2.constrain(&abstract_v(&["baz"])).unwrap());
    assert_eq!(s2, abstract_v(&["baz"]));
}

#[test]
fn constrain_mixed_concreteness() {
    assert_eq!(
        abstract_v(&["bar"]).constrain(&concrete_v(&["baz"])),
        Err(VariantError::Unsatisfiable)
    );

    let mut s1 = abstract_v(&["bar"]);
    assert!(s1.constrain(&concrete_v(&["bar"])).unwrap()); // the change is concreteness
    assert_eq!(s1, concrete_v(&["bar"]));

    assert_eq!(
        concrete_v(&["bar"]).constrain(&concrete_v(&["bar", "baz"])),
        Err(VariantError::Unsatisfiable)
    );
    assert!(!concrete_v(&["bar"])
        .constrain(&concrete_v(&["bar"]))
        .unwrap());

    assert_eq!(
        concrete_v(&["bar"]).constrain(&abstract_v(&["baz"])),
        Err(VariantError::Unsatisfiable)
    );
    let mut s2 = concrete_v(&["bar", "baz"]);
    assert!(!s2.constrain(&abstract_v(&["bar"])).unwrap());
    assert!(!s2.constrain(&abstract_v(&[])).unwrap());
}

fn patches(values: &[&str], concrete: bool) -> VariantData {
    multi("patches", values, concrete)
}

#[test]
fn patches_prefix_satisfies() {
    assert!(patches(&["abcdef"], true).satisfies(&patches(&["ab"], false)));
    assert!(patches(&["abcdef"], true).satisfies(&patches(&["abcdef"], false)));
    assert!(!patches(&["abcdef"], true).satisfies(&patches(&["xyz"], false)));
    assert!(patches(&["abcdef", "xyz"], true).satisfies(&patches(&["xyz"], false)));
    assert!(!patches(&["abcdef"], true).satisfies(&patches(&["abcdefghi"], false)));

    // concrete rhs must match exactly
    assert!(patches(&["abcdef"], true).satisfies(&patches(&["abcdef"], true)));
    assert!(!patches(&["abcdef"], true).satisfies(&patches(&["ab"], true)));
    assert!(!patches(&["abcdef", "xyz"], true).satisfies(&patches(&["abc", "xyz"], true)));
    assert!(!patches(&["abcdef"], true).satisfies(&patches(&["abcdefghi"], true)));
}

#[test]
fn patches_prefix_intersects_and_constrains() {
    assert!(patches(&["abcdef"], true).intersects(&patches(&["ab"], false)));
    assert!(patches(&["ab"], false).intersects(&patches(&["abcdef"], true)));
    assert!(!patches(&["abcdef"], true).intersects(&patches(&["xyz"], false)));
    assert!(!patches(&["abcdef"], true).intersects(&patches(&["abcdefghi"], false)));

    let mut s = patches(&["abcdef"], true);
    assert!(!s.constrain(&patches(&["ab"], false)).unwrap());
    assert_eq!(s.values(), &[Value::Str("abcdef".into())]);

    let mut s = patches(&["ab"], false);
    assert!(s.constrain(&patches(&["abcdef"], true)).unwrap());
    assert_eq!(s.values(), &[Value::Str("abcdef".into())]);
}

#[test]
fn constrain_narrowing_to_bool() {
    let mut s = abstract_v(&[]);
    assert_eq!(s.kind, VariantKind::Multi);
    assert!(!s.concrete);
    let plus_foo = VariantData::new(
        VariantKind::Bool,
        "foo",
        vec![Value::Bool(true)],
        false,
        false,
    )
    .unwrap();
    assert!(s.constrain(&plus_foo).unwrap());
    assert_eq!(s.kind, VariantKind::Bool);
    assert!(s.concrete);
}

#[test]
fn render_forms() {
    let plus = VariantData::new(
        VariantKind::Bool,
        "foo",
        vec![Value::Bool(true)],
        false,
        false,
    )
    .unwrap();
    assert_eq!(plus.render(), "+foo");
    let tilde = VariantData::new(
        VariantKind::Bool,
        "foo",
        vec![Value::Bool(false)],
        true,
        false,
    )
    .unwrap();
    assert_eq!(tilde.render(), "~~foo");
    assert_eq!(abstract_v(&["bar", "baz"]).render(), "foo=bar,baz");
    assert_eq!(concrete_v(&["bar"]).render(), "foo:=bar");
    assert_eq!(abstract_v(&[]).render(), "foo='*'"); // quote_if_needed quotes the `*`
    let prop = VariantData::new(
        VariantKind::Multi,
        "foo",
        vec![Value::Str("bar".into())],
        true,
        false,
    )
    .unwrap();
    assert_eq!(prop.render(), "foo==bar");
    let spaced = VariantData::new(
        VariantKind::Single,
        "cflags",
        vec![Value::Str("-O2 -g".into())],
        false,
        false,
    )
    .unwrap();
    assert_eq!(spaced.render(), "cflags='-O2 -g'");
}

#[test]
fn set_sorts_and_dedups() {
    let v = multi("foo", &["c", "a", "b", "a"], false);
    assert_eq!(
        v.values(),
        &[
            Value::Str("a".into()),
            Value::Str("b".into()),
            Value::Str("c".into())
        ]
    );

    let single = VariantData::new(
        VariantKind::Single,
        "foo",
        vec![Value::Str("a".into()), Value::Str("b".into())],
        false,
        false,
    );
    assert_eq!(single.unwrap_err(), VariantError::MultipleValues);

    let bad_bool = VariantData::new(
        VariantKind::Bool,
        "foo",
        vec![Value::Str("bar".into())],
        false,
        false,
    );
    assert!(matches!(bad_bool, Err(VariantError::NonBooleanValue(_))));

    let star = VariantData::new(
        VariantKind::Multi,
        "foo",
        vec![Value::Str("*".into())],
        false,
        false,
    );
    assert_eq!(star.unwrap_err(), VariantError::ReservedStar);
}

#[test]
fn merged_values_patches_drops_proper_prefixes() {
    let lhs = patches(&["abcdef"], true);
    let rhs = patches(&["ab", "xyz"], false);
    let merged = lhs.merged_values(&rhs);
    assert_eq!(
        merged,
        vec![Value::Str("abcdef".into()), Value::Str("xyz".into())]
    );
}
