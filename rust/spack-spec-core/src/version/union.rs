// SPDX-License-Identifier: (Apache-2.0 OR MIT)

//! The dynamic `VersionType` surface: parsing entry points and the set algebra whose result
//! type varies at runtime in Python.

use std::cmp::Ordering;
use std::fmt;

use super::git::GitVersion;
use super::list::{
    item_cmp, item_intersection, item_intersects, item_satisfies, list_cmp,
    range_union_if_not_disjoint, VersionItem, VersionList,
};
use super::parse::is_git_version;
use super::range::ClosedOpenRange;
use super::standard::StandardVersion;
use super::VersionError;

/// Rust stand-in for Python's dynamically typed `VersionType`: the result of parsing and of
/// `union`/`intersection`, which return different types depending on the values.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum VersionUnion {
    Version(StandardVersion),
    Git(GitVersion),
    Range(ClosedOpenRange),
    List(VersionList),
}

impl VersionUnion {
    pub fn from_string(s: &str) -> Result<Self, VersionError> {
        from_string(s)
    }

    /// The corresponding list element for non-list variants.
    pub fn into_item(self) -> Option<VersionItem> {
        match self {
            VersionUnion::Version(v) => Some(VersionItem::Version(v)),
            VersionUnion::Git(v) => Some(VersionItem::Git(v)),
            VersionUnion::Range(v) => Some(VersionItem::Range(v)),
            VersionUnion::List(_) => None,
        }
    }

    fn as_item(&self) -> Option<VersionItem> {
        self.clone().into_item()
    }

