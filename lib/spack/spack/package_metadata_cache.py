# Copyright Spack Project Developers. See COPYRIGHT file for details.
#
# SPDX-License-Identifier: (Apache-2.0 OR MIT)
"""Static package metadata snapshots.

A snapshot captures, per package, every directive-produced attribute the concretizer reads,
so that a solve can run entirely without importing package modules. Snapshots are pickled
per repository and invalidated by ``package.py`` (and build system) mtimes, like the
provider/tag/patch indexes. When a repository serves a fresh snapshot, importing its package
modules is blocked outright (see :class:`HermeticImportGuard`).
"""

import base64
import builtins
import hashlib
import marshal
import os
import pickle
import sys
import types
from typing import Any, Callable, Dict, List, Optional, Tuple

import spack.error

#: bump when the snapshot format or the captured surface changes
SNAPSHOT_VERSION = 2

#: simple class attributes captured for ``version_or_package_attr`` and fetcher construction
_CAPTURED_CLASS_ATTRS = (
    "git",
    "url",
    "urls",
    "list_url",
    "commit",
    "tag",
    "branch",
    "homepage",
    "keep_werror",
    "manual_download",
    "build_system_class",
    "maintainers",
    "submodules",
    "git_sparse_paths",
    "get_full_repo",
    "fetch_options",
)

#: types a captured class attribute may have; anything else (e.g. a callable) is dropped
_SIMPLE_ATTR_TYPES = (str, int, bool, tuple, list, dict)


class _OpaqueCallable:
    """Stands in for a package-local callable that cannot be serialized.

    Permissive on purpose: it is only ever used as a variant value predicate or group
    validator, where the fallback is to accept the value and let the build fail later.
    """

    __slots__ = ()

    def __call__(self, *args, **kwargs) -> bool:
        return True

    def __repr__(self) -> str:
        return "<opaque callable>"

    def __reduce__(self):
        return (_OpaqueCallable, ())


OPAQUE_CALLABLE = _OpaqueCallable()


def _is_package_local(module_name: Optional[str]) -> bool:
    import spack.repo

    return module_name is not None and spack.repo.is_package_module(module_name)


class _SanitizingPickler(pickle.Pickler):
    """Pickler that replaces callables which live in package modules (or are closures)
    with :data:`OPAQUE_CALLABLE`, so a snapshot never has to import a package module."""

    def reducer_override(self, obj):
        if isinstance(obj, types.FunctionType):
            if "<locals>" in obj.__qualname__ or _is_package_local(obj.__module__):
                return (_OpaqueCallable, ())
        elif isinstance(obj, types.MethodType):
            if _is_package_local(type(obj.__self__).__module__):
                return (_OpaqueCallable, ())
        elif isinstance(obj, type) and _is_package_local(obj.__module__):
            # e.g. a package-local type used as a variant value filter
            return (_OpaqueCallable, ())
        return NotImplemented


def sanitized_dumps(obj: Any) -> bytes:
    import io

    buf = io.BytesIO()
    _SanitizingPickler(buf, protocol=pickle.HIGHEST_PROTOCOL).dump(obj)
    return buf.getvalue()


class _FakeBase(types.SimpleNamespace):
    """Stand-in for an entry of ``cls.__mro__`` (name/module only)."""


