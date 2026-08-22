// SPDX-License-Identifier: (Apache-2.0 OR MIT)

//! PyO3 bindings for the version algebra in `spack-spec-core`, mirroring the classes and
//! module-level functions of `spack.version.version_types`.
//!
//! Layout: `VersionType` <- `ConcreteVersion` <- {`StandardVersion`, `GitVersion`}, and
//! `VersionType` <- {`ClosedOpenRange`, `VersionList`}, so Python `isinstance` checks hold.
//! `VersionList` stores its elements as shared Python objects (like the Python list) so that
//! `attach_lookup` / `std_version` mutations through `list[i]` are visible to the list; the
//! set algebra converts elements to core values per operation.
//!
//! Git ref lookups stay lazy: operations run on unresolved snapshots first, and only when the
//! core reports `LookupNeeded` are attached lookups consulted (then cached on the wrapper,
//! as Python caches `std_version`) and the operation retried.

use std::cmp::Ordering;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use pyo3::basic::CompareOp;
use pyo3::exceptions::{
    PyIndexError, PyNotImplementedError, PyRuntimeError, PyTypeError, PyValueError,
};
use pyo3::prelude::*;
use pyo3::sync::PyOnceLock;
use pyo3::types::{PyDict, PyFloat, PyInt, PyList, PySlice, PyString, PyTuple, PyType};
use pyo3::IntoPyObjectExt;

use spack_spec_core::version as cv;
use spack_spec_core::version::{VersionError, VersionItem, VersionUnion};

use crate::registry;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Instantiate a registered Python exception class with a message.
fn registered_err(py: Python<'_>, slot: &PyOnceLock<Py<PyAny>>, msg: String) -> PyErr {
    match slot.get(py) {
        Some(cls) => match cls.bind(py).call1((&msg,)) {
            Ok(exc) => PyErr::from_value(exc),
            Err(e) => e,
        },
        None => PyRuntimeError::new_err(msg),
    }
}

/// Map a core `VersionError` to the Python exception the reference implementation raises.
fn verr(py: Python<'_>, e: VersionError) -> PyErr {
    match e {
        VersionError::Parse(m) => PyValueError::new_err(m),
        VersionError::Type(m) => PyTypeError::new_err(m),
        VersionError::EmptyRange(m) => registered_err(py, &registry::EMPTY_RANGE_ERROR, m),
        VersionError::LookupNeeded(m) => registered_err(py, &registry::VERSION_LOOKUP_ERROR, m),
    }
}

fn is_lookup_error(py: Python<'_>, e: &PyErr) -> bool {
    match registry::VERSION_LOOKUP_ERROR.get(py) {
        Some(cls) => e.matches(py, cls.bind(py)).unwrap_or(false),
        None => false,
    }
}

/// Python `{type(x)}` as used in the reference error messages, e.g. `<class 'str'>`.
fn type_repr(obj: &Bound<'_, PyAny>) -> String {
    obj.get_type()
        .repr()
        .map(|r| r.to_string())
        .unwrap_or_else(|_| "<unknown type>".to_string())
}

// ---------------------------------------------------------------------------
// Hashing (self-consistent stand-ins for Python's tuple hashes)
// ---------------------------------------------------------------------------

/// CPython reserves -1 for errors in `tp_hash`.
fn normalize_hash(h: u64) -> isize {
    let v = h as i64 as isize;
    if v == -1 {
        -2
    } else {
        v
    }
}

fn sv_hash(v: &cv::StandardVersion) -> isize {
    let mut h = DefaultHasher::new();
    v.hash(&mut h);
    normalize_hash(h.finish())
}

fn git_hash(v: &cv::GitVersion) -> isize {
    // Like Python, hash only the ref so that hashing never triggers a lookup.
    let mut h = DefaultHasher::new();
    v.hash(&mut h);
    normalize_hash(h.finish())
}

/// Python `hash((lo, prev(hi)))` for a range.
fn range_hash(r: &cv::ClosedOpenRange) -> isize {
    let mut h = DefaultHasher::new();
    r.lo().hash(&mut h);
    cv::prev_version(r.hi()).hash(&mut h);
    normalize_hash(h.finish())
}

// ---------------------------------------------------------------------------
// Class hierarchy
// ---------------------------------------------------------------------------

/// Base type for all versions (`spack.version.VersionType`).
#[pyclass(subclass, name = "VersionType", module = "spack_spec")]
pub struct VersionType;

/// Base type for single (non-range, non-list) versions.
#[pyclass(subclass, extends = VersionType, name = "ConcreteVersion", module = "spack_spec")]
pub struct ConcreteVersion;

/// A single concrete version (`spack.version.StandardVersion`).
#[pyclass(extends = ConcreteVersion, name = "StandardVersion", module = "spack_spec")]
pub struct StandardVersion {
    inner: cv::StandardVersion,
    /// Cached string form; `None`/empty means "recompute from components" like Python's
    /// `_string` after `v.string = None`.
    string: Option<String>,
}

/// A version interpreted from a git ref (`spack.version.GitVersion`).
#[pyclass(extends = ConcreteVersion, name = "GitVersion", module = "spack_spec")]
pub struct GitVersion {
    inner: cv::GitVersion,
    lookup: Option<Py<PyAny>>,
}

/// The half-open version interval `[lo, hi)` (`spack.version.ClosedOpenRange`).
#[pyclass(extends = VersionType, name = "ClosedOpenRange", module = "spack_spec")]
pub struct ClosedOpenRange {
    inner: cv::ClosedOpenRange,
    string: Option<String>,
    hash: Option<isize>,
}

/// Sorted, non-redundant list of versions and ranges (`spack.version.VersionList`).
#[pyclass(extends = VersionType, name = "VersionList", module = "spack_spec")]
pub struct VersionList {
    items: Vec<Py<PyAny>>,
}

// ---------------------------------------------------------------------------
// Wrapper construction
// ---------------------------------------------------------------------------

fn sv_init(
    inner: cv::StandardVersion,
    string: Option<String>,
) -> PyClassInitializer<StandardVersion> {
    PyClassInitializer::from(VersionType)
        .add_subclass(ConcreteVersion)
        .add_subclass(StandardVersion { inner, string })
}

fn new_sv(py: Python<'_>, inner: cv::StandardVersion) -> PyResult<Py<PyAny>> {
    let s = inner.to_string();
    Py::new(py, sv_init(inner, Some(s)))?.into_py_any(py)
}

fn git_init(inner: cv::GitVersion, lookup: Option<Py<PyAny>>) -> PyClassInitializer<GitVersion> {
    PyClassInitializer::from(VersionType)
        .add_subclass(ConcreteVersion)
        .add_subclass(GitVersion { inner, lookup })
}

fn new_git(
    py: Python<'_>,
    inner: cv::GitVersion,
    lookup: Option<Py<PyAny>>,
) -> PyResult<Py<PyAny>> {
    Py::new(py, git_init(inner, lookup))?.into_py_any(py)
}

fn range_init(inner: cv::ClosedOpenRange) -> PyClassInitializer<ClosedOpenRange> {
    let string = Some(inner.to_string());
    let hash = Some(range_hash(&inner));
    PyClassInitializer::from(VersionType).add_subclass(ClosedOpenRange {
        inner,
        string,
        hash,
    })
}

fn new_range(py: Python<'_>, inner: cv::ClosedOpenRange) -> PyResult<Py<PyAny>> {
    Py::new(py, range_init(inner))?.into_py_any(py)
}

fn list_init(items: Vec<Py<PyAny>>) -> PyClassInitializer<VersionList> {
    PyClassInitializer::from(VersionType).add_subclass(VersionList { items })
}

fn new_list(py: Python<'_>, items: Vec<Py<PyAny>>) -> PyResult<Py<PyAny>> {
    Py::new(py, list_init(items))?.into_py_any(py)
}

