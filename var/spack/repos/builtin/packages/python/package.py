# Copyright 2013-2024 Lawrence Livermore National Security, LLC and other
# Spack Project Developers. See the top-level COPYRIGHT file for details.
#
# SPDX-License-Identifier: (Apache-2.0 OR MIT)

import glob
import json
import os
import platform
import re
import stat
import subprocess
import sys
from shutil import copy
from typing import Dict, List, Tuple

import llnl.util.tty as tty
from llnl.util.filesystem import is_nonsymlink_exe_with_shebang, path_contains_subdirectory
from llnl.util.lang import dedupe

from spack.build_environment import dso_suffix, stat_suffix
from spack.package import *
from spack.util.environment import is_system_path
from spack.util.prefix import Prefix


class Python(Package):
    """The Python programming language."""

    homepage = "https://www.python.org/"
    url = "https://www.python.org/ftp/python/3.8.0/Python-3.8.0.tgz"
    list_url = "https://www.python.org/ftp/python/"
    list_depth = 1
    tags = ["windows", "build-tools"]

    maintainers("skosukhin", "scheibelp")

    phases = ["configure", "build", "install"]

    #: phase
    install_targets = ["install"]
    build_targets: List[str] = []

    license("0BSD")

    version("3.11.7", sha256="068c05f82262e57641bd93458dfa883128858f5f4997aad7a36fd25b13b29209")
    version("3.9.1", sha256="29cb91ba038346da0bd9ab84a0a55a845d872c341a4da6879f462e94c741f117")
    version("3.8.4", sha256="32c4d9817ef11793da4d0d95b3191c4db81d2e45544614e8449255ca9ae3cc18")
    version("3.8.0", sha256="f1069ad3cae8e7ec467aa98a6565a62a48ef196cb8f1455a245a08db5e1792df")

    extendable = True

    # Used to cache various attributes that are expensive to compute
    _config_vars: Dict[str, Dict[str, str]] = {}

    # An in-source build with --enable-optimizations fails for python@3.X
    build_directory = "spack-build"

    executables = [r"^python\d?$"]
