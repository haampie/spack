// SPDX-License-Identifier: (Apache-2.0 OR MIT)

//! Port of `spack.util.lang.lazy_eq` / `lazy_lt`: lexicographic comparison of two
//! lazily generated sequences, each given as a zero-arg callable returning an
//! iterable whose items are values or further such callables.

use pyo3::prelude::*;

pub fn lazy_eq(lseq: &Bound<'_, PyAny>, rseq: &Bound<'_, PyAny>) -> PyResult<bool> {
    let mut liter = lseq.call0()?.try_iter()?;
    let mut riter = rseq.call0()?.try_iter()?;
    loop {
        match (liter.next(), riter.next()) {
            (None, None) => return Ok(true),
            (None, Some(_)) | (Some(_), None) => return Ok(false),
            (Some(left), Some(right)) => {
                let left = left?;
                let right = right?;
                let equal = if left.is_callable() {
                    lazy_eq(&left, &right)?
                } else {
                    left.eq(&right)?
                };
                if !equal {
                    return Ok(false);
                }
            }
        }
    }
}

pub fn lazy_lt(lseq: &Bound<'_, PyAny>, rseq: &Bound<'_, PyAny>) -> PyResult<bool> {
    let mut liter = lseq.call0()?.try_iter()?;
    let mut riter = rseq.call0()?.try_iter()?;
    loop {
        match (liter.next(), riter.next()) {
            (None, None) => return Ok(false),
            (None, Some(_)) => return Ok(true), // left was shorter than right
            (Some(_), None) => return Ok(false),
            (Some(left), Some(right)) => {
                let left = left?;
                let right = right?;
                let sequence = left.is_callable();
                let equal = if sequence {
                    lazy_eq(&left, &right)?
                } else {
                    left.eq(&right)?
                };
                if equal {
                    continue;
                }
                if sequence {
                    return lazy_lt(&left, &right);
                }
                // Python-2 style None ordering: None is less than everything else.
                if left.is_none() {
                    return Ok(true);
                }
                if right.is_none() {
                    return Ok(false);
                }
                return left.lt(&right);
            }
        }
    }
}
