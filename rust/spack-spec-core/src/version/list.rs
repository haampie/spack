// SPDX-License-Identifier: (Apache-2.0 OR MIT)

//! Port of `VersionList` and the pairwise algebra between its element types.

use std::cmp::Ordering;
use std::fmt;

use super::git::GitVersion;
use super::range::ClosedOpenRange;
use super::standard::{next_version, StandardVersion};
use super::union::VersionUnion;
use super::VersionError;

/// One element of a [`VersionList`]: Python's `StandardVersion`, `GitVersion`, or
/// `ClosedOpenRange` (a `VersionList` never nests).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum VersionItem {
    Version(StandardVersion),
    Git(GitVersion),
    Range(ClosedOpenRange),
}

impl fmt::Display for VersionItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VersionItem::Version(v) => v.fmt(f),
            VersionItem::Git(v) => v.fmt(f),
            VersionItem::Range(v) => v.fmt(f),
        }
    }
}

/// Total order across items, transcribed from the Python `__lt__`/`__gt__` cross-type
/// comparisons: items order by their effective version (the version itself, a git version's
/// reference version, or a range's lower bound); ties break as
/// `StandardVersion < GitVersion < ClosedOpenRange`, then by git ref or range upper bound.
pub(crate) fn item_cmp(a: &VersionItem, b: &VersionItem) -> Result<Ordering, VersionError> {
    use VersionItem::{Git, Range, Version};
    Ok(match (a, b) {
        (Version(x), Version(y)) => x.cmp(y),
        (Range(x), Range(y)) => x.lo().cmp(y.lo()).then_with(|| x.hi().cmp(y.hi())),
        (Git(x), Git(y)) => x
            .ref_version()?
            .cmp(y.ref_version()?)
            .then_with(|| x.git_ref().cmp(y.git_ref())),
        (Version(x), Git(y)) => x.cmp(y.ref_version()?).then(Ordering::Less),
        (Git(x), Version(y)) => x.ref_version()?.cmp(y).then(Ordering::Greater),
        (Version(x), Range(y)) => x.cmp(y.lo()).then(Ordering::Less),
        (Range(x), Version(y)) => x.lo().cmp(y).then(Ordering::Greater),
        (Git(x), Range(y)) => x.ref_version()?.cmp(y.lo()).then(Ordering::Less),
        (Range(x), Git(y)) => x.lo().cmp(y.ref_version()?).then(Ordering::Greater),
    })
}

/// Pairwise `intersects()` between items.
pub(crate) fn item_intersects(a: &VersionItem, b: &VersionItem) -> Result<bool, VersionError> {
    use VersionItem::{Git, Range, Version};
    Ok(match (a, b) {
        (Version(x), Version(y)) => x == y,
        (Version(_), Git(_)) | (Git(_), Version(_)) => false,
        (Git(x), Git(y)) => x.eq_checked(y)?,
        (Version(x), Range(r)) | (Range(r), Version(x)) => r.contains_version(x),
        (Git(x), Range(r)) | (Range(r), Git(x)) => r.contains_version(x.ref_version()?),
        (Range(x), Range(y)) => x.lo() < y.hi() && y.lo() < x.hi(),
    })
}

/// Pairwise `satisfies()` between items.
pub(crate) fn item_satisfies(a: &VersionItem, b: &VersionItem) -> Result<bool, VersionError> {
    use VersionItem::{Git, Range, Version};
    Ok(match (a, b) {
        (Version(x), Version(y)) => x == y,
        (Version(_), Git(_)) | (Git(_), Version(_)) => false,
        (Git(x), Git(y)) => x.eq_checked(y)?,
        (Version(x), Range(r)) => r.contains_version(x),
        (Git(x), Range(r)) => r.contains_version(x.ref_version()?),
        // A range never satisfies a concrete version.
        (Range(_), Version(_)) | (Range(_), Git(_)) => false,
        (Range(x), Range(y)) => !(x.lo() < y.lo() || y.hi() < x.hi()),
    })
}

