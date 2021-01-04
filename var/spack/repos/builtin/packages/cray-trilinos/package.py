# Copyright 2013-2021 Lawrence Livermore National Security, LLC and other
# Spack Project Developers. See the top-level COPYRIGHT file for details.
#
# SPDX-License-Identifier: (Apache-2.0 OR MIT)

from spack import *


class CrayTrilinos(Package):
    """The Trilinos Project is an effort to develop algorithms and enabling
    technologies within an object-oriented software framework for the solution
    of large-scale, complex multi-physics engineering and scientific problems.
    A unique design feature of Trilinos is its focus on packages.
    This package is a wrapper for Cray's version of Trilinos.
    """

    homepage = "https://pubs.cray.com/bundle/XC_Series_Programming_Environment_User_Guide_1705_S-2529/page/Trilinos.html"
    has_code = False    # Skip attempts to fetch source that is not available

    maintainers = ['haampie']

    version('12.18.1.1')

    def install(self, spec, prefix):
        raise InstallError(
            self.spec.format('{name} is not installable, you need to specify '
                             'it as an external package in packages.yaml'))
