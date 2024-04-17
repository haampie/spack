# Copyright 2013-2024 Lawrence Livermore National Security, LLC and other
# Spack Project Developers. See the top-level COPYRIGHT file for details.
#
# SPDX-License-Identifier: (Apache-2.0 OR MIT)


from spack.package import *
from spack.pkg.builtin.singularityce import SingularityBase


# Apptainer is the new name of Singularity, piggy-back on the original package
class Apptainer(SingularityBase):
    """Apptainer is an open source container platform designed to be simple, fast, and
    secure. Many container platforms are available, but Apptainer is designed for
    ease-of-use on shared systems and in high performance computing (HPC)
    environments.

    Needs post-install chmod/chown steps to enable full functionality.
    See package definition or `spack-build-out.txt` build log for details,
    e.g.:

        tail -15 $(spack location -i apptainer)/.spack/spack-build-out.txt
    """

    homepage = "https://apptainer.org"
    url = "https://github.com/apptainer/apptainer/releases/download/v1.0.2/apptainer-1.0.2.tar.gz"
    git = "https://github.com/apptainer/apptainer.git"

    version("1.1.9", sha256="c615777539154288542cf393d3fd44c04ccb3260bc6330dc324d4e4ebe902bfa")

    #depends_on("go-bootstrap", type="build")
    depends_on("libseccomp")
    depends_on("glib")

