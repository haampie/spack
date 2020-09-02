# Copyright 2013-2020 Lawrence Livermore National Security, LLC and other
# Spack Project Developers. See the top-level COPYRIGHT file for details.
#
# SPDX-License-Identifier: (Apache-2.0 OR MIT)

from spack import *


class Hipmagma(MakefilePackage):
    """hipMAGMA is the AMD/ROCM/HIP enabled version of Magma"""

    homepage = "https://bitbucket.org/icl/magma"
    git = "https://bitbucket.org/icl/magma.git"
    
    version('2.0.0', branch='hipMAGMAv2.0.0')

    depends_on('hip')
    depends_on('blas')

    def edit(self, spec, prefix):
        copy('make.inc-examples/make.inc.hip_openblas', 'make.inc')

    def install(self, spec, prefix):
        cuda = self.spec['cuda'].prefix
        make('install', 'BACKEND=hip' 'CUDADIR={0}'.format(cuda))