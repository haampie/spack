# Copyright Spack Project Developers. See COPYRIGHT file for details.
#
# SPDX-License-Identifier: (Apache-2.0 OR MIT)

"""
This module implements Version and version-ish objects. These are:

* :class:`~spack.version.version_types.StandardVersion`: A single version of a package.
* :class:`~spack.version.version_types.ClosedOpenRange`: A range of versions of a package.
* :class:`~spack.version.version_types.VersionList`: A ordered list of Version and VersionRange
  elements.
"""

import os
from typing import TYPE_CHECKING

from .common import (
    EmptyRangeError,
    VersionChecksumError,
    VersionError,
    VersionLookupError,
    infinity_versions,
    is_git_commit_sha,
    is_git_version,
)
from .version_types import (
    ClosedOpenRange,
    ConcreteVersion,
    GitVersion,
    StandardVersion,
    Version,
    VersionList,
    VersionRange,
    VersionType,
    _next_version,
    _prev_version,
    from_string,
    ver,
)

# Same toggle as spack.spec.USE_RUST_SPEC; read from the environment directly since importing
# spack.spec here would be circular.
if not TYPE_CHECKING and os.environ.get("SPACK_SPEC_IMPL", "python").lower() == "rust":
    import spack_spec

    spack_spec.register_version_errors(VersionError, EmptyRangeError, VersionLookupError)

    VersionType = spack_spec.VersionType
    ConcreteVersion = spack_spec.ConcreteVersion
    StandardVersion = spack_spec.StandardVersion
    GitVersion = spack_spec.GitVersion
    ClosedOpenRange = spack_spec.ClosedOpenRange
    VersionList = spack_spec.VersionList
    Version = spack_spec.Version
    VersionRange = spack_spec.VersionRange
    ver = spack_spec.ver
    from_string = spack_spec.from_string
    _next_version = spack_spec._next_version
    _prev_version = spack_spec._prev_version

#: This version contains all possible versions.
any_version: VersionList = VersionList([":"])

__all__ = [
    "ClosedOpenRange",
    "ConcreteVersion",
    "EmptyRangeError",
    "GitVersion",
    "StandardVersion",
    "Version",
    "VersionChecksumError",
    "VersionError",
    "VersionList",
    "VersionLookupError",
    "VersionRange",
    "VersionType",
    "_next_version",
    "_prev_version",
    "any_version",
    "from_string",
    "infinity_versions",
    "is_git_commit_sha",
    "is_git_version",
    "ver",
]