class StaticPackage:
    """Drop-in stand-in for a package class, holding only directive metadata.

    Exposes the attributes and pure classmethod equivalents the concretizer reads, and can
    be "instantiated" like a class: calling it with a spec returns a
    :class:`StaticPackageInstance`.
    """

    __slots__ = (
        "name",
        "namespace",
        "fullname",
        "fullnames",
        "package_dir",
        "virtual",
        "has_code",
        "tags",
        "versions",
        "dependencies",
        "extendees",
        "conflicts",
        "requirements",
        "provided",
        "provided_together",
        "patches",
        "_patches_dependencies",
        "variants",
        "splice_specs",
        "resources",
        "licenses",
        "disable_redistribute",
        "_class_attrs",
        "_source_ids",
        "_runtime_constraints_code",
        "_base_classes",
        "_runtime_constraints_fn",
        "_doc",
        "_version_urls",
    )

    name: str
    namespace: str
    fullname: str
    fullnames: List[str]
    package_dir: str
    virtual: bool
    has_code: bool
    tags: Tuple[str, ...]
    versions: Dict[Any, Dict[str, Any]]
    dependencies: Dict[Any, Any]
    extendees: Dict[Any, Any]
    conflicts: Dict[Any, Any]
    requirements: Dict[Any, Any]
    provided: Dict[Any, Any]
    provided_together: Dict[Any, Any]
    patches: Dict[Any, Any]
    _patches_dependencies: bool
    variants: Dict[Any, Any]
    splice_specs: Dict[Any, Any]
    resources: Dict[Any, Any]
    licenses: Dict[Any, Any]
    disable_redistribute: Dict[Any, Any]
    _class_attrs: Dict[str, Any]
    _source_ids: Dict[Any, Optional[str]]
    _runtime_constraints_code: Optional[bytes]
    _base_classes: Tuple[_FakeBase, ...]
    _runtime_constraints_fn: Optional[Callable]
    _doc: Optional[str]
    _version_urls: Dict[Any, List[str]]

    def __getattr__(self, name: str):
        # only reached for names not in __slots__: captured simple class attributes, and
        # the runtime_constraints callback, which must be absent when the package has none
        # so that ``hasattr`` checks stay faithful
        if name == "runtime_constraints":
            if object.__getattribute__(self, "_runtime_constraints_code") is None:
                raise AttributeError(name)
            return object.__getattribute__(self, "_rebuild_runtime_constraints")()
        try:
            return object.__getattribute__(self, "_class_attrs")[name]
        except KeyError:
            raise AttributeError(name) from None

    def __repr__(self) -> str:
        return f"<StaticPackage {self.fullname}>"

    def __call__(self, spec) -> "StaticPackageInstance":
        return StaticPackageInstance(self, spec)

    @property
    def __mro__(self) -> Tuple[_FakeBase, ...]:
        return self._base_classes

    # -- pure classmethod equivalents (same implementations as PackageBase) ----------

    def dependency_names(self):
        import spack.package_base as pb

        return pb._subkeys(self.dependencies)

    def dependencies_by_name(self, when: bool = False):
        import spack.package_base as pb

        return pb._by_subkey(self.dependencies, when=when)

    def variant_names(self) -> List[str]:
        import spack.package_base as pb

        return pb._subkeys(self.variants)

    def has_variant(self, name: str) -> bool:
        import spack.package_base as pb

        return pb._has_subkey(self.variants, name)

    def num_variant_definitions(self) -> int:
        import spack.package_base as pb

        return pb._num_definitions(self.variants)

    def variant_definitions(self, name: str):
        import spack.package_base as pb

        return pb._definitions(self.variants, name)

    @property  # type: ignore[misc]
    def __doc__(self) -> Optional[str]:  # type: ignore[override]
        return self._doc

    def format_doc(self, **kwargs) -> str:
        import spack.package_base as pb

        return pb.PackageBase.format_doc.__func__(self, **kwargs)  # type: ignore[attr-defined]

    def variant_items(self):
        import spack.package_base as pb

        return pb.PackageBase.variant_items.__func__(self)

    def validate_variant_names(self, spec) -> None:
        import spack.package_base as pb

        return pb.PackageBase.validate_variant_names.__func__(  # type: ignore[attr-defined]
            self, spec
        )

    def provided_virtual_names(self):
        return sorted(
            {vpkg.name for virtuals in self.provided.values() for vpkg in sorted(virtuals)}
        )

    def all_urls_for_version(self, version) -> List[str]:
        # precomputed at capture time, so custom ``url_for_version`` overrides are exact
        return self._version_urls.get(version, [])

    def url_for_version(self, version) -> str:
        urls = self.all_urls_for_version(version)
        if not urls:
            raise spack.error.NoURLError(types.SimpleNamespace(__name__=self.name))
        return urls[0]

    def needs_commit(self, version) -> bool:
        from spack.version import GitVersion

        if isinstance(version, GitVersion):
            return True
        ver_attrs = self.versions.get(version)
        if ver_attrs:
            return bool(ver_attrs.get("commit") or ver_attrs.get("tag") or ver_attrs.get("branch"))
        return False

    def version_or_package_attr(self, attr, version, default=...):
        version_attrs = self.versions.get(version)
        if version_attrs and attr in version_attrs:
            return version_attrs.get(attr)
        if default is ... and not hasattr(self, attr):
            raise spack.error.PackageError(f"{attr} attribute not defined on {self.name}")
        return getattr(self, attr, None if default is ... else default)

    def _rebuild_runtime_constraints(self) -> Callable:
        fn = object.__getattribute__(self, "_runtime_constraints_fn")
        if fn is None:
            code = object.__getattribute__(self, "_runtime_constraints_code")
            fn = types.FunctionType(marshal.loads(code), {"__builtins__": builtins})
            object.__setattr__(self, "_runtime_constraints_fn", fn)
        return types.MethodType(fn, self)


