// SPDX-License-Identifier: (Apache-2.0 OR MIT)

//! String helpers shared by rendering and parsing, ported from `spack.spec_parser`.

use std::borrow::Cow;

/// `NO_QUOTES_NEEDED` in spec_parser.py: `^[a-zA-Z0-9,/_.\-\[\]]+$`.
fn no_quotes_needed(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b',' | b'/' | b'_' | b'.' | b'-' | b'[' | b']')
        })
}

/// `quote_if_needed`: single quotes by default; double quotes with JSON escaping when the
/// value contains a single quote.
pub fn quote_if_needed(value: &str) -> String {
    if no_quotes_needed(value) {
        return value.to_string();
    }
    if value.contains('\'') {
        json_escape(value)
    } else {
        format!("'{}'", value)
    }
}

/// `strip_quotes_and_unescape` in spec_parser.py: remove surrounding single or double quotes,
/// if present, and replace escaped quotes (`\'` or `\"`) with bare ones.
///
/// The quote match is `STRIP_QUOTES` = `^(['\"])(.*)\1$`: `.` does not cross a newline, and `$`
/// also matches just before one final trailing newline, which is then dropped from the result.
pub fn strip_quotes_and_unescape(value: &str) -> Cow<'_, str> {
    fn quoted_inner(v: &str) -> Option<(char, &str)> {
        let mut chars = v.chars();
        let quote = chars.next()?;
        if quote != '\'' && quote != '"' {
            return None;
        }
        let rest = chars.as_str();
        let inner = rest.strip_suffix(quote)?;
        // `rest` empty means a lone quote character: `(.*)\1` needs a second quote.
        if rest.is_empty() || inner.contains('\n') {
            return None;
        }
        Some((quote, inner))
    }

    let (quote, inner) = match quoted_inner(value) {
        Some(m) => m,
        // `$` can also match before a single trailing newline.
        None => match value.strip_suffix('\n').and_then(quoted_inner) {
            Some(m) => m,
            None => return Cow::Borrowed(value),
        },
    };
    let escaped = ['\\', quote].iter().collect::<String>();
    if inner.contains(&escaped) {
        Cow::Owned(inner.replace(&escaped, &quote.to_string()))
    } else {
        Cow::Borrowed(inner)
    }
}

/// `SPLIT_KVP` in spec_parser.py: `^({NAME})(:?==?)(.*)$`, splitting `key:==value` forms into
/// (name, separator, value). `None` when the string is not a key-value pair (also when the
/// value spans more than one line: `.` does not cross a newline, but `$` tolerates one final
/// trailing newline, which is then dropped from the value).
pub fn split_kvp(s: &str) -> Option<(&str, &str, &str)> {
    let bytes = s.as_bytes();
    if bytes.is_empty() || !(bytes[0].is_ascii_alphanumeric() || bytes[0] == b'_') {
        return None;
    }
    let mut name_end = 1;
    while name_end < bytes.len()
        && (bytes[name_end].is_ascii_alphanumeric()
            || matches!(bytes[name_end], b'_' | b'-' | b'.'))
    {
        name_end += 1;
    }
    let mut sep_end = name_end;
    if bytes.get(sep_end) == Some(&b':') {
        sep_end += 1;
    }
    if bytes.get(sep_end) != Some(&b'=') {
        return None;
    }
    sep_end += 1;
    if bytes.get(sep_end) == Some(&b'=') {
        sep_end += 1;
    }
    let value = s[sep_end..].strip_suffix('\n').unwrap_or(&s[sep_end..]);
    if value.contains('\n') {
        return None;
    }
    Some((&s[..name_end], &s[name_end..sep_end], value))
}

#[cfg(test)]
mod tests {
    use super::{split_kvp, strip_quotes_and_unescape};

    /// Expectations produced by the Python `strip_quotes_and_unescape`.
    #[test]
    fn strip_quotes_matches_python() {
        let rows: &[(&str, &str)] = &[
            ("'abc'", "abc"),
            ("\"a\\\"b\"", "a\"b"),
            ("'a\\'b'", "a'b"),
            ("noquotes", "noquotes"),
            ("''", ""),
            ("'", "'"),
            ("'a'\n", "a"),       // `$` matches before one final newline
            ("'a\nb'", "'a\nb'"), // `.` does not cross a newline
            ("\"mixed'", "\"mixed'"),
            ("'''", "'"),
        ];
        for (input, expected) in rows {
            assert_eq!(strip_quotes_and_unescape(input), *expected, "{input:?}");
        }
    }

    /// Expectations produced by the Python `SPLIT_KVP` regex.
    #[test]
    fn split_kvp_matches_python() {
        let rows: &[(&str, Option<(&str, &str, &str)>)] = &[
            ("foo:==bar", Some(("foo", ":==", "bar"))),
            ("a=b=c", Some(("a", "=", "b=c"))),
            ("a==b", Some(("a", "==", "b"))),
            ("a:=b", Some(("a", ":=", "b"))),
            ("=x", None),
            ("a", None),
            ("k.e-y_2=v", Some(("k.e-y_2", "=", "v"))),
            ("a=b\n", Some(("a", "=", "b"))),
            ("a=b\nc", None),
            ("a:b=c", None),
        ];
        for (input, expected) in rows {
            assert_eq!(split_kvp(input), *expected, "{input:?}");
        }
    }
}

/// Python `json.dumps(str)`: escape `\`, `"`, and control codes; keep non-ASCII as
/// `\uXXXX` like the default `ensure_ascii=True`.
fn json_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\x08' => out.push_str("\\b"),
            '\x0c' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c if c.is_ascii() => out.push(c),
            c => {
                let mut buf = [0u16; 2];
                for unit in c.encode_utf16(&mut buf) {
                    out.push_str(&format!("\\u{:04x}", unit));
                }
            }
        }
    }
    out.push('"');
    out
}
