// SPDX-License-Identifier: (Apache-2.0 OR MIT)

//! The construction, mutation and copy path of `spack.spec.Spec`: the parser entry points
//! (`_add_flag`, `_set_architecture`, `add_dependency_edge`), the edge decision procedure
//! `_add_or_merge_edge` with its predicates, the read accessors over the edge maps, and
//! `_dup`/`_dup_deps`/`copy`. Transcribed statement by statement from the reference
//! implementation in `lib/spack/spack/spec.py`, which remains the python-mode
//! implementation. All mutations go through the Python attribute protocol so subclass
//! guards (`_ImmutableSpec.__setattr__`) fire exactly like they do for the reference.

use std::collections::HashMap;

use pyo3::exceptions::PyAssertionError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyInt, PyList, PyString, PyTuple};

use crate::algebra::{
    add_edge_to_map, condition_must_hold, edge_child, edge_depflag, edge_direct, edge_parent,
    edge_propagation, edge_virtuals, edge_when, empty_spec_obj, has_deps, out_edges, ptr,
    registered, satisfies_edge_attributes, siblings_of, spec_name, spec_satisfies, DT_ALL,
    PROPAGATION_NONE,
};
use crate::registry;

/// `getattr(obj, name, None)`: only an `AttributeError` maps to `None`.
pub(crate) fn getattr_or_none<'py>(
    obj: &Bound<'py, PyAny>,
    name: &str,
) -> PyResult<Bound<'py, PyAny>> {
    let py = obj.py();
    match obj.getattr(name) {
        Ok(value) => Ok(value),
        Err(err) if err.is_instance_of::<pyo3::exceptions::PyAttributeError>(py) => {
            Ok(py.None().into_bound(py))
        }
        Err(err) => Err(err),
    }
}

/// An error instance of a registered class with the given message, raised as an exception.
fn registered_error(
    py: Python<'_>,
    cell: &'static pyo3::sync::PyOnceLock<Py<PyAny>>,
    what: &str,
    message: &str,
) -> PyResult<PyErr> {
    let cls = registered(py, cell, what)?;
    Ok(PyErr::from_value(cls.call1((message,))?))
}

// --------------------------------------------------------------------------------------
// Edge selection: `_select_edges` and the read accessors built on it
// --------------------------------------------------------------------------------------

/// `_select_edges`, with the filters applied in the reference order.
pub(crate) fn select_edges<'py>(
    edge_map: &Bound<'py, PyAny>,
    parent: Option<&str>,
    child: Option<&str>,
    depflag: u64,
    virtuals: Option<&Bound<'py, PyAny>>,
) -> PyResult<Vec<Bound<'py, PyAny>>> {
    if depflag == 0 {
        return Ok(Vec::new());
    }

    // Start from all the edges we store. A non-dict edge map (e.g. None on a spec that was
    // never initialized) fails with the same AttributeError the reference produces.
    let mut all_edges: Vec<Bound<'py, PyAny>> = Vec::new();
    if let Ok(dict) = edge_map.downcast::<PyDict>() {
        for bucket in dict.values() {
            for edge in bucket.try_iter()? {
                all_edges.push(edge?);
            }
        }
    } else {
        for bucket in edge_map.call_method0("values")?.try_iter()? {
            for edge in bucket?.try_iter()? {
                all_edges.push(edge?);
            }
        }
    }

    let virtuals_filter: Option<Vec<String>> = match virtuals {
        Some(v) if v.downcast::<PyString>().is_err() => Some(
            v.try_iter()?
                .map(|item| item?.extract::<String>())
                .collect::<PyResult<_>>()?,
        ),
        _ => None,
    };

    let mut selected = Vec::new();
    for edge in all_edges {
        // Filter by parent name
        if let Some(parent) = parent {
            let name: String = edge_parent(&edge)?.getattr("name")?.extract()?;
            if name != parent {
                continue;
            }
        }

        // Filter by child name
        if let Some(child) = child {
            let name: String = edge_child(&edge)?.getattr("name")?.extract()?;
            if name != child {
                continue;
            }
        }

        // Filter by allowed dependency types
        if depflag != DT_ALL {
            let edge_flag = edge_depflag(&edge)?;
            if edge_flag != 0 && depflag & edge_flag == 0 {
                continue;
            }
        }

        // Filter by virtuals
        if let Some(v) = virtuals {
            let edge_vs = edge_virtuals(&edge)?;
            if let Some(wanted) = &virtuals_filter {
                if !wanted.iter().any(|w| edge_vs.contains(w)) {
                    continue;
                }
            } else {
                let wanted: String = v.extract()?;
                if !edge_vs.contains(&wanted) {
                    continue;
                }
            }
        }

        selected.push(edge);
    }
    Ok(selected)
}