class StaticPackageInstance:
    """What ``StaticPackage(spec)`` returns: the small instance surface the solver uses."""

    __slots__ = ("meta", "spec")

    def __init__(self, meta: StaticPackage, spec):
        self.meta = meta
        self.spec = spec

    def __getattr__(self, name: str):
        return getattr(object.__getattribute__(self, "meta"), name)

    @property  # type: ignore[misc]
    def __doc__(self) -> Optional[str]:  # type: ignore[override]
        return self.meta._doc

    def __repr__(self) -> str:
        return f"<StaticPackageInstance {self.meta.fullname} {self.spec}>"

    def intersects(self, spec) -> bool:
        import spack.package_base as pb

        return pb.PackageBase.intersects(self, spec)  # type: ignore[arg-type]

    def provides(self, vpkg_name) -> bool:
        return any(
            any(vpkg.name == vpkg_name for vpkg in provided) and self.spec.intersects(when)
            for when, provided in self.meta.provided.items()
        )

    @property
    def virtuals_provided(self):
        return [
            vspec
            for when_spec, provided in self.meta.provided.items()
            for vspec in sorted(provided)
            if self.spec.satisfies(when_spec)
        ]

    def content_hash(self, content: Optional[bytes] = None) -> str:
        """Same digest as ``PackageBase.content_hash``, from snapshot data."""
        from spack.util.package_hash import package_hash

        hash_content = []
        if self.spec.versions.concrete:
            source_id = self.meta._source_ids.get(self.spec.version)
            hash_content.append((source_id or "").encode("utf-8"))
        if self.spec._patches_assigned():
            hash_content.extend(
                ":".join((p.sha256, str(p.level))).encode("utf-8") for p in self.spec.patches
            )
        hash_content.append(package_hash(self.spec, source=content).encode("utf-8"))
        return (
            base64.b32encode(hashlib.sha256(b"".join(sorted(hash_content))).digest())
            .decode("utf-8")
            .lower()
        )


def _source_id_for(kwargs: dict) -> Optional[str]:
    """The ``source_id`` a fetcher built from these version kwargs would report."""
    import spack.util.crypto

    for h in spack.util.crypto.hashes:
        if h in kwargs:
            return kwargs[h]
    if "checksum" in kwargs:
        return kwargs["checksum"]
    if "commit" in kwargs:
        return kwargs["commit"]
    if "revision" in kwargs:
        return str(kwargs["revision"])
    return None


def _capture_version_urls(cls) -> Dict[Any, List[str]]:
    """Precompute ``all_urls_for_version`` for every known version, so that custom
    ``url_for_version`` overrides are exact without package code."""
    import spack.package_base
    import spack.spec

    if cls.virtual or not cls.has_code or not cls.versions:
        return {}
    try:
        spec = spack.spec.Spec(cls.name)
        pkg = cls(spec)
        has_custom_url = (
            cls.url_for_version is not spack.package_base.PackageBase.url_for_version
        )
    except Exception:
        return {}
    urls = {}
    for v in cls.versions:
        try:
            # a fresh instance per version, since custom ``url_for_version`` overrides
            # may keep state on the instance (e.g. gcc)
            if has_custom_url:
                pkg = cls(spec)
            urls[v] = list(pkg.all_urls_for_version(v))
        except Exception:
            urls[v] = []
    return urls