/// Pairwise `intersection()` between items; `None` is Python's empty `VersionList()`.
pub(crate) fn item_intersection(
    a: &VersionItem,
    b: &VersionItem,
) -> Result<Option<VersionItem>, VersionError> {
    use VersionItem::{Git, Range, Version};
    Ok(match (a, b) {
        (Version(x), Version(y)) => (x == y).then(|| a.clone()),
        (Version(_), Git(_)) | (Git(_), Version(_)) => None,
        (Git(x), Git(y)) => x.eq_checked(y)?.then(|| a.clone()),
        (concrete @ Version(x), Range(r)) | (Range(r), concrete @ Version(x)) => {
            r.contains_version(x).then(|| concrete.clone())
        }
        (concrete @ Git(x), Range(r)) | (Range(r), concrete @ Git(x)) => r
            .contains_version(x.ref_version()?)
            .then(|| concrete.clone()),
        (Range(x), Range(y)) => {
            let max_lo = x.lo().max(y.lo());
            let min_hi = x.hi().min(y.hi());
            (max_lo < min_hi)
                .then(|| VersionItem::Range(ClosedOpenRange::raw(max_lo.clone(), min_hi.clone())))
        }
    })
}

/// Port of `ClosedOpenRange._union_if_not_disjoint`: the connected union of a range and
/// another item, or `None` when they are disjoint.
pub(crate) fn range_union_if_not_disjoint(
    range: &ClosedOpenRange,
    other: &VersionItem,
) -> Result<Option<ClosedOpenRange>, VersionError> {
    Ok(match other {
        // Note <= because union(1:2, 3:4) = 1:4.
        VersionItem::Range(o) => (range.lo() <= o.hi() && o.lo() <= range.hi()).then(|| {
            ClosedOpenRange::raw(
                range.lo().min(o.lo()).clone(),
                range.hi().max(o.hi()).clone(),
            )
        }),
        VersionItem::Version(v) => range.contains_version(v).then(|| range.clone()),
        VersionItem::Git(g) => range
            .contains_version(g.ref_version()?)
            .then(|| range.clone()),
    })
}

