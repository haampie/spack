// SPDX-License-Identifier: (Apache-2.0 OR MIT)

//! PyO3 binding for the target-range algebra in `spack-spec-core`, mirroring
//! `spack.spec.ArchSpec`.
//!
//! Platform and operating system are plain optional strings; the target is stored as the
//! canonical expression string next to the `archspec.cpu.Microarchitecture` object the
//! `target` getter returns, both assigned together by the setter like the reference stores
//! the object itself. Everything host-dependent (platform resolution of the reserved
//! `default_os`/`frontend`/... names, `_make_microarchitecture`, the live `TARGETS` table)
//! goes through the registered `spack.spec._ArchOracle`, resolved per call so that
//! `use_platform` and monkeypatched modules are observed live. The [`TargetGraph`] built
//! from the live table is cached by the table's identity and length.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use pyo3::basic::CompareOp;
use pyo3::exceptions::{PyRuntimeError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyString, PyTuple};
use pyo3::IntoPyObjectExt;

use spack_spec_core::targets::{TargetExpr, TargetGraph};

use crate::registry;

// ---------------------------------------------------------------------------
// Oracle access and the target-graph cache
// ---------------------------------------------------------------------------

fn oracle(py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
    match registry::ARCH_ORACLE.get(py) {
        Some(o) => Ok(o.bind(py).clone()),
        None => Err(PyRuntimeError::new_err(
            "spack_spec: the arch oracle is not registered; import spack.spec first",
        )),
    }
}

/// The graph built from the live `TARGETS` table, keyed by the table's identity and length
/// so a monkeypatched or lazily materialized table triggers a rebuild.
static GRAPH_CACHE: Mutex<Option<(usize, usize, Arc<TargetGraph>)>> = Mutex::new(None);

