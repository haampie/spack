// SPDX-License-Identifier: (Apache-2.0 OR MIT)

//! Pure-Rust port of Spack's version algebra (`lib/spack/spack/version/version_types.py`).
//!
//! This is a bug-for-bug port: the Python implementation is the specification, including its
//! quirks (e.g. `1.0 != 1`, `"final"` being accepted as a prerelease marker, and the asymmetric
//! `TypeError`/`NotImplementedError` paths, which surface here as [`VersionError::Type`]).
//!
//! Python's version types form a dynamic hierarchy whose set operations return varying types;
//! here that dynamism is modeled by the [`VersionUnion`] enum. Operations that in Python may
//! lazily raise `VersionLookupError` (git versions without an attached lookup) return
//! `Result<_, VersionError>` with [`VersionError::LookupNeeded`] instead. The git ref-version
//! lookup itself is external: a [`GitVersion`] either has its reference version pinned
//! ([`GitVersion::ref_version`] succeeds) or reports [`GitVersion::needs_lookup`]; the binding
//! layer resolves lookups via [`GitVersion::resolve_lookup`] / [`GitVersion::set_ref_version`].
//!
//! # Python name → Rust name
//!
//! | Python                                   | Rust                                          |
//! |------------------------------------------|-----------------------------------------------|
//! | `VersionStrComponent`                    | [`VersionStrComponent`]                       |
//! | version tuple component (`int` or str)   | [`Component`]                                 |
//! | prerelease tuple `(kind[, num])`         | [`Prerelease`]                                |
//! | `StandardVersion`                        | [`StandardVersion`]                           |
//! | `GitVersion`                             | [`GitVersion`]                                |
//! | `GitVersion.ref`                         | [`GitVersion::git_ref`]                       |
//! | `GitVersion.ref_version` (property)      | [`GitVersion::ref_version`] (fallible)        |
//! | `ClosedOpenRange`                        | [`ClosedOpenRange`]                           |
//! | `VersionList`                            | [`VersionList`]                               |
//! | `VersionList` element                    | [`VersionItem`]                               |
//! | `VersionType` (dynamic)                  | [`VersionUnion`]                              |
//! | `Version(string)` (factory)              | [`parse_version`]                             |
//! | `VersionRange(lo, hi)` (factory)         | [`version_range`] / `ClosedOpenRange::from_version_range` |
//! | `from_string(string)` / `ver(str)`       | [`from_string`] (also `VersionUnion::from_string`) |
//! | `any_version`                            | [`any_version`]                               |
//! | `_next_version` / `_prev_version`        | [`next_version`] / [`prev_version`]           |
//! | `StandardVersion.__getitem__(int)`       | [`StandardVersion::component`]                |
//! | `StandardVersion.__getitem__(slice)`     | [`StandardVersion::slice`]                    |
//! | `x.up_to(n)`                             | [`StandardVersion::up_to`]                    |
//! | `x < y` (cross-type)                     | [`VersionUnion::compare`] (fallible)          |
//! | `x in y`                                 | [`VersionUnion::contains`] (fallible)         |
//! | `is_git_version` / `is_git_commit_sha`   | [`is_git_version`] / [`is_git_commit_sha`]    |
//! | `EmptyRangeError`                        | [`VersionError::EmptyRange`]                  |
//! | `VersionLookupError`                     | [`VersionError::LookupNeeded`]                |
//! | `ValueError` (bad version strings)       | [`VersionError::Parse`]                       |
//! | `TypeError` / `NotImplementedError`      | [`VersionError::Type`]                        |
//!
//! The binding layer is expected to wrap `parse_version` for the `Version()` factory,
//! `from_string`/`ver` for spec parsing, and the `VersionUnion` methods for the set algebra.
//!
//! Known deviations from Python (all flagged, none observable in the reference test suite):
//!
//! * Numeric components are `i64`, not arbitrary-precision; a numeric component larger than
//!   `i64::MAX` is a parse error instead of being accepted.
//! * Equality and hashing of a [`GitVersion`] whose lookup is unresolved compare the stored
//!   fields instead of raising (Python raises `VersionLookupError` from `__eq__`).

mod component;
mod git;
mod list;
mod parse;
mod range;
mod standard;
#[cfg(test)]
mod tests;
mod union;

use std::fmt;

pub use component::{Component, Prerelease, VersionStrComponent};
pub use git::GitVersion;
pub use list::{VersionItem, VersionList};
pub use parse::{is_git_commit_sha, is_git_version};
pub use range::ClosedOpenRange;
pub use standard::{next_version, prev_version, StandardVersion};
pub use union::{any_version, from_string, parse_version, ver, version_range, VersionUnion};

/// Infinity-like versions. The order in the list implies the comparison rules.
pub const INFINITY_VERSIONS: [&str; 7] = [
    "stable", "nightly", "trunk", "head", "master", "main", "develop",
];

/// Minimum length of an infinity version name (`len("main")`).
pub(crate) const IV_MIN_LEN: usize = 4;

pub const ALPHA: u8 = 0;
pub const BETA: u8 = 1;
pub const RC: u8 = 2;
pub const FINAL: u8 = 3;

pub(crate) const PRERELEASE_TO_STRING: [&str; 3] = ["alpha", "beta", "rc"];

pub(crate) fn string_to_prerelease(s: &str) -> Option<u8> {
    match s {
        "alpha" => Some(ALPHA),
        "beta" => Some(BETA),
        "rc" => Some(RC),
        "final" => Some(FINAL),
        _ => None,
    }
}

/// Errors mirroring Python's `VersionError` subclasses (plus `TypeError` paths).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VersionError {
    /// Python `EmptyRangeError`.
    EmptyRange(String),
    /// Python `VersionLookupError`: a git ref needs an external lookup.
    LookupNeeded(String),
    /// Python `ValueError` from parsing.
    Parse(String),
    /// Python `TypeError` / `NotImplementedError` on unsupported operand combinations.
    Type(String),
}

impl fmt::Display for VersionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VersionError::EmptyRange(m)
            | VersionError::LookupNeeded(m)
            | VersionError::Parse(m)
            | VersionError::Type(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for VersionError {}