/// `dt.canonicalize` for `deptype` arguments: an int (`dt.DepFlag`) passes through, high
/// level input goes through the registered Python function.
pub(crate) fn canonical_depflag(deptype: &Bound<'_, PyAny>) -> PyResult<u64> {
    if deptype.is_instance_of::<PyInt>() {
        deptype.extract()
    } else {
        registered(
            deptype.py(),
            &registry::DEPTYPE_CANONICALIZE,
            "deptypes.canonicalize",
        )?
        .call1((deptype,))?
        .extract()
    }
}

/// `Spec._get_dependency`.
pub(crate) fn get_dependency<'py>(
    slf: &Bound<'py, PyAny>,
    name: &str,
) -> PyResult<Bound<'py, PyAny>> {
    let deps = select_edges(
        &slf.getattr("_dependencies")?,
        None,
        Some(name),
        DT_ALL,
        None,
    )?;
    if deps.len() != 1 {
        return Err(registered_error(
            slf.py(),
            &registry::SPEC_ERROR,
            "SpecError",
            &format!(
                "expected only 1 \"{}\" dependency, but got {}",
                name,
                deps.len()
            ),
        )?);
    }
    Ok(deps[0].clone())
}

// --------------------------------------------------------------------------------------
// Parser entry points: `_add_flag` and `_set_architecture`
// --------------------------------------------------------------------------------------

/// `Spec._set_architecture`.
pub(crate) fn set_architecture(
    slf: &Bound<'_, PyAny>,
    kwargs: Option<&Bound<'_, PyDict>>,
) -> PyResult<()> {
    let py = slf.py();
    let arch_attrs = ["platform", "os", "target"];
    let arch = slf.getattr("architecture")?;
    if arch.is_truthy()? && arch.getattr("concrete")?.is_truthy()? {
        return Err(registered_error(
            py,
            &registry::DUPLICATE_ARCH_ERROR,
            "DuplicateArchitectureError",
            "Spec cannot have two architectures.",
        )?);
    }

    if !arch.is_truthy()? {
        let mut new_vals: Vec<Bound<'_, PyAny>> = Vec::new();
        for attr in arch_attrs {
            let value = match kwargs.and_then(|k| k.get_item(attr).transpose()) {
                Some(value) => value?,
                None => py.None().into_bound(py),
            };
            new_vals.push(value);
        }
        let arch_cls = py.get_type::<crate::arch::ArchSpec>();
        slf.setattr(
            "architecture",
            arch_cls.call1((PyTuple::new(py, new_vals)?,))?,
        )?;
    } else if let Some(kwargs) = kwargs {
        for (key, value) in kwargs.iter() {
            let attr: String = key.extract()?;
            if !arch_attrs.contains(&attr.as_str()) {
                continue;
            }
            if arch.getattr(attr.as_str())?.is_truthy()? {
                return Err(registered_error(
                    py,
                    &registry::DUPLICATE_ARCH_ERROR,
                    "DuplicateArchitectureError",
                    &format!("Cannot specify '{attr}' twice"),
                )?);
            }
            arch.setattr(attr.as_str(), value)?;
        }
    }
    Ok(())
}