/// Port of `bisect_left` over a sorted item slice, using the cross-type `<`.
pub(crate) fn bisect_left(items: &[VersionItem], x: &VersionItem) -> Result<usize, VersionError> {
    let (mut lo, mut hi) = (0, items.len());
    while lo < hi {
        let mid = (lo + hi) / 2;
        if item_cmp(&items[mid], x)? == Ordering::Less {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    Ok(lo)
}

/// Port of Python's `VersionList`: a sorted, non-redundant list of versions, git versions,
/// and ranges.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct VersionList {
    versions: Vec<VersionItem>,
}

impl VersionList {
    /// Port of `VersionList()`.
    pub fn new() -> Self {
        VersionList {
            versions: Vec::new(),
        }
    }

    /// Port of `VersionList(string)`.
    pub fn from_string(s: &str) -> Result<Self, VersionError> {
        Ok(match super::union::from_string(s)? {
            VersionUnion::List(l) => l,
            other => VersionList {
                versions: vec![other.into_item().expect("non-list variant")],
            },
        })
    }

    /// A `VersionList` holding exactly the given item, without normalization
    /// (Python `VersionList(version_type)`).
    pub(crate) fn from_item(item: VersionItem) -> Self {
        VersionList {
            versions: vec![item],
        }
    }

    pub fn versions(&self) -> &[VersionItem] {
        &self.versions
    }

    pub fn len(&self) -> usize {
        self.versions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.versions.is_empty()
    }

    pub fn get(&self, index: usize) -> Option<&VersionItem> {
        self.versions.get(index)
    }

    pub fn iter(&self) -> std::slice::Iter<'_, VersionItem> {
        self.versions.iter()
    }

    /// Port of `VersionList.add`.
    pub fn add(&mut self, item: VersionUnion) -> Result<(), VersionError> {
        match item {
            VersionUnion::List(l) => {
                for it in l.versions {
                    self.add_item(it)?;
                }
                Ok(())
            }
            other => self.add_item(other.into_item().expect("non-list variant")),
        }
    }

    pub(crate) fn add_item(&mut self, item: VersionItem) -> Result<(), VersionError> {
        match item {
            VersionItem::Range(range) => {
                let mut item = range;
                let mut i = bisect_left(&self.versions, &VersionItem::Range(item.clone()))?;

                // Note: can span multiple concrete versions to the left (as well as to the
                // right). For instance insert 1.2: into [1.2, hash=1.2, 1.3, 1.4:1.5].
                while i > 0 {
                    match range_union_if_not_disjoint(&item, &self.versions[i - 1])? {
                        None => break, // disjoint
                        Some(union) => {
                            item = union;
                            self.versions.remove(i - 1);
                            i -= 1;
                        }
                    }
                }

                while i < self.versions.len() {
                    match range_union_if_not_disjoint(&item, &self.versions[i])? {
                        None => break,
                        Some(union) => {
                            item = union;
                            self.versions.remove(i);
                        }
                    }
                }

                self.versions.insert(i, VersionItem::Range(item));
                Ok(())
            }
            concrete => {
                let i = bisect_left(&self.versions, &concrete)?;
                // Only insert when prev and next are not intersected.
                if i > 0 && item_intersects(&concrete, &self.versions[i - 1])? {
                    return Ok(());
                }
                if i < self.versions.len() && item_intersects(&concrete, &self.versions[i])? {
                    return Ok(());
                }
                self.versions.insert(i, concrete);
                Ok(())
            }
        }
    }

    /// Port of the `concrete` property: the single concrete element, if any.
    pub fn concrete(&self) -> Option<&VersionItem> {
        match self.versions.as_slice() {
            [item @ (VersionItem::Version(_) | VersionItem::Git(_))] => Some(item),
            _ => None,
        }
    }

    /// Port of `concrete_range_as_version`: like `concrete`, but collapses the range
    /// `x:x` to the version `x` (compatibility with old Spack).
    pub fn concrete_range_as_version(&self) -> Option<VersionItem> {
        match self.versions.as_slice() {
            [item @ (VersionItem::Version(_) | VersionItem::Git(_))] => Some(item.clone()),
            [VersionItem::Range(r)] if next_version(r.lo()) == *r.hi() => {
                Some(VersionItem::Version(r.lo().clone()))
            }
            _ => None,
        }
    }

    /// Port of `lowest()`: the lowest `StandardVersion` in the list.
    pub fn lowest(&self) -> Option<&StandardVersion> {
        self.versions.iter().find_map(|v| match v {
            VersionItem::Version(v) => Some(v),
            _ => None,
        })
    }

    /// Port of `highest()`: the highest `StandardVersion` in the list.
    pub fn highest(&self) -> Option<&StandardVersion> {
        self.versions.iter().rev().find_map(|v| match v {
            VersionItem::Version(v) => Some(v),
            _ => None,
        })
    }

    /// Port of `highest_numeric()`: the highest non-develop `StandardVersion` in the list.
    pub fn highest_numeric(&self) -> Option<&StandardVersion> {
        self.versions.iter().rev().find_map(|v| match v {
            VersionItem::Version(v) if !v.isdevelop() => Some(v),
            _ => None,
        })
    }

    /// Port of `preferred()`.
    pub fn preferred(&self) -> Option<&StandardVersion> {
        self.highest_numeric().or_else(|| self.highest())
    }

    /// Port of `VersionList.satisfies`.
    pub fn satisfies(&self, other: &VersionUnion) -> Result<bool, VersionError> {
        match other {
            VersionUnion::List(rhs) => {
                for lhs in &self.versions {
                    let mut any = false;
                    for r in &rhs.versions {
                        if item_satisfies(lhs, r)? {
                            any = true;
                            break;
                        }
                    }
                    if !any {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            other => {
                let rhs = other.clone().into_item().expect("non-list variant");
                for lhs in &self.versions {
                    if !item_satisfies(lhs, &rhs)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
        }
    }

    /// Port of `VersionList.intersects`.
    pub fn intersects(&self, other: &VersionUnion) -> Result<bool, VersionError> {
        match other {
            VersionUnion::List(rhs) => {
                let (mut s, mut o) = (0, 0);
                while s < self.versions.len() && o < rhs.versions.len() {
                    if item_intersects(&self.versions[s], &rhs.versions[o])? {
                        return Ok(true);
                    } else if item_cmp(&self.versions[s], &rhs.versions[o])? == Ordering::Less {
                        s += 1;
                    } else {
                        o += 1;
                    }
                }
                Ok(false)
            }
            // Python `VersionList.intersects` accepts only ranges and standard versions;
            // a bare `GitVersion` right-hand side raises TypeError.
            VersionUnion::Git(_) => Err(VersionError::Type(
                "'intersects()' not supported for instances of GitVersion".to_string(),
            )),
            other => {
                let rhs = other.clone().into_item().expect("non-list variant");
                for v in &self.versions {
                    if item_intersects(v, &rhs)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
        }
    }

    /// Port of `VersionList.union` (`copy()` + `add`).
    pub fn union(&self, other: &VersionUnion) -> Result<VersionList, VersionError> {
        let mut result = self.clone();
        result.add(other.clone())?;
        Ok(result)
    }

    /// Port of `VersionList.intersection`.
    pub fn intersection(&self, other: &VersionUnion) -> Result<VersionList, VersionError> {
        match other {
            VersionUnion::List(rhs) => {
                let mut result = VersionList::new();
                for (lhs, rhs) in [(self, rhs), (rhs, self)] {
                    for x in &lhs.versions {
                        let i = bisect_left(&rhs.versions, x)?;
                        if i > 0 {
                            if let Some(it) = item_intersection(&rhs.versions[i - 1], x)? {
                                result.add_item(it)?;
                            }
                        }
                        if i < rhs.versions.len() {
                            if let Some(it) = item_intersection(&rhs.versions[i], x)? {
                                result.add_item(it)?;
                            }
                        }
                    }
                }
                Ok(result)
            }
            other => {
                let wrapped =
                    VersionList::from_item(other.clone().into_item().expect("non-list variant"));
                self.intersection(&VersionUnion::List(wrapped))
            }
        }
    }

    /// Port of `VersionList.intersect`: in-place intersection, returns whether it changed.
    pub fn intersect(&mut self, other: &VersionUnion) -> Result<bool, VersionError> {
        let isection = self.intersection(other)?;
        let changed = isection.versions != self.versions;
        self.versions = isection.versions;
        Ok(changed)
    }

    /// Port of `VersionList.__contains__` for a single (non-list) element.
    pub(crate) fn contains_item(&self, other: &VersionItem) -> Result<bool, VersionError> {
        // Python's isinstance check covers only ranges and standard versions; anything else
        // (git versions) falls through to False.
        if matches!(other, VersionItem::Git(_)) {
            return Ok(false);
        }
        let i = bisect_left(&self.versions, other)?;
        if i > 0 && item_contains(&self.versions[i - 1], other)? {
            return Ok(true);
        }
        if i < self.versions.len() && item_contains(&self.versions[i], other)? {
            return Ok(true);
        }
        Ok(false)
    }

    /// Port of `VersionList.__contains__`.
    pub fn contains(&self, other: &VersionUnion) -> Result<bool, VersionError> {
        match other {
            VersionUnion::List(l) => {
                for item in &l.versions {
                    if !self.contains_item(item)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            other => self.contains_item(&other.clone().into_item().expect("non-list variant")),
        }
    }
}

/// Python `x in y` for item-typed `y`: `StandardVersion.__contains__` and
/// `ClosedOpenRange.__contains__` are subset checks; `GitVersion.__contains__` raises.
pub(crate) fn item_contains(
    haystack: &VersionItem,
    needle: &VersionItem,
) -> Result<bool, VersionError> {
    match haystack {
        VersionItem::Git(_) => Err(VersionError::Type(
            "'in' not supported for instances of GitVersion".to_string(),
        )),
        _ => item_satisfies(needle, haystack),
    }
}

impl fmt::Display for VersionList {
    /// Port of `VersionList.__str__`: comma-separated items; concrete standard versions get
    /// an `=` prefix.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, v) in self.versions.iter().enumerate() {
            if i > 0 {
                f.write_str(",")?;
            }
            match v {
                VersionItem::Version(v) => write!(f, "={v}")?,
                other => other.fmt(f)?,
            }
        }
        Ok(())
    }
}

/// Lexicographic comparison of two lists, mirroring Python list comparison.
pub(crate) fn list_cmp(a: &VersionList, b: &VersionList) -> Result<Ordering, VersionError> {
    for (x, y) in a.versions.iter().zip(b.versions.iter()) {
        let c = item_cmp(x, y)?;
        if c != Ordering::Equal {
            return Ok(c);
        }
    }
    Ok(a.versions.len().cmp(&b.versions.len()))
}
