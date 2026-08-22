// SPDX-License-Identifier: (Apache-2.0 OR MIT)

//! Tests for the event-stream parser: the core-level rows of
//! `spec_syntax.py::test_error_conditions` (multiple versions, edge-attribute errors; the
//! duplicate-variant / architecture / propagation rows are `Spec._add_flag` errors that belong
//! to the binding), plus event streams for the constructs the binding replays.

use std::borrow::Cow;

use crate::parse::events::{
    parse_events, ParsingErrorKind, Propagation, SpecAction, SpecParseError, DEP_BUILD, DEP_LINK,
};

fn actions(input: &str) -> Vec<SpecAction<'_>> {
    parse_events(input)
        .unwrap_or_else(|e| panic!("parse error on {input:?}: {e:?}"))
        .into_iter()
        .map(|e| e.action)
        .collect()
}

fn parse_error_kind(input: &str) -> ParsingErrorKind {
    match parse_events(input) {
        Err(SpecParseError::Parsing(e)) => e.kind,
        other => panic!("expected a parsing error for {input:?}, got {other:?}"),
    }
}

/// Multiple-version rows of `test_error_conditions`.
#[test]
fn test_error_conditions_multiple_versions() {
    for input in [
        "x@1.2@2.3",
        "x@1.2:2.3@1.4",
        "x@1.2@2.3:2.4",
        "x@1.2@2.3,2.4",
        "x@1.2 +foo~bar @2.3",
        "x@1.2%y@1.2@2.3:2.4",
    ] {
        match parse_events(input) {
            Err(SpecParseError::Parsing(e)) => {
                assert_eq!(e.kind, ParsingErrorKind::MultipleVersions, "{input:?}");
                assert_eq!(e.message, "Spec cannot have multiple versions");
                assert!(e.token.is_some());
            }
            other => panic!("expected multiple-version error for {input:?}, got {other:?}"),
        }
    }
}

/// Edge-attribute rows of `test_error_conditions`.
#[test]
fn test_error_conditions_edge_attributes() {
    // `@foo` is not a key-value pair, so it cannot appear between brackets.
    assert_eq!(
        parse_error_kind("^[@foo] zlib"),
        ParsingErrorKind::UnexpectedTokenInEdgeAttributes
    );
    // Only deptypes/virtuals/when are accepted.
    assert_eq!(
        parse_error_kind("^[foo=bar] zlib"),
        ParsingErrorKind::InvalidEdgeAttribute
    );
    // Unterminated bracket: Python points at a `None` token.
    match parse_events("^[virtuals=c") {
        Err(SpecParseError::Parsing(e)) => {
            assert_eq!(e.kind, ParsingErrorKind::UnexpectedTokenInEdgeAttributes);
            assert!(e.token.is_none());
        }
        other => panic!("expected edge-attribute error, got {other:?}"),
    }
    // Bad deptypes element: a bare ValueError in Python, no token context.
    match parse_events("^[deptypes=foo] zlib") {
        Err(SpecParseError::Parsing(e)) => {
            assert_eq!(e.kind, ParsingErrorKind::InvalidDeptype);
            assert_eq!(e.message, "Invalid dependency type: foo");
            assert!(e.token.is_none());
        }
        other => panic!("expected deptype error, got {other:?}"),
    }
}

/// Rows of `test_error_conditions` that are *binding-level* (`Spec._add_flag`) errors: the core
/// parser produces an event stream and the binding raises while replaying it.
#[test]
fn test_binding_level_error_rows_still_parse() {
    for input in [
        "x@1.2+debug+debug",
        "x ^y@1.2+debug debug=true",
        "x arch=linux-rhel7-x86_64 arch=linux-rhel7-x86_64",
        "x os=debian6 os=redhat6",
        "x namespace==foo.bar.baz",
        "x target==x86_64",
        "x patches==abcde12345,12345abcde",
    ] {
        assert!(
            parse_events(input).is_ok(),
            "{input:?} should parse into events"
        );
    }
}

