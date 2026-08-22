// SPDX-License-Identifier: (Apache-2.0 OR MIT)

//! Components of a version tuple: integers, string components, and prereleases.

use std::cmp::Ordering;
use std::fmt;

use super::{FINAL, INFINITY_VERSIONS, IV_MIN_LEN, PRERELEASE_TO_STRING};

/// String (non-integer) component of a version, mirroring Python's `VersionStrComponent`.
///
/// Python stores `data: Union[int, str]`: an `int` index into `infinity_versions` for
/// infinity-like components, a `str` for ordinary literals. The index is `i64` because
/// `prev_version` may step below zero and the typemax sentinel uses `len(infinity_versions)`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum VersionStrComponent {
    Infinity(i64),
    Literal(String),
}

impl VersionStrComponent {
    /// Port of `VersionStrComponent.from_string`.
    pub fn from_string(s: &str) -> Self {
        if s.len() >= IV_MIN_LEN {
            if let Some(i) = INFINITY_VERSIONS.iter().position(|iv| *iv == s) {
                return VersionStrComponent::Infinity(i as i64);
            }
        }
        VersionStrComponent::Literal(s.to_string())
    }
}

impl fmt::Display for VersionStrComponent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VersionStrComponent::Literal(s) => f.write_str(s),
            VersionStrComponent::Infinity(i) => {
                let n = INFINITY_VERSIONS.len() as i64;
                if *i >= n {
                    f.write_str("infinity")
                } else {
                    // Python indexes `infinity_versions[i]`, which supports negative indices.
                    let idx = if *i < 0 { i + n } else { *i };
                    f.write_str(INFINITY_VERSIONS[idx as usize])
                }
            }
        }
    }
}

/// One component of a version's release tuple: `int` or `VersionStrComponent` in Python.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Component {
    Int(i64),
    Str(VersionStrComponent),
}

impl Component {
    fn rank(&self) -> u8 {
        match self {
            Component::Str(VersionStrComponent::Literal(_)) => 0,
            Component::Int(_) => 1,
            Component::Str(VersionStrComponent::Infinity(_)) => 2,
        }
    }
}

impl Ord for Component {
    /// Mirrors Python's mixed `int` / `VersionStrComponent` comparison:
    /// literal strings < integers < infinity versions.
    fn cmp(&self, other: &Self) -> Ordering {
        use VersionStrComponent::{Infinity, Literal};
        match (self, other) {
            (Component::Int(a), Component::Int(b)) => a.cmp(b),
            (Component::Str(Literal(a)), Component::Str(Literal(b))) => a.cmp(b),
            (Component::Str(Infinity(a)), Component::Str(Infinity(b))) => a.cmp(b),
            _ => self.rank().cmp(&other.rank()),
        }
    }
}

impl PartialOrd for Component {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for Component {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Component::Int(i) => write!(f, "{i}"),
            Component::Str(s) => s.fmt(f),
        }
    }
}

/// Python's prerelease tuple: `(FINAL,)`, `(ALPHA,)`, `(RC, 1)`, ...
///
/// The derived ordering matches Python tuple comparison: kind first, then a missing number
/// sorts before any number (`(ALPHA,) < (ALPHA, 0)`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Prerelease {
    pub kind: u8,
    pub num: Option<i64>,
}

impl Prerelease {
    pub const fn final_release() -> Self {
        Prerelease {
            kind: FINAL,
            num: None,
        }
    }

    pub fn is_final(&self) -> bool {
        self.kind == FINAL
    }

    /// The prerelease name as printed (`alpha`, `beta`, `rc`).
    pub(crate) fn name(&self) -> &'static str {
        PRERELEASE_TO_STRING[self.kind as usize]
    }
}
