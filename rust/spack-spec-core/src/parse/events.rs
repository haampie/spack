// SPDX-License-Identifier: (Apache-2.0 OR MIT)

//! Event-stream spec parser mirroring `spack.spec_parser.SpecParser` / `SpecNodeParser` /
//! `EdgeAttributeParser`. See the module docs in `parse/mod.rs` for the event contract and the
//! Python-to-Rust mapping.

use std::borrow::Cow;

use crate::parse::lexer::{Lexer, SpecTokenizationError, Token, TokenKind};
use crate::strutil::strip_quotes_and_unescape;

/// `spack.deptypes` flag values.
pub const DEP_LINK: u32 = 0b0001;
pub const DEP_RUN: u32 = 0b0010;
pub const DEP_BUILD: u32 = 0b0100;
pub const DEP_TEST: u32 = 0b1000;

/// `spack.deptypes.flag_from_string` over a `,`-split deptypes value. Elements are not trimmed,
/// matching Python (`deptypes=link, build` fails on `" build"`).
pub fn canonicalize_deptypes<'s>(parts: impl Iterator<Item = &'s str>) -> Result<u32, String> {
    let mut flag = 0;
    for part in parts {
        flag |= match part {
            "build" => DEP_BUILD,
            "link" => DEP_LINK,
            "run" => DEP_RUN,
            "test" => DEP_TEST,
            other => return Err(format!("Invalid dependency type: {other}")),
        };
    }
    Ok(flag)
}

/// `spack.enums.PropagationPolicy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Propagation {
    None,
    Preference,
}

/// Character span of the token an event came from (Python `Token.start`/`Token.end`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    fn of(token: &Token<'_>) -> Span {
        Span {
            start: token.start,
            end: token.end,
        }
    }
}

/// One event with the span of the token that produced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecEvent<'a> {
    pub action: SpecAction<'a>,
    pub span: Span,
}

