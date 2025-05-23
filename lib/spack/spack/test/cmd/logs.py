# Copyright Spack Project Developers. See COPYRIGHT file for details.
#
# SPDX-License-Identifier: (Apache-2.0 OR MIT)

import gzip
import os
import shutil
import sys
import tempfile
from contextlib import contextmanager
from io import BytesIO, TextIOWrapper

import pytest

import spack
import spack.cmd.logs
import spack.concretize
import spack.main
import spack.spec
from spack.main import SpackCommand

logs = SpackCommand("logs")
install = SpackCommand("install")


@contextmanager
def stdout_as_buffered_text_stream():
    """Attempt to simulate "typical" interface for stdout when user is
    running Spack/Python from terminal. "spack log" should not be run
    for all possible cases of what stdout might look like, in
    particular some programmatic redirections of stdout like StringIO
    are not meant to be supported by this command; more-generally,
    mechanisms that depend on decoding binary output prior to write
    are not supported for "spack log".
    """
    original_stdout = sys.stdout

    with tempfile.TemporaryFile(mode="w+b") as tf:
        sys.stdout = TextIOWrapper(tf, encoding="utf-8")
        try:
            yield tf
        finally:
            sys.stdout = original_stdout


def _rewind_collect_and_decode(rw_stream):
    rw_stream.seek(0)
    return rw_stream.read().decode("utf-8")


@pytest.fixture
def disable_capture(capfd):
    with capfd.disabled():
        yield


def test_logs_cmd_errors(install_mockery, mock_fetch, mock_archive, mock_packages):
    spec = spack.concretize.concretize_one("pkg-c")
    assert not spec.installed

    with pytest.raises(spack.main.SpackCommandError, match="is not installed or staged"):
        logs("pkg-c")

    with pytest.raises(spack.main.SpackCommandError, match="Too many specs"):
        logs("pkg-c mpi")

    install("pkg-c")
    os.remove(spec.package.install_log_path)
    with pytest.raises(spack.main.SpackCommandError, match="No logs are available"):
        logs("pkg-c")


def _write_string_to_path(string, path):
    """Write a string to a file, preserving newline format in the string."""
    with open(path, "wb") as f:
        f.write(string.encode("utf-8"))


def test_dump_logs(install_mockery, mock_fetch, mock_archive, mock_packages, disable_capture):
    """Test that ``spack log`` can find (and print) the logs for partial
    builds and completed installs.

    Also make sure that for compressed logs, that we automatically
    decompress them.
    """
    cmdline_spec = spack.spec.Spec("libelf")
    concrete_spec = spack.concretize.concretize_one(cmdline_spec)

    # Sanity check, make sure this test is checking what we want: to
    # start with
    assert not concrete_spec.installed

    stage_log_content = "test_log stage output\nanother line"
    installed_log_content = "test_log install output\nhere to test multiple lines"

    with concrete_spec.package.stage:
        _write_string_to_path(stage_log_content, concrete_spec.package.log_path)
        with stdout_as_buffered_text_stream() as redirected_stdout:
            spack.cmd.logs._logs(cmdline_spec, concrete_spec)
            assert _rewind_collect_and_decode(redirected_stdout) == stage_log_content

    install("--fake", "libelf")

    # Sanity check: make sure a path is recorded, regardless of whether
    # it exists (if it does exist, we will overwrite it with content
    # in this test)
    assert concrete_spec.package.install_log_path

    with gzip.open(concrete_spec.package.install_log_path, "wb") as compressed_file:
        bstream = BytesIO(installed_log_content.encode("utf-8"))
        compressed_file.writelines(bstream)

    with stdout_as_buffered_text_stream() as redirected_stdout:
        spack.cmd.logs._logs(cmdline_spec, concrete_spec)
        assert _rewind_collect_and_decode(redirected_stdout) == installed_log_content


@pytest.mark.usefixtures("mock_packages", "mock_archive")
def test_logs_ambiguous_spec_in_env(tmp_path, mock_fetch):
    """Test that logs command errors if spec is ambiguous in an environment."""
    env_dir = tmp_path / "test_env"
    env_dir.mkdir()

    with spack.environment.create(str(env_dir)):
        install("--fake", "zlib@1.0")
        install("--fake", "zlib@1.2")

        # Add specs to the environment without full concretization to simulate choice
        # For this test, we need them to be "in the environment" abstractly
        # so that disambiguate_spec has to choose.
        # However, install() already makes them concrete and "installed".
        # The ambiguity for disambiguate_spec in an env typically comes from
        # abstract specs in spack.yaml that could resolve to multiple concrete ones,
        # or if the user query itself is abstract.

        # Let's simulate by having two installed versions that match "zlib"
        # and ensure logs command is called within the env context.
        with spack.environment.active_environment(str(env_dir)):
            # At this point, both zlib@1.0 and zlib@1.2 are "in the environment"
            # because they were installed into it.
            # The disambiguate_spec function, when given "zlib" and this env,
            # should find multiple matches.
            with pytest.raises(
                spack.main.SpackCommandError, match="matches multiple packages"
            ) or pytest.raises(
                spack.spec.AmbiguousSpecError, match="matches multiple packages"
            ):
                # We need to call the main `logs` command, not `_logs`
                logs("zlib")


