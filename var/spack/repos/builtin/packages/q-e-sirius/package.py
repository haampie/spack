# Copyright 2013-2020 Lawrence Livermore National Security, LLC and other
# Spack Project Developers. See the top-level COPYRIGHT file for details.
#
# SPDX-License-Identifier: (Apache-2.0 OR MIT)
# adapted from official quantum espresso package


class QESirius(CMakePackage):
    """SIRIUS enabled fork of QuantumESPRESSO. """

    homepage = 'https://github.com/electronic-structure/q-e-sirius/'
    url = 'https://github.com/electronic-structure/q-e-sirius/archive/v6.5-rc4-sirius.tar.gz'
    git = 'https://github.com/electronic-structure/q-e-sirius.git'

    maintainers = ['simonpintarelli']

    version('develop', branch='ristretto')

    version('6.5-rc4-sirius', sha256='be5529d65e4b301d6a6d1235e8d88277171c1732768bf1cf0c7fdeae154c79f1')
    version('6.5-rc3-sirius', sha256='1bfb8c1bba815b5ab2d733f51a8f9aa7b079f2859e6f14e4dcda708ebf172b02')
    version('6.5-rc2-sirius', sha256='460b678406eec36e4ee828c027929cf8720c3965a85c20084c53398b123c9ae9')

    variant('mpi', default=True, description='Builds with mpi support')
    variant('openmp', default=True, description='Enables openMP support')
    variant('scalapack', default=True, description='Enables scalapack support')
    variant('elpa', default=False, description='Uses elpa as an eigenvalue solver')

    # Enables building Electron-phonon Wannier 'epw.x' executable
    # http://epw.org.uk/Main/About
    variant('epw', default=False,
            description='Builds Electron-phonon Wannier executable')

    # Apply upstream patches by default. Variant useful for 3rd party
    # patches which are incompatible with upstream patches
    desc = 'Apply recommended upstream patches. May need to be set '
    desc += 'to False for third party patches or plugins'
    variant('patch', default=True, description=desc)

    # QMCPACK converter patch
    # https://github.com/QMCPACK/qmcpack/tree/develop/external_codes/quantum_espresso
    variant('qmcpack', default=False,
            description='Build QE-to-QMCPACK wave function converter')

    # Dependencies
    depends_on('blas')
    depends_on('lapack')
    depends_on('fftw-api@3')
    depends_on('sirius+fortran+shared')
    depends_on('mpi', when='+mpi')
    depends_on('scalapack', when='+scalapack+mpi')
    depends_on('elpa+openmp', when='+elpa+openmp')
    depends_on('elpa~openmp', when='+elpa~openmp')
    # TODO: enable building EPW when ~mpi
    depends_on('mpi', when='+epw')

    # CONFLICTS SECTION
    # Omitted for now due to concretizer bug
    # MKL with 64-bit integers not supported.
    # conflicts(
    #     '^mkl+ilp64',
    #     msg='Quantum ESPRESSO does not support MKL 64-bit integer variant'
    # )

    # We can't ask for scalapack or elpa if we don't want MPI
    conflicts(
        '+scalapack',
        when='~mpi',
        msg='scalapack is a parallel library and needs MPI support'
    )

    conflicts(
        '+elpa',
        when='~mpi',
        msg='elpa is a parallel library and needs MPI support'
    )

    # Elpa is formally supported by @:5.4.0, but QE configure searches
    # for it in the wrong folders (or tries to download it within
    # the build directory). Instead of patching Elpa to provide the
    # folder QE expects as a link, we issue a conflict here.
    conflicts('+elpa', when='@:5.4.0')

    # The first version of Q-E to feature integrated EPW is 6.0.0,
    # as per http://epw.org.uk/Main/DownloadAndInstall .
    # Complain if trying to install a version older than this.
    conflicts('+epw', when='@:5',
              msg='EPW only available from version 6.0.0 and on')

    # Below goes some constraints as shown in the link above.
    # Constraints may be relaxed as successful reports
    # of different compiler+mpi combinations arrive

    # TODO: enable building EPW when ~mpi
    conflicts('+epw', when='~mpi', msg='EPW needs MPI')

    # EPW doesn't gets along well with OpenMPI 2.x.x
    conflicts('+epw', when='^openmpi@2.0.0:2.999.999',
              msg='OpenMPI version incompatible with EPW')

    # EPW also doesn't gets along well with PGI 17.x + OpenMPI 1.10.7
    conflicts('+epw', when='^openmpi@1.10.7%pgi@17.0:17.12',
              msg='PGI+OpenMPI version combo incompatible with EPW')

    def cmake_args(self):
        args = [
            '-DQE_ENABLE_TEST=OFF',
            self.define_from_variant('QE_ENABLE_MPI', 'mpi'),
            self.define_from_variant('QE_ENABLE_OPENMP', 'openmp'),
            self.define_from_variant('QE_ENABLE_SCALAPACK', 'scalapack'),
            self.define_from_variant('QE_ENABLE_ELPA', 'elpa'),
        ]

        return args