/// One `_set_architecture(key=value, ...)` call with exactly the given keys.
fn set_architecture_kwargs(
    slf: &Bound<'_, PyAny>,
    pairs: &[(&str, &Bound<'_, PyAny>)],
) -> PyResult<()> {
    let kwargs = PyDict::new(slf.py());
    for (key, value) in pairs {
        kwargs.set_item(*key, *value)?;
    }
    set_architecture(slf, Some(&kwargs))
}

/// `Spec._add_flag`: called by the parser to add a known flag.
pub(crate) fn add_flag(
    slf: &Bound<'_, PyAny>,
    name: &str,
    value: &Bound<'_, PyAny>,
    propagate: bool,
    concrete: bool,
) -> PyResult<()> {
    let py = slf.py();

    if propagate
        && registered(py, &registry::RESERVED_VARIANT_NAMES, "RESERVED_NAMES")?.contains(name)?
    {
        return Err(registered_error(
            py,
            &registry::UNSUPPORTED_PROPAGATION_ERROR,
            "UnsupportedPropagationError",
            &format!("Propagation with '==' is not supported for '{name}'."),
        )?);
    }

    let valid_flags = registered(py, &registry::FLAG_MAP_CLASS, "the FlagMap class")?
        .call_method0("valid_compiler_flags")?;
    if name == "arch" || name == "architecture" {
        if !value.is_exact_instance_of::<PyString>() {
            return Err(PyAssertionError::new_err(
                "architecture have a string value",
            ));
        }
        let s: String = value.extract()?;
        let parts: Vec<&str> = s.split('-').collect();
        let none = py.None().into_bound(py);
        let (plat, os, tgt): (Bound<'_, PyAny>, Bound<'_, PyAny>, Bound<'_, PyAny>) =
            if parts.len() == 3 {
                (
                    PyString::new(py, parts[0]).into_any(),
                    PyString::new(py, parts[1]).into_any(),
                    PyString::new(py, parts[2]).into_any(),
                )
            } else {
                (none.clone(), none.clone(), value.clone())
            };
        set_architecture_kwargs(slf, &[("platform", &plat), ("os", &os), ("target", &tgt)])?;
    } else if name == "platform" {
        set_architecture_kwargs(slf, &[("platform", value)])?;
    } else if name == "os" || name == "operating_system" {
        set_architecture_kwargs(slf, &[("os", value)])?;
    } else if name == "target" {
        set_architecture_kwargs(slf, &[("target", value)])?;
    } else if name == "namespace" {
        slf.setattr("namespace", value)?;
    } else if valid_flags.contains(name)? {
        let compiler_flags = slf.getattr("compiler_flags")?;
        if compiler_flags.is_none() {
            return Err(PyAssertionError::new_err(()));
        }
        if !value.is_exact_instance_of::<PyString>() {
            return Err(PyAssertionError::new_err(format!(
                "{name} must have a string value"
            )));
        }
        let flags_and_propagation: Vec<(String, Bound<'_, PyAny>)> =
            registered(py, &registry::TOKENIZE_FLAGS, "tokenize_flags")?
                .call1((value, propagate))?
                .extract()?;
        let flag_group = flags_and_propagation
            .iter()
            .map(|(x, _)| x.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        for (flag, propagation) in &flags_and_propagation {
            compiler_flags.call_method1(
                "add_flag",
                (name, flag.as_str(), propagation, flag_group.as_str()),
            )?;
        }
    } else {
        let kwargs = PyDict::new(py);
        kwargs.set_item("propagate", propagate)?;
        kwargs.set_item("concrete", concrete)?;
        let variant = py.get_type::<crate::variant::VariantValue>().call_method(
            "from_string_or_bool",
            (name, value),
            Some(&kwargs),
        )?;
        slf.getattr("variants")?.set_item(name, variant)?;
    }
    Ok(())
}

// --------------------------------------------------------------------------------------
// The edge decision procedure: `add_dependency_edge` and `_add_or_merge_edge`
// --------------------------------------------------------------------------------------

/// `_same_direct_dep`: two edges of one parent refer to the same dependency when they are
/// direct deps, refer to the same package name, and have the same when condition.
fn same_direct_dep(lhs: &Bound<'_, PyAny>, rhs: &Bound<'_, PyAny>) -> PyResult<bool> {
    if !edge_direct(lhs)? || !edge_direct(rhs)? {
        return Ok(false);
    }
    let lhs_name = spec_name(&edge_child(lhs)?)?;
    if lhs_name.is_empty() || lhs_name != spec_name(&edge_child(rhs)?)? {
        return Ok(false);
    }
    edge_when(lhs)?.eq(&edge_when(rhs)?)
}

/// `_implies_edge`: whether every DAG satisfying `narrower` also satisfies `wider`, which
/// makes `wider` redundant.
fn implies_edge(narrower: &Bound<'_, PyAny>, wider: &Bound<'_, PyAny>) -> PyResult<bool> {
    // A direct dependency can only be implied by another direct dependency.
    if !(edge_direct(narrower)? || !edge_direct(wider)?) {
        return Ok(false);
    }
    // A propagated edge is an input to the solver's objective even where it does not narrow
    // the solution set, so satisfaction alone does not make it redundant.
    let wider_propagation = edge_propagation(wider)?;
    if wider_propagation != PROPAGATION_NONE && edge_propagation(narrower)? != wider_propagation {
        return Ok(false);
    }
    if !satisfies_edge_attributes(narrower, wider)? {
        return Ok(false);
    }
    // A dependency of wider's child that narrower's lacks has to survive: the two are
    // independent requirements, one edge requiring both at once is narrower than their meet.
    Ok(!has_deps(&edge_child(wider)?)?
        || spec_satisfies(&edge_child(narrower)?, &edge_child(wider)?, true)?)
}

/// `Spec.add_dependency_edge`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn add_dependency_edge(
    slf: &Bound<'_, PyAny>,
    dependency_spec: &Bound<'_, PyAny>,
    depflag: u64,
    virtuals: &Bound<'_, PyAny>,
    direct: bool,
    propagation: Option<&Bound<'_, PyAny>>,
    when: Option<&Bound<'_, PyAny>>,
) -> PyResult<()> {
    let py = slf.py();
    let when: Bound<'_, PyAny> = match when {
        Some(w) if !w.is_none() => w.clone(),
        _ => empty_spec_obj(py)?,
    };

    // An edge object already registered on both the parent and the child is updated in
    // place, rather than treated as a second, competing constraint: both sides have to see
    // the same modification.
    let name = spec_name(dependency_spec)?;
    let bucket = slf
        .getattr("_dependencies")?
        .call_method1("get", (name.as_str(), PyList::empty(py)))?;
    for edge in bucket.try_iter()? {
        let edge = edge?;
        if edge_child(&edge)?.is(dependency_spec) && edge_when(&edge)?.eq(&when)? {
            let kwargs = PyDict::new(py);
            kwargs.set_item("depflag", depflag)?;
            edge.call_method("update_deptypes", (), Some(&kwargs))?;
            let kwargs = PyDict::new(py);
            kwargs.set_item("virtuals", virtuals)?;
            edge.call_method("update_virtuals", (), Some(&kwargs))?;
            return Ok(());
        }
    }

    let cls = registered(py, &registry::DEPENDENCY_SPEC_CLASS, "DependencySpec class")?;
    let kwargs = PyDict::new(py);
    kwargs.set_item("depflag", depflag)?;
    kwargs.set_item("virtuals", virtuals)?;
    kwargs.set_item("direct", direct)?;
    if let Some(propagation) = propagation {
        kwargs.set_item("propagation", propagation)?;
    }
    kwargs.set_item("when", &when)?;
    let candidate = cls.call((slf, dependency_spec), Some(&kwargs))?;
    slf.call_method1("_add_or_merge_edge", (candidate,))?;
    Ok(())
}

/// `Spec._add_or_merge_edge`: add `candidate` as a dependency edge. Returns whether `self`
/// changed, and the edge that carries the candidate's constraint afterwards.
pub(crate) fn add_or_merge_edge<'py>(
    slf: &Bound<'py, PyAny>,
    candidate: &Bound<'py, PyAny>,
    owned: bool,
) -> PyResult<(bool, Bound<'py, PyAny>)> {
    let py = slf.py();

    // The candidate is merged into an existing edge when the two can only ever be realized
    // by a single dependency, and only where its when condition must hold.
    let name = spec_name(&edge_child(candidate)?)?;
    let mut partner: Option<Bound<'py, PyAny>> = None;
    if !name.is_empty() && condition_must_hold(&edge_when(candidate)?, slf)? {
        for edge in out_edges(slf)? {
            if same_direct_dep(&edge, candidate)? {
                partner = Some(edge);
                break;
            }
        }
    }

    // A candidate implied by an existing edge adds nothing and is discarded. Implication is
    // one-sided: only an edge implying `candidate` makes it redundant.
    if partner.is_none() {
        for edge in out_edges(slf)? {
            if !name.is_empty() {
                let edge_name = spec_name(&edge_child(&edge)?)?;
                if edge_name != name && !edge_virtuals(&edge)?.contains(&name) {
                    continue;
                }
            }
            if implies_edge(&edge, candidate)? {
                return Ok((false, edge));
            }
        }
    }

    // Each virtual of a node comes from exactly one of its direct dependencies, so a direct
    // candidate bringing a virtual that a differently named direct edge already lists can
    // never be concretized. Only unconditional pairs are refused here.
    let empty = empty_spec_obj(py)?;
    let candidate_virtuals = edge_virtuals(candidate)?;
    if edge_direct(candidate)?
        && !candidate_virtuals.is_empty()
        && !name.is_empty()
        && edge_when(candidate)?.is(&empty)
    {
        for edge in out_edges(slf)? {
            let edge_name = spec_name(&edge_child(&edge)?)?;
            let is_partner = partner.as_ref().is_some_and(|p| p.is(&edge));
            if edge_direct(&edge)?
                && !edge_name.is_empty()
                && edge_name != name
                && !is_partner
                && edge_when(&edge)?.is(&empty)
            {
                let edge_vs = edge_virtuals(&edge)?;
                for virtual_name in &candidate_virtuals {
                    if edge_vs.contains(virtual_name) {
                        return Err(registered_error(
                            py,
                            &registry::SPEC_ERROR,
                            "SpecError",
                            &format!(
                                "two direct dependencies cannot both provide \
                                 '{virtual_name}': '{edge_name}' and '{name}'"
                            ),
                        )?);
                    }
                }
            }
        }
    }

    // Merge into the partner before detaching: a failed merge raises with self unchanged.
    let mut changed = false;
    if let Some(partner) = &partner {
        if let Some(error) = crate::algebra::edge_conflict_reason(partner, candidate)? {
            return Err(PyErr::from_value(error.into_bound(py)));
        }
        changed = crate::algebra::edge_merge(partner, candidate)?;
    }

    // Existing edges the candidate implies are detached in its favor.
    let mut implied = Vec::new();
    for edge in out_edges(slf)? {
        if partner.as_ref().is_some_and(|p| p.is(&edge)) {
            continue;
        }
        let edge_name = spec_name(&edge_child(&edge)?)?;
        if !edge_name.is_empty() && edge_name != name && !candidate_virtuals.contains(&edge_name) {
            continue;
        }
        if implies_edge(candidate, &edge)? {
            implied.push(edge);
        }
    }
    for edge in implied {
        detach_edge(slf, &edge)?;
        changed = true;
    }

    if let Some(partner) = partner {
        return Ok((changed, partner));
    }

    if !owned {
        let kwargs = PyDict::new(py);
        kwargs.set_item("deps", true)?;
        let child_copy = edge_child(candidate)?.call_method("copy", (), Some(&kwargs))?;
        candidate.setattr("spec", child_copy)?;
    }
    let child = edge_child(candidate)?;
    add_edge_to_map(
        &slf.getattr("_dependencies")?,
        &spec_name(&child)?,
        candidate,
    )?;
    add_edge_to_map(&child.getattr("_dependents")?, &spec_name(slf)?, candidate)?;
    Ok((true, candidate.clone()))
}

/// `Spec._detach_edge`: remove an edge from this spec and from the dependents of the node
/// it points at. A bucket that runs empty is deleted.
pub(crate) fn detach_edge(slf: &Bound<'_, PyAny>, edge: &Bound<'_, PyAny>) -> PyResult<()> {
    let deps_map = slf.getattr("_dependencies")?;
    let child = edge_child(edge)?;
    let child_name = spec_name(&child)?;
    let bucket = deps_map.get_item(child_name.as_str())?;
    let remaining = siblings_of(&bucket, edge)?;
    if remaining.len() > 0 {
        deps_map.set_item(child_name.as_str(), remaining)?;
    } else {
        deps_map.del_item(child_name.as_str())?;
    }
    let dependents = child.getattr("_dependents")?;
    let self_name = spec_name(slf)?;
    let bucket = dependents.get_item(self_name.as_str())?;
    let remaining = siblings_of(&bucket, edge)?;
    if remaining.len() > 0 {
        dependents.set_item(self_name.as_str(), remaining)?;
    } else {
        dependents.del_item(self_name.as_str())?;
    }
    Ok(())
}

// --------------------------------------------------------------------------------------
// The copy path: `_dup`, `_dup_deps` and `copy`
// --------------------------------------------------------------------------------------

/// The `changed` conjunction of `_dup`, transcribed with the reference short-circuit.
fn dup_changed(slf: &Bound<'_, PyAny>, other: &Bound<'_, PyAny>) -> PyResult<bool> {
    for attr in [
        "name",
        "versions",
        "architecture",
        "variants",
        "concrete",
        "external_path",
        "external_modules",
        "compiler_flags",
        "abstract_hash",
    ] {
        if !slf.getattr(attr)?.ne(&other.getattr(attr)?)? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// `Spec._dup`: copies `other` into self, by overwriting all attributes.
pub(crate) fn spec_dup(
    slf: &Bound<'_, PyAny>,
    other: &Bound<'_, PyAny>,
    deps: &Bound<'_, PyAny>,
    propagation: Option<&Bound<'_, PyAny>>,
) -> PyResult<bool> {
    let py = slf.py();

    // We don't count dependencies as changes here. The probe distinguishes an initialized
    // spec from a bare __new__ instance.
    let mut changed = true;
    if slf.hasattr("annotations")? {
        changed = dup_changed(slf, other)?;
    }

    slf.setattr("_package", py.None())?;
    let other_provided = other.getattr("_provided_virtuals")?;
    if other_provided.is_none() {
        slf.setattr("_provided_virtuals", py.None())?;
    } else {
        let copy = py.get_type::<PyDict>().call1((other_provided,))?;
        slf.setattr("_provided_virtuals", copy)?;
    }

    // Local node attributes get copied first.
    slf.setattr("name", other.getattr("name")?)?;
    slf.setattr("versions", other.getattr("versions")?.call_method0("copy")?)?;
    let other_arch = other.getattr("architecture")?;
    if other_arch.is_truthy()? {
        slf.setattr("architecture", other_arch.call_method0("copy")?)?;
    } else {
        slf.setattr("architecture", py.None())?;
    }
    slf.setattr(
        "compiler_flags",
        other.getattr("compiler_flags")?.call_method0("copy")?,
    )?;
    slf.setattr("variants", other.getattr("variants")?.call_method0("copy")?)?;
    slf.setattr("_build_spec", other.getattr("_build_spec")?)?;

    // Clear dependencies
    slf.setattr("_dependents", PyDict::new(py))?;
    slf.setattr("_dependencies", PyDict::new(py))?;

    // _patches_in_order_of_appearance is managed specially to keep it from leaking out of
    // spec.py.
    let self_variants = slf.getattr("variants")?;
    for item in other
        .getattr("variants")?
        .call_method0("items")?
        .try_iter()?
    {
        let (k, v): (Bound<'_, PyAny>, Bound<'_, PyAny>) = item?.extract()?;
        let patches = getattr_or_none(&v, "_patches_in_order_of_appearance")?;
        if patches.is_truthy()? {
            self_variants
                .get_item(&k)?
                .setattr("_patches_in_order_of_appearance", patches)?;
        }
    }

    slf.setattr("external_path", other.getattr("external_path")?)?;
    slf.setattr("external_modules", other.getattr("external_modules")?)?;
    slf.setattr("extra_attributes", other.getattr("extra_attributes")?)?;
    slf.setattr("namespace", other.getattr("namespace")?)?;
    slf.setattr("annotations", other.getattr("annotations")?)?;

    // If we copy dependencies, preserve DAG structure in the new spec
    if deps.is_truthy()? {
        // If the caller restricted deptypes to be copied, adjust that here.
        let mut depflag = DT_ALL;
        if deps.is_instance_of::<PyTuple>()
            || deps.is_instance_of::<PyList>()
            || deps.is_instance_of::<PyString>()
        {
            depflag = registered(py, &registry::DEPTYPE_CANONICALIZE, "deptypes.canonicalize")?
                .call1((deps,))?
                .extract()?;
        }
        let kwargs = PyDict::new(py);
        kwargs.set_item("propagation", propagation)?;
        slf.call_method("_dup_deps", (other, depflag), Some(&kwargs))?;
    }

    slf.setattr("_prefix", other.getattr("_prefix")?)?;
    slf.setattr("_concrete", other.getattr("_concrete")?)?;

    slf.setattr("abstract_hash", other.getattr("abstract_hash")?)?;

    let hashes = registered(py, &registry::HASH_DESCRIPTORS, "hash descriptors")?;
    if slf.getattr("_concrete")?.is_truthy()? {
        slf.setattr("_dunder_hash", other.getattr("_dunder_hash")?)?;
        for h in hashes.try_iter()? {
            let attr: String = h?.getattr("attr")?.extract()?;
            slf.setattr(attr.as_str(), getattr_or_none(other, &attr)?)?;
        }
    } else {
        slf.setattr("_dunder_hash", py.None())?;
        for h in hashes.try_iter()? {
            let attr: String = h?.getattr("attr")?.extract()?;
            slf.setattr(attr.as_str(), py.None())?;
        }
    }

    Ok(changed)
}

/// `Spec._dup_deps`: copy the edges of `other` verbatim onto fresh node copies.
pub(crate) fn spec_dup_deps(
    slf: &Bound<'_, PyAny>,
    other: &Bound<'_, PyAny>,
    depflag: u64,
    propagation: Option<&Bound<'_, PyAny>>,
) -> PyResult<()> {
    let py = slf.py();
    let mut new_specs: HashMap<usize, Py<PyAny>> = HashMap::new();
    new_specs.insert(ptr(other), slf.clone().unbind());

    let kwargs = PyDict::new(py);
    kwargs.set_item("cover", "edges")?;
    kwargs.set_item("root", false)?;
    let edges = other.call_method("traverse_edges", (), Some(&kwargs))?;
    let cls = registered(py, &registry::DEPENDENCY_SPEC_CLASS, "DependencySpec class")?;
    for edge in edges.try_iter()? {
        let edge = edge?;
        let edge_flag = edge_depflag(&edge)?;
        if edge_flag != 0 && depflag & edge_flag == 0 {
            continue;
        }

        let parent = edge_parent(&edge)?;
        let child = edge_child(&edge)?;
        for node in [&parent, &child] {
            if !new_specs.contains_key(&ptr(node)) {
                let copy_kwargs = PyDict::new(py);
                copy_kwargs.set_item("deps", false)?;
                let copy = node.call_method("copy", (), Some(&copy_kwargs))?;
                new_specs.insert(ptr(node), copy.unbind());
            }
        }

        let edge_propagation: Bound<'_, PyAny> = match propagation {
            None => edge.getattr("propagation")?,
            Some(p) => p.clone(),
        };
        let new_parent = new_specs[&ptr(&parent)].bind(py).clone();
        let new_child = new_specs[&ptr(&child)].bind(py).clone();
        let edge_kwargs = PyDict::new(py);
        edge_kwargs.set_item("depflag", edge_flag)?;
        edge_kwargs.set_item("virtuals", edge.getattr("virtuals")?)?;
        edge_kwargs.set_item("propagation", edge_propagation)?;
        edge_kwargs.set_item("direct", edge_direct(&edge)?)?;
        edge_kwargs.set_item("when", edge_when(&edge)?)?;
        let new_edge = cls.call((&new_parent, &new_child), Some(&edge_kwargs))?;

        // Don't use _add_or_merge_edge here, copy edges verbatim.
        add_edge_to_map(
            &new_parent.getattr("_dependencies")?,
            &spec_name(&new_child)?,
            &new_edge,
        )?;
        add_edge_to_map(
            &new_child.getattr("_dependents")?,
            &spec_name(&new_parent)?,
            &new_edge,
        )?;
    }
    Ok(())
}

/// `Spec.copy`: make a copy of this spec, constructed through the registered Python Spec
/// class exactly like the reference `Spec.__new__(Spec)`.
pub(crate) fn spec_copy<'py>(
    slf: &Bound<'py, PyAny>,
    deps: Option<&Bound<'py, PyAny>>,
    kwargs: Option<&Bound<'py, PyDict>>,
) -> PyResult<Bound<'py, PyAny>> {
    let py = slf.py();
    let cls = registered(py, &registry::SPEC_CLASS, "the Spec class")?;
    let clone = cls.getattr("__new__")?.call1((&cls,))?;
    let dup_kwargs = PyDict::new(py);
    if let Some(kwargs) = kwargs {
        dup_kwargs.update(kwargs.as_mapping())?;
    }
    match deps {
        Some(deps) => dup_kwargs.set_item("deps", deps)?,
        None => dup_kwargs.set_item("deps", true)?,
    }
    clone.call_method("_dup", (slf,), Some(&dup_kwargs))?;
    Ok(clone)
}
