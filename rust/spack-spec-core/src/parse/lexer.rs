// SPDX-License-Identifier: (Apache-2.0 OR MIT)

//! Hand-written lexer reproducing `spack.spec_parser.SpecTokens` exactly.
//!
//! The Python tokenizer compiles the 16 token regexes of `SpecTokens` into one alternation, in
//! declaration order, and scans the input with `regex.scanner`: at each position the first
//! alternative that matches wins, and each alternative matches with Python `re` backtracking
//! semantics. This module transcribes each alternative into direct character-dispatch code; the
//! places where the transcription has to model backtracking or zero-width assertions are called
//! out with `PY:` comments.
//!
//! Known divergences from Python, all outside the byte range of spec strings seen in practice:
//!
//! * `\s` is implemented as `char::is_whitespace` plus U+001C..U+001F; Python's `\s` matches the
//!   same set for `str` patterns.
//! * The `\b` in `VERSION` uses `char::is_alphanumeric() || c == '_'` for the following
//!   character, a close approximation of Python's Unicode `\w` (which also has a few connector
//!   punctuation and mark characters).

/// The 16 token kinds of `spack.spec_parser.SpecTokens`, in declaration (priority) order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenKind {
    /// `(?:(?:\^|\%\%|\%)\[)`
    StartEdgeProperties,
    /// `(?:\](?:\s*VIRTUAL_ASSIGNMENT)?)`
    EndEdgeProperties,
    /// `(?:(?:\^|\%\%|\%)(?:\s*VIRTUAL_ASSIGNMENT)?)`
    Dependency,
    /// `(?:@(?:GIT_VERSION_PATTERN)=(?:VERSION))`
    VersionHashPair,
    /// `@(?:GIT_VERSION_PATTERN)`
    GitVersion,
    /// `(?:@\s*(?:VERSION_LIST))`
    Version,
    /// `(?:(?:\+\+|~~|--)\s*NAME)`
    PropagatedBoolVariant,
    /// `(?:[~+-]\s*NAME)`
    BoolVariant,
    /// `(?:NAME:?==(?:VALUE|QUOTED_VALUE))`
    PropagatedKeyValuePair,
    /// `(?:NAME:?=(?:VALUE|QUOTED_VALUE))`
    KeyValuePair,
    /// `(?:\.|\/|[a-zA-Z0-9-_]*\/)(?:[a-zA-Z0-9-_\.\/]*)(?:\.json|\.yaml)` on unix
    Filename,
    /// `(?:IDENTIFIER(?:\.IDENTIFIER)+)`
    FullyQualifiedPackageName,
    /// `(?:IDENTIFIER|\*)`
    UnqualifiedPackageName,
    /// `(?:/(?:[a-zA-Z_0-9]+))`
    DagHash,
    /// `(?:\s+)`
    Ws,
    /// `(?:.[\s]*)`
    Unexpected,
}

impl TokenKind {
    /// The Python `SpecTokens` member name, for differential comparison.
    pub fn python_name(self) -> &'static str {
        match self {
            TokenKind::StartEdgeProperties => "START_EDGE_PROPERTIES",
            TokenKind::EndEdgeProperties => "END_EDGE_PROPERTIES",
            TokenKind::Dependency => "DEPENDENCY",
            TokenKind::VersionHashPair => "VERSION_HASH_PAIR",
            TokenKind::GitVersion => "GIT_VERSION",
            TokenKind::Version => "VERSION",
            TokenKind::PropagatedBoolVariant => "PROPAGATED_BOOL_VARIANT",
            TokenKind::BoolVariant => "BOOL_VARIANT",
            TokenKind::PropagatedKeyValuePair => "PROPAGATED_KEY_VALUE_PAIR",
            TokenKind::KeyValuePair => "KEY_VALUE_PAIR",
            TokenKind::Filename => "FILENAME",
            TokenKind::FullyQualifiedPackageName => "FULLY_QUALIFIED_PACKAGE_NAME",
            TokenKind::UnqualifiedPackageName => "UNQUALIFIED_PACKAGE_NAME",
            TokenKind::DagHash => "DAG_HASH",
            TokenKind::Ws => "WS",
            TokenKind::Unexpected => "UNEXPECTED",
        }
    }
}

