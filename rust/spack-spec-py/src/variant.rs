// SPDX-License-Identifier: (Apache-2.0 OR MIT)

//! PyO3 binding for the variant algebra in `spack-spec-core`, mirroring
//! `spack.variant.VariantValue`.
//!
//! The `type` property surfaces as a member of the registered `spack.variant.VariantType`
//! IntEnum, and the exceptions raised are the registered `spack.variant` classes, so
//! Python-side enum comparisons and `except` clauses keep working. `VariantValueRemoval`
//! stays a Python subclass; the class is `subclass` + `dict` so spec.py can stick
//! `_patches_in_order_of_appearance` onto instances.

use std::cmp::Ordering;

use pyo3::basic::CompareOp;
use pyo3::exceptions::{PyIndexError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyList, PyString, PyTuple};
use pyo3::IntoPyObjectExt;

use spack_spec_core::variant::{Value, VariantData, VariantError, VariantKind};

use crate::registry;

// ---------------------------------------------------------------------------
// Conversions
// ---------------------------------------------------------------------------

/// Python `{type(x)}` as used in error messages, e.g. `<class 'str'>`.
fn type_repr(obj: &Bound<'_, PyAny>) -> String {
    obj.get_type()
        .repr()
        .map(|r| r.to_string())
        .unwrap_or_else(|_| "<unknown type>".to_string())
}

fn value_from_py(obj: &Bound<'_, PyAny>) -> PyResult<Value> {
    if obj.is_none() {
        return Ok(Value::None);
    }
    if let Ok(b) = obj.downcast::<PyBool>() {
        return Ok(Value::Bool(b.is_true()));
    }
    if let Ok(s) = obj.downcast::<PyString>() {
        return Ok(Value::Str(s.to_cow()?.into_owned()));
    }
    Err(PyTypeError::new_err(format!(
        "variant values must be str or bool, not {}",
        type_repr(obj)
    )))
}

fn value_to_py(py: Python<'_>, v: &Value) -> PyResult<Py<PyAny>> {
    match v {
        Value::Bool(b) => b.into_py_any(py),
        Value::Str(s) => s.into_py_any(py),
        Value::None => Ok(py.None()),
    }
}

fn values_from_iterable(obj: &Bound<'_, PyAny>) -> PyResult<Vec<Value>> {
    obj.try_iter()?.map(|item| value_from_py(&item?)).collect()
}

/// The variant type as the registered `spack.variant.VariantType` member, or the bare
/// integer when the binding runs without the Python side registered.
fn kind_to_py(py: Python<'_>, kind: VariantKind) -> PyResult<Py<PyAny>> {
    match registry::VARIANT_TYPE.get(py) {
        Some(cls) => Ok(cls.bind(py).call1((kind as i64,))?.unbind()),
        None => (kind as i64).into_py_any(py),
    }
}

/// Python `int(obj)` to a [`VariantKind`], accepting the IntEnum members and plain ints.
fn kind_from_py(obj: &Bound<'_, PyAny>) -> PyResult<VariantKind> {
    let value: i64 = match obj.extract() {
        Ok(v) => v,
        Err(_) => obj.call_method0("__int__")?.extract()?,
    };
    VariantKind::from_int(value)
        .ok_or_else(|| PyValueError::new_err(format!("invalid variant type: {value}")))
}

fn values_tuple<'py>(py: Python<'py>, data: &VariantData) -> PyResult<Bound<'py, PyTuple>> {
    let values: Vec<Py<PyAny>> = data
        .values()
        .iter()
        .map(|v| value_to_py(py, v))
        .collect::<PyResult<_>>()?;
    PyTuple::new(py, values)
}

/// The `_cmp_iter` stream as the tuple `tuplify` would build: name, propagate, concrete,
/// then `str(v)` for each value.
fn cmp_tuple<'py>(py: Python<'py>, data: &VariantData) -> PyResult<Bound<'py, PyTuple>> {
    let mut items: Vec<Py<PyAny>> = vec![
        data.name.clone().into_py_any(py)?,
        data.propagate.into_py_any(py)?,
        data.concrete.into_py_any(py)?,
    ];
    for v in data.values() {
        items.push(v.render().into_py_any(py)?);
    }
    PyTuple::new(py, items)
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Instantiate a registered Python exception class with the given arguments.
fn registered_err<'py, A>(
    py: Python<'py>,
    slot: &pyo3::sync::PyOnceLock<Py<PyAny>>,
    args: A,
    fallback: PyErr,
) -> PyErr
where
    A: pyo3::call::PyCallArgs<'py>,
{
    match slot.get(py) {
        Some(cls) => match cls.bind(py).call1(args) {
            Ok(exc) => PyErr::from_value(exc),
            Err(e) => e,
        },
        None => fallback,
    }
}

