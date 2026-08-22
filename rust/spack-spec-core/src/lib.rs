// SPDX-License-Identifier: (Apache-2.0 OR MIT)

//! Pure-Rust core of the Spack spec model: no Python dependency, natively testable.
//! Node-local algebra (versions, targets, variants, parsing, formatting) lives here;
//! graph-level algebra over Python objects lives in the `spack-spec-py` binding crate.
