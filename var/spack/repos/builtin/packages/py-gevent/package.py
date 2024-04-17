# Copyright 2013-2024 Lawrence Livermore National Security, LLC and other
# Spack Project Developers. See the top-level COPYRIGHT file for details.
#
# SPDX-License-Identifier: (Apache-2.0 OR MIT)

from spack.package import *


class PyGevent(PythonPackage):
    """gevent is a coroutine-based Python networking library."""

    homepage = "https://www.gevent.org"
    pypi = "gevent/gevent-23.7.0.tar.gz"

    license("MIT")

    version("21.12.0", sha256="f48b64578c367b91fa793bf8eaaaf4995cb93c8bc45860e473bf868070ad094e")
    depends_on("py-cython@3:", when="@20.5.1:", type="build")

