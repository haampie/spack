// SPDX-License-Identifier: (Apache-2.0 OR MIT)

//! Spec tokenizer and node-level parser, a pure-Rust port of `spack.spec_parser`.
//!
//! # Layout
//!
//! * [`lexer`]: [`lexer::tokenize`] / [`lexer::tokenize_all`] reproduce the token stream of
//!   Python's `SPEC_TOKENIZER` (the `"|"`-joined `SpecTokens` regex) byte for byte: same kinds,
//!   same values (including internal whitespace and `WS` tokens), same `virtuals`/`substitute`
//!   subvalues, same character spans.
//! * [`events`]: [`events::parse_events`] runs the grammar of `SpecParser`, `SpecNodeParser` and
//!   `EdgeAttributeParser` over that stream and returns the parser's semantic actions as a flat
//!   event list, for a binding layer to replay into `Spec` objects without re-tokenizing.
//!
//! # Event contract
//!
//! `parse_events` parses *all* specs in the input (Python `SpecParser.all_specs`). Each spec is
//! the event subsequence from one [`events::SpecAction::StartRootNode`] up to the next (or the
//! end). Within a spec, a dependency edge is opened by either
//!
//! * [`events::SpecAction::StartDependencyNode`] (bare `^` / `%` / `%%`, with the virtuals of an
//!   inline `virt=pkg` assignment), or
//! * [`events::SpecAction::StartEdgeProperties`] followed by optional
//!   [`events::SpecAction::EdgeDeptypes`] / [`events::SpecAction::EdgeWhen`] and a closing
//!   [`events::SpecAction::EndEdgeProperties`] (which carries the edge's final virtuals list).
//!
//! and is followed by the events of the dependency node itself (`NodeName`/`Star`,
//! `NodeVersion`, `BoolVariant`, `KeyValuePair`, `DagHash`, `Filename`). A node without a name
//! token has no `NodeName` event; a node may be entirely empty.
//!
//! ## Replaying into a DAG
//!
//! The binding keeps `root`, `current` (initially the root) and `pending` (an unattached `^`
//! dependency), mirroring `SpecParser.next_spec`:
//!
//! * On `StartRootNode`: finish the previous spec (attach its open edge and `pending`), then
//!   start a fresh root; `current = root`, `pending = None`.
//! * On `StartDependencyNode` / `StartEdgeProperties`: the node of the previous open edge (if
//!   any) is now complete. If that edge was `direct`, attach its node to `current` with the
//!   edge's properties (`depflag` defaults to 0, `virtuals` to `()`); this does not change
//!   `current`. Otherwise attach `pending` to `root`, then set `current` to the completed node
//!   and make it the new `pending`. This ordering is what lets `pkg-a ^pkg-b ^pkg-b %pkg-c`
//!   merge the two `^pkg-b` edges only after the `%pkg-c` sub-dag is present.
//! * At the end of the stream: finish the last spec the same way.
//! * If any `EdgeWhen` was seen in a spec, run `Spec._canonicalize_conditional_edges()` on the
//!   root and each of its dependencies, wrapping `SpecError` in `SpecParsingError` like
//!   `next_spec` does.
//!
//! Node events apply to the innermost open node: the root after `StartRootNode`, the dependency
//! after `StartDependencyNode`/`EndEdgeProperties`. `NodeName` already carries the split
//! namespace and, for direct dependencies, the legacy compiler alias rewrite
//! (`spack.aliases.LEGACY_COMPILER_TO_BUILTIN`, e.g. `%clang` -> `llvm`); `KeyValuePair` values
//! are already unquoted/unescaped; `EdgeDeptypes` is already canonicalized to a depflag;
//! `EdgeWhen` is the raw spec string for the binding to parse recursively (Python calls
//! `parse_one_or_raise` on it). `Filename` ends its node: the binding loads the spec file
//! (`FileParser.parse`) and must reject dependents of the resulting concrete spec
//! (`SpecParser._parse_node`).
//!
//! # Toolchains
//!
//! Toolchain expansion is *not* part of this layer: in Python, `spack.spec_parser.parse` and
//! `parse_one_or_raise` run `expand_toolchains` on the fully parsed specs, after parsing. A
//! binding that supports toolchains must call the Python `expand_toolchains` (or an equivalent)
//! on the replayed result.
//!
//! # Errors
//!
//! * [`events::SpecParseError::Tokenization`] mirrors `SpecTokenizationError`: the full token
//!   list (with `WS` and `UNEXPECTED`) plus the input, enough to rebuild the caret underline.
//!   Like Python's lazy tokenizer with one token of parser lookahead, the error surfaces only
//!   when the parser reaches the bad token, so an earlier parse error still wins.
//! * [`events::SpecParseError::Parsing`] mirrors `SpecParsingError` (message + offending token
//!   span). [`events::ParsingErrorKind::InvalidDeptype`] is a bare `ValueError` in Python, and
//!   [`events::ParsingErrorKind::NoProgress`] reports inputs like a stray `]` on which Python's
//!   `all_specs` loops forever.
//!
//! Errors that need package data or a Spec object stay in the binding: duplicate variants and
//! architecture pieces (`Spec._add_flag`), propagation of reserved names, `_add_dependency`
//! conflicts, missing spec files, and `when=` strings that fail to parse. Both parsers emit
//! their semantic actions in the same token order — including
//! [`SpecAction::EndDependencyNode`], which marks the point where Python attaches an edge,
//! before the loop pulls another token — so a binding that replays the partial event stream of
//! [`events::parse_events_one`] before raising its error reproduces Python's interleaving of
//! `Spec` mutation errors with parse errors, down to an attach error beating a tokenization
//! error later in the input (`x %[virtuals=c]gcc %[virtuals=c]clang %&`).

pub mod events;
pub mod lexer;

#[cfg(test)]
mod events_tests;
#[cfg(test)]
mod lexer_tests;

pub use events::{
    parse_events, parse_events_one, ParsingError, ParsingErrorKind, SpecAction, SpecEvent,
};
pub use lexer::{tokenize, tokenize_all, SpecTokenizationError, Token, TokenKind};