/// One token. `start`/`end` are *character* offsets into the input, matching Python's
/// `Match.start()`/`end()`. `virtuals`/`substitute` mirror the named capture groups on
/// `DEPENDENCY` and `END_EDGE_PROPERTIES` tokens (Python `Token.subvalues`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token<'a> {
    pub kind: TokenKind,
    pub value: &'a str,
    pub start: usize,
    pub end: usize,
    pub virtuals: Option<&'a str>,
    pub substitute: Option<&'a str>,
}

/// Payload of Python's `SpecTokenizationError`: the full token list (including `WS` and
/// `UNEXPECTED` tokens) and the input, enough to rebuild the caret underline of
/// `spec_parser.SpecTokenizationError.__init__`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecTokenizationError<'a> {
    pub tokens: Vec<Token<'a>>,
    pub input: &'a str,
}

impl SpecTokenizationError<'_> {
    /// The `^^^` line under the input: carets under `UNEXPECTED` tokens, spaces elsewhere.
    pub fn underline(&self) -> String {
        let mut line = String::new();
        for token in &self.tokens {
            let c = if token.kind == TokenKind::Unexpected {
                '^'
            } else {
                ' '
            };
            for _ in token.start..token.end {
                line.push(c);
            }
        }
        line
    }

    /// The uncolored Python message: `unexpected characters in the spec string\n{input}\n{underline}`.
    pub fn message(&self) -> String {
        format!(
            "unexpected characters in the spec string\n{}\n{}",
            self.input,
            self.underline()
        )
    }
}

/// Tokenize without failing: `UNEXPECTED` and `WS` tokens are included in the result. This is
/// the raw scanner, i.e. Python's `SPEC_TOKENIZER.tokenize` before the `UNEXPECTED` check.
pub fn tokenize_all(input: &str) -> Vec<Token<'_>> {
    Lexer::new(input).collect()
}

/// Tokenize like `spack.spec_parser.tokenize`: `Err` when the input has unexpected characters,
/// with the full token list as payload; `Ok` includes `WS` tokens.
pub fn tokenize(input: &str) -> Result<Vec<Token<'_>>, SpecTokenizationError<'_>> {
    let tokens = tokenize_all(input);
    if tokens.iter().any(|t| t.kind == TokenKind::Unexpected) {
        Err(SpecTokenizationError { tokens, input })
    } else {
        Ok(tokens)
    }
}

/// Streaming lexer over `input`; yields tokens covering the whole input.
pub struct Lexer<'a> {
    input: &'a str,
    /// Byte offset of the next token.
    pos: usize,
    /// Character offset of the next token (Python `Match.start()` is in characters).
    char_pos: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(input: &'a str) -> Self {
        Lexer {
            input,
            pos: 0,
            char_pos: 0,
        }
    }
}

impl<'a> Iterator for Lexer<'a> {
    type Item = Token<'a>;

    fn next(&mut self) -> Option<Token<'a>> {
        if self.pos >= self.input.len() {
            return None;
        }
        let (kind, end, virtuals, substitute) = scan_token(self.input, self.pos);
        let value = &self.input[self.pos..end];
        let start = self.char_pos;
        let end_char = start + value.chars().count();
        let token = Token {
            kind,
            value,
            start,
            end: end_char,
            virtuals: virtuals.map(|(s, e)| &self.input[s..e]),
            substitute: substitute.map(|(s, e)| &self.input[s..e]),
        };
        self.pos = end;
        self.char_pos = end_char;
        Some(token)
    }
}

type Span = (usize, usize);

