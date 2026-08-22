// SPDX-License-Identifier: (Apache-2.0 OR MIT)

//! Port of `GitVersion`: versions interpreted from git refs.

use std::fmt;
use std::hash::{Hash, Hasher};

use super::parse::is_git_commit_sha;
use super::standard::StandardVersion;
use super::VersionError;

/// Port of Python's `GitVersion`.
///
/// The ref-version lookup is external: `git.<ref>=<version>` pins the reference version at
/// parse time; a bare ref or commit sha leaves it unresolved ([`GitVersion::needs_lookup`]).
/// Operations that require the reference version return [`VersionError::LookupNeeded`] until
/// the binding resolves it via [`GitVersion::resolve_lookup`] or [`GitVersion::set_ref_version`].
#[derive(Clone, Debug)]
pub struct GitVersion {
    has_git_prefix: bool,
    git_ref: String,
    is_commit: bool,
    std_version: Option<StandardVersion>,
}

impl GitVersion {
    /// Port of `GitVersion.__init__`: parses `git.<ref>=<version>`, `git.<ref>`,
    /// `<40-hex-sha>[=<version>]`, or `<ref>=<version>`.
    pub fn from_string(string: &str) -> Result<Self, VersionError> {
        let has_git_prefix = string.starts_with("git.");
        let normalized = if has_git_prefix { &string[4..] } else { string };

        let (git_ref, std_version) = if normalized.contains('=') {
            let parts: Vec<&str> = normalized.split('=').collect();
            if parts.len() != 2 {
                // Python: `self.ref, spack_version = normalized_string.split("=")` raises
                // ValueError when there is more than one "=".
                return Err(VersionError::Parse(format!(
                    "Bad git version string (expected a single '='): {string}"
                )));
            }
            (
                parts[0].to_string(),
                Some(StandardVersion::from_string(parts[1])?),
            )
        } else {
            (normalized.to_string(), None)
        };

        let is_commit = is_git_commit_sha(&git_ref);
        Ok(GitVersion {
            has_git_prefix,
            git_ref,
            is_commit,
            std_version,
        })
    }

    /// Python `GitVersion.ref`.
    pub fn git_ref(&self) -> &str {
        &self.git_ref
    }

    /// Python `GitVersion.is_commit`.
    pub fn is_commit(&self) -> bool {
        self.is_commit
    }

    /// Python `GitVersion.commit_sha`.
    pub fn commit_sha(&self) -> Option<&str> {
        if self.is_commit {
            Some(&self.git_ref)
        } else {
            None
        }
    }

    pub fn has_git_prefix(&self) -> bool {
        self.has_git_prefix
    }

    /// The pinned reference version, if any (Python `GitVersion.std_version`).
    pub fn std_version(&self) -> Option<&StandardVersion> {
        self.std_version.as_ref()
    }

    /// Whether comparisons need an external ref-version lookup first.
    pub fn needs_lookup(&self) -> bool {
        self.std_version.is_none()
    }

    /// Port of the `ref_version` property. Errors with [`VersionError::LookupNeeded`]
    /// (Python's lazy `VersionLookupError`) when no version is pinned or resolved.
    pub fn ref_version(&self) -> Result<&StandardVersion, VersionError> {
        self.std_version.as_ref().ok_or_else(|| {
            VersionError::LookupNeeded(format!(
                "git ref '{}' cannot be looked up: call attach_lookup first",
                self.git_ref
            ))
        })
    }

    /// Pin the reference version directly.
    pub fn set_ref_version(&mut self, version: StandardVersion) {
        self.std_version = Some(version);
    }

    /// Apply the result of an external ref lookup, porting the `ref_version` property logic:
    /// a missing version becomes `"0"`, and a nonzero commit distance appends `-git.<distance>`.
    pub fn resolve_lookup(
        &mut self,
        version: Option<&str>,
        distance: u64,
    ) -> Result<(), VersionError> {
        let mut version_string = version.unwrap_or("0").to_string();
        if distance > 0 {
            version_string.push_str(&format!("-git.{distance}"));
        }
        self.std_version = Some(StandardVersion::from_string(&version_string)?);
        Ok(())
    }

    // Accessors delegating to the reference version, as Python does.

    pub fn len(&self) -> Result<usize, VersionError> {
        Ok(self.ref_version()?.len())
    }

    pub fn is_empty(&self) -> Result<bool, VersionError> {
        Ok(self.ref_version()?.is_empty())
    }

    pub fn component(&self, idx: isize) -> Result<Option<super::Component>, VersionError> {
        Ok(self.ref_version()?.component(idx).cloned())
    }

    pub fn slice(
        &self,
        start: Option<isize>,
        stop: Option<isize>,
    ) -> Result<StandardVersion, VersionError> {
        Ok(self.ref_version()?.slice(start, stop))
    }

    pub fn up_to(&self, index: isize) -> Result<StandardVersion, VersionError> {
        Ok(self.ref_version()?.up_to(index))
    }

    pub fn isdevelop(&self) -> Result<bool, VersionError> {
        Ok(self.ref_version()?.isdevelop())
    }

    pub fn is_prerelease(&self) -> Result<bool, VersionError> {
        Ok(self.ref_version()?.is_prerelease())
    }

    pub fn dotted(&self) -> Result<StandardVersion, VersionError> {
        Ok(self.ref_version()?.dotted())
    }

    pub fn underscored(&self) -> Result<StandardVersion, VersionError> {
        Ok(self.ref_version()?.underscored())
    }

    pub fn dashed(&self) -> Result<StandardVersion, VersionError> {
        Ok(self.ref_version()?.dashed())
    }

    pub fn joined(&self) -> Result<StandardVersion, VersionError> {
        Ok(self.ref_version()?.joined())
    }

    /// Fallible equality mirroring Python `__eq__`, which dereferences `ref_version` and can
    /// raise `VersionLookupError`.
    pub(crate) fn eq_checked(&self, other: &GitVersion) -> Result<bool, VersionError> {
        Ok(self.git_ref == other.git_ref && self.ref_version()? == other.ref_version()?)
    }
}

impl fmt::Display for GitVersion {
    /// Port of `GitVersion.__str__`: the (possibly `git.`-prefixed) ref, followed by
    /// `=<ref_version>` when the reference version is known.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.git_ref.is_empty() {
            if self.has_git_prefix {
                write!(f, "git.{}", self.git_ref)?;
            } else {
                f.write_str(&self.git_ref)?;
            }
        }
        if let Some(v) = &self.std_version {
            write!(f, "={v}")?;
        }
        Ok(())
    }
}

/// Deviation: Python `__eq__` raises `VersionLookupError` on unresolved refs; this compares
/// the stored fields instead (identical whenever both reference versions are known).
impl PartialEq for GitVersion {
    fn eq(&self, other: &Self) -> bool {
        self.git_ref == other.git_ref && self.std_version == other.std_version
    }
}

impl Eq for GitVersion {}

/// Python hashes only the ref, so that hashing never triggers a version lookup.
impl Hash for GitVersion {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.git_ref.hash(state);
    }
}