/// A stand-in variant object for `MultipleValuesInExclusiveVariantError(variant)` when the
/// error occurs before a Python object exists; the exception only reads `variant.name`.
fn placeholder(py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
    let inner = VariantData::new(VariantKind::Multi, name, Vec::new(), false, false)
        .expect("an empty multi-valued variant is always valid");
    Py::new(py, VariantValue { inner })?.into_py_any(py)
}

/// Map a core `VariantError` from `set` to the Python exception the reference raises.
/// `variant` is the object under mutation, or `None` during construction.
fn set_err(
    py: Python<'_>,
    e: VariantError,
    variant: Option<&Bound<'_, PyAny>>,
    name: &str,
) -> PyErr {
    match e {
        VariantError::MultipleValues => {
            let obj = match variant {
                Some(v) => v.clone().unbind(),
                None => match placeholder(py, name) {
                    Ok(p) => p,
                    Err(e) => return e,
                },
            };
            registered_err(
                py,
                &registry::MULTIPLE_VALUES_ERROR,
                (obj,),
                PyValueError::new_err(format!(
                    "multiple values are not allowed for variant '{name}'"
                )),
            )
        }
        VariantError::NonBooleanValue(value) => PyValueError::new_err(format!(
            "cannot set a boolean variant to a value that is not a boolean: {value}"
        )),
        VariantError::ReservedStar => {
            let msg = "cannot use reserved value '*'";
            registered_err(
                py,
                &registry::INVALID_VARIANT_VALUE_ERROR,
                (msg,),
                PyValueError::new_err(msg),
            )
        }
        // Python's sorted() over mixed value types.
        VariantError::UnorderableValues => {
            PyTypeError::new_err("'<' not supported between instances of 'bool' and 'str'")
        }
        VariantError::Unsatisfiable => {
            PyValueError::new_err(format!("unsatisfiable constraint on variant '{name}'"))
        }
    }
}

/// `UnsatisfiableVariantSpecError(provided, required)`.
fn unsat_err(py: Python<'_>, provided: &Bound<'_, PyAny>, required: &Bound<'_, PyAny>) -> PyErr {
    let fallback = PyValueError::new_err(format!(
        "'{}' does not satisfy '{}'",
        provided.str().map(|s| s.to_string()).unwrap_or_default(),
        required.str().map(|s| s.to_string()).unwrap_or_default(),
    ));
    registered_err(
        py,
        &registry::UNSATISFIABLE_VARIANT_ERROR,
        (provided, required),
        fallback,
    )
}

// ---------------------------------------------------------------------------
// VariantValue
// ---------------------------------------------------------------------------

/// A named set of variant values (`spack.variant.VariantValue`).
#[pyclass(subclass, dict, name = "VariantValue", module = "spack_spec")]
pub struct VariantValue {
    inner: VariantData,
}

fn new_variant(
    py: Python<'_>,
    kind: VariantKind,
    name: &str,
    values: Vec<Value>,
    propagate: bool,
    concrete: bool,
) -> PyResult<Py<PyAny>> {
    match VariantData::new(kind, name, values, propagate, concrete) {
        Ok(inner) => Py::new(py, VariantValue { inner })?.into_py_any(py),
        Err(e) => Err(set_err(py, e, None, name)),
    }
}

fn downcast_variant<'a, 'py>(
    method: &str,
    other: &'a Bound<'py, PyAny>,
) -> PyResult<&'a Bound<'py, VariantValue>> {
    other.downcast::<VariantValue>().map_err(|_| {
        PyTypeError::new_err(format!(
            "'{method}()' not supported for instances of {}",
            type_repr(other)
        ))
    })
}

/// Compare the `_cmp_iter` streams like the lazy lexicographic ordering would.
fn cmp_data(a: &VariantData, b: &VariantData) -> Ordering {
    a.cmp_key().cmp(&b.cmp_key())
}

