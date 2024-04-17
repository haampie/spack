# Copyright 2013-2024 Lawrence Livermore National Security, LLC and other
# Spack Project Developers. See the top-level COPYRIGHT file for details.
#
# SPDX-License-Identifier: (Apache-2.0 OR MIT)

from spack.package import *


class PyWheel(Package, PythonExtension):
    """A built-package format for Python."""

    homepage = "https://github.com/pypa/wheel"
    url = "https://files.pythonhosted.org/packages/py3/w/wheel/wheel-0.41.2-py3-none-any.whl"
    list_url = "https://pypi.org/simple/wheel/"

    version("0.41.2", sha256="75909db2664838d015e3d9139004ee16711748a52c8f336b52882266540215d8")

    extends("python")
