# Copyright 2013-2024 Lawrence Livermore National Security, LLC and other
# Spack Project Developers. See the top-level COPYRIGHT file for details.
#
# SPDX-License-Identifier: (Apache-2.0 OR MIT)

import os.path

from spack.package import *
from spack.util.environment import is_system_path


class Glib(Package):
    """GLib provides the core application building blocks for
    libraries and applications written in C.

    The GLib package contains a low-level libraries useful for
    providing data structure handling for C, portability wrappers
    and interfaces for such runtime functionality as an event loop,
    threads, dynamic loading and an object system.
    """

    homepage = "https://developer.gnome.org/glib/"
    url = "https://download.gnome.org/sources/glib/2.53/glib-2.53.1.tar.xz"
    list_url = "https://download.gnome.org/sources/glib"
    list_depth = 1

    maintainers("michaelkuhn")

    license("LGPL-2.1-or-later")

    version("2.78.3", sha256="609801dd373796e515972bf95fc0b2daa44545481ee2f465c4f204d224b2bc21")
    variant("libmount", default=False, description="Build with libmount support")

    # Uses distutils in gio/gdbus-2.0/codegen/utils.py
    depends_on("python@:3.11", type=("build", "run"), when="@2.53.4:")
