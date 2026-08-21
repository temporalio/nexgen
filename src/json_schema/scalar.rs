//! Language-neutral scalar matcher descriptors.
//!
//! JSON Schema uses the same scalar assertion vocabulary in several places:
//! ordinary values (including `const`, `enum`, and `default` validation), the
//! `contains` applicator, and `propertyNames`. The loader proves that a matcher
//! is in the supported subset; this descriptor is the normalized hand-off used
//! by target backends so they do not independently decide which assertions a
//! matcher contains.

use serde_json::{Number, Value};

/// A JSON scalar kind accepted by the generated model surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScalarKind {
    String,
    Number,
    Integer,
    Boolean,
}

impl ScalarKind {
    pub(crate) fn from_name(name: &str) -> Option<Self> {
        match name {
            "string" => Some(Self::String),
            "number" => Some(Self::Number),
            "integer" => Some(Self::Integer),
            "boolean" => Some(Self::Boolean),
            _ => None,
        }
    }
}

/// The supported, normalized scalar assertions on one schema node.
///
/// Fields intentionally retain JSON numbers rather than converting to `f64`:
/// each backend must render the authored decimal without losing precision, and
/// integer-valued number literals are normalized by the loader before this
/// descriptor is constructed.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct ScalarMatcher {
    pub(crate) kind: Option<ScalarKind>,
    pub(crate) const_value: Option<Value>,
    pub(crate) enum_values: Vec<Value>,
    pub(crate) minimum: Option<Number>,
    pub(crate) maximum: Option<Number>,
    pub(crate) exclusive_minimum: Option<Number>,
    pub(crate) exclusive_maximum: Option<Number>,
    pub(crate) multiple_of: Option<Number>,
    pub(crate) min_length: Option<u64>,
    pub(crate) max_length: Option<u64>,
    pub(crate) pattern: Option<String>,
    pub(crate) format: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalar_kind_classifies_only_supported_scalar_types() {
        assert_eq!(ScalarKind::from_name("string"), Some(ScalarKind::String));
        assert_eq!(ScalarKind::from_name("integer"), Some(ScalarKind::Integer));
        assert_eq!(ScalarKind::from_name("object"), None);
        assert_eq!(ScalarKind::from_name("null"), None);
    }
}