@pytest.mark.usefixtures("mock_packages", "mock_archive", "install_mockery")
def test_logs_failed_install_in_env(tmp_path, mock_fetch, disable_capture):
    """Test logs from staging for a failed install in an environment."""
    env_dir = tmp_path / "test_env"
    env_dir.mkdir()
    log_content = "This is the log from a failed build in staging."

    with spack.environment.create(str(env_dir)) as env:
        env.add("aext")
        env.concretize()
        spec = env.specs_by_name("aext")[0]

        # Simulate failed build: spec is not installed, but stage log exists
        assert not spec.installed
        stage_dir = spec.package.stage.path
        os.makedirs(stage_dir, exist_ok=True)
        _write_string_to_path(log_content, spec.package.log_path)

        with spack.environment.active_environment(env):
            with stdout_as_buffered_text_stream() as redirected_stdout:
                # Call the main `logs` command
                logs(str(spec.name))
                assert _rewind_collect_and_decode(redirected_stdout) == log_content

        # Clean up stage
        shutil.rmtree(stage_dir)


@pytest.mark.usefixtures("mock_packages", "mock_archive", "install_mockery")
def test_logs_installed_in_env(tmp_path, mock_fetch, disable_capture):
    """Test logs from install dir for a successfully installed package in an environment."""
    env_dir = tmp_path / "test_env"
    env_dir.mkdir()
    install_log_content = "This is the log from a successful install in an env."
    stage_log_content = "This is a stale log in staging for an installed pkg."

    with spack.environment.create(str(env_dir)) as env:
        # Install a package into the environment
        # Using spack.main.install directly into env context
        with spack.environment.active_environment(env):
            install("--fake", "mpileaks") # mpileaks is a simple package
        
        env.concretize() # Concretize again after install to update env status
        spec = env.specs_by_name("mpileaks")[0]

        assert spec.installed

        # Create its install log
        install_log_path = spec.package.install_log_path
        os.makedirs(os.path.dirname(install_log_path), exist_ok=True)
        _write_string_to_path(install_log_content, install_log_path)

        # Create a (potentially stale) stage log
        stage_dir = spec.package.stage.path
        os.makedirs(stage_dir, exist_ok=True)
        _write_string_to_path(stage_log_content, spec.package.log_path)

        with spack.environment.active_environment(env):
            with stdout_as_buffered_text_stream() as redirected_stdout:
                logs(str(spec.name))
                assert _rewind_collect_and_decode(redirected_stdout) == install_log_content
        
        # Clean up stage and install prefix if necessary (though install_mockery handles prefix)
        shutil.rmtree(stage_dir, ignore_errors=True)
        # No need to clean install_log_path explicitly as it's in mock prefix


@pytest.mark.usefixtures("mock_packages", "mock_archive", "install_mockery")
def test_logs_uninstalled_from_env(tmp_path, mock_fetch, disable_capture):
    """Test logs from staging for a package uninstalled from an environment,
    assuming stage logs persist.
    """
    env_dir = tmp_path / "test_env"
    env_dir.mkdir()
    stage_log_content = "Log from staging after uninstall."
    install_log_content = "This was the install log." # Should not be found

    with spack.environment.create(str(env_dir)) as env:
        # Install a package
        install("--fake", "aext")
        spec = spack.spec.Spec("aext").concretized()
        assert spec.installed

        # Create its install log
        _write_string_to_path(install_log_content, spec.package.install_log_path)

        # Simulate its stage log also exists (e.g. from build process)
        stage_dir = spec.package.stage.path
        os.makedirs(stage_dir, exist_ok=True)
        _write_string_to_path(stage_log_content, spec.package.log_path)

        # Now, add it to the environment, then "uninstall" it
        # For this test, "uninstall" means it's no longer considered installed by spack,
        # but its staging logs might remain.
        # We'll achieve this by changing the spec's installed status.
        # A real `spack uninstall` might also clean the stage, so we manually ensure it.
        env.add(str(spec)) # Add the concrete installed spec
        env.concretize() # Make sure env knows about it

        # Mark as uninstalled - this is a bit of a mock up of uninstallation
        # The important part is `spec.installed` becomes False
        # and we ensure the install log is gone / not preferred
        spec.package.installed = False # Simulate uninstallation state
        if os.path.exists(spec.package.install_log_path):
            os.remove(spec.package.install_log_path)


        with spack.environment.active_environment(env):
            # Retrieve the spec from the environment to ensure it's the one env is tracking
            env_spec = env.specs_by_name("aext")[0]
            env_spec.package.installed = False # Ensure the env's copy also reflects this

            # Sanity checks
            assert not env_spec.installed
            assert os.path.exists(env_spec.package.log_path) # Stage log
            assert not os.path.exists(env_spec.package.install_log_path) # Install log gone


            with stdout_as_buffered_text_stream() as redirected_stdout:
                logs(str(env_spec.name))
                assert _rewind_collect_and_decode(redirected_stdout) == stage_log_content
        
        # Clean up stage
        shutil.rmtree(stage_dir, ignore_errors=True)
