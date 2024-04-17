# Copyright 2013-2024 Lawrence Livermore National Security, LLC and other
# Spack Project Developers. See the top-level COPYRIGHT file for details.
#
# SPDX-License-Identifier: (Apache-2.0 OR MIT)

import os
import shutil

import llnl.util.tty as tty

from spack.package import *


class SingularityBase(MakefilePackage):
    pass

class Singularityce(SingularityBase):
    """Singularity is a container technology focused on building portable
    encapsulated environments to support "Mobility of Compute" For older
    versions of Singularity (pre 3.0) you should use singularity-legacy,
    which has a different install base (Autotools).

    Needs post-install chmod/chown steps to enable full functionality.
    See package definition or `spack-build-out.txt` build log for details,
    e.g.

    tail -15 $(spack location -i singularityce)/.spack/spack-build-out.txt
    """

    homepage = "https://sylabs.io/singularity/"
    url = "https://github.com/sylabs/singularity/releases/download/v3.9.1/singularity-ce-3.9.1.tar.gz"
    git = "https://github.com/sylabs/singularity.git"

    license("Apache-2.0")

    maintainers("alalazo")
    version("master", branch="master")

    version("4.1.0", sha256="119667f18e76a750b7d4f8612d7878c18a824ee171852795019aa68875244813")
    version("4.0.3", sha256="b3789c9113edcac62032ce67cd1815cab74da6c33c96da20e523ffb54cdcedf3")
    version("3.11.5", sha256="5acfbb4a109d9c63a25c230e263f07c1e83f6c726007fbcd97a533f03d33a86a")
    version("3.11.4", sha256="751dbea64ec16fd7e7af1e36953134c778c404909f9d27ba89006644160b2fde")
    version("3.11.3", sha256="a77ede063fd115f85f98f82d2e30459b5565db7d098665497bcd684bf8edaec9")
    version("3.10.3", sha256="f87d8e212ce209c5212d6faf253b97a24b5d0b6e6b17b5e58b316cdda27a332f")
    version("3.10.2", sha256="b4f279856ea4bf28a1f34f89320c02b545d6e57d4143679920e1ac4267f540e1")
    version("3.10.1", sha256="e3af12edc0260bc3a3a481459a3a4457de9235025e6b37288da80e3cdc011a7a")
    version("3.10.0", sha256="5e22e6cdad66c331668f6cff4544c83917bb3db90da3cf92403a394c5bf8cc8f")
    version("3.9.9", sha256="1381433d64138c08e93ffacdfb4844e82c2288f1e39a9d2c631a1c4021381f2a")
    version("3.9.1", sha256="1ba3bb1719a420f48e9b0a6afdb5011f6c786d0f107ef272528c632fff9fd153")
    version("3.8.0", sha256="5fa2c0e7ef2b814d8aa170826b833f91e5031a85d85cd1292a234e6c55da1be1")