fn item_to_py(py: Python<'_>, item: &VersionItem) -> PyResult<Py<PyAny>> {
    match item {
        VersionItem::Version(v) => new_sv(py, v.clone()),
        VersionItem::Git(v) => new_git(py, v.clone(), None),
        VersionItem::Range(v) => new_range(py, v.clone()),
    }
}

fn core_list_to_items(py: Python<'_>, l: &cv::VersionList) -> PyResult<Vec<Py<PyAny>>> {
    l.versions().iter().map(|it| item_to_py(py, it)).collect()
}

fn union_to_py(py: Python<'_>, u: VersionUnion) -> PyResult<Py<PyAny>> {
    match u {
        VersionUnion::Version(v) => new_sv(py, v),
        VersionUnion::Git(v) => new_git(py, v, None),
        VersionUnion::Range(v) => new_range(py, v),
        VersionUnion::List(l) => {
            let items = core_list_to_items(py, &l)?;
            new_list(py, items)
        }
    }
}

// ---------------------------------------------------------------------------
// Snapshots and lazy lookup resolution
// ---------------------------------------------------------------------------

enum OpErr {
    Ver(VersionError),
    Py(PyErr),
}

impl From<PyErr> for OpErr {
    fn from(e: PyErr) -> Self {
        OpErr::Py(e)
    }
}

/// Core snapshot of a version object; `None` when the object is not a version type.
/// Does not resolve git lookups: core operations report `LookupNeeded` when required.
fn snapshot(obj: &Bound<'_, PyAny>) -> Result<Option<VersionUnion>, OpErr> {
    if let Ok(v) = obj.downcast::<StandardVersion>() {
        return Ok(Some(VersionUnion::Version(v.borrow().inner.clone())));
    }
    if let Ok(v) = obj.downcast::<GitVersion>() {
        return Ok(Some(VersionUnion::Git(v.borrow().inner.clone())));
    }
    if let Ok(v) = obj.downcast::<ClosedOpenRange>() {
        return Ok(Some(VersionUnion::Range(v.borrow().inner.clone())));
    }
    if let Ok(v) = obj.downcast::<VersionList>() {
        let items: Vec<Py<PyAny>> = v.borrow().items.clone();
        let mut list = cv::VersionList::new();
        for item in &items {
            match snapshot(item.bind(obj.py()))? {
                Some(u) => list.add(u).map_err(OpErr::Ver)?,
                None => {
                    return Err(OpErr::Py(PyTypeError::new_err(
                        "VersionList contains a non-version element",
                    )))
                }
            }
        }
        return Ok(Some(VersionUnion::List(list)));
    }
    Ok(None)
}

fn snapshot_required(obj: &Bound<'_, PyAny>) -> Result<VersionUnion, OpErr> {
    snapshot(obj)?.ok_or_else(|| {
        OpErr::Py(PyTypeError::new_err(format!(
            "expected a version object, got {}",
            type_repr(obj)
        )))
    })
}

/// Resolve the ref version of every git version (recursing into lists) that has an attached
/// lookup, caching the result on the wrapper like Python caches `std_version`. Returns
/// whether any resolution happened.
fn resolve_attached(py: Python<'_>, obj: &Bound<'_, PyAny>) -> PyResult<bool> {
    if let Ok(g) = obj.downcast::<GitVersion>() {
        let (needs, lookup, git_ref) = {
            let b = g.borrow();
            (
                b.inner.needs_lookup(),
                b.lookup.as_ref().map(|l| l.clone_ref(py)),
                b.inner.git_ref().to_string(),
            )
        };
        if !needs {
            return Ok(false);
        }
        let Some(lookup) = lookup else {
            return Ok(false);
        };
        // Call the Python lookup without holding a borrow of the wrapper. Like Python's
        // tuple unpacking, accept any two-element sequence (the JSON-backed lookup cache
        // returns lists).
        let result = lookup.bind(py).call_method1("get", (git_ref.as_str(),))?;
        let version: Option<String> = result.get_item(0)?.extract()?;
        let distance: i64 = result.get_item(1)?.extract()?;
        let mut b = g.borrow_mut();
        if b.inner.needs_lookup() {
            b.inner
                .resolve_lookup(version.as_deref(), distance.max(0) as u64)
                .map_err(|e| verr(py, e))?;
        }
        return Ok(true);
    }
    if let Ok(l) = obj.downcast::<VersionList>() {
        let items: Vec<Py<PyAny>> = l.borrow().items.clone();
        let mut any = false;
        for item in &items {
            any |= resolve_attached(py, item.bind(py))?;
        }
        return Ok(any);
    }
    Ok(false)
}

/// Run a core operation over snapshots of the given objects. On `LookupNeeded`, resolve
/// attached git lookups and retry once, so lookups stay as lazy as in Python.
fn run_op<T>(
    py: Python<'_>,
    objs: &[&Bound<'_, PyAny>],
    f: impl Fn(&[VersionUnion]) -> Result<T, VersionError>,
) -> PyResult<T> {
    let attempt = |f: &dyn Fn(&[VersionUnion]) -> Result<T, VersionError>| -> Result<T, OpErr> {
        let units: Vec<VersionUnion> = objs
            .iter()
            .map(|o| snapshot_required(o))
            .collect::<Result<_, _>>()?;
        f(&units).map_err(OpErr::Ver)
    };
    match attempt(&f) {
        Ok(t) => Ok(t),
        Err(OpErr::Ver(VersionError::LookupNeeded(m))) => {
            let mut resolved = false;
            for o in objs {
                resolved |= resolve_attached(py, o)?;
            }
            if !resolved {
                return Err(verr(py, VersionError::LookupNeeded(m)));
            }
            match attempt(&f) {
                Ok(t) => Ok(t),
                Err(OpErr::Ver(e)) => Err(verr(py, e)),
                Err(OpErr::Py(e)) => Err(e),
            }
        }
        Err(OpErr::Ver(e)) => Err(verr(py, e)),
        Err(OpErr::Py(e)) => Err(e),
    }
}

/// Whether the object is one of the version classes (Python `isinstance(x, VersionType)`).
fn is_version_obj(obj: &Bound<'_, PyAny>) -> bool {
    obj.is_instance_of::<VersionType>()
}

fn not_supported(method: &str, obj: &Bound<'_, PyAny>) -> PyErr {
    PyTypeError::new_err(format!(
        "'{method}()' not supported for instances of {}",
        type_repr(obj)
    ))
}

// ---------------------------------------------------------------------------
// StandardVersion helpers shared with GitVersion's delegating accessors
// ---------------------------------------------------------------------------

fn component_to_py(py: Python<'_>, c: &cv::Component) -> PyResult<Py<PyAny>> {
    match c {
        cv::Component::Int(i) => i.into_py_any(py),
        cv::Component::Str(s) => s.to_string().into_py_any(py),
    }
}

fn sv_components_list<'py>(
    py: Python<'py>,
    v: &cv::StandardVersion,
) -> PyResult<Bound<'py, PyList>> {
    let comps: Vec<Py<PyAny>> = v
        .release()
        .iter()
        .map(|c| component_to_py(py, c))
        .collect::<PyResult<_>>()?;
    PyList::new(py, comps)
}

/// Shared `__getitem__` transcription for StandardVersion and GitVersion (via ref version).
fn sv_getitem(
    py: Python<'_>,
    v: &cv::StandardVersion,
    idx: &Bound<'_, PyAny>,
    cls_name: &str,
) -> PyResult<Py<PyAny>> {
    if let Ok(i) = idx.extract::<isize>() {
        return match v.component(i) {
            Some(c) => component_to_py(py, c),
            None => Err(PyIndexError::new_err("tuple index out of range")),
        };
    }
    if let Ok(slice) = idx.downcast::<PySlice>() {
        let start: Option<isize> = slice.getattr("start")?.extract()?;
        let stop: Option<isize> = slice.getattr("stop")?.extract()?;
        let step: Option<isize> = slice.getattr("step")?.extract()?;
        if !matches!(step, None | Some(1)) {
            return Err(PyValueError::new_err(
                "version slices with a step are not supported",
            ));
        }
        return new_sv(py, v.slice(start, stop));
    }
    Err(PyTypeError::new_err(format!(
        "{cls_name} indices must be integers or slices"
    )))
}

