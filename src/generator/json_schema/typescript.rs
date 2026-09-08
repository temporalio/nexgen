use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use heck::{ToShoutySnakeCase, ToUpperCamelCase};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use std::cell::{Cell, RefCell};

use crate::error::{Error, Result};
use crate::generator::json_schema::build_json_name_manifest;
use crate::generator::json_schema::{
    bare_ref_target, register_cross_module_ref_names, violation_member_segment,
};
use crate::generator::typescript::{
    RenderedExternalModelFragments, RenderedTypeScriptSupport, WireValueConversion,
    render_typescript_doc_comment, typescript_generated_field_name,
};
use crate::generator::{ExternalModelBackend, TsDateTimeTypes};
use crate::json_schema::format::TemporalKind;
use crate::json_schema::scalar::{ScalarKind, ScalarMatcher};
use crate::language::Language;
use crate::parser::NameManifest;
use crate::planning::{PlannedFamily, PlannedJsonType, PlannedSpec};
use crate::spec::{ModulePath, RecordSpec};

/// The converter identifier is owned by the parser's per-language naming policy,
/// which also enters it into the P15 collision namespace. Re-exported so the
/// shared TypeScript emitter reaches the name through this backend — the JSON
/// tier that emits the converters — and never spells the derivation itself.
pub(in crate::generator) use crate::parser::ts_transfer_type_converter_name;

thread_local! {
    /// The active `--date-time-types` while rendering the TS models/runtime.
    /// Generation is single-threaded per file, so a thread-local avoids threading
    /// the flag through every recursive `type_annotation`/parser/serializer call.
    static TS_DATE_TIME_TYPES: Cell<TsDateTimeTypes> = const { Cell::new(TsDateTimeTypes::String) };
    static USES_TEMPORAL: Cell<bool> = const { Cell::new(false) };
    static USES_CONTENT_ENCODING: Cell<bool> = const { Cell::new(false) };
    static USES_CODE_POINT_LENGTH: Cell<bool> = const { Cell::new(false) };
    /// Every JSON model in the input closure, keyed by `full_name`, so a `$ref`
    /// branch that crosses a module boundary still resolves to its target's
    /// shape. Registered once per generation from the whole spec tree; the
    /// per-module model list alone cannot see a sibling leaf's `$defs`.
    static TREE_JSON_MODELS: RefCell<BTreeMap<String, PlannedJsonType>> =
        const { RefCell::new(BTreeMap::new()) };
    /// Resolved type identifiers keyed by both `full_name` and the `#/$defs/<full_name>`
    /// `$ref` form, so `reference_model_name` follows the same name manifest as the
    /// declaration (honoring `x-ts-name` overrides) instead of recasing the ref segment.
    static REF_NAMES: RefCell<BTreeMap<String, String>> = const { RefCell::new(BTreeMap::new()) };
}

/// Registers the whole input closure's JSON models for cross-module `$ref`
/// resolution. Called once per generation, before any leaf renders.
pub(in crate::generator) fn set_tree_json_models(models: BTreeMap<String, PlannedJsonType>) {
    TREE_JSON_MODELS.with(|cell| *cell.borrow_mut() = models);
}

fn set_ref_names(ref_names: &BTreeMap<String, String>) {
    REF_NAMES.with(|cell| cell.borrow_mut().clone_from(ref_names));
}

fn set_temporal_context(repr: TsDateTimeTypes, uses_temporal: bool) {
    TS_DATE_TIME_TYPES.with(|cell| cell.set(repr));
    USES_TEMPORAL.with(|cell| cell.set(uses_temporal));
}

fn set_content_encoding_context(uses_content_encoding: bool) {
    USES_CONTENT_ENCODING.with(|cell| cell.set(uses_content_encoding));
}

fn set_code_point_length_context(uses_code_point_length: bool) {
    USES_CODE_POINT_LENGTH.with(|cell| cell.set(uses_code_point_length));
}

/// The qualified call to the shared surrogate-aware code-point scan in the
/// generated runtime module.
fn code_point_length_fn() -> String {
    format!("{DEFINITIONS_NAMESPACE}.codePointLength")
}

fn active_repr() -> TsDateTimeTypes {
    TS_DATE_TIME_TYPES.with(Cell::get)
}

fn value_schema_type_includes(schema: &Value, ty: &str) -> bool {
    match schema.get("type") {
        Some(Value::String(value)) => value == ty,
        Some(Value::Array(values)) => values.iter().any(|value| value.as_str() == Some(ty)),
        _ => false,
    }
}

fn value_temporal_kind_direct(schema: &Value) -> Option<TemporalKind> {
    if !value_schema_type_includes(schema, "string") {
        return None;
    }
    schema
        .get("format")
        .and_then(Value::as_str)
        .and_then(TemporalKind::from_name)
}

fn value_content_encoding_direct(schema: &Value) -> bool {
    value_schema_type_includes(schema, "string")
        && schema
            .get("contentEncoding")
            .and_then(Value::as_str)
            .and_then(crate::json_schema::content_encoding::Encoding::from_name)
            .is_some()
}

fn temporal_serializes_non_identity(kind: TemporalKind, repr: Option<TsDateTimeTypes>) -> bool {
    match repr {
        Some(repr) => matches!(
            (kind, repr),
            (TemporalKind::DateTime, TsDateTimeTypes::Date)
                | (TemporalKind::DateTime, TsDateTimeTypes::Temporal)
                | (TemporalKind::Date, TsDateTimeTypes::Temporal)
                | (TemporalKind::Duration, TsDateTimeTypes::Temporal)
        ),
        // P15 is checked before a rendering mode is selected. Reserve an
        // identifier if any supported mode can emit it, keeping the accepted
        // namespace stable across `--date-time-types`.
        None => !matches!(kind, TemporalKind::Time),
    }
}

/// Whether the TypeScript wire serializer changes this schema's in-memory
/// value. The emitter supplies its active temporal representation; P15 supplies
/// `None` to ask whether any supported representation can change it. Keeping
/// both callers on this predicate prevents ordinary assertion-only formats
/// such as `email` from reserving helpers that are never emitted.
pub(crate) fn schema_serializes_non_identity(
    schema: &Value,
    repr: Option<TsDateTimeTypes>,
) -> bool {
    if schema.get("$ref").and_then(Value::as_str).is_some() {
        return true;
    }
    if let Some(kind) = value_temporal_kind_direct(schema) {
        return temporal_serializes_non_identity(kind, repr);
    }
    if value_content_encoding_direct(schema) {
        return true;
    }
    if let Some(branches) = schema.get("oneOf").and_then(Value::as_array)
        && branches
            .iter()
            .any(|branch| value_schema_type_includes(branch, "null"))
        && let Some(non_null) = branches
            .iter()
            .find(|branch| !value_schema_type_includes(branch, "null"))
        && (non_null.get("$ref").and_then(Value::as_str).is_some()
            || value_temporal_kind_direct(non_null).is_some()
            || value_content_encoding_direct(non_null))
    {
        return schema_serializes_non_identity(non_null, repr);
    }
    value_schema_type_includes(schema, "array")
        && schema
            .get("items")
            .is_some_and(|items| schema_serializes_non_identity(items, repr))
}

/// The materialized `TemporalKind` of a schema that is directly a temporal string
/// (the `oneOf[…, null]` wrapper is handled by the callers' recursion).
/// The materialized `contentEncoding` of a schema that is directly a bytes
/// string (the `oneOf[…, null]` wrapper is handled by the callers' recursion).
fn content_encoding_direct(
    schema: &Schema,
) -> Option<crate::json_schema::content_encoding::Encoding> {
    if schema.ty.as_ref().and_then(Value::as_str) != Some("string") {
        return None;
    }
    schema
        .content_encoding
        .as_deref()
        .and_then(crate::json_schema::content_encoding::Encoding::from_name)
}

/// The generator-owned runtime decode / encode function names for a
/// `contentEncoding`, qualified under [`DEFINITIONS_NAMESPACE`] for use in
/// model-body call sites.
fn ts_content_encoding_parse_fn(
    encoding: crate::json_schema::content_encoding::Encoding,
) -> String {
    let name = match encoding {
        crate::json_schema::content_encoding::Encoding::Base64 => "base64ToBytes",
        crate::json_schema::content_encoding::Encoding::Base64Url => "base64UrlToBytes",
    };
    format!("{DEFINITIONS_NAMESPACE}.{name}")
}

fn ts_content_encoding_serialize_fn(
    encoding: crate::json_schema::content_encoding::Encoding,
) -> String {
    let name = match encoding {
        crate::json_schema::content_encoding::Encoding::Base64 => "bytesToBase64",
        crate::json_schema::content_encoding::Encoding::Base64Url => "bytesToBase64Url",
    };
    format!("{DEFINITIONS_NAMESPACE}.{name}")
}

fn temporal_kind_direct(schema: &Schema) -> Option<TemporalKind> {
    if schema.ty.as_ref().and_then(Value::as_str) != Some("string") {
        return None;
    }
    schema.format.as_deref().and_then(TemporalKind::from_name)
}

/// The TypeScript in-memory type for a materialized temporal `format` under the
/// active `--date-time-types`. `time` is always a `string`.
fn ts_temporal_type(kind: TemporalKind, repr: TsDateTimeTypes) -> &'static str {
    match (kind, repr) {
        (TemporalKind::Time, _) => "string",
        (_, TsDateTimeTypes::String) => "string",
        (TemporalKind::DateTime, TsDateTimeTypes::Date) => "Date",
        (TemporalKind::Date | TemporalKind::Duration, TsDateTimeTypes::Date) => "string",
        (TemporalKind::DateTime, TsDateTimeTypes::Temporal) => "Temporal.ZonedDateTime",
        (TemporalKind::Date, TsDateTimeTypes::Temporal) => "Temporal.PlainDate",
        (TemporalKind::Duration, TsDateTimeTypes::Temporal) => "Temporal.Duration",
    }
}

/// The runtime parse-adapter function name for a temporal kind, qualified
/// under [`DEFINITIONS_NAMESPACE`] for use in model-body call sites.
fn ts_temporal_parse_fn(kind: TemporalKind) -> String {
    let name = match kind {
        TemporalKind::DateTime => "parseTemporalDateTime",
        TemporalKind::Date => "parseTemporalDate",
        TemporalKind::Time => "parseTemporalTime",
        TemporalKind::Duration => "parseTemporalDuration",
    };
    format!("{DEFINITIONS_NAMESPACE}.{name}")
}

fn ts_temporal_validate_fn(kind: TemporalKind) -> String {
    let name = match kind {
        TemporalKind::DateTime => "validateTemporalDateTime",
        TemporalKind::Date => "validateTemporalDate",
        TemporalKind::Time => "validateTemporalTime",
        TemporalKind::Duration => "validateTemporalDuration",
    };
    format!("{DEFINITIONS_NAMESPACE}.{name}")
}

/// The serialize expression for a materialized temporal value. `string`-stored
/// temporals (all of `string` mode, plus `date`/`time`/`duration` in `date` mode
/// and `time` in `temporal` mode) are already canonical and pass through; the
/// native-typed ones call a runtime serializer.
fn ts_temporal_serialize_call(
    kind: TemporalKind,
    repr: TsDateTimeTypes,
    value_expr: &str,
) -> String {
    let native = matches!(
        (kind, repr),
        (TemporalKind::DateTime, TsDateTimeTypes::Date)
            | (TemporalKind::DateTime, TsDateTimeTypes::Temporal)
            | (TemporalKind::Date, TsDateTimeTypes::Temporal)
            | (TemporalKind::Duration, TsDateTimeTypes::Temporal)
    );
    if !native {
        return value_expr.to_string();
    }
    let func = match kind {
        TemporalKind::DateTime => "serializeTemporalDateTime",
        TemporalKind::Date => "serializeTemporalDate",
        TemporalKind::Duration => "serializeTemporalDuration",
        TemporalKind::Time => unreachable!("time is always a string"),
    };
    let call = format!("{DEFINITIONS_NAMESPACE}.{func}({value_expr})");
    // `Date#toISOString` throws a RangeError for an invalid Date. The ordinary
    // serialize-side validator has already added a path-aware violation, so do
    // not let the wire mapper bypass that aggregated payload-validation failure while the
    // model is still assembling its output object.
    if matches!(
        (kind, repr),
        (TemporalKind::DateTime, TsDateTimeTypes::Date)
    ) {
        return format!("Number.isFinite({value_expr}.getTime()) ? {call} : undefined");
    }
    call
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
struct Schema {
    #[serde(rename = "$ref")]
    reference: Option<String>,
    #[serde(rename = "type")]
    ty: Option<Value>,
    title: Option<String>,
    description: Option<String>,
    deprecated: Option<bool>,
    properties: Option<IndexMap<String, Schema>>,
    required: Option<Vec<String>>,
    #[serde(rename = "additionalProperties")]
    additional_properties: Option<Value>,
    items: Option<Box<Schema>>,
    #[serde(rename = "oneOf")]
    one_of: Option<Vec<Schema>>,
    default: Option<Value>,
    #[serde(rename = "const")]
    const_value: Option<Value>,
    #[serde(rename = "maxProperties")]
    max_properties: Option<usize>,
    #[serde(rename = "minProperties")]
    min_properties: Option<usize>,
    #[serde(rename = "propertyNames")]
    property_names: Option<Box<Schema>>,
    #[serde(rename = "dependentRequired")]
    dependent_required: Option<IndexMap<String, Vec<String>>>,
    minimum: Option<serde_json::Number>,
    maximum: Option<serde_json::Number>,
    #[serde(rename = "exclusiveMinimum")]
    exclusive_minimum: Option<serde_json::Number>,
    #[serde(rename = "exclusiveMaximum")]
    exclusive_maximum: Option<serde_json::Number>,
    #[serde(rename = "multipleOf")]
    multiple_of: Option<serde_json::Number>,
    #[serde(rename = "minLength")]
    min_length: Option<u64>,
    #[serde(rename = "maxLength")]
    max_length: Option<u64>,
    pattern: Option<String>,
    format: Option<String>,
    #[serde(rename = "contentEncoding")]
    content_encoding: Option<String>,
    #[serde(rename = "minItems")]
    min_items: Option<u64>,
    #[serde(rename = "maxItems")]
    max_items: Option<u64>,
    #[serde(rename = "uniqueItems")]
    unique_items: Option<bool>,
    contains: Option<Box<Schema>>,
    #[serde(rename = "minContains")]
    min_contains: Option<u64>,
    #[serde(rename = "maxContains")]
    max_contains: Option<u64>,
    #[serde(rename = "enum")]
    enum_values: Option<Vec<Value>>,
    #[serde(rename = "x-ts-name")]
    x_ts_name: Option<String>,
}

impl Schema {
    /// The emitted TypeScript member identifier for a property: the `x-ts-name`
    /// override if present (verbatim), otherwise the camelCased JSON name. The
    /// wire key is unaffected (the interface pins the original key). See
    /// specs/json-schema/features/properties.md.
    fn ts_member_name(&self, json_name: &str) -> String {
        self.x_ts_name
            .clone()
            .unwrap_or_else(|| typescript_generated_field_name(json_name))
    }

    fn has_numeric_constraints(&self) -> bool {
        self.minimum.is_some()
            || self.maximum.is_some()
            || self.exclusive_minimum.is_some()
            || self.exclusive_maximum.is_some()
            || self.multiple_of.is_some()
    }

    fn has_string_constraints(&self) -> bool {
        // `minLength: 0` is the spec's explicit no-op ([[minLength]]); emitting
        // it would spread the whole string for an unreachable comparison.
        self.effective_min_length().is_some()
            || self.max_length.is_some()
            || self.pattern.is_some()
            || self.format.is_some()
    }

    /// The `minLength` that actually constrains anything: `0` is equivalent to
    /// the keyword being absent.
    fn effective_min_length(&self) -> Option<u64> {
        self.min_length.filter(|min| *min > 0)
    }

    fn has_array_constraints(&self) -> bool {
        self.min_items.is_some()
            || self.max_items.is_some()
            || self.unique_items == Some(true)
            || self.contains.is_some()
    }
}

fn ts_index_access(object: &str, key: &str) -> String {
    format!("{object}[{}]", typescript_string_literal(key))
}

fn ts_has_own(object: &str, key: &str) -> String {
    format!(
        "Object.prototype.hasOwnProperty.call({object}, {})",
        typescript_string_literal(key)
    )
}

fn ts_member_conflicts_with_object_prototype(name: &str) -> bool {
    matches!(
        name,
        "constructor"
            | "hasOwnProperty"
            | "isPrototypeOf"
            | "propertyIsEnumerable"
            | "toLocaleString"
            | "toString"
            | "valueOf"
            | "__defineGetter__"
            | "__defineSetter__"
            | "__lookupGetter__"
            | "__lookupSetter__"
            | "__proto__"
    )
}

fn ts_bound_literal(number: &serde_json::Number, is_integer: bool) -> String {
    if is_integer && let Some(value) = number.as_f64() {
        return (value.trunc() as i64).to_string();
    }
    number.to_string()
}

/// Emits the numeric-constraint predicates over `value_expr` (a validated
/// `number` in scope), pushing Violations. `emit_integer_cap` adds the P12
/// serialize-side ±(2^53−1) re-check; the parse adapter has already classified
/// the token with `Number.isSafeInteger`, so it passes `false` rather than
/// emitting the same comparison twice.
fn render_ts_numeric_checks(
    output: &mut String,
    value_expr: &str,
    path_expr: &str,
    schema: &Schema,
    indent: &str,
    emit_integer_cap: bool,
) {
    let is_integer = schema.ty.as_ref().and_then(Value::as_str) == Some("integer");
    let mut body = String::new();
    let mut emit = |condition: String, reason: String| {
        body.push_str(indent);
        body.push_str("if (");
        body.push_str(&condition);
        body.push_str(") {\n");
        body.push_str(indent);
        body.push_str("  violations.push({ path: ");
        body.push_str(path_expr);
        body.push_str(", reason: `");
        body.push_str(&reason);
        body.push_str("` });\n");
        body.push_str(indent);
        body.push_str("}\n");
    };
    if let Some(min) = &schema.minimum {
        let bound = ts_bound_literal(min, is_integer);
        emit(
            format!("{value_expr} < {bound}"),
            format!("must be >= {bound}, got ${{{value_expr}}}"),
        );
    }
    if let Some(max) = &schema.maximum {
        let bound = ts_bound_literal(max, is_integer);
        emit(
            format!("{value_expr} > {bound}"),
            format!("must be <= {bound}, got ${{{value_expr}}}"),
        );
    }
    if let Some(min) = &schema.exclusive_minimum {
        let bound = ts_bound_literal(min, is_integer);
        emit(
            format!("{value_expr} <= {bound}"),
            format!("must be > {bound}, got ${{{value_expr}}}"),
        );
    }
    if let Some(max) = &schema.exclusive_maximum {
        let bound = ts_bound_literal(max, is_integer);
        emit(
            format!("{value_expr} >= {bound}"),
            format!("must be < {bound}, got ${{{value_expr}}}"),
        );
    }
    if let Some(divisor) = &schema.multiple_of {
        let bound = ts_bound_literal(divisor, is_integer);
        emit(
            format!("{value_expr} % {bound} !== 0"),
            format!("must be a multiple of {bound}, got ${{{value_expr}}}"),
        );
    }
    if is_integer && !emit_integer_cap {
        output.push_str(&body);
        return;
    }
    if is_integer {
        // The `integer` cap (`type.md` §Serialize-side). A JS
        // `number` also has no integral or finite guarantee in memory, so the
        // one `Number.isSafeInteger` guard covers the cap, the fractional part
        // and `NaN`/`±Infinity` — all of which would otherwise emit a payload
        // every target's parser (including this one) rejects.
        output.push_str(indent);
        output.push_str(&format!("if (!Number.isSafeInteger({value_expr})) {{\n"));
        output.push_str(indent);
        output.push_str(&format!(
            "  violations.push({{ path: {path_expr}, reason: 'exceeds ±(2^53-1) integer cap' }});\n"
        ));
        output.push_str(indent);
        output.push_str("}");
        if body.is_empty() {
            output.push('\n');
        } else {
            output.push_str(" else {\n");
            output.push_str(&body);
            output.push_str(indent);
            output.push_str("}\n");
        }
        return;
    }

    output.push_str(indent);
    output.push_str(&format!("if (!Number.isFinite({value_expr})) {{\n"));
    output.push_str(indent);
    output.push_str(&format!(
        "  violations.push({{ path: {path_expr}, reason: `must be a finite number, got ${{{value_expr}}}` }});\n"
    ));
    output.push_str(indent);
    output.push_str("}");
    if body.is_empty() {
        output.push('\n');
    } else {
        output.push_str(" else {\n");
        output.push_str(&body);
        output.push_str(indent);
        output.push_str("}\n");
    }
}

/// Emits the string-length predicates (`minLength`/`maxLength`) over
/// `value_expr` (a validated `string` in scope). Length is the Unicode
/// code-point count from the shared surrogate-aware `codePointLength` scan —
/// never `s.length` (UTF-16 code units) and never `[...s].length` (which
/// allocates a whole code-point array before the bound can fire). See
/// `specs/json-schema/features/maxLength.md`.
fn render_ts_string_checks(
    output: &mut String,
    value_expr: &str,
    path_expr: &str,
    schema: &Schema,
    indent: &str,
) {
    render_ts_string_assertions(output, value_expr, path_expr, schema, indent, true);
}

/// Emits string assertions over the observable wire string. Materialized
/// temporals validate their format through the dedicated calendar-aware helper,
/// so their sibling assertions use this with `include_format = false`.
fn render_ts_string_assertions(
    output: &mut String,
    value_expr: &str,
    path_expr: &str,
    schema: &Schema,
    indent: &str,
    include_format: bool,
) {
    if let Some(min) = schema.effective_min_length() {
        // The scan stops the moment the count reaches `min`; if the string ended
        // first the count in hand is already exact, so the failure path needs no
        // second pass (`minLength.md` TS row).
        output.push_str(indent);
        output.push_str("{\n");
        output.push_str(indent);
        output.push_str(&format!(
            "  const codePoints = {}({value_expr}, {min});\n",
            code_point_length_fn()
        ));
        output.push_str(indent);
        output.push_str(&format!("  if (codePoints < {min}) {{\n"));
        output.push_str(indent);
        output.push_str(&format!(
            "    violations.push({{ path: {path_expr}, reason: `must have length >= {min}, got ${{codePoints}}` }});\n"
        ));
        output.push_str(indent);
        output.push_str("  }\n");
        output.push_str(indent);
        output.push_str("}\n");
    }
    if let Some(max) = schema.max_length {
        // Early-exit against the bound: the scan stops at `max + 1` code points,
        // so an adversarially long string costs O(max), not O(n). Only the
        // (rare) failure path pays the exact recount for the message
        // (`maxLength.md` TS row).
        output.push_str(indent);
        output.push_str(&format!(
            "if ({}({value_expr}, {max}) > {max}) {{\n",
            code_point_length_fn()
        ));
        output.push_str(indent);
        output.push_str(&format!(
            "  violations.push({{ path: {path_expr}, reason: `must have length <= {max}, got ${{{}({value_expr})}}` }});\n",
            code_point_length_fn()
        ));
        output.push_str(indent);
        output.push_str("}\n");
    }
    if let Some(pattern) = &schema.pattern {
        render_ts_pattern_check(output, value_expr, path_expr, pattern, indent);
    }
    if include_format && let Some(format) = &schema.format {
        render_ts_format_check(output, value_expr, path_expr, format, indent);
    }
}

/// Emits the `format` predicate over `value_expr` (a validated `string`): the
/// length guard (if any) short-circuits **before** the pinned regex, so one
/// combined condition pushes a single Violation naming the format + value. TS
/// keeps `$` (JS end-anchor is exception-free). See
/// `specs/json-schema/features/format.md`.
fn render_ts_format_check(
    output: &mut String,
    value_expr: &str,
    path_expr: &str,
    format: &str,
    indent: &str,
) {
    let Some(check) = crate::json_schema::format::check_for(format) else {
        return;
    };
    let const_name = ts_pattern_const_name(&check.pattern);
    output.push_str(indent);
    output.push_str("if (");
    if let Some(max) = check.max_code_points {
        output.push_str(&format!(
            "{}({value_expr}, {max}) > {max} || ",
            code_point_length_fn()
        ));
    }
    output.push_str(&format!("!{const_name}.test({value_expr})) {{\n"));
    output.push_str(indent);
    output.push_str(&format!(
        "  violations.push({{ path: {path_expr}, reason: `must be a valid {}, got ${{JSON.stringify({value_expr})}}` }});\n",
        check.name
    ));
    output.push_str(indent);
    output.push_str("}\n");
}

/// The module-level compiled `RegExp` const name for a `pattern`, keyed by the
/// (normalized) pattern text so identical patterns share one compiled instance
/// per module. Stable FNV-1a hash → a valid TS identifier.
fn ts_pattern_const_name(pattern: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in pattern.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("PATTERN_{hash:016X}")
}

/// Escapes a string for inclusion in a JS template literal (backtick, backslash,
/// and `${` interpolation), so an emitted `pattern` displays verbatim without
/// breaking the surrounding `` `...` ``.
fn ts_template_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('`', "\\`")
        .replace('$', "\\$")
}