/// Match one token at byte offset `i`; returns (kind, end byte offset, virtuals, substitute).
/// Never fails: `UNEXPECTED` matches any non-whitespace character.
fn scan_token(s: &str, i: usize) -> (TokenKind, usize, Option<Span>, Option<Span>) {
    let b = s.as_bytes();
    match b[i] {
        // PY: START_EDGE_PROPERTIES, then END_EDGE_PROPERTIES, then DEPENDENCY.
        b'^' | b'%' => {
            // PY: the sigil alternation `(?:\^|\%\%|\%)` tries `%%` before `%`.
            let sig_end = if b[i] == b'%' && b.get(i + 1) == Some(&b'%') {
                i + 2
            } else {
                i + 1
            };
            if b.get(sig_end) == Some(&b'[') {
                (TokenKind::StartEdgeProperties, sig_end + 1, None, None)
            } else if let Some((end, v, sub)) = scan_virtual_assignment(s, sig_end) {
                (TokenKind::Dependency, end, Some(v), Some(sub))
            } else {
                (TokenKind::Dependency, sig_end, None, None)
            }
        }
        b']' => {
            if let Some((end, v, sub)) = scan_virtual_assignment(s, i + 1) {
                (TokenKind::EndEdgeProperties, end, Some(v), Some(sub))
            } else {
                (TokenKind::EndEdgeProperties, i + 1, None, None)
            }
        }
        b'@' => {
            // PY: VERSION_HASH_PAIR before GIT_VERSION before VERSION. A failed `=VERSION` tail
            // cannot be rescued by a shorter git ref (the ref character class excludes `=`), so
            // the pattern match is the same for both attempts.
            let git = scan_git_version_pattern(s, i + 1);
            let pair = git.and_then(|ge| {
                if b.get(ge) == Some(&b'=') {
                    scan_version(s, ge + 1)
                } else {
                    None
                }
            });
            if let Some(end) = pair {
                (TokenKind::VersionHashPair, end, None, None)
            } else if let Some(end) = git {
                (TokenKind::GitVersion, end, None, None)
            } else if let Some(end) = scan_version_list(s, skip_ws(s, i + 1)) {
                (TokenKind::Version, end, None, None)
            } else {
                (TokenKind::Unexpected, skip_ws(s, i + 1), None, None)
            }
        }
        c @ (b'+' | b'~' | b'-') => {
            // PY: PROPAGATED_BOOL_VARIANT (`++`/`~~`/`--`) before BOOL_VARIANT.
            if b.get(i + 1) == Some(&c) {
                if let Some(end) = scan_name(s, skip_ws(s, i + 2)) {
                    return (TokenKind::PropagatedBoolVariant, end, None, None);
                }
            }
            if let Some(end) = scan_name(s, skip_ws(s, i + 1)) {
                (TokenKind::BoolVariant, end, None, None)
            } else {
                (TokenKind::Unexpected, unexpected_end(s, i), None, None)
            }
        }
        c if is_ident_start(c) => {
            // PY: PROPAGATED_KEY_VALUE_PAIR, KEY_VALUE_PAIR, FILENAME,
            // FULLY_QUALIFIED_PACKAGE_NAME, UNQUALIFIED_PACKAGE_NAME, in that order.
            let name_end = scan_name(s, i).unwrap();
            // `:?==` (propagated) or `:?=`; NAME cannot contain `:` or `=`, so no backtracking.
            let sep = if b.get(name_end) == Some(&b':') {
                name_end + 1
            } else {
                name_end
            };
            if s[sep..].starts_with("==") {
                if let Some(end) = scan_kvp_value(s, sep + 2) {
                    return (TokenKind::PropagatedKeyValuePair, end, None, None);
                }
            }
            // PY: KEY_VALUE_PAIR can still match after PROPAGATED failed on its value: the
            // second `=` of `==` is a VALUE character (e.g. `foo==` tokenizes as
            // KEY_VALUE_PAIR with value `=`).
            if s[sep..].starts_with('=') {
                if let Some(end) = scan_kvp_value(s, sep + 1) {
                    return (TokenKind::KeyValuePair, end, None, None);
                }
            }
            if let Some(end) = scan_filename(s, i) {
                return (TokenKind::Filename, end, None, None);
            }
            if let Some(end) = scan_dotted_identifier(s, i) {
                return (TokenKind::FullyQualifiedPackageName, end, None, None);
            }
            (
                TokenKind::UnqualifiedPackageName,
                scan_identifier(s, i).unwrap(),
                None,
                None,
            )
        }
        b'*' => (TokenKind::UnqualifiedPackageName, i + 1, None, None),
        b'/' => {
            // PY: FILENAME before DAG_HASH.
            if let Some(end) = scan_filename(s, i) {
                (TokenKind::Filename, end, None, None)
            } else if let Some(end) = scan_hash(s, i + 1) {
                (TokenKind::DagHash, end, None, None)
            } else {
                (TokenKind::Unexpected, unexpected_end(s, i), None, None)
            }
        }
        b'.' => {
            if let Some(end) = scan_filename(s, i) {
                (TokenKind::Filename, end, None, None)
            } else {
                (TokenKind::Unexpected, unexpected_end(s, i), None, None)
            }
        }
        _ => {
            let c = s[i..].chars().next().unwrap();
            if is_py_ws(c) {
                (TokenKind::Ws, skip_ws(s, i), None, None)
            } else {
                // PY: UNEXPECTED is `.[\s]*`: one character plus trailing whitespace. `.` does
                // not match `\n`, but a newline is always taken by WS first.
                (TokenKind::Unexpected, unexpected_end(s, i), None, None)
            }
        }
    }
}