/// Stringify from components (Python's lazy `_stringify_version`), bypassing any string
/// cached inside the core value: `prev(next(v))` reproduces the components with no string.
fn sv_stringify(v: &cv::StandardVersion) -> String {
    cv::prev_version(&cv::next_version(v)).to_string()
}

/// Python `_str_range(lo, hi)` over inclusive bounds.
fn str_range(lo: &cv::StandardVersion, hi_incl: &cv::StandardVersion) -> String {
    let tmin = cv::StandardVersion::typemin();
    let tmax = cv::StandardVersion::typemax();
    if *lo == tmin {
        if *hi_incl == tmax {
            ":".to_string()
        } else {
            format!(":{hi_incl}")
        }
    } else if *hi_incl == tmax {
        format!("{lo}:")
    } else if lo == hi_incl {
        lo.to_string()
    } else {
        format!("{lo}:{hi_incl}")
    }
}

#[pymethods]
impl StandardVersion {
    /// Unlike Python's three-argument `__init__`, the binding constructor takes the version
    /// string; external code only constructs through `from_string` and pickling.
    #[new]
    fn new(py: Python<'_>, string: &str) -> PyResult<PyClassInitializer<Self>> {
        let inner = cv::StandardVersion::from_string(string).map_err(|e| verr(py, e))?;
        Ok(sv_init(inner, Some(string.to_string())))
    }

    #[staticmethod]
    fn from_string(py: Python<'_>, string: &str) -> PyResult<Py<PyAny>> {
        let inner = cv::StandardVersion::from_string(string).map_err(|e| verr(py, e))?;
        Py::new(py, sv_init(inner, Some(string.to_string())))?.into_py_any(py)
    }

    #[staticmethod]
    fn typemin(py: Python<'_>) -> PyResult<Py<PyAny>> {
        new_sv(py, cv::StandardVersion::typemin())
    }

    #[staticmethod]
    fn typemax(py: Python<'_>) -> PyResult<Py<PyAny>> {
        new_sv(py, cv::StandardVersion::typemax())
    }

    #[getter]
    fn get_string(&mut self) -> String {
        if self.string.as_deref().is_none_or(str::is_empty) {
            self.string = Some(sv_stringify(&self.inner));
        }
        self.string.clone().expect("just filled")
    }

    #[setter]
    fn set_string(&mut self, value: Option<String>) {
        self.string = value;
    }

    /// Python's `(release, prerelease)` tuple; string components surface as `str`.
    #[getter]
    fn version<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        let release = PyTuple::new(py, sv_components_list(py, &self.inner)?.iter())?;
        let pre = self.inner.prerelease();
        let prerelease = match pre.num {
            Some(n) => PyTuple::new(py, [pre.kind as i64, n])?,
            None => PyTuple::new(py, [pre.kind as i64])?,
        };
        PyTuple::new(py, [release.as_any(), prerelease.as_any()])
    }

    fn __str__(&mut self) -> String {
        self.get_string()
    }

    fn __repr__(&mut self) -> String {
        format!("Version(\"{}\")", self.get_string())
    }

    fn __bool__(&self) -> bool {
        true
    }

    fn __hash__(&self) -> isize {
        sv_hash(&self.inner)
    }

    fn __len__(&self) -> usize {
        self.inner.len()
    }

    fn __iter__(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        sv_components_list(py, &self.inner)?
            .as_any()
            .try_iter()?
            .into_py_any(py)
    }

    fn __getitem__(&self, py: Python<'_>, idx: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        sv_getitem(py, &self.inner, idx, "StandardVersion")
    }

    fn __contains__(slf: &Bound<'_, Self>, lhs: &Bound<'_, PyAny>) -> PyResult<bool> {
        if !is_version_obj(lhs) {
            return Err(PyTypeError::new_err(format!(
                "'in' not supported for instances of {}",
                type_repr(lhs)
            )));
        }
        run_op(slf.py(), &[lhs, slf.as_any()], |u| u[0].satisfies(&u[1]))
    }

    fn __richcmp__(
        slf: &Bound<'_, Self>,
        other: &Bound<'_, PyAny>,
        op: CompareOp,
    ) -> PyResult<Py<PyAny>> {
        let py = other.py();
        if let Ok(b) = other.downcast::<StandardVersion>() {
            let a = &slf.borrow().inner;
            let b = &b.borrow().inner;
            let r = match op {
                CompareOp::Eq => a == b,
                CompareOp::Ne => a != b,
                CompareOp::Lt => a < b,
                CompareOp::Le => a <= b,
                CompareOp::Ge => a >= b,
                CompareOp::Gt => a > b,
            };
            return r.into_py_any(py);
        }
        if let Ok(b) = other.downcast::<ClosedOpenRange>() {
            // Versions are never equal to ranges; `<`/`<=` hold up to the range's lo bound
            // so that `Version(x) < VersionRange(x, y)`.
            let a = &slf.borrow().inner;
            let lo = b.borrow().inner.lo().clone();
            let r = match op {
                CompareOp::Eq => false,
                CompareOp::Ne => true,
                CompareOp::Lt | CompareOp::Le => *a <= lo,
                CompareOp::Ge | CompareOp::Gt => *a > lo,
            };
            return r.into_py_any(py);
        }
        match op {
            CompareOp::Eq => false.into_py_any(py),
            CompareOp::Ne => true.into_py_any(py),
            _ => Ok(py.NotImplemented()),
        }
    }

    fn intersects(slf: &Bound<'_, Self>, other: &Bound<'_, PyAny>) -> PyResult<bool> {
        if !is_version_obj(other) {
            return Err(not_supported("intersects", other));
        }
        run_op(slf.py(), &[slf.as_any(), other], |u| u[0].intersects(&u[1]))
    }

    fn overlaps(slf: &Bound<'_, Self>, other: &Bound<'_, PyAny>) -> PyResult<bool> {
        Self::intersects(slf, other)
    }

    fn satisfies(slf: &Bound<'_, Self>, other: &Bound<'_, PyAny>) -> PyResult<bool> {
        if !is_version_obj(other) {
            // Python falls through to `raise NotImplementedError`.
            return Err(PyNotImplementedError::new_err(()));
        }
        run_op(slf.py(), &[slf.as_any(), other], |u| u[0].satisfies(&u[1]))
    }

    fn union(slf: &Bound<'_, Self>, other: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        if other.is_instance_of::<GitVersion>() {
            // Python delegates to GitVersion.union, which is not implemented.
            return Err(PyNotImplementedError::new_err(()));
        }
        if !is_version_obj(other) {
            return Err(not_supported("union", other));
        }
        let py = slf.py();
        let u = run_op(py, &[slf.as_any(), other], |u| u[0].union(&u[1]))?;
        union_to_py(py, u)
    }

    fn intersection(slf: &Bound<'_, Self>, other: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        if !is_version_obj(other) {
            return Err(not_supported("intersection", other));
        }
        let py = slf.py();
        let u = run_op(py, &[slf.as_any(), other], |u| u[0].intersection(&u[1]))?;
        union_to_py(py, u)
    }

    fn isdevelop(&self) -> bool {
        self.inner.isdevelop()
    }

    fn is_prerelease(&self) -> bool {
        self.inner.is_prerelease()
    }

    #[getter]
    fn dotted_numeric_string(&self) -> String {
        self.inner.dotted_numeric_string()
    }

    #[getter]
    fn dotted(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        new_sv(py, self.inner.dotted())
    }

    #[getter]
    fn underscored(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        new_sv(py, self.inner.underscored())
    }

    #[getter]
    fn dashed(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        new_sv(py, self.inner.dashed())
    }

    #[getter]
    fn joined(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        new_sv(py, self.inner.joined())
    }

    fn up_to(&self, py: Python<'_>, index: isize) -> PyResult<Py<PyAny>> {
        new_sv(py, self.inner.up_to(index))
    }

    #[getter]
    fn up_to_1(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        self.up_to(py, 1)
    }

    #[getter]
    fn up_to_2(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        self.up_to(py, 2)
    }

    #[getter]
    fn up_to_3(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        self.up_to(py, 3)
    }

    fn __reduce__(slf: &Bound<'_, Self>) -> PyResult<(Py<PyAny>, (String,))> {
        let py = slf.py();
        let string = slf.borrow_mut().get_string();
        Ok((slf.get_type().into_py_any(py)?, (string,)))
    }
}

// ---------------------------------------------------------------------------
// GitVersion
// ---------------------------------------------------------------------------

/// The resolved reference version, consulting the attached lookup like Python's
/// `ref_version` property (and caching the result on the wrapper).
fn git_ref_version(slf: &Bound<'_, GitVersion>) -> PyResult<cv::StandardVersion> {
    let py = slf.py();
    {
        let b = slf.borrow();
        if let Ok(rv) = b.inner.ref_version() {
            return Ok(rv.clone());
        }
        if b.lookup.is_none() {
            return Err(registered_err(
                py,
                &registry::VERSION_LOOKUP_ERROR,
                format!(
                    "git ref '{}' cannot be looked up: call attach_lookup first",
                    b.inner.git_ref()
                ),
            ));
        }
    }
    resolve_attached(py, slf.as_any())?;
    slf.borrow()
        .inner
        .ref_version()
        .cloned()
        .map_err(|e| verr(py, e))
}

fn cmp_symbol(op: CompareOp) -> &'static str {
    match op {
        CompareOp::Lt => "<",
        CompareOp::Le => "<=",
        CompareOp::Gt => ">",
        CompareOp::Ge => ">=",
        CompareOp::Eq => "==",
        CompareOp::Ne => "!=",
    }
}

#[pymethods]
impl GitVersion {
    #[new]
    fn new(py: Python<'_>, string: &str) -> PyResult<PyClassInitializer<Self>> {
        let inner = cv::GitVersion::from_string(string).map_err(|e| verr(py, e))?;
        Ok(git_init(inner, None))
    }

    #[getter]
    #[pyo3(name = "ref")]
    fn get_ref(&self) -> String {
        self.inner.git_ref().to_string()
    }

    #[getter]
    fn has_git_prefix(&self) -> bool {
        self.inner.has_git_prefix()
    }

    #[getter]
    fn is_commit(&self) -> bool {
        self.inner.is_commit()
    }

    #[getter]
    fn commit_sha(&self) -> Option<String> {
        self.inner.commit_sha().map(str::to_string)
    }

    #[getter]
    fn std_version(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        match self.inner.std_version() {
            Some(v) => Ok(Some(new_sv(py, v.clone())?)),
            None => Ok(None),
        }
    }

    #[setter]
    fn set_std_version(&mut self, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let sv = value.downcast::<StandardVersion>().map_err(|_| {
            PyTypeError::new_err(format!(
                "std_version must be a StandardVersion, not {}",
                type_repr(value)
            ))
        })?;
        self.inner.set_ref_version(sv.borrow().inner.clone());
        Ok(())
    }

    #[getter]
    fn ref_version(slf: &Bound<'_, Self>) -> PyResult<Py<PyAny>> {
        let rv = git_ref_version(slf)?;
        new_sv(slf.py(), rv)
    }

    #[getter]
    fn _ref_lookup(&self, py: Python<'_>) -> Option<Py<PyAny>> {
        self.lookup.as_ref().map(|l| l.clone_ref(py))
    }

    #[getter]
    fn ref_lookup(slf: &Bound<'_, Self>) -> PyResult<Option<Py<PyAny>>> {
        let py = slf.py();
        let (lookup, git_ref) = {
            let b = slf.borrow();
            (
                b.lookup.as_ref().map(|l| l.clone_ref(py)),
                b.inner.git_ref().to_string(),
            )
        };
        match lookup {
            Some(l) => {
                // Python's property performs a get() to ensure the lookup is populated.
                l.bind(py).call_method1("get", (git_ref.as_str(),))?;
                Ok(Some(l))
            }
            None => Ok(None),
        }
    }

    fn attach_lookup(&mut self, lookup: Py<PyAny>) {
        self.lookup = Some(lookup);
    }

    fn __str__(slf: &Bound<'_, Self>) -> PyResult<String> {
        let py = slf.py();
        let (needs, has_lookup) = {
            let b = slf.borrow();
            (b.inner.needs_lookup(), b.lookup.is_some())
        };
        if needs && has_lookup {
            // Like Python, swallow only VersionLookupError and print the bare ref.
            if let Err(e) = resolve_attached(py, slf.as_any()) {
                if !is_lookup_error(py, &e) {
                    return Err(e);
                }
            }
        }
        Ok(slf.borrow().inner.to_string())
    }

    fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
        Ok(format!("GitVersion(\"{}\")", Self::__str__(slf)?))
    }

    fn __bool__(&self) -> bool {
        true
    }

    fn __hash__(&self) -> isize {
        git_hash(&self.inner)
    }

    fn __contains__(&self, _other: &Bound<'_, PyAny>) -> PyResult<bool> {
        Err(PyNotImplementedError::new_err(()))
    }

    fn __richcmp__(
        slf: &Bound<'_, Self>,
        other: &Bound<'_, PyAny>,
        op: CompareOp,
    ) -> PyResult<Py<PyAny>> {
        let py = other.py();
        if other.downcast::<GitVersion>().is_ok() {
            // Equality first: different refs are unequal without any lookup.
            if matches!(op, CompareOp::Eq | CompareOp::Ne) {
                let refs_equal = {
                    let a = slf.borrow();
                    let b = other.downcast::<GitVersion>().unwrap().borrow();
                    a.inner.git_ref() == b.inner.git_ref()
                };
                let eq = if refs_equal {
                    run_op(py, &[slf.as_any(), other], |u| u[0].satisfies(&u[1]))?
                } else {
                    false
                };
                return (if matches!(op, CompareOp::Eq) { eq } else { !eq }).into_py_any(py);
            }
            let cmp = run_op(py, &[slf.as_any(), other], |u| u[0].compare(&u[1]))?;
            let r = match op {
                CompareOp::Lt => cmp == Ordering::Less,
                CompareOp::Le => cmp != Ordering::Greater,
                CompareOp::Ge => cmp != Ordering::Less,
                CompareOp::Gt => cmp == Ordering::Greater,
                CompareOp::Eq | CompareOp::Ne => unreachable!("handled above"),
            };
            return r.into_py_any(py);
        }
        if matches!(op, CompareOp::Eq) {
            return false.into_py_any(py);
        }
        if matches!(op, CompareOp::Ne) {
            return true.into_py_any(py);
        }
        if other.is_instance_of::<StandardVersion>() {
            // A git version at equal ref version is larger than the standard version, so
            // `<`/`<=` need strict Less and `>=`/`>` accept the tie (which compares Greater).
            let cmp = run_op(py, &[slf.as_any(), other], |u| u[0].compare(&u[1]))?;
            let r = match op {
                CompareOp::Lt | CompareOp::Le => cmp == Ordering::Less,
                _ => cmp != Ordering::Less,
            };
            return r.into_py_any(py);
        }
        if other.is_instance_of::<ClosedOpenRange>() {
            // Versus a range the tie on the lo bound compares Less, so `>=`/`>` need strict
            // Greater.
            let cmp = run_op(py, &[slf.as_any(), other], |u| u[0].compare(&u[1]))?;
            let r = match op {
                CompareOp::Lt | CompareOp::Le => cmp == Ordering::Less,
                _ => cmp == Ordering::Greater,
            };
            return r.into_py_any(py);
        }
        Err(PyTypeError::new_err(format!(
            "'{}' not supported between instances of {} and {}",
            cmp_symbol(op),
            type_repr(slf.as_any()),
            type_repr(other)
        )))
    }

    fn intersects(slf: &Bound<'_, Self>, other: &Bound<'_, PyAny>) -> PyResult<bool> {
        if !is_version_obj(other) {
            return Err(not_supported("intersects", other));
        }
        run_op(slf.py(), &[slf.as_any(), other], |u| u[0].intersects(&u[1]))
    }

    fn overlaps(slf: &Bound<'_, Self>, other: &Bound<'_, PyAny>) -> PyResult<bool> {
        Self::intersects(slf, other)
    }

    fn satisfies(slf: &Bound<'_, Self>, other: &Bound<'_, PyAny>) -> PyResult<bool> {
        if !is_version_obj(other) {
            return Err(not_supported("satisfies", other));
        }
        run_op(slf.py(), &[slf.as_any(), other], |u| u[0].satisfies(&u[1]))
    }

    fn union(&self, _other: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        Err(PyNotImplementedError::new_err(()))
    }

    fn intersection(slf: &Bound<'_, Self>, other: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        if !is_version_obj(other) {
            return Err(not_supported("intersection", other));
        }
        let py = slf.py();
        let u = run_op(py, &[slf.as_any(), other], |u| u[0].intersection(&u[1]))?;
        union_to_py(py, u)
    }

    fn __iter__(slf: &Bound<'_, Self>) -> PyResult<Py<PyAny>> {
        let py = slf.py();
        let rv = git_ref_version(slf)?;
        sv_components_list(py, &rv)?
            .as_any()
            .try_iter()?
            .into_py_any(py)
    }

    fn __len__(slf: &Bound<'_, Self>) -> PyResult<usize> {
        Ok(git_ref_version(slf)?.len())
    }

    fn __getitem__(slf: &Bound<'_, Self>, idx: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let rv = git_ref_version(slf)?;
        sv_getitem(slf.py(), &rv, idx, "StandardVersion")
    }

    fn isdevelop(slf: &Bound<'_, Self>) -> PyResult<bool> {
        Ok(git_ref_version(slf)?.isdevelop())
    }

    fn is_prerelease(slf: &Bound<'_, Self>) -> PyResult<bool> {
        Ok(git_ref_version(slf)?.is_prerelease())
    }

    #[getter]
    fn dotted(slf: &Bound<'_, Self>) -> PyResult<Py<PyAny>> {
        let rv = git_ref_version(slf)?;
        new_sv(slf.py(), rv.dotted())
    }

    #[getter]
    fn underscored(slf: &Bound<'_, Self>) -> PyResult<Py<PyAny>> {
        let rv = git_ref_version(slf)?;
        new_sv(slf.py(), rv.underscored())
    }

    #[getter]
    fn dashed(slf: &Bound<'_, Self>) -> PyResult<Py<PyAny>> {
        let rv = git_ref_version(slf)?;
        new_sv(slf.py(), rv.dashed())
    }

    #[getter]
    fn joined(slf: &Bound<'_, Self>) -> PyResult<Py<PyAny>> {
        let rv = git_ref_version(slf)?;
        new_sv(slf.py(), rv.joined())
    }

    fn up_to(slf: &Bound<'_, Self>, index: isize) -> PyResult<Py<PyAny>> {
        let rv = git_ref_version(slf)?;
        new_sv(slf.py(), rv.up_to(index))
    }

    fn __reduce__(slf: &Bound<'_, Self>) -> PyResult<(Py<PyAny>, (String,))> {
        // Rebuild from the full string form; the attached lookup is dropped and re-attached
        // by spec deserialization.
        let py = slf.py();
        Ok((slf.get_type().into_py_any(py)?, (Self::__str__(slf)?,)))
    }
}

// ---------------------------------------------------------------------------
// ClosedOpenRange
// ---------------------------------------------------------------------------

/// `__reduce__` payload of a range: the class and its `(lo, hi)` constructor arguments.
type ReduceBounds = (Py<PyAny>, (Py<PyAny>, Py<PyAny>));

#[pymethods]
impl ClosedOpenRange {
    /// Like Python's constructor: `hi` is *exclusive*.
    #[new]
    fn new(
        py: Python<'_>,
        lo: &Bound<'_, PyAny>,
        hi: &Bound<'_, PyAny>,
    ) -> PyResult<PyClassInitializer<Self>> {
        let lo = lo
            .downcast::<StandardVersion>()
            .map_err(|_| PyTypeError::new_err("ClosedOpenRange bounds must be StandardVersion"))?
            .borrow()
            .inner
            .clone();
        let hi = hi
            .downcast::<StandardVersion>()
            .map_err(|_| PyTypeError::new_err("ClosedOpenRange bounds must be StandardVersion"))?
            .borrow()
            .inner
            .clone();
        let inner = cv::ClosedOpenRange::new(lo, hi).map_err(|e| verr(py, e))?;
        // Python's __init__ leaves the string and hash caches unset.
        Ok(
            PyClassInitializer::from(VersionType).add_subclass(ClosedOpenRange {
                inner,
                string: None,
                hash: None,
            }),
        )
    }

    /// `hi` is *inclusive*, like the Spack range syntax `lo:hi`.
    #[staticmethod]
    fn from_version_range(
        py: Python<'_>,
        lo: &Bound<'_, PyAny>,
        hi: &Bound<'_, PyAny>,
    ) -> PyResult<Py<PyAny>> {
        let lo = lo
            .downcast::<StandardVersion>()
            .map_err(|_| PyTypeError::new_err("ClosedOpenRange bounds must be StandardVersion"))?
            .borrow()
            .inner
            .clone();
        let hi = hi
            .downcast::<StandardVersion>()
            .map_err(|_| PyTypeError::new_err("ClosedOpenRange bounds must be StandardVersion"))?
            .borrow()
            .inner
            .clone();
        let inner = cv::ClosedOpenRange::from_version_range(lo, hi).map_err(|e| verr(py, e))?;
        new_range(py, inner)
    }

    #[getter]
    fn lo(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        new_sv(py, self.inner.lo().clone())
    }

    #[getter]
    fn hi(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        new_sv(py, self.inner.hi().clone())
    }

    #[getter(_string)]
    fn string_cache(&self) -> Option<String> {
        self.string.clone()
    }

    #[setter(_string)]
    fn set_string_cache(&mut self, value: Option<String>) {
        self.string = value;
    }

    #[getter(_hash)]
    fn hash_cache(&self) -> Option<isize> {
        self.hash
    }

    #[setter(_hash)]
    fn set_hash_cache(&mut self, value: Option<isize>) {
        self.hash = value;
    }

    fn __str__(&mut self) -> String {
        if self.string.as_deref().is_none_or(str::is_empty) {
            self.string = Some(str_range(
                self.inner.lo(),
                &cv::prev_version(self.inner.hi()),
            ));
        }
        self.string.clone().expect("just filled")
    }

    fn __repr__(&mut self) -> String {
        self.__str__()
    }

    fn __hash__(&mut self) -> isize {
        if let Some(h) = self.hash {
            return h;
        }
        let h = range_hash(&self.inner);
        self.hash = Some(h);
        h
    }