fn target_graph(py: Python<'_>) -> PyResult<Arc<TargetGraph>> {
    let table = oracle(py)?.call_method0("targets_dict")?;
    let key = table.as_ptr() as usize;
    let len = table.len()?;
    if let Some((k, l, graph)) = GRAPH_CACHE.lock().unwrap().as_ref() {
        if *k == key && *l == len {
            return Ok(graph.clone());
        }
    }
    let mut records: Vec<(String, Vec<String>, String)> = Vec::with_capacity(len);
    for item in table.call_method0("items")?.try_iter()? {
        let (name, micro): (String, Bound<'_, PyAny>) = item?.extract()?;
        let mut parents = Vec::new();
        for parent in micro.getattr("parents")?.try_iter()? {
            parents.push(parent?.getattr("name")?.extract()?);
        }
        let vendor: String = micro.getattr("vendor")?.extract()?;
        records.push((name, parents, vendor));
    }
    let graph = Arc::new(
        TargetGraph::from_records(records).map_err(|e| PyValueError::new_err(e.to_string()))?,
    );
    *GRAPH_CACHE.lock().unwrap() = Some((key, len, graph.clone()));
    Ok(graph)
}

/// Canonicalization results of [`TargetExpr::parse`] per graph: stored targets are already
/// canonical, so parsing them again on every operation only reproduces the same expression.
static EXPR_CACHE: Mutex<Option<(usize, HashMap<String, TargetExpr>)>> = Mutex::new(None);

fn parse_expr(graph: &Arc<TargetGraph>, s: &str) -> TargetExpr {
    if !s.contains(':') && !s.contains(',') {
        // single names are stored verbatim; skip the cache
        return TargetExpr::parse(graph, s);
    }
    let key = Arc::as_ptr(graph) as usize;
    let mut guard = EXPR_CACHE.lock().unwrap();
    match guard.as_mut() {
        Some((k, map)) if *k == key => {
            if let Some(expr) = map.get(s) {
                return expr.clone();
            }
            let expr = TargetExpr::parse(graph, s);
            map.insert(s.to_string(), expr.clone());
            expr
        }
        _ => {
            let expr = TargetExpr::parse(graph, s);
            *guard = Some((key, HashMap::from([(s.to_string(), expr.clone())])));
            expr
        }
    }
}

/// `UnsatisfiableArchitectureSpecError(provided, required)`.
fn unsat_err(py: Python<'_>, provided: &Bound<'_, PyAny>, required: &Bound<'_, PyAny>) -> PyErr {
    let fallback = PyValueError::new_err(format!(
        "'{}' does not satisfy '{}'",
        provided.str().map(|s| s.to_string()).unwrap_or_default(),
        required.str().map(|s| s.to_string()).unwrap_or_default(),
    ));
    match registry::UNSAT_ARCH_ERROR.get(py) {
        Some(cls) => match cls.bind(py).call1((provided, required)) {
            Ok(exc) => PyErr::from_value(exc),
            Err(e) => e,
        },
        None => fallback,
    }
}

// ---------------------------------------------------------------------------
// The platform and operating-system dimensions
// ---------------------------------------------------------------------------

/// `ArchSpec._attr_satisfies`: whether a platform or operating system on the lhs satisfies
/// the one on the rhs.
fn attr_satisfies(lhs: Option<&str>, rhs: Option<&str>) -> bool {
    match rhs {
        None | Some("") => true,
        Some("*") => lhs.is_some(),
        Some(r) => lhs == Some(r),
    }
}

/// `ArchSpec._attr_intersects`: whether a platform or operating system on either side
/// intersect.
fn attr_intersects(lhs: Option<&str>, rhs: Option<&str>) -> bool {
    match (lhs, rhs) {
        (None | Some(""), _) | (_, None | Some("")) => true,
        (Some(l), Some(r)) => l == "*" || r == "*" || l == r,
    }
}

/// One platform/os step of `ArchSpec._merge`: a star constrains the value to be set, so it
/// is kept when there is none yet and is replaced by a named value from the other side.
fn merge_attr(svalue: &mut Option<String>, ovalue: &Option<String>) -> bool {
    if ovalue.is_some()
        && (svalue.is_none() || (svalue.as_deref() == Some("*") && ovalue.as_deref() != Some("*")))
    {
        *svalue = ovalue.clone();
        return true;
    }
    false
}

/// Python truthiness of an optional string attribute.
fn truthy(value: &Option<String>) -> bool {
    value.as_deref().is_some_and(|s| !s.is_empty())
}

// ---------------------------------------------------------------------------
// ArchSpec
// ---------------------------------------------------------------------------

/// Aggregate of the target platform, the operating system and the target microarchitecture
/// (`spack.spec.ArchSpec`).
#[pyclass(name = "ArchSpec", module = "spack_spec")]
pub struct ArchSpec {
    platform: Option<String>,
    os: Option<String>,
    /// `str()` of the stored microarchitecture; `Some` iff `target_obj` is `Some`.
    target: Option<String>,
    /// The `archspec.cpu.Microarchitecture` the `target` getter returns.
    target_obj: Option<Py<PyAny>>,
}

impl ArchSpec {
    fn empty() -> Self {
        ArchSpec {
            platform: None,
            os: None,
            target: None,
            target_obj: None,
        }
    }

    fn clone_fields(&self, py: Python<'_>) -> Self {
        ArchSpec {
            platform: self.platform.clone(),
            os: self.os.clone(),
            target: self.target.clone(),
            target_obj: self.target_obj.as_ref().map(|t| t.clone_ref(py)),
        }
    }

    /// `str(value)` or `None`, the conversion of the platform and os setters.
    fn str_or_none(value: Option<&Bound<'_, PyAny>>) -> PyResult<Option<String>> {
        match value {
            Some(v) if !v.is_none() => Ok(Some(v.str()?.to_cow()?.into_owned())),
            _ => Ok(None),
        }
    }

    /// `_string_or_none` in the reference constructor: also drops falsy values and `"None"`.
    fn string_or_none(value: &Bound<'_, PyAny>) -> PyResult<Option<String>> {
        if value.is_truthy()? && !value.eq("None")? {
            Ok(Some(value.str()?.to_cow()?.into_owned()))
        } else {
            Ok(None)
        }
    }

    /// The reference's os setter after string conversion: reserved values resolve through
    /// the host platform, which is also assigned when missing; the resolution goes through
    /// `str()` on the Python side and can therefore store the string `"None"`.
    fn assign_os(&mut self, py: Python<'_>, mut value: Option<String>) -> PyResult<()> {
        if let Some(v) = value.as_deref() {
            let oracle = oracle(py)?;
            let reserved: Vec<String> = oracle.getattr("reserved_oss")?.extract()?;
            if reserved.iter().any(|r| r == v) {
                let curr: String = oracle.call_method0("host_name")?.extract()?;
                if !truthy(&self.platform) {
                    self.platform = Some(curr.clone());
                }
                let platform = self.platform.clone().unwrap();
                if platform != curr {
                    return Err(PyValueError::new_err(format!(
                        "Can't set arch spec OS to reserved value '{v}' when the arch \
                         platform ({platform}) isn't the current platform ({curr})"
                    )));
                }
                value = Some(
                    oracle
                        .call_method1("platform_os", (platform, v))?
                        .extract()?,
                );
            }
        }
        self.os = value;
        Ok(())
    }

    /// The reference's target setter: strings become microarchitectures via
    /// `_make_microarchitecture`, and reserved names resolve through the host platform.
    fn assign_target(&mut self, py: Python<'_>, value: Option<&Bound<'_, PyAny>>) -> PyResult<()> {
        let oracle = oracle(py)?;
        let raw = match value {
            Some(v) => v.clone(),
            None => py.None().into_bound(py),
        };
        let resolved = oracle.call_method1("target_or_none", (raw,))?;
        let mut resolved = (!resolved.is_none()).then_some(resolved);
        if let Some(t) = &resolved {
            let name = t.str()?.to_cow()?.into_owned();
            let reserved: Vec<String> = oracle.getattr("reserved_targets")?.extract()?;
            if reserved.contains(&name) {
                let curr: String = oracle.call_method0("host_name")?.extract()?;
                if !truthy(&self.platform) {
                    self.platform = Some(curr.clone());
                }
                let platform = self.platform.clone().unwrap();
                if platform != curr {
                    return Err(PyValueError::new_err(format!(
                        "Can't set arch spec target to reserved value '{name}' when the \
                         arch platform ({platform}) isn't the current platform ({curr})"
                    )));
                }
                let r = oracle.call_method1("platform_target", (platform, t))?;
                resolved = (!r.is_none()).then_some(r);
            }
        }
        match resolved {
            Some(t) => {
                self.target = Some(t.str()?.to_cow()?.into_owned());
                self.target_obj = Some(t.unbind());
            }
            None => {
                self.target = None;
                self.target_obj = None;
            }
        }
        Ok(())
    }

    /// Copy the target dimension from another instance, object identity included.
    fn adopt_target(&mut self, py: Python<'_>, other: &ArchSpec) {
        self.target = other.target.clone();
        self.target_obj = other.target_obj.as_ref().map(|t| t.clone_ref(py));
    }

    /// The stored target as a canonical core expression.
    fn target_expr(&self, graph: &Arc<TargetGraph>) -> Option<TargetExpr> {
        self.target.as_deref().map(|s| parse_expr(graph, s))
    }

    /// The reference constructor's platform/os/target tuple assignment, with the
    /// `_string_or_none` filter already applied to platform and os.
    fn from_tuple_values(
        py: Python<'_>,
        platform: Option<String>,
        os: Option<String>,
        target: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        let mut arch = ArchSpec::empty();
        arch.platform = platform;
        arch.assign_os(py, os)?;
        arch.assign_target(py, target)?;
        Ok(arch)
    }

    /// `ArchSpec._autospec`.
    fn autospec<'py>(other: &Bound<'py, PyAny>) -> PyResult<Bound<'py, ArchSpec>> {
        if let Ok(arch) = other.downcast::<ArchSpec>() {
            return Ok(arch.clone());
        }
        let py = other.py();
        py.get_type::<ArchSpec>()
            .call1((other,))?
            .downcast_into::<ArchSpec>()
            .map_err(PyErr::from)
    }

    /// The comparison key of `_cmp_iter`: platform, os, target name.
    fn cmp_key(&self) -> (Option<&str>, Option<&str>, Option<&str>) {
        (
            self.platform.as_deref(),
            self.os.as_deref(),
            self.target.as_deref(),
        )
    }

    /// The `_cmp_iter` stream as the tuple `tuplify` would build.
    fn cmp_tuple<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        PyTuple::new(
            py,
            [
                self.platform.clone().into_py_any(py)?,
                self.os.clone().into_py_any(py)?,
                self.target.clone().into_py_any(py)?,
            ],
        )
    }

    fn satisfies_impl(&self, py: Python<'_>, other: &ArchSpec) -> PyResult<bool> {
        if !attr_satisfies(self.platform.as_deref(), other.platform.as_deref())
            || !attr_satisfies(self.os.as_deref(), other.os.as_deref())
        {
            return Ok(false);
        }
        // `_target_satisfies`: a missing rhs constrains nothing, a missing lhs fails.
        match (&self.target, &other.target) {
            (_, None) => Ok(true),
            (None, Some(_)) => Ok(false),
            (Some(_), Some(_)) => {
                let graph = target_graph(py)?;
                let lhs = self.target_expr(&graph);
                let rhs = other.target_expr(&graph);
                Ok(graph.target_satisfies(lhs.as_ref(), rhs.as_ref()))
            }
        }
    }

    fn intersects_impl(&self, py: Python<'_>, other: &ArchSpec) -> PyResult<bool> {
        if !attr_intersects(self.platform.as_deref(), other.platform.as_deref())
            || !attr_intersects(self.os.as_deref(), other.os.as_deref())
        {
            return Ok(false);
        }
        match (&self.target, &other.target) {
            (Some(_), Some(_)) => {
                let graph = target_graph(py)?;
                let lhs = self.target_expr(&graph);
                let rhs = other.target_expr(&graph);
                Ok(graph.target_intersects(lhs.as_ref(), rhs.as_ref()))
            }
            _ => Ok(true),
        }
    }

    /// `ArchSpec._merge` on an aliasing-safe snapshot of the other side; `other` is only
    /// used to construct the error on disjoint targets.
    fn merge_impl(
        slf: &Bound<'_, Self>,
        other: &Bound<'_, ArchSpec>,
        snapshot: &ArchSpec,
    ) -> PyResult<bool> {
        let py = slf.py();
        let mut constrained = {
            let mut this = slf.borrow_mut();
            let platform_changed = merge_attr(&mut this.platform, &snapshot.platform);
            merge_attr(&mut this.os, &snapshot.os) || platform_changed
        };
        // `_target_constrain`: an unconstrained target on either side means the other side
        // is the intersection.
        let self_target = slf.borrow().target.clone();
        constrained |= match (self_target, &snapshot.target) {
            (_, None) => false,
            (None, Some(_)) => {
                slf.borrow_mut().adopt_target(py, snapshot);
                true
            }
            (Some(lhs), Some(rhs)) => {
                let graph = target_graph(py)?;
                let lhs = parse_expr(&graph, &lhs);
                let rhs = parse_expr(&graph, rhs);
                match graph.target_constrain(Some(&lhs), Some(&rhs)) {
                    Err(_) => return Err(unsat_err(py, slf.as_any(), other.as_any())),
                    Ok((_, false)) => false,
                    Ok((Some(expr), true)) => {
                        let micro =
                            oracle(py)?.call_method1("make_microarchitecture", (expr.as_str(),))?;
                        let mut this = slf.borrow_mut();
                        this.target = Some(micro.str()?.to_cow()?.into_owned());
                        this.target_obj = Some(micro.unbind());
                        true
                    }
                    Ok((None, true)) => unreachable!("both sides are present"),
                }
            }
        };
        Ok(constrained)
    }
}

