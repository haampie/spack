# Copyright 2013-2023 Lawrence Livermore National Security, LLC and other
# Spack Project Developers. See the top-level COPYRIGHT file for details.
#
# SPDX-License-Identifier: (Apache-2.0 OR MIT)

import os

from spack.package import *


class PythonVenv(Package):
    """A Spack managed Python virtual environment"""

    homepage = "https://docs.python.org/3/library/venv.html"
    has_code = False

    maintainers("haampie")

    version("1.0")

    depends_on("python", type=("build", "link", "run"))

    def install(self, spec, prefix):
        spec["python"].command("-m", "venv", "--without-pip", prefix)

    @property
    def command(self):
        """Returns the Python Exectable object"""
        version = self.spec["python"].version
        for ver in (version.up_to(2), version.up_to(1), ""):
            path = os.path.join(self.prefix.bin, f"python{ver}")
            if os.path.exists(path):
                return Executable(path)

        raise RuntimeError(f"Unable to locate {self.name} command in {self.prefix}")