    fn __richcmp__(
        slf: &Bound<'_, Self>,
        other: &Bound<'_, PyAny>,
        op: CompareOp,
    ) -> PyResult<Py<PyAny>> {
        let py = other.py();
        if let Ok(b) = other.downcast::<ClosedOpenRange>() {
            let a = slf.borrow().inner.clone();
            let b = b.borrow().inner.clone();
            // Python compares the (lo, hi) tuples lexicographically.
            let cmp = a.lo().cmp(b.lo()).then_with(|| a.hi().cmp(b.hi()));
            let r = match op {
                CompareOp::Eq => cmp == Ordering::Equal,
                CompareOp::Ne => cmp != Ordering::Equal,
                CompareOp::Lt => cmp == Ordering::Less,
                CompareOp::Le => cmp != Ordering::Greater,
                CompareOp::Ge => cmp != Ordering::Less,
                CompareOp::Gt => cmp == Ordering::Greater,
            };
            return r.into_py_any(py);
        }
        if let Ok(b) = other.downcast::<StandardVersion>() {
            // Delegated to the StandardVersion comparisons in Python; versions and ranges
            // are never equal.
            let lo = slf.borrow().inner.lo().clone();
            let v = &b.borrow().inner;
            let r = match op {
                CompareOp::Eq => false,
                CompareOp::Ne => true,
                CompareOp::Lt | CompareOp::Le => *v > lo,
                CompareOp::Ge | CompareOp::Gt => *v <= lo,
            };
            return r.into_py_any(py);
        }
        Ok(py.NotImplemented())
    }

    fn __contains__(slf: &Bound<'_, Self>, lhs: &Bound<'_, PyAny>) -> PyResult<bool> {
        if !is_version_obj(lhs) {
            return Err(PyTypeError::new_err(format!(
                "'in' not supported between instances of {} and {}",
                type_repr(slf.as_any()),
                type_repr(lhs)
            )));
        }
        run_op(slf.py(), &[lhs, slf.as_any()], |u| u[0].satisfies(&u[1]))
    }

    fn intersects(slf: &Bound<'_, Self>, other: &Bound<'_, PyAny>) -> PyResult<bool> {
        if !is_version_obj(other) {
            return Err(PyTypeError::new_err(format!(
                "'intersects' not supported for instances of {}",
                type_repr(other)
            )));
        }
        run_op(slf.py(), &[slf.as_any(), other], |u| u[0].intersects(&u[1]))
    }

    fn overlaps(slf: &Bound<'_, Self>, other: &Bound<'_, PyAny>) -> PyResult<bool> {
        Self::intersects(slf, other)
    }

    fn satisfies(slf: &Bound<'_, Self>, other: &Bound<'_, PyAny>) -> PyResult<bool> {
        if !is_version_obj(other) {
            return Err(not_supported("satisfies", other));
        }
        run_op(slf.py(), &[slf.as_any(), other], |u| u[0].satisfies(&u[1]))
    }

    fn union(slf: &Bound<'_, Self>, other: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        if !is_version_obj(other) {
            return Err(not_supported("union", other));
        }
        let py = slf.py();
        let u = run_op(py, &[slf.as_any(), other], |u| u[0].union(&u[1]))?;
        union_to_py(py, u)
    }

    fn intersection(slf: &Bound<'_, Self>, other: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        if !is_version_obj(other) {
            return Err(not_supported("intersection", other));
        }
        let py = slf.py();
        let u = run_op(py, &[slf.as_any(), other], |u| u[0].intersection(&u[1]))?;
        union_to_py(py, u)
    }

    fn __reduce__(slf: &Bound<'_, Self>) -> PyResult<ReduceBounds> {
        let py = slf.py();
        let (lo, hi) = {
            let b = slf.borrow();
            (b.inner.lo().clone(), b.inner.hi().clone())
        };
        Ok((
            slf.get_type().into_py_any(py)?,
            (new_sv(py, lo)?, new_sv(py, hi)?),
        ))
    }
}

// ---------------------------------------------------------------------------
// VersionList
// ---------------------------------------------------------------------------

