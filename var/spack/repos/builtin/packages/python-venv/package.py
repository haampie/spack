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
        """Returns a python Executable instance"""
        subdir = "Scripts" if self.spec.satisfies("platform=windows") else "bin"
        return which("python", path=join_path(self.prefix, subdir))

    def _get_path(self, name) -> str:
        return self.command(
            "-Ec", f"import sysconfig; print(sysconfig.get_path('{name}'))", output=str
        ).strip()

    @property
    def platlib(self) -> str:
        """Directory for site-specific, platform-specific files."""
        relative_platlib = os.path.relpath(self._get_path("platlib"), self.prefix)
        assert not relative_platlib.startswith("..")
        return relative_platlib

    @property
    def purelib(self) -> str:
        """Directory for site-specific, non-platform-specific files."""
        relative_purelib = os.path.relpath(self._get_path("purelib"), self.prefix)
        assert not relative_purelib.startswith("..")
        return relative_purelib

    @property
    def headers(self):
        return HeaderList([])

    @property
    def libs(self):
        return LibraryList([])

    def setup_dependent_run_environment(self, env, dependent_spec):
        """Set PYTHONPATH to include the site-packages directory for the
        extension and any other python extensions it depends on."""
        # Packages may be installed in platform-specific or platform-independent site-packages
        # directories
        for directory in {self.platlib, self.purelib}:
            path = os.path.join(dependent_spec.prefix, directory)
            if os.path.isdir(path):
                env.prepend_path("PYTHONPATH", path)

    def setup_dependent_package(self, module, dependent_spec):
        """Called before python modules' install() methods."""

        module.python = self.command
        module.python_platlib = join_path(dependent_spec.prefix, self.platlib)
        module.python_purelib = join_path(dependent_spec.prefix, self.purelib)
