# Copyright 2013-2022 Lawrence Livermore National Security, LLC and other
# Spack Project Developers. See the top-level COPYRIGHT file for details.
#
# SPDX-License-Identifier: (Apache-2.0 OR MIT)

"""Test Spack's URL handling utility functions."""
import os
import os.path
import urllib.parse

import pytest

import spack.util.url as url_util


def test_url_local_file_path(tmpdir):
    # Create a file
    path = str(tmpdir.join("hello.txt"))
    with open(path, "wb") as f:
        f.write(b"hello world")

    # Go from path -> url -> path.
    roundtrip = url_util.local_file_path(url_util.path_to_file_url(path))

    # Verify it's the same file.
    assert os.path.samefile(roundtrip, path)

    # Test if it accepts urlparse objects
    parsed = urllib.parse.urlparse(url_util.path_to_file_url(path))
    assert os.path.samefile(url_util.local_file_path(parsed), path)


def test_url_local_file_path_no_file_scheme():
    assert url_util.local_file_path("https://example.com/hello.txt") is None
    assert url_util.local_file_path("C:\\Program Files\\hello.txt") is None


def test_relative_path_to_file_url(tmpdir):
    # Create a file
    path = str(tmpdir.join("hello.txt"))
    with open(path, "wb") as f:
        f.write(b"hello world")

    with tmpdir.as_cwd():
        roundtrip = url_util.local_file_path(url_util.path_to_file_url("hello.txt"))
        assert os.path.samefile(roundtrip, path)


def test_url_join_local_paths():
    # Resolve local link against page URL

    # wrong:
    assert (
        url_util.join("s3://bucket/index.html", "../other-bucket/document.txt")
        == "s3://bucket/other-bucket/document.txt"
    )

    # correct - need to specify resolve_href=True:
    assert (
        url_util.join("s3://bucket/index.html", "../other-bucket/document.txt", resolve_href=True)
        == "s3://other-bucket/document.txt"
    )

    # same as above: make sure several components are joined together correctly
    assert (
        url_util.join(
            # with resolve_href=True, first arg is the base url; can not be
            # broken up
            "s3://bucket/index.html",
            # with resolve_href=True, remaining arguments are the components of
            # the local href that needs to be resolved
            "..",
            "other-bucket",
            "document.txt",
            resolve_href=True,
        )
        == "s3://other-bucket/document.txt"
    )

    # Append local path components to prefix URL

    # wrong:
    assert (
        url_util.join("https://mirror.spack.io/build_cache", "my-package", resolve_href=True)
        == "https://mirror.spack.io/my-package"
    )

    # correct - Need to specify resolve_href=False:
    assert (
        url_util.join("https://mirror.spack.io/build_cache", "my-package", resolve_href=False)
        == "https://mirror.spack.io/build_cache/my-package"
    )

    # same as above; make sure resolve_href=False is default
    assert (
        url_util.join("https://mirror.spack.io/build_cache", "my-package")
        == "https://mirror.spack.io/build_cache/my-package"
    )

    # same as above: make sure several components are joined together correctly
    assert (
        url_util.join(
            # with resolve_href=False, first arg is just a prefix. No
            # resolution is done.  So, there should be no difference between
            # join('/a/b/c', 'd/e'),
            # join('/a/b', 'c', 'd/e'),
            # join('/a', 'b/c', 'd', 'e'), etc.
            "https://mirror.spack.io",
            "build_cache",
            "my-package",
        )
        == "https://mirror.spack.io/build_cache/my-package"
    )

    # For s3:// URLs, the "netloc" (bucket) is considered part of the path.
    # Make sure join() can cross bucket boundaries in this case.
    args = ["s3://bucket/a/b", "new-bucket", "c"]
    assert url_util.join(*args) == "s3://bucket/a/b/new-bucket/c"

    args.insert(1, "..")
    assert url_util.join(*args) == "s3://bucket/a/new-bucket/c"

    args.insert(1, "..")
    assert url_util.join(*args) == "s3://bucket/new-bucket/c"

    # new-bucket is now the "netloc" (bucket name)
    args.insert(1, "..")
    assert url_util.join(*args) == "s3://new-bucket/c"