/// Python `bisect_left(list, x)` with the cross-type `<` of the version types.
fn bisect_objs(py: Python<'_>, items: &[Py<PyAny>], x: &Bound<'_, PyAny>) -> PyResult<usize> {
    let (mut lo, mut hi) = (0, items.len());
    while lo < hi {
        let mid = (lo + hi) / 2;
        let mid_obj = items[mid].bind(py).clone();
        let less = run_op(py, &[&mid_obj, x], |u| {
            Ok(u[0].compare(&u[1])? == Ordering::Less)
        })?;
        if less {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    Ok(lo)
}

/// Transcription of `VersionList.add` over the shared item objects.
fn add_obj(py: Python<'_>, items: &mut Vec<Py<PyAny>>, item: &Bound<'_, PyAny>) -> PyResult<()> {
    if let Ok(l) = item.downcast::<VersionList>() {
        let sub: Vec<Py<PyAny>> = l.borrow().items.clone();
        for v in &sub {
            add_obj(py, items, v.bind(py))?;
        }
        return Ok(());
    }
    if let Ok(r) = item.downcast::<ClosedOpenRange>() {
        let mut merged = r.borrow().inner.clone();
        let mut changed = false;
        let mut i = bisect_objs(py, items, item)?;

        // The incoming range can merge with multiple neighbors on both sides.
        while i > 0 {
            let left = items[i - 1].bind(py).clone();
            let m = merged.clone();
            let union = run_op(py, &[&left], |u| {
                match VersionUnion::Range(m.clone()).union(&u[0])? {
                    VersionUnion::Range(r) => Ok(Some(r)),
                    _ => Ok(None), // disjoint
                }
            })?;
            match union {
                None => break,
                Some(u) => {
                    merged = u;
                    changed = true;
                    items.remove(i - 1);
                    i -= 1;
                }
            }
        }
        while i < items.len() {
            let right = items[i].bind(py).clone();
            let m = merged.clone();
            let union = run_op(py, &[&right], |u| {
                match VersionUnion::Range(m.clone()).union(&u[0])? {
                    VersionUnion::Range(r) => Ok(Some(r)),
                    _ => Ok(None),
                }
            })?;
            match union {
                None => break,
                Some(u) => {
                    merged = u;
                    changed = true;
                    items.remove(i);
                }
            }
        }
        let obj = if changed {
            new_range(py, merged)?
        } else {
            item.clone().unbind()
        };
        items.insert(i, obj);
        return Ok(());
    }
    if item.is_instance_of::<ConcreteVersion>() {
        let i = bisect_objs(py, items, item)?;
        // Only insert when prev and next do not intersect the version.
        if i > 0 {
            let left = items[i - 1].bind(py).clone();
            if run_op(py, &[item, &left], |u| u[0].intersects(&u[1]))? {
                return Ok(());
            }
        }
        if i < items.len() {
            let right = items[i].bind(py).clone();
            if run_op(py, &[item, &right], |u| u[0].intersects(&u[1]))? {
                return Ok(());
            }
        }
        items.insert(i, item.clone().unbind());
        return Ok(());
    }
    Err(PyTypeError::new_err(format!(
        "Can't add {} to VersionList",
        type_repr(item)
    )))
}

/// The `VersionList(vlist)` constructor logic, shared with `ver()`.
fn version_list_items(
    py: Python<'_>,
    vlist: Option<&Bound<'_, PyAny>>,
) -> PyResult<Vec<Py<PyAny>>> {
    let Some(vlist) = vlist else {
        return Ok(Vec::new());
    };
    if vlist.is_none() {
        return Ok(Vec::new());
    }
    if let Ok(s) = vlist.downcast::<PyString>() {
        return match cv::from_string(s.to_str()?).map_err(|e| verr(py, e))? {
            VersionUnion::List(l) => core_list_to_items(py, &l),
            single => Ok(vec![union_to_py(py, single)?]),
        };
    }
    if let Ok(l) = vlist.downcast::<VersionList>() {
        return Ok(l.borrow().items.clone());
    }
    if vlist.is_instance_of::<ConcreteVersion>() || vlist.is_instance_of::<ClosedOpenRange>() {
        return Ok(vec![vlist.clone().unbind()]);
    }
    if let Ok(iter) = vlist.try_iter() {
        let mut items = Vec::new();
        for elem in iter {
            let v = ver_any(py, &elem?)?;
            add_obj(py, &mut items, v.bind(py))?;
        }
        return Ok(items);
    }
    Err(PyTypeError::new_err(format!(
        "Cannot construct VersionList from {}",
        type_repr(vlist)
    )))
}

#[pymethods]
impl VersionList {
    #[new]
    #[pyo3(signature = (vlist=None))]
    fn new(py: Python<'_>, vlist: Option<&Bound<'_, PyAny>>) -> PyResult<PyClassInitializer<Self>> {
        Ok(list_init(version_list_items(py, vlist)?))
    }

    fn add(slf: &Bound<'_, Self>, item: &Bound<'_, PyAny>) -> PyResult<()> {
        let py = slf.py();
        let mut items = slf.borrow().items.clone();
        add_obj(py, &mut items, item)?;
        slf.borrow_mut().items = items;
        Ok(())
    }

    fn update(slf: &Bound<'_, Self>, other: &Bound<'_, PyAny>) -> PyResult<()> {
        Self::add(slf, other)
    }

    #[getter]
    fn versions<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyList>> {
        PyList::new(py, &self.items)
    }

    #[setter]
    fn set_versions(&mut self, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let mut items = Vec::new();
        for elem in value.try_iter()? {
            items.push(elem?.unbind());
        }
        self.items = items;
        Ok(())
    }

    #[getter]
    fn concrete(&self, py: Python<'_>) -> Option<Py<PyAny>> {
        match self.items.as_slice() {
            [only] if only.bind(py).is_instance_of::<ConcreteVersion>() => Some(only.clone_ref(py)),
            _ => None,
        }
    }

    #[getter]
    fn concrete_range_as_version(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        let [only] = self.items.as_slice() else {
            return Ok(None);
        };
        let only = only.bind(py);
        if only.is_instance_of::<ConcreteVersion>() {
            return Ok(Some(only.clone().unbind()));
        }
        if let Ok(r) = only.downcast::<ClosedOpenRange>() {
            let r = r.borrow().inner.clone();
            if cv::next_version(r.lo()) == *r.hi() {
                return Ok(Some(new_sv(py, r.lo().clone())?));
            }
        }
        Ok(None)
    }

    fn copy(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        new_list(py, self.items.clone())
    }

    fn lowest(&self, py: Python<'_>) -> Option<Py<PyAny>> {
        self.items
            .iter()
            .find(|v| v.bind(py).is_instance_of::<StandardVersion>())
            .map(|v| v.clone_ref(py))
    }

    fn highest(&self, py: Python<'_>) -> Option<Py<PyAny>> {
        self.items
            .iter()
            .rev()
            .find(|v| v.bind(py).is_instance_of::<StandardVersion>())
            .map(|v| v.clone_ref(py))
    }

    fn highest_numeric(&self, py: Python<'_>) -> Option<Py<PyAny>> {
        self.items
            .iter()
            .rev()
            .find(|v| match v.bind(py).downcast::<StandardVersion>() {
                Ok(sv) => !sv.borrow().inner.isdevelop(),
                Err(_) => false,
            })
            .map(|v| v.clone_ref(py))
    }

    fn preferred(&self, py: Python<'_>) -> Option<Py<PyAny>> {
        self.highest_numeric(py).or_else(|| self.highest(py))
    }

    fn satisfies(slf: &Bound<'_, Self>, other: &Bound<'_, PyAny>) -> PyResult<bool> {
        if !is_version_obj(other) {
            return Err(not_supported("satisfies", other));
        }
        run_op(slf.py(), &[slf.as_any(), other], |u| u[0].satisfies(&u[1]))
    }

    fn intersects(slf: &Bound<'_, Self>, other: &Bound<'_, PyAny>) -> PyResult<bool> {
        if !is_version_obj(other) {
            return Err(not_supported("intersects", other));
        }
        run_op(slf.py(), &[slf.as_any(), other], |u| u[0].intersects(&u[1]))
    }

    fn overlaps(slf: &Bound<'_, Self>, other: &Bound<'_, PyAny>) -> PyResult<bool> {
        Self::intersects(slf, other)
    }

    fn union(slf: &Bound<'_, Self>, other: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let py = slf.py();
        let mut items = slf.borrow().items.clone();
        add_obj(py, &mut items, other)?;
        new_list(py, items)
    }

    fn intersection(slf: &Bound<'_, Self>, other: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        if !is_version_obj(other) {
            return Err(not_supported("intersection", other));
        }
        let py = slf.py();
        let u = run_op(py, &[slf.as_any(), other], |u| u[0].intersection(&u[1]))?;
        union_to_py(py, u)
    }

    /// In-place intersection; returns whether the list changed.
    fn intersect(slf: &Bound<'_, Self>, other: &Bound<'_, PyAny>) -> PyResult<bool> {
        if !is_version_obj(other) {
            return Err(not_supported("intersection", other));
        }
        let py = slf.py();
        let (isection, changed) = run_op(py, &[slf.as_any(), other], |u| {
            let result = u[0].intersection(&u[1])?;
            let VersionUnion::List(result) = result else {
                unreachable!("list intersection yields a list")
            };
            let changed = !matches!(&u[0], VersionUnion::List(a) if *a == result);
            Ok((result, changed))
        })?;
        let items = core_list_to_items(py, &isection)?;
        slf.borrow_mut().items = items;
        Ok(changed)
    }

    fn to_dict<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, PyDict>> {
        let py = slf.py();
        let d = PyDict::new(py);
        let items: Vec<Py<PyAny>> = slf.borrow().items.clone();
        let is_concrete = slf.borrow().concrete(py).is_some();
        if is_concrete {
            d.set_item("version", items[0].bind(py).str()?)?;
        } else {
            let strings: Vec<String> = items
                .iter()
                .map(|v| v.bind(py).str().map(|s| s.to_string()))
                .collect::<PyResult<_>>()?;
            d.set_item("versions", strings)?;
        }
        Ok(d)
    }

    #[staticmethod]
    fn from_dict(py: Python<'_>, dictionary: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        if dictionary.contains("versions")? {
            let versions = dictionary.get_item("versions")?;
            let items = version_list_items(py, Some(&versions))?;
            new_list(py, items)
        } else if dictionary.contains("version")? {
            let version = dictionary.get_item("version")?;
            let v = version_factory(py, &version)?;
            new_list(py, vec![v])
        } else {
            Err(PyValueError::new_err(
                "Dict must have 'version' or 'versions' in it.",
            ))
        }
    }

    /// A VersionList that matches any version.
    #[classmethod]
    fn any(_cls: &Bound<'_, PyType>, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let items = core_list_to_items(py, &cv::any_version())?;
        new_list(py, items)
    }

    fn __contains__(slf: &Bound<'_, Self>, other: &Bound<'_, PyAny>) -> PyResult<bool> {
        // Python's isinstance checks cover only ranges, standard versions, and lists;
        // anything else (including git versions) is not contained.
        if other.is_instance_of::<GitVersion>() || !is_version_obj(other) {
            return Ok(false);
        }
        run_op(slf.py(), &[slf.as_any(), other], |u| u[0].contains(&u[1]))
    }

    fn __getitem__(slf: &Bound<'_, Self>, idx: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let py = slf.py();
        let items = slf.borrow().versions(py)?;
        Ok(items.as_any().get_item(idx)?.unbind())
    }

    fn __iter__(slf: &Bound<'_, Self>) -> PyResult<Py<PyAny>> {
        let py = slf.py();
        let items = slf.borrow().versions(py)?;
        items.as_any().try_iter()?.into_py_any(py)
    }

    fn __reversed__(slf: &Bound<'_, Self>) -> PyResult<Py<PyAny>> {
        let py = slf.py();
        let items = slf.borrow().versions(py)?;
        Ok(items.as_any().call_method0("__reversed__")?.unbind())
    }

    fn __len__(&self) -> usize {
        self.items.len()
    }

    fn __bool__(&self) -> bool {
        !self.items.is_empty()
    }

    fn __hash__(slf: &Bound<'_, Self>) -> PyResult<isize> {
        // Python hashes the tuple of elements.
        let py = slf.py();
        let items: Vec<Py<PyAny>> = slf.borrow().items.clone();
        let mut h = DefaultHasher::new();
        items.len().hash(&mut h);
        for item in &items {
            item.bind(py).hash()?.hash(&mut h);
        }
        Ok(normalize_hash(h.finish()))
    }

    fn __str__(slf: &Bound<'_, Self>) -> PyResult<String> {
        let py = slf.py();
        let items: Vec<Py<PyAny>> = slf.borrow().items.clone();
        let mut parts = Vec::with_capacity(items.len());
        for item in &items {
            let item = item.bind(py);
            let s = item.str()?.to_string();
            // Concrete standard versions are printed with an `=` prefix.
            if item.is_exact_instance_of::<StandardVersion>() {
                parts.push(format!("={s}"));
            } else {
                parts.push(s);
            }
        }
        Ok(parts.join(","))
    }

    fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
        let py = slf.py();
        let items = slf.borrow().versions(py)?;
        Ok(items.as_any().repr()?.to_string())
    }

    fn __richcmp__(
        slf: &Bound<'_, Self>,
        other: &Bound<'_, PyAny>,
        op: CompareOp,
    ) -> PyResult<Py<PyAny>> {
        let py = other.py();
        if other.downcast::<VersionList>().is_ok() {
            if matches!(op, CompareOp::Eq | CompareOp::Ne) {
                let eq = run_op(py, &[slf.as_any(), other], |u| match (&u[0], &u[1]) {
                    (VersionUnion::List(a), VersionUnion::List(b)) => Ok(a == b),
                    _ => unreachable!("both operands are lists"),
                })?;
                return (if matches!(op, CompareOp::Eq) { eq } else { !eq }).into_py_any(py);
            }
            let cmp = run_op(py, &[slf.as_any(), other], |u| u[0].compare(&u[1]))?;
            let r = match op {
                CompareOp::Lt => cmp == Ordering::Less,
                CompareOp::Le => cmp != Ordering::Greater,
                CompareOp::Ge => cmp != Ordering::Less,
                CompareOp::Gt => cmp == Ordering::Greater,
                CompareOp::Eq | CompareOp::Ne => unreachable!("handled above"),
            };
            return r.into_py_any(py);
        }
        match op {
            // Quirk faithfully kept: Python's __ne__ also returns False for non-lists.
            CompareOp::Eq | CompareOp::Ne => false.into_py_any(py),
            _ => Ok(py.NotImplemented()),
        }
    }

    fn __reduce__(slf: &Bound<'_, Self>) -> PyResult<(Py<PyAny>, (Vec<String>,))> {
        let py = slf.py();
        let items: Vec<Py<PyAny>> = slf.borrow().items.clone();
        let mut strings = Vec::with_capacity(items.len());
        for item in &items {
            let item = item.bind(py);
            let s = item.str()?.to_string();
            if item.is_exact_instance_of::<StandardVersion>() {
                strings.push(format!("={s}"));
            } else {
                strings.push(s);
            }
        }
        Ok((slf.get_type().into_py_any(py)?, (strings,)))
    }
}