#[pymethods]
impl VariantValue {
    #[new]
    #[pyo3(signature = (r#type, name, value, *, propagate=false, concrete=false))]
    fn new(
        py: Python<'_>,
        r#type: &Bound<'_, PyAny>,
        name: &str,
        value: &Bound<'_, PyAny>,
        propagate: bool,
        concrete: bool,
    ) -> PyResult<Self> {
        let kind = kind_from_py(r#type)?;
        let values = values_from_iterable(value)?;
        match VariantData::new(kind, name, values, propagate, concrete) {
            Ok(inner) => Ok(VariantValue { inner }),
            Err(e) => Err(set_err(py, e, None, name)),
        }
    }

    #[getter]
    fn name(&self) -> String {
        self.inner.name.clone()
    }

    #[setter]
    fn set_name(&mut self, value: String) {
        self.inner.name = value;
    }

    #[getter]
    fn propagate(&self) -> bool {
        self.inner.propagate
    }

    #[setter]
    fn set_propagate(&mut self, value: bool) {
        self.inner.propagate = value;
    }

    #[getter]
    fn concrete(&self) -> bool {
        self.inner.concrete
    }

    #[setter]
    fn set_concrete(&mut self, value: bool) {
        self.inner.concrete = value;
    }

    #[getter(r#type)]
    fn get_type(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        kind_to_py(py, self.inner.kind)
    }

    /// A raw attribute assignment in Python: narrowing the type does not touch the
    /// concreteness bit or re-validate the values.
    #[setter(r#type)]
    fn set_type(&mut self, value: &Bound<'_, PyAny>) -> PyResult<()> {
        self.inner.kind = kind_from_py(value)?;
        Ok(())
    }

    #[getter]
    fn values<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        values_tuple(py, &self.inner)
    }

    #[getter]
    fn value(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        if self.inner.kind == VariantKind::Multi {
            return values_tuple(py, &self.inner)?.into_py_any(py);
        }
        match self.inner.values().first() {
            Some(v) => value_to_py(py, v),
            None => Err(PyIndexError::new_err("tuple index out of range")),
        }
    }

    #[staticmethod]
    #[pyo3(signature = (name, value, *, propagate=false, r#abstract=false))]
    fn from_node_dict(
        py: Python<'_>,
        name: &str,
        value: &Bound<'_, PyAny>,
        propagate: bool,
        r#abstract: bool,
    ) -> PyResult<Py<PyAny>> {
        if let Ok(list) = value.downcast::<PyList>() {
            let values = values_from_iterable(list.as_any())?;
            return new_variant(py, VariantKind::Multi, name, values, propagate, !r#abstract);
        }
        let s = value.str()?.to_cow()?.into_owned();
        if s.eq_ignore_ascii_case("true") || s.eq_ignore_ascii_case("false") {
            let b = Value::Bool(s.eq_ignore_ascii_case("true"));
            return new_variant(py, VariantKind::Bool, name, vec![b], propagate, false);
        }
        let single = value_from_py(value)?;
        new_variant(
            py,
            VariantKind::Single,
            name,
            vec![single],
            propagate,
            false,
        )
    }

    #[staticmethod]
    #[pyo3(signature = (name, value, *, propagate=false, concrete=false))]
    fn from_string_or_bool(
        py: Python<'_>,
        name: &str,
        value: &Bound<'_, PyAny>,
        propagate: bool,
        concrete: bool,
    ) -> PyResult<Py<PyAny>> {
        if let Ok(b) = value.downcast::<PyBool>() {
            let b = Value::Bool(b.is_true());
            return new_variant(py, VariantKind::Bool, name, vec![b], propagate, false);
        }
        let s = value
            .downcast::<PyString>()
            .map_err(|_| {
                PyTypeError::new_err(format!(
                    "variant value must be str or bool, not {}",
                    type_repr(value)
                ))
            })?
            .to_cow()?
            .into_owned();
        if s.eq_ignore_ascii_case("true") || s.eq_ignore_ascii_case("false") {
            let b = Value::Bool(s.eq_ignore_ascii_case("true"));
            return new_variant(py, VariantKind::Bool, name, vec![b], propagate, false);
        }
        if s == "*" {
            return new_variant(py, VariantKind::Multi, name, Vec::new(), propagate, false);
        }
        let values = s.split(',').map(|p| Value::Str(p.to_string())).collect();
        new_variant(py, VariantKind::Multi, name, values, propagate, concrete)
    }

    /// Reconstruct a variant from concretizer output.
    #[staticmethod]
    fn from_concretizer(
        py: Python<'_>,
        name: &str,
        value: &str,
        r#type: &str,
    ) -> PyResult<Py<PyAny>> {
        match r#type {
            "bool" => {
                let b = Value::Bool(value == "True");
                new_variant(py, VariantKind::Bool, name, vec![b], false, false)
            }
            "multi" => {
                let v = Value::Str(value.to_string());
                new_variant(py, VariantKind::Multi, name, vec![v], false, true)
            }
            _ => {
                let v = Value::Str(value.to_string());
                new_variant(py, VariantKind::Single, name, vec![v], false, false)
            }
        }
    }

