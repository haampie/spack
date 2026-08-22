// SPDX-License-Identifier: (Apache-2.0 OR MIT)

//! Port of `ClosedOpenRange`.

use std::fmt;
use std::hash::{Hash, Hasher};

use super::standard::{next_version, prev_version, StandardVersion};
use super::VersionError;

/// Port of `_str_range`: string representation of the inclusive range `lo:hi`.
fn str_range(lo: &StandardVersion, hi: &StandardVersion) -> String {
    if *lo == StandardVersion::typemin() {
        if *hi == StandardVersion::typemax() {
            ":".to_string()
        } else {
            format!(":{hi}")
        }
    } else if *hi == StandardVersion::typemax() {
        format!("{lo}:")
    } else if lo == hi {
        lo.to_string()
    } else {
        format!("{lo}:{hi}")
    }
}

/// Port of Python's `ClosedOpenRange`: the half-open interval `[lo, hi)`.
///
/// Spack's inclusive range `x:y` is represented as `[x, next_version(y))`.
#[derive(Clone, Debug)]
pub struct ClosedOpenRange {
    lo: StandardVersion,
    hi: StandardVersion,
    string: Option<String>,
}

impl ClosedOpenRange {
    /// Port of `ClosedOpenRange(lo, hi)`: `hi` is *exclusive*.
    pub fn new(lo: StandardVersion, hi: StandardVersion) -> Result<Self, VersionError> {
        if hi < lo {
            return Err(VersionError::EmptyRange(format!(
                "{lo}..{hi} is an empty range"
            )));
        }
        Ok(ClosedOpenRange {
            lo,
            hi,
            string: None,
        })
    }

    /// Internal constructor for ranges that are non-empty by construction.
    pub(crate) fn raw(lo: StandardVersion, hi: StandardVersion) -> Self {
        debug_assert!(lo <= hi);
        ClosedOpenRange {
            lo,
            hi,
            string: None,
        }
    }

    /// Port of `ClosedOpenRange.from_version_range(lo, hi)`: `hi` is *inclusive*.
    pub fn from_version_range(
        lo: StandardVersion,
        hi: StandardVersion,
    ) -> Result<Self, VersionError> {
        match ClosedOpenRange::new(lo.clone(), next_version(&hi)) {
            Ok(mut r) => {
                r.string = Some(str_range(&lo, &hi));
                Ok(r)
            }
            Err(_) => Err(VersionError::EmptyRange(format!(
                "{lo}:{hi} is an empty range"
            ))),
        }
    }

    /// The inclusive lower bound.
    pub fn lo(&self) -> &StandardVersion {
        &self.lo
    }

    /// The *exclusive* upper bound.
    pub fn hi(&self) -> &StandardVersion {
        &self.hi
    }

    /// Whether a concrete version is inside this half-open interval.
    pub(crate) fn contains_version(&self, v: &StandardVersion) -> bool {
        self.lo <= *v && *v < self.hi
    }
}

impl fmt::Display for ClosedOpenRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.string {
            Some(s) => f.write_str(s),
            None => f.write_str(&str_range(&self.lo, &prev_version(&self.hi))),
        }
    }
}

impl PartialEq for ClosedOpenRange {
    fn eq(&self, other: &Self) -> bool {
        self.lo == other.lo && self.hi == other.hi
    }
}

impl Eq for ClosedOpenRange {}

impl Hash for ClosedOpenRange {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.lo.hash(state);
        self.hi.hash(state);
    }
}