#[pymethods]
impl ArchSpec {
    #[new]
    #[pyo3(signature = (spec_or_platform_tuple=None))]
    fn new(py: Python<'_>, spec_or_platform_tuple: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
        let Some(arg) = spec_or_platform_tuple else {
            return Self::from_tuple_values(py, None, None, None);
        };
        // If another instance of ArchSpec was passed, duplicate it
        if let Ok(other) = arg.downcast::<ArchSpec>() {
            return Ok(other.borrow().clone_fields(py));
        }
        // A spec string "platform-os-target" or a (platform, os, target) tuple
        if let Ok(s) = arg.downcast::<PyString>() {
            let s = s.to_cow()?.into_owned();
            let fields: Vec<&str> = s.split('-').collect();
            if fields.len() != 3 {
                return Err(PyValueError::new_err(format!(
                    "cannot construct an ArchSpec from {s}"
                )));
            }
            let platform = Self::string_or_none(&PyString::new(py, fields[0]))?;
            let os = Self::string_or_none(&PyString::new(py, fields[1]))?;
            let target = PyString::new(py, fields[2]).into_any();
            return Self::from_tuple_values(py, platform, os, Some(&target));
        }
        if let Ok(t) = arg.downcast::<PyTuple>() {
            if t.len() != 3 {
                return Err(PyValueError::new_err(format!(
                    "cannot unpack a tuple of length {} into (platform, os, target)",
                    t.len()
                )));
            }
            let platform = Self::string_or_none(&t.get_item(0)?)?;
            let os = Self::string_or_none(&t.get_item(1)?)?;
            let target = t.get_item(2)?;
            return Self::from_tuple_values(py, platform, os, Some(&target));
        }
        Err(PyTypeError::new_err(format!(
            "cannot construct an ArchSpec from {}",
            arg.repr()?
        )))
    }