/// The semantic actions of the Python parser, in parse order. See `parse/mod.rs` for how a
/// binding replays these into a Spec DAG.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpecAction<'a> {
    /// Begin the root node of the next spec (`SpecParser.next_spec`). The span is zero-width at
    /// the spec's first token.
    StartRootNode,
    /// Begin a dependency node introduced by a bare `^`, `%` or `%%` sigil, with the virtuals of
    /// an inline assignment such as `^mpi=openmpi` (the substituted package arrives as the
    /// following `NodeName`). Attach semantics: `direct` edges attach to the node the last `^`
    /// introduced (or the root); `^` edges attach to the root, but only once their trailing `%`
    /// edges are parsed (`pending` in `SpecParser.next_spec`).
    StartDependencyNode {
        direct: bool,
        propagation: Propagation,
        virtuals: Vec<&'a str>,
    },
    /// Begin a dependency edge written as `^[...]`, `%[...]` or `%%[...]`.
    StartEdgeProperties {
        direct: bool,
        propagation: Propagation,
    },
    /// `deptypes=` inside edge properties, canonicalized to a depflag. At most one per edge; a
    /// repeated key keeps the last value, like the Python attribute dict.
    EdgeDeptypes { depflag: u32 },
    /// `when=` inside edge properties: the raw spec string (quotes and spaces stripped), to be
    /// parsed recursively by the binding, exactly like `parse_one_or_raise(attributes["when"])`.
    /// At most one per edge; a repeated key keeps the last value.
    EdgeWhen { when: &'a str },
    /// Closing `]` of edge properties; begins the dependency node. `virtuals` is the final edge
    /// virtuals tuple: the last `virtuals=` value split on `,`, plus the virtuals of a fused
    /// assignment such as `] c=gcc` appended (whose package arrives as the following
    /// `NodeName`).
    EndEdgeProperties { virtuals: Vec<&'a str> },
    /// Package name (and namespace for `builtin.name` forms) of the current node. For direct
    /// dependencies the legacy compiler aliases are already applied (`clang` -> `llvm`, ...).
    NodeName {
        name: &'a str,
        namespace: Option<&'a str>,
    },
    /// The dependency node introduced by the last sigil is complete. This is the point where
    /// `SpecParser.next_spec` runs its concrete-root check, rewrites legacy compiler aliases and
    /// attaches the edge (or defers a `^` edge into `pending`) — before the loop pulls another
    /// token, which is what makes an attach error beat a tokenization error later in the input.
    /// The span is the node's last token, i.e. `ctx.current_token` where Python attaches.
    EndDependencyNode,
    /// `*`: the current node is anonymous (Python leaves the name unset).
    Star,
    /// `@...` version list, git version, or `git-ref=version` pair: the token value without the
    /// leading `@`, whitespace preserved, to be fed to `spack.version.from_string`.
    NodeVersion { version: &'a str },
    /// `+name`, `~name`, `-name` and the `++`/`~~`/`--` propagated forms.
    /// Python: `Spec._add_flag(name, value, propagate, concrete=True)`.
    BoolVariant {
        name: &'a str,
        value: bool,
        propagate: bool,
    },
    /// `key=value`, `key==value`, `key:=value`, `key:==value` on a node; the value is unquoted
    /// and unescaped. Python: `Spec._add_flag(name, value, propagate, concrete)`.
    KeyValuePair {
        name: &'a str,
        value: Cow<'a, str>,
        propagate: bool,
        concrete: bool,
    },
    /// `/hash`: the abstract hash without the slash. At most one per node; a second `/hash`
    /// starts the next spec.
    DagHash { hash: &'a str },
    /// A `.json`/`.yaml` spec file path; the node has no further attributes. The binding loads
    /// the file like `FileParser.parse` (and must reject dependents of a concrete spec like
    /// `SpecParser._parse_node`).
    Filename { path: &'a str },
}

/// Kinds of core parse errors (binding-level errors like duplicate variants are not here: they
/// are raised when replaying events).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParsingErrorKind {
    /// `Spec cannot have multiple versions`
    MultipleVersions,
    /// A key other than deptypes/virtuals/when inside `[...]`.
    InvalidEdgeAttribute,
    /// A token other than `key=value` or `]` inside `[...]`.
    UnexpectedTokenInEdgeAttributes,
    /// Invalid `deptypes=` element. Python raises a bare `ValueError` here, with no token
    /// underline.
    InvalidDeptype,
    /// A token no rule consumes at spec level (e.g. a stray `]`). Python's `all_specs` loops
    /// forever on these; the Rust parser reports them instead.
    NoProgress,
}

/// Mirror of `SpecParsingError`: message plus the offending token (`None` mirrors Python passing
/// a `None` token, e.g. at end of input).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsingError<'a> {
    pub kind: ParsingErrorKind,
    pub message: String,
    pub token: Option<Token<'a>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpecParseError<'a> {
    /// Unexpected characters in the input (Python `SpecTokenizationError`).
    Tokenization(SpecTokenizationError<'a>),
    /// A syntax error at token level (Python `SpecParsingError`, plus `NoProgress`).
    Parsing(ParsingError<'a>),
}

impl<'a> From<SpecTokenizationError<'a>> for SpecParseError<'a> {
    fn from(e: SpecTokenizationError<'a>) -> Self {
        SpecParseError::Tokenization(e)
    }
}

impl<'a> From<ParsingError<'a>> for SpecParseError<'a> {
    fn from(e: ParsingError<'a>) -> Self {
        SpecParseError::Parsing(e)
    }
}

/// `spack.aliases.LEGACY_COMPILER_TO_BUILTIN`, applied to direct dependency names.
pub fn legacy_compiler_to_builtin(name: &str) -> Option<&'static str> {
    match name {
        "clang" => Some("llvm"),
        "oneapi" => Some("intel-oneapi-compilers"),
        "rocmcc" => Some("llvm-amdgpu"),
        "intel" => Some("intel-oneapi-compilers-classic"),
        "arm" => Some("acfl"),
        _ => None,
    }
}

/// Parse every spec in `input` into a flat event stream; each spec starts with `StartRootNode`.
/// This is `SpecParser(input).all_specs()` up to (but not including) Spec construction.
pub fn parse_events(input: &str) -> Result<Vec<SpecEvent<'_>>, SpecParseError<'_>> {
    let mut ctx = Ctx::new(input)?;
    let mut events = Vec::new();
    while ctx.next.is_some() {
        let before = ctx.consumed;
        next_spec(&mut ctx, &mut events)?;
        if ctx.consumed == before {
            // PY: `all_specs` keeps calling `next_spec`, which consumes nothing and returns an
            // empty spec forever. Reporting is deliberate divergence from that hang.
            return Err(ParsingError {
                kind: ParsingErrorKind::NoProgress,
                message: "unexpected token; the spec parser cannot make progress".to_string(),
                token: ctx.next,
            }
            .into());
        }
    }
    Ok(events)
}

/// The first spec in `input` as an event stream, plus, on success, the one-token lookahead
/// left behind (`SpecParser.ctx.next_token` after `next_spec`), which `parse_one_or_raise`
/// uses to reject trailing text. Tokens past the lookahead are never pulled, so an error there
/// stays silent, exactly like the lazy Python tokenizer. Empty input yields no events, like
/// `next_spec` returning its `initial_spec` untouched.
///
/// On error the events up to the offending token are returned with it: the parser and the
/// Python original emit semantic actions in the same token order, so a binding that replays
/// the partial stream before raising reproduces Python's interleaving of `Spec` mutation
/// errors with parse errors (`x+debug+debug @1@2` is a duplicate-variant error, not a
/// multiple-versions error). The partial stream ends mid-spec: the binding must not run its
/// end-of-spec attachments before raising.
pub fn parse_events_one(
    input: &str,
) -> (
    Vec<SpecEvent<'_>>,
    Result<Option<Token<'_>>, SpecParseError<'_>>,
) {
    let mut events = Vec::new();
    let mut ctx = match Ctx::new(input) {
        Ok(ctx) => ctx,
        Err(e) => return (events, Err(e.into())),
    };
    if ctx.next.is_some() {
        if let Err(e) = next_spec(&mut ctx, &mut events) {
            return (events, Err(e));
        }
    }
    (events, Ok(ctx.next))
}

/// `TokenContext`: one token of lookahead over the whitespace-filtered stream, with a pushback
/// slot for the substituted package of a virtual assignment.
struct Ctx<'a> {
    input: &'a str,
    lexer: Lexer<'a>,
    /// Last accepted token (`current_token`).
    curr: Option<Token<'a>>,
    /// Next token to be accepted (`next_token`).
    next: Option<Token<'a>>,
    /// Pushed-back tokens; the back of the list is the front of the stream (`pushed_tokens`).
    pushed: Vec<Option<Token<'a>>>,
    /// Number of `advance` calls, to detect a stalled parse.
    consumed: usize,
}

impl<'a> Ctx<'a> {
    fn new(input: &'a str) -> Result<Self, SpecTokenizationError<'a>> {
        let mut ctx = Ctx {
            input,
            lexer: Lexer::new(input),
            curr: None,
            next: None,
            pushed: Vec::new(),
            consumed: 0,
        };
        // PY: TokenContext.__init__ advances once, so a bad first token fails construction.
        ctx.advance()?;
        ctx.consumed = 0;
        Ok(ctx)
    }

    /// Next non-whitespace token from the lexer; an `UNEXPECTED` token aborts, carrying the full
    /// re-tokenization of the input like `spack.spec_parser.tokenize`.
    fn pull(&mut self) -> Result<Option<Token<'a>>, SpecTokenizationError<'a>> {
        loop {
            match self.lexer.next() {
                None => return Ok(None),
                Some(t) if t.kind == TokenKind::Ws => continue,
                Some(t) if t.kind == TokenKind::Unexpected => {
                    return Err(SpecTokenizationError {
                        tokens: crate::parse::lexer::tokenize_all(self.input),
                        input: self.input,
                    })
                }
                Some(t) => return Ok(Some(t)),
            }
        }
    }

    fn advance(&mut self) -> Result<(), SpecTokenizationError<'a>> {
        self.curr = self.next;
        self.next = match self.pushed.pop() {
            Some(t) => t,
            None => self.pull()?,
        };
        self.consumed += 1;
        Ok(())
    }

    fn accept(&mut self, kind: TokenKind) -> Result<bool, SpecTokenizationError<'a>> {
        if self.next.map(|t| t.kind) == Some(kind) {
            self.advance()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// `TokenContext.push_front`: the pushed token becomes `next`, the old `next` goes behind it.
    fn push_front(&mut self, token: Token<'a>) {
        self.pushed.push(self.next.take());
        self.next = Some(token);
    }

    /// Unwraps `curr` right after a successful `accept`.
    fn current(&self) -> Token<'a> {
        self.curr.expect("current token after accept")
    }
}

/// `SpecParser.next_spec`, emitting events instead of building a Spec.
fn next_spec<'a>(
    ctx: &mut Ctx<'a>,
    events: &mut Vec<SpecEvent<'a>>,
) -> Result<(), SpecParseError<'a>> {
    let first = ctx.next.expect("next_spec requires a pending token");
    events.push(SpecEvent {
        action: SpecAction::StartRootNode,
        span: Span {
            start: first.start,
            end: first.start,
        },
    });
    parse_node(ctx, events, false)?;
    loop {
        let has_edge_attrs = if ctx.accept(TokenKind::StartEdgeProperties)? {
            true
        } else if ctx.accept(TokenKind::Dependency)? {
            false
        } else {
            break;
        };
        let token = ctx.current();
        let direct = token.value.starts_with('%');
        let propagation = if token.value.starts_with("%%") {
            Propagation::Preference
        } else {
            Propagation::None
        };

        if has_edge_attrs {
            events.push(SpecEvent {
                action: SpecAction::StartEdgeProperties {
                    direct,
                    propagation,
                },
                span: Span::of(&token),
            });
            parse_edge_attributes(ctx, events)?;
        } else {
            let virtuals = push_virtual_assignment(ctx, &token);
            events.push(SpecEvent {
                action: SpecAction::StartDependencyNode {
                    direct,
                    propagation,
                    virtuals,
                },
                span: Span::of(&token),
            });
        }
        parse_node(ctx, events, direct)?;
        events.push(SpecEvent {
            action: SpecAction::EndDependencyNode,
            span: Span::of(&ctx.current()),
        });
    }
    Ok(())
}