// ---------------------------------------------------------------------------
// Module-level factory functions
// ---------------------------------------------------------------------------

/// Python `Version(x)`: a GitVersion when the string looks like a git version, otherwise a
/// StandardVersion.
#[pyfunction(name = "Version")]
pub fn version_factory(py: Python<'_>, x: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let s: String = if x.is_instance_of::<PyString>() || x.is_instance_of::<PyInt>() {
        x.str()?.extract()?
    } else {
        return Err(PyTypeError::new_err(format!(
            "Cannot construct a version from {}",
            type_repr(x)
        )));
    };
    if cv::is_git_version(&s) {
        new_git(
            py,
            cv::GitVersion::from_string(&s).map_err(|e| verr(py, e))?,
            None,
        )
    } else {
        StandardVersion::from_string(py, &s)
    }
}

/// Python `VersionRange(lo, hi)`: inclusive bounds, from strings or StandardVersions.
#[pyfunction(name = "VersionRange")]
pub fn version_range_factory(
    py: Python<'_>,
    lo: &Bound<'_, PyAny>,
    hi: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let parse = |b: &Bound<'_, PyAny>| -> PyResult<cv::StandardVersion> {
        if let Ok(sv) = b.downcast::<StandardVersion>() {
            return Ok(sv.borrow().inner.clone());
        }
        if let Ok(s) = b.downcast::<PyString>() {
            return cv::StandardVersion::from_string(s.to_str()?).map_err(|e| verr(py, e));
        }
        Err(PyTypeError::new_err(format!(
            "VersionRange bounds must be str or StandardVersion, not {}",
            type_repr(b)
        )))
    };
    let inner =
        cv::ClosedOpenRange::from_version_range(parse(lo)?, parse(hi)?).map_err(|e| verr(py, e))?;
    new_range(py, inner)
}

/// Python `from_string(s)`: parse a version, range, or list from its string form.
#[pyfunction]
pub fn from_string(py: Python<'_>, string: &str) -> PyResult<Py<PyAny>> {
    union_to_py(py, cv::from_string(string).map_err(|e| verr(py, e))?)
}

fn ver_any(py: Python<'_>, obj: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    if is_version_obj(obj) {
        return Ok(obj.clone().unbind());
    }
    if let Ok(s) = obj.downcast::<PyString>() {
        return from_string(py, s.to_str()?);
    }
    if obj.is_instance_of::<PyList>() || obj.is_instance_of::<PyTuple>() {
        let items = version_list_items(py, Some(obj))?;
        return new_list(py, items);
    }
    if obj.is_instance_of::<PyInt>() || obj.is_instance_of::<PyFloat>() {
        return from_string(py, obj.str()?.to_str()?);
    }
    Err(PyTypeError::new_err(format!(
        "ver() can't convert {} to version!",
        type_repr(obj)
    )))
}

/// Python `ver(x)`: convert a string, number, iterable, or version object to a version type.
#[pyfunction]
pub fn ver(py: Python<'_>, obj: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    ver_any(py, obj)
}

/// Python `_next_version(v)`: the smallest version greater than v.
#[pyfunction(name = "_next_version")]
pub fn next_version_py(py: Python<'_>, v: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let sv = v
        .downcast::<StandardVersion>()
        .map_err(|_| PyTypeError::new_err("_next_version() expects a StandardVersion"))?;
    let inner = sv.borrow().inner.clone();
    new_sv(py, cv::next_version(&inner))
}

/// Python `_prev_version(v)`; only meaningful as `_prev_version(_next_version(v))`.
#[pyfunction(name = "_prev_version")]
pub fn prev_version_py(py: Python<'_>, v: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let sv = v
        .downcast::<StandardVersion>()
        .map_err(|_| PyTypeError::new_err("_prev_version() expects a StandardVersion"))?;
    let inner = sv.borrow().inner.clone();
    new_sv(py, cv::prev_version(&inner))
}
