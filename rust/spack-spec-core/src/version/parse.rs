// SPDX-License-Identifier: (Apache-2.0 OR MIT)

//! Port of `parse_string_components` and the git-version detection from `common.py`.

use super::component::{Component, Prerelease, VersionStrComponent};
use super::{string_to_prerelease, VersionError, FINAL};

/// Port of `VALID_VERSION`: `^[A-Za-z0-9_.-][=A-Za-z0-9_.-]*$`.
fn valid_version(s: &str) -> bool {
    let mut chars = s.chars();
    let first = match chars.next() {
        Some(c) => c,
        None => return false,
    };
    let base = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-';
    base(first) && chars.all(|c| base(c) || c == '=')
}

/// Port of `is_git_commit_sha` from `spack.util.git`.
pub fn is_git_commit_sha(s: &str) -> bool {
    s.len() == 40
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Port of `is_git_version` from `common.py`.
pub fn is_git_version(s: &str) -> bool {
    s.starts_with("git.") || is_git_commit_sha(s) || s.chars().skip(1).any(|c| c == '=')
}

/// One match of Python's `SEGMENT_REGEX`: `(?:(?P<num>[0-9]+)|(?P<str>[a-zA-Z]+))(?P<sep>[_.-]*)`.
struct Segment {
    num: Option<i64>,
    text: Option<String>,
    sep: String,
}

/// Equivalent of `SEGMENT_REGEX.findall`: scans the string, skipping characters that cannot
/// start a match (as `re.findall` does).
fn scan_segments(s: &str) -> Result<Vec<Segment>, VersionError> {
    let bytes = s.as_bytes();
    let mut segments = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let start = i;
        let (num, text) = if bytes[i].is_ascii_digit() {
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            let digits = &s[start..i];
            let value: i64 = digits.parse().map_err(|_| {
                // Deviation: Python ints are arbitrary precision; we reject i64 overflow.
                VersionError::Parse(format!("Numeric version component too large: {digits}"))
            })?;
            (Some(value), None)
        } else if bytes[i].is_ascii_alphabetic() {
            while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
                i += 1;
            }
            (None, Some(s[start..i].to_string()))
        } else {
            i += 1;
            continue;
        };
        let sep_start = i;
        while i < bytes.len() && matches!(bytes[i], b'_' | b'.' | b'-') {
            i += 1;
        }
        segments.push(Segment {
            num,
            text,
            sep: s[sep_start..i].to_string(),
        });
    }
    Ok(segments)
}

/// Port of `parse_string_components`: returns `(release, prerelease, separators)`.
pub(crate) fn parse_string_components(
    string: &str,
) -> Result<(Vec<Component>, Prerelease, Vec<String>), VersionError> {
    let string = string.trim();

    if !string.is_empty() && !valid_version(string) {
        return Err(VersionError::Parse(format!(
            "Bad characters in version string: {string}"
        )));
    }

    let mut segments = scan_segments(string)?;
    let separators: Vec<String> = segments.iter().map(|m| m.sep.clone()).collect();

    let n = segments.len();
    let prerelease = if n >= 3
        && segments[n - 2]
            .text
            .as_deref()
            .and_then(string_to_prerelease)
            .is_some()
        && segments[n - 1].num.is_some()
    {
        let kind = string_to_prerelease(segments[n - 2].text.as_deref().unwrap()).unwrap();
        let num = segments[n - 1].num;
        segments.truncate(n - 2);
        Prerelease { kind, num }
    } else if n >= 2
        && segments[n - 1]
            .text
            .as_deref()
            .and_then(string_to_prerelease)
            .is_some()
    {
        let kind = string_to_prerelease(segments[n - 1].text.as_deref().unwrap()).unwrap();
        segments.truncate(n - 1);
        Prerelease { kind, num: None }
    } else {
        Prerelease {
            kind: FINAL,
            num: None,
        }
    };

    let release: Vec<Component> = segments
        .iter()
        .map(|m| match m.num {
            Some(v) => Component::Int(v),
            None => Component::Str(VersionStrComponent::from_string(m.text.as_deref().unwrap())),
        })
        .collect();

    Ok((release, prerelease, separators))
}