fn unexpected_end(s: &str, i: usize) -> usize {
    let c = s[i..].chars().next().unwrap();
    skip_ws(s, i + c.len_utf8())
}

// ---------------------------------------------------------------------------
// character classes
// ---------------------------------------------------------------------------

/// `IDENTIFIER` first char and `HASH` char: `[a-zA-Z_0-9]`.
fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// `IDENTIFIER` continuation: `[a-zA-Z_0-9\-]`.
fn is_ident_char(b: u8) -> bool {
    is_ident_start(b) || b == b'-'
}

/// `NAME` and version-id continuation: `[a-zA-Z_0-9\-.]`.
fn is_name_char(b: u8) -> bool {
    is_ident_char(b) || b == b'.'
}

/// `GIT_REF` continuation: `[a-zA-Z_0-9./\-]`.
fn is_git_ref_char(b: u8) -> bool {
    is_ident_start(b) || matches!(b, b'.' | b'/' | b'-')
}

/// `VALUE`: `[a-zA-Z_0-9\-+\*.,:=%^\~\/\\]`.
fn is_value_char(b: u8) -> bool {
    b.is_ascii_alphanumeric()
        || matches!(
            b,
            b'_' | b'-'
                | b'+'
                | b'*'
                | b'.'
                | b','
                | b':'
                | b'='
                | b'%'
                | b'^'
                | b'~'
                | b'/'
                | b'\\'
        )
}

/// `FILENAME` leading directory name: `[a-zA-Z0-9-_]`.
fn is_filename_prefix_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_'
}

/// `FILENAME` body: `[a-zA-Z0-9-_\.\/]`.
fn is_filename_body_char(b: u8) -> bool {
    is_filename_prefix_char(b) || b == b'.' || b == b'/'
}

