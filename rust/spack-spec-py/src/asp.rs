// SPDX-License-Identifier: (Apache-2.0 OR MIT)

//! PyO3 binding for the ASP fact-string building in `spack-spec-core`, mirroring
//! `spack.solver.core.AspVar` / `AspFunction` and `spack.solver.asp.ProblemInstanceBuilder`.
//!
//! Rendering an atom never crosses back into Python and copies each argument's text exactly
//! once: `fact()` walks the argument tuple and writes it straight into the `AspWriter` the
//! builder reuses, then pushes the line into a Rust buffer. So the ~10^5 lines of a problem
//! instance exist as Python strings only if somebody asks for `asp_problem`; the common path
//! asks for [`ProblemInstanceBuilder::stripped_sorted_str`], which strips, sorts and joins
//! Rust-side and hands Python one string.
//!
//! `AspFunction` keeps `args` as an opaque Python object rather than a tuple: the Python class
//! does no validation either, and `solver/asp.py` both reassigns `args` and relies on
//! `self.args + args` failing the way tuple concatenation fails. Values are acyclic by
//! construction (a fact tree is built bottom-up out of strings, ints, `AspVar`s and other
//! `AspFunction`s), so the classes do not implement the GC protocol.

use pyo3::basic::CompareOp;
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyInt, PyList, PyString, PyTuple};

use spack_spec_core::asp::{AspWriter, ProblemBuffer};

/// Write one argument, following `AspFunction.__str__`'s type ladder in the order
/// `spack.solver.core` tests it: the exact types first, then the subclass fallbacks (config
/// values are `syaml_str` / `syaml_int`), then anything else as its unescaped `str()` in quotes
/// (which is where `bool` lands).
fn write_arg(w: &mut AspWriter, arg: &Bound<'_, PyAny>) -> PyResult<()> {
    if arg.is_exact_instance_of::<PyString>() {
        w.str_arg(&arg.downcast::<PyString>()?.to_cow()?);
        return Ok(());
    }
    if arg.is_exact_instance_of::<AspFunction>() {
        return write_function(w, arg.downcast::<AspFunction>()?);
    }
    if arg.is_exact_instance_of::<PyInt>() {
        return write_int(w, arg);
    }
    if arg.is_exact_instance_of::<AspVar>() {
        w.bare_arg(&arg.downcast::<AspVar>()?.borrow().name);
        return Ok(());
    }
    // A str/int subclass misses the checks above. bool is an int subclass, but is not a number
    // in ASP, so it falls through to the quoted branch.
    if arg.is_instance_of::<PyInt>() && !arg.is_instance_of::<PyBool>() {
        return write_int(w, arg);
    }
    if arg.is_instance_of::<PyString>() {
        w.str_arg(&arg.downcast::<PyString>()?.to_cow()?);
        return Ok(());
    }
    w.raw_arg(&arg.str()?.to_cow()?);
    Ok(())
}

/// A Python int as a bare ASP term. A value outside `i64` keeps Python's `str()` digits, which
/// render bare like any other number.
fn write_int(w: &mut AspWriter, arg: &Bound<'_, PyAny>) -> PyResult<()> {
    match arg.extract::<i64>() {
        Ok(value) => w.int_arg(value),
        Err(_) => w.bare_arg(&arg.str()?.to_cow()?),
    }
    Ok(())
}

/// One `AspFunction`, arguments and all.
fn write_function(w: &mut AspWriter, func: &Bound<'_, AspFunction>) -> PyResult<()> {
    let this = func.borrow();
    w.open(&this.name);
    let args = this.args.bind(func.py());
    if let Ok(tuple) = args.downcast::<PyTuple>() {
        for arg in tuple.iter() {
            write_arg(w, &arg)?;
        }
    } else {
        // The Python class does not require a tuple either; anything iterable renders.
        for arg in args.try_iter()? {
            write_arg(w, &arg?)?;
        }
    }
    w.close();
    Ok(())
}

/// One term as a string, for `__str__`.
fn render(func: &Bound<'_, AspFunction>) -> PyResult<String> {
    let mut w = AspWriter::new();
    write_function(&mut w, func)?;
    Ok(w.as_str().to_string())
}

/// A variable in an ASP rule, rendered as its bare name.
#[pyclass(name = "AspVar", module = "spack_spec")]
pub struct AspVar {
    #[pyo3(get, set)]
    pub name: String,
}

#[pymethods]
impl AspVar {
    #[new]
    fn new(name: String) -> Self {
        AspVar { name }
    }

    fn __str__(&self) -> String {
        self.name.clone()
    }
}

/// A term in the ASP logic program.
#[pyclass(name = "AspFunction", module = "spack_spec")]
pub struct AspFunction {
    #[pyo3(get, set)]
    pub name: String,
    #[pyo3(get, set)]
    pub args: Py<PyAny>,
}

