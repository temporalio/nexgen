//! The planned JSON Schema subset the C# backend reads, plus the constraint
//! groupings its emitters consume. The loader has already normalized every
//! model onto this shape (merged `allOf`, hoisted inline objects, normalized
//! patterns), so nothing here re-validates the schema.

use std::path::PathBuf;

use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::Value;

use crate::error::{Error, Result};
use crate::json_schema::content_encoding::Encoding;
use crate::json_schema::format::TemporalKind;
use crate::planning::PlannedJsonType;

#[derive(Debug, Deserialize, Default, Clone)]
pub(super) struct Schema {
    #[serde(rename = "$ref")]
    pub(super) reference: Option<String>,
    #[serde(rename = "type")]
    pub(super) ty: Option<Value>,
    pub(super) title: Option<String>,
    pub(super) description: Option<String>,
    pub(super) deprecated: Option<bool>,
    pub(super) properties: Option<IndexMap<String, Schema>>,
    pub(super) required: Option<Vec<String>>,
    #[serde(rename = "additionalProperties")]
    pub(super) additional_properties: Option<Value>,
    pub(super) items: Option<Box<Schema>>,
    #[serde(rename = "oneOf")]
    pub(super) one_of: Option<Vec<Schema>>,
    pub(super) default: Option<Value>,
    #[serde(rename = "const")]
    pub(super) const_value: Option<Value>,
    #[serde(rename = "maxProperties")]
    pub(super) max_properties: Option<u64>,
    #[serde(rename = "minProperties")]
    pub(super) min_properties: Option<u64>,
    #[serde(rename = "propertyNames")]
    pub(super) property_names: Option<Box<Schema>>,
    #[serde(rename = "dependentRequired")]
    pub(super) dependent_required: Option<IndexMap<String, Vec<String>>>,
    pub(super) minimum: Option<serde_json::Number>,
    pub(super) maximum: Option<serde_json::Number>,
    #[serde(rename = "exclusiveMinimum")]
    pub(super) exclusive_minimum: Option<serde_json::Number>,
    #[serde(rename = "exclusiveMaximum")]
    pub(super) exclusive_maximum: Option<serde_json::Number>,
    #[serde(rename = "multipleOf")]
    pub(super) multiple_of: Option<serde_json::Number>,
    #[serde(rename = "minLength")]
    pub(super) min_length: Option<u64>,
    #[serde(rename = "maxLength")]
    pub(super) max_length: Option<u64>,
    pub(super) pattern: Option<String>,
    pub(super) format: Option<String>,
    #[serde(rename = "contentEncoding")]
    pub(super) content_encoding: Option<String>,
    #[serde(rename = "minItems")]
    pub(super) min_items: Option<u64>,
    #[serde(rename = "maxItems")]
    pub(super) max_items: Option<u64>,
    #[serde(rename = "uniqueItems")]
    pub(super) unique_items: Option<bool>,
    pub(super) contains: Option<Box<Schema>>,
    #[serde(rename = "minContains")]
    pub(super) min_contains: Option<u64>,
    #[serde(rename = "maxContains")]
    pub(super) max_contains: Option<u64>,
    #[serde(rename = "enum")]
    pub(super) enum_values: Option<Vec<Value>>,
    #[serde(rename = "x-cs-name")]
    pub(super) x_cs_name: Option<String>,
    #[serde(rename = "x-cs-const-name")]
    pub(super) x_cs_const_name: Option<String>,
    #[serde(rename = "x-cs-enum-names")]
    pub(super) x_cs_enum_names: Option<IndexMap<String, String>>,
}

impl Schema {
    pub(super) fn type_name(&self) -> Option<&str> {
        self.ty.as_ref().and_then(Value::as_str)
    }

    /// The closed value set a `const` (one value) or `enum` (many) declares.
    pub(super) fn closed_values(&self) -> Vec<Value> {
        if let Some(value) = &self.const_value {
            vec![value.clone()]
        } else if let Some(values) = &self.enum_values {
            values.clone()
        } else {
            Vec::new()
        }
    }
}

pub(super) fn decode_schema(model: &PlannedJsonType) -> Result<Schema> {
    decode_value(&model.schema, &model.full_name)
}

