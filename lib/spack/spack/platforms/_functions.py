# Copyright Spack Project Developers. See COPYRIGHT file for details.
#
# SPDX-License-Identifier: (Apache-2.0 OR MIT)
from typing import List, Optional, Type
import llnl.util.lang

from ._platform import Platform # Platform is the base class
from .darwin import Darwin
from .freebsd import FreeBSD
from .linux import Linux
from .test import Test
from .windows import Windows

# Type alias for platform classes
PlatformClass = Type[Platform]

#: List of all the platform classes known to Spack
platforms: List[PlatformClass] = [Darwin, Linux, Windows, FreeBSD, Test]


@llnl.util.lang.memoized
def _host() -> Optional[Platform]:
    """Detect and return the platform for this machine or None if detection fails."""
    # The lambda `plt` will now correctly infer its type from the `platforms` list annotation
    for platform_cls in sorted(platforms, key=lambda plt: plt.priority):
        if platform_cls.detect(): # detect() is a classmethod
            return platform_cls() # platform_cls() creates an instance
    return None


def reset() -> None:
    """The result of the host search is memoized. In case it needs to be recomputed
    we must clear the cache, which is what this function does.
    """
    _host.cache.clear()


@llnl.util.lang.memoized
def cls_by_name(name: str) -> Optional[PlatformClass]:
    """Return a platform class that corresponds to the given name or None
    if there is no match.

    Args:
        name (str): name of the platform
    """
    # The lambda `plt` will now correctly infer its type
    for platform_cls in sorted(platforms, key=lambda plt: plt.priority):
        if name.replace("_", "").lower() == platform_cls.__name__.lower():
            return platform_cls
    return None


def by_name(name: str) -> Optional[Platform]:
    """Return a platform object that corresponds to the given name or None
    if there is no match.

    Args:
        name (str): name of the platform
    """
    platform_cls = cls_by_name(name)
    return platform_cls() if platform_cls else None