    /// A `(key, value)` tuple suitable to be an entry in a yaml dict.
    fn yaml_entry(&self, py: Python<'_>) -> PyResult<(String, Py<PyAny>)> {
        let name = self.inner.name.clone();
        if self.inner.kind == VariantKind::Multi {
            let values: Vec<Py<PyAny>> = self
                .inner
                .values()
                .iter()
                .map(|v| value_to_py(py, v))
                .collect::<PyResult<_>>()?;
            return Ok((name, PyList::new(py, values)?.into_py_any(py)?));
        }
        match self.inner.values().first() {
            Some(v) => Ok((name, value_to_py(py, v)?)),
            None => Err(PyIndexError::new_err("tuple index out of range")),
        }
    }

    /// Set the value(s) of the variant.
    #[pyo3(signature = (*values))]
    fn set(slf: &Bound<'_, Self>, values: &Bound<'_, PyTuple>) -> PyResult<()> {
        let values = values_from_iterable(values.as_any())?;
        let name = slf.borrow().inner.name.clone();
        let result = slf.borrow_mut().inner.set(values);
        result.map_err(|e| set_err(slf.py(), e, Some(slf.as_any()), &name))
    }

    fn append(slf: &Bound<'_, Self>, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let value = value_from_py(value)?;
        let name = slf.borrow().inner.name.clone();
        let result = slf.borrow_mut().inner.append(value);
        result.map_err(|e| set_err(slf.py(), e, Some(slf.as_any()), &name))
    }

    /// A new independent `VariantValue`; like Python's `copy`, the base class is
    /// constructed and the instance `__dict__` is not copied.
    fn copy(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        Py::new(
            py,
            VariantValue {
                inner: self.inner.clone(),
            },
        )?
        .into_py_any(py)
    }

    /// The lhs satisfies the rhs if all possible concretizations of lhs are also
    /// possible concretizations of rhs.
    fn satisfies(&self, other: &Bound<'_, PyAny>) -> PyResult<bool> {
        let other = downcast_variant("satisfies", other)?;
        Ok(self.inner.satisfies(&other.borrow().inner))
    }

    /// True iff there exists a concretization that satisfies both lhs and rhs.
    fn intersects(&self, other: &Bound<'_, PyAny>) -> PyResult<bool> {
        let other = downcast_variant("intersects", other)?;
        Ok(self.inner.intersects(&other.borrow().inner))
    }

    /// Constrain self with other if they intersect. Returns true iff self was changed.
    fn constrain(slf: &Bound<'_, Self>, other: &Bound<'_, PyAny>) -> PyResult<bool> {
        let other_data = downcast_variant("constrain", other)?.borrow().inner.clone();
        if !slf.borrow().inner.intersects(&other_data) {
            return Err(unsat_err(slf.py(), slf.as_any(), other));
        }
        let name = slf.borrow().inner.name.clone();
        let result = slf.borrow_mut().inner.merge(&other_data);
        result.map_err(|e| set_err(slf.py(), e, Some(slf.as_any()), &name))
    }

    /// Constrain self with other, which is known to intersect.
    fn _merge(slf: &Bound<'_, Self>, other: &Bound<'_, PyAny>) -> PyResult<bool> {
        let other_data = downcast_variant("_merge", other)?.borrow().inner.clone();
        let name = slf.borrow().inner.name.clone();
        let result = slf.borrow_mut().inner.merge(&other_data);
        result.map_err(|e| set_err(slf.py(), e, Some(slf.as_any()), &name))
    }

    fn __contains__(&self, item: &Bound<'_, PyAny>) -> bool {
        match value_from_py(item) {
            Ok(v) => self.inner.values().contains(&v),
            Err(_) => false,
        }
    }

    fn __str__(&self) -> String {
        self.inner.render()
    }

    fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
        let py = slf.py();
        let b = slf.borrow();
        let type_repr = kind_to_py(py, b.inner.kind)?.bind(py).repr()?.to_string();
        let name_repr = PyString::new(py, &b.inner.name).repr()?.to_string();
        let values_repr = values_tuple(py, &b.inner)?.repr()?.to_string();
        let propagate = if b.inner.propagate { "True" } else { "False" };
        let concrete = if b.inner.concrete { "True" } else { "False" };
        Ok(format!(
            "VariantValue({type_repr}, {name_repr}, {values_repr}, \
             propagate={propagate}, concrete={concrete})"
        ))
    }

    /// A zero-arg callable returning an iterator, for `lang.lazy_eq` / `lang.tuplify`.
    fn _cmp_iter(slf: &Bound<'_, Self>) -> PyResult<Py<PyAny>> {
        let py = slf.py();
        cmp_tuple(py, &slf.borrow().inner)?
            .as_any()
            .try_iter()?
            .into_py_any(py)
    }

    fn __richcmp__(
        slf: &Bound<'_, Self>,
        other: &Bound<'_, PyAny>,
        op: CompareOp,
    ) -> PyResult<Py<PyAny>> {
        let py = slf.py();
        if let Ok(o) = other.downcast::<VariantValue>() {
            let cmp = cmp_data(&slf.borrow().inner, &o.borrow().inner);
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
    /// that tuples containing these objects (e.g. in `HashableMap.__hash__`) hash the same.
    fn __hash__(slf: &Bound<'_, Self>) -> PyResult<isize> {
        cmp_tuple(slf.py(), &slf.borrow().inner)?.as_any().hash()
    }

    /// Like the reference's slots-based pickling: rebuild from `(type, name, values)` and
    /// restore propagate/concrete plus any instance `__dict__` entries
    /// (`_patches_in_order_of_appearance`) from the state dict.
    fn __reduce__(slf: &Bound<'_, Self>) -> PyResult<(Py<PyAny>, Py<PyAny>, Py<PyAny>)> {
        let py = slf.py();
        let (kind, name, propagate, concrete, values) = {
            let b = slf.borrow();
            (
                b.inner.kind,
                b.inner.name.clone(),
                b.inner.propagate,
                b.inner.concrete,
                values_tuple(py, &b.inner)?,
            )
        };
        let args = PyTuple::new(
            py,
            [
                kind_to_py(py, kind)?,
                name.into_py_any(py)?,
                values.into_py_any(py)?,
            ],
        )?;
        let state = PyDict::new(py);
        state.set_item("propagate", propagate)?;
        state.set_item("concrete", concrete)?;
        if let Ok(instance_dict) = slf.getattr("__dict__") {
            if let Ok(instance_dict) = instance_dict.downcast::<PyDict>() {
                state.update(instance_dict.as_mapping())?;
            }
        }
        Ok((
            py.get_type::<VariantValue>().into_py_any(py)?,
            args.into_py_any(py)?,
            state.into_py_any(py)?,
        ))
    }

    fn __setstate__(slf: &Bound<'_, Self>, state: &Bound<'_, PyAny>) -> PyResult<()> {
        let state = state.downcast::<PyDict>()?;
        for (key, value) in state.iter() {
            let name: String = key.extract()?;
            match name.as_str() {
                "propagate" => slf.borrow_mut().inner.propagate = value.extract()?,
                "concrete" => slf.borrow_mut().inner.concrete = value.extract()?,
                _ => slf.setattr(name.as_str(), value)?,
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// VariantMap rendering
// ---------------------------------------------------------------------------

/// `VariantMap.__str__`, without a Python frame per entry.
///
/// Formatting a spec asks for this once per node, and the Python version pays the `Mapping` ABC
/// for `keys()`, a lambda per key and a `str()` call per value. Here the map's backing dict is
/// read once and every value renders through [`VariantData::render`] directly.
///
/// PY: boolean variants come first, with no separator, so a spec never starts a token with
/// ` ~foo` (which zsh eats); key-value variants each get a leading space. Both groups keep
/// sorted key order, and Rust orders `str` by UTF-8 bytes exactly as Python orders by code
/// point.
#[pyfunction]
pub fn render_variant_map(map: &Bound<'_, PyAny>) -> PyResult<String> {
    let dict = map.getattr("dict")?;
    let dict = dict.downcast::<PyDict>()?;
    if dict.is_empty() {
        return Ok(String::new());
    }

    let mut entries: Vec<(String, Bound<'_, PyAny>)> = Vec::with_capacity(dict.len());
    for (key, value) in dict.iter() {
        entries.push((key.extract()?, value));
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let mut bools = String::new();
    let mut pairs = String::new();
    for (_, value) in &entries {
        // A value that is not backed by the extension (nothing in spack does this today) still
        // renders, through its own __str__.
        let (is_bool, rendered) = match value.downcast::<VariantValue>() {
            Ok(variant) => {
                let variant = variant.borrow();
                (
                    variant.inner.kind == VariantKind::Bool,
                    variant.inner.render(),
                )
            }
            Err(_) => (
                value
                    .getattr("type")?
                    .eq(kind_to_py(map.py(), VariantKind::Bool)?)?,
                value.str()?.to_cow()?.into_owned(),
            ),
        };
        if is_bool {
            bools.push_str(&rendered);
        } else {
            pairs.push(' ');
            pairs.push_str(&rendered);
        }
    }
    bools.push_str(&pairs);
    Ok(bools)
}
