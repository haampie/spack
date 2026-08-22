// SPDX-License-Identifier: (Apache-2.0 OR MIT)

//! String helpers shared by rendering and parsing, ported from `spack.spec_parser`.

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
