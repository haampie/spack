# Copyright 2013-2021 Lawrence Livermore National Security, LLC and other
# Spack Project Developers. See the top-level COPYRIGHT file for details.
#
# SPDX-License-Identifier: (Apache-2.0 OR MIT)

from spack import *
from spack.pkg.builtin.boost import Boost
import os


class Paraver(Package):
    """"A very powerful performance visualization and analysis tool
        based on traces that can be used to analyse any information that
        is expressed on its input trace format.  Traces for parallel MPI,
        OpenMP and other programs can be genereated with Extrae."""
    homepage = "https://tools.bsc.es/paraver"
    url = "https://ftp.tools.bsc.es/paraver/wxparaver-4.6.3-src.tar.bz2"

    # NOTE: Paraver provides only latest version for download.
    #       Don't keep/add older versions.
    version('4.6.3', sha256='ac6025eec5419e1060967eab71dfd123e585be5b5f3ac3241085895dbeca255a')
    version('4.6.2', sha256='74b85bf9e6570001d372b376b58643526e349b1d2f1e7633ca38bb0800ecf929')

    # TODO: replace this with an explicit list of components of Boost,
    # for instance depends_on('boost +filesystem')
    # See https://github.com/spack/spack/pull/22303 for reference
    depends_on(Boost.with_default_variants)
    # depends_on("extrae")
    depends_on("wxwidgets")
    depends_on("wxpropgrid")

    def install(self, spec, prefix):
        os.chdir("ptools_common_files")
        configure("--prefix=%s" % prefix)
        make()
        make("install")

        os.chdir("../paraver-kernel")
        # "--with-extrae=%s" % spec['extrae'].prefix,
        configure("--prefix=%s" % prefix,
                  "--with-ptools-common-files=%s" % prefix,
                  "--with-boost=%s" % spec['boost'].prefix,
                  "--with-boost-serialization=boost_serialization")
        make()
        make("install")

        os.chdir("../paraver-toolset")
        configure("--prefix=%s" % prefix)
        make()
        make("install")

        os.chdir("../wxparaver")
        # "--with-extrae=%s" % spec['extrae'].prefix,
        configure("--prefix=%s" % prefix,
                  "--with-paraver=%s" % prefix,
                  "--with-boost=%s" % spec['boost'].prefix,
                  "--with-boost-serialization=boost_serialization",
                  "--with-wxdir=%s" % spec['wxwidgets'].prefix.bin)
        make()
        make("install")
