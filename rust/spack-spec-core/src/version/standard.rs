// SPDX-License-Identifier: (Apache-2.0 OR MIT)

//! Port of `StandardVersion` and the `_next_version` / `_prev_version` helpers.

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};

use super::component::{Component, Prerelease, VersionStrComponent};
use super::parse::parse_string_components;
use super::{VersionError, INFINITY_VERSIONS};

/// Port of Python's `StandardVersion`: a single concrete version.
///
/// Equality, ordering, and hashing consider only the parsed `(release, prerelease)` tuple;
/// the original string and separators are cosmetic. When constructed without a string
/// (e.g. by [`next_version`]), `Display` stringifies lazily from the components.
#[derive(Clone, Debug)]
pub struct StandardVersion {
    string: Option<String>,
    release: Vec<Component>,
    prerelease: Prerelease,
    separators: Vec<String>,
}

impl StandardVersion {
    /// Port of `StandardVersion.from_string`.
    pub fn from_string(string: &str) -> Result<Self, VersionError> {
        let (release, prerelease, separators) = parse_string_components(string)?;
        Ok(StandardVersion {
            string: Some(string.to_string()),
            release,
            prerelease,
            separators,
        })
    }

    /// Port of `StandardVersion.typemin()`: smaller than any version.
    pub fn typemin() -> Self {
        StandardVersion {
            string: None,
            release: Vec::new(),
            prerelease: Prerelease {
                kind: super::ALPHA,
                num: None,
            },
            separators: vec![String::new()],
        }
    }

    /// Port of `StandardVersion.typemax()`: larger than any version.
    pub fn typemax() -> Self {
        StandardVersion {
            string: Some("infinity".to_string()),
            release: vec![Component::Str(VersionStrComponent::Infinity(
                INFINITY_VERSIONS.len() as i64,
            ))],
            prerelease: Prerelease::final_release(),
            separators: vec![String::new()],
        }
    }

    /// The release components (Python `version[0]` / iteration / `len()`).
    pub fn release(&self) -> &[Component] {
        &self.release
    }

    pub fn prerelease(&self) -> Prerelease {
        self.prerelease
    }

    /// Number of release components (Python `len(version)`).
    pub fn len(&self) -> usize {
        self.release.len()
    }

    pub fn is_empty(&self) -> bool {
        self.release.is_empty()
    }

    /// Port of `version[idx]` for an integer index (supports negative indices).
    pub fn component(&self, idx: isize) -> Option<&Component> {
        let n = self.release.len() as isize;
        let i = if idx < 0 { n + idx } else { idx };
        if (0..n).contains(&i) {
            Some(&self.release[i as usize])
        } else {
            None
        }
    }

    /// Port of `version[start:stop]`: rebuilds a version from the sliced components and
    /// separators, re-parsing the joined string exactly as Python does.
    pub fn slice(&self, start: Option<isize>, stop: Option<isize>) -> StandardVersion {
        let n = self.release.len() as isize;
        let norm = |v: isize| -> usize {
            let v = if v < 0 { n + v } else { v };
            v.clamp(0, n) as usize
        };
        let a = start.map_or(0, norm);
        let b = stop.map_or(self.release.len(), norm);
        if a >= b {
            return StandardVersion::from_string("").expect("empty version is valid");
        }
        let mut parts: Vec<String> = Vec::new();
        for (token, sep) in self.release[a..b].iter().zip(self.separators[a..b].iter()) {
            parts.push(token.to_string());
            parts.push(sep.clone());
        }
        parts.pop(); // We don't need the last separator.
        StandardVersion::from_string(&parts.concat()).expect("slice of a valid version is valid")
    }

    /// Port of `version.up_to(index)`.
    pub fn up_to(&self, index: isize) -> StandardVersion {
        self.slice(None, Some(index))
    }

    /// Port of the `string` property.
    pub fn string(&self) -> String {
        self.to_string()
    }

    /// Port of `isdevelop()`: true if any release component is an infinity version.
    pub fn isdevelop(&self) -> bool {
        self.release
            .iter()
            .any(|c| matches!(c, Component::Str(VersionStrComponent::Infinity(_))))
    }

    /// Port of `is_prerelease()`.
    pub fn is_prerelease(&self) -> bool {
        !self.prerelease.is_final()
    }

    /// Port of the `dotted_numeric_string` property.
    pub fn dotted_numeric_string(&self) -> String {
        let mut numeric: Vec<i64> = self
            .release
            .iter()
            .map(|c| match c {
                Component::Int(i) => *i,
                Component::Str(_) => 0,
            })
            .collect();
        if self.is_prerelease() {
            numeric.push(0);
            if let Some(n) = self.prerelease.num {
                numeric.push(n);
            }
        }
        numeric
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join(".")
    }

    fn replaced(&self, from: [char; 2], to: char) -> StandardVersion {
        let s: String = self
            .string()
            .chars()
            .map(|c| if from.contains(&c) { to } else { c })
            .collect();
        StandardVersion::from_string(&s).expect("separator replacement keeps a version valid")
    }

    /// Port of the `dotted` property.
    pub fn dotted(&self) -> StandardVersion {
        self.replaced(['-', '_'], '.')
    }

    /// Port of the `underscored` property.
    pub fn underscored(&self) -> StandardVersion {
        self.replaced(['.', '-'], '_')
    }

    /// Port of the `dashed` property.
    pub fn dashed(&self) -> StandardVersion {
        self.replaced(['.', '_'], '-')
    }