/// Python's `all_specs` loops forever on tokens no rule consumes; the Rust parser reports them.
#[test]
fn test_no_progress_instead_of_hang() {
    assert_eq!(parse_error_kind("]"), ParsingErrorKind::NoProgress);
    assert_eq!(parse_error_kind("x ] y"), ParsingErrorKind::NoProgress);
}

/// Tokenization errors surface with the laziness of the Python token stream: the parser has one
/// token of lookahead, so a parse error two or more tokens before the bad character still wins.
#[test]
fn test_lazy_tokenization_error_ordering() {
    match parse_events("x@1.2@2.3 =") {
        Err(SpecParseError::Tokenization(e)) => assert_eq!(e.input, "x@1.2@2.3 ="),
        other => panic!("expected tokenization error, got {other:?}"),
    }
    // The duplicate version is detected before the `=` is pulled from the lexer.
    assert_eq!(
        parse_error_kind("x@1.2@2.3 y ="),
        ParsingErrorKind::MultipleVersions
    );
}

#[test]
fn test_events_full_spec() {
    use SpecAction as A;
    assert_eq!(
        actions("mvapich_foo ^_openmpi@1.2:1.4,1.6+debug~qt_4 %intel@12.1 ^stackwalker@8.1_1e"),
        vec![
            A::StartRootNode,
            A::NodeName {
                name: "mvapich_foo",
                namespace: None
            },
            A::StartDependencyNode {
                direct: false,
                propagation: Propagation::None,
                virtuals: vec![]
            },
            A::NodeName {
                name: "_openmpi",
                namespace: None
            },
            A::NodeVersion {
                version: "1.2:1.4,1.6"
            },
            A::BoolVariant {
                name: "debug",
                value: true,
                propagate: false
            },
            A::BoolVariant {
                name: "qt_4",
                value: false,
                propagate: false
            },
            A::StartDependencyNode {
                direct: true,
                propagation: Propagation::None,
                virtuals: vec![]
            },
            A::NodeName {
                name: "intel-oneapi-compilers-classic",
                namespace: None
            },
            A::NodeVersion { version: "12.1" },
            A::StartDependencyNode {
                direct: false,
                propagation: Propagation::None,
                virtuals: vec![]
            },
            A::NodeName {
                name: "stackwalker",
                namespace: None
            },
            A::NodeVersion { version: "8.1_1e" },
        ]
    );
}

#[test]
fn test_events_virtual_assignment_pushback() {
    use SpecAction as A;
    // The substituted package of `%c,cxx=gcc` becomes the dependency's name token.
    assert_eq!(
        actions("zlib %c,cxx=gcc@14.1"),
        vec![
            A::StartRootNode,
            A::NodeName {
                name: "zlib",
                namespace: None
            },
            A::StartDependencyNode {
                direct: true,
                propagation: Propagation::None,
                virtuals: vec!["c", "cxx"]
            },
            A::NodeName {
                name: "gcc",
                namespace: None
            },
            A::NodeVersion { version: "14.1" },
        ]
    );
    // Dotted substitutes become fully qualified names.
    assert_eq!(
        actions("^mpi=builtin.openmpi"),
        vec![
            A::StartRootNode,
            A::StartDependencyNode {
                direct: false,
                propagation: Propagation::None,
                virtuals: vec!["mpi"]
            },
            A::NodeName {
                name: "openmpi",
                namespace: Some("builtin")
            },
        ]
    );
}

#[test]
fn test_events_legacy_compiler_alias_on_direct_edges_only() {
    use SpecAction as A;
    // `%clang` is rewritten to llvm (the formatter maps it back for display)...
    assert_eq!(
        actions("zlib %[virtuals=c,cxx] clang"),
        vec![
            A::StartRootNode,
            A::NodeName {
                name: "zlib",
                namespace: None
            },
            A::StartEdgeProperties {
                direct: true,
                propagation: Propagation::None
            },
            A::EndEdgeProperties {
                virtuals: vec!["c", "cxx"]
            },
            A::NodeName {
                name: "llvm",
                namespace: None
            },
        ]
    );
    // ... but a root or `^` node named clang is not.
    assert_eq!(
        actions("clang ^intel"),
        vec![
            A::StartRootNode,
            A::NodeName {
                name: "clang",
                namespace: None
            },
            A::StartDependencyNode {
                direct: false,
                propagation: Propagation::None,
                virtuals: vec![]
            },
            A::NodeName {
                name: "intel",
                namespace: None
            },
        ]
    );
}