#[pymethods]
impl AspFunction {
    #[new]
    #[pyo3(signature = (name, args=None))]
    fn new(py: Python<'_>, name: String, args: Option<Py<PyAny>>) -> PyResult<Self> {
        let args = match args {
            Some(args) => args,
            None => PyTuple::empty(py).into_any().unbind(),
        };
        Ok(AspFunction { name, args })
    }

    /// `AspFunction(self.name, args if not self.args else self.args + args)`: calls are
    /// additive, so `attr("version")("foo")` is `attr("version", "foo")`.
    #[pyo3(signature = (*args))]
    fn __call__(&self, py: Python<'_>, args: Py<PyTuple>) -> PyResult<AspFunction> {
        let args = if self.args.bind(py).is_truthy()? {
            self.args.bind(py).add(args)?.unbind()
        } else {
            args.into_any()
        };
        Ok(AspFunction {
            name: self.name.clone(),
            args,
        })
    }

    fn _cmp_key<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        PyTuple::new(
            py,
            [
                PyString::new(py, &self.name).into_any(),
                self.args.bind(py).clone(),
            ],
        )
    }

    /// The comparisons `spack.util.lang.key_ordering` installs on the Python class, including
    /// its treatment of `None` (never equal, always smaller).
    fn __richcmp__(
        slf: &Bound<'_, Self>,
        other: &Bound<'_, PyAny>,
        op: CompareOp,
    ) -> PyResult<bool> {
        let py = slf.py();
        if other.is_none() {
            return Ok(match op {
                CompareOp::Eq | CompareOp::Lt | CompareOp::Le => false,
                CompareOp::Ne | CompareOp::Gt | CompareOp::Ge => true,
            });
        }
        if matches!(op, CompareOp::Eq) && slf.is(other) {
            return Ok(true);
        }
        if matches!(op, CompareOp::Ne) && slf.is(other) {
            return Ok(false);
        }
        let mine = slf.borrow()._cmp_key(py)?;
        let theirs = other.call_method0("_cmp_key")?;
        mine.rich_compare(theirs, op)?.is_truthy()
    }

    fn __hash__(&self, py: Python<'_>) -> PyResult<isize> {
        self._cmp_key(py)?.hash()
    }

    fn __str__(slf: &Bound<'_, Self>) -> PyResult<String> {
        render(slf)
    }

    fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
        render(slf)
    }

    fn __reduce__<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, PyTuple>> {
        let py = slf.py();
        let this = slf.borrow();
        let args = PyTuple::new(
            py,
            [
                PyString::new(py, &this.name).into_any(),
                this.args.bind(py).clone(),
            ],
        )?;
        PyTuple::new(py, [slf.get_type().into_any(), args.into_any()])
    }
}

/// The lines of a problem instance, built without materializing them as Python strings.
#[pyclass(name = "ProblemInstanceBuilder", module = "spack_spec")]
pub struct ProblemInstanceBuilder {
    buffer: ProblemBuffer,
    /// Reused across facts so rendering an instance allocates one buffer, not one per atom.
    writer: AspWriter,
}

#[pymethods]
impl ProblemInstanceBuilder {
    #[new]
    fn new() -> Self {
        ProblemInstanceBuilder {
            buffer: ProblemBuffer::new(),
            writer: AspWriter::new(),
        }
    }

    /// The atom rendered into the writer this builder reuses, then pushed as a line.
    fn fact(&mut self, atom: &Bound<'_, AspFunction>) -> PyResult<()> {
        self.writer.clear();
        write_function(&mut self.writer, atom)?;
        self.buffer.fact(self.writer.as_str());
        Ok(())
    }

    fn append(&mut self, rule: String) {
        self.buffer.append(rule);
    }

    fn title(&mut self, header: &str, fill: &str) {
        self.buffer.title(header, fill);
    }

    fn h1(&mut self, header: &str) {
        self.title(header, "=");
    }

    fn h2(&mut self, header: &str) {
        self.title(header, "-");
    }

    fn h3(&mut self, header: &str) {
        self.buffer.append(format!("% {header}"));
    }

    fn newline(&mut self) {
        self.buffer.newline();
    }

    /// `_strip_asp_problem(problem)`, `problem.sort()` and `"\n".join(problem)` in one step,
    /// which is what the solver does with the instance unless it is dumping or shuffling it.
    fn stripped_sorted_str(&self) -> String {
        self.buffer.stripped_sorted_joined()
    }

    /// The lines as a Python list, for the paths that print or shuffle the problem.
    #[getter]
    fn asp_problem<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyList>> {
        PyList::new(py, &self.buffer.lines)
    }
}
