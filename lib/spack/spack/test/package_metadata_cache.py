# Copyright Spack Project Developers. See COPYRIGHT file for details.
#
# SPDX-License-Identifier: (Apache-2.0 OR MIT)
import pickle

import pytest

import spack.package_metadata_cache as pmc


def test_snapshot_round_trip(mock_packages, mutable_config, tmp_path):
    """A snapshot is built on the first call and served fresh on the second."""
    mutable_config.set("config:misc_cache", str(tmp_path / "misc"))
    repo = mock_packages.repos[0]

    snapshot, fresh = pmc.load_or_build_snapshot(repo)
    assert not fresh
    snapshot2, fresh2 = pmc.load_or_build_snapshot(repo)
    assert fresh2
    assert set(snapshot) == set(snapshot2)

    real = repo.get_pkg_class("mpileaks")
    static = snapshot2["mpileaks"]
    assert isinstance(static, pmc.StaticPackage)
    assert static.name == real.name
    assert static.fullname == real.fullname
    assert set(static.versions) == set(real.versions)
    assert static.variant_names() == real.variant_names()
    assert static.dependency_names() == real.dependency_names()
    assert static.tags == tuple(getattr(real, "tags", ()))
    assert static.__doc__ == real.__doc__

    provider = repo.get_pkg_class("mpich")
    static_provider = snapshot2["mpich"]
    assert static_provider.provided_virtual_names() == provider.provided_virtual_names()


def test_hermetic_import_guard_blocks_only_registered_namespaces():
    guard = pmc.HermeticImportGuard()
    guard.add_namespace("spack_repo.test_hermetic")
    with pytest.raises(RuntimeError, match="hermetic"):
        guard.find_spec("spack_repo.test_hermetic.packages.foo.package")
    with pytest.raises(RuntimeError, match="hermetic"):
        guard.find_spec("spack_repo.test_hermetic")
    assert guard.find_spec("spack_repo.other.packages.foo") is None


def test_sanitized_pickle_replaces_local_callables():
    def local_fn(x):
        return False

    data = pickle.loads(pmc.sanitized_dumps({"f": local_fn, "n": 42}))
    assert data["n"] == 42
    assert data["f"]("anything") is True  # opaque stand-in accepts everything
