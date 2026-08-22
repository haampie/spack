// SPDX-License-Identifier: (Apache-2.0 OR MIT)

//! Port of the algebra of `spack.variant.VariantValue`: a named set of values with a
//! type tag that can narrow from multi to single to bool, an abstract/concrete bit
//! (abstract multi = "at least these values"), and a propagation bit.
//!
//! Python -> Rust mapping:
//! - `VariantType` -> [`VariantKind`] (same integer values)
//! - `VariantValue` -> [`VariantData`] (pure data; the pyclass wraps this)
//! - `VariantValue.set` -> [`VariantData::set`]
//! - `satisfies` / `intersects` / `_merge` / `_merged_values` / `_contains` -> same names
//!   (with `merge` / `merged_values` / `contains_value`)
//! - `__str__` -> [`VariantData::render`] (the `quote_if_needed` part of spec_parser)

use std::cmp::Ordering;
use std::fmt::Write as _;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VariantKind {
    Bool = 1,
    Single = 2,
    Multi = 3,
    /// Placeholder used by `VariantValueRemoval`.
    Indicator = 4,
}

impl VariantKind {
    pub fn from_int(value: i64) -> Option<VariantKind> {
        match value {
            1 => Some(VariantKind::Bool),
            2 => Some(VariantKind::Single),
            3 => Some(VariantKind::Multi),
            4 => Some(VariantKind::Indicator),
            _ => None,
        }
    }

    /// `VariantType.string`
    pub fn as_str(self) -> &'static str {
        match self {
            VariantKind::Bool => "bool",
            VariantKind::Single => "single",
            VariantKind::Multi => "multi",
            VariantKind::Indicator => "indicator",
        }
    }
}

/// A single variant value: Python `bool | str`, plus `None` used by the removal indicator.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Value {
    Bool(bool),
    Str(String),
    None,
}

impl Value {
    /// Python `str(v)`; used by `_cmp_iter` and rendering.
    pub fn render(&self) -> String {
        match self {
            Value::Bool(true) => "True".to_string(),
            Value::Bool(false) => "False".to_string(),
            Value::Str(s) => s.clone(),
            Value::None => "None".to_string(),
        }
    }

    fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VariantError {
    /// `MultipleValuesInExclusiveVariantError(self)`
    MultipleValues,
    /// `ValueError` for a non-boolean value in a bool variant; payload is the rendered value.
    NonBooleanValue(String),
    /// `InvalidVariantValueError("cannot use reserved value '*'")`
    ReservedStar,
    /// Python `sorted(set(...))` on values of mixed types raises `TypeError`.
    UnorderableValues,
    /// `UnsatisfiableVariantSpecError(self, other)`; the binding reconstructs the args.
    Unsatisfiable,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VariantData {
    pub name: String,
    pub kind: VariantKind,
    pub propagate: bool,
    pub concrete: bool,
    values: Vec<Value>,
}

impl VariantData {
    /// `VariantValue.__init__`: only multi-valued variants can be abstract, and the
    /// value tuple goes through `set`.
    pub fn new(
        kind: VariantKind,
        name: &str,
        values: Vec<Value>,
        propagate: bool,
        concrete: bool,
    ) -> Result<VariantData, VariantError> {
        let mut data = VariantData {
            name: name.to_string(),
            kind,
            propagate,
            concrete: concrete || matches!(kind, VariantKind::Bool | VariantKind::Single),
            values: Vec::new(),
        };
        data.set(values)?;
        Ok(data)
    }

    pub fn values(&self) -> &[Value] {
        &self.values
    }

    /// `VariantValue.set`: `tuple(sorted(set(value)))` when there is more than one value
    /// (dedupe first, then sort; sorting mixed types raises in Python), then enforce
    /// single/bool arity and the reserved `*`.
    pub fn set(&mut self, values: Vec<Value>) -> Result<(), VariantError> {
        let mut values = values;
        if values.len() > 1 {
            let mut unique: Vec<Value> = Vec::with_capacity(values.len());
            for v in values {
                if !unique.contains(&v) {
                    unique.push(v);
                }
            }
            values = unique;
        }
        if values.len() > 1 {
            if values.iter().all(|v| matches!(v, Value::Str(_))) {
                values.sort_by(cmp_str_values);
            } else if values.iter().all(|v| matches!(v, Value::Bool(_))) {
                values.sort_by_key(|v| matches!(v, Value::Bool(true)));
            } else {
                return Err(VariantError::UnorderableValues);
            }
        }

        if self.kind != VariantKind::Multi {
            if values.len() != 1 {
                return Err(VariantError::MultipleValues);
            }
            if self.kind == VariantKind::Bool && !matches!(values[0], Value::Bool(_)) {
                return Err(VariantError::NonBooleanValue(values[0].render()));
            }
        }

        if values
            .iter()
            .any(|v| matches!(v, Value::Str(s) if s == "*"))
        {
            return Err(VariantError::ReservedStar);
        }

        self.values = values;
        Ok(())
    }

    /// `VariantValue._contains`: for `patches`, a value is covered by any stored value
    /// that starts with it (checksum prefixes).
    pub fn contains_value(&self, value: &Value) -> bool {
        if self.name == "patches" {
            if let Some(prefix) = value.as_str() {
                return self
                    .values
                    .iter()
                    .any(|w| matches!(w, Value::Str(s) if s.starts_with(prefix)));
            }
        }
        self.values.contains(value)
    }

    /// `VariantValue._merged_values`: union of both sides; for `patches`, drop values
    /// that are a proper prefix of another.
    pub fn merged_values(&self, other: &VariantData) -> Vec<Value> {
        let values: Vec<Value> = self.values.iter().chain(&other.values).cloned().collect();
        if self.name != "patches" {
            return values;
        }
        values
            .iter()
            .filter(|v| {
                !values.iter().any(|w| {
                    w != *v
                        && matches!((w.as_str(), v.as_str()), (Some(w), Some(v)) if w.starts_with(v))
                })
            })
            .cloned()
            .collect()
    }

    /// `VariantValue.satisfies`
    pub fn satisfies(&self, other: &VariantData) -> bool {
        if self.name != other.name {
            return false;
        }
        if !other.concrete {
            return other.values.iter().all(|v| self.contains_value(v));
        }
        if self.concrete {
            return self.values == other.values;
        }
        false
    }

    /// `VariantValue.intersects`
    pub fn intersects(&self, other: &VariantData) -> bool {
        if self.name != other.name {
            return false;
        }
        if self.concrete {
            if other.concrete {
                return self.values == other.values;
            }
            return other.values.iter().all(|v| self.contains_value(v));
        }
        if other.concrete {
            return self.values.iter().all(|v| other.contains_value(v));
        }
        // both abstract: their union is a concretization of both
        true
    }

    /// `VariantValue.constrain`
    pub fn constrain(&mut self, other: &VariantData) -> Result<bool, VariantError> {
        if !self.intersects(other) {
            return Err(VariantError::Unsatisfiable);
        }
        self.merge(other)
    }

    /// `VariantValue._merge`: never raises in Python; the value-set errors cannot occur
    /// for intersecting variants, but `set` is still fallible in Rust so thread it.
    pub fn merge(&mut self, other: &VariantData) -> Result<bool, VariantError> {
        let old_values = self.values.clone();
        let merged = self.merged_values(other);
        self.set(merged)?;
        let mut changed = old_values != self.values;
        if self.propagate && !other.propagate {
            self.propagate = false;
            changed = true;
        }
        if !self.concrete && other.concrete {
            self.concrete = true;
            changed = true;
        }
        if self.kind > other.kind {
            self.kind = other.kind;
            changed = true;
        }
        Ok(changed)
    }

    /// `VariantValue.append`
    pub fn append(&mut self, value: Value) -> Result<(), VariantError> {
        let mut values = self.values.clone();
        values.push(value);
        self.set(values)
    }

    /// `VariantValue._cmp_iter` scalar stream: name, propagate, concrete, *str(values).
    pub fn cmp_key(&self) -> (&str, bool, bool, Vec<String>) {
        (
            &self.name,
            self.propagate,
            self.concrete,
            self.values.iter().map(Value::render).collect(),
        )
    }

    /// `VariantValue.__str__`
    pub fn render(&self) -> String {
        if self.kind == VariantKind::Bool {
            let sigil = if matches!(self.values.first(), Some(Value::Bool(true))) {
                "+"
            } else {
                "~"
            };
            let sigil = if self.propagate {
                [sigil, sigil].concat()
            } else {
                sigil.to_string()
            };
            return format!("{}{}", sigil, self.name);
        }

        let concrete = if self.kind == VariantKind::Multi && self.concrete {
            ":"
        } else {
            ""
        };
        let delim = if self.propagate { "==" } else { "=" };
        let value_str = if self.values.is_empty() {
            "*".to_string()
        } else {
            let mut s = String::new();
            for (i, v) in self.values.iter().enumerate() {
                if i > 0 {
                    s.push(',');
                }
                let _ = write!(s, "{}", v.render());
            }
            s
        };
        format!(
            "{}{}{}{}",
            self.name,
            concrete,
            delim,
            crate::strutil::quote_if_needed(&value_str)
        )
    }
}

/// Python string ordering for the `sorted(set(...))` in `set`; values are known to be
/// strings when this is called.
fn cmp_str_values(a: &Value, b: &Value) -> Ordering {
    a.as_str().unwrap_or("").cmp(b.as_str().unwrap_or(""))
}