    /// Return the default architecture.
    #[staticmethod]
    fn default_arch(py: Python<'_>) -> PyResult<Self> {
        let (platform, os, target): (String, String, Bound<'_, PyAny>) =
            oracle(py)?.call_method0("default_arch_tuple")?.extract()?;
        Self::from_tuple_values(py, Some(platform), Some(os), Some(&target))
    }

    #[staticmethod]
    #[pyo3(name = "override", signature = (init_spec, change_spec))]
    fn override_(
        py: Python<'_>,
        init_spec: Option<&Bound<'_, PyAny>>,
        change_spec: &Bound<'_, PyAny>,
    ) -> PyResult<Self> {
        let mut new_spec = match init_spec {
            Some(init) if !init.is_none() => init.downcast::<ArchSpec>()?.borrow().clone_fields(py),
            _ => ArchSpec::empty(),
        };
        let change = change_spec.downcast::<ArchSpec>()?.borrow();
        if truthy(&change.platform) {
            // TODO: if the platform is changed to something that is incompatible
            // with the current os, we should implicitly remove it
            new_spec.platform = change.platform.clone();
        }
        if truthy(&change.os) {
            new_spec.os = change.os.clone();
        }
        if change.target.is_some() {
            new_spec.adopt_target(py, &change);
        }
        Ok(new_spec)
    }

    /// The platform of the architecture.
    #[getter]
    fn platform(&self) -> Option<String> {
        self.platform.clone()
    }

    #[setter(platform)]
    fn set_platform(&mut self, value: Option<&Bound<'_, PyAny>>) -> PyResult<()> {
        self.platform = Self::str_or_none(value)?;
        Ok(())
    }

    /// The OS of this ArchSpec.
    #[getter]
    fn os(&self) -> Option<String> {
        self.os.clone()
    }

    #[setter(os)]
    fn set_os(&mut self, py: Python<'_>, value: Option<&Bound<'_, PyAny>>) -> PyResult<()> {
        let value = Self::str_or_none(value)?;
        self.assign_os(py, value)
    }

    /// The target of the architecture, as an `archspec.cpu.Microarchitecture`.
    #[getter]
    fn target(&self, py: Python<'_>) -> Option<Py<PyAny>> {
        self.target_obj.as_ref().map(|t| t.clone_ref(py))
    }

    #[setter(target)]
    fn set_target(&mut self, py: Python<'_>, value: Option<&Bound<'_, PyAny>>) -> PyResult<()> {
        self.assign_target(py, value)
    }

    /// Return True if all concrete specs matching self also match other, otherwise False.
    fn satisfies(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<bool> {
        let other = Self::autospec(other)?;
        let other = other.borrow();
        self.satisfies_impl(py, &other)
    }

    /// Return True if there exists at least one concrete spec that matches both self and
    /// other, otherwise False.
    fn intersects(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<bool> {
        let other = Self::autospec(other)?;
        let other = other.borrow();
        self.intersects_impl(py, &other)
    }

    /// Projects all architecture fields that are specified in the given spec onto the
    /// instance spec if they're missing from the instance spec. Returns True iff the
    /// current instance was constrained.
    fn constrain(slf: &Bound<'_, Self>, other: &Bound<'_, PyAny>) -> PyResult<bool> {
        let py = slf.py();
        let other = Self::autospec(other)?;
        let snapshot = other.borrow().clone_fields(py);
        if !snapshot.intersects_impl(py, &slf.borrow())? {
            return Err(unsat_err(py, other.as_any(), slf.as_any()));
        }
        Self::merge_impl(slf, &other, &snapshot)
    }

    /// Intersect self with other in place, and return True iff self changed. Precondition:
    /// the two intersect.
    fn _merge(slf: &Bound<'_, Self>, other: &Bound<'_, PyAny>) -> PyResult<bool> {
        let py = slf.py();
        let other = Self::autospec(other)?;
        let snapshot = other.borrow().clone_fields(py);
        Self::merge_impl(slf, &other, &snapshot)
    }

    /// Copy the current instance and returns the clone.
    fn copy(&self, py: Python<'_>) -> Self {
        self.clone_fields(py)
    }

    /// True if the spec is concrete, False otherwise.
    #[getter]
    fn concrete(&self) -> bool {
        truthy(&self.platform)
            && truthy(&self.os)
            && self.target.is_some()
            && self.target_concrete()
    }

    /// True if the target is not a range or list.
    #[getter]
    fn target_concrete(&self) -> bool {
        self.target
            .as_deref()
            .is_some_and(|t| !t.contains(':') && !t.contains(','))
    }

    fn complete_with_defaults(slf: &Bound<'_, Self>) -> PyResult<()> {
        let py = slf.py();
        let default = Self::default_arch(py)?;
        if !truthy(&slf.borrow().platform) {
            slf.borrow_mut().platform = default.platform.clone();
        }
        if !truthy(&slf.borrow().os) {
            slf.borrow_mut().os = default.os.clone();
        }
        if slf.borrow().target.is_none() {
            slf.borrow_mut().adopt_target(py, &default);
        }
        Ok(())
    }

    fn to_dict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let target_data: Py<PyAny> = match &self.target_obj {
            // Abstract specs may have no target
            None => py.None(),
            Some(t) => {
                let t = t.bind(py);
                let vendor: String = t.getattr("vendor")?.extract()?;
                if vendor == "generic" {
                    // Generic targets represent either an architecture family (like
                    // x86_64) or a custom micro-architecture
                    t.str()?.into_py_any(py)?
                } else {
                    // Drop compiler flag information before turning the uarch into a dict
                    let data = t.call_method0("to_dict")?;
                    data.call_method1("pop", ("compilers", py.None()))?;
                    data.unbind()
                }
            }
        };
        let arch = PyDict::new(py);
        arch.set_item("platform", self.platform.clone())?;
        arch.set_item("platform_os", self.os.clone())?;
        arch.set_item("target", target_data)?;
        let result = PyDict::new(py);
        result.set_item("arch", arch)?;
        result.into_py_any(py)
    }

    /// Import an ArchSpec from raw YAML/JSON data.
    #[staticmethod]
    fn from_dict(py: Python<'_>, d: &Bound<'_, PyAny>) -> PyResult<Self> {
        let arch = d.get_item("arch")?;
        let platform = Self::string_or_none(&arch.get_item("platform")?)?;
        let os = Self::string_or_none(&arch.get_item("platform_os")?)?;
        let target_name = arch.get_item("target")?;
        if target_name.is_none() {
            return Self::from_tuple_values(py, platform, os, None);
        }
        let name = if target_name.downcast::<PyString>().is_ok() {
            target_name
        } else {
            target_name.get_item("name")?
        };
        let micro = oracle(py)?.call_method1("make_microarchitecture", (name,))?;
        Self::from_tuple_values(py, platform, os, Some(&micro))
    }

    fn __str__(&self) -> String {
        format!(
            "{}-{}-{}",
            self.platform.as_deref().unwrap_or("None"),
            self.os.as_deref().unwrap_or("None"),
            self.target.as_deref().unwrap_or("None"),
        )
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let repr_of = |value: &Option<String>| -> PyResult<String> {
            match value {
                Some(s) => Ok(PyString::new(py, s).repr()?.to_string()),
                None => Ok("None".to_string()),
            }
        };
        // The reference formats `str(self.target)`, so a missing target reprs as 'None'.
        let target = PyString::new(py, self.target.as_deref().unwrap_or("None"))
            .repr()?
            .to_string();
        Ok(format!(
            "ArchSpec(({}, {}, {target}))",
            repr_of(&self.platform)?,
            repr_of(&self.os)?,
        ))
    }

    fn __contains__(&self, py: Python<'_>, string: &Bound<'_, PyAny>) -> PyResult<bool> {
        if PyString::new(py, &self.__str__())
            .as_any()
            .contains(string)?
        {
            return Ok(true);
        }
        match &self.target_obj {
            Some(t) => t.bind(py).contains(string),
            // `string in None` raises like the reference
            None => Err(PyTypeError::new_err(
                "argument of type 'NoneType' is not iterable",
            )),
        }
    }

    /// A zero-arg callable returning an iterator, for `lang.lazy_eq` / `lang.tuplify`.
    fn _cmp_iter(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        self.cmp_tuple(py)?.as_any().try_iter()?.into_py_any(py)
    }

    fn __richcmp__(
        &self,
        py: Python<'_>,
        other: &Bound<'_, PyAny>,
        op: CompareOp,
    ) -> PyResult<Py<PyAny>> {
        if let Ok(o) = other.downcast::<ArchSpec>() {
            let o = o.borrow();
            let cmp = self.cmp_key().cmp(&o.cmp_key());
            return op.matches(cmp).into_py_any(py);
        }
        if other.is_none() {
            // The lazy_lexicographic_ordering None semantics: None is less than everything.
            let r = match op {
                CompareOp::Eq | CompareOp::Lt | CompareOp::Le => false,
                CompareOp::Ne | CompareOp::Gt | CompareOp::Ge => true,
            };
            return r.into_py_any(py);
        }
        Ok(py.NotImplemented())
    }

    /// Python's `hash(tuplify(self._cmp_iter))`: the hash of the actual Python tuple, so
    /// that tuples containing these objects (e.g. `Spec.__hash__`) hash the same.
    fn __hash__(&self, py: Python<'_>) -> PyResult<isize> {
        self.cmp_tuple(py)?.as_any().hash()
    }

    /// Rebuild from the `(platform, os, target-name)` tuple through the constructor.
    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, Py<PyAny>)> {
        let state = self.cmp_tuple(py)?;
        let args = PyTuple::new(py, [state])?;
        Ok((
            py.get_type::<ArchSpec>().into_py_any(py)?,
            args.into_py_any(py)?,
        ))
    }
}