/// Python `\s` for str patterns.
fn is_py_ws(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// Python `\w` for str patterns (approximation, see module docs).
fn is_py_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn skip_ws(s: &str, mut i: usize) -> usize {
    while i < s.len() {
        let c = s[i..].chars().next().unwrap();
        if !is_py_ws(c) {
            break;
        }
        i += c.len_utf8();
    }
    i
}

// ---------------------------------------------------------------------------
// sub-scanners; each returns the end byte offset of a match starting at `i`
// ---------------------------------------------------------------------------

fn scan_class(s: &str, i: usize, first: fn(u8) -> bool, rest: fn(u8) -> bool) -> Option<usize> {
    let b = s.as_bytes();
    if i >= b.len() || !first(b[i]) {
        return None;
    }
    let mut j = i + 1;
    while j < b.len() && rest(b[j]) {
        j += 1;
    }
    Some(j)
}

/// `IDENTIFIER`
fn scan_identifier(s: &str, i: usize) -> Option<usize> {
    scan_class(s, i, is_ident_start, is_ident_char)
}

/// `NAME`
fn scan_name(s: &str, i: usize) -> Option<usize> {
    scan_class(s, i, is_ident_start, is_name_char)
}

/// `DOTTED_IDENTIFIER`: at least two identifiers joined by `.`.
fn scan_dotted_identifier(s: &str, i: usize) -> Option<usize> {
    let b = s.as_bytes();
    let mut end = scan_identifier(s, i)?;
    let mut dotted = false;
    while b.get(end) == Some(&b'.') {
        match scan_identifier(s, end + 1) {
            Some(k) => {
                end = k;
                dotted = true;
            }
            None => break,
        }
    }
    if dotted {
        Some(end)
    } else {
        None
    }
}

/// `HASH`: `[a-zA-Z_0-9]+`.
fn scan_hash(s: &str, i: usize) -> Option<usize> {
    scan_class(s, i, is_ident_start, is_ident_start)
}

/// `(?:\s*(?P<virtuals>ID(?:,ID)*)=(?P<substitute>DOTTED|ID))`, the optional tail of
/// `DEPENDENCY` and `END_EDGE_PROPERTIES`. Returns (end, virtuals span, substitute span).
///
/// PY: the greedy `(?:,ID)*` needs no backtracking: a shorter virtuals list is followed by `,`,
/// never by `=`, so only the maximal list can complete the match.
fn scan_virtual_assignment(s: &str, i: usize) -> Option<(usize, Span, Span)> {
    let b = s.as_bytes();
    let vstart = skip_ws(s, i);
    let mut vend = scan_identifier(s, vstart)?;
    while b.get(vend) == Some(&b',') {
        match scan_identifier(s, vend + 1) {
            Some(k) => vend = k,
            None => break,
        }
    }
    if b.get(vend) != Some(&b'=') {
        return None;
    }
    let sstart = vend + 1;
    // PY: substitute is `DOTTED_IDENTIFIER|IDENTIFIER`, dotted tried first.
    let send = scan_dotted_identifier(s, sstart).or_else(|| scan_identifier(s, sstart))?;
    Some((send, (vstart, vend), (sstart, send)))
}

/// `VERSION` = `=?(?:[a-zA-Z0-9_][a-zA-Z_0-9\-\.]*\b)`.
fn scan_version(s: &str, i: usize) -> Option<usize> {
    scan_version_impl(s, i, false)
}

/// `VERSION(?!\s*=)`, the upper bound of `VERSION_RANGE`.
fn scan_version_upper(s: &str, i: usize) -> Option<usize> {
    scan_version_impl(s, i, true)
}

/// PY: the trailing `\b` makes the greedy run backtrack to the largest prefix end where exactly
/// one of the surrounding characters is a word character (`\b` holds on both word-to-non-word
/// and non-word-to-word transitions: `2.7=` backs off to `2.`, whose `\b` sits between `.` and
/// `7`). Inside the run the only non-word characters are `.` and `-`. With `lookahead_not_eq`,
/// candidates whose tail is optional whitespace and `=` are skipped as well; a candidate inside
/// the run is followed by a run character, so the lookahead only bites at the run end.
/// Backtracking the `=?` prefix can never help: without it the leading `[a-zA-Z0-9_]` fails on
/// the `=` itself.
fn scan_version_impl(s: &str, i: usize, lookahead_not_eq: bool) -> Option<usize> {
    let b = s.as_bytes();
    let mut j = i;
    if b.get(j) == Some(&b'=') {
        j += 1;
    }
    if j >= b.len() || !is_ident_start(b[j]) {
        return None;
    }
    let run_start = j;
    let mut k = j + 1;
    while k < b.len() && is_name_char(b[k]) {
        k += 1;
    }
    for e in (run_start + 1..=k).rev() {
        let prev_word = !matches!(b[e - 1], b'.' | b'-');
        let next_word = if e < k {
            !matches!(b[e], b'.' | b'-')
        } else {
            s[k..].chars().next().is_some_and(is_py_word)
        };
        if prev_word == next_word {
            continue; // no boundary
        }
        if lookahead_not_eq {
            let la = skip_ws(s, e);
            if b.get(la) == Some(&b'=') {
                continue;
            }
        }
        return Some(e);
    }
    None
}

/// `VERSION_RANGE` = `(?:(?:VERSION)?:(?:VERSION(?!\s*=))?)`.
///
/// PY: if the lower `VERSION` matches, the next character is never `:` after any amount of
/// backtracking (the run only stops at non-class characters, and shorter `\b`-valid prefixes are
/// followed by `.`/`-`), so an unmatched `:` fails the range outright.
fn scan_version_range(s: &str, i: usize) -> Option<usize> {
    let b = s.as_bytes();
    let colon = match scan_version(s, i) {
        Some(e) => e,
        None => i,
    };
    if b.get(colon) != Some(&b':') {
        return None;
    }
    Some(scan_version_upper(s, colon + 1).unwrap_or(colon + 1))
}

/// `VERSION_LIST` = `(?:VERSION_RANGE|VERSION)(?:\s*,\s*(?:VERSION_RANGE|VERSION))*`.
fn scan_version_list(s: &str, i: usize) -> Option<usize> {
    // PY: ordered alternation, range first.
    fn element(s: &str, i: usize) -> Option<usize> {
        scan_version_range(s, i).or_else(|| scan_version(s, i))
    }
    let b = s.as_bytes();
    let mut end = element(s, i)?;
    loop {
        let j = skip_ws(s, end);
        if b.get(j) != Some(&b',') {
            break;
        }
        match element(s, skip_ws(s, j + 1)) {
            Some(k) => end = k,
            None => break, // PY: the whole `\s*,\s*element` iteration is given back
        }
    }
    Some(end)
}

/// `GIT_VERSION_PATTERN` = `(?:(?:git\.(?:GIT_REF))|(?:GIT_HASH))`; `GIT_HASH` is exactly 40 hex
/// characters. The alternatives cannot overlap (`git.` is not hex).
fn scan_git_version_pattern(s: &str, i: usize) -> Option<usize> {
    let b = s.as_bytes();
    if s[i..].starts_with("git.") {
        if let Some(end) = scan_class(s, i + 4, is_ident_start, is_git_ref_char) {
            return Some(end);
        }
    }
    if i + 40 <= b.len() && b[i..i + 40].iter().all(u8::is_ascii_hexdigit) {
        return Some(i + 40);
    }
    None
}

/// `VALUE|QUOTED_VALUE`, ordered; the classes are disjoint on the first character.
fn scan_kvp_value(s: &str, i: usize) -> Option<usize> {
    let b = s.as_bytes();
    if i >= b.len() {
        return None;
    }
    if is_value_char(b[i]) {
        return scan_class(s, i, is_value_char, is_value_char);
    }
    if b[i] == b'\'' || b[i] == b'"' {
        return scan_quoted_value(s, i);
    }
    None
}

/// `QUOTED_VALUE` = `(?:'(?:[^']|(?<=\\)')*'|"(?:[^"]|(?<=\\)")*")`.
///
/// PY: the inner group consumes every character except unescaped quotes (an escaped quote is a
/// quote whose preceding input character is `\`); the first unescaped quote closes the value. If
/// the input ends first, backtracking hands the *last* consumed escaped quote to the closing
/// quote instead, so `"a\"` matches with the escaped quote as the closer.
fn scan_quoted_value(s: &str, i: usize) -> Option<usize> {
    let b = s.as_bytes();
    let q = b[i];
    let mut last_escaped_quote = None;
    let mut j = i + 1;
    while j < b.len() {
        // Byte scanning is safe: `q` is ASCII and never a UTF-8 continuation byte.
        if b[j] == q {
            if b[j - 1] == b'\\' {
                last_escaped_quote = Some(j);
            } else {
                return Some(j + 1);
            }
        }
        j += 1;
    }
    last_escaped_quote.map(|p| p + 1)
}

/// `FILENAME` (unix) = `(?:\.|\/|[a-zA-Z0-9-_]*\/)(?:[a-zA-Z0-9-_\.\/]*)(?:\.json|\.yaml)`.
///
/// PY: the greedy body backtracks minimally, so the match ends at the *last* `.json`/`.yaml`
/// inside the body run, and any body tail after it is left unconsumed. The prefix alternatives
/// need no cross-backtracking: they are disjoint on the first character (`.`, `/`, or a name
/// character), and the directory-name run cannot give back characters to find a `/`.
fn scan_filename(s: &str, i: usize) -> Option<usize> {
    let b = s.as_bytes();
    let body_start = match b[i] {
        b'.' | b'/' => i + 1,
        _ => {
            let mut j = i;
            while j < b.len() && is_filename_prefix_char(b[j]) {
                j += 1;
            }
            if b.get(j) == Some(&b'/') {
                j + 1
            } else {
                return None;
            }
        }
    };
    let mut body_end = body_start;
    while body_end < b.len() && is_filename_body_char(b[body_end]) {
        body_end += 1;
    }
    if body_end < body_start + 5 {
        return None;
    }
    (body_start..=body_end - 5)
        .rev()
        .find(|&p| matches!(&s[p..p + 5], ".json" | ".yaml"))
        .map(|p| p + 5)
}