    /// Port of the `joined` property.
    pub fn joined(&self) -> StandardVersion {
        let s: String = self
            .string()
            .chars()
            .filter(|c| !['.', '-', '_'].contains(c))
            .collect();
        StandardVersion::from_string(&s).expect("separator removal keeps a version valid")
    }

    /// Port of `_stringify_version`.
    fn stringify(&self) -> String {
        let mut out = String::new();
        for (rel, sep) in self.release.iter().zip(self.separators.iter()) {
            out.push_str(&rel.to_string());
            out.push_str(sep);
        }
        if !self.prerelease.is_final() {
            out.push_str(self.prerelease.name());
        }
        if let Some(num) = self.prerelease.num {
            out.push_str(
                self.separators
                    .get(self.release.len())
                    .map(String::as_str)
                    .unwrap_or(""),
            );
            out.push_str(&num.to_string());
        }
        out
    }

    /// Test hook mirroring `v.string = None`: drops the cached string so that `Display`
    /// reconstructs it from components.
    #[cfg(test)]
    pub(crate) fn clear_string(&mut self) {
        self.string = None;
    }
}

impl fmt::Display for StandardVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.string {
            Some(s) => f.write_str(s),
            None => f.write_str(&self.stringify()),
        }
    }
}

impl PartialEq for StandardVersion {
    fn eq(&self, other: &Self) -> bool {
        self.release == other.release && self.prerelease == other.prerelease
    }
}

impl Eq for StandardVersion {}

impl Hash for StandardVersion {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.release.hash(state);
        self.prerelease.hash(state);
    }
}

impl Ord for StandardVersion {
    /// Python compares the `(release, prerelease)` tuples lexicographically.
    fn cmp(&self, other: &Self) -> Ordering {
        self.release
            .cmp(&other.release)
            .then_with(|| self.prerelease.cmp(&other.prerelease))
    }
}

impl PartialOrd for StandardVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Port of `_next_str`: produce the next string of A-Z and a-z characters.
fn next_str(s: &str) -> String {
    let mut chars: Vec<char> = s.chars().collect();
    match chars.last().copied() {
        None | Some('z') => {
            chars.push('A');
        }
        Some('Z') => *chars.last_mut().unwrap() = 'a',
        Some(c) => *chars.last_mut().unwrap() = (c as u8 + 1) as char,
    }
    chars.into_iter().collect()
}

/// Port of `_prev_str`: produce the previous string of A-Z and a-z characters.
fn prev_str(s: &str) -> String {
    let mut chars: Vec<char> = s.chars().collect();
    match chars.last().copied() {
        None | Some('A') => {
            chars.pop();
        }
        Some('a') => *chars.last_mut().unwrap() = 'Z',
        Some(c) => *chars.last_mut().unwrap() = (c as u8 - 1) as char,
    }
    chars.into_iter().collect()
}

/// Port of `_next_version_str_component` (e.g. `masteq -> mastes`, `master -> main`).
fn next_version_str_component(v: &VersionStrComponent) -> VersionStrComponent {
    match v {
        VersionStrComponent::Infinity(i) => VersionStrComponent::Infinity(i + 1),
        VersionStrComponent::Literal(s) => {
            let mut data = s.clone();
            loop {
                data = next_str(&data);
                if !INFINITY_VERSIONS.contains(&data.as_str()) {
                    break;
                }
            }
            VersionStrComponent::Literal(data)
        }
    }
}

/// Port of `_prev_version_str_component` (e.g. `mastes -> masteq`, `master -> head`).
fn prev_version_str_component(v: &VersionStrComponent) -> VersionStrComponent {
    match v {
        VersionStrComponent::Infinity(i) => VersionStrComponent::Infinity(i - 1),
        VersionStrComponent::Literal(s) => {
            let mut data = s.clone();
            loop {
                data = prev_str(&data);
                if !INFINITY_VERSIONS.contains(&data.as_str()) {
                    break;
                }
            }
            VersionStrComponent::Literal(data)
        }
    }
}

/// Port of `_next_version`: the smallest version greater than `v`.
pub fn next_version(v: &StandardVersion) -> StandardVersion {
    let mut release = v.release.clone();
    let mut separators = v.separators.clone();
    let mut prerelease = v.prerelease;
    if !prerelease.is_final() {
        prerelease.num = Some(prerelease.num.map_or(0, |n| n + 1));
    } else if release.is_empty() {
        release = vec![Component::Str(VersionStrComponent::Literal(
            "A".to_string(),
        ))];
        separators = vec![String::new()];
    } else {
        let last = release.last_mut().unwrap();
        *last = match last {
            Component::Str(s) => Component::Str(next_version_str_component(s)),
            Component::Int(i) => Component::Int(*i + 1),
        };
    }
    // Pass no string to lazily stringify, as Python does for performance.
    StandardVersion {
        string: None,
        release,
        prerelease,
        separators,
    }
}

/// Port of `_prev_version`. Like Python, this does not deal with underflow: it is only
/// meaningful as `prev_version(next_version(v))`.
pub fn prev_version(v: &StandardVersion) -> StandardVersion {
    let mut release = v.release.clone();
    let mut prerelease = v.prerelease;
    if !prerelease.is_final() {
        prerelease.num = match prerelease.num {
            Some(0) | None => None,
            Some(n) => Some(n - 1),
        };
    } else if release.is_empty() {
        return v.clone();
    } else {
        let last = release.last_mut().unwrap();
        *last = match last {
            Component::Str(s) => Component::Str(prev_version_str_component(s)),
            Component::Int(i) => Component::Int(*i - 1),
        };
    }
    StandardVersion {
        string: None,
        release,
        prerelease,
        separators: v.separators.clone(),
    }
}
