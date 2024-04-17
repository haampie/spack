# Copyright 2013-2024 Lawrence Livermore National Security, LLC and other
# Spack Project Developers. See the top-level COPYRIGHT file for details.
#
# SPDX-License-Identifier: (Apache-2.0 OR MIT)

from spack.package import *


class PyCython(PythonPackage):
    """The Cython compiler for writing C extensions for the Python language."""

    homepage = "https://github.com/cython/cython"
    pypi = "cython/Cython-0.29.21.tar.gz"
    tags = ["build-tools"]

    license("Apache-2.0")

    version("3.0.8", sha256="8333423d8fd5765e7cceea3a9985dd1e0a5dfeb2734629e1a2ed2d6233d39de6")
    version("3.0.6", sha256="399d185672c667b26eabbdca420c98564583798af3bc47670a8a09e9f19dd660")
    version("0.29.14", sha256="e4d6bb8703d0319eb04b7319b12ea41580df44fd84d83ccda13ea463c6801414")
    version("0.29", sha256="94916d1ede67682638d3cc0feb10648ff14dc51fb7a7f147f4fedce78eaaea97")
    version("0.23.4", sha256="fec42fecee35d6cc02887f1eef4e4952c97402ed2800bfe41bbd9ed1a0730d8e")

    # https://github.com/cython/cython/issues/5751 (distutils not yet dropped)
    depends_on("python@:3.11", type=("build", "link", "run"))

    # https://github.com/cython/cython/commit/1cd24026e9cf6d63d539b359f8ba5155fd48ae21
    # collections.Iterable was removed in Python 3.10
    depends_on("python@:3.9", when="@:0.29.14", type=("build", "link", "run"))

    # https://github.com/cython/cython/commit/430e2ca220c8fed49604daf578df98aadb33a87d
    depends_on("python@:3.8", when="@:0.29.13", type=("build", "link", "run"))