#[test]
fn test_events_edge_properties() {
    use SpecAction as A;
    assert_eq!(
        actions("foo ^[when='%c' deptypes=link,build] c=gcc"),
        vec![
            A::StartRootNode,
            A::NodeName {
                name: "foo",
                namespace: None
            },
            A::StartEdgeProperties {
                direct: false,
                propagation: Propagation::None
            },
            A::EdgeDeptypes {
                depflag: DEP_BUILD | DEP_LINK
            },
            A::EdgeWhen { when: "%c" },
            A::EndEdgeProperties {
                virtuals: vec!["c"]
            },
            A::NodeName {
                name: "gcc",
                namespace: None
            },
        ]
    );
    // A repeated key keeps the last value, and fused virtuals are appended to `virtuals=`.
    assert_eq!(
        actions("^[virtuals=a virtuals=b,c] d=e"),
        vec![
            A::StartRootNode,
            A::StartEdgeProperties {
                direct: false,
                propagation: Propagation::None
            },
            A::EndEdgeProperties {
                virtuals: vec!["b", "c", "d"]
            },
            A::NodeName {
                name: "e",
                namespace: None
            },
        ]
    );
}

#[test]
fn test_events_propagated_dependency_and_variants() {
    use SpecAction as A;
    assert_eq!(
        actions("foo %%gcc ++debug languages:=='c,c++'"),
        vec![
            A::StartRootNode,
            A::NodeName {
                name: "foo",
                namespace: None
            },
            A::StartDependencyNode {
                direct: true,
                propagation: Propagation::Preference,
                virtuals: vec![]
            },
            A::NodeName {
                name: "gcc",
                namespace: None
            },
            A::BoolVariant {
                name: "debug",
                value: true,
                propagate: true
            },
            A::KeyValuePair {
                name: "languages",
                value: Cow::Borrowed("c,c++"),
                propagate: true,
                concrete: true,
            },
        ]
    );
}

#[test]
fn test_events_second_hash_starts_next_spec() {
    use SpecAction as A;
    assert_eq!(
        actions("/abc /def zlib"),
        vec![
            A::StartRootNode,
            A::DagHash { hash: "abc" },
            A::StartRootNode,
            A::DagHash { hash: "def" },
            A::StartRootNode,
            A::NodeName {
                name: "zlib",
                namespace: None
            },
        ]
    );
}

#[test]
fn test_events_star_and_anonymous() {
    use SpecAction as A;
    assert_eq!(
        actions("* ^*+foo"),
        vec![
            A::StartRootNode,
            A::Star,
            A::StartDependencyNode {
                direct: false,
                propagation: Propagation::None,
                virtuals: vec![]
            },
            A::Star,
            A::BoolVariant {
                name: "foo",
                value: true,
                propagate: false
            },
        ]
    );
    assert_eq!(actions(""), vec![]);
}

#[test]
fn test_events_filename_ends_node() {
    use SpecAction as A;
    // Node options after a spec file belong to the next spec (`FileParser.parse` returns).
    assert_eq!(
        actions("./libelf.yaml @1.2"),
        vec![
            A::StartRootNode,
            A::Filename {
                path: "./libelf.yaml"
            },
            A::StartRootNode,
            A::NodeVersion { version: "1.2" },
        ]
    );
}

#[test]
fn test_events_unquote_and_unescape_values() {
    use SpecAction as A;
    assert_eq!(
        actions(r#"cflags=="-O3 -g" foo='a\'b'"#),
        vec![
            A::StartRootNode,
            A::KeyValuePair {
                name: "cflags",
                value: Cow::Borrowed("-O3 -g"),
                propagate: true,
                concrete: false,
            },
            A::KeyValuePair {
                name: "foo",
                value: Cow::Owned("a'b".to_string()),
                propagate: false,
                concrete: false,
            },
        ]
    );
}
