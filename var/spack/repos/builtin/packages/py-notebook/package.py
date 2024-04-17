# Copyright 2013-2024 Lawrence Livermore National Security, LLC and other
# Spack Project Developers. See the top-level COPYRIGHT file for details.
#
# SPDX-License-Identifier: (Apache-2.0 OR MIT)


from spack.package import *


class PyNotebook(PythonPackage):
    """Jupyter Interactive Notebook"""

    homepage = "https://github.com/jupyter/notebook"
    pypi = "notebook/notebook-6.1.4.tar.gz"

    version("6.5.4", sha256="517209568bd47261e2def27a140e97d49070602eea0d226a696f42a7f16c9a4e")

    depends_on("py-jupyter-packaging@0.9:0", when="@6.4.1:", type="build")

    depends_on("py-setuptools", when="@5:", type="build")
    depends_on("py-jinja2", type=("build", "run"))
    depends_on("py-tornado@6.1:", when="@6.4.5:", type=("build", "run"))
    depends_on("py-tornado@5.0:", when="@6:", type=("build", "run"))
    depends_on("py-tornado@4.1:6", when="@5.7.5:5", type=("build", "run"))
    depends_on("py-tornado@4.0:6", when="@:5.7.4", type=("build", "run"))
    depends_on("py-pyzmq@17:", when="@6:", type=("build", "run"))
    depends_on("py-argon2-cffi", when="@6.1:", type=("build", "run"))
    depends_on("py-traitlets@4.2.1:", when="@5:", type=("build", "run"))
    depends_on("py-traitlets", type=("build", "run"))
    depends_on("py-jupyter-core@4.6.1:", when="@6.0.3:", type=("build", "run"))
    depends_on("py-jupyter-core@4.6.0:", when="@6.0.2", type=("build", "run"))
    depends_on("py-jupyter-core@4.4.0:", when="@5.7.0:6.0.1", type=("build", "run"))
    depends_on("py-jupyter-core", type=("build", "run"))
    depends_on("py-jupyter-client@5.3.4:", when="@6.0.2:", type=("build", "run"))
    depends_on("py-jupyter-client@5.3.1:", when="@6.0.0:6.0.1", type=("build", "run"))
    depends_on("py-jupyter-client@5.2.0:", when="@5.7.0:5", type=("build", "run"))
    depends_on("py-jupyter-client", type=("build", "run"))
    depends_on("py-ipython-genutils", type=("build", "run"))
    depends_on("py-nbformat", type=("build", "run"))
    # https://github.com/jupyter/notebook/pull/6286
    depends_on("py-nbconvert@5:", when="@5.5:", type=("build", "run"))
    depends_on("py-nbconvert", type=("build", "run"))
    depends_on("py-nest-asyncio@1.5:", when="@6.4.10:", type=("build", "run"))
    depends_on("py-ipykernel", type=("build", "run"))
    depends_on("py-send2trash@1.8:", when="@6.4.10:", type=("build", "run"))
    depends_on("py-send2trash@1.5:", when="@6.2.0:", type=("build", "run"))
    depends_on("py-send2trash", when="@6:", type=("build", "run"))
    depends_on("py-terminado@0.8.3:", when="@6.1:", type=("build", "run"))
    depends_on("py-terminado@0.8.1:", when="@5.7.0:", type=("build", "run"))
    depends_on("py-terminado@0.3.3:", when="@:5.7.0", type=("build", "run"))
    depends_on("py-prometheus-client", when="@5.7.0:", type=("build", "run"))
    depends_on("py-nbclassic@0.4.7:", when="@6.5:", type=("build", "run"))