    /// Port of `intersects()`: whether self and other overlap.
    pub fn intersects(&self, other: &VersionUnion) -> Result<bool, VersionError> {
        match (self, other) {
            (VersionUnion::List(a), _) => a.intersects(other),
            (_, VersionUnion::List(b)) => {
                // StandardVersion/GitVersion/ClosedOpenRange vs list: any element overlaps.
                let lhs = self.as_item().expect("non-list variant");
                for rhs in b.versions() {
                    if item_intersects(&lhs, rhs)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            _ => item_intersects(
                &self.as_item().expect("non-list variant"),
                &other.as_item().expect("non-list variant"),
            ),
        }
    }

    /// Port of `overlaps()` (same as `intersects()`).
    pub fn overlaps(&self, other: &VersionUnion) -> Result<bool, VersionError> {
        self.intersects(other)
    }

    /// Port of `satisfies()`: whether self is entirely contained in other.
    pub fn satisfies(&self, other: &VersionUnion) -> Result<bool, VersionError> {
        match (self, other) {
            (VersionUnion::List(a), _) => a.satisfies(other),
            (_, VersionUnion::List(b)) => {
                // `StandardVersion.satisfies(list)` delegates to `list.intersects(self)`;
                // `GitVersion`/`ClosedOpenRange` check `any(self.satisfies(rhs))`. Both reduce
                // to the same pairwise checks.
                let lhs = self.as_item().expect("non-list variant");
                for rhs in b.versions() {
                    if item_satisfies(&lhs, rhs)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            _ => item_satisfies(
                &self.as_item().expect("non-list variant"),
                &other.as_item().expect("non-list variant"),
            ),
        }
    }

    /// Port of `union()`. The result type depends on the values, exactly as in Python;
    /// combinations where Python raises `NotImplementedError` (a `GitVersion` left-hand side,
    /// or a concrete version unioned with a `GitVersion`) yield [`VersionError::Type`].
    pub fn union(&self, other: &VersionUnion) -> Result<VersionUnion, VersionError> {
        use VersionUnion as U;
        match (self, other) {
            (U::List(a), _) => Ok(U::List(a.union(other)?)),
            // Python: GitVersion does not implement union().
            (U::Git(_), _) | (U::Version(_), U::Git(_)) => Err(VersionError::Type(
                "'union()' not implemented for GitVersion".to_string(),
            )),
            (U::Version(a), U::Version(b)) => {
                if a == b {
                    Ok(self.clone())
                } else {
                    let mut l = VersionList::new();
                    l.add_item(VersionItem::Version(a.clone()))?;
                    l.add_item(VersionItem::Version(b.clone()))?;
                    Ok(U::List(l))
                }
            }
            (U::Version(_), U::Range(b)) => range_union(b, &self.as_item().unwrap()),
            (U::Version(_), U::List(b)) => Ok(U::List(b.union(self)?)),
            (U::Range(a), U::List(b)) => Ok(U::List(b.union(&U::Range(a.clone()))?)),
            (U::Range(a), _) => range_union(a, &other.as_item().expect("non-list variant")),
        }
    }

    /// Port of `intersection()`: any versions contained in both self and other, or an empty
    /// `VersionList` when there is no overlap. Like Python, a range intersected with a list
    /// on the right raises (`TypeError` there, [`VersionError::Type`] here).
    pub fn intersection(&self, other: &VersionUnion) -> Result<VersionUnion, VersionError> {
        use VersionUnion as U;
        match (self, other) {
            (U::List(a), _) => Ok(U::List(a.intersection(other)?)),
            (U::Version(_) | U::Git(_), U::List(b)) => Ok(U::List(b.intersection(self)?)),
            (U::Range(_), U::List(_)) => Err(VersionError::Type(
                "'intersection()' not supported for instances of VersionList".to_string(),
            )),
            _ => {
                let lhs = self.as_item().expect("non-list variant");
                let rhs = other.as_item().expect("non-list variant");
                Ok(match item_intersection(&lhs, &rhs)? {
                    Some(item) => VersionUnion::from(item),
                    None => U::List(VersionList::new()),
                })
            }
        }
    }

    /// The cross-type ordering Python defines via `__lt__` and friends; comparisons between a
    /// list and a non-list raise `TypeError` in Python and yield [`VersionError::Type`] here.
    pub fn compare(&self, other: &VersionUnion) -> Result<Ordering, VersionError> {
        match (self, other) {
            (VersionUnion::List(a), VersionUnion::List(b)) => list_cmp(a, b),
            (VersionUnion::List(_), _) | (_, VersionUnion::List(_)) => Err(VersionError::Type(
                "'<' not supported between a VersionList and a version".to_string(),
            )),
            _ => item_cmp(
                &self.as_item().expect("non-list variant"),
                &other.as_item().expect("non-list variant"),
            ),
        }
    }

    /// Port of `other in self`. For versions and ranges this is `other.satisfies(self)`;
    /// lists use a bisection; `GitVersion.__contains__` raises.
    pub fn contains(&self, other: &VersionUnion) -> Result<bool, VersionError> {
        match self {
            VersionUnion::List(l) => l.contains(other),
            VersionUnion::Git(_) => Err(VersionError::Type(
                "'in' not supported for instances of GitVersion".to_string(),
            )),
            _ => other.satisfies(self),
        }
    }
}

impl From<VersionItem> for VersionUnion {
    fn from(item: VersionItem) -> Self {
        match item {
            VersionItem::Version(v) => VersionUnion::Version(v),
            VersionItem::Git(v) => VersionUnion::Git(v),
            VersionItem::Range(v) => VersionUnion::Range(v),
        }
    }
}

impl fmt::Display for VersionUnion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VersionUnion::Version(v) => v.fmt(f),
            VersionUnion::Git(v) => v.fmt(f),
            VersionUnion::Range(v) => v.fmt(f),
            VersionUnion::List(v) => v.fmt(f),
        }
    }
}

/// Port of `ClosedOpenRange.union`.
fn range_union(range: &ClosedOpenRange, other: &VersionItem) -> Result<VersionUnion, VersionError> {
    match range_union_if_not_disjoint(range, other)? {
        Some(r) => Ok(VersionUnion::Range(r)),
        None => {
            let mut l = VersionList::new();
            l.add_item(VersionItem::Range(range.clone()))?;
            l.add_item(other.clone())?;
            Ok(VersionUnion::List(l))
        }
    }
}

/// Port of the `Version(string)` factory: a `GitVersion` when the string looks like a git
/// version, otherwise a `StandardVersion`. Always concrete, never a range.
pub fn parse_version(string: &str) -> Result<VersionUnion, VersionError> {
    if is_git_version(string) {
        Ok(VersionUnion::Git(GitVersion::from_string(string)?))
    } else {
        Ok(VersionUnion::Version(StandardVersion::from_string(string)?))
    }
}

/// Port of the `VersionRange(lo, hi)` factory (inclusive bounds, parsed from strings).
pub fn version_range(lo: &str, hi: &str) -> Result<ClosedOpenRange, VersionError> {
    ClosedOpenRange::from_version_range(
        StandardVersion::from_string(lo)?,
        StandardVersion::from_string(hi)?,
    )
}

/// Port of `from_string`: parses `1.2:1.4,1.6`, `=1.2.3`, `git.foo=1.2`, `1.2.3`, ... into
/// the matching version type. A bare version like `@1.2.3` parses as the range
/// `1.2.3:1.2.3`; only `=`-prefixed versions are concrete.
pub fn from_string(string: &str) -> Result<VersionUnion, VersionError> {
    let string = string.replace(' ', "");

    if string.contains(',') {
        let mut list = VersionList::new();
        for part in string.split(',') {
            list.add(from_string(part)?)?;
        }
        Ok(VersionUnion::List(list))
    } else if string.contains(':') {
        let parts: Vec<&str> = string.split(':').collect();
        if parts.len() != 2 {
            // Python: `s, e = string.split(":")` raises ValueError.
            return Err(VersionError::Parse(format!(
                "Bad version range (expected a single ':'): {string}"
            )));
        }
        let lo = if parts[0].is_empty() {
            StandardVersion::typemin()
        } else {
            StandardVersion::from_string(parts[0])?
        };
        let hi = if parts[1].is_empty() {
            StandardVersion::typemax()
        } else {
            StandardVersion::from_string(parts[1])?
        };
        Ok(VersionUnion::Range(ClosedOpenRange::from_version_range(
            lo, hi,
        )?))
    } else if let Some(rest) = string.strip_prefix('=') {
        // @=1.2.3 is an exact version.
        parse_version(rest)
    } else if is_git_version(&string) {
        Ok(VersionUnion::Git(GitVersion::from_string(&string)?))
    } else {
        // @1.2.3 is short for 1.2.3:1.2.3.
        let v = StandardVersion::from_string(&string)?;
        Ok(VersionUnion::Range(ClosedOpenRange::from_version_range(
            v.clone(),
            v,
        )?))
    }
}

/// Port of `ver()` for strings. (Python `ver()` additionally accepts lists/tuples/numbers,
/// which the binding layer is expected to stringify or feed through `VersionList`.)
pub fn ver(string: &str) -> Result<VersionUnion, VersionError> {
    from_string(string)
}

/// Port of `any_version` / `VersionList.any()`: a list containing all possible versions.
pub fn any_version() -> VersionList {
    let unbounded =
        ClosedOpenRange::from_version_range(StandardVersion::typemin(), StandardVersion::typemax())
            .expect("the unbounded range is not empty");
    VersionList::from_item(VersionItem::Range(unbounded))
}