pub(super) fn decode_value(value: &Value, what: &str) -> Result<Schema> {
    serde_json::from_value(value.clone()).map_err(|error| Error::InvalidJsonSchema {
        path: PathBuf::from("<json-generator>"),
        reason: format!("failed to read planned JSON schema `{what}`: {error}"),
    })
}

#[derive(Debug, Clone, Default)]
pub(super) struct ArrayConstraints {
    pub(super) min_items: Option<u64>,
    pub(super) max_items: Option<u64>,
    pub(super) unique_items: bool,
    pub(super) contains: Option<Box<Schema>>,
    pub(super) min_contains: Option<u64>,
    pub(super) max_contains: Option<u64>,
}

impl ArrayConstraints {
    pub(super) fn from_schema(schema: &Schema) -> Self {
        Self {
            min_items: schema.min_items,
            max_items: schema.max_items,
            unique_items: schema.unique_items == Some(true),
            contains: schema.contains.clone(),
            min_contains: schema.min_contains,
            max_contains: schema.max_contains,
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.min_items.is_none()
            && self.max_items.is_none()
            && !self.unique_items
            && self.contains.is_none()
    }
}

/// A pinned string `format` lowered for C#: the canonical name (for the
/// reason), the pinned regex in the .NET dialect, and the optional
/// total-length guard. See `specs/json-schema/features/format.md`.
#[derive(Debug, Clone)]
pub(super) struct CsFormat {
    pub(super) name: &'static str,
    pub(super) pattern: String,
    pub(super) max_code_points: Option<usize>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct StringConstraints {
    pub(super) min_length: Option<u64>,
    pub(super) max_length: Option<u64>,
    /// The loader-normalized `pattern` rewritten into the .NET dialect.
    pub(super) pattern: Option<String>,
    pub(super) format: Option<CsFormat>,
}

impl StringConstraints {
    pub(super) fn from_schema(schema: &Schema) -> Self {
        Self {
            min_length: schema.min_length,
            max_length: schema.max_length,
            pattern: schema.pattern.as_deref().map(dotnet_pattern),
            format: schema
                .format
                .as_deref()
                .and_then(crate::json_schema::format::check_for)
                .map(|check| CsFormat {
                    name: check.name,
                    pattern: dotnet_pattern(&check.pattern),
                    max_code_points: check.max_code_points,
                }),
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.min_length.is_none()
            && self.max_length.is_none()
            && self.pattern.is_none()
            && self.format.is_none()
    }

    pub(super) fn has_length(&self) -> bool {
        self.min_length.is_some() || self.max_length.is_some()
    }

    pub(super) fn without_length(&self) -> Self {
        Self {
            min_length: None,
            max_length: None,
            ..self.clone()
        }
    }
}

/// Rewrites a loader-normalized (portable) pattern into the dialect the
/// generated `Regex` runs: the end anchor is `\z` (.NET's `$` also matches
/// before a final newline), and UTF-16 code units are grouped so astral code
/// points match as one character. The `Regex` itself runs under
/// `RegexOptions.ECMAScript`, which pins `\d`/`\w` to ASCII.
pub(super) fn dotnet_pattern(pattern: &str) -> String {
    let anchored = crate::json_schema::pattern::rewrite_end_anchor(pattern, r"\z");
    crate::json_schema::pattern::rewrite_for_utf16_code_units(&anchored)
}

#[derive(Debug, Clone, Default)]
pub(super) struct NumericConstraints {
    pub(super) minimum: Option<serde_json::Number>,
    pub(super) maximum: Option<serde_json::Number>,
    pub(super) exclusive_minimum: Option<serde_json::Number>,
    pub(super) exclusive_maximum: Option<serde_json::Number>,
    pub(super) multiple_of: Option<serde_json::Number>,
}

impl NumericConstraints {
    pub(super) fn from_schema(schema: &Schema) -> Self {
        Self {
            minimum: schema.minimum.clone(),
            maximum: schema.maximum.clone(),
            exclusive_minimum: schema.exclusive_minimum.clone(),
            exclusive_maximum: schema.exclusive_maximum.clone(),
            multiple_of: schema.multiple_of.clone(),
        }
    }
}

fn schema_type_includes(schema: &Schema, ty: &str) -> bool {
    match schema.ty.as_ref() {
        Some(Value::String(value)) => value == ty,
        Some(Value::Array(values)) => values
            .iter()
            .any(|value| value.as_str().is_some_and(|value| value == ty)),
        _ => false,
    }
}

pub(super) fn allows_null(schema: &Schema) -> bool {
    schema.const_value.as_ref() == Some(&Value::Null)
        || schema_type_includes(schema, "null")
        || schema
            .one_of
            .as_ref()
            .is_some_and(|branches| branches.iter().any(allows_null))
}

/// The non-null branch of the nullability `oneOf: [T, null]` wrapper.
pub(super) fn nullable_non_null_schema(schema: &Schema) -> Option<&Schema> {
    schema.one_of.as_ref()?.iter().find(|branch| {
        !schema_type_includes(branch, "null") && branch.const_value.as_ref() != Some(&Value::Null)
    })
}

/// The member shape of a possibly-nullable position: the non-null branch of a
/// nullability wrapper, or the schema itself.
pub(super) fn shape_of(schema: &Schema) -> &Schema {
    if is_union_schema(schema) {
        return schema;
    }
    nullable_non_null_schema(schema).unwrap_or(schema)
}

/// An array's element shape, with the nullability wrapper stripped. The
/// element's *nullability* is read from the wrapper separately.
pub(super) fn element_shape(schema: &Schema) -> Option<&Schema> {
    let items = schema.items.as_deref()?;
    Some(shape_of(items))
}

/// True when a schema is a `oneOf` sum type (two or more non-null branches).
pub(super) fn is_union_schema(schema: &Schema) -> bool {
    schema.one_of.as_ref().is_some_and(|branches| {
        branches
            .iter()
            .filter(|branch| branch.type_name() != Some("null"))
            .count()
            >= 2
    })
}

/// A typed-map model: no declared properties and a schema-valued
/// `additionalProperties`.
pub(super) fn typed_map_value(schema: &Schema) -> Option<&Value> {
    if schema
        .properties
        .as_ref()
        .is_some_and(|properties| !properties.is_empty())
    {
        return None;
    }
    match &schema.additional_properties {
        Some(value @ Value::Object(_)) => Some(value),
        _ => None,
    }
}

/// The authored member schema of a map-shaped position, if it declares one.
pub(super) fn additional_properties_schema(schema: &Schema) -> Result<Option<Schema>> {
    match &schema.additional_properties {
        Some(value @ Value::Object(_)) => Ok(Some(decode_value(value, "additionalProperties")?)),
        _ => Ok(None),
    }
}

pub(super) fn temporal_kind_direct(schema: &Schema) -> Option<TemporalKind> {
    if schema.type_name() != Some("string") {
        return None;
    }
    schema.format.as_deref().and_then(TemporalKind::from_name)
}

pub(super) fn content_encoding_direct(schema: &Schema) -> Option<Encoding> {
    if schema.type_name() != Some("string") {
        return None;
    }
    schema
        .content_encoding
        .as_deref()
        .and_then(Encoding::from_name)
}

/// Recursively walks schema-bearing positions within one planned model.
pub(super) fn schema_uses_feature(schema: &Schema, direct: fn(&Schema) -> bool) -> bool {
    if direct(schema) {
        return true;
    }
    if schema.properties.as_ref().is_some_and(|properties| {
        properties
            .values()
            .any(|value| schema_uses_feature(value, direct))
    }) {
        return true;
    }
    if schema
        .items
        .as_deref()
        .is_some_and(|items| schema_uses_feature(items, direct))
        || schema.one_of.as_ref().is_some_and(|branches| {
            branches
                .iter()
                .any(|branch| schema_uses_feature(branch, direct))
        })
        || schema
            .contains
            .as_deref()
            .is_some_and(|contains| schema_uses_feature(contains, direct))
        || schema
            .property_names
            .as_deref()
            .is_some_and(|names| schema_uses_feature(names, direct))
    {
        return true;
    }
    let Some(Value::Object(member)) = &schema.additional_properties else {
        return false;
    };
    serde_json::from_value::<Schema>(Value::Object(member.clone()))
        .is_ok_and(|member| schema_uses_feature(&member, direct))
}

/// Composes doc text from a `title` (summary) and a `description` (body).
pub(super) fn compose_doc(title: Option<&str>, description: Option<&str>) -> Option<String> {
    let parts: Vec<&str> = [title, description]
        .into_iter()
        .flatten()
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect();
    (!parts.is_empty()).then(|| parts.join("\n\n"))
}