def static_package_from_class(cls) -> StaticPackage:
    """Capture a real package class into a :class:`StaticPackage`."""
    import spack.repo

    result = StaticPackage.__new__(StaticPackage)
    result.name = cls.name
    result.namespace = cls.namespace
    result.fullname = cls.fullname
    result.fullnames = list(cls.fullnames)
    result.package_dir = cls.package_dir
    result.virtual = cls.virtual
    result.has_code = cls.has_code
    result.tags = tuple(getattr(cls, "tags", ()) or ())
    result._doc = cls.__doc__

    result.versions = cls.versions
    result.dependencies = cls.dependencies
    result.extendees = cls.extendees
    result.conflicts = cls.conflicts
    result.requirements = cls.requirements
    result.provided = cls.provided
    result.provided_together = cls.provided_together
    result.patches = cls.patches
    result._patches_dependencies = getattr(cls, "_patches_dependencies", False)
    result.variants = cls.variants
    result.splice_specs = cls.splice_specs
    result.resources = cls.resources
    result.licenses = cls.licenses
    result.disable_redistribute = cls.disable_redistribute

    attrs = {}
    for name in _CAPTURED_CLASS_ATTRS:
        value = getattr(cls, name, None)
        if value is not None and isinstance(value, _SIMPLE_ATTR_TYPES):
            attrs[name] = value
    result._class_attrs = attrs

    result._source_ids = {v: _source_id_for(kw) for v, kw in cls.versions.items()}
    result._version_urls = _capture_version_urls(cls)

    rc = None
    for base in cls.__mro__:
        member = base.__dict__.get("runtime_constraints")
        if member is not None:
            fn = member.__func__ if isinstance(member, (classmethod, staticmethod)) else member
            rc = marshal.dumps(fn.__code__)
            break
    result._runtime_constraints_code = rc
    result._runtime_constraints_fn = None

    bases = []
    for base in cls.__mro__:
        fake = _FakeBase(__name__=base.__name__, __module__=base.__module__)
        # FilePatch resolves patch files by walking the MRO for `module.__file__`
        if spack.repo.is_package_module(base.__module__):
            module_file = getattr(getattr(base, "module", None), "__file__", None)
            if module_file:
                fake.module = types.SimpleNamespace(__file__=module_file)
        bases.append(fake)
    result._base_classes = tuple(bases)
    return result


class HermeticImportGuard:
    """Meta path finder that forbids importing package modules of hermetic repos."""

    def __init__(self) -> None:
        self.blocked_prefixes: List[str] = []

    def add_namespace(self, full_namespace: str) -> None:
        self.blocked_prefixes.append(f"{full_namespace}.")

    def find_spec(self, name, path=None, target=None):
        for prefix in self.blocked_prefixes:
            if name.startswith(prefix) or name == prefix.rstrip("."):
                raise RuntimeError(
                    f"cannot import package module '{name}': its repository is served from "
                    f"the static metadata cache (hermetic mode)"
                )
        return None


#: process-wide guard; installed on first hermetic repo
IMPORT_GUARD = HermeticImportGuard()


def _snapshot_path(repo) -> str:
    import spack.caches

    root = spack.caches.misc_cache_location()
    pyver = f"py{sys.version_info[0]}.{sys.version_info[1]}"
    return os.path.join(
        str(root), "static_metadata", f"{repo.namespace}-v{SNAPSHOT_VERSION}-{pyver}.pickle"
    )


def _newest_source_mtime(repo) -> float:
    """Newest mtime among package.py files and repo-level python sources (build systems)."""
    newest = 0.0
    checker = repo._pkg_checker
    if len(checker) > 0:
        newest = checker.last_mtime()
    # build_systems and other shared repo code also shape directive output
    for dirpath, _, filenames in os.walk(os.path.join(repo.root, "build_systems")):
        for f in filenames:
            if f.endswith(".py"):
                try:
                    newest = max(newest, os.stat(os.path.join(dirpath, f)).st_mtime)
                except OSError:
                    pass
    return newest


def load_or_build_snapshot(repo) -> Tuple[Dict[str, StaticPackage], bool]:
    """Return ``(snapshot, was_fresh)`` for a repo; builds and saves when stale.

    ``was_fresh`` is True when the snapshot was loaded without importing any package
    module, in which case hermetic import blocking is sound.
    """
    path = _snapshot_path(repo)
    names = set(repo.all_package_names(include_virtuals=True))
    try:
        st = os.stat(path)
        if st.st_mtime > _newest_source_mtime(repo):
            with open(path, "rb") as f:
                snapshot = pickle.load(f)
            if set(snapshot) == names:
                return snapshot, True
    except (OSError, pickle.UnpicklingError, EOFError):
        pass

    snapshot = {name: static_package_from_class(repo.get_pkg_class(name)) for name in names}
    os.makedirs(os.path.dirname(path), exist_ok=True)
    tmp = f"{path}.tmp.{os.getpid()}"
    with open(tmp, "wb") as f:
        _SanitizingPickler(f, protocol=pickle.HIGHEST_PROTOCOL).dump(snapshot)
    os.replace(tmp, path)
    return snapshot, False


def static_metadata_enabled() -> bool:
    return os.environ.get("SPACK_STATIC_METADATA", "").lower() in ("1", "true", "yes")