/// `parse_virtual_assignment`: split the virtuals of a `DEPENDENCY`/`END_EDGE_PROPERTIES` token
/// and push a package-name token for the substitute back onto the stream.
fn push_virtual_assignment<'a>(ctx: &mut Ctx<'a>, token: &Token<'a>) -> Vec<&'a str> {
    let (virtuals, substitute) = match (token.virtuals, token.substitute) {
        (Some(v), Some(s)) => (v, s),
        _ => return Vec::new(),
    };
    // PY: the pushed token's span is relative to the token value (`value.index(pkg)`), found at
    // the first occurrence of the substitute string.
    let byte_idx = token
        .value
        .find(substitute)
        .expect("substitute is a substring of the token");
    let start = token.value[..byte_idx].chars().count();
    let kind = if substitute.contains('.') {
        TokenKind::FullyQualifiedPackageName
    } else {
        TokenKind::UnqualifiedPackageName
    };
    ctx.push_front(Token {
        kind,
        value: substitute,
        start,
        end: start + substitute.chars().count(),
        virtuals: None,
        substitute: None,
    });
    virtuals.split(',').collect()
}

/// `EdgeAttributeParser.parse`. Consumes up to and including the closing `]`, then emits
/// `EdgeDeptypes`/`EdgeWhen` (if present) followed by `EndEdgeProperties`.
fn parse_edge_attributes<'a>(
    ctx: &mut Ctx<'a>,
    events: &mut Vec<SpecEvent<'a>>,
) -> Result<(), SpecParseError<'a>> {
    // PY: a dict keyed by attribute name, so a repeated key keeps the last value.
    let mut deptypes: Option<(&'a str, Span)> = None;
    let mut virtuals: Option<(&'a str, Span)> = None;
    let mut when: Option<(&'a str, Span)> = None;
    let end_token;
    loop {
        if ctx.accept(TokenKind::KeyValuePair)? {
            let token = ctx.current();
            let (mut name, value) = token.value.split_once('=').expect("KVP contains =");
            name = name.strip_suffix(':').unwrap_or(name);
            // PY: `value.strip("'\" ")`, not strip_quotes_and_unescape.
            let value = value.trim_matches(['\'', '"', ' ']);
            let slot = match name {
                "deptypes" => &mut deptypes,
                "virtuals" => &mut virtuals,
                "when" => &mut when,
                _ => {
                    return Err(ParsingError {
                        kind: ParsingErrorKind::InvalidEdgeAttribute,
                        message: "the only edge attributes that are currently accepted \
                                  are \"deptypes\", \"virtuals\", and \"when\""
                            .to_string(),
                        token: Some(token),
                    }
                    .into())
                }
            };
            *slot = Some((value, Span::of(&token)));
        } else if ctx.accept(TokenKind::EndEdgeProperties)? {
            end_token = ctx.current();
            break;
        } else {
            return Err(ParsingError {
                kind: ParsingErrorKind::UnexpectedTokenInEdgeAttributes,
                message: "unexpected token in edge attributes".to_string(),
                token: ctx.next,
            }
            .into());
        }
    }

    if let Some((value, span)) = deptypes {
        let depflag = canonicalize_deptypes(value.split(',')).map_err(|message| ParsingError {
            kind: ParsingErrorKind::InvalidDeptype,
            message,
            token: None, // PY: a bare ValueError, no token context
        })?;
        events.push(SpecEvent {
            action: SpecAction::EdgeDeptypes { depflag },
            span,
        });
    }
    if let Some((value, span)) = when {
        events.push(SpecEvent {
            action: SpecAction::EdgeWhen { when: value },
            span,
        });
    }
    // PY: `virtuals = attributes.get("virtuals", ()) + parse_virtual_assignment(...)`.
    let mut all_virtuals: Vec<&'a str> = virtuals
        .map(|(value, _)| value.split(',').collect())
        .unwrap_or_default();
    all_virtuals.extend(push_virtual_assignment(ctx, &end_token));
    events.push(SpecEvent {
        action: SpecAction::EndEdgeProperties {
            virtuals: all_virtuals,
        },
        span: Span::of(&end_token),
    });
    Ok(())
}

/// `SpecNodeParser.parse` plus `FileParser.parse`, emitting events. `direct_edge` selects the
/// legacy compiler alias rewrite that `SpecParser.next_spec` applies to `%` dependency names.
fn parse_node<'a>(
    ctx: &mut Ctx<'a>,
    events: &mut Vec<SpecEvent<'a>>,
    direct_edge: bool,
) -> Result<(), SpecParseError<'a>> {
    let alias = |name: &'a str| -> &'a str {
        if direct_edge {
            legacy_compiler_to_builtin(name).unwrap_or(name)
        } else {
            name
        }
    };

    match ctx.next {
        None => return Ok(()),
        Some(t) if t.kind == TokenKind::Dependency => return Ok(()),
        _ => {}
    }

    if ctx.accept(TokenKind::UnqualifiedPackageName)? {
        let token = ctx.current();
        let action = if token.value == "*" {
            SpecAction::Star
        } else {
            SpecAction::NodeName {
                name: alias(token.value),
                namespace: None,
            }
        };
        events.push(SpecEvent {
            action,
            span: Span::of(&token),
        });
    } else if ctx.accept(TokenKind::FullyQualifiedPackageName)? {
        let token = ctx.current();
        let (namespace, name) = token
            .value
            .rsplit_once('.')
            .expect("dotted name contains .");
        events.push(SpecEvent {
            action: SpecAction::NodeName {
                name: alias(name),
                namespace: Some(namespace),
            },
            span: Span::of(&token),
        });
    } else if ctx.accept(TokenKind::Filename)? {
        let token = ctx.current();
        events.push(SpecEvent {
            action: SpecAction::Filename { path: token.value },
            span: Span::of(&token),
        });
        return Ok(());
    }

    let mut has_version = false;
    let mut has_hash = false;
    loop {
        if ctx.accept(TokenKind::VersionHashPair)?
            || ctx.accept(TokenKind::GitVersion)?
            || ctx.accept(TokenKind::Version)?
        {
            let token = ctx.current();
            if has_version {
                return Err(ParsingError {
                    kind: ParsingErrorKind::MultipleVersions,
                    message: "Spec cannot have multiple versions".to_string(),
                    token: Some(token),
                }
                .into());
            }
            events.push(SpecEvent {
                action: SpecAction::NodeVersion {
                    version: &token.value[1..],
                },
                span: Span::of(&token),
            });
            has_version = true;
        } else if ctx.accept(TokenKind::BoolVariant)? {
            let token = ctx.current();
            events.push(SpecEvent {
                action: SpecAction::BoolVariant {
                    name: token.value[1..].trim(),
                    value: token.value.starts_with('+'),
                    propagate: false,
                },
                span: Span::of(&token),
            });
        } else if ctx.accept(TokenKind::PropagatedBoolVariant)? {
            let token = ctx.current();
            events.push(SpecEvent {
                action: SpecAction::BoolVariant {
                    name: token.value[2..].trim(),
                    value: token.value.starts_with("++"),
                    propagate: true,
                },
                span: Span::of(&token),
            });
        } else if ctx.accept(TokenKind::KeyValuePair)? {
            let token = ctx.current();
            let (mut name, value) = token.value.split_once('=').expect("KVP contains =");
            let concrete = name.ends_with(':');
            if concrete {
                name = &name[..name.len() - 1];
            }
            events.push(SpecEvent {
                action: SpecAction::KeyValuePair {
                    name,
                    value: strip_quotes_and_unescape(value),
                    propagate: false,
                    concrete,
                },
                span: Span::of(&token),
            });
        } else if ctx.accept(TokenKind::PropagatedKeyValuePair)? {
            let token = ctx.current();
            let (mut name, value) = token.value.split_once("==").expect("propagated KVP");
            let concrete = name.ends_with(':');
            if concrete {
                name = &name[..name.len() - 1];
            }
            events.push(SpecEvent {
                action: SpecAction::KeyValuePair {
                    name,
                    value: strip_quotes_and_unescape(value),
                    propagate: true,
                    concrete,
                },
                span: Span::of(&token),
            });
        } else if ctx.next.map(|t| t.kind) == Some(TokenKind::DagHash) {
            if has_hash {
                break; // PY: a second `/hash` starts the next spec
            }
            ctx.accept(TokenKind::DagHash)?;
            let token = ctx.current();
            events.push(SpecEvent {
                action: SpecAction::DagHash {
                    hash: &token.value[1..],
                },
                span: Span::of(&token),
            });
            has_hash = true;
        } else {
            break;
        }
    }
    Ok(())
}
