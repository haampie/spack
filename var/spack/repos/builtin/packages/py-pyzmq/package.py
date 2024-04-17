# Copyright 2013-2024 Lawrence Livermore National Security, LLC and other
# Spack Project Developers. See the top-level COPYRIGHT file for details.
#
# SPDX-License-Identifier: (Apache-2.0 OR MIT)

from spack.package import *


class PyPyzmq(PythonPackage):
    """PyZMQ: Python bindings for zeromq."""

    homepage = "https://github.com/zeromq/pyzmq"
    pypi = "pyzmq/pyzmq-22.3.0.tar.gz"

    license("BSD-3-Clause")

    version("14.7.0", sha256="77994f80360488e7153e64e5959dc5471531d1648e3a4bff14a714d074a38cc2")

    depends_on("py-cython", type="build")
    depends_on("py-gevent", type=("build", "run"))