def test_url_join_absolute_paths():
    # urllib.parse.urljoin(x, y) works as if you're following a link to y on a
    # website x in a web browser, which is different from os.path.join(x, y).
    # url_util.join(x, y) does the same as urljoin, except that it also handles
    # urls in the s3:// scheme like a web browser.

    # Absolute path components
    assert (
        url_util.join("http://example.com/a/b/c", "/path/to/resource")
        == "http://example.com/path/to/resource"
    )
    assert url_util.join("s3://example.com/a/b/c", "/path/to/resource") == "s3://path/to/resource"
    assert url_util.join("file:///a/b/c", "/d") == "file:///d"

    # Relative path components
    assert url_util.join("http://example.com/a/b/c", "d") == "http://example.com/a/b/c/d"
    assert url_util.join("s3://example.com/a/b/c", "d") == "s3://example.com/a/b/c/d"
    assert url_util.join("file:///a/b/c", "./d") == "file:///a/b/c/d"


@pytest.mark.parametrize(
    "url,parts",
    [
        (
            "ssh://user@host.xz:500/path/to/repo.git/",
            ("ssh", "user", "host.xz", 500, "/path/to/repo.git"),
        ),
        (
            "ssh://user@host.xz/path/to/repo.git/",
            ("ssh", "user", "host.xz", None, "/path/to/repo.git"),
        ),
        (
            "ssh://host.xz:500/path/to/repo.git/",
            ("ssh", None, "host.xz", 500, "/path/to/repo.git"),
        ),
        ("ssh://host.xz/path/to/repo.git/", ("ssh", None, "host.xz", None, "/path/to/repo.git")),
        (
            "ssh://user@host.xz/path/to/repo.git/",
            ("ssh", "user", "host.xz", None, "/path/to/repo.git"),
        ),
        ("ssh://host.xz/path/to/repo.git/", ("ssh", None, "host.xz", None, "/path/to/repo.git")),
        (
            "ssh://user@host.xz/~user/path/to/repo.git/",
            ("ssh", "user", "host.xz", None, "~user/path/to/repo.git"),
        ),
        (
            "ssh://host.xz/~user/path/to/repo.git/",
            ("ssh", None, "host.xz", None, "~user/path/to/repo.git"),
        ),
        (
            "ssh://user@host.xz/~/path/to/repo.git",
            ("ssh", "user", "host.xz", None, "~/path/to/repo.git"),
        ),
        ("ssh://host.xz/~/path/to/repo.git", ("ssh", None, "host.xz", None, "~/path/to/repo.git")),
        ("git@github.com:spack/spack.git", (None, "git", "github.com", None, "spack/spack.git")),
        ("user@host.xz:/path/to/repo.git/", (None, "user", "host.xz", None, "/path/to/repo.git")),
        ("host.xz:/path/to/repo.git/", (None, None, "host.xz", None, "/path/to/repo.git")),
        (
            "user@host.xz:~user/path/to/repo.git/",
            (None, "user", "host.xz", None, "~user/path/to/repo.git"),
        ),
        (
            "host.xz:~user/path/to/repo.git/",
            (None, None, "host.xz", None, "~user/path/to/repo.git"),
        ),
        ("user@host.xz:path/to/repo.git", (None, "user", "host.xz", None, "path/to/repo.git")),
        ("host.xz:path/to/repo.git", (None, None, "host.xz", None, "path/to/repo.git")),
        (
            "rsync://host.xz/path/to/repo.git/",
            ("rsync", None, "host.xz", None, "/path/to/repo.git"),
        ),
        ("git://host.xz/path/to/repo.git/", ("git", None, "host.xz", None, "/path/to/repo.git")),
        (
            "git://host.xz/~user/path/to/repo.git/",
            ("git", None, "host.xz", None, "~user/path/to/repo.git"),
        ),
        ("http://host.xz/path/to/repo.git/", ("http", None, "host.xz", None, "/path/to/repo.git")),
        (
            "https://host.xz/path/to/repo.git/",
            ("https", None, "host.xz", None, "/path/to/repo.git"),
        ),
        ("https://github.com/spack/spack", ("https", None, "github.com", None, "/spack/spack")),
        ("https://github.com/spack/spack/", ("https", None, "github.com", None, "/spack/spack")),
        ("file:///path/to/repo.git/", ("file", None, None, None, "/path/to/repo.git")),
        ("file://~/path/to/repo.git/", ("file", None, None, None, "~/path/to/repo.git")),
        # bad ports should give us None
        ("ssh://host.xz:port/path/to/repo.git/", None),
        # bad ports should give us None
        ("ssh://host-foo.xz:port/path/to/repo.git/", None),
        # regular file paths should give us None
        ("/path/to/repo.git/", None),
        ("path/to/repo.git/", None),
        ("~/path/to/repo.git", None),
    ],
)
def test_git_url_parse(url, parts):
    if parts is None:
        with pytest.raises(ValueError):
            url_util.parse_git_url(url)
    else:
        assert parts == url_util.parse_git_url(url)