/// Emits the `pattern` predicate over `value_expr` (a validated `string`):
/// `if (!PAT.test(v)) violations.push(...)`. `.test` is unanchored and — with no
/// `g` flag — stateless; the const carries the `u` flag (code-point `.`). TS
/// keeps `$` (JS end-anchor is already exception-free). See
/// `specs/json-schema/features/pattern.md`.
fn render_ts_pattern_check(
    output: &mut String,
    value_expr: &str,
    path_expr: &str,
    pattern: &str,
    indent: &str,
) {
    let const_name = ts_pattern_const_name(pattern);
    let escaped = ts_template_escape(pattern);
    output.push_str(indent);
    output.push_str(&format!("if (!{const_name}.test({value_expr})) {{\n"));
    output.push_str(indent);
    output.push_str(&format!(
        "  violations.push({{ path: {path_expr}, reason: `must match pattern {escaped}, got ${{JSON.stringify({value_expr})}}` }});\n"
    ));
    output.push_str(indent);
    output.push_str("}\n");
}

/// Emits the module-level `const PATTERN_… = new RegExp("…", "u");` for every
/// distinct `pattern` across `models`' string fields — compiled once per module.
/// The pattern is the loader-normalized form; TS keeps `$`.
fn render_pattern_regexes(output: &mut String, models: &[&PlannedJsonType]) -> Result<()> {
    let mut patterns = Vec::new();
    for model in models {
        collect_schema_patterns(&decode_schema(model)?, &mut patterns);
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut emitted = false;
    for pattern in patterns {
        let name = ts_pattern_const_name(&pattern);
        if !seen.insert(name.clone()) {
            continue;
        }
        if !emitted {
            output.push('\n');
            emitted = true;
        }
        match ts_regex_literal(&pattern) {
            Some(literal) => output.push_str(&format!("const {name} = {literal};\n")),
            None => output.push_str(&format!(
                "const {name} = new RegExp({}, \"u\");\n",
                typescript_string_literal(&pattern)
            )),
        }
    }
    Ok(())
}

/// Spells a `pattern` as a module-level regex literal (`/…/u`), the default form
/// `pattern.md`'s TypeScript row prescribes. Returns `None` for the shapes a
/// literal cannot carry — an empty body (`//` opens a comment), a line
/// terminator, or a trailing lone backslash — so those fall back to
/// `new RegExp(<string>, "u")`. An unescaped `/` is escaped as `\/`, which the
/// `u` grammar admits as an identity escape.
fn ts_regex_literal(pattern: &str) -> Option<String> {
    if pattern.is_empty()
        || pattern
            .chars()
            .any(|ch| matches!(ch, '\n' | '\r' | '\u{2028}' | '\u{2029}'))
    {
        return None;
    }
    let mut literal = String::from("/");
    let mut escaped = false;
    for ch in pattern.chars() {
        if escaped {
            literal.push(ch);
            escaped = false;
            continue;
        }
        match ch {
            '\\' => {
                literal.push('\\');
                escaped = true;
            }
            '/' => literal.push_str("\\/"),
            _ => literal.push(ch),
        }
    }
    if escaped {
        return None;
    }
    literal.push_str("/u");
    Some(literal)
}

/// Collects every compiled-regex source a model's checks reference, in every
/// string position it can occur: a declared property, an array element at any
/// depth, a typed map's member, a key-shape subschema, and a nullability
/// wrapper's branch. Missing one leaves the emitted check referencing an
/// undeclared `PATTERN_…` const.
fn collect_schema_patterns(schema: &Schema, patterns: &mut Vec<String>) {
    if let Some(pattern) = &schema.pattern {
        patterns.push(pattern.clone());
    }
    if let Some(format) = &schema.format
        && let Some(check) = crate::json_schema::format::check_for(format)
    {
        patterns.push(check.pattern.to_string());
    }
    for property in schema
        .properties
        .iter()
        .flat_map(|entries| entries.values())
    {
        collect_schema_patterns(property, patterns);
    }
    if let Some(items) = &schema.items {
        collect_schema_patterns(items, patterns);
    }
    if let Some(contains) = &schema.contains {
        collect_schema_patterns(contains, patterns);
    }
    for branch in schema.one_of.iter().flatten() {
        collect_schema_patterns(branch, patterns);
    }
    if let Some(names) = &schema.property_names {
        collect_schema_patterns(names, patterns);
    }
    if let Some(Value::Object(members)) = &schema.additional_properties
        && let Ok(member) = serde_json::from_value::<Schema>(Value::Object(members.clone()))
    {
        collect_schema_patterns(&member, patterns);
    }
}

/// Emits the object member-count predicates (`minProperties`/`maxProperties`)
/// over `count_expr` (the number of distinct wire member keys, one number over
/// the whole object). See `specs/json-schema/features/minProperties.md`.
fn render_ts_property_count_checks(
    output: &mut String,
    count_expr: &str,
    schema: &Schema,
    indent: &str,
) {
    if let Some(min) = schema.min_properties {
        output.push_str(indent);
        output.push_str(&format!("if ({count_expr} < {min}) {{\n"));
        output.push_str(indent);
        output.push_str(&format!(
            "  violations.push({{ path: '', reason: `must have at least {min} properties, got ${{{count_expr}}}` }});\n"
        ));
        output.push_str(indent);
        output.push_str("}\n");
    }
    if let Some(max) = schema.max_properties {
        output.push_str(indent);
        output.push_str(&format!("if ({count_expr} > {max}) {{\n"));
        output.push_str(indent);
        output.push_str(&format!(
            "  violations.push({{ path: '', reason: `must have at most {max} properties, got ${{{count_expr}}}` }});\n"
        ));
        output.push_str(indent);
        output.push_str("}\n");
    }
}

/// Emits the `propertyNames` key-shape predicate over the keys of `keys_expr`
/// (a `string[]`), applying the (string-length) key subschema to each key. See
/// `specs/json-schema/features/propertyNames.md`.
fn render_ts_property_name_checks(
    output: &mut String,
    keys_expr: &str,
    subschema: &Schema,
    indent: &str,
) {
    let matcher = scalar_matcher(subschema);
    if matcher.min_length.is_none_or(|min| min == 0)
        && matcher.max_length.is_none()
        && matcher.pattern.is_none()
        && matcher.format.is_none()
        && matcher.enum_values.is_empty()
    {
        return;
    }
    output.push_str(indent);
    output.push_str(&format!("for (const key of {keys_expr}) {{\n"));
    let inner = format!("{indent}  ");
    let mut emit = |condition: String, reason: String| {
        output.push_str(&inner);
        output.push_str(&format!("if ({condition}) {{\n"));
        output.push_str(&inner);
        output.push_str(&format!(
            "  violations.push({{ path: {DEFINITIONS_NAMESPACE}.memberPath(key), reason: `invalid property name \"${{key}}\": {reason}` }});\n"
        ));
        output.push_str(&inner);
        output.push_str("}\n");
    };
    if let Some(min) = matcher.min_length.filter(|min| *min > 0) {
        let count = format!("{}(key, {min})", code_point_length_fn());
        emit(
            format!("{count} < {min}"),
            format!("must have length >= {min}, got ${{{count}}}"),
        );
    }
    if let Some(max) = matcher.max_length {
        emit(
            format!("{}(key, {max}) > {max}", code_point_length_fn()),
            format!(
                "must have length <= {max}, got ${{{}(key)}}",
                code_point_length_fn()
            ),
        );
    }
    if !matcher.enum_values.is_empty() {
        let literals = matcher
            .enum_values
            .iter()
            .map(|value| typescript_value_literal(value).unwrap_or_else(|_| "undefined".into()))
            .collect::<Vec<_>>();
        emit(
            literals
                .iter()
                .map(|literal| format!("key !== {literal}"))
                .collect::<Vec<_>>()
                .join(" && "),
            format!(
                "must be one of [{}], got ${{JSON.stringify(key)}}",
                literals.join(", ")
            ),
        );
    }
    drop(emit);
    if let Some(pattern) = &matcher.pattern {
        let const_name = ts_pattern_const_name(pattern);
        let escaped = ts_template_escape(pattern);
        output.push_str(&inner);
        output.push_str(&format!("if (!{const_name}.test(key)) {{\n"));
        output.push_str(&inner);
        output.push_str(&format!(
            "  violations.push({{ path: {DEFINITIONS_NAMESPACE}.memberPath(key), reason: `invalid property name \"${{key}}\": must match pattern {escaped}, got ${{JSON.stringify(key)}}` }});\n"
        ));
        output.push_str(&inner);
        output.push_str("}\n");
    }
    if let Some(format) = &matcher.format
        && let Some(check) = crate::json_schema::format::check_for(format)
    {
        let const_name = ts_pattern_const_name(&check.pattern);
        output.push_str(&inner);
        output.push_str("if (");
        if let Some(max) = check.max_code_points {
            output.push_str(&format!(
                "{}(key, {max}) > {max} || ",
                code_point_length_fn()
            ));
        }
        output.push_str(&format!("!{const_name}.test(key)) {{\n"));
        output.push_str(&inner);
        output.push_str(&format!(
            "  violations.push({{ path: {DEFINITIONS_NAMESPACE}.memberPath(key), reason: `invalid property name \"${{key}}\": must be a valid {}, got ${{JSON.stringify(key)}}` }});\n",
            check.name
        ));
        output.push_str(&inner);
        output.push_str("}\n");
    }
    output.push_str(indent);
    output.push_str("}\n");
}

/// Emits the `dependentRequired` cross-field presence predicate over the
/// presence object `obj_expr`: for each present trigger key, each dependent key
/// must also be present. See `specs/json-schema/features/dependentRequired.md`.
fn render_ts_dependent_required(
    output: &mut String,
    obj_expr: &str,
    schema: &Schema,
    indent: &str,
) {
    let Some(dependent_required) = &schema.dependent_required else {
        return;
    };
    for (trigger, deps) in dependent_required {
        if deps.is_empty() {
            continue;
        }
        output.push_str(indent);
        output.push_str(&format!("if ({}) {{\n", ts_has_own(obj_expr, trigger)));
        for dep in deps {
            output.push_str(indent);
            output.push_str(&format!("  if (!{}) {{\n", ts_has_own(obj_expr, dep)));
            output.push_str(indent);
            output.push_str(&format!(
                "    violations.push({{ path: {}, reason: `property \"{dep}\" is required when \"{trigger}\" is present` }});\n",
                typescript_string_literal(dep)
            ));
            output.push_str(indent);
            output.push_str("  }\n");
        }
        output.push_str(indent);
        output.push_str("}\n");
    }
}

/// Renders a TypeScript literal for a scalar matcher value in the element's
/// static type.
fn ts_scalar_literal(value: &Value, integer: bool) -> String {
    match value {
        Value::Number(number) => ts_bound_literal(number, integer),
        _ => typescript_value_literal(value).unwrap_or_else(|_| "undefined".to_string()),
    }
}

/// Removes container/applicator details from a decoded node before rendering a
/// scalar predicate. The loader has already proved this is a supported matcher.
fn scalar_matcher(schema: &Schema) -> ScalarMatcher {
    ScalarMatcher {
        kind: schema
            .ty
            .as_ref()
            .and_then(Value::as_str)
            .and_then(ScalarKind::from_name),
        const_value: schema.const_value.clone(),
        enum_values: schema.enum_values.clone().unwrap_or_default(),
        minimum: schema.minimum.clone(),
        maximum: schema.maximum.clone(),
        exclusive_minimum: schema.exclusive_minimum.clone(),
        exclusive_maximum: schema.exclusive_maximum.clone(),
        multiple_of: schema.multiple_of.clone(),
        min_length: schema.min_length,
        max_length: schema.max_length,
        pattern: schema.pattern.clone(),
        format: schema.format.clone(),
    }
}

/// Builds the boolean TypeScript sub-conditions that define "match" for a scalar
/// `contains` matcher over `elem`. A type-only matcher matches every element, so
/// an empty condition set renders as the literal `true`.
fn ts_matcher_condition(matcher: &Schema, elem: &str, element_ty: Option<&str>) -> String {
    let matcher = scalar_matcher(matcher);
    let is_integer = match matcher.kind {
        Some(ScalarKind::Number) => false,
        Some(ScalarKind::Integer) => element_ty != Some("number"),
        _ => element_ty == Some("integer"),
    };
    // A matcher that declares no `type` still needs a runtime kind guard: the
    // raw wire array can hold anything, and an unguarded string/number predicate
    // either throws (`[...5]`) or silently coerces (`"9" >= 5`) instead of
    // aggregating a Violation. Fall back to the element type, as Java and Python
    // do (`contains.md` §Validator mapping, P11).
    let kind = matcher
        .kind
        .or_else(|| element_ty.and_then(ScalarKind::from_name));
    let mut parts: Vec<String> = Vec::new();
    match kind {
        Some(ScalarKind::String) => parts.push(format!("typeof {elem} === 'string'")),
        Some(ScalarKind::Boolean) => parts.push(format!("typeof {elem} === 'boolean'")),
        Some(ScalarKind::Number) => parts.push(format!(
            "typeof {elem} === 'number' && Number.isFinite({elem})"
        )),
        Some(ScalarKind::Integer) => parts.push(format!(
            "typeof {elem} === 'number' && Number.isSafeInteger({elem})"
        )),
        None => {}
    }
    if let Some(value) = &matcher.const_value {
        parts.push(format!(
            "{elem} === {}",
            ts_scalar_literal(value, is_integer)
        ));
    }
    if !matcher.enum_values.is_empty() {
        let alternatives = matcher
            .enum_values
            .iter()
            .map(|value| format!("{elem} === {}", ts_scalar_literal(value, is_integer)))
            .collect::<Vec<_>>()
            .join(" || ");
        if !alternatives.is_empty() {
            parts.push(format!("({alternatives})"));
        }
    }
    if let Some(min) = &matcher.minimum {
        parts.push(format!("{elem} >= {}", ts_bound_literal(min, is_integer)));
    }
    if let Some(max) = &matcher.maximum {
        parts.push(format!("{elem} <= {}", ts_bound_literal(max, is_integer)));
    }
    if let Some(min) = &matcher.exclusive_minimum {
        parts.push(format!("{elem} > {}", ts_bound_literal(min, is_integer)));
    }
    if let Some(max) = &matcher.exclusive_maximum {
        parts.push(format!("{elem} < {}", ts_bound_literal(max, is_integer)));
    }
    if let Some(divisor) = &matcher.multiple_of {
        parts.push(format!(
            "{elem} % {} === 0",
            ts_bound_literal(divisor, is_integer)
        ));
    }
    if let Some(min) = matcher.min_length.filter(|min| *min > 0) {
        parts.push(format!(
            "{}({elem}, {min}) >= {min}",
            code_point_length_fn()
        ));
    }
    if let Some(max) = matcher.max_length {
        parts.push(format!(
            "{}({elem}, {max}) <= {max}",
            code_point_length_fn()
        ));
    }
    if let Some(pattern) = &matcher.pattern {
        parts.push(format!("{}.test({elem})", ts_pattern_const_name(pattern)));
    }
    if let Some(format) = &matcher.format
        && let Some(check) = crate::json_schema::format::check_for(format)
    {
        if let Some(max) = check.max_code_points {
            parts.push(format!(
                "{}({elem}, {max}) <= {max}",
                code_point_length_fn()
            ));
        }
        parts.push(format!(
            "{}.test({elem})",
            ts_pattern_const_name(&check.pattern)
        ));
    }
    if parts.is_empty() {
        "true".to_string()
    } else {
        parts.join(" && ")
    }
}

/// Emits the array-constraint predicates over `array_expr` (a built array in
/// scope) into the parser or serializer body, pushing Violations.
///
/// `depth` is the array's own nesting level; every loop variable this emits
/// carries that depth, so an inner level never shadows the enclosing level's
/// `index` — which the surrounding `path_expr` interpolates
/// (`items.md` §"Nested arrays").
///
/// `serialize` selects the in-memory side. Element equality and the `contains`
/// matcher are always evaluated over the **wire** value in both directions (D10),
/// so a materialized element (`Uint8Array`, `Temporal.*`, `Date`) is compared by
/// its canonical wire string rather than by reference.
fn render_ts_array_checks(
    output: &mut String,
    array_expr: &str,
    path_expr: &str,
    schema: &Schema,
    indent: &str,
    depth: usize,
    serialize: bool,
) {
    // A nullable element ([[nullability]]'s `oneOf: [T, null]` wrapper) declares
    // no `type` of its own; the element kind lives on its non-null branch. Reading
    // the wrapper would leave the matcher unguarded — and `null >= 0` is `true` in
    // JS, so an unguarded numeric matcher would count `null` as a match
    // (`contains.md` Interactions → [[nullability]]: a `null` element never
    // satisfies a scalar matcher).
    let element_schema = schema
        .items
        .as_deref()
        .map(|item| nullable_non_null_schema(item).unwrap_or(item));
    let element_ty = element_schema
        .and_then(|item| item.ty.as_ref())
        .and_then(Value::as_str);
    let suffix = ts_depth_suffix(depth);
    let element = format!("element{suffix}");
    let index = format!("index{suffix}");
    if let Some(min) = schema.min_items {
        output.push_str(indent);
        output.push_str(&format!("if ({array_expr}.length < {min}) {{\n"));
        output.push_str(indent);
        output.push_str(&format!(
            "  violations.push({{ path: {path_expr}, reason: `must have at least {min} items, got ${{{array_expr}.length}}` }});\n"
        ));
        output.push_str(indent);
        output.push_str("}\n");
    }
    if let Some(max) = schema.max_items {
        output.push_str(indent);
        output.push_str(&format!("if ({array_expr}.length > {max}) {{\n"));
        output.push_str(indent);
        output.push_str(&format!(
            "  violations.push({{ path: {path_expr}, reason: `must have at most {max} items, got ${{{array_expr}.length}}` }});\n"
        ));
        output.push_str(indent);
        output.push_str("}\n");
    }
    if schema.unique_items.is_none_or(|unique| !unique) && schema.contains.is_none() {
        return;
    }
    // Both value-comparing keywords run over the wire value. On the parse side
    // `array_expr` already holds raw wire elements; on the serialize side a
    // materialized element is mapped through its canonical wire form first.
    let mut values_expr = array_expr.to_string();
    let mut indent = indent.to_string();
    let mut wrapped = false;
    if serialize && let Some(items) = schema.items.as_deref() {
        let wire = serialize_expr(items, &element);
        if wire != element {
            let wire_items = format!("wireItems{suffix}");
            output.push_str(&indent);
            output.push_str("{\n");
            let inner = format!("{indent}  ");
            output.push_str(&inner);
            output.push_str(&format!(
                "const {wire_items} = {array_expr}.map(({element}) => {wire});\n"
            ));
            values_expr = wire_items;
            indent = inner;
            wrapped = true;
        }
    }
    let indent = indent.as_str();
    if schema.unique_items == Some(true) {
        output.push_str(indent);
        output.push_str("{\n");
        output.push_str(indent);
        output.push_str("  const seen = new Map<unknown, number>();\n");
        output.push_str(indent);
        output.push_str(&format!(
            "  {values_expr}.forEach(({element}, {index}) => {{\n"
        ));
        output.push_str(indent);
        output.push_str(&format!("    if (seen.has({element})) {{\n"));
        output.push_str(indent);
        output.push_str(&format!(
            "      violations.push({{ path: {path_expr}, reason: `duplicate items: element at index ${{{index}}} equals index ${{seen.get({element})}}` }});\n"
        ));
        output.push_str(indent);
        output.push_str("    } else {\n");
        output.push_str(indent);
        output.push_str(&format!("      seen.set({element}, {index});\n"));
        output.push_str(indent);
        output.push_str("    }\n");
        output.push_str(indent);
        output.push_str("  });\n");
        output.push_str(indent);
        output.push_str("}\n");
    }
    if let Some(matcher) = &schema.contains {
        let condition = ts_matcher_condition(matcher, &element, element_ty);
        let effective_min = schema.min_contains.unwrap_or(1);
        let inner = format!("{indent}  ");
        output.push_str(indent);
        output.push_str("{\n");
        output.push_str(&inner);
        output.push_str(&format!(
            "const matchCount{suffix} = {values_expr}.filter(({element}) => {condition}).length;\n"
        ));
        if effective_min > 0 {
            output.push_str(&inner);
            output.push_str(&format!("if (matchCount{suffix} < {effective_min}) {{\n"));
            output.push_str(&inner);
            if schema.min_contains.is_some() {
                output.push_str(&format!(
                    "  violations.push({{ path: {path_expr}, reason: `too few matching items: at least {effective_min}, got ${{matchCount{suffix}}}` }});\n"
                ));
            } else {
                output.push_str(&format!(
                    "  violations.push({{ path: {path_expr}, reason: 'no element matches the required schema' }});\n"
                ));
            }
            output.push_str(&inner);
            output.push_str("}\n");
        }
        if let Some(max) = schema.max_contains {
            output.push_str(&inner);
            output.push_str(&format!("if (matchCount{suffix} > {max}) {{\n"));
            output.push_str(&inner);
            output.push_str(&format!(
                "  violations.push({{ path: {path_expr}, reason: `too many matching items: at most {max}, got ${{matchCount{suffix}}}` }});\n"
            ));
            output.push_str(&inner);
            output.push_str("}\n");
        }
        output.push_str(indent);
        output.push_str("}\n");
    }
    if wrapped {
        output.push_str(&indent[2..]);
        output.push_str("}\n");
    }
}

/// The per-nesting-level suffix for generated loop variables. Level 0 keeps the
/// unsuffixed names; every deeper level suffixes its own, so an inner loop never
/// shadows the enclosing one (`items.md` §"Nested arrays").
fn ts_depth_suffix(depth: usize) -> String {
    if depth == 0 {
        String::new()
    } else {
        depth.to_string()
    }
}

/// True when a field schema carries a constraint the serialize path must
/// re-check over the in-memory value (P12, both directions). Mirrors the
/// dispatch in `render_ts_field_checks`.
fn field_needs_serialize_check(schema: &Schema) -> bool {
    // A nullability wrapper declares nothing itself; its non-null branch carries
    // the constraints, checked under a `!== null` guard.
    if let Some(non_null) = nullable_non_null_schema(schema) {
        return field_needs_serialize_check(non_null);
    }
    // A nested converter may reject a mutated in-memory value. The containing
    // converter needs its own collection so it can re-path that failure.
    if schema.reference.is_some() {
        return true;
    }
    if temporal_kind_direct(schema).is_some() {
        return true;
    }
    if schema.const_value.is_some() || schema.enum_values.is_some() {
        return true;
    }
    // An inline sum type: any branch that declares something is re-checked
    // against the member it holds ([[oneOf]] §"Serialize-side"). A `$ref` branch
    // validates through its own mapper, so only the non-object branches count.
    if is_ts_union(schema) {
        return schema
            .one_of
            .iter()
            .flatten()
            .any(field_needs_serialize_check);
    }
    match schema.ty.as_ref().and_then(Value::as_str) {
        Some("string") | Some("boolean") => true,
        // `number` always carries the wire-wide finiteness predicate, and
        // `integer` the ±(2^53−1) cap, even when the schema declares no bound.
        Some("number") | Some("integer") => true,
        Some("array") => true,
        _ => false,
    }
}

/// True when a model's `toTransferType` must run collecting validation before
/// emitting the wire object: any constrained declared field, a constrained
/// typed-map value, or an object-level count/name/dependency constraint.
fn model_needs_serialize_validation(schema: &Schema) -> Result<bool> {
    if schema.min_properties.is_some()
        || schema.max_properties.is_some()
        || schema.dependent_required.is_some()
        || schema.property_names.is_some()
    {
        return Ok(true);
    }
    // A mixed object must always reject catch-all keys that collide with a
    // declared wire property, even when its value schema has no assertions.
    if is_open_object(schema) {
        return Ok(true);
    }
    if let Some(value_schema) = additional_properties_value_schema(schema)?
        && field_needs_serialize_check(&value_schema)
    {
        return Ok(true);
    }
    // A map-shaped model's members are the wire object itself, so the
    // object-level checks above already cover it — typed or free-form.
    if let Some(properties) = &schema.properties {
        for property in properties.values() {
            if field_needs_serialize_check(property) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Emits the closed value-set membership predicate over `value_expr` (a typed
/// in-memory value) for the serialize path, pushing the same informative
/// Violation as the parse path. `compare_exprs` are the admissible JS literals.
fn render_ts_serialize_closed_check(
    output: &mut String,
    compare_exprs: &[String],
    value_expr: &str,
    path_expr: &str,
    indent: &str,
    reason: &str,
) {
    let membership = compare_exprs
        .iter()
        .map(|expr| format!("{value_expr} !== {expr}"))
        .collect::<Vec<_>>()
        .join(" && ");
    output.push_str(indent);
    output.push_str(&format!("if ({membership}) {{\n"));
    output.push_str(indent);
    output.push_str(&format!(
        "  violations.push({{ path: {path_expr}, reason: `{reason}` }});\n"
    ));
    output.push_str(indent);
    output.push_str("}\n");
}

fn render_ts_schema_closed_check(
    output: &mut String,
    schema: &Schema,
    value_expr: &str,
    path_expr: &str,
    indent: &str,
) {
    if let Some(const_value) = &schema.const_value {
        let literal =
            typescript_value_literal(const_value).unwrap_or_else(|_| "undefined".to_string());
        render_ts_serialize_closed_check(
            output,
            std::slice::from_ref(&literal),
            value_expr,
            path_expr,
            indent,
            &format!("must equal {literal}"),
        );
    } else if let Some(values) = &schema.enum_values {
        let literals = values
            .iter()
            .map(|value| {
                typescript_value_literal(value).unwrap_or_else(|_| "undefined".to_string())
            })
            .collect::<Vec<_>>();
        render_ts_serialize_closed_check(
            output,
            &literals,
            value_expr,
            path_expr,
            indent,
            &format!(
                "must be one of [{}], got ${{JSON.stringify({value_expr})}}",
                literals.join(", ")
            ),
        );
    }
}

fn has_materialized_wire_constraints(schema: &Schema, include_format: bool) -> bool {
    schema.effective_min_length().is_some()
        || schema.max_length.is_some()
        || schema.pattern.is_some()
        || include_format && schema.format.is_some()
        || schema.const_value.is_some()
        || schema.enum_values.is_some()
}

/// Emits the per-field constraint checks over an in-memory `value_expr` for the
/// serialize path, reusing the same emitters as the parse path (numeric /
/// string-length / pattern / format / array / enum / const). References,
/// temporal, and contentEncoding carry no serialize-side field check here
/// (nested converters validate their own values; materialized reprs re-encode
/// losslessly). An **inline** `oneOf` sum type narrows to the branch it holds and
/// runs that branch's own checks; a `$ref` to a named union validates through the
/// union's converter instead.
fn render_ts_field_checks(
    output: &mut String,
    schema: &Schema,
    value_expr: &str,
    path_expr: &str,
    indent: &str,
) {
    render_ts_field_checks_at_depth(output, schema, value_expr, path_expr, indent, 0, true);
}

fn ts_serialize_type_check(schema: &Schema, value_expr: &str) -> Option<(String, &'static str)> {
    if let Some(kind) = temporal_kind_direct(schema) {
        let native = ts_temporal_type(kind, active_repr());
        let predicate = match native {
            "string" => format!("typeof {value_expr} === 'string'"),
            "Date" => format!("{value_expr} instanceof Date"),
            _ => format!("{value_expr} instanceof {native}"),
        };
        return Some((
            predicate,
            match kind {
                TemporalKind::DateTime => "expected date-time",
                TemporalKind::Date => "expected date",
                TemporalKind::Time => "expected time",
                TemporalKind::Duration => "expected duration",
            },
        ));
    }
    if content_encoding_direct(schema).is_some() {
        return Some((
            format!("{value_expr} instanceof Uint8Array"),
            "expected bytes",
        ));
    }
    match schema.ty.as_ref().and_then(Value::as_str) {
        Some("string") => Some((
            format!("typeof {value_expr} === 'string'"),
            "expected string",
        )),
        Some("boolean") => Some((
            format!("typeof {value_expr} === 'boolean'"),
            "expected boolean",
        )),
        Some("number") => Some((
            format!("typeof {value_expr} === 'number'"),
            "expected number",
        )),
        Some("integer") => Some((
            format!("typeof {value_expr} === 'number' && Number.isInteger({value_expr})"),
            "expected integer",
        )),
        Some("array") => Some((format!("Array.isArray({value_expr})"), "expected array")),
        _ => None,
    }
}

fn render_ts_field_checks_at_depth(
    output: &mut String,
    schema: &Schema,
    value_expr: &str,
    path_expr: &str,
    indent: &str,
    depth: usize,
    check_type: bool,
) {
    // A nullability wrapper's constraints live on its non-null branch; the caller
    // has already guarded the value against `null`.
    if let Some(non_null) = nullable_non_null_schema(schema) {
        render_ts_field_checks_at_depth(
            output, non_null, value_expr, path_expr, indent, depth, check_type,
        );
        return;
    }
    if check_type && let Some((predicate, reason)) = ts_serialize_type_check(schema, value_expr) {
        let inner_indent = format!("{indent}  ");
        let mut body = String::new();
        render_ts_field_checks_at_depth(
            &mut body,
            schema,
            value_expr,
            path_expr,
            &inner_indent,
            depth,
            false,
        );
        output.push_str(indent);
        output.push_str(&format!("if (!({predicate})) {{\n"));
        output.push_str(indent);
        output.push_str(&format!(
            "  violations.push({{ path: {path_expr}, reason: '{reason}' }});\n"
        ));
        if body.is_empty() {
            output.push_str(indent);
            output.push_str("}\n");
        } else {
            output.push_str(indent);
            output.push_str("} else {\n");
            output.push_str(&body);
            output.push_str(indent);
            output.push_str("}\n");
        }
        return;
    }
    if let Some(kind) = temporal_kind_direct(schema) {
        output.push_str(indent);
        output.push_str(&format!(
            "{}({value_expr}, {path_expr}, violations);\n",
            ts_temporal_validate_fn(kind)
        ));
        if has_materialized_wire_constraints(schema, false) {
            output.push_str(indent);
            output.push_str("{\n");
            let inner = format!("{indent}  ");
            let wire = ts_temporal_serialize_call(kind, active_repr(), value_expr);
            output.push_str(&inner);
            output.push_str(&format!("const wire = {wire};\n"));
            output.push_str(&inner);
            output.push_str("if (wire !== undefined) {\n");
            let check_indent = format!("{inner}  ");
            render_ts_string_assertions(output, "wire", path_expr, schema, &check_indent, false);
            render_ts_schema_closed_check(output, schema, "wire", path_expr, &check_indent);
            output.push_str(&inner);
            output.push_str("}\n");
            output.push_str(indent);
            output.push_str("}\n");
        }
        return;
    }
    if let Some(encoding) = content_encoding_direct(schema) {
        if has_materialized_wire_constraints(schema, true) {
            output.push_str(indent);
            output.push_str("{\n");
            let inner = format!("{indent}  ");
            output.push_str(&inner);
            output.push_str(&format!(
                "const wire = {}({value_expr});\n",
                ts_content_encoding_serialize_fn(encoding)
            ));
            render_ts_string_assertions(output, "wire", path_expr, schema, &inner, true);
            render_ts_schema_closed_check(output, schema, "wire", path_expr, &inner);
            output.push_str(indent);
            output.push_str("}\n");
        }
        return;
    }
    if is_ts_union(schema) {
        // The union's branches were classified without model lookup: only a
        // non-object branch contributes a check, and those need no `$ref`
        // resolution.
        if let Some(union) = classify_ts_union(schema, &[]) {
            render_ts_union_value_checks(output, &union, value_expr, path_expr, indent);
        }
        return;
    }
    if let Some(const_value) = &schema.const_value {
        let literal =
            typescript_value_literal(const_value).unwrap_or_else(|_| "undefined".to_string());
        let reason = format!("must equal {literal}");
        render_ts_serialize_closed_check(
            output,
            std::slice::from_ref(&literal),
            value_expr,
            path_expr,
            indent,
            &reason,
        );
        return;
    }
    if let Some(values) = &schema.enum_values {
        let literals = values
            .iter()
            .map(|value| {
                typescript_value_literal(value).unwrap_or_else(|_| "undefined".to_string())
            })
            .collect::<Vec<_>>();
        let reason = format!(
            "must be one of [{}], got ${{JSON.stringify({value_expr})}}",
            literals.join(", ")
        );
        render_ts_serialize_closed_check(output, &literals, value_expr, path_expr, indent, &reason);
        return;
    }
    match schema.ty.as_ref().and_then(Value::as_str) {
        Some("string") if schema.has_string_constraints() => {
            render_ts_string_checks(output, value_expr, path_expr, schema, indent);
        }
        Some("number") => {
            render_ts_numeric_checks(output, value_expr, path_expr, schema, indent, false);
        }
        Some("integer") => {
            render_ts_numeric_checks(output, value_expr, path_expr, schema, indent, true);
        }
        Some("array") => {
            if let Some(items) = schema.items.as_deref()
                && field_needs_serialize_check(items)
            {
                let suffix = ts_depth_suffix(depth);
                let element = format!("element{suffix}");
                let index = format!("index{suffix}");
                output.push_str(indent);
                output.push_str(&format!(
                    "{value_expr}.forEach(({element}, {index}) => {{\n"
                ));
                let item_path = ts_indexed_path(path_expr, &index);
                let item_indent = format!("{indent}  ");
                let check_indent = if allows_null(items) {
                    output.push_str(&item_indent);
                    output.push_str(&format!("if ({element} !== null) {{\n"));
                    format!("{item_indent}  ")
                } else {
                    item_indent.clone()
                };
                render_ts_field_checks_at_depth(
                    output,
                    items,
                    &element,
                    &item_path,
                    &check_indent,
                    depth + 1,
                    true,
                );
                if allows_null(items) {
                    output.push_str(&item_indent);
                    output.push_str("}\n");
                }
                output.push_str(indent);
                output.push_str("});\n");
            }
            if schema.has_array_constraints() {
                render_ts_array_checks(output, value_expr, path_expr, schema, indent, depth, true);
            }
        }
        _ => {}
    }
}

/// Emits the serialize-side validation for one typed-map member against its
/// in-memory value, keyed by the member's own key. The member counterpart of
/// [`render_ts_serialize_property_check`]: same predicates, same nullable guard
/// ([[additionalProperties]] §"Serialize-side" — a catch-all mutated to an
/// invalid value fails serialization rather than emitting bad data).
fn render_ts_member_check(
    output: &mut String,
    value_schema: &Schema,
    value_expr: &str,
    path_expr: &str,
    indent: &str,
) {
    let guard_null = allows_null(value_schema);
    let body_indent = if guard_null {
        format!("{indent}  ")
    } else {
        indent.to_string()
    };
    let mut body = String::new();
    render_ts_field_checks(&mut body, value_schema, value_expr, path_expr, &body_indent);
    if body.is_empty() {
        return;
    }
    if guard_null {
        output.push_str(indent);
        output.push_str(&format!("if ({value_expr} !== null) {{\n"));
        output.push_str(&body);
        output.push_str(indent);
        output.push_str("}\n");
    } else {
        output.push_str(&body);
    }
}

/// Emits the serialize-side validation for one declared property against its
/// in-memory value (`value.<field>`), guarding a nullable value so the checks
/// only fire on a materialized (non-null) value. The caller is responsible for
/// the optional (`!== undefined`) guard.
fn render_ts_serialize_property_check(
    output: &mut String,
    json_name: &str,
    property: &Schema,
    indent: &str,
) {
    let field_name = property.ts_member_name(json_name);
    let value_expr = format!("value.{field_name}");
    let path_expr = typescript_violation_path_literal(json_name);
    let guard_null = allows_null(property);
    let body_indent = if guard_null {
        format!("{indent}  ")
    } else {
        indent.to_string()
    };
    let mut body = String::new();
    render_ts_field_checks(&mut body, property, &value_expr, &path_expr, &body_indent);
    if body.is_empty() {
        return;
    }
    if guard_null {
        output.push_str(indent);
        output.push_str(&format!("if ({value_expr} !== null) {{\n"));
        output.push_str(&body);
        output.push_str(indent);
        output.push_str("}\n");
    } else {
        output.push_str(&body);
    }
}

fn push_indented(output: &mut String, body: &str, indent: &str) {
    for line in body.lines() {
        output.push_str(indent);
        output.push_str(line);
        output.push('\n');
    }
}

#[derive(Debug, Default)]
pub(in crate::generator) struct ModelBackend {
    json_models: Vec<PlannedJsonType>,
    tree_leaf: bool,
    runtime_import_module: String,
    /// The `--date-time-types` selection for materialized temporal fields.
    pub(in crate::generator) ts_date_time_types: crate::generator::TsDateTimeTypes,
    /// Resolved emitted identifiers (with `x-ts-name` overrides applied).
    manifest: NameManifest,
    /// Resolved type names keyed by `full_name` and the `#/$defs/<full_name>` ref form.
    ref_names: BTreeMap<String, String>,
}

impl ModelBackend {
    /// A model's emitted type identifier, resolved through the name manifest so
    /// an `x-ts-name` override applies. Every reference the backend answers has
    /// to come back through the manifest: `prepare` rewrites `model_name` only on
    /// the clones this backend renders, while the plan hands operations (and
    /// fields) their own clones still carrying the pre-override derived name.
    /// A model declared in another module is absent from this leaf's manifest and
    /// keeps its planned name.
    fn resolved_model_name(&self, json_type: &PlannedJsonType) -> String {
        self.manifest
            .type_name(&json_type.full_name)
            .unwrap_or(json_type.model_name.as_str())
            .to_string()
    }

    /// The identifier of the model's exported `TransferTypeConverter` instance,
    /// which the operation type info and the cross-module value imports name.
    pub(in crate::generator) fn transfer_type_converter(
        &self,
        json_type: &PlannedJsonType,
    ) -> String {
        ts_transfer_type_converter_name(&self.resolved_model_name(json_type))
    }

    pub(in crate::generator) fn render_support(&self) -> Result<RenderedTypeScriptSupport> {
        Ok(rendered_support(self.render_support_files()?))
    }
}

impl ExternalModelBackend<PlannedJsonType> for ModelBackend {
    type ModelFragments = RenderedExternalModelFragments;
    type WireConversion = WireValueConversion;

    fn prepare(&mut self, api_plan: &PlannedSpec) -> Result<()> {
        self.tree_leaf = !api_plan.module_path.is_root();
        self.runtime_import_module = if self.tree_leaf {
            root_typescript_runtime_module(&api_plan.module_path)
        } else {
            "./definitions".to_string()
        };
        // Resolve every emitted identifier once (overrides applied), then adopt the
        // resolved type name as each model's `model_name` so every downstream
        // derivation (interface/type decl, converter const) follows the same
        // identifier. `$ref` targets are resolved via `ref_names` below, and
        // references handed in from outside the backend via
        // [`ModelBackend::resolved_model_name`].
        self.manifest = build_json_name_manifest(Language::TypeScript, api_plan)?;
        self.json_models = api_plan
            .external_types()
            .map(|(_, binding)| binding)
            .filter_map(|binding| binding.json_model().cloned())
            .map(|mut json_type| {
                if let Some(resolved) = self.manifest.type_name(&json_type.full_name) {
                    json_type.model_name = resolved.to_string();
                }
                json_type
            })
            .collect();
        self.ref_names.clear();
        register_cross_module_ref_names(api_plan, &mut self.ref_names);
        for model in &self.json_models {
            // A resolved `$ref` is `#/$defs/<full_name>`; register that form (plus the
            // bare `full_name`) so `reference_model_name` resolves through the manifest
            // instead of recasing the ref segment (which would drop a type override).
            self.ref_names
                .insert(model.full_name.clone(), model.model_name.clone());
            self.ref_names.insert(
                format!("#/$defs/{}", model.full_name),
                model.model_name.clone(),
            );
        }
        Ok(())
    }

    fn render_models(&self) -> Result<RenderedExternalModelFragments> {
        set_ref_names(&self.ref_names);
        let json_models = self.json_models.iter().collect::<Vec<_>>();
        set_temporal_context(
            self.ts_date_time_types,
            self.json_models.iter().any(|m| model_uses_temporal(m)),
        );
        set_content_encoding_context(self.json_models.iter().any(model_uses_content_encoding));
        set_code_point_length_context(self.json_models.iter().any(model_uses_code_point_length));
        render_external_models(json_models.as_slice(), &self.runtime_import_module)
    }

    fn render_support_files(&self) -> Result<BTreeMap<PathBuf, String>> {
        if self.tree_leaf || self.json_models.is_empty() {
            return Ok(BTreeMap::new());
        }
        set_temporal_context(
            self.ts_date_time_types,
            self.json_models.iter().any(|m| model_uses_temporal(m)),
        );
        set_content_encoding_context(self.json_models.iter().any(model_uses_content_encoding));
        set_code_point_length_context(self.json_models.iter().any(model_uses_code_point_length));

        Ok(BTreeMap::from([(
            PathBuf::from("definitions.ts"),
            render_json_runtime_module(),
        )]))
    }

    fn model_type_annotation(&self, json_type: &PlannedJsonType) -> Option<String> {
        Some(self.resolved_model_name(json_type))
    }

    fn wire_type_identifier(&self, json_type: &PlannedJsonType) -> Option<String> {
        Some(json_type.full_name.clone())
    }

    fn wire_conversion(
        &self,
        json_type: &PlannedJsonType,
        _planned_record: Option<&RecordSpec<PlannedFamily>>,
    ) -> Option<WireValueConversion> {
        Some(WireValueConversion {
            annotation: self.resolved_model_name(json_type),
            from_wire: "{wire}".to_string(),
            to_wire: "{value}".to_string(),
            function_name_to_wire: None,
            wire_function_names: None,
            uses_rendered_model_annotation: false,
        })
    }
}

pub(in crate::generator) fn render_tree_support(
    branch: &crate::spec::ApiSpecBranch<PlannedFamily>,
) -> RenderedTypeScriptSupport {
    if !branch_has_json_models(branch) {
        return RenderedTypeScriptSupport::default();
    }

    rendered_support(BTreeMap::from([(
        PathBuf::from("definitions.ts"),
        render_json_runtime_module(),
    )]))
}

fn rendered_support(files: BTreeMap<PathBuf, String>) -> RenderedTypeScriptSupport {
    let root_exports = if files.is_empty() {
        Vec::new()
    } else {
        vec!["export type { Violation } from './definitions';".to_string()]
    };
    RenderedTypeScriptSupport {
        files,
        root_exports,
    }
}

fn branch_has_json_models(branch: &crate::spec::ApiSpecBranch<PlannedFamily>) -> bool {
    branch.children.values().any(|node| match node {
        crate::spec::ApiSpecNode::Leaf(leaf) => leaf
            .spec
            .external_types()
            .map(|(_, binding)| binding)
            .any(|binding| binding.json_model().is_some()),
        crate::spec::ApiSpecNode::Branch(branch) => branch_has_json_models(branch),
    })
}

fn root_typescript_runtime_module(module_path: &ModulePath) -> String {
    format!("{}definitions", "../".repeat(module_path.0.len()))
}

fn render_external_models(
    json_models: &[&PlannedJsonType],
    runtime_import_module: &str,
) -> Result<RenderedExternalModelFragments> {
    if json_models.is_empty() {
        return Ok(RenderedExternalModelFragments::default());
    }

    let mut output = String::new();
    render_default_constants(&mut output, json_models)?;
    render_pattern_regexes(&mut output, json_models)?;
    if !output.is_empty() {
        output.push('\n');
    }

    for model in json_models {
        output.push('\n');
        render_model_interface(&mut output, model)?;
    }

    render_ts_inline_union_serializers(&mut output, json_models)?;

    for model in converter_dependency_order(json_models)? {
        output.push('\n');
        render_model_transfer_type_converter(&mut output, model, json_models)?;
    }

    let uses_payload_validation_factory = json_models
        .iter()
        .any(|model| bare_ref_target(model).is_none());
    Ok(RenderedExternalModelFragments {
        imports: render_json_model_imports(
            runtime_import_module,
            &output,
            uses_payload_validation_factory,
        ),
        body: output,
        type_exported_names: json_models
            .iter()
            .map(|model| model.model_name.clone())
            .collect(),
        value_exported_names: json_models
            .iter()
            .map(|model| ts_transfer_type_converter_name(&model.model_name))
            .collect(),
    })
}

/// Orders converter value declarations by same-module bare-alias dependencies.
///
/// Type aliases themselves are hoisted by TypeScript, but the companion
/// converters are `const` values: `aConverter = zConverter` executes at module
/// initialization and cannot precede `zConverter`. Cross-module targets are
/// imports and therefore already initialized; ordinary models have no ordering
/// dependency here. The parser rejects alias cycles, while the defensive error
/// below keeps a malformed planned graph from becoming a TDZ at runtime.
fn converter_dependency_order<'a>(
    models: &[&'a PlannedJsonType],
) -> Result<Vec<&'a PlannedJsonType>> {
    let local_names = models
        .iter()
        .map(|model| model.model_name.as_str())
        .collect::<BTreeSet<_>>();
    let mut emitted = BTreeSet::<String>::new();
    let mut pending = models.to_vec();
    let mut ordered = Vec::with_capacity(models.len());

    while !pending.is_empty() {
        let Some(index) = pending.iter().position(|model| {
            let Some(reference) = bare_ref_target(model) else {
                return true;
            };
            let target = reference_model_name(reference);
            !local_names.contains(target.as_str()) || emitted.contains(&target)
        }) else {
            return Err(Error::InvalidJsonSchema {
                path: PathBuf::from("<json-generator>"),
                reason: format!(
                    "bare-`$ref` alias converter cycle among {}; break the alias cycle by making one declaration a concrete schema",
                    pending
                        .iter()
                        .map(|model| model.model_name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            });
        };
        let model = pending.remove(index);
        emitted.insert(model.model_name.clone());
        ordered.push(model);
    }
    Ok(ordered)
}

/// The namespace under which every generated `models.ts` imports its sibling
/// `definitions.ts` runtime module, so the generated file doesn't pollute its
/// own module namespace with generic names (`collect`, `isPlainObject`, …) that
/// could collide with user-authored identifiers.
const DEFINITIONS_NAMESPACE: &str = "__nexgenDefinitions";

fn render_json_model_imports(
    runtime_import_module: &str,
    body: &str,
    uses_payload_validation_factory: bool,
) -> String {
    let mut imports = String::new();
    // Every model gets a converter, so the SDK contract it implements is always
    // referenced. Type-only: nexus-rpc contributes no runtime code to
    // `models.ts`.
    imports.push_str("import type { TransferTypeConverter } from \"nexus-rpc\";\n");
    if uses_payload_validation_factory {
        imports.push_str("import { createPayloadValidationError } from \"@temporalio/common\";\n");
    }
    // Temporal-repr models reference the ambient global `Temporal.*` types
    // (TS 6's `esnext.temporal` lib) — no import required (P4).
    // `isPlainObject`/`Violation` are referenced by every generated model's
    // parser, so the import is always live.
    if body.contains(DEFINITIONS_NAMESPACE) {
        imports.push_str("import * as ");
        imports.push_str(DEFINITIONS_NAMESPACE);
        imports.push_str(" from \"");
        imports.push_str(runtime_import_module);
        imports.push_str("\";\n");
    }
    imports
}

fn render_json_runtime_module() -> String {
    let repr = active_repr();
    let uses_temporal = USES_TEMPORAL.with(Cell::get);
    let mut output = String::new();
    output.push_str(crate::generator::typescript::GENERATED_HEADER);
    output.push_str("\n\n");
    output.push_str("import { ApplicationFailure } from \"@temporalio/common\";\n\n");
    render_validator_core(&mut output);
    output.push('\n');
    render_collect_helper(&mut output);
    if USES_CODE_POINT_LENGTH.with(Cell::get) {
        output.push('\n');
        render_code_point_length_helper(&mut output);
    }
    if uses_temporal {
        output.push('\n');
        render_ts_temporal_helpers(&mut output, repr);
    }
    if USES_CONTENT_ENCODING.with(Cell::get) {
        output.push('\n');
        render_ts_content_encoding_helpers(&mut output);
    }
    output
}

/// True when any string position in the model asserts a code-point count — a
/// `minLength`/`maxLength` bound or a `format` with a length cap — anywhere the
/// emitters reach, including key subschemas and `contains` matchers.
fn schema_uses_code_point_length(schema: &Schema) -> bool {
    let direct = schema.max_length.is_some()
        || schema.effective_min_length().is_some()
        || schema
            .format
            .as_deref()
            .and_then(crate::json_schema::format::check_for)
            .is_some_and(|check| check.max_code_points.is_some());
    direct
        || schema
            .properties
            .iter()
            .flat_map(|entries| entries.values())
            .any(schema_uses_code_point_length)
        || schema
            .items
            .as_deref()
            .is_some_and(schema_uses_code_point_length)
        || schema
            .contains
            .as_deref()
            .is_some_and(schema_uses_code_point_length)
        || schema
            .property_names
            .as_deref()
            .is_some_and(schema_uses_code_point_length)
        || schema
            .one_of
            .iter()
            .flatten()
            .any(schema_uses_code_point_length)
        || schema
            .additional_properties
            .as_ref()
            .and_then(Value::as_object)
            .and_then(|value| serde_json::from_value::<Schema>(Value::Object(value.clone())).ok())
            .as_ref()
            .is_some_and(schema_uses_code_point_length)
}

fn model_uses_code_point_length(model: &PlannedJsonType) -> bool {
    decode_schema(model).is_ok_and(|schema| schema_uses_code_point_length(&schema))
}

fn schema_uses_content_encoding(schema: &Schema) -> bool {
    content_encoding_direct(schema).is_some()
        || schema
            .properties
            .as_ref()
            .is_some_and(|properties| properties.values().any(schema_uses_content_encoding))
        || schema
            .items
            .as_deref()
            .is_some_and(schema_uses_content_encoding)
        || schema
            .one_of
            .as_ref()
            .is_some_and(|branches| branches.iter().any(schema_uses_content_encoding))
        || schema
            .additional_properties
            .as_ref()
            .and_then(Value::as_object)
            .and_then(|value| serde_json::from_value::<Schema>(Value::Object(value.clone())).ok())
            .as_ref()
            .is_some_and(schema_uses_content_encoding)
}

/// True when any materialized position in the model uses `contentEncoding`.
fn model_uses_content_encoding(model: &PlannedJsonType) -> bool {
    let Ok(schema) = decode_schema(model) else {
        return false;
    };
    schema_uses_content_encoding(&schema)
}

/// Emits the generator-owned pure-JS `contentEncoding` codec: the pinned
/// canonical regexes (the validity oracle) plus base64 / base64url decode +
/// canonical encode over a `Uint8Array`, using a lookup table and plain
/// arithmetic — **no `Buffer`, no `atob`/`btoa`** — so the generated TS runs
/// unchanged in the browser and Node (P4). See `contentEncoding.md`.
fn render_ts_content_encoding_helpers(output: &mut String) {
    use crate::json_schema::content_encoding::Encoding;
    output.push_str(&format!(
        "const BASE64_RE = /{}/u;\n",
        Encoding::Base64.pattern()
    ));
    output.push_str(&format!(
        "const BASE64URL_RE = /{}/u;\n",
        Encoding::Base64Url.pattern()
    ));
    output.push_str(TS_CONTENT_ENCODING_BODY);
}

const TS_CONTENT_ENCODING_BODY: &str = r####"const BASE64_ALPHABET = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
const BASE64URL_ALPHABET = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_';

function buildDecodeTable(alphabet: string): Int16Array {
  const table = new Int16Array(128).fill(-1);
  for (let i = 0; i < alphabet.length; i++) {
    table[alphabet.charCodeAt(i)] = i;
  }
  return table;
}

const BASE64_DECODE = buildDecodeTable(BASE64_ALPHABET);
const BASE64URL_DECODE = buildDecodeTable(BASE64URL_ALPHABET);

// decodeCanonical assumes the input has already matched the canonical regex, so
// every non-padding character is a valid alphabet member. Padding (`=`) is
// stripped by the caller-supplied `stripped` length.
function decodeCanonical(value: string, table: Int16Array): Uint8Array {
  let stripped = value.length;
  while (stripped > 0 && value.charCodeAt(stripped - 1) === 0x3d /* '=' */) {
    stripped--;
  }
  const byteLength = Math.floor((stripped * 6) / 8);
  const out = new Uint8Array(byteLength);
  let accumulator = 0;
  let bits = 0;
  let outIndex = 0;
  for (let i = 0; i < stripped; i++) {
    accumulator = (accumulator << 6) | table[value.charCodeAt(i)];
    bits += 6;
    if (bits >= 8) {
      bits -= 8;
      out[outIndex++] = (accumulator >> bits) & 0xff;
    }
  }
  return out;
}

function encodeCanonical(bytes: Uint8Array, alphabet: string, pad: boolean): string {
  let out = '';
  let accumulator = 0;
  let bits = 0;
  for (let i = 0; i < bytes.length; i++) {
    accumulator = (accumulator << 8) | bytes[i];
    bits += 8;
    while (bits >= 6) {
      bits -= 6;
      out += alphabet[(accumulator >> bits) & 0x3f];
    }
  }
  if (bits > 0) {
    out += alphabet[(accumulator << (6 - bits)) & 0x3f];
  }
  if (pad) {
    while (out.length % 4 !== 0) {
      out += '=';
    }
  }
  return out;
}

export function base64ToBytes(value: string, path: string, violations: Violation[]): Uint8Array | undefined {
  if (!BASE64_RE.test(value)) {
    violations.push({ path, reason: `must be base64-encoded, got ${JSON.stringify(value)}` });
    return undefined;
  }
  return decodeCanonical(value, BASE64_DECODE);
}

export function bytesToBase64(bytes: Uint8Array): string {
  return encodeCanonical(bytes, BASE64_ALPHABET, true);
}

export function base64UrlToBytes(value: string, path: string, violations: Violation[]): Uint8Array | undefined {
  if (!BASE64URL_RE.test(value)) {
    violations.push({ path, reason: `must be base64url-encoded, got ${JSON.stringify(value)}` });
    return undefined;
  }
  return decodeCanonical(value, BASE64URL_DECODE);
}

export function bytesToBase64Url(bytes: Uint8Array): string {
  return encodeCanonical(bytes, BASE64URL_ALPHABET, false);
}
"####;

/// `Temporal.ZonedDateTime` caps at nanoseconds and its ISO parser **throws**
/// past nine fractional digits. Go and Java truncate at nine, Python at six, and
/// TypeScript's `string` repr keeps the wire verbatim (it has no capacity to
/// lose), so `temporal` truncates rather than rejecting: a uniform reject would
/// split the accept set, which the bounded-loss P1 exception (b) does not cover.
/// Mirrors Java's `truncateFraction`. See `specs/json-schema/features/format.md`.
const TS_TEMPORAL_FRACTION_TRUNCATION: &str = r####"function truncateTemporalFraction(value: string): string {
  const dot = value.indexOf('.');
  if (dot < 0) {
    return value;
  }
  let end = dot + 1;
  while (end < value.length && value[end] >= '0' && value[end] <= '9') {
    end++;
  }
  return end - dot - 1 <= 9 ? value : value.slice(0, dot + 10) + value.slice(end);
}

"####;

fn schema_uses_temporal(schema: &Schema) -> bool {
    temporal_kind_direct(schema).is_some()
        || schema
            .properties
            .as_ref()
            .is_some_and(|properties| properties.values().any(schema_uses_temporal))
        || schema.items.as_deref().is_some_and(schema_uses_temporal)
        || schema
            .one_of
            .as_ref()
            .is_some_and(|branches| branches.iter().any(schema_uses_temporal))
        || schema
            .additional_properties
            .as_ref()
            .and_then(Value::as_object)
            .and_then(|value| serde_json::from_value::<Schema>(Value::Object(value.clone())).ok())
            .as_ref()
            .is_some_and(schema_uses_temporal)
}

/// True when any materialized position in the model uses a temporal `format`.
fn model_uses_temporal(model: &PlannedJsonType) -> bool {
    let Ok(schema) = decode_schema(model) else {
        return false;
    };
    schema_uses_temporal(&schema)
}

/// Emits the materialized-temporal runtime for the active repr: the pinned
/// narrowed regexes, the Gregorian calendar predicate, string canonicalizers,
/// and the parse/serialize adapters. See `specs/json-schema/features/format.md`.
fn render_ts_temporal_helpers(output: &mut String, repr: TsDateTimeTypes) {
    output.push_str(&format!(
        "const TEMPORAL_DATE_TIME_RE = /{}/u;\n",
        TemporalKind::DateTime.pattern()
    ));
    output.push_str(&format!(
        "const TEMPORAL_DATE_RE = /{}/u;\n",
        TemporalKind::Date.pattern()
    ));
    output.push_str(&format!(
        "const TEMPORAL_TIME_RE = /{}/u;\n",
        TemporalKind::Time.pattern()
    ));
    output.push_str(&format!(
        "const TEMPORAL_DURATION_RE = /{}/u;\n",
        TemporalKind::Duration.pattern()
    ));
    output.push_str(TS_TEMPORAL_COMMON);
    if repr == TsDateTimeTypes::Temporal {
        output.push_str(TS_TEMPORAL_FRACTION_TRUNCATION);
    }

    let dt_type = ts_temporal_type(TemporalKind::DateTime, repr);
    let date_type = ts_temporal_type(TemporalKind::Date, repr);
    let dur_type = ts_temporal_type(TemporalKind::Duration, repr);

    // parseTemporalDateTime
    output.push_str(&format!(
        "export function parseTemporalDateTime(value: string, path: string, violations: Violation[]): {dt_type} | undefined {{\n"
    ));
    output
        .push_str("  if (!TEMPORAL_DATE_TIME_RE.test(value) || !validTemporalCalendar(value)) {\n");
    output.push_str("    violations.push({ path, reason: `must be a valid date-time, got ${JSON.stringify(value)}` });\n    return undefined;\n  }\n");
    match repr {
        TsDateTimeTypes::String => {
            output.push_str("  return canonicalizeTemporalDateTime(value);\n");
        }
        TsDateTimeTypes::Date => {
            output.push_str("  return new Date(value.toUpperCase());\n");
        }
        TsDateTimeTypes::Temporal => {
            output.push_str(
                "  const canon = truncateTemporalFraction(canonicalizeTemporalDateTime(value));\n",
            );
            output.push_str("  const zone = canon.endsWith('Z') ? 'UTC' : canon.slice(-6);\n");
            // The pinned regex plus `validTemporalCalendar` already admit only
            // values `Temporal` accepts, but a constructor that throws must never
            // escape the converter as a bare `RangeError` (P11): every rejection
            // is a `Violation` in the aggregated payload-validation failure.
            output.push_str("  try {\n");
            output.push_str("    return Temporal.ZonedDateTime.from(`${canon}[${zone}]`);\n");
            output.push_str("  } catch {\n");
            output.push_str("    violations.push({ path, reason: `must be a valid date-time, got ${JSON.stringify(value)}` });\n");
            output.push_str("    return undefined;\n");
            output.push_str("  }\n");
        }
    }
    output.push_str("}\n\n");

    // parseTemporalDate
    output.push_str(&format!(
        "export function parseTemporalDate(value: string, path: string, violations: Violation[]): {date_type} | undefined {{\n"
    ));
    output.push_str("  if (!TEMPORAL_DATE_RE.test(value) || !validTemporalCalendar(value)) {\n");
    output.push_str("    violations.push({ path, reason: `must be a valid date, got ${JSON.stringify(value)}` });\n    return undefined;\n  }\n");
    match repr {
        TsDateTimeTypes::Temporal => {
            output.push_str("  try {\n");
            output.push_str("    return Temporal.PlainDate.from(value);\n");
            output.push_str("  } catch {\n");
            output.push_str("    violations.push({ path, reason: `must be a valid date, got ${JSON.stringify(value)}` });\n");
            output.push_str("    return undefined;\n");
            output.push_str("  }\n");
        }
        _ => output.push_str("  return value;\n"),
    }
    output.push_str("}\n\n");

    // parseTemporalTime — always a string.
    output.push_str(
        "export function parseTemporalTime(value: string, path: string, violations: Violation[]): string | undefined {\n",
    );
    output.push_str("  if (!TEMPORAL_TIME_RE.test(value)) {\n");
    output.push_str("    violations.push({ path, reason: `must be a valid time, got ${JSON.stringify(value)}` });\n    return undefined;\n  }\n");
    output.push_str("  return canonicalizeTemporalTime(value);\n}\n\n");

    // parseTemporalDuration
    output.push_str(&format!(
        "export function parseTemporalDuration(value: string, path: string, violations: Violation[]): {dur_type} | undefined {{\n"
    ));
    output.push_str("  if (!TEMPORAL_DURATION_RE.test(value)) {\n");
    output.push_str("    violations.push({ path, reason: `must be a valid duration, got ${JSON.stringify(value)}` });\n    return undefined;\n  }\n");
    output.push_str("  const seconds = temporalDurationSeconds(value);\n");
    output.push_str("  if (seconds === undefined) {\n");
    output.push_str("    violations.push({ path, reason: `must be a valid duration, got ${JSON.stringify(value)}` });\n    return undefined;\n  }\n");
    match repr {
        TsDateTimeTypes::Temporal => {
            output.push_str("  return Temporal.Duration.from({ seconds });\n");
        }
        _ => output.push_str("  return formatTemporalDuration(seconds);\n"),
    }
    output.push_str("}\n");

    // Serializers for native-typed reprs.
    if repr == TsDateTimeTypes::Date {
        output.push_str(
            "\nexport function serializeTemporalDateTime(value: Date): string {\n  return value.toISOString();\n}\n",
        );
    }
    if repr == TsDateTimeTypes::Temporal {
        output.push_str(
            "\nexport function serializeTemporalDateTime(value: Temporal.ZonedDateTime): string {\n",
        );
        output.push_str("  const s = value.toString({ timeZoneName: 'never' });\n");
        output.push_str(
            "  return s.endsWith('+00:00') || s.endsWith('-00:00') ? s.slice(0, -6) + 'Z' : s;\n}\n",
        );
        output.push_str(
            "\nexport function serializeTemporalDate(value: Temporal.PlainDate): string {\n  return value.toString();\n}\n",
        );
        output.push_str(
            "\nexport function serializeTemporalDuration(value: Temporal.Duration): string {\n  return formatTemporalDuration(value.total({ unit: 'seconds' }));\n}\n",
        );
    }

    let date_time_wire = match repr {
        TsDateTimeTypes::String => "value",
        TsDateTimeTypes::Date => "serializeTemporalDateTime(value)",
        TsDateTimeTypes::Temporal => "serializeTemporalDateTime(value)",
    };
    output.push_str(&format!(
        "\nexport function validateTemporalDateTime(value: {dt_type}, path: string, violations: Violation[]): void {{\n"
    ));
    if repr == TsDateTimeTypes::Date {
        output.push_str("  if (!Number.isFinite(value.getTime())) {\n    violations.push({ path, reason: `must be a valid date-time, got ${JSON.stringify(value)}` });\n    return;\n  }\n");
    }
    output.push_str(&format!("  const wire = {date_time_wire};\n"));
    output.push_str("  if (!TEMPORAL_DATE_TIME_RE.test(wire) || !validTemporalCalendar(wire)) {\n    violations.push({ path, reason: `must be a valid date-time, got ${JSON.stringify(wire)}` });\n  }\n}\n");

    let date_wire = if repr == TsDateTimeTypes::Temporal {
        "serializeTemporalDate(value)"
    } else {
        "value"
    };
    output.push_str(&format!(
        "\nexport function validateTemporalDate(value: {date_type}, path: string, violations: Violation[]): void {{\n  const wire = {date_wire};\n"
    ));
    output.push_str("  if (!TEMPORAL_DATE_RE.test(wire) || !validTemporalCalendar(wire)) {\n    violations.push({ path, reason: `must be a valid date, got ${JSON.stringify(wire)}` });\n  }\n}\n");

    output.push_str("\nexport function validateTemporalTime(value: string, path: string, violations: Violation[]): void {\n  if (!TEMPORAL_TIME_RE.test(value)) {\n    violations.push({ path, reason: `must be a valid time, got ${JSON.stringify(value)}` });\n  }\n}\n");

    let duration_wire = if repr == TsDateTimeTypes::Temporal {
        "serializeTemporalDuration(value)"
    } else {
        "value"
    };
    output.push_str(&format!(
        "\nexport function validateTemporalDuration(value: {dur_type}, path: string, violations: Violation[]): void {{\n  const wire = {duration_wire};\n"
    ));
    output.push_str("  if (!TEMPORAL_DURATION_RE.test(wire) || temporalDurationSeconds(wire) === undefined) {\n    violations.push({ path, reason: `must be a valid duration, got ${JSON.stringify(wire)}` });\n  }\n}\n");
}

const TS_TEMPORAL_COMMON: &str = r#"const TEMPORAL_DATE_TIME_CAP = /^(\d{4}-\d{2}-\d{2})[Tt](\d{2}:\d{2}:\d{2})(\.\d+)?([Zz]|[+-]\d{2}:\d{2})$/u;
const TEMPORAL_TIME_CAP = /^(\d{2}:\d{2}:\d{2})(\.\d+)?([Zz]|[+-]\d{2}:\d{2})?$/u;

function daysInTemporalMonth(year: number, month: number): number {
  switch (month) {
    case 1: case 3: case 5: case 7: case 8: case 10: case 12:
      return 31;
    case 4: case 6: case 9: case 11:
      return 30;
    case 2:
      return (year % 4 === 0 && year % 100 !== 0) || year % 400 === 0 ? 29 : 28;
    default:
      return 0;
  }
}

function validTemporalCalendar(value: string): boolean {
  if (value.length < 10) {
    return false;
  }
  const year = Number(value.slice(0, 4));
  const month = Number(value.slice(5, 7));
  const day = Number(value.slice(8, 10));
  if (year < 1) {
    return false;
  }
  const max = daysInTemporalMonth(year, month);
  return max > 0 && day >= 1 && day <= max;
}

function trimTemporalFraction(fraction: string | undefined): string {
  if (!fraction) {
    return '';
  }
  const trimmed = fraction.replace(/0+$/u, '');
  return trimmed === '.' ? '' : trimmed;
}

function normalizeTemporalOffset(offset: string): string {
  if (offset.toUpperCase() === 'Z' || offset === '+00:00' || offset === '-00:00') {
    return 'Z';
  }
  return offset;
}

function canonicalizeTemporalDateTime(value: string): string {
  const m = TEMPORAL_DATE_TIME_CAP.exec(value)!;
  return `${m[1]}T${m[2]}${trimTemporalFraction(m[3])}${normalizeTemporalOffset(m[4])}`;
}

function canonicalizeTemporalTime(value: string): string {
  const m = TEMPORAL_TIME_CAP.exec(value)!;
  return `${m[1]}${trimTemporalFraction(m[2])}${m[3] ? normalizeTemporalOffset(m[3]) : ''}`;
}

function temporalDurationSeconds(value: string): number | undefined {
  let total = 0;
  let digits = '';
  for (const ch of value.slice(2)) {
    if (ch >= '0' && ch <= '9') {
      digits += ch;
      continue;
    }
    const unit = ch === 'H' ? 3600 : ch === 'M' ? 60 : 1;
    total += Number(digits) * unit;
    digits = '';
    if (total > 9223372036) {
      return undefined;
    }
  }
  return total;
}

function formatTemporalDuration(total: number): string {
  if (total === 0) {
    return 'PT0S';
  }
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const seconds = total % 60;
  let out = 'PT';
  if (hours) {
    out += `${hours}H`;
  }
  if (minutes) {
    out += `${minutes}M`;
  }
  if (seconds) {
    out += `${seconds}S`;
  }
  return out;
}

"#;

fn render_validator_core(output: &mut String) {
    output.push_str("/** A single constraint failure, located by JSON path. */\n");
    output.push_str("export interface Violation {\n");
    output.push_str("  readonly path: string;\n");
    output.push_str("  readonly reason: string;\n");
    output.push_str("}\n\n");
    output.push_str(
        "export function isPlainObject(value: unknown): value is Record<string, unknown> {\n",
    );
    output.push_str(
        "  return typeof value === 'object' && value !== null && !Array.isArray(value);\n",
    );
    output.push_str("}\n");
}

/// True when a property's wire string is materialized into a native in-memory
/// value (a temporal `format` or a `contentEncoding`), directly or through a
/// nullability wrapper. Such a node's `const`/`enum` is compared against the
/// re-encoded wire string, not against a module-level literal.
fn is_materialized_property(property: &Schema) -> bool {
    temporal_kind_direct(property).is_some()
        || content_encoding_direct(property).is_some()
        || nullable_non_null_schema(property).is_some_and(|non_null| {
            temporal_kind_direct(non_null).is_some() || content_encoding_direct(non_null).is_some()
        })
}

fn render_default_constants(output: &mut String, models: &[&PlannedJsonType]) -> Result<()> {
    #[derive(Debug)]
    struct Constant {
        name: String,
        value: String,
        exported: bool,
    }

    let mut default_fields = Vec::new();
    let mut const_fields = Vec::new();
    for model in models {
        let schema = decode_schema(model)?;
        let Some(properties) = &schema.properties else {
            continue;
        };
        for (field_name, property) in properties {
            // The constant is named after the emitted member, not the JSON key,
            // so an `x-ts-name` override moves it too (P15).
            let member_ident = property.ts_member_name(field_name);
            if let Some(default) = &property.default {
                default_fields.push((
                    model.model_name.clone(),
                    member_ident.clone(),
                    typescript_default_literal(property, default)?,
                ));
            }
            // A materialized node inlines its literal at the comparison site
            // (`render_property_parser`), so declaring the constant would leave an
            // unreferenced binding occupying a P15 identifier slot.
            if let Some(const_value) = &property.const_value
                && !is_materialized_property(property)
            {
                const_fields.push((
                    model.model_name.clone(),
                    member_ident,
                    typescript_value_literal(const_value)?,
                ));
            }
        }
    }

    if default_fields.is_empty() && const_fields.is_empty() {
        return Ok(());
    }

    let mut constants = Vec::new();
    for (model_name, field_name, value) in const_fields {
        constants.push(Constant {
            name: const_const_name(
                &model_name,
                &field_name,
                models,
                ConstNameCollisionKind::Const,
            )?,
            value,
            exported: false,
        });
    }
    for (model_name, field_name, value) in default_fields {
        constants.push(Constant {
            name: default_const_name(
                &model_name,
                &field_name,
                models,
                ConstNameCollisionKind::Default,
            )?,
            value,
            exported: true,
        });
    }

    output.push('\n');
    for constant in constants {
        if constant.exported {
            output.push_str("export ");
        }
        output.push_str("const ");
        output.push_str(&constant.name);
        output.push_str(" = ");
        output.push_str(&constant.value);
        output.push_str(";\n");
    }
    Ok(())
}

/// Renders a default in the member's in-memory type. Ordinary scalar defaults
/// stay literals; temporal and content-encoded defaults pass through the same
/// generated materializer as a present wire value. Loader validation guarantees
/// these calls succeed, hence the non-null assertion.
fn typescript_default_literal(schema: &Schema, value: &Value) -> Result<String> {
    let schema = nullable_non_null_schema(schema).unwrap_or(schema);
    let wire = typescript_value_literal(value)?;
    if let Some(kind) = temporal_kind_direct(schema) {
        return Ok(format!("{}({wire}, '', [])!", ts_temporal_parse_fn(kind)));
    }
    if let Some(encoding) = content_encoding_direct(schema) {
        return Ok(format!(
            "{}({wire}, '', [])!",
            ts_content_encoding_parse_fn(encoding)
        ));
    }
    Ok(wire)
}

// ---------------------------------------------------------------------------
// `oneOf` closed sum types (specs/json-schema/features/oneOf.md)
// ---------------------------------------------------------------------------

/// A member of a TypeScript union (a native `A | B | …` type).
#[derive(Debug, Clone)]
struct TsUnionVariant {
    ts_type: String,
    is_object: bool,
    converter: Option<String>,
    discriminant_value: Option<Value>,
    typeof_guard: Option<&'static str>,
    is_integer: bool,
    is_array: bool,
    label: String,
    /// The branch's own schema, whose constraints the narrowed value is held to —
    /// a scalar/array branch has no mapper to carry them ([[oneOf]] §"Validator
    /// mapping").
    schema: Schema,
}

#[derive(Debug, Clone)]
struct TsUnion {
    nullable: bool,
    discriminant: Option<String>,
    variants: Vec<TsUnionVariant>,
}

impl TsUnion {
    fn admissible(&self) -> String {
        self.variants
            .iter()
            .map(|variant| variant.label.clone())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// True when a schema is a `oneOf` sum type (two or more non-null branches),
/// rather than the degenerate nullability pattern.
fn is_ts_union(schema: &Schema) -> bool {
    schema.one_of.as_ref().is_some_and(|branches| {
        branches
            .iter()
            .filter(|branch| !schema_type_includes(branch, "null"))
            .count()
            >= 2
    })
}

fn ts_discriminator_const(property: &Schema) -> Option<Value> {
    if let Some(value) = &property.const_value {
        return Some(value.clone());
    }
    if let Some(values) = &property.enum_values
        && values.len() == 1
    {
        return Some(values[0].clone());
    }
    None
}

fn ts_branch_discriminator_tags(object: &Schema) -> BTreeMap<String, Value> {
    let required: BTreeSet<String> = object.required.iter().flatten().cloned().collect();
    let mut tags = BTreeMap::new();
    if let Some(properties) = &object.properties {
        for (name, property) in properties {
            if required.contains(name)
                && let Some(value) = ts_discriminator_const(property)
            {
                tags.insert(name.clone(), value);
            }
        }
    }
    tags
}

/// Resolves a `$ref` branch to its target's decoded schema. A `$ref` is resolved
/// against the **whole input closure**, not just the current module's model list:
/// a cross-file branch is declared in another leaf, and dropping it silently
/// collapses a sum type to its first local branch (`ref.md` §"Type-name
/// derivation"). The local list is searched first so a module-local override
/// wins; the tree registry (populated once per generation from every leaf) backs
/// it up.
fn find_ref_schema(reference: &str, models: &[&PlannedJsonType]) -> Option<Schema> {
    let target = reference_model_name(reference);
    if let Some(model) = models
        .iter()
        .copied()
        .find(|model| model.model_name == target || model.full_name == target)
    {
        return decode_schema(model).ok();
    }
    let full_name = reference_full_name(reference);
    TREE_JSON_MODELS.with(|registry| {
        let registry = registry.borrow();
        registry
            .get(&full_name)
            .or_else(|| {
                registry
                    .values()
                    .find(|model| model.model_name == target || model.full_name == target)
            })
            .and_then(|model| decode_schema(model).ok())
    })
}

/// The `$defs`-qualified full name a `$ref` names, before the per-language name
/// manifest recases it.
fn reference_full_name(reference: &str) -> String {
    reference
        .rsplit_once("#/$defs/")
        .map(|(_, name)| name.to_string())
        .unwrap_or_else(|| reference.to_string())
}

/// The TypeScript type a **scalar** branch contributes to the union: the branch's
/// own annotation, so a `const`/`enum` branch narrows to the closed literal set it
/// declares (`"auto" | "manual"`) rather than the wider primitive — which the
/// narrowed assignment would not even typecheck against. `primitive` is the
/// fallback for a branch that declares nothing but its kind.
fn ts_scalar_branch_type(resolved: &Schema, primitive: &str) -> String {
    type_annotation(resolved).unwrap_or_else(|_| primitive.to_string())
}

/// Classifies a `oneOf` schema into a TypeScript union, or `None` for the
/// degenerate nullability pattern.
fn classify_ts_union(schema: &Schema, models: &[&PlannedJsonType]) -> Option<TsUnion> {
    if !is_ts_union(schema) {
        return None;
    }
    let branches = schema.one_of.as_ref()?;
    let mut nullable = false;
    let mut variants: Vec<TsUnionVariant> = Vec::new();
    let mut object_schemas: Vec<Schema> = Vec::new();
    for branch in branches {
        let resolved = if let Some(reference) = &branch.reference {
            find_ref_schema(reference, models).unwrap_or_else(|| branch.clone())
        } else {
            branch.clone()
        };
        let ty = resolved.ty.as_ref().and_then(Value::as_str);
        match ty {
            Some("null") => nullable = true,
            Some("object") => {
                // A `$ref` branch is the named model (parsed by its converter); an
                // inline branch is the free-form object (loader-enforced), so it
                // stays an anonymous `Record` carried verbatim — TS needs no
                // synthesized name to narrow on the object token.
                let (name, converter, label) = match &branch.reference {
                    Some(reference) => {
                        let name = reference_model_name(reference);
                        let converter = ts_transfer_type_converter_name(&name);
                        (name.clone(), Some(converter), name)
                    }
                    None => {
                        let value = ts_map_shape(&resolved)
                            .ok()
                            .flatten()
                            .map(|shape| shape.value_annotation)
                            .unwrap_or_else(|| "unknown".to_string());
                        (
                            format!("Record<string, {value}>"),
                            None,
                            "object".to_string(),
                        )
                    }
                };
                object_schemas.push(resolved.clone());
                variants.push(TsUnionVariant {
                    ts_type: name,
                    is_object: true,
                    converter,
                    discriminant_value: None,
                    typeof_guard: None,
                    is_integer: false,
                    is_array: false,
                    label,
                    schema: resolved.clone(),
                });
            }
            Some("string") => variants.push(TsUnionVariant {
                ts_type: ts_scalar_branch_type(&resolved, "string"),
                is_object: false,
                converter: None,
                discriminant_value: None,
                typeof_guard: Some("string"),
                is_integer: false,
                is_array: false,
                label: "string".to_string(),
                schema: resolved.clone(),
            }),
            Some("integer") => variants.push(TsUnionVariant {
                ts_type: ts_scalar_branch_type(&resolved, "number"),
                is_object: false,
                converter: None,
                discriminant_value: None,
                typeof_guard: Some("number"),
                is_integer: true,
                is_array: false,
                label: "integer".to_string(),
                schema: resolved.clone(),
            }),
            Some("number") => variants.push(TsUnionVariant {
                ts_type: ts_scalar_branch_type(&resolved, "number"),
                is_object: false,
                converter: None,
                discriminant_value: None,
                typeof_guard: Some("number"),
                is_integer: false,
                is_array: false,
                label: "number".to_string(),
                schema: resolved.clone(),
            }),
            Some("boolean") => variants.push(TsUnionVariant {
                ts_type: ts_scalar_branch_type(&resolved, "boolean"),
                is_object: false,
                converter: None,
                discriminant_value: None,
                typeof_guard: Some("boolean"),
                is_integer: false,
                is_array: false,
                label: "boolean".to_string(),
                schema: resolved.clone(),
            }),
            Some("array") => {
                let ts_type =
                    type_annotation(&resolved).unwrap_or_else(|_| "unknown[]".to_string());
                variants.push(TsUnionVariant {
                    ts_type: ts_type.clone(),
                    is_object: false,
                    converter: None,
                    discriminant_value: None,
                    typeof_guard: None,
                    is_integer: false,
                    is_array: true,
                    label: ts_type,
                    schema: resolved.clone(),
                });
            }
            _ => {}
        }
    }
    let mut discriminant = None;
    if object_schemas.len() >= 2 {
        let mut shared: Option<BTreeMap<String, Value>> = None;
        for object in &object_schemas {
            let tags = ts_branch_discriminator_tags(object);
            shared = Some(match shared {
                None => tags,
                Some(existing) => existing
                    .into_iter()
                    .filter(|(name, _)| tags.contains_key(name))
                    .collect(),
            });
        }
        let shared = shared.unwrap_or_default();
        let name = shared
            .keys()
            .find(|name| {
                let values: Vec<Value> = object_schemas
                    .iter()
                    .filter_map(|object| ts_branch_discriminator_tags(object).get(*name).cloned())
                    .collect();
                values
                    .iter()
                    .enumerate()
                    .all(|(index, value)| !values[..index].iter().any(|existing| existing == value))
            })
            .cloned();
        if let Some(name) = &name {
            let mut object_index = 0;
            for variant in variants.iter_mut().filter(|variant| variant.is_object) {
                variant.discriminant_value =
                    ts_branch_discriminator_tags(&object_schemas[object_index])
                        .get(name)
                        .cloned();
                object_index += 1;
            }
        }
        discriminant = name;
    }
    Some(TsUnion {
        nullable,
        discriminant,
        variants,
    })
}

/// Emits the parse dispatch for a union value: token / discriminant selection
/// into `target`, pushing a `Violation` when no branch matches.
fn render_ts_union_parse(
    output: &mut String,
    union: &TsUnion,
    raw_expr: &str,
    target: &str,
    path_expr: &str,
    indent: &str,
) {
    let mut first = true;
    let mut clause = |output: &mut String, condition: &str| {
        if first {
            output.push_str(indent);
            output.push_str(&format!("if ({condition}) {{\n"));
            first = false;
        } else {
            output.push_str(indent);
            output.push_str(&format!("}} else if ({condition}) {{\n"));
        }
    };

    let object_variants: Vec<&TsUnionVariant> = union
        .variants
        .iter()
        .filter(|variant| variant.is_object)
        .collect();
    if !object_variants.is_empty() {
        clause(
            output,
            &format!("{DEFINITIONS_NAMESPACE}.isPlainObject({raw_expr})"),
        );
        if let Some(discriminant) = &union.discriminant {
            let disc_key = format!(
                "({raw_expr} as Record<string, unknown>)[{}]",
                typescript_string_literal(discriminant)
            );
            output.push_str(indent);
            output.push_str(&format!("  switch ({disc_key}) {{\n"));
            let mut values_display = Vec::new();
            for variant in &object_variants {
                let Some(value) = &variant.discriminant_value else {
                    continue;
                };
                let literal =
                    typescript_value_literal(value).unwrap_or_else(|_| "undefined".to_string());
                values_display.push(literal.clone());
                output.push_str(indent);
                output.push_str(&format!("    case {literal}:\n"));
                output.push_str(indent);
                output.push_str("      try {\n");
                output.push_str(indent);
                output.push_str(&format!(
                    "        {target} = {}.fromTransferType({raw_expr});\n",
                    variant.converter.as_deref().unwrap_or("")
                ));
                output.push_str(indent);
                output.push_str("      } catch (error) {\n");
                output.push_str(indent);
                output.push_str(&format!(
                    "        {DEFINITIONS_NAMESPACE}.collect(violations, {path_expr}, error);\n"
                ));
                output.push_str(indent);
                output.push_str("      }\n");
                output.push_str(indent);
                output.push_str("      break;\n");
            }
            output.push_str(indent);
            output.push_str("    default:\n");
            output.push_str(indent);
            output.push_str(&format!(
                "      violations.push({{ path: {path_expr}, reason: `unknown discriminator {discriminant} ${{String({disc_key})}}: expected one of [{}]` }});\n",
                values_display.join(", ")
            ));
            output.push_str(indent);
            output.push_str("  }\n");
        } else {
            let variant = object_variants[0];
            match variant.converter.as_deref() {
                Some(converter) => {
                    output.push_str(indent);
                    output.push_str("  try {\n");
                    output.push_str(indent);
                    output.push_str(&format!(
                        "    {target} = {converter}.fromTransferType({raw_expr});\n"
                    ));
                    output.push_str(indent);
                    output.push_str("  } catch (error) {\n");
                    output.push_str(indent);
                    output.push_str(&format!(
                        "    {DEFINITIONS_NAMESPACE}.collect(violations, {path_expr}, error);\n"
                    ));
                    output.push_str(indent);
                    output.push_str("  }\n");
                }
                // An inline map-shaped branch has no converter: the wire object is
                // already the in-memory value.
                None => {
                    output.push_str(indent);
                    output.push_str(&format!(
                        "  {target} = {raw_expr} as {};\n",
                        variant.ts_type
                    ));
                }
            }
        }
    }

    for variant in union.variants.iter().filter(|variant| !variant.is_object) {
        if let Some(guard) = ts_variant_guard(variant, raw_expr) {
            clause(output, &guard);
        }
        if variant.is_array {
            // The array token only selects the branch. Its elements still take
            // the ordinary recursive parser so references, temporals, bytes,
            // nested arrays, and their indexed violations are preserved.
            let array_target = format!("{target}ArrayBranch");
            output.push_str(indent);
            output.push_str(&format!(
                "  let {array_target}: {} = undefined as unknown as {};\n",
                variant.ts_type, variant.ts_type
            ));
            render_array_parser(
                output,
                &variant.schema,
                raw_expr,
                &array_target,
                path_expr,
                &format!("{indent}  "),
                0,
            );
            output.push_str(indent);
            output.push_str(&format!(
                "  if ({array_target} !== undefined) {{\n    {target} = {array_target};\n  }}\n"
            ));
            continue;
        }
        output.push_str(indent);
        output.push_str(&format!(
            "  {target} = {raw_expr} as {};\n",
            variant.ts_type
        ));
        // The token has selected the branch; the value is now held to everything
        // the branch declares (P12 — the same predicates the property position
        // runs for a value of that type).
        render_ts_field_checks(
            output,
            &variant.schema,
            &format!("({target} as {})", variant.ts_type),
            path_expr,
            &format!("{indent}  "),
        );
    }

    if union.nullable {
        clause(output, &format!("{raw_expr} === null"));
        output.push_str(indent);
        output.push_str(&format!(
            "  {target} = null as unknown as {};\n",
            union.variants[0].ts_type
        ));
    }

    output.push_str(indent);
    output.push_str("} else {\n");
    output.push_str(indent);
    output.push_str(&format!(
        "  violations.push({{ path: {path_expr}, reason: 'expected one of: {}' }});\n",
        union.admissible()
    ));
    output.push_str(indent);
    output.push_str("}\n");
}

/// The narrowing guard that selects a non-object variant from a value of the
/// union type — the JSON token, expressed in TypeScript's own narrowing
/// primitives (`typeof` / `Array.isArray`). The same guard selects the branch on
/// the wire (parse) and in memory (serialize), because a scalar/array member *is*
/// its wire form.
fn ts_variant_guard(variant: &TsUnionVariant, value_expr: &str) -> Option<String> {
    if variant.is_array {
        return Some(format!("Array.isArray({value_expr})"));
    }
    if variant.is_integer {
        return Some(format!(
            "typeof {value_expr} === 'number' && Number.isSafeInteger({value_expr})"
        ));
    }
    variant
        .typeof_guard
        .map(|guard| format!("typeof {value_expr} === '{guard}'"))
}

/// Emits the constraint checks a union's **in-memory** value is held to, narrowed
/// to the branch it holds: one guarded block per non-object branch that declares
/// anything (P12 — an in-memory member violating its own branch's rules fails
/// before emit rather than being written). Object branches carry their own
/// validation in their model's converter, so they contribute no block here.
///
/// Emits nothing when no branch declares a constraint, so a plain sum type of
/// unconstrained kinds keeps its verbatim assignment.
fn render_ts_union_value_checks(
    output: &mut String,
    union: &TsUnion,
    value_expr: &str,
    path_expr: &str,
    indent: &str,
) {
    for variant in union.variants.iter().filter(|variant| !variant.is_object) {
        let mut body = String::new();
        render_ts_field_checks(
            &mut body,
            &variant.schema,
            &format!("({value_expr} as {})", variant.ts_type),
            path_expr,
            &format!("{indent}  "),
        );
        if body.is_empty() {
            continue;
        }
        let Some(guard) = ts_variant_guard(variant, value_expr) else {
            continue;
        };
        output.push_str(indent);
        output.push_str(&format!("if ({guard}) {{\n"));
        output.push_str(&body);
        output.push_str(indent);
        output.push_str("}\n");
    }
}

/// Emits the serialize dispatch for a union def's `toTransferType`.
fn render_ts_union_serialize(output: &mut String, union: &TsUnion, value_expr: &str) {
    for variant in &union.variants {
        if variant.is_object {
            let member = &variant.ts_type;
            let Some(converter) = variant.converter.as_deref() else {
                // An inline map-shaped branch: the in-memory value is the wire
                // object already.
                output.push_str(&format!(
                    "  if ({DEFINITIONS_NAMESPACE}.isPlainObject({value_expr})) {{\n    return {value_expr};\n  }}\n"
                ));
                continue;
            };
            if let (Some(discriminant), Some(value)) =
                (&union.discriminant, &variant.discriminant_value)
            {
                let literal =
                    typescript_value_literal(value).unwrap_or_else(|_| "undefined".to_string());
                output.push_str(&format!(
                    "  if (({value_expr} as unknown as Record<string, unknown>)[{}] === {literal}) {{\n    return {converter}.toTransferType({value_expr} as {member});\n  }}\n",
                    typescript_string_literal(discriminant)
                ));
            } else {
                // The lone object branch of a mixed-kind union: guard on the
                // object token so a scalar/array member still reaches its own
                // branch below (the token is the selector, both directions).
                output.push_str(&format!(
                    "  if ({DEFINITIONS_NAMESPACE}.isPlainObject({value_expr})) {{\n    return {converter}.toTransferType({value_expr} as unknown as {member});\n  }}\n"
                ));
            }
        } else if variant.is_array {
            let member = format!("({value_expr} as {})", variant.ts_type);
            // Array branches recurse through the same collecting mapper as an
            // ordinary model property. A referenced child can reject a mutated
            // in-memory value, and that failure must retain its element index
            // before the enclosing inline/named union prefixes its own path.
            let root_path = typescript_string_literal("");
            let serialized = serialize_expr_collecting(&variant.schema, &member, &root_path);
            output.push_str(&format!(
                "  if (Array.isArray({value_expr})) {{\n    const out = {serialized};\n    if (violations.length) {{\n      throw createPayloadValidationError(violations);\n    }}\n    return out;\n  }}\n"
            ));
        } else if variant.is_integer {
            output.push_str(&format!(
                "  if (typeof {value_expr} === 'number' && Number.isSafeInteger({value_expr})) {{\n    return {value_expr};\n  }}\n"
            ));
        } else if let Some(guard) = variant.typeof_guard {
            output.push_str(&format!(
                "  if (typeof {value_expr} === '{guard}') {{\n    return {value_expr};\n  }}\n"
            ));
        }
    }
    if union.nullable {
        output.push_str(&format!(
            "  if ({value_expr} === null) {{\n    return null;\n  }}\n"
        ));
    }
    output.push_str(&format!(
        "  throw createPayloadValidationError([{{ path: '', reason: 'expected one of: {}' }}]);\n",
        union.admissible()
    ));
}

/// The module-private serializer function an **inline** (property-level) union
/// needs when a member's in-memory form differs from its wire form — an object
/// branch, whose converter spreads `additionalProperties` back out, or an array
/// branch whose element mapper changes its in-memory representation.
fn ts_inline_union_serializer(
    model_name: &str,
    json_name: &str,
    property: &Schema,
    models: &[&PlannedJsonType],
) -> Option<(String, TsUnion)> {
    if property.one_of.is_none() {
        return None;
    }
    let union = classify_ts_union(property, models)?;
    if !union.variants.iter().any(|variant| {
        variant.converter.is_some()
            || (variant.is_array
                && schema_serializes_non_identity(
                    &serde_json::to_value(&variant.schema)
                        .expect("decoded TypeScript schema re-serializes"),
                    Some(active_repr()),
                ))
    }) {
        return None;
    }
    // Named off the union itself (`<Model><Property>`, the synthesized-name rule)
    // in the module's value namespace, which no generated type occupies.
    let name = format!(
        "serialize{model_name}{}",
        property.ts_member_name(json_name).to_upper_camel_case()
    );
    Some((name, union))
}

/// Emits the inline-union serializers a module's models reference (see
/// [`ts_inline_union_serializer`]). A named `$defs` union needs none — its own
/// `toTransferType` is the same dispatch.
fn render_ts_inline_union_serializers(
    output: &mut String,
    models: &[&PlannedJsonType],
) -> Result<()> {
    for model in models {
        let schema = decode_schema(model)?;
        let Some(properties) = &schema.properties else {
            continue;
        };
        for (json_name, property) in properties {
            let Some((name, union)) =
                ts_inline_union_serializer(&model.model_name, json_name, property, models)
            else {
                continue;
            };
            output.push_str(&format!(
                "\nfunction {name}(value: {}): unknown {{\n",
                type_annotation(property)?
            ));
            output.push_str(&format!(
                "  const violations: {DEFINITIONS_NAMESPACE}.Violation[] = [];\n"
            ));
            render_ts_union_serialize(output, &union, "value");
            output.push_str("}\n");
        }
    }
    Ok(())
}

fn render_model_interface(output: &mut String, model: &PlannedJsonType) -> Result<()> {
    if let Some(reference) = bare_ref_target(model) {
        output.push_str("export type ");
        output.push_str(&model.model_name);
        output.push_str(" = ");
        output.push_str(&reference_model_name(reference));
        output.push_str(";\n");
        return Ok(());
    }
    let schema = decode_schema(model)?;
    if is_ts_union(&schema) {
        render_ts_schema_doc(output, "", &schema);
        output.push_str("export type ");
        output.push_str(&model.model_name);
        output.push_str(" = ");
        output.push_str(&type_annotation(&schema)?);
        output.push_str(";\n");
        return Ok(());
    }
    render_ts_schema_doc(output, "", &schema);
    let use_type_alias = schema.properties.as_ref().is_some_and(|properties| {
        properties.iter().any(|(json_name, property)| {
            ts_member_conflicts_with_object_prototype(&property.ts_member_name(json_name))
        })
    });
    output.push_str(if use_type_alias {
        "export type "
    } else {
        "export interface "
    });
    output.push_str(&model.model_name);
    output.push_str(if use_type_alias { " = {\n" } else { " {\n" });

    if let Some(shape) = ts_map_shape(&schema)? {
        output.push_str("  additionalProperties: Record<string, ");
        output.push_str(&shape.value_annotation);
        output.push_str(">;\n");
        output.push_str(if use_type_alias { "};\n" } else { "}\n" });
        return Ok(());
    }

    let required = required_fields(&schema);
    if let Some(properties) = &schema.properties {
        for (json_name, property) in properties {
            render_ts_schema_doc(output, "  ", property);
            output.push_str("  ");
            if property.const_value.is_some() {
                output.push_str("readonly ");
            }
            output.push_str(&typescript_object_key(&property.ts_member_name(json_name)));
            if !required.contains(json_name) {
                output.push('?');
            }
            output.push_str(": ");
            output.push_str(&type_annotation(property)?);
            output.push_str(";\n");
        }
    }

    if is_open_object(&schema) {
        output.push_str("  additionalProperties: Record<string, ");
        output.push_str(&additional_properties_annotation(&schema)?);
        output.push_str(">;\n");
    }

    output.push_str(if use_type_alias { "};\n" } else { "}\n" });
    Ok(())
}

/// Opens the model's converter: an anonymous `TransferTypeConverter<Model>` class
/// expression instantiated in place, so consumers reference a ready instance
/// (`inputType: { transferTypeConverter: … }`) instead of constructing one.
fn open_transfer_type_converter(output: &mut String, model_name: &str) {
    output.push_str("export const ");
    output.push_str(&ts_transfer_type_converter_name(model_name));
    output.push_str(" = new class implements TransferTypeConverter<");
    output.push_str(model_name);
    output.push_str("> {\n");
    output.push_str("  public fromTransferType(raw: unknown): ");
    output.push_str(model_name);
    output.push_str(" {\n");
}

/// Closes the parse method and opens the serialize one.
fn split_transfer_type_converter(output: &mut String, model_name: &str) {
    output.push_str("  }\n\n");
    output.push_str("  public toTransferType(value: ");
    output.push_str(model_name);
    output.push_str("): unknown {\n");
}

fn close_transfer_type_converter(output: &mut String) {
    output.push_str("  }\n");
    output.push_str("}();\n");
}

fn render_model_transfer_type_converter(
    output: &mut String,
    model: &PlannedJsonType,
    models: &[&PlannedJsonType],
) -> Result<()> {
    if let Some(reference) = bare_ref_target(model) {
        output.push_str("export const ");
        output.push_str(&ts_transfer_type_converter_name(&model.model_name));
        output.push_str(": TransferTypeConverter<");
        output.push_str(&model.model_name);
        output.push_str("> = ");
        output.push_str(&ts_transfer_type_converter_name(&reference_model_name(
            reference,
        )));
        output.push_str(";\n");
        return Ok(());
    }
    let schema = decode_schema(model)?;
    if let Some(union) = classify_ts_union(&schema, models) {
        open_transfer_type_converter(output, &model.model_name);
        output.push_str(&format!(
            "    const violations: {DEFINITIONS_NAMESPACE}.Violation[] = [];\n"
        ));
        output.push_str("    let out: ");
        output.push_str(&model.model_name);
        output.push_str(" = undefined as unknown as ");
        output.push_str(&model.model_name);
        output.push_str(";\n");
        render_ts_union_parse(output, &union, "raw", "out", "''", "    ");
        output.push_str("    if (violations.length) {\n");
        output.push_str(&format!(
            "      throw createPayloadValidationError(violations);\n"
        ));
        output.push_str("    }\n");
        output.push_str("    return out;\n");
        split_transfer_type_converter(output, &model.model_name);
        // A named union has no enclosing model to aggregate into, so it collects
        // its own branch violations and throws the one aggregated error (P11/P12).
        output.push_str(&format!(
            "    const violations: {DEFINITIONS_NAMESPACE}.Violation[] = [];\n"
        ));
        let mut checks = String::new();
        render_ts_union_value_checks(&mut checks, &union, "value", "''", "    ");
        if !checks.is_empty() {
            output.push_str(&checks);
            output.push_str("    if (violations.length) {\n");
            output.push_str(&format!(
                "      throw createPayloadValidationError(violations);\n"
            ));
            output.push_str("    }\n");
        }
        render_ts_union_serialize(output, &union, "value");
        close_transfer_type_converter(output);
        return Ok(());
    }
    if is_open_object(&schema) {
        render_declared_field_set(output, model, &schema);
        output.push('\n');
    }

    open_transfer_type_converter(output, &model.model_name);

    let mut parser_body = String::new();
    render_model_parser_body(&mut parser_body, model, &schema, models)?;
    push_indented(output, &parser_body, "  ");

    split_transfer_type_converter(output, &model.model_name);

    let mut serializer_body = String::new();
    render_model_serializer_body(&mut serializer_body, model, &schema, models)?;
    push_indented(output, &serializer_body, "  ");

    close_transfer_type_converter(output);
    Ok(())
}

fn render_model_parser_body(
    output: &mut String,
    model: &PlannedJsonType,
    schema: &Schema,
    models: &[&PlannedJsonType],
) -> Result<()> {
    output.push_str(&format!(
        "  const violations: {DEFINITIONS_NAMESPACE}.Violation[] = [];\n"
    ));
    output.push_str(&format!(
        "  if (!{DEFINITIONS_NAMESPACE}.isPlainObject(raw)) {{\n"
    ));
    output.push_str(&format!(
        "    throw createPayloadValidationError([{{ path: '', reason: 'expected object' }}]);\n"
    ));
    output.push_str("  }\n\n");

    if let Some(shape) = ts_map_shape(schema)? {
        render_map_parser_body(output, schema, &shape);
        return Ok(());
    }

    let required = required_fields(&schema);
    let mut parsed_fields = Vec::new();
    if let Some(properties) = &schema.properties {
        for (json_name, property) in properties {
            render_property_parser(
                output,
                model,
                models,
                json_name,
                property,
                required.contains(json_name),
            )?;
            parsed_fields.push((json_name.clone(), property.ts_member_name(json_name)));
            output.push('\n');
        }
    }

    if schema.additional_properties.as_ref() == Some(&Value::Bool(false)) {
        render_closed_object_unknown_key_check(output, &parsed_fields);
    } else if schema
        .properties
        .as_ref()
        .is_some_and(|properties| !properties.is_empty())
    {
        render_open_object_collection(output, model, schema)?;
    }

    // Object member-count and cross-field constraints over the wire member set
    // (`raw` holds every distinct wire key, before default population).
    render_ts_property_count_checks(output, "Object.keys(raw).length", &schema, "  ");
    render_ts_dependent_required(output, "raw", &schema, "  ");

    output.push_str("  if (violations.length) {\n");
    output.push_str(&format!(
        "    throw createPayloadValidationError(violations);\n"
    ));
    output.push_str("  }\n");
    // An optional member is assigned after the literal is built, which a
    // `readonly` modifier (emitted for every `const` member) forbids — TS2540.
    // The staging binding therefore drops the modifier; the declared return type
    // puts it back, so callers still see an immutable `const` member.
    let readonly_optional = schema
        .properties
        .iter()
        .flatten()
        .any(|(json_name, property)| {
            property.const_value.is_some() && !required.contains(json_name)
        });
    output.push_str("  const out: ");
    if readonly_optional {
        output.push_str(&format!(
            "{{ -readonly [K in keyof {0}]: {0}[K] }}",
            model.model_name
        ));
    } else {
        output.push_str(&model.model_name);
    }
    output.push_str(" = { ");
    let mut required_out = parsed_fields
        .iter()
        .filter(|(json_name, _)| required.contains(json_name))
        .map(|(_, field_name)| field_name.clone())
        .collect::<Vec<_>>();
    if is_open_object(&schema) {
        required_out.push("additionalProperties".to_string());
    }
    output.push_str(&required_out.join(", "));
    output.push_str(" };\n");
    for (json_name, field_name) in &parsed_fields {
        if !required.contains(json_name) {
            output.push_str("  if (");
            output.push_str(field_name);
            output.push_str(" !== undefined) {\n");
            output.push_str("    out.");
            output.push_str(field_name);
            output.push_str(" = ");
            output.push_str(field_name);
            output.push_str(";\n");
            output.push_str("  }\n");
        }
    }
    output.push_str("  return out;\n");
    Ok(())
}

fn render_model_serializer_body(
    output: &mut String,
    model: &PlannedJsonType,
    schema: &Schema,
    models: &[&PlannedJsonType],
) -> Result<()> {
    // Serialize-side (P12): re-run the shared field validation over the
    // in-memory model and throw the aggregated payload-validation failure before emitting
    // the wire object — matching the parse path (both directions over one set of
    // check emitters).
    // Guard through an `unknown` alias. Calling this type predicate directly on
    // the typed model parameter narrows `value` to `Record<string, unknown>` in
    // the remainder of the serializer, which erases the model member types.
    output.push_str("  const candidate: unknown = value;\n");
    output.push_str(&format!(
        "  if (!{DEFINITIONS_NAMESPACE}.isPlainObject(candidate)) {{\n    throw createPayloadValidationError([{{ path: '', reason: 'expected object' }}]);\n  }}\n"
    ));
    let needs_validation = model_needs_serialize_validation(&schema)?;
    if needs_validation {
        output.push_str(&format!(
            "  const violations: {DEFINITIONS_NAMESPACE}.Violation[] = [];\n"
        ));
    }
    output.push_str(
        "  const out: Record<string, unknown> = Object.create(null) as Record<string, unknown>;\n",
    );

    if let Some(shape) = ts_map_shape(&schema)? {
        output.push_str(&format!(
            "  if (!{DEFINITIONS_NAMESPACE}.isPlainObject(value.additionalProperties)) {{\n    throw createPayloadValidationError([{{ path: '', reason: 'expected object' }}]);\n  }}\n"
        ));
        output.push_str(
            "  for (const [key, entry] of Object.entries(value.additionalProperties)) {\n",
        );
        // Every member is re-checked against `T` before emit (P12), keyed by its
        // own key — the same predicates the parse side ran.
        if let Some(value_schema) = &shape.value_schema {
            if serialize_expr(value_schema, "entry") != "entry" {
                output.push_str("    const memberViolationCount = violations.length;\n");
            }
            output.push_str(&format!(
                "    const path = {DEFINITIONS_NAMESPACE}.memberPath(key);\n"
            ));
            render_ts_member_check(output, value_schema, "entry", "path", "    ");
        }
        // A typed member re-serializes through its own mapper; an untyped one
        // (`additionalProperties: true`) is carried verbatim (P13).
        let entry = match &shape.value_schema {
            Some(value_schema) => serialize_expr_collecting(value_schema, "entry", "path"),
            None => "entry".to_string(),
        };
        if shape
            .value_schema
            .as_ref()
            .is_some_and(|value_schema| serialize_expr(value_schema, "entry") != "entry")
        {
            output.push_str("    if (violations.length === memberViolationCount) {\n");
            output.push_str(&format!("      out[key] = {entry};\n"));
            output.push_str("    }\n");
        } else {
            output.push_str(&format!("    out[key] = {entry};\n"));
        }
        output.push_str("  }\n");
        if needs_validation {
            let mut checks = String::new();
            render_ts_property_count_checks(&mut checks, "keys.length", &schema, "  ");
            if let Some(subschema) = &schema.property_names {
                render_ts_property_name_checks(&mut checks, "keys", subschema, "  ");
            }
            if !checks.is_empty() {
                output.push_str("  const keys = Object.keys(out);\n");
                output.push_str(&checks);
            }
            output.push_str("  if (violations.length) {\n");
            output.push_str(&format!(
                "    throw createPayloadValidationError(violations);\n"
            ));
            output.push_str("  }\n");
        }
        output.push_str("  return out;\n");
        return Ok(());
    }

    let required = required_fields(&schema);
    if let Some(properties) = &schema.properties {
        for (json_name, property) in properties {
            let field_name = property.ts_member_name(json_name);
            let value_expr = format!("value.{field_name}");
            // A union whose members need a transform goes through the module's
            // inline-union serializer; everything else is a plain expression.
            let path_expr = typescript_violation_path_literal(json_name);
            let assignment =
                match ts_inline_union_serializer(&model.model_name, json_name, property, models) {
                    Some((name, _)) => {
                        let call = format!("{name}({value_expr})");
                        if field_needs_serialize_check(property) {
                            collect_serialize_expr(&call, &path_expr)
                        } else {
                            call
                        }
                    }
                    None => serialize_expr_collecting(property, &value_expr, &path_expr),
                };
            if required.contains(json_name) {
                let requires_value = !allows_null(property);
                let indent = if requires_value {
                    output.push_str(&format!(
                        "  if ({value_expr} === undefined || {value_expr} === null) {{\n    violations.push({{ path: {path_expr}, reason: 'required' }});\n  }} else {{\n"
                    ));
                    "    "
                } else {
                    "  "
                };
                let gates_conversion = assignment != value_expr;
                let violation_count = format!("{field_name}ViolationCount");
                if gates_conversion {
                    output.push_str(indent);
                    output.push_str(&format!("const {violation_count} = violations.length;\n"));
                }
                render_ts_serialize_property_check(output, json_name, property, indent);
                let assignment_indent = if gates_conversion {
                    output.push_str(indent);
                    output.push_str(&format!(
                        "if (violations.length === {violation_count}) {{\n"
                    ));
                    format!("{indent}  ")
                } else {
                    indent.to_string()
                };
                output.push_str(&assignment_indent);
                output.push_str(&ts_index_access("out", json_name));
                output.push_str(" = ");
                output.push_str(&assignment);
                output.push_str(";\n");
                if gates_conversion {
                    output.push_str(indent);
                    output.push_str("}\n");
                }
                if requires_value {
                    output.push_str("  }\n");
                }
            } else {
                output.push_str("  if (value.");
                output.push_str(&field_name);
                output.push_str(" !== undefined) {\n");
                let rejects_null = !allows_null(property);
                let indent = if rejects_null {
                    output.push_str(&format!(
                        "    if ({value_expr} === null) {{\n      violations.push({{ path: {path_expr}, reason: 'explicit null not allowed' }});\n    }} else {{\n"
                    ));
                    "      "
                } else {
                    "    "
                };
                let gates_conversion = assignment != value_expr;
                let violation_count = format!("{field_name}ViolationCount");
                if gates_conversion {
                    output.push_str(indent);
                    output.push_str(&format!("const {violation_count} = violations.length;\n"));
                }
                render_ts_serialize_property_check(output, json_name, property, indent);
                let assignment_indent = if gates_conversion {
                    output.push_str(indent);
                    output.push_str(&format!(
                        "if (violations.length === {violation_count}) {{\n"
                    ));
                    format!("{indent}  ")
                } else {
                    indent.to_string()
                };
                output.push_str(&assignment_indent);
                output.push_str(&ts_index_access("out", json_name));
                output.push_str(" = ");
                output.push_str(&assignment);
                output.push_str(";\n");
                if gates_conversion {
                    output.push_str(indent);
                    output.push_str("}\n");
                }
                if rejects_null {
                    output.push_str("    }\n");
                }
                output.push_str("  }\n");
            }
        }
    }
    if is_open_object(&schema) {
        output.push_str(
            "  for (const [key, entry] of Object.entries(value.additionalProperties ?? {})) {\n",
        );
        output.push_str(&format!(
            "    const path = {DEFINITIONS_NAMESPACE}.memberPath(key);\n"
        ));
        output.push_str("    if (");
        output.push_str(&declared_fields_const_name(&model.model_name));
        output.push_str(".has(key)) {\n");
        output.push_str("      violations.push({ path, reason: 'catch-all key collides with declared property' });\n");
        output.push_str("      continue;\n");
        output.push_str("    }\n");
        if let Some(value_schema) = additional_properties_value_schema(&schema)? {
            let gates_conversion = serialize_expr(&value_schema, "entry") != "entry";
            if gates_conversion {
                output.push_str("    const memberViolationCount = violations.length;\n");
            }
            render_ts_member_check(output, &value_schema, "entry", "path", "    ");
            let entry = serialize_expr_collecting(&value_schema, "entry", "path");
            if gates_conversion {
                output.push_str("    if (violations.length === memberViolationCount) {\n");
                output.push_str(&format!("      out[key] = {entry};\n"));
                output.push_str("    }\n");
            } else {
                output.push_str(&format!("    out[key] = {entry};\n"));
            }
        } else {
            output.push_str("    out[key] = entry;\n");
        }
        output.push_str("  }\n");
    }
    if needs_validation {
        // Object member-count and cross-field constraints over the to-be-emitted
        // wire key set (`out` holds every distinct wire key, JSON-named).
        render_ts_property_count_checks(output, "Object.keys(out).length", &schema, "  ");
        render_ts_dependent_required(output, "out", &schema, "  ");
        output.push_str("  if (violations.length) {\n");
        output.push_str(&format!(
            "    throw createPayloadValidationError(violations);\n"
        ));
        output.push_str("  }\n");
    }
    output.push_str("  return out;\n");
    Ok(())
}

/// Emits the parse body of a map-shaped model: the member-count/key-shape
/// checks, then every wire member into `additionalProperties` (through the
/// member type's parse adapter when the members are typed).
fn render_map_parser_body(output: &mut String, schema: &Schema, shape: &TsMapShape) {
    output.push_str("  const keys = Object.keys(raw);\n");
    render_ts_property_count_checks(output, "keys.length", schema, "  ");
    if let Some(subschema) = &schema.property_names {
        render_ts_property_name_checks(output, "keys", subschema, "  ");
    }
    output.push_str("  const additionalProperties: Record<string, ");
    output.push_str(&shape.value_annotation);
    output.push_str("> = Object.create(null) as Record<string, ");
    output.push_str(&shape.value_annotation);
    output.push_str(">;\n");
    output.push_str("  for (const key of keys) {\n");
    match &shape.value_schema {
        // Untyped members are carried verbatim, `null` included (P13).
        None => output.push_str("    additionalProperties[key] = raw[key];\n"),
        Some(value_schema) => {
            output.push_str(&format!(
                "    const path = {DEFINITIONS_NAMESPACE}.memberPath(key);\n"
            ));
            output.push_str("    let entry: ");
            output.push_str(&shape.value_annotation);
            output.push_str(" | undefined = undefined;\n");
            render_value_parser(
                output,
                value_schema,
                "raw[key]",
                "entry",
                "path",
                "    ",
                true,
            );
            output.push_str("    if (entry !== undefined) {\n");
            output.push_str("      additionalProperties[key] = entry;\n");
            output.push_str("    }\n");
        }
    }
    output.push_str("  }\n");
    output.push_str("  if (violations.length) {\n");
    output.push_str(&format!(
        "    throw createPayloadValidationError(violations);\n"
    ));
    output.push_str("  }\n");
    output.push_str("  return { additionalProperties };\n");
}

fn render_property_parser(
    output: &mut String,
    model: &PlannedJsonType,
    models: &[&PlannedJsonType],
    json_name: &str,
    property: &Schema,
    required: bool,
) -> Result<()> {
    let field_name = property.ts_member_name(json_name);
    let annotation = if required {
        type_annotation(property)?
    } else {
        optional_type_annotation(&type_annotation(property)?)
    };
    output.push_str("  let ");
    output.push_str(&field_name);
    output.push_str(": ");
    output.push_str(&annotation);
    output.push_str(" = undefined as unknown as ");
    output.push_str(&annotation);
    output.push_str(";\n");

    let has_own = ts_has_own("raw", json_name);
    let raw_access = ts_index_access("raw", json_name);
    if required {
        if allows_null(property) {
            output.push_str(&format!("  if (!{has_own}) {{\n"));
        } else {
            output.push_str(&format!("  if (!{has_own} || {raw_access} === null) {{\n"));
        }
        output.push_str("    violations.push({ path: ");
        output.push_str(&typescript_violation_path_literal(json_name));
        output.push_str(", reason: 'required' });\n");
        output.push_str("  } else {\n");
        render_property_value_parser(output, model, models, json_name, property, &field_name)?;
        output.push_str("  }\n");
    } else {
        if allows_null(property) {
            output.push_str(&format!("  if ({has_own}) {{\n"));
        } else {
            output.push_str(&format!("  if ({has_own} && {raw_access} === null) {{\n"));
            output.push_str("    violations.push({ path: ");
            output.push_str(&typescript_violation_path_literal(json_name));
            output.push_str(", reason: 'explicit null not allowed' });\n");
            output.push_str(&format!("  }} else if ({has_own}) {{\n"));
        }
        render_property_value_parser(output, model, models, json_name, property, &field_name)?;
        output.push_str("  }\n");
    }

    Ok(())
}

fn render_property_value_parser(
    output: &mut String,
    model: &PlannedJsonType,
    models: &[&PlannedJsonType],
    json_name: &str,
    property: &Schema,
    field_name: &str,
) -> Result<()> {
    let raw_expr = ts_index_access("raw", json_name);
    let path_expr = typescript_violation_path_literal(json_name);
    let materialized = is_materialized_property(property);
    if !materialized && let Some(const_value) = &property.const_value {
        let const_name = const_const_name(
            &model.model_name,
            field_name,
            models,
            ConstNameCollisionKind::Const,
        )?;
        render_const_parser(
            output,
            const_value,
            &raw_expr,
            field_name,
            &path_expr,
            "    ",
            &const_name,
        );
        return Ok(());
    }
    if !materialized && let Some(values) = &property.enum_values {
        render_enum_parser(output, values, &raw_expr, field_name, &path_expr, "    ");
        return Ok(());
    }

    // An inline `oneOf` sum-type union dispatches on the wire token /
    // discriminant (a `$ref` to a named union routes through its converter via the
    // reference path below).
    if let Some(union) = classify_ts_union(property, models) {
        render_ts_union_parse(output, &union, &raw_expr, field_name, &path_expr, "    ");
        return Ok(());
    }

    render_value_parser(
        output, property, &raw_expr, field_name, &path_expr, "    ", false,
    );
    Ok(())
}

fn render_value_parser(
    output: &mut String,
    schema: &Schema,
    raw_expr: &str,
    target: &str,
    path_expr: &str,
    indent: &str,
    target_optional: bool,
) {
    render_value_parser_at_depth(
        output,
        schema,
        raw_expr,
        target,
        path_expr,
        indent,
        target_optional,
        0,
    );
}

/// The value parser, carrying the array nesting `depth`: an array's loop
/// variables are suffixed with their level so a nested array (`T[][]`) never
/// shadows the element, index, or item of the array above it.
fn render_value_parser_at_depth(
    output: &mut String,
    schema: &Schema,
    raw_expr: &str,
    target: &str,
    path_expr: &str,
    indent: &str,
    target_optional: bool,
    depth: usize,
) {
    if let Some(reference) = &schema.reference {
        let model_name = reference_model_name(reference);
        output.push_str(indent);
        output.push_str("try {\n");
        output.push_str(indent);
        output.push_str("  ");
        output.push_str(target);
        output.push_str(" = ");
        output.push_str(&ts_transfer_type_converter_name(&model_name));
        output.push_str(".fromTransferType(");
        output.push_str(raw_expr);
        output.push_str(");\n");
        output.push_str(indent);
        output.push_str("} catch (error) {\n");
        output.push_str(indent);
        output.push_str(&format!("  {DEFINITIONS_NAMESPACE}.collect(violations, "));
        output.push_str(path_expr);
        output.push_str(", error);\n");
        output.push_str(indent);
        output.push_str("}\n");
        return;
    }

    if let Some(branches) = &schema.one_of {
        let non_null = branches
            .iter()
            .filter(|branch| !schema_type_includes(branch, "null"))
            .collect::<Vec<_>>();
        if branches
            .iter()
            .any(|branch| schema_type_includes(branch, "null"))
        {
            output.push_str(indent);
            output.push_str("if (");
            output.push_str(raw_expr);
            output.push_str(" === null) {\n");
            output.push_str(indent);
            output.push_str("  ");
            output.push_str(target);
            output.push_str(" = null;\n");
            output.push_str(indent);
            output.push_str("} else {\n");
            if let Some(branch) = non_null.first() {
                render_value_parser(
                    output,
                    branch,
                    raw_expr,
                    target,
                    path_expr,
                    &format!("{indent}  "),
                    target_optional,
                );
            }
            output.push_str(indent);
            output.push_str("}\n");
            return;
        }
    }

    // A materialized temporal: check the wire is a string, then parse into the
    // repr-appropriate in-memory value via the runtime adapter (undefined on a
    // validation failure, which has already pushed a Violation).
    if let Some(kind) = temporal_kind_direct(schema) {
        let parse_fn = ts_temporal_parse_fn(kind);
        output.push_str(indent);
        output.push_str("if (typeof ");
        output.push_str(raw_expr);
        output.push_str(" !== 'string') {\n");
        output.push_str(indent);
        output.push_str("  violations.push({ path: ");
        output.push_str(path_expr);
        output.push_str(", reason: 'expected string' });\n");
        output.push_str(indent);
        output.push_str("} else {\n");
        render_ts_string_assertions(
            output,
            raw_expr,
            path_expr,
            schema,
            &format!("{indent}  "),
            false,
        );
        render_ts_schema_closed_check(output, schema, raw_expr, path_expr, &format!("{indent}  "));
        output.push_str(indent);
        output.push_str("  const parsed = ");
        output.push_str(&parse_fn);
        output.push('(');
        output.push_str(raw_expr);
        output.push_str(", ");
        output.push_str(path_expr);
        output.push_str(", violations);\n");
        output.push_str(indent);
        output.push_str("  if (parsed !== undefined) {\n");
        output.push_str(indent);
        output.push_str("    ");
        output.push_str(target);
        output.push_str(" = parsed;\n");
        output.push_str(indent);
        output.push_str("  }\n");
        output.push_str(indent);
        output.push_str("}\n");
        return;
    }

    // A materialized `contentEncoding`: check the wire is a string, then decode
    // into a `Uint8Array` via the generator-owned pure-JS codec (undefined on a
    // validation failure, which has already pushed a Violation).
    if let Some(encoding) = content_encoding_direct(schema) {
        let parse_fn = ts_content_encoding_parse_fn(encoding);
        output.push_str(indent);
        output.push_str("if (typeof ");
        output.push_str(raw_expr);
        output.push_str(" !== 'string') {\n");
        output.push_str(indent);
        output.push_str("  violations.push({ path: ");
        output.push_str(path_expr);
        output.push_str(", reason: 'expected string' });\n");
        output.push_str(indent);
        output.push_str("} else {\n");
        render_ts_string_assertions(
            output,
            raw_expr,
            path_expr,
            schema,
            &format!("{indent}  "),
            true,
        );
        render_ts_schema_closed_check(output, schema, raw_expr, path_expr, &format!("{indent}  "));
        output.push_str(indent);
        output.push_str("  const parsed = ");
        output.push_str(&parse_fn);
        output.push('(');
        output.push_str(raw_expr);
        output.push_str(", ");
        output.push_str(path_expr);
        output.push_str(", violations);\n");
        output.push_str(indent);
        output.push_str("  if (parsed !== undefined) {\n");
        output.push_str(indent);
        output.push_str("    ");
        output.push_str(target);
        output.push_str(" = parsed;\n");
        output.push_str(indent);
        output.push_str("  }\n");
        output.push_str(indent);
        output.push_str("}\n");
        return;
    }

    if let Some(const_value) = &schema.const_value {
        let const_literal =
            typescript_value_literal(const_value).unwrap_or_else(|_| "undefined".to_string());
        render_const_parser(
            output,
            const_value,
            raw_expr,
            target,
            path_expr,
            indent,
            &const_literal,
        );
        return;
    }

    if let Some(values) = &schema.enum_values {
        render_enum_parser(output, values, raw_expr, target, path_expr, indent);
        return;
    }

    match schema.ty.as_ref().and_then(Value::as_str) {
        Some("string") if schema.has_string_constraints() => {
            output.push_str(indent);
            output.push_str("if (typeof ");
            output.push_str(raw_expr);
            output.push_str(" !== 'string') {\n");
            output.push_str(indent);
            output.push_str("  violations.push({ path: ");
            output.push_str(path_expr);
            output.push_str(", reason: 'expected string' });\n");
            output.push_str(indent);
            output.push_str("} else {\n");
            output.push_str(indent);
            output.push_str("  ");
            output.push_str(target);
            output.push_str(" = ");
            output.push_str(raw_expr);
            output.push_str(";\n");
            render_ts_string_checks(output, raw_expr, path_expr, schema, &format!("{indent}  "));
            output.push_str(indent);
            output.push_str("}\n");
        }
        Some("string") => {
            render_typeof_parser(output, raw_expr, target, path_expr, indent, "string")
        }
        Some("number") => {
            output.push_str(indent);
            output.push_str("if (typeof ");
            output.push_str(raw_expr);
            output.push_str(" !== 'number') {\n");
            output.push_str(indent);
            output.push_str("  violations.push({ path: ");
            output.push_str(path_expr);
            output.push_str(", reason: 'expected number' });\n");
            output.push_str(indent);
            output.push_str("} else {\n");
            output.push_str(indent);
            output.push_str("  ");
            output.push_str(target);
            output.push_str(" = ");
            output.push_str(raw_expr);
            output.push_str(";\n");
            render_ts_numeric_checks(
                output,
                raw_expr,
                path_expr,
                schema,
                &format!("{indent}  "),
                false,
            );
            output.push_str(indent);
            output.push_str("}\n");
        }
        Some("boolean") => {
            render_typeof_parser(output, raw_expr, target, path_expr, indent, "boolean")
        }
        Some("integer") => {
            output.push_str(indent);
            output.push_str("if (typeof ");
            output.push_str(raw_expr);
            output.push_str(" !== 'number' || !Number.isSafeInteger(");
            output.push_str(raw_expr);
            output.push_str(")) {\n");
            output.push_str(indent);
            output.push_str("  violations.push({ path: ");
            output.push_str(path_expr);
            output.push_str(", reason: 'expected integer' });\n");
            output.push_str(indent);
            output.push_str("} else {\n");
            output.push_str(indent);
            output.push_str("  ");
            output.push_str(target);
            output.push_str(" = ");
            output.push_str(raw_expr);
            output.push_str(";\n");
            if schema.has_numeric_constraints() {
                render_ts_numeric_checks(
                    output,
                    raw_expr,
                    path_expr,
                    schema,
                    &format!("{indent}  "),
                    false,
                );
            }
            output.push_str(indent);
            output.push_str("}\n");
        }
        Some("array") => {
            render_array_parser(output, schema, raw_expr, target, path_expr, indent, depth)
        }
        Some("object") => {
            output.push_str(indent);
            output.push_str(target);
            output.push_str(" = ");
            output.push_str(raw_expr);
            output.push_str(" as ");
            output.push_str(&type_annotation(schema).unwrap_or_else(|_| "unknown".to_string()));
            output.push_str(";\n");
        }
        Some("null") => {
            output.push_str(indent);
            output.push_str(target);
            output.push_str(" = null;\n");
        }
        _ if target_optional => {
            output.push_str(indent);
            output.push_str(target);
            output.push_str(" = ");
            output.push_str(raw_expr);
            output.push_str(" as never;\n");
        }
        _ => {
            output.push_str(indent);
            output.push_str(target);
            output.push_str(" = ");
            output.push_str(raw_expr);
            output.push_str(" as never;\n");
        }
    }
}

fn render_typeof_parser(
    output: &mut String,
    raw_expr: &str,
    target: &str,
    path_expr: &str,
    indent: &str,
    ty: &str,
) {
    output.push_str(indent);
    output.push_str("if (typeof ");
    output.push_str(raw_expr);
    output.push_str(" !== '");
    output.push_str(ty);
    output.push_str("') {\n");
    output.push_str(indent);
    output.push_str("  violations.push({ path: ");
    output.push_str(path_expr);
    output.push_str(", reason: 'expected ");
    output.push_str(ty);
    output.push_str("' });\n");
    output.push_str(indent);
    output.push_str("} else {\n");
    output.push_str(indent);
    output.push_str("  ");
    output.push_str(target);
    output.push_str(" = ");
    output.push_str(raw_expr);
    output.push_str(";\n");
    output.push_str(indent);
    output.push_str("}\n");
}

/// The JS `typeof` guard string for a scalar const/enum value kind.
fn ts_typeof_guard(value: &Value) -> Option<&'static str> {
    match value {
        Value::String(_) => Some("string"),
        Value::Number(_) => Some("number"),
        Value::Bool(_) => Some("boolean"),
        _ => None,
    }
}

/// Emits the closed-value (`const` single-value / `enum` multi-value) parser: a
/// `typeof` guard, a membership check against the fixed set, and the assignment
/// on success. `compare_exprs` are the JS expressions to test against (a named
/// const constant for `const`, or inline literals for `enum`); `reason` is the
/// informative violation message. See `specs/json-schema/features/{const,enum}.md`.
fn render_closed_value_parser(
    output: &mut String,
    values: &[Value],
    compare_exprs: &[String],
    raw_expr: &str,
    target: &str,
    path_expr: &str,
    indent: &str,
    reason: &str,
) {
    let guard = values.first().and_then(ts_typeof_guard);
    let membership = compare_exprs
        .iter()
        .map(|expr| format!("{raw_expr} !== {expr}"))
        .collect::<Vec<_>>()
        .join(" && ");
    if let Some(guard) = guard {
        output.push_str(indent);
        output.push_str("if (typeof ");
        output.push_str(raw_expr);
        output.push_str(" !== '");
        output.push_str(guard);
        output.push_str("') {\n");
        output.push_str(indent);
        output.push_str("  violations.push({ path: ");
        output.push_str(path_expr);
        output.push_str(", reason: 'expected ");
        output.push_str(guard);
        output.push_str("' });\n");
        output.push_str(indent);
        output.push_str("} else if (");
    } else {
        output.push_str(indent);
        output.push_str("if (");
    }
    output.push_str(&membership);
    output.push_str(") {\n");
    output.push_str(indent);
    output.push_str("  violations.push({ path: ");
    output.push_str(path_expr);
    output.push_str(", reason: `");
    output.push_str(reason);
    output.push_str("` });\n");
    output.push_str(indent);
    output.push_str("} else {\n");
    output.push_str(indent);
    output.push_str("  ");
    output.push_str(target);
    output.push_str(" = ");
    output.push_str(raw_expr);
    // The membership check has narrowed `raw` to the base scalar type; cast it to
    // the closed literal union that types the field.
    let cast_type = join_union(
        values
            .iter()
            .filter_map(typescript_const_annotation)
            .collect(),
    );
    if !cast_type.is_empty() {
        output.push_str(" as ");
        output.push_str(&cast_type);
    }
    output.push_str(";\n");
    output.push_str(indent);
    output.push_str("}\n");
}

fn render_const_parser(
    output: &mut String,
    const_value: &Value,
    raw_expr: &str,
    target: &str,
    path_expr: &str,
    indent: &str,
    const_expr: &str,
) {
    let literal = typescript_value_literal(const_value).unwrap_or_else(|_| "undefined".to_string());
    let reason = format!("must equal {literal}");
    render_closed_value_parser(
        output,
        std::slice::from_ref(const_value),
        std::slice::from_ref(&const_expr.to_string()),
        raw_expr,
        target,
        path_expr,
        indent,
        &reason,
    );
}

fn render_enum_parser(
    output: &mut String,
    values: &[Value],
    raw_expr: &str,
    target: &str,
    path_expr: &str,
    indent: &str,
) {
    let literals = values
        .iter()
        .map(|value| typescript_value_literal(value).unwrap_or_else(|_| "undefined".to_string()))
        .collect::<Vec<_>>();
    let reason = format!(
        "must be one of [{}], got ${{JSON.stringify({raw_expr})}}",
        literals.join(", ")
    );
    render_closed_value_parser(
        output, values, &literals, raw_expr, target, path_expr, indent, &reason,
    );
}

fn ts_indexed_path(path_expr: &str, index_expr: &str) -> String {
    if let Some(path) = string_literal_value(path_expr) {
        format!("`{path}[${{{index_expr}}}]`")
    } else if let Some(path) = path_expr
        .strip_prefix('`')
        .and_then(|path| path.strip_suffix('`'))
    {
        format!("`{path}[${{{index_expr}}}]`")
    } else {
        format!("`${{{path_expr}}}[${{{index_expr}}}]`")
    }
}

fn render_array_parser(
    output: &mut String,
    schema: &Schema,
    raw_expr: &str,
    target: &str,
    path_expr: &str,
    indent: &str,
    depth: usize,
) {
    // Level 0 keeps the unsuffixed names; every nested level suffixes its own.
    let suffix = ts_depth_suffix(depth);
    let element = format!("element{suffix}");
    let index = format!("index{suffix}");
    let item = format!("item{suffix}");
    let item_annotation = schema
        .items
        .as_ref()
        .map(|item| type_annotation(item).unwrap_or_else(|_| "unknown".to_string()))
        .unwrap_or_else(|| "unknown".to_string());
    output.push_str(indent);
    output.push_str("if (!Array.isArray(");
    output.push_str(raw_expr);
    output.push_str(")) {\n");
    output.push_str(indent);
    output.push_str("  violations.push({ path: ");
    output.push_str(path_expr);
    output.push_str(", reason: 'expected array' });\n");
    output.push_str(indent);
    output.push_str("} else {\n");
    output.push_str(indent);
    output.push_str("  ");
    output.push_str(target);
    output.push_str(" = [];\n");
    output.push_str(indent);
    output.push_str("  ");
    output.push_str(raw_expr);
    output.push_str(&format!(
        ".forEach(({element}: unknown, {index}: number) => {{\n"
    ));
    output.push_str(indent);
    output.push_str(&format!("    let {item}: "));
    output.push_str(&item_annotation);
    output.push_str(" = undefined as unknown as ");
    output.push_str(&item_annotation);
    output.push_str(";\n");
    if let Some(element_schema) = &schema.items {
        let item_path_expr = ts_indexed_path(path_expr, &index);
        // Every element kind takes the same parse the value in that position
        // would take anywhere else, so a `string` element's own constraints
        // (`minLength`, `pattern`, `format`, …) are enforced and a mistyped
        // element names the type it failed to be (`expected string`) at its own
        // index — see `specs/json-schema/features/items.md`.
        render_value_parser_at_depth(
            output,
            element_schema,
            &element,
            &item,
            &item_path_expr,
            &format!("{indent}    "),
            false,
            depth + 1,
        );
    } else {
        output.push_str(indent);
        output.push_str(&format!("    {item} = {element} as unknown;\n"));
    }
    output.push_str(indent);
    output.push_str("    ");
    output.push_str(&format!("if ({item} !== undefined) {{\n"));
    output.push_str(indent);
    output.push_str("      ");
    output.push_str(target);
    output.push_str(&format!("!.push({item});\n"));
    output.push_str(indent);
    output.push_str("    }\n");
    output.push_str(indent);
    output.push_str("  });\n");
    // Sibling array keywords inspect the wire array, not the successfully
    // converted subset. A bad `items` element therefore cannot fabricate a
    // minItems failure or make two distinct raw elements look like duplicate
    // conversion placeholders.
    if schema.has_array_constraints() {
        render_ts_array_checks(
            output,
            raw_expr,
            path_expr,
            schema,
            &format!("{indent}  "),
            depth,
            false,
        );
    }
    output.push_str(indent);
    output.push_str("}\n");
}

fn render_closed_object_unknown_key_check(output: &mut String, fields: &[(String, String)]) {
    output.push_str("  for (const key of Object.keys(raw)) {\n");
    if fields.is_empty() {
        // A closed object with no declared properties rejects every member;
        // joining zero `key !== "…"` terms would emit `if () {`, which does not
        // parse (`additionalProperties.md` "Closed empty object" row).
        output.push_str(&format!(
            "    violations.push({{ path: {DEFINITIONS_NAMESPACE}.memberPath(key), reason: 'unknown field' }});\n"
        ));
        output.push_str("  }\n\n");
        return;
    }
    output.push_str("    if (");
    output.push_str(
        &fields
            .iter()
            .map(|(json_name, _)| format!("key !== {}", typescript_string_literal(json_name)))
            .collect::<Vec<_>>()
            .join(" && "),
    );
    output.push_str(") {\n");
    output.push_str(&format!(
        "      violations.push({{ path: {DEFINITIONS_NAMESPACE}.memberPath(key), reason: 'unknown field' }});\n"
    ));
    output.push_str("    }\n");
    output.push_str("  }\n\n");
}

fn render_declared_field_set(output: &mut String, model: &PlannedJsonType, schema: &Schema) {
    let fields = schema
        .properties
        .as_ref()
        .map(|properties| {
            properties
                .keys()
                .map(|field| typescript_string_literal(field))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    output.push_str("const ");
    output.push_str(&declared_fields_const_name(&model.model_name));
    output.push_str(" = new Set([");
    output.push_str(&fields.join(", "));
    output.push_str("]);\n");
}

fn render_open_object_collection(
    output: &mut String,
    model: &PlannedJsonType,
    schema: &Schema,
) -> Result<()> {
    let value_schema = additional_properties_value_schema(schema)?;
    let annotation = value_schema
        .as_ref()
        .map(type_annotation)
        .transpose()?
        .unwrap_or_else(|| "unknown".to_string());
    output.push_str("  const additionalProperties: Record<string, ");
    output.push_str(&annotation);
    output.push_str("> = Object.create(null) as Record<string, ");
    output.push_str(&annotation);
    output.push_str(">;\n");
    output.push_str("  for (const key of Object.keys(raw)) {\n");
    output.push_str("    if (!");
    output.push_str(&declared_fields_const_name(&model.model_name));
    output.push_str(".has(key)) {\n");
    if let Some(value_schema) = &value_schema {
        output.push_str(&format!(
            "      const path = {DEFINITIONS_NAMESPACE}.memberPath(key);\n"
        ));
        output.push_str("      let entry: ");
        output.push_str(&annotation);
        output.push_str(" | undefined = undefined;\n");
        render_value_parser(
            output,
            value_schema,
            "raw[key]",
            "entry",
            "path",
            "      ",
            true,
        );
        output.push_str("      if (entry !== undefined) {\n");
        output.push_str("        additionalProperties[key] = entry;\n");
        output.push_str("      }\n");
    } else {
        output.push_str("      additionalProperties[key] = raw[key];\n");
    }
    output.push_str("    }\n");
    output.push_str("  }\n\n");
    Ok(())
}

/// Emits the shared surrogate-aware code-point scan. `limit` bounds the walk:
/// the scan stops as soon as the running count exceeds it and returns
/// `limit + 1`, so a length assertion never traverses (or allocates) more of an
/// adversarially long string than the bound requires. Called with no `limit` it
/// returns the exact count. See `specs/json-schema/features/maxLength.md`.
fn render_code_point_length_helper(output: &mut String) {
    output.push_str(
        "export function codePointLength(value: string, limit = Number.POSITIVE_INFINITY): number {\n",
    );
    output.push_str("  let count = 0;\n");
    output.push_str("  for (let index = 0; index < value.length; index += 1) {\n");
    output.push_str("    const unit = value.charCodeAt(index);\n");
    output.push_str("    if (unit >= 0xd800 && unit <= 0xdbff && index + 1 < value.length) {\n");
    output.push_str("      const low = value.charCodeAt(index + 1);\n");
    output.push_str("      if (low >= 0xdc00 && low <= 0xdfff) {\n");
    output.push_str("        index += 1;\n");
    output.push_str("      }\n");
    output.push_str("    }\n");
    output.push_str("    count += 1;\n");
    output.push_str("    if (count > limit) {\n");
    output.push_str("      return count;\n");
    output.push_str("    }\n");
    output.push_str("  }\n");
    output.push_str("  return count;\n");
    output.push_str("}\n");
}

fn render_collect_helper(output: &mut String) {
    output.push_str("export function memberPath(key: string): string {\n");
    output.push_str(
        r#"  return /^[A-Za-z_][A-Za-z0-9_]*$/u.test(key) ? key : `["${key.replaceAll('\\', '\\\\').replaceAll('"', '\\"')}"]`;
"#,
    );
    output.push_str("}\n\n");
    output.push_str(
        "export function collect(violations: Violation[], path: string, error: unknown): void {\n",
    );
    output.push_str("  if (error instanceof ApplicationFailure && error.type === 'PayloadValidationError' && Array.isArray(error.details?.[0])) {\n");
    output.push_str("    // Generated failures keep the original array as their first detail; this cast performs no serialization.\n");
    output.push_str("    const nestedViolations = error.details?.[0] as Violation[];\n");
    output.push_str("    for (const inner of nestedViolations) {\n");
    // A nested violation about the value *itself* carries no path of its own (a
    // union branch's own constraint, an element-level check), so the prefix is
    // the whole path — never `segments[0].` with a dangling separator (P11).
    output.push_str("      const nested = !inner.path ? path : inner.path.startsWith('[') ? `${path}${inner.path}` : `${path}.${inner.path}`;\n");
    output.push_str("      violations.push({ path: nested, reason: inner.reason });\n");
    output.push_str("    }\n");
    output.push_str("  } else {\n");
    output.push_str("    violations.push({ path, reason: String(error) });\n");
    output.push_str("  }\n");
    output.push_str("}\n");
}

fn serialize_expr(schema: &Schema, value_expr: &str) -> String {
    if let Some(reference) = &schema.reference {
        let model_name = reference_model_name(reference);
        return format!(
            "{}.toTransferType({value_expr})",
            ts_transfer_type_converter_name(&model_name)
        );
    }
    // A materialized temporal re-serializes through the generator-owned
    // serializer (native-typed reprs) or passes the stored canonical string
    // through. `string`-stored temporals need no transform.
    if let Some(kind) = temporal_kind_direct(schema) {
        return ts_temporal_serialize_call(kind, active_repr(), value_expr);
    }
    // A materialized `contentEncoding` re-encodes bytes to the canonical wire
    // string via the generator-owned pure-JS codec.
    if let Some(encoding) = content_encoding_direct(schema) {
        return format!(
            "{}({value_expr})",
            ts_content_encoding_serialize_fn(encoding)
        );
    }
    if let Some(branches) = &schema.one_of
        && branches
            .iter()
            .any(|branch| schema_type_includes(branch, "null"))
        && let Some(non_null) = branches
            .iter()
            .find(|branch| !schema_type_includes(branch, "null"))
    {
        if non_null.reference.is_some()
            || temporal_kind_direct(non_null).is_some()
            || content_encoding_direct(non_null).is_some()
        {
            return format!(
                "{value_expr} === null ? null : {}",
                serialize_expr(non_null, value_expr)
            );
        }
    }
    // An array whose elements need a transform re-serializes elementwise: an
    // element model's own `toTransferType` flattens its catch-all bag onto the
    // wire object and re-encodes its temporal/bytes members, none of which the
    // in-memory value carries in wire form.
    if schema.ty.as_ref().and_then(Value::as_str) == Some("array")
        && let Some(items) = schema.items.as_deref()
    {
        let element = serialize_expr(items, "element");
        if element != "element" {
            return format!("{value_expr}.map((element) => {element})");
        }
    }
    value_expr.to_string()
}

fn collect_serialize_expr(expr: &str, path_expr: &str) -> String {
    format!(
        "(() => {{ try {{ return {expr}; }} catch (error) {{ {DEFINITIONS_NAMESPACE}.collect(violations, {path_expr}, error); return undefined; }} }})()"
    )
}

/// The ordinary serialize mapper with nested converter failures folded into the
/// current model's collector. Arrays recurse with their own index path, so a
/// child failure is reported as `field[1].member` rather than escaping at the
/// child's unqualified path.
fn serialize_expr_collecting(schema: &Schema, value_expr: &str, path_expr: &str) -> String {
    if let Some(reference) = &schema.reference {
        let model_name = reference_model_name(reference);
        let call = format!(
            "{}.toTransferType({value_expr})",
            ts_transfer_type_converter_name(&model_name)
        );
        return collect_serialize_expr(&call, path_expr);
    }
    if let Some(non_null) = nullable_non_null_schema(schema) {
        let converted = serialize_expr_collecting(non_null, value_expr, path_expr);
        if converted != value_expr {
            return format!("{value_expr} === null ? null : {converted}");
        }
    }
    if schema.ty.as_ref().and_then(Value::as_str) == Some("array")
        && let Some(items) = schema.items.as_deref()
    {
        let item_path = ts_indexed_path(path_expr, "index");
        let element = serialize_expr_collecting(items, "element", &item_path);
        if element != "element" {
            return format!("{value_expr}.map((element, index) => {element})");
        }
    }
    serialize_expr(schema, value_expr)
}

fn decode_schema(model: &PlannedJsonType) -> Result<Schema> {
    serde_json::from_value(model.schema.clone()).map_err(|error| Error::InvalidJsonSchema {
        path: PathBuf::from("<json-generator>"),
        reason: format!(
            "failed to read planned JSON schema `{}`: {error}",
            model.full_name
        ),
    })
}

fn additional_properties_value_schema(schema: &Schema) -> Result<Option<Schema>> {
    match &schema.additional_properties {
        Some(Value::Object(_)) => serde_json::from_value(
            schema
                .additional_properties
                .clone()
                .expect("additional properties presence checked"),
        )
        .map(Some)
        .map_err(|error| Error::InvalidJsonSchema {
            path: PathBuf::from("<json-generator>"),
            reason: format!("failed to read `additionalProperties`: {error}"),
        }),
        _ => Ok(None),
    }
}

/// A map-shaped object model — no declared `properties`, members governed by
/// `additionalProperties` — emitted as an interface wrapping a single
/// `additionalProperties` member (specs/json-schema/features/additionalProperties.md).
#[derive(Debug, Clone)]
struct TsMapShape {
    /// The declared member schema; `None` for untyped members
    /// (`additionalProperties: true`), which are carried verbatim as `unknown`.
    value_schema: Option<Schema>,
    /// The TypeScript element type of `Record<string, …>`.
    value_annotation: String,
}

/// Classifies a schema as map-shaped: an object with no declared `properties`
/// whose members are open. A closed empty object (`additionalProperties: false`)
/// admits no members and is not map-shaped.
fn ts_map_shape(schema: &Schema) -> Result<Option<TsMapShape>> {
    if schema.ty.as_ref().and_then(Value::as_str) != Some("object") {
        return Ok(None);
    }
    if schema
        .properties
        .as_ref()
        .is_some_and(|properties| !properties.is_empty())
    {
        return Ok(None);
    }
    if let Some(value_schema) = additional_properties_value_schema(schema)? {
        let value_annotation = type_annotation(&value_schema)?;
        return Ok(Some(TsMapShape {
            value_schema: Some(value_schema),
            value_annotation,
        }));
    }
    if schema.additional_properties.as_ref() == Some(&Value::Bool(false)) {
        return Ok(None);
    }
    Ok(Some(TsMapShape {
        value_schema: None,
        value_annotation: "unknown".to_string(),
    }))
}

fn type_annotation(schema: &Schema) -> Result<String> {
    if let Some(reference) = &schema.reference {
        return Ok(reference_model_name(reference));
    }
    // A materialized temporal `format` field type (per --date-time-types). The
    // `oneOf[…, null]` nullable wrapper is handled by the `one_of` join below,
    // which recurses into this branch for the non-null member.
    if let Some(kind) = temporal_kind_direct(schema) {
        return Ok(ts_temporal_type(kind, active_repr()).to_string());
    }
    // A materialized `contentEncoding` field type: the idiomatic binary
    // `Uint8Array` in both the browser and Node. The `oneOf[…, null]` wrapper is
    // handled by the `one_of` join below.
    if content_encoding_direct(schema).is_some() {
        return Ok("Uint8Array".to_string());
    }
    if let Some(const_value) = &schema.const_value
        && let Some(annotation) = typescript_const_annotation(const_value)
    {
        return Ok(annotation);
    }
    if let Some(values) = &schema.enum_values
        && !values.is_empty()
        && values
            .iter()
            .all(|value| typescript_const_annotation(value).is_some())
    {
        let literals = values
            .iter()
            .filter_map(typescript_const_annotation)
            .collect::<Vec<_>>();
        return Ok(join_union(literals));
    }
    if let Some(one_of) = &schema.one_of {
        let values = one_of
            .iter()
            .map(type_annotation)
            .collect::<Result<Vec<_>>>()?;
        return Ok(join_union(values));
    }
    match schema.ty.as_ref().and_then(Value::as_str) {
        Some("string") => Ok("string".to_string()),
        Some("integer" | "number") => Ok("number".to_string()),
        Some("boolean") => Ok("boolean".to_string()),
        Some("array") => {
            let item = schema
                .items
                .as_ref()
                .map(|item| type_annotation(item))
                .transpose()?
                .unwrap_or_else(|| "unknown".to_string());
            Ok(format!("{}[]", element_annotation(&item)))
        }
        Some("object") => object_annotation(schema),
        Some("null") => Ok("null".to_string()),
        _ => Ok("unknown".to_string()),
    }
}

/// An element type as it appears under the `[]` array suffix. A top-level union
/// has to be parenthesized: `string | null[]` is "a string or an array of
/// nulls", not the array of nullable strings the element schema declares.
fn element_annotation(annotation: &str) -> String {
    if split_top_level_union(annotation).len() > 1 {
        format!("({annotation})")
    } else {
        annotation.to_string()
    }
}

/// Splits a type annotation on its top-level `|`, ignoring any inside a type
/// argument list (`Record<string, A | B>` is one member, not two).
fn split_top_level_union(annotation: &str) -> Vec<&str> {
    let mut members = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (index, character) in annotation.char_indices() {
        match character {
            '<' | '(' | '[' | '{' => depth += 1,
            '>' | ')' | ']' | '}' => depth = depth.saturating_sub(1),
            '|' if depth == 0 => {
                members.push(annotation[start..index].trim());
                start = index + character.len_utf8();
            }
            _ => {}
        }
    }
    members.push(annotation[start..].trim());
    members
}

fn object_annotation(schema: &Schema) -> Result<String> {
    if schema
        .properties
        .as_ref()
        .is_some_and(|properties| !properties.is_empty())
    {
        return Ok("Record<string, unknown>".to_string());
    }
    Ok(format!(
        "Record<string, {}>",
        additional_properties_annotation(schema)?
    ))
}

fn additional_properties_annotation(schema: &Schema) -> Result<String> {
    match &schema.additional_properties {
        Some(Value::Object(value)) => {
            let additional: Schema =
                serde_json::from_value(Value::Object(value.clone())).map_err(|error| {
                    Error::InvalidJsonSchema {
                        path: PathBuf::from("<json-generator>"),
                        reason: format!("failed to read `additionalProperties`: {error}"),
                    }
                })?;
            type_annotation(&additional)
        }
        _ => Ok("unknown".to_string()),
    }
}

fn optional_type_annotation(annotation: &str) -> String {
    if annotation.contains("undefined") {
        annotation.to_string()
    } else {
        format!("{annotation} | undefined")
    }
}

fn typescript_const_annotation(value: &Value) -> Option<String> {
    match value {
        Value::Null => Some("null".to_string()),
        Value::Bool(value) => Some(value.to_string()),
        Value::Number(value) => Some(value.to_string()),
        Value::String(value) => Some(typescript_string_literal(value)),
        _ => None,
    }
}

fn required_fields(schema: &Schema) -> BTreeSet<String> {
    schema
        .required
        .iter()
        .flatten()
        .cloned()
        .collect::<BTreeSet<_>>()
}

fn allows_null(schema: &Schema) -> bool {
    schema.const_value.as_ref() == Some(&Value::Null)
        || schema_type_includes(schema, "null")
        || schema
            .one_of
            .as_ref()
            .is_some_and(|branches| branches.iter().any(allows_null))
}

fn is_open_object(schema: &Schema) -> bool {
    schema.ty.as_ref().and_then(Value::as_str) == Some("object")
        && schema
            .properties
            .as_ref()
            .is_some_and(|properties| !properties.is_empty())
        && schema.additional_properties.as_ref() != Some(&Value::Bool(false))
}

/// The single non-null branch of a `oneOf[T, null]` nullable wrapper, if any.
fn nullable_non_null_schema(schema: &Schema) -> Option<&Schema> {
    let branches = schema.one_of.as_ref()?;
    let non_null: Vec<&Schema> = branches
        .iter()
        .filter(|branch| !schema_type_includes(branch, "null"))
        .collect();
    if non_null.len() == 1 {
        Some(non_null[0])
    } else {
        None
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

fn join_union(values: Vec<String>) -> String {
    let mut deduped = Vec::<String>::new();
    for value in values {
        if !deduped.contains(&value) {
            deduped.push(value);
        }
    }
    deduped.join(" | ")
}

fn reference_model_name(reference: &str) -> String {
    if let Some(resolved) = REF_NAMES.with(|cell| cell.borrow().get(reference).cloned()) {
        return resolved;
    }
    let name = reference
        .split('#')
        .next_back()
        .unwrap_or(reference)
        .trim_start_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(reference);
    name.rsplit('#')
        .next()
        .unwrap_or(name)
        .to_upper_camel_case()
}

fn declared_fields_const_name(model_name: &str) -> String {
    format!("{}_DECLARED", model_name.to_shouty_snake_case())
}

#[derive(Debug, Clone, Copy)]
enum ConstNameCollisionKind {
    Const,
    Default,
}

fn const_const_name(
    model_name: &str,
    field_name: &str,
    models: &[&PlannedJsonType],
    kind: ConstNameCollisionKind,
) -> Result<String> {
    const_name(model_name, field_name, models, kind, "", "_CONST")
}

fn default_const_name(
    _model_name: &str,
    field_name: &str,
    _models: &[&PlannedJsonType],
    _kind: ConstNameCollisionKind,
) -> Result<String> {
    // This exported name must not depend on unrelated models in the run. The
    // P15 manifest rejects a second DEFAULT_<FIELD> claimant and points at the
    // declaring member's x-ts-name escape hatch.
    Ok(format!("DEFAULT_{}", field_name.to_shouty_snake_case()))
}

/// Names a synthesized module-scope constant after the **emitted member
/// identifier**, so an `x-ts-name` override on the declaring property moves the
/// constant with it: a name synthesized *from the member* follows the member
/// (P15, see specs/json-schema/features/default.md). The JSON name still selects
/// the property; only the identifier is derived from the override.
///
/// Private `_CONST` bindings may use the model name to disambiguate within one
/// module. Exported `DEFAULT_` bindings take the stable path above: another
/// claimant rejects rather than changing an existing consumer-facing name.
fn const_name(
    model_name: &str,
    member_ident: &str,
    models: &[&PlannedJsonType],
    kind: ConstNameCollisionKind,
    prefix: &str,
    suffix: &str,
) -> Result<String> {
    let field_count = models
        .iter()
        .map(|model| decode_schema(model))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .filter(|schema| {
            schema.properties.as_ref().is_some_and(|properties| {
                properties.iter().any(|(json_name, property)| {
                    property.ts_member_name(json_name) == member_ident
                        && match kind {
                            ConstNameCollisionKind::Const => property.const_value.is_some(),
                            ConstNameCollisionKind::Default => property.default.is_some(),
                        }
                })
            })
        })
        .count();

    let mut name = if field_count == 1 {
        member_ident.to_shouty_snake_case()
    } else {
        format!(
            "{}_{}",
            model_name.to_shouty_snake_case(),
            member_ident.to_shouty_snake_case()
        )
    };
    name.insert_str(0, prefix);
    name.push_str(suffix);
    Ok(name)
}

fn string_literal_value(value: &str) -> Option<String> {
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .map(ToOwned::to_owned)
}

/// Renders the JSDoc for a schema declaration: `title` summary line,
/// `description` body, and a bare `@deprecated` tag when `deprecated: true`.
/// See specs/json-schema/features/{title,description,deprecated}.md.
fn render_ts_schema_doc(output: &mut String, indent: &str, schema: &Schema) {
    let mut parts: Vec<String> = Vec::new();
    if let Some(title) = schema
        .title
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
    {
        parts.push(title.to_string());
    }
    if let Some(desc) = schema
        .description
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty())
    {
        parts.push(desc.to_string());
    }
    let summary = (!parts.is_empty()).then(|| parts.join("\n\n"));
    let tags = if schema.deprecated == Some(true) {
        vec![("@deprecated".to_string(), String::new())]
    } else {
        Vec::new()
    };
    render_typescript_doc_comment(output, indent, summary.as_deref(), &tags);
}

fn typescript_object_key(name: &str) -> String {
    let mut chars = name.chars();
    let valid = match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' || first == '$' => {
            chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
        }
        _ => false,
    };
    if valid {
        name.to_string()
    } else {
        typescript_string_literal(name)
    }
}

fn typescript_string_literal(value: &str) -> String {
    serde_json::to_string(value)
        .expect("a Rust string is always JSON-serializable")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

fn typescript_violation_path_literal(key: &str) -> String {
    typescript_string_literal(&violation_member_segment(key))
}

fn typescript_value_literal(value: &Value) -> Result<String> {
    match value {
        Value::Null => Ok("null".to_string()),
        Value::Bool(value) => Ok(value.to_string()),
        Value::Number(value) => Ok(value.to_string()),
        Value::String(value) => Ok(typescript_string_literal(value)),
        Value::Array(values) => {
            let values = values
                .iter()
                .map(typescript_value_literal)
                .collect::<Result<Vec<_>>>()?;
            Ok(format!("[{}]", values.join(", ")))
        }
        Value::Object(values) => {
            let values = values
                .iter()
                .map(|(key, value)| {
                    Ok(format!(
                        "{}: {}",
                        typescript_object_key(key),
                        typescript_value_literal(value)?
                    ))
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(format!("{{ {} }}", values.join(", ")))
        }
    }
}
