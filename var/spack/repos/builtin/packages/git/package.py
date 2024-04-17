# Copyright 2013-2024 Lawrence Livermore National Security, LLC and other
# Spack Project Developers. See the top-level COPYRIGHT file for details.
#
# SPDX-License-Identifier: (Apache-2.0 OR MIT)

import os
import re

from spack.package import *
from spack.util.environment import is_system_path


class Git(AutotoolsPackage):
    """Git is a free and open source distributed version control
    system designed to handle everything from small to very large
    projects with speed and efficiency.
    """

    homepage = "http://git-scm.com"
    url = "https://mirrors.edge.kernel.org/pub/software/scm/git/git-2.12.0.tar.gz"
    maintainers("jennfshr")

    tags = ["build-tools"]

    executables = ["^git$"]

    license("GPL-2.0-only")

    # Every new git release comes with a corresponding manpage resource:
    # https://www.kernel.org/pub/software/scm/git/git-manpages-{version}.tar.gz
    # https://mirrors.edge.kernel.org/pub/software/scm/git/sha256sums.asc
    version("2.42.0", sha256="34aedd54210d7216a55d642bbb4cfb22695b7610719a106bf0ddef4c82a8beed")

    depends_on("autoconf", type="build")
    depends_on("automake", type="build")
    depends_on("libtool", type="build")
    depends_on("m4", type="build")

