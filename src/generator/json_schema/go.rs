use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use heck::ToUpperCamelCase;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::Value;

use crate::error::{Error, Result};
use crate::generator::ExternalModelBackend;
use crate::generator::go::{
    GoPackageContext, PlannedMessageSource, PlannedMessageType, PlannedValueType, go_field_name,
    go_string_literal,
};
use crate::generator::json_schema::build_json_name_manifest;
use crate::generator::json_schema::register_cross_module_ref_names;
use crate::json_schema::scalar::{ScalarKind, ScalarMatcher};
use crate::language::Language;
use crate::parser::NameManifest;
use crate::planning::{PlannedFamily, PlannedJsonType, PlannedSpec};
use crate::spec::{ExternalTypeSpec, OperationSpec, RecordSpec, TypeSpec};

#[derive(Debug, Deserialize, Default, Clone)]
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
    #[serde(rename = "x-go-name")]
    x_go_name: Option<String>,
    #[serde(rename = "x-go-const-name")]
    x_go_const_name: Option<String>,
    #[serde(rename = "x-go-enum-names")]
    x_go_enum_names: Option<IndexMap<String, String>>,
}

impl Schema {
    /// The emitted Go field identifier for a property: the `x-go-name` override
    /// if present (used verbatim), otherwise the PascalCased JSON name. The wire
    /// name is unaffected (the `json:"<name>"` tag pins it). See
    /// specs/json-schema/features/properties.md.
    fn go_member_name(&self, json_name: &str) -> String {
        self.x_go_name
            .clone()
            .unwrap_or_else(|| go_field_name(json_name))
    }

    fn has_numeric_constraints(&self) -> bool {
        self.minimum.is_some()
            || self.maximum.is_some()
            || self.exclusive_minimum.is_some()
            || self.exclusive_maximum.is_some()
            || self.multiple_of.is_some()
    }

    fn has_string_constraints(&self) -> bool {
        self.min_length.is_some()
            || self.max_length.is_some()
            || self.pattern.is_some()
            || self.format.is_some()
    }

    fn has_array_constraints(&self) -> bool {
        self.min_items.is_some()
            || self.max_items.is_some()
            || self.unique_items == Some(true)
            || self.contains.is_some()
    }
}

/// Formats a numeric-bound literal for the given field kind. Integer-field
/// bounds are integer-valued (loader-enforced), so they render without a
/// fractional suffix; number-field bounds render as-is.
fn go_bound_literal(number: &serde_json::Number, is_integer: bool) -> String {
    if is_integer {
        if let Some(value) = number.as_f64() {
            return (value.trunc() as i64).to_string();
        }
    }
    number.to_string()
}

/// Emits the numeric-constraint predicates (`minimum`/`maximum`/`exclusive*`/
/// `multipleOf`) over `value_expr` (an `int64` or `float64` already in scope),
/// appending Violations to `errs`. Shared by the parse (`UnmarshalJSON`) and
/// serialize (`Validate`) paths per P12.
fn render_go_numeric_checks(
    output: &mut String,
    value_expr: &str,
    path: &str,
    schema: &Schema,
    indent: &str,
) {
    render_go_numeric_checks_to(output, value_expr, path, schema, indent, "errs");
}

fn render_go_numeric_checks_to(
    output: &mut String,
    value_expr: &str,
    path: &str,
    schema: &Schema,
    indent: &str,
    errs_value: &str,
) {
    let is_integer = schema.ty.as_ref().and_then(Value::as_str) == Some("integer");
    let mut emit = |condition: String, reason: String| {
        output.push_str(indent);
        output.push_str("if ");
        output.push_str(&condition);
        output.push_str(" {\n");
        output.push_str(indent);
        output.push('\t');
        output.push_str(errs_value);
        output.push_str(" = append(");
        output.push_str(errs_value);
        output.push_str(", Violation{");
        output.push_str(path);
        output.push_str(", fmt.Sprintf(");
        output.push_str(&go_string_literal(&reason));
        output.push_str(", ");
        output.push_str(value_expr);
        output.push_str(")})\n");
        output.push_str(indent);
        output.push_str("}\n");
    };
    if let Some(min) = &schema.minimum {
        let bound = go_bound_literal(min, is_integer);
        emit(
            format!("{value_expr} < {bound}"),
            format!("must be >= {bound}, got %v"),
        );
    }
    if let Some(max) = &schema.maximum {
        let bound = go_bound_literal(max, is_integer);
        emit(
            format!("{value_expr} > {bound}"),
            format!("must be <= {bound}, got %v"),
        );
    }
    if let Some(min) = &schema.exclusive_minimum {
        let bound = go_bound_literal(min, is_integer);
        emit(
            format!("{value_expr} <= {bound}"),
            format!("must be > {bound}, got %v"),
        );
    }
    if let Some(max) = &schema.exclusive_maximum {
        let bound = go_bound_literal(max, is_integer);
        emit(
            format!("{value_expr} >= {bound}"),
            format!("must be < {bound}, got %v"),
        );
    }
    if let Some(divisor) = &schema.multiple_of {
        let bound = go_bound_literal(divisor, is_integer);
        let condition = if is_integer {
            format!("{value_expr}%{bound} != 0")
        } else {
            format!("math.Mod({value_expr}, {bound}) != 0")
        };
        emit(condition, format!("must be a multiple of {bound}, got %v"));
    }
}

/// Emits the string-length predicates (`minLength`/`maxLength`) over
/// `value_expr` (a `string` already in scope), appending Violations to `errs`.
/// Length is the Unicode code-point count via `utf8.RuneCountInString` (never
/// `len`, which is the UTF-8 byte count) — see `specs/json-schema/features/maxLength.md`.
/// Shared by the parse (`UnmarshalJSON`) and serialize (`Validate`) paths per P12.
fn render_go_string_checks(
    output: &mut String,
    value_expr: &str,
    path: &str,
    schema: &Schema,
    indent: &str,
) {
    render_go_string_checks_to(output, value_expr, path, schema, indent, "errs");
}

fn render_go_string_checks_to(
    output: &mut String,
    value_expr: &str,
    path: &str,
    schema: &Schema,
    indent: &str,
    errs_value: &str,
) {
    let mut emit = |condition: &str, reason: String| {
        output.push_str(indent);
        output.push_str("if n := utf8.RuneCountInString(");
        output.push_str(value_expr);
        output.push_str("); ");
        output.push_str(condition);
        output.push_str(" {\n");
        output.push_str(indent);
        output.push('\t');
        output.push_str(errs_value);
        output.push_str(" = append(");
        output.push_str(errs_value);
        output.push_str(", Violation{");
        output.push_str(path);
        output.push_str(", fmt.Sprintf(");
        output.push_str(&go_string_literal(&reason));
        output.push_str(", n)})\n");
        output.push_str(indent);
        output.push_str("}\n");
    };
    if let Some(min) = schema.min_length {
        emit(
            &format!("n < {min}"),
            format!("must have length >= {min}, got %d"),
        );
    }
    if let Some(max) = schema.max_length {
        emit(
            &format!("n > {max}"),
            format!("must have length <= {max}, got %d"),
        );
    }
}

/// Lowercases the first byte of an exported Go identifier so the generated
/// package-level regex vars stay unexported implementation detail.
fn go_unexported(name: &str) -> String {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The package-level compiled-regex var name for a `pattern` on `json_name` of
/// `model_name` — unique per (model, field) so the pattern compiles once at
/// package init (the load-time gate already proved it compiles). See
/// `specs/json-schema/features/pattern.md`.
fn go_pattern_var_name(model_name: &str, json_name: &str) -> String {
    go_unexported(&format!("{model_name}{}Pattern", go_field_name(json_name)))
}

/// Emits the package-level `var <Model><Field>Pattern = regexp.MustCompile(...)`
/// declarations for every string field of `schema` carrying a `pattern`. The
/// pattern is the loader-normalized form (`\s`/`\S` already the explicit ASCII
/// class); Go keeps `$` (RE2 end-anchor is already exception-free).
fn render_go_pattern_vars(output: &mut String, model: &PlannedJsonType, schema: &Schema) {
    // A typed map's member carries the same string refinements a property does,
    // under the `Value` position name the loader uses for a member shape.
    if let Ok(Some(value)) = typed_map_value_schema(schema) {
        render_go_string_vars_recursive(output, &model.model_name, MAP_MEMBER_POSITION, &value);
    }
    let Some(properties) = &schema.properties else {
        return;
    };
    for (json_name, property) in properties {
        render_go_string_vars_recursive(output, &model.model_name, json_name, property);
    }
}

fn render_go_string_vars_recursive(
    output: &mut String,
    model_name: &str,
    position: &str,
    schema: &Schema,
) {
    render_go_string_vars(output, model_name, position, schema);
    if let Some(items) = &schema.items {
        render_go_string_vars_recursive(output, model_name, &format!("{position}Item"), items);
    }
}

/// Emits the compiled-regex vars one string schema needs at `position` (a JSON
/// member name, or the `Value` position of a typed map's member).
fn render_go_string_vars(output: &mut String, model_name: &str, position: &str, schema: &Schema) {
    let schema = nullable_non_null_schema(schema).unwrap_or(schema);
    if schema.ty.as_ref().and_then(Value::as_str) != Some("string") {
        return;
    }
    if let Some(pattern) = &schema.pattern {
        output.push_str("var ");
        output.push_str(&go_pattern_var_name(model_name, position));
        output.push_str(" = regexp.MustCompile(");
        output.push_str(&go_string_literal(pattern));
        output.push_str(")\n");
    }
    if let Some(format) = &schema.format
        && let Some(check) = crate::json_schema::format::check_for(format)
    {
        // Go keeps the `$` end-anchor (RE2 is exception-free); the pinned
        // regex compiles once at package init.
        output.push_str("var ");
        output.push_str(&go_format_var_name(model_name, position));
        output.push_str(" = regexp.MustCompile(");
        output.push_str(&go_string_literal(&check.pattern));
        output.push_str(")\n");
    }
    if let Some(encoding) = content_encoding_kind(schema) {
        output.push_str("var ");
        output.push_str(&go_content_encoding_var_name(model_name, position));
        output.push_str(" = regexp.MustCompile(");
        output.push_str(&go_string_literal(encoding.pattern()));
        output.push_str(")\n");
    }
}

/// The package-level compiled-regex var name for a `contentEncoding` on
/// `json_name` of `model_name`. See `specs/json-schema/features/contentEncoding.md`.
fn go_content_encoding_var_name(model_name: &str, json_name: &str) -> String {
    go_unexported(&format!(
        "{model_name}{}ContentEncoding",
        go_field_name(json_name)
    ))
}

/// The package-level compiled-regex var name for a `format` on `json_name` of
/// `model_name`. See `specs/json-schema/features/format.md`.
fn go_format_var_name(model_name: &str, json_name: &str) -> String {
    go_unexported(&format!("{model_name}{}Format", go_field_name(json_name)))
}

/// Emits the `format` predicate over `value_expr` (a `string` in scope): the
/// length guard (if any) short-circuits **before** the pinned regex, so a single
/// combined condition pushes one Violation naming the format + value. Shared by
/// the parse (`UnmarshalJSON`) and serialize (`Validate`) paths per P12. See
/// `specs/json-schema/features/format.md`.
fn render_go_format_check(
    output: &mut String,
    value_expr: &str,
    path: &str,
    var_name: &str,
    format: &str,
    indent: &str,
) {
    render_go_format_check_to(output, value_expr, path, var_name, format, indent, "errs");
}

fn render_go_format_check_to(
    output: &mut String,
    value_expr: &str,
    path: &str,
    var_name: &str,
    format: &str,
    indent: &str,
    errs_value: &str,
) {
    let Some(check) = crate::json_schema::format::check_for(format) else {
        return;
    };
    output.push_str(indent);
    output.push_str("if ");
    if let Some(max) = check.max_code_points {
        output.push_str(&format!("utf8.RuneCountInString({value_expr}) > {max} || "));
    }
    output.push_str("!");
    output.push_str(var_name);
    output.push_str(".MatchString(");
    output.push_str(value_expr);
    output.push_str(") {\n");
    output.push_str(indent);
    output.push('\t');
    output.push_str(errs_value);
    output.push_str(" = append(");
    output.push_str(errs_value);
    output.push_str(", Violation{");
    output.push_str(path);
    output.push_str(", fmt.Sprintf(");
    output.push_str(&go_string_literal(&format!(
        "must be a valid {}, got %q",
        check.name
    )));
    output.push_str(", ");
    output.push_str(value_expr);
    output.push_str(")})\n");
    output.push_str(indent);
    output.push_str("}\n");
}

/// Emits the `pattern` predicate over `value_expr` (a `string` in scope):
/// `if !<var>.MatchString(v) { push Violation }`. `MatchString` is unanchored
/// (RE2), ASCII-class, code-point `.`. Shared by the parse (`UnmarshalJSON`) and
/// serialize (`Validate`) paths per P12. See `specs/json-schema/features/pattern.md`.
fn render_go_pattern_check(
    output: &mut String,
    value_expr: &str,
    path: &str,
    var_name: &str,
    pattern: &str,
    indent: &str,
) {
    render_go_pattern_check_to(output, value_expr, path, var_name, pattern, indent, "errs");
}

fn render_go_pattern_check_to(
    output: &mut String,
    value_expr: &str,
    path: &str,
    var_name: &str,
    pattern: &str,
    indent: &str,
    errs_value: &str,
) {
    output.push_str(indent);
    output.push_str("if !");
    output.push_str(var_name);
    output.push_str(".MatchString(");
    output.push_str(value_expr);
    output.push_str(") {\n");
    output.push_str(indent);
    output.push('\t');
    output.push_str(errs_value);
    output.push_str(" = append(");
    output.push_str(errs_value);
    output.push_str(", Violation{");
    output.push_str(path);
    output.push_str(", fmt.Sprintf(");
    output.push_str(&go_string_literal("must match pattern %q, got %q"));
    output.push_str(", ");
    output.push_str(&go_string_literal(pattern));
    output.push_str(", ");
    output.push_str(value_expr);
    output.push_str(")})\n");
    output.push_str(indent);
    output.push_str("}\n");
}

/// Renders a Go literal for a scalar matcher value in the element's static type
/// (`int64`/`float64`/`string`/`bool`).
fn go_scalar_literal(value: &Value, element_ty: Option<&str>) -> String {
    match value {
        Value::String(text) => go_string_literal(text),
        Value::Bool(boolean) => boolean.to_string(),
        Value::Number(number) => match element_ty {
            Some("integer") => go_bound_literal(number, true),
            _ => number.to_string(),
        },
        _ => "nil".to_string(),
    }
}

/// Lowers the generator's decoded schema node to the language-neutral scalar
/// matcher descriptor. The loader has already normalized and validated these
/// values; this step only removes applicator/container details before target
/// rendering.
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

/// Builds the boolean Go sub-conditions that define "match" for a scalar
/// `contains` matcher over `elem` (an element of the array, already of the
/// element's static type). A type-only matcher matches every element, so an
/// empty condition set renders as the literal `true`.
fn go_matcher_condition(matcher: &Schema, elem: &str, element_ty: Option<&str>) -> String {
    let matcher = scalar_matcher(matcher);
    let is_integer = element_ty == Some("integer")
        || matcher.kind == Some(ScalarKind::Integer) && element_ty != Some("number");
    let mut parts: Vec<String> = Vec::new();
    if let Some(value) = &matcher.const_value {
        parts.push(format!(
            "{elem} == {}",
            go_scalar_literal(value, element_ty)
        ));
    }
    if !matcher.enum_values.is_empty() {
        let alternatives = matcher
            .enum_values
            .iter()
            .map(|value| format!("{elem} == {}", go_scalar_literal(value, element_ty)))
            .collect::<Vec<_>>()
            .join(" || ");
        if !alternatives.is_empty() {
            parts.push(format!("({alternatives})"));
        }
    }
    if let Some(min) = &matcher.minimum {
        parts.push(format!("{elem} >= {}", go_bound_literal(min, is_integer)));
    }
    if let Some(max) = &matcher.maximum {
        parts.push(format!("{elem} <= {}", go_bound_literal(max, is_integer)));
    }
    if let Some(min) = &matcher.exclusive_minimum {
        parts.push(format!("{elem} > {}", go_bound_literal(min, is_integer)));
    }
    if let Some(max) = &matcher.exclusive_maximum {
        parts.push(format!("{elem} < {}", go_bound_literal(max, is_integer)));
    }
    if let Some(divisor) = &matcher.multiple_of {
        let bound = go_bound_literal(divisor, is_integer);
        if is_integer {
            parts.push(format!("{elem}%{bound} == 0"));
        } else {
            parts.push(format!("math.Mod({elem}, {bound}) == 0"));
        }
    }
    if let Some(min) = matcher.min_length {
        parts.push(format!("utf8.RuneCountInString({elem}) >= {min}"));
    }
    if let Some(max) = matcher.max_length {
        parts.push(format!("utf8.RuneCountInString({elem}) <= {max}"));
    }
    if let Some(pattern) = &matcher.pattern {
        parts.push(format!(
            "regexp.MustCompile({}).MatchString({elem})",
            go_string_literal(pattern)
        ));
    }
    if let Some(format) = &matcher.format
        && let Some(check) = crate::json_schema::format::check_for(format)
    {
        let length_guard = check
            .max_code_points
            .map(|max| format!("utf8.RuneCountInString({elem}) <= {max} && "))
            .unwrap_or_default();
        parts.push(format!(
            "{length_guard}regexp.MustCompile({}).MatchString({elem})",
            go_string_literal(&check.pattern)
        ));
    }
    if parts.is_empty() {
        "true".to_string()
    } else {
        parts.join(" && ")
    }
}

/// Emits the array-constraint predicates (`minItems`/`maxItems`/`uniqueItems`/
/// `contains`/`minContains`/`maxContains`) over `slice_expr` (a `[]T` already in
/// scope), appending Violations to `errs`. Shared by the parse (`UnmarshalJSON`)
/// and serialize (`Validate`) paths per P12.
fn render_go_array_checks(
    output: &mut String,
    slice_expr: &str,
    path: &str,
    schema: &Schema,
    indent: &str,
) {
    let element_ty = schema
        .items
        .as_ref()
        .and_then(|item| item.ty.as_ref())
        .and_then(Value::as_str);
    if let Some(min) = schema.min_items {
        output.push_str(indent);
        output.push_str(&format!("if n := len({slice_expr}); n < {min} {{\n"));
        output.push_str(indent);
        output.push_str(&format!(
            "\terrs = append(errs, Violation{{{path}, fmt.Sprintf(\"must have at least {min} items, got %d\", n)}})\n"
        ));
        output.push_str(indent);
        output.push_str("}\n");
    }
    if let Some(max) = schema.max_items {
        output.push_str(indent);
        output.push_str(&format!("if n := len({slice_expr}); n > {max} {{\n"));
        output.push_str(indent);
        output.push_str(&format!(
            "\terrs = append(errs, Violation{{{path}, fmt.Sprintf(\"must have at most {max} items, got %d\", n)}})\n"
        ));
        output.push_str(indent);
        output.push_str("}\n");
    }
    if schema.unique_items == Some(true) {
        let key_ty = match element_ty {
            Some("integer") => "int64",
            Some("number") => "float64",
            Some("boolean") => "bool",
            _ => "string",
        };
        output.push_str(indent);
        output.push_str(&format!(
            "{{\n{indent}\tseen := make(map[{key_ty}]int, len({slice_expr}))\n"
        ));
        output.push_str(indent);
        output.push_str(&format!("\tfor i, e := range {slice_expr} {{\n"));
        output.push_str(indent);
        output.push_str("\t\tif j, ok := seen[e]; ok {\n");
        output.push_str(indent);
        output.push_str(&format!(
            "\t\t\terrs = append(errs, Violation{{{path}, fmt.Sprintf(\"duplicate items: element at index %d equals index %d\", i, j)}})\n"
        ));
        output.push_str(indent);
        output.push_str("\t\t} else {\n");
        output.push_str(indent);
        output.push_str("\t\t\tseen[e] = i\n");
        output.push_str(indent);
        output.push_str("\t\t}\n");
        output.push_str(indent);
        output.push_str("\t}\n");
        output.push_str(indent);
        output.push_str("}\n");
    }
    if let Some(matcher) = &schema.contains {
        let condition = go_matcher_condition(matcher, "e", element_ty);
        let effective_min = schema.min_contains.unwrap_or(1);
        output.push_str(indent);
        output.push_str("{\n");
        output.push_str(indent);
        output.push_str("\tmatchCount := 0\n");
        output.push_str(indent);
        output.push_str(&format!("\tfor _, e := range {slice_expr} {{\n"));
        output.push_str(indent);
        output.push_str(&format!("\t\tif {condition} {{\n"));
        output.push_str(indent);
        output.push_str("\t\t\tmatchCount++\n");
        output.push_str(indent);
        output.push_str("\t\t}\n");
        output.push_str(indent);
        output.push_str("\t}\n");
        if effective_min > 0 {
            output.push_str(indent);
            output.push_str(&format!("\tif matchCount < {effective_min} {{\n"));
            output.push_str(indent);
            if schema.min_contains.is_some() {
                output.push_str(&format!(
                    "\t\terrs = append(errs, Violation{{{path}, fmt.Sprintf(\"too few matching items: at least {effective_min}, got %d\", matchCount)}})\n"
                ));
            } else {
                output.push_str(&format!(
                    "\t\terrs = append(errs, Violation{{{path}, \"no element matches the required schema\"}})\n"
                ));
            }
            output.push_str(indent);
            output.push_str("\t}\n");
        }
        if let Some(max) = schema.max_contains {
            output.push_str(indent);
            output.push_str(&format!("\tif matchCount > {max} {{\n"));
            output.push_str(indent);
            output.push_str(&format!(
                "\t\terrs = append(errs, Violation{{{path}, fmt.Sprintf(\"too many matching items: at most {max}, got %d\", matchCount)}})\n"
            ));
            output.push_str(indent);
            output.push_str("\t}\n");
        }
        output.push_str(indent);
        output.push_str("}\n");
    }
}

/// Emits deserialize-side array checks over the original `json.RawMessage`
/// elements. The typed slice is allowed to omit failed conversions internally,
/// but sibling array keywords must still see the complete wire instance.
fn render_go_raw_array_checks(
    output: &mut String,
    elements: &str,
    path: &str,
    schema: &Schema,
    indent: &str,
    level: usize,
    errs: &GoErrsBinding,
) {
    let errs_value = errs.value;
    if let Some(min) = schema.min_items {
        output.push_str(&format!("{indent}if n := len({elements}); n < {min} {{\n"));
        output.push_str(&format!(
            "{indent}\t{errs_value} = append({errs_value}, Violation{{{path}, fmt.Sprintf(\"must have at least {min} items, got %d\", n)}})\n{indent}}}\n"
        ));
    }
    if let Some(max) = schema.max_items {
        output.push_str(&format!("{indent}if n := len({elements}); n > {max} {{\n"));
        output.push_str(&format!(
            "{indent}\t{errs_value} = append({errs_value}, Violation{{{path}, fmt.Sprintf(\"must have at most {max} items, got %d\", n)}})\n{indent}}}\n"
        ));
    }
    if schema.unique_items == Some(true) {
        output.push_str(&format!(
            "{indent}{{\n{indent}\trawSeen{level} := make(map[string]int, len({elements}))\n{indent}\tfor rawIndex{level}, rawElement{level} := range {elements} {{\n{indent}\t\tvar rawValue{level} any\n{indent}\t\tif err := json.Unmarshal(rawElement{level}, &rawValue{level}); err != nil {{\n{indent}\t\t\tcontinue\n{indent}\t\t}}\n{indent}\t\tif rawNumber{level}, ok := rawValue{level}.(float64); ok && rawNumber{level} == 0 {{\n{indent}\t\t\trawValue{level} = float64(0)\n{indent}\t\t}}\n{indent}\t\trawKeyBytes{level}, _ := json.Marshal(rawValue{level})\n{indent}\t\trawKey{level} := string(rawKeyBytes{level})\n{indent}\t\tif priorIndex{level}, ok := rawSeen{level}[rawKey{level}]; ok {{\n{indent}\t\t\t{errs_value} = append({errs_value}, Violation{{{path}, fmt.Sprintf(\"duplicate items: element at index %d equals index %d\", rawIndex{level}, priorIndex{level})}})\n{indent}\t\t}} else {{\n{indent}\t\t\trawSeen{level}[rawKey{level}] = rawIndex{level}\n{indent}\t\t}}\n{indent}\t}}\n{indent}}}\n"
        ));
    }
    if let Some(matcher) = &schema.contains {
        let element_ty = schema
            .items
            .as_deref()
            .and_then(nullable_non_null_schema)
            .or(schema.items.as_deref())
            .and_then(|item| item.ty.as_ref())
            .and_then(Value::as_str);
        let candidate = format!("rawCandidate{level}");
        let condition = go_matcher_condition(matcher, &candidate, element_ty);
        let effective_min = schema.min_contains.unwrap_or(1);
        output.push_str(&format!("{indent}rawMatchCount{level} := 0\n"));
        output.push_str(&format!(
            "{indent}for _, rawElement{level} := range {elements} {{\n"
        ));
        match element_ty {
            Some("integer") => {
                output.push_str(&format!(
                    "{indent}\tdecoder{level} := json.NewDecoder(bytes.NewReader(rawElement{level}))\n{indent}\tdecoder{level}.UseNumber()\n{indent}\tvar rawNumber{level} json.Number\n{indent}\tif err := decoder{level}.Decode(&rawNumber{level}); err == nil {{\n{indent}\t\tif {candidate}, err := parseSpecInteger(rawNumber{level}); err == nil && ({condition}) {{\n{indent}\t\t\trawMatchCount{level}++\n{indent}\t\t}}\n{indent}\t}}\n"
                ));
            }
            Some("number") => {
                output.push_str(&format!(
                    "{indent}\tvar {candidate} float64\n{indent}\tif err := json.Unmarshal(rawElement{level}, &{candidate}); err == nil && math.IsInf({candidate}, 0) == false && ({condition}) {{\n{indent}\t\trawMatchCount{level}++\n{indent}\t}}\n"
                ));
            }
            Some("boolean") => {
                output.push_str(&format!(
                    "{indent}\tvar {candidate} bool\n{indent}\tif err := json.Unmarshal(rawElement{level}, &{candidate}); err == nil && ({condition}) {{\n{indent}\t\trawMatchCount{level}++\n{indent}\t}}\n"
                ));
            }
            _ => {
                output.push_str(&format!(
                    "{indent}\tvar {candidate} string\n{indent}\tif err := json.Unmarshal(rawElement{level}, &{candidate}); err == nil && ({condition}) {{\n{indent}\t\trawMatchCount{level}++\n{indent}\t}}\n"
                ));
            }
        }
        output.push_str(&format!("{indent}}}\n"));
        if effective_min > 0 {
            output.push_str(&format!(
                "{indent}if rawMatchCount{level} < {effective_min} {{\n"
            ));
            if schema.min_contains.is_some() {
                output.push_str(&format!(
                    "{indent}\t{errs_value} = append({errs_value}, Violation{{{path}, fmt.Sprintf(\"too few matching items: at least {effective_min}, got %d\", rawMatchCount{level})}})\n"
                ));
            } else {
                output.push_str(&format!(
                    "{indent}\t{errs_value} = append({errs_value}, Violation{{{path}, \"no element matches the required schema\"}})\n"
                ));
            }
            output.push_str(&format!("{indent}}}\n"));
        }
        if let Some(max) = schema.max_contains {
            output.push_str(&format!(
                "{indent}if rawMatchCount{level} > {max} {{\n{indent}\t{errs_value} = append({errs_value}, Violation{{{path}, fmt.Sprintf(\"too many matching items: at most {max}, got %d\", rawMatchCount{level})}})\n{indent}}}\n"
            ));
        }
    }
}

/// Emits the object member-count predicates (`minProperties`/`maxProperties`)
/// over `count_expr` (an `int` giving the number of distinct wire member keys —
/// one number over the whole object, never a per-bucket sum), appending
/// Violations to `errs`. Shared by the deserialize (`UnmarshalJSON`, counting
/// wire keys) and serialize (`MarshalJSON`, counting to-be-emitted keys) paths
/// per P12. See `specs/json-schema/features/minProperties.md`.
fn render_go_property_count_checks(
    output: &mut String,
    count_expr: &str,
    schema: &Schema,
    indent: &str,
) {
    if let Some(min) = schema.min_properties {
        output.push_str(indent);
        output.push_str(&format!("if n := {count_expr}; n < {min} {{\n"));
        output.push_str(indent);
        output.push_str(&format!(
            "\terrs = append(errs, Violation{{\"\", fmt.Sprintf(\"must have at least {min} properties, got %d\", n)}})\n"
        ));
        output.push_str(indent);
        output.push_str("}\n");
    }
    if let Some(max) = schema.max_properties {
        output.push_str(indent);
        output.push_str(&format!("if n := {count_expr}; n > {max} {{\n"));
        output.push_str(indent);
        output.push_str(&format!(
            "\terrs = append(errs, Violation{{\"\", fmt.Sprintf(\"must have at most {max} properties, got %d\", n)}})\n"
        ));
        output.push_str(indent);
        output.push_str("}\n");
    }
}

/// Emits the `propertyNames` key-shape predicate over the keys of `map_expr`,
/// applying the supported string key subschema to each key `k` and pushing a
/// `Violation{k, "invalid property name \"k\": <why>"}` per bad key. Reuses the
/// string-length assertions applied to keys instead of values. See
/// `specs/json-schema/features/propertyNames.md`.
fn render_go_property_name_checks(
    output: &mut String,
    map_expr: &str,
    subschema: &Schema,
    indent: &str,
) {
    if subschema.min_length.is_none()
        && subschema.max_length.is_none()
        && subschema.pattern.is_none()
        && subschema.format.is_none()
        && subschema.enum_values.is_none()
    {
        return;
    }
    output.push_str(indent);
    output.push_str(&format!("for k := range {map_expr} {{\n"));
    let inner = format!("{indent}\t");
    let mut emit = |condition: &str, reason: &str| {
        output.push_str(&inner);
        output.push_str(&format!(
            "if n := utf8.RuneCountInString(k); {condition} {{\n"
        ));
        output.push_str(&inner);
        output.push_str(&format!(
            "\terrs = append(errs, Violation{{k, fmt.Sprintf({}, k, n)}})\n",
            go_string_literal(&format!("invalid property name %q: {reason}"))
        ));
        output.push_str(&inner);
        output.push_str("}\n");
    };
    if let Some(min) = subschema.min_length {
        emit(
            &format!("n < {min}"),
            &format!("must have length >= {min}, got %d"),
        );
    }
    if let Some(max) = subschema.max_length {
        emit(
            &format!("n > {max}"),
            &format!("must have length <= {max}, got %d"),
        );
    }
    if let Some(pattern) = &subschema.pattern {
        output.push_str(&inner);
        output.push_str(&format!(
            "if !regexp.MustCompile({}).MatchString(k) {{\n",
            go_string_literal(pattern)
        ));
        output.push_str(&inner);
        output.push_str(&format!(
            "\terrs = append(errs, Violation{{k, fmt.Sprintf({}, k)}})\n",
            go_string_literal(&format!(
                "invalid property name %q: must match pattern {pattern}"
            ))
        ));
        output.push_str(&inner);
        output.push_str("}\n");
    }
    if let Some(values) = &subschema.enum_values {
        let alternatives = values
            .iter()
            .filter_map(Value::as_str)
            .map(|value| format!("k == {}", go_string_literal(value)))
            .collect::<Vec<_>>()
            .join(" || ");
        if !alternatives.is_empty() {
            output.push_str(&inner);
            output.push_str(&format!("if !({alternatives}) {{\n"));
            output.push_str(&inner);
            output.push_str(&format!(
                "\terrs = append(errs, Violation{{k, fmt.Sprintf({}, k)}})\n",
                go_string_literal("invalid property name %q: must equal an allowed value")
            ));
            output.push_str(&inner);
            output.push_str("}\n");
        }
    }
    if let Some(format) = &subschema.format
        && let Some(check) = crate::json_schema::format::check_for(format)
    {
        let mut condition = String::new();
        if let Some(max) = check.max_code_points {
            condition.push_str(&format!("utf8.RuneCountInString(k) > {max} || "));
        }
        condition.push_str(&format!(
            "!regexp.MustCompile({}).MatchString(k)",
            go_string_literal(&check.pattern)
        ));
        output.push_str(&inner);
        output.push_str(&format!("if {condition} {{\n"));
        output.push_str(&inner);
        output.push_str(&format!(
            "\terrs = append(errs, Violation{{k, fmt.Sprintf({}, k)}})\n",
            go_string_literal(&format!(
                "invalid property name %q: must be a valid {}",
                check.name
            ))
        ));
        output.push_str(&inner);
        output.push_str("}\n");
    }
    output.push_str(indent);
    output.push_str("}\n");
}

/// Emits the `dependentRequired` cross-field presence predicate over the
/// presence set `map_expr` (`all` on the wire deserialize path, `out` on the
/// serialize path): for each present trigger key, each dependent key must also
/// be present. See `specs/json-schema/features/dependentRequired.md`.
fn render_go_dependent_required(
    output: &mut String,
    map_expr: &str,
    schema: &Schema,
    indent: &str,
) {
    let Some(dependent_required) = &schema.dependent_required else {
        return;
    };
    for (trigger, deps) in dependent_required {
        output.push_str(indent);
        output.push_str(&format!(
            "if _, ok := {map_expr}[{}]; ok {{\n",
            go_string_literal(trigger)
        ));
        for dep in deps {
            output.push_str(indent);
            output.push_str(&format!(
                "\tif _, ok := {map_expr}[{}]; !ok {{\n",
                go_string_literal(dep)
            ));
            output.push_str(indent);
            let reason = format!("property {dep:?} is required when {trigger:?} is present");
            output.push_str(&format!(
                "\t\terrs = append(errs, Violation{{{}, {}}})\n",
                go_string_literal(dep),
                go_string_literal(&reason)
            ));
            output.push_str(indent);
            output.push_str("\t}\n");
        }
        output.push_str(indent);
        output.push_str("}\n");
    }
}

#[derive(Debug, Default)]
pub(in crate::generator) struct ModelFragments {
    pub(in crate::generator) imports: BTreeSet<String>,
    pub(in crate::generator) body: String,
}

#[derive(Debug)]
pub(in crate::generator) struct ModelBackend {
    include_service_imports: bool,
    json_models: Vec<PlannedJsonType>,
    local_json_models: Vec<PlannedJsonType>,
    model_names: BTreeMap<String, String>,
    manifest: NameManifest,
}

impl ModelBackend {
    pub(in crate::generator) fn new(include_service_imports: bool) -> Self {
        Self {
            include_service_imports,
            json_models: Vec::new(),
            local_json_models: Vec::new(),
            model_names: BTreeMap::new(),
            manifest: NameManifest::default(),
        }
    }
}

impl ExternalModelBackend<PlannedValueType> for ModelBackend {
    type ModelFragments = ModelFragments;
    type WireConversion = ();

    // Go flattens every input file in a generate closure into one flat
    // package (see `specs/json-schema/generated-file-layout.md`), so every
    // JSON model and cross-file service reference resolves as a local,
    // unqualified name -- there is never a real cross-package Go import to
    // emit here.
    fn prepare(&mut self, api_plan: &PlannedSpec) -> Result<()> {
        // Resolve every emitted identifier once (overrides applied), then adopt
        // the resolved type name as each model's `model_name` so every
        // downstream derivation (struct decl, unions, const defined types,
        // `$ref` targets) follows the same identifier — no re-derivation.
        self.manifest = build_json_name_manifest(Language::Go, api_plan)?;
        self.json_models = api_plan
            .external_types()
            .map(|(_, binding)| binding)
            .filter_map(|binding| match &binding.external_type {
                ExternalTypeSpec::Json(json_type) => Some(json_type.clone()),
                _ => None,
            })
            .map(|mut json_type| {
                if let Some(resolved) = self.manifest.type_name(&json_type.full_name) {
                    json_type.model_name = resolved.to_string();
                }
                json_type
            })
            .collect();
        self.local_json_models.clear();
        self.model_names.clear();
        // Go flattens the whole tree into one package, so a model another input
        // file declares is still an unqualified local name here -- but only the
        // tree-wide resolution knows the identifier that file's `x-go-name`
        // moved it to.
        register_cross_module_ref_names(api_plan, &mut self.model_names);

        for model in &self.json_models {
            // Go flattens every input file in a generate closure into one flat
            // package, so every model is a local, unqualified name -- there is
            // never a cross-package import to alias here.
            self.local_json_models.push(model.clone());
            // A resolved `$ref` is `#/$defs/<full_name>`; register that form too
            // so `reference_model_name` resolves through the manifest instead of
            // recasing the ref segment (which would drop a type override).
            self.model_names
                .insert(model.full_name.clone(), model.model_name.clone());
            self.model_names.insert(
                format!("#/$defs/{}", model.full_name),
                model.model_name.clone(),
            );
        }
        Ok(())
    }

    fn render_models(&self) -> Result<ModelFragments> {
        render_external_models(
            &self.local_json_models.iter().collect::<Vec<_>>(),
            &self.model_names,
        )
    }

    fn render_support_files(&self) -> Result<BTreeMap<PathBuf, String>> {
        Ok(BTreeMap::new())
    }

    fn model_type_annotation(&self, model_type: &PlannedValueType) -> Option<String> {
        let PlannedValueType::Message(message) = model_type else {
            return None;
        };
        if message.source != PlannedMessageSource::Json {
            return None;
        }
        self.model_names
            .get(&message.info.full_name)
            .cloned()
            .or_else(|| Some(message.model_name.clone()))
    }

    fn wire_type_identifier(&self, model_type: &PlannedValueType) -> Option<String> {
        let PlannedValueType::Message(message) = model_type else {
            return None;
        };
        if message.source != PlannedMessageSource::Json {
            return None;
        }
        Some(message.info.full_name.clone())
    }

    fn wire_conversion(
        &self,
        _model_type: &PlannedValueType,
        _planned_record: Option<&RecordSpec<PlannedFamily>>,
    ) -> Option<()> {
        None
    }
}

impl ModelBackend {
    pub(in crate::generator) fn is_active(&self) -> bool {
        !self.json_models.is_empty()
    }

    fn json_model_type_annotation(&self, json_type: &PlannedJsonType) -> String {
        self.model_names
            .get(&json_type.full_name)
            .cloned()
            .unwrap_or_else(|| json_type.model_name.clone())
    }

    pub(in crate::generator) fn has_message_wire_type(&self, message: &PlannedMessageType) -> bool {
        message.source == PlannedMessageSource::Json
            && self.model_names.contains_key(&message.info.full_name)
    }

    pub(in crate::generator) fn render_services(
        &self,
        api_plan: &PlannedSpec,
        package: &GoPackageContext,
    ) -> Result<String> {
        if api_plan.services.is_empty() {
            return Ok(String::new());
        }
        let mut output = String::new();
        for service in &api_plan.services {
            let service_var = service
                .code_name
                .for_language(crate::language::Language::Go)
                .map(str::to_string)
                .unwrap_or_else(|| go_field_name(&service.name));
            render_go_doc_comment(
                &mut output,
                "",
                service.doc.for_language(crate::language::Language::Go),
                &format!(
                    "{service_var} is the Nexus service binding for {:?}.",
                    service.wire_name
                ),
            );
            output.push_str("var ");
            output.push_str(&service_var);
            output.push_str(" = struct {\n");
            output.push_str("\tServiceName string\n");
            for operation in &service.operations {
                let operation_field = go_operation_field(operation);
                render_go_doc_comment(
                    &mut output,
                    "\t",
                    operation.doc.for_language(crate::language::Language::Go),
                    &format!(
                        "{operation_field} is the {:?} Nexus operation.",
                        operation.wire_name
                    ),
                );
                output.push('\t');
                output.push_str(&operation_field);
                output.push(' ');
                render_operation_reference_type(&mut output, operation, api_plan, package, self)?;
                output.push('\n');
            }
            output.push_str("}{\n");
            output.push_str("\tServiceName: ");
            output.push_str(&go_string_literal(&service.wire_name));
            output.push_str(",\n");
            for operation in &service.operations {
                output.push('\t');
                output.push_str(&go_operation_field(operation));
                output.push_str(": nexus.NewOperationReference[");
                output.push_str(&operation_io_type(
                    operation.input.as_ref(),
                    api_plan,
                    package,
                    self,
                )?);
                output.push_str(", ");
                output.push_str(&operation_io_type(
                    operation.output.as_ref(),
                    api_plan,
                    package,
                    self,
                )?);
                output.push_str("](");
                output.push_str(&go_string_literal(&operation.wire_name));
                output.push_str("),\n");
            }
            output.push_str("}\n\n");
            // The workflow client is part of the NativeApi surface; the
            // definitions-only output emits just the service/operation
            // reference struct so callers drive the Nexus client directly.
            if self.include_service_imports {
                render_service_client(&mut output, service, api_plan, package, self)?;
            }
        }
        Ok(output)
    }
}

/// The rendered Go source with its comment lines dropped, for deciding which
/// standard-library packages the file actually uses. Every generated comment is
/// a whole line (a doc comment above the declaration it documents), so dropping
/// those lines leaves the code that a package qualifier can appear in.
fn go_code_without_comments(output: &str) -> String {
    output
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The emitted Go identifier for an operation: the verbatim per-language
/// `x-<lang>-name` override when present, else the derived field name. Mirrors
/// the service `code_name` handling; never affects the wire name.
fn go_operation_field(operation: &crate::spec::OperationSpec<PlannedFamily>) -> String {
    operation
        .code_name
        .for_language(crate::language::Language::Go)
        .map(str::to_string)
        .unwrap_or_else(|| go_field_name(&operation.name))
}

fn render_service_client(
    output: &mut String,
    service: &crate::spec::ServiceSpec<PlannedFamily>,
    api_plan: &PlannedSpec,
    package: &GoPackageContext,
    backend: &ModelBackend,
) -> Result<()> {
    let service_var = service
        .code_name
        .for_language(crate::language::Language::Go)
        .map(str::to_string)
        .unwrap_or_else(|| go_field_name(&service.name));
    let client_name = format!("{service_var}Client");
    render_go_doc_comment(
        output,
        "",
        service.doc.for_language(crate::language::Language::Go),
        &format!(
            "{client_name} is a client for the {:?} Nexus service.",
            service.wire_name
        ),
    );
    output.push_str("type ");
    output.push_str(&client_name);
    output.push_str(" struct {\n\tclient workflow.NexusClient\n}\n\n");

    output.push_str("// New");
    output.push_str(&client_name);
    output.push_str(" constructs a ");
    output.push_str(&client_name);
    output.push_str(" bound to the given Nexus endpoint.\n");
    output.push_str("func New");
    output.push_str(&client_name);
    output.push_str("(endpoint string) *");
    output.push_str(&client_name);
    output.push_str(" {\n\treturn &");
    output.push_str(&client_name);
    output.push_str("{client: workflow.NewNexusClient(endpoint, ");
    output.push_str(&service_var);
    output.push_str(".ServiceName)}\n}\n\n");

    for operation in &service.operations {
        let operation_field = go_operation_field(operation);
        render_go_doc_comment(
            output,
            "",
            operation.doc.for_language(crate::language::Language::Go),
            &format!(
                "{operation_field} invokes the {:?} Nexus operation.",
                operation.wire_name
            ),
        );
        output.push_str("func (c *");
        output.push_str(&client_name);
        output.push_str(") ");
        output.push_str(&operation_field);
        output.push_str("(ctx workflow.Context");
        let input_expr = if operation.input.is_some() {
            output.push_str(", request ");
            output.push_str(&operation_io_type(
                operation.input.as_ref(),
                api_plan,
                package,
                backend,
            )?);
            "request"
        } else {
            "nil"
        };
        output.push_str(") workflow.Future {\n\treturn c.client.ExecuteOperation(ctx, ");
        output.push_str(&service_var);
        output.push('.');
        output.push_str(&operation_field);
        output.push_str(", ");
        output.push_str(input_expr);
        output.push_str(", workflow.NexusOperationOptions{})\n}\n\n");
    }

    Ok(())
}

fn render_operation_reference_type(
    output: &mut String,
    operation: &OperationSpec<PlannedFamily>,
    api_plan: &PlannedSpec,
    package: &GoPackageContext,
    backend: &ModelBackend,
) -> Result<()> {
    output.push_str("nexus.OperationReference[");
    output.push_str(&operation_io_type(
        operation.input.as_ref(),
        api_plan,
        package,
        backend,
    )?);
    output.push_str(", ");
    output.push_str(&operation_io_type(
        operation.output.as_ref(),
        api_plan,
        package,
        backend,
    )?);
    output.push(']');
    Ok(())
}

fn operation_io_type(
    ty: Option<&TypeSpec<PlannedFamily>>,
    api_plan: &PlannedSpec,
    package: &GoPackageContext,
    backend: &ModelBackend,
) -> Result<String> {
    let Some(ty) = ty else {
        return Ok("nexus.NoValue".to_string());
    };
    match ty.without_option() {
        TypeSpec::External(ExternalTypeSpec::Json(json_type)) => {
            Ok(backend.json_model_type_annotation(json_type))
        }
        TypeSpec::Record(record_ref) => Ok(api_plan
            .record(&record_ref.full_name)
            .map(|record| record.name.clone())
            .unwrap_or_else(|| record_ref.model_name.clone())),
        TypeSpec::External(ExternalTypeSpec::Alias { type_name, .. }) => Ok(type_name
            .for_language(crate::language::Language::Go)
            .map(|annotation| package.go_type_expr(annotation))
            .unwrap_or_else(|| "any".to_string())),
        _ => Ok("nexus.NoValue".to_string()),
    }
}

fn render_external_models(
    models: &[&PlannedJsonType],
    model_names: &BTreeMap<String, String>,
) -> Result<ModelFragments> {
    if models.is_empty() {
        return Ok(ModelFragments::default());
    }

    let mut imports = BTreeSet::from(["encoding/json".to_string()]);
    let mut output = String::new();
    render_const_discriminators(&mut output, models)?;
    let unions = collect_go_unions(models, model_names)?;
    if !unions.is_empty() {
        output.push('\n');
        render_go_unions(&mut output, &unions, models, model_names)?;
    }
    if models.iter().any(|model| model_uses_temporal(model)) {
        output.push('\n');
        render_go_temporal_helpers(&mut output);
    }
    if models
        .iter()
        .any(|model| model_uses_content_encoding(model))
    {
        output.push('\n');
        render_go_content_encoding_helpers(&mut output);
    }
    for model in models {
        output.push('\n');
        render_model(&mut output, model, models, model_names, &unions)?;
    }
    // Package use is read off the emitted *code*: a doc comment carries the
    // schema's own prose, and an unused import is a Go compile error, so a
    // description ending a sentence with "at a time." must not pull in `time`.
    let code = go_code_without_comments(&output);
    if code.contains("bytes.") {
        imports.insert("bytes".to_string());
    }
    if code.contains("fmt.") {
        imports.insert("fmt".to_string());
    }
    if code.contains("math.") {
        imports.insert("math".to_string());
    }
    if code.contains("utf8.") {
        imports.insert("unicode/utf8".to_string());
    }
    if code.contains("regexp.") {
        imports.insert("regexp".to_string());
    }
    if code.contains("base64.") {
        imports.insert("encoding/base64".to_string());
    }
    if code.contains("time.") {
        imports.insert("time".to_string());
    }
    if code.contains("strconv.") {
        imports.insert("strconv".to_string());
    }
    if code.contains("strings.") {
        imports.insert("strings".to_string());
    }
    if code.contains("nexus.") {
        imports.insert("github.com/nexus-rpc/sdk-go/nexus".to_string());
    }
    Ok(ModelFragments {
        imports,
        body: output,
    })
}

/// Renders the shared `definitions.go` file: the schema-independent runtime
/// (`Violation`/`ValidationError`, `parseSpecInteger`, the (de)serialize
/// helpers) defined once in the models' own package. The identifiers stay
/// unexported because the generated model files reference them within the same
/// package.
pub(in crate::generator) fn render_definitions_file(package_name: &str) -> String {
    let mut output = String::new();
    output.push_str(crate::generator::go::GENERATED_HEADER);
    output.push('\n');
    output.push_str("package ");
    output.push_str(package_name);
    output.push_str("\n\n");
    output.push_str("import (\n");
    output.push_str("\t\"bytes\"\n");
    output.push_str("\t\"encoding/json\"\n");
    output.push_str("\t\"errors\"\n");
    output.push_str("\t\"fmt\"\n");
    output.push_str("\t\"math\"\n");
    output.push_str("\t\"reflect\"\n");
    output.push_str("\t\"strconv\"\n");
    output.push_str("\t\"strings\"\n");
    output.push_str(")\n\n");
    render_validator_core(&mut output);
    output
}

fn render_validator_core(output: &mut String) {
    output.push_str("// Violation is a single constraint failure. Path is the JSON member path\n");
    output.push_str("// (dotted for nested members); Reason is a human-readable message.\n");
    output.push_str("type Violation struct {\n\tPath   string\n\tReason string\n}\n\n");
    output.push_str(
        "// String implements fmt.Stringer, returning \"Path: Reason\", or just Reason\n",
    );
    output.push_str("// when Path is empty.\n");
    output.push_str("func (v Violation) String() string {\n");
    output.push_str("\tif v.Path == \"\" {\n\t\treturn v.Reason\n\t}\n");
    output.push_str("\treturn v.Path + \": \" + v.Reason\n}\n\n");
    output
        .push_str("// ValidationError aggregates every Violation found while (de)serializing a\n");
    output.push_str("// value, surfacing them all in one error (never a partial first-failure).\n");
    output.push_str("type ValidationError struct {\n\tViolations []Violation\n}\n\n");
    output.push_str("// Error implements the error interface, joining every Violation into one\n");
    output.push_str("// message.\n");
    output.push_str("func (e *ValidationError) Error() string {\n");
    output.push_str("\tparts := make([]string, len(e.Violations))\n");
    output.push_str("\tfor i, v := range e.Violations {\n\t\tparts[i] = v.String()\n\t}\n");
    output.push_str("\treturn fmt.Sprintf(\"%d validation error(s): %s\", len(e.Violations), strings.Join(parts, \"; \"))\n");
    output.push_str("}\n\n");
    output.push_str("func addViolations(errs *[]Violation, err error) {\n");
    output.push_str("\tif err == nil {\n\t\treturn\n\t}\n");
    output.push_str("\tvar ve *ValidationError\n");
    output.push_str("\tif errors.As(err, &ve) {\n\t\t*errs = append(*errs, ve.Violations...)\n\t\treturn\n\t}\n");
    output.push_str("\t*errs = append(*errs, Violation{\"\", err.Error()})\n}\n\n");
    output.push_str("func mergeNested(errs *[]Violation, path string, err error) {\n");
    output.push_str("\tif err == nil {\n\t\treturn\n\t}\n");
    output.push_str("\tvar ve *ValidationError\n");
    output.push_str("\tif errors.As(err, &ve) {\n");
    output.push_str("\t\tfor _, v := range ve.Violations {\n");
    output.push_str("\t\t\tp := v.Path\n\t\t\tif p == \"\" {\n\t\t\t\tp = path\n\t\t\t} else {\n\t\t\t\tp = path + \".\" + v.Path\n\t\t\t}\n");
    output.push_str(
        "\t\t\tif strings.HasPrefix(v.Path, \"[\") {\n\t\t\t\tp = path + v.Path\n\t\t\t}\n",
    );
    output
        .push_str("\t\t\t*errs = append(*errs, Violation{p, v.Reason})\n\t\t}\n\t\treturn\n\t}\n");
    output.push_str("\t*errs = append(*errs, Violation{path, err.Error()})\n}\n\n");
    output.push_str("const integerCap = 1<<53 - 1\n\n");
    output.push_str("var (\n\terrFractional = errors.New(\"not an integer\")\n\terrRange      = errors.New(\"exceeds ±(2^53-1) integer cap\")\n)\n\n");
    output.push_str("func parseSpecInteger(n json.Number) (int64, error) {\n");
    output.push_str("\ts := n.String()\n\tnegative := strings.HasPrefix(s, \"-\")\n\tif negative {\n\t\ts = s[1:]\n\t}\n");
    output.push_str("\texponentText := \"\"\n\tif i := strings.IndexAny(s, \"eE\"); i >= 0 {\n\t\texponentText = s[i+1:]\n\t\ts = s[:i]\n\t}\n");
    output.push_str("\tfractionalDigits := int64(0)\n\tif i := strings.IndexByte(s, '.'); i >= 0 {\n\t\tfractionalDigits = int64(len(s) - i - 1)\n\t\ts = s[:i] + s[i+1:]\n\t}\n");
    output.push_str(
        "\tdigits := strings.TrimLeft(s, \"0\")\n\tif digits == \"\" {\n\t\treturn 0, nil\n\t}\n",
    );
    output.push_str("\tvar exponent int64\n\tif exponentText != \"\" {\n\t\tvar err error\n\t\texponent, err = strconv.ParseInt(exponentText, 10, 64)\n\t\tif err != nil {\n\t\t\tif strings.HasPrefix(exponentText, \"-\") {\n\t\t\t\treturn 0, errFractional\n\t\t\t}\n\t\t\treturn 0, errRange\n\t\t}\n\t}\n");
    output.push_str("\tif exponent < -int64(len(digits)) {\n\t\treturn 0, errFractional\n\t}\n\tscale := exponent - fractionalDigits\n\tif scale < 0 {\n\t\ttrim := -scale\n\t\tif trim > int64(len(digits)) || strings.Trim(digits[len(digits)-int(trim):], \"0\") != \"\" {\n\t\t\treturn 0, errFractional\n\t\t}\n\t\tdigits = digits[:len(digits)-int(trim)]\n\t} else {\n\t\tif scale > int64(len(\"9007199254740991\")) {\n\t\t\treturn 0, errRange\n\t\t}\n\t\tdigits += strings.Repeat(\"0\", int(scale))\n\t}\n");
    output.push_str("\tconst capText = \"9007199254740991\"\n\tif len(digits) > len(capText) || len(digits) == len(capText) && digits > capText {\n\t\treturn 0, errRange\n\t}\n");
    output.push_str("\tv, err := strconv.ParseInt(digits, 10, 64)\n\tif err != nil {\n\t\treturn 0, errRange\n\t}\n\tif negative {\n\t\tv = -v\n\t}\n\treturn v, nil\n}\n\n");
    output.push_str("func isNilValue(v any) bool {\n\tif v == nil {\n\t\treturn true\n\t}\n\trv := reflect.ValueOf(v)\n\tswitch rv.Kind() {\n\tcase reflect.Chan, reflect.Func, reflect.Interface, reflect.Map, reflect.Pointer, reflect.Slice:\n\t\treturn rv.IsNil()\n\tdefault:\n\t\treturn false\n\t}\n}\n\n");
    output.push_str("func isNull(raw json.RawMessage) bool {\n\treturn bytes.Equal(bytes.TrimSpace(raw), []byte(\"null\"))\n}\n\n");
    output.push_str("func parseStringField(raw *json.RawMessage, path string, required, nullable bool, errs *[]Violation) (string, bool) {\n");
    output.push_str("\tif raw == nil {\n\t\tif required {\n\t\t\t*errs = append(*errs, Violation{path, \"required\"})\n\t\t}\n\t\treturn \"\", false\n\t}\n");
    output.push_str("\tif isNull(*raw) {\n\t\tif !nullable {\n\t\t\t*errs = append(*errs, Violation{path, \"explicit null not allowed\"})\n\t\t}\n\t\treturn \"\", false\n\t}\n");
    output.push_str("\tvar s string\n\tif err := json.Unmarshal(*raw, &s); err != nil {\n\t\t*errs = append(*errs, Violation{path, \"expected string\"})\n\t\treturn \"\", false\n\t}\n\treturn s, true\n}\n\n");
    output.push_str("func parseIntegerField(raw *json.RawMessage, path string, required, nullable bool, errs *[]Violation) (int64, bool) {\n");
    output.push_str("\tif raw == nil {\n\t\tif required {\n\t\t\t*errs = append(*errs, Violation{path, \"required\"})\n\t\t}\n\t\treturn 0, false\n\t}\n");
    output.push_str("\tif isNull(*raw) {\n\t\tif !nullable {\n\t\t\t*errs = append(*errs, Violation{path, \"explicit null not allowed\"})\n\t\t}\n\t\treturn 0, false\n\t}\n");
    output.push_str(
        "\tdec := json.NewDecoder(bytes.NewReader(*raw))\n\tdec.UseNumber()\n\tvar n json.Number\n",
    );
    output.push_str("\tif err := dec.Decode(&n); err != nil {\n\t\t*errs = append(*errs, Violation{path, \"expected integer\"})\n\t\treturn 0, false\n\t}\n");
    output.push_str("\tv, err := parseSpecInteger(n)\n\tif err != nil {\n\t\t*errs = append(*errs, Violation{path, err.Error()})\n\t\treturn 0, false\n\t}\n\treturn v, true\n}\n\n");
    output.push_str("func parseNumberField(raw *json.RawMessage, path string, required, nullable bool, errs *[]Violation) (float64, bool) {\n");
    output.push_str("\tif raw == nil {\n\t\tif required {\n\t\t\t*errs = append(*errs, Violation{path, \"required\"})\n\t\t}\n\t\treturn 0, false\n\t}\n");
    output.push_str("\tif isNull(*raw) {\n\t\tif !nullable {\n\t\t\t*errs = append(*errs, Violation{path, \"explicit null not allowed\"})\n\t\t}\n\t\treturn 0, false\n\t}\n");
    output.push_str(
        "\tdec := json.NewDecoder(bytes.NewReader(*raw))\n\tdec.UseNumber()\n\tvar n json.Number\n",
    );
    output.push_str("\tif err := dec.Decode(&n); err != nil {\n\t\t*errs = append(*errs, Violation{path, \"expected number\"})\n\t\treturn 0, false\n\t}\n");
    output.push_str("\tf, err := n.Float64()\n\tif err != nil || math.IsNaN(f) || math.IsInf(f, 0) {\n\t\t*errs = append(*errs, Violation{path, \"expected finite number\"})\n\t\treturn 0, false\n\t}\n\treturn f, true\n}\n\n");
    output.push_str("func parseBoolField(raw *json.RawMessage, path string, required, nullable bool, errs *[]Violation) (bool, bool) {\n");
    output.push_str("\tif raw == nil {\n\t\tif required {\n\t\t\t*errs = append(*errs, Violation{path, \"required\"})\n\t\t}\n\t\treturn false, false\n\t}\n");
    output.push_str("\tif isNull(*raw) {\n\t\tif !nullable {\n\t\t\t*errs = append(*errs, Violation{path, \"explicit null not allowed\"})\n\t\t}\n\t\treturn false, false\n\t}\n");
    output.push_str("\tvar b bool\n\tif err := json.Unmarshal(*raw, &b); err != nil {\n\t\t*errs = append(*errs, Violation{path, \"expected boolean\"})\n\t\treturn false, false\n\t}\n\treturn b, true\n}\n\n");
    output.push_str("func marshalField(out map[string]json.RawMessage, key string, v any, errs *[]Violation) {\n");
    output.push_str(
        "\tif len(*errs) > 0 {\n\t\tout[key] = json.RawMessage(\"null\")\n\t\treturn\n\t}\n",
    );
    output.push_str("\tb, err := json.Marshal(v)\n\tif err != nil {\n\t\tmergeNested(errs, key, err)\n\t\treturn\n\t}\n\tout[key] = b\n}\n\n");
}

/// Renders the closed-value (`const`/`enum`) defined types and their typed value
/// constants. Each scalar `const`/`enum` field synthesizes a defined type over
/// the primitive (`type ShowcaseStatus string`) plus one typed constant per
/// member (`const ShowcaseStatusActive ShowcaseStatus = "active"`). See
/// `specs/json-schema/features/{const,enum}.md`.
fn render_const_discriminators(output: &mut String, models: &[&PlannedJsonType]) -> Result<()> {
    struct Declared {
        type_name: String,
        underlying: &'static str,
        consts: Vec<(String, String)>,
        model_name: String,
        field_name: String,
        schema: Schema,
    }
    let mut declared = Vec::new();
    for model in models {
        let schema = decode_schema(model)?;
        let Some(properties) = &schema.properties else {
            continue;
        };
        for (field_name, property) in properties {
            if !is_closed_value_schema(property) {
                continue;
            }
            let type_name =
                const_type_name(&model.model_name, &property.go_member_name(field_name));
            let underlying = go_closed_underlying(property);
            let consts = closed_values(property)
                .iter()
                .map(|value| {
                    (
                        go_closed_value_name(property, &model.model_name, field_name, value),
                        go_closed_value_literal(value),
                    )
                })
                .collect::<Vec<_>>();
            declared.push(Declared {
                type_name,
                underlying,
                consts,
                model_name: model.model_name.clone(),
                field_name: field_name.clone(),
                schema: property.clone(),
            });
        }
    }
    if declared.is_empty() {
        return Ok(());
    }
    for decl in declared {
        render_go_schema_doc(
            output,
            "",
            &decl.type_name,
            &decl.schema,
            "type",
            &format!(
                "{} is the closed value set for {}.{}.",
                decl.type_name, decl.model_name, decl.field_name
            ),
        );
        output.push_str("type ");
        output.push_str(&decl.type_name);
        output.push(' ');
        output.push_str(decl.underlying);
        output.push_str("\n\n");
        if decl.consts.len() == 1 {
            let (name, literal) = &decl.consts[0];
            output.push_str("// ");
            output.push_str(name);
            output.push_str(" is the ");
            output.push_str(&decl.type_name);
            output.push_str(" value ");
            output.push_str(literal);
            output.push_str(".\n");
            output.push_str("const ");
            output.push_str(name);
            output.push(' ');
            output.push_str(&decl.type_name);
            output.push_str(" = ");
            output.push_str(literal);
            output.push_str("\n\n");
        } else {
            output.push_str("const (\n");
            for (name, literal) in &decl.consts {
                output.push_str("\t// ");
                output.push_str(name);
                output.push_str(" is the ");
                output.push_str(&decl.type_name);
                output.push_str(" value ");
                output.push_str(literal);
                output.push_str(".\n\t");
                output.push_str(name);
                output.push(' ');
                output.push_str(&decl.type_name);
                output.push_str(" = ");
                output.push_str(literal);
                output.push('\n');
            }
            output.push_str(")\n\n");
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// `oneOf` closed sum types (specs/json-schema/features/oneOf.md)
// ---------------------------------------------------------------------------

/// A single member of a Go union (sealed interface).
#[derive(Debug, Clone)]
struct GoUnionVariant {
    /// The concrete Go type used as the interface member (`Circle`,
    /// `ShowcaseIdOrNameString`, …).
    go_type: String,
    /// A synthesized wrapper type this union owns and must declare (a scalar or
    /// array branch), with its underlying Go type. `None` for a `$ref` object
    /// branch, whose named type already exists and just gains a marker method.
    synthesized: Option<String>,
    /// The leading wire-token bytes that select this branch (`"{"`, `"\""`,
    /// digits, …).
    tokens: Vec<char>,
    /// A human label for the "expected …" reason (`string`, `Circle`, …).
    label: String,
    /// For an object branch of a tagged union: its discriminant `const` value.
    discriminant_value: Option<Value>,
    /// The branch schema (resolved for a `$ref`), for wrapper `Validate`.
    schema: Schema,
    /// True when the concrete type has its own `Validate`/`UnmarshalJSON` (an
    /// object branch); false for a synthesized scalar/array wrapper.
    is_object: bool,
    /// Set for an **inline** object branch: the map shape this union declares as
    /// `<Union>Object` (Go needs a named type to carry the marker method). `None`
    /// for a `$ref` object branch, whose named model already exists. The loader
    /// admits only the free-form object inline, so a struct wrapping
    /// `AdditionalProperties` expresses the branch in full.
    owned_map: Option<GoMapShape>,
}

/// A Go closed sum type emitted as a sealed interface.
#[derive(Debug, Clone)]
struct GoUnion {
    name: String,
    nullable: bool,
    discriminant: Option<String>,
    variants: Vec<GoUnionVariant>,
}

impl GoUnion {
    fn marker_method(&self) -> String {
        format!("is{}", self.name)
    }

    fn admissible(&self) -> String {
        self.variants
            .iter()
            .map(|variant| variant.label.clone())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// The scalar discriminator `const` of a property (bare `const`, or a
/// single-member `enum`).
fn go_discriminator_const(property: &Schema) -> Option<Value> {
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

/// The `{name: const}` discriminator tags an object branch carries (required +
/// `const`).
fn go_branch_discriminator_tags(object: &Schema) -> BTreeMap<String, Value> {
    let required: BTreeSet<String> = object.required.iter().flatten().cloned().collect();
    let mut tags = BTreeMap::new();
    if let Some(properties) = &object.properties {
        for (name, property) in properties {
            if required.contains(name)
                && let Some(value) = go_discriminator_const(property)
            {
                tags.insert(name.clone(), value);
            }
        }
    }
    tags
}

/// Finds the planned model a resolved `$ref` points at (matched by its Go type
/// name).
fn find_ref_model<'a>(
    reference: &str,
    models: &'a [&PlannedJsonType],
    model_names: &BTreeMap<String, String>,
) -> Option<&'a PlannedJsonType> {
    let target = reference_model_name(reference, model_names);
    models
        .iter()
        .copied()
        .find(|model| model.model_name == target || model.full_name == target)
}

/// Classifies a `oneOf` schema into a Go union, or `None` when it is the
/// degenerate nullability pattern (fewer than two non-null branches) rather than
/// a sum type. The loader has already proven the sum-type invariants.
fn classify_go_union(
    union_name: &str,
    schema: &Schema,
    models: &[&PlannedJsonType],
    model_names: &BTreeMap<String, String>,
) -> Option<GoUnion> {
    let branches = schema.one_of.as_ref()?;
    let mut nullable = false;
    let mut variants: Vec<GoUnionVariant> = Vec::new();
    for branch in branches {
        let resolved = if let Some(reference) = &branch.reference {
            find_ref_model(reference, models, model_names)
                .and_then(|model| decode_schema(model).ok())
                .unwrap_or_else(|| branch.clone())
        } else {
            branch.clone()
        };
        let ty = resolved.ty.as_ref().and_then(Value::as_str);
        match ty {
            Some("null") => {
                nullable = true;
            }
            Some("object") => {
                // An inline branch has no named model, so the union declares the
                // variant type itself (`<Union>Object`).
                let owned_map = match &branch.reference {
                    Some(_) => None,
                    // A branch's own map shape is the free-form object's verbatim
                    // member map, which never holds a union.
                    None => go_map_shape(&resolved, model_names, &BTreeMap::new())
                        .ok()
                        .flatten(),
                };
                let go_type = branch
                    .reference
                    .as_ref()
                    .map(|reference| reference_model_name(reference, model_names))
                    .unwrap_or_else(|| format!("{union_name}Object"));
                variants.push(GoUnionVariant {
                    go_type: go_type.clone(),
                    synthesized: None,
                    tokens: vec!['{'],
                    label: go_type,
                    discriminant_value: None,
                    schema: resolved,
                    is_object: true,
                    owned_map,
                });
            }
            Some("string") => variants.push(GoUnionVariant {
                go_type: format!("{union_name}String"),
                synthesized: Some("string".to_string()),
                tokens: vec!['"'],
                label: "string".to_string(),
                discriminant_value: None,
                schema: resolved,
                is_object: false,
                owned_map: None,
            }),
            Some("integer") => variants.push(GoUnionVariant {
                go_type: format!("{union_name}Integer"),
                synthesized: Some("int64".to_string()),
                tokens: "-0123456789".chars().collect(),
                label: "integer".to_string(),
                discriminant_value: None,
                schema: resolved,
                is_object: false,
                owned_map: None,
            }),
            Some("number") => variants.push(GoUnionVariant {
                go_type: format!("{union_name}Number"),
                synthesized: Some("float64".to_string()),
                tokens: "-0123456789".chars().collect(),
                label: "number".to_string(),
                discriminant_value: None,
                schema: resolved,
                is_object: false,
                owned_map: None,
            }),
            Some("boolean") => variants.push(GoUnionVariant {
                go_type: format!("{union_name}Boolean"),
                synthesized: Some("bool".to_string()),
                tokens: vec!['t', 'f'],
                label: "boolean".to_string(),
                discriminant_value: None,
                schema: resolved,
                is_object: false,
                owned_map: None,
            }),
            Some("array") => {
                let item = resolved
                    .items
                    .as_ref()
                    .map(|item| go_type_annotation(item, "", model_names))
                    .transpose()
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| "any".to_string());
                variants.push(GoUnionVariant {
                    go_type: format!("{union_name}Array"),
                    synthesized: Some(format!("[]{item}")),
                    tokens: vec!['['],
                    label: format!("[]{item}"),
                    discriminant_value: None,
                    schema: resolved,
                    is_object: false,
                    owned_map: None,
                });
            }
            _ => {}
        }
    }
    if variants.len() < 2 {
        return None;
    }
    // Tagged object union: locate the shared required-`const` discriminator and
    // record each object variant's tag value.
    let object_count = variants.iter().filter(|variant| variant.is_object).count();
    let mut discriminant = None;
    if object_count >= 2 {
        let object_schemas: Vec<&Schema> = variants
            .iter()
            .filter(|variant| variant.is_object)
            .map(|variant| &variant.schema)
            .collect();
        let mut shared: Option<BTreeMap<String, Value>> = None;
        for object in &object_schemas {
            let tags = go_branch_discriminator_tags(object);
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
                    .filter_map(|object| go_branch_discriminator_tags(object).get(*name).cloned())
                    .collect();
                values
                    .iter()
                    .enumerate()
                    .all(|(index, value)| !values[..index].iter().any(|existing| existing == value))
            })
            .cloned();
        if let Some(name) = &name {
            for variant in variants.iter_mut().filter(|variant| variant.is_object) {
                variant.discriminant_value = go_branch_discriminator_tags(&variant.schema)
                    .get(name)
                    .cloned();
            }
        }
        discriminant = name;
    }
    Some(GoUnion {
        name: union_name.to_string(),
        nullable,
        discriminant,
        variants,
    })
}

/// A Pascal-case suffix for a union field name (`idOrName` → `IdOrName`).
fn go_union_field_suffix(json_name: &str) -> String {
    go_field_name(json_name)
}

/// Collects every union in the model set: named `$def` unions (a top-level
/// `oneOf`) and inline `oneOf` unions on object properties.
fn collect_go_unions(
    models: &[&PlannedJsonType],
    model_names: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, GoUnion>> {
    let mut unions = BTreeMap::new();
    for model in models {
        let schema = decode_schema(model)?;
        if schema.one_of.is_some() {
            if let Some(union) = classify_go_union(&model.model_name, &schema, models, model_names)
            {
                unions.insert(union.name.clone(), union);
            }
            continue;
        }
        if let Some(properties) = &schema.properties {
            for (json_name, property) in properties {
                if property.one_of.is_some() {
                    let name = format!("{}{}", model.model_name, go_union_field_suffix(json_name));
                    if let Some(union) = classify_go_union(&name, property, models, model_names) {
                        unions.insert(union.name.clone(), union);
                    }
                }
            }
        }
    }
    Ok(unions)
}

/// The union a `$ref` schema points at, if any.
fn union_reference_name(
    schema: &Schema,
    model_names: &BTreeMap<String, String>,
    unions: &BTreeMap<String, GoUnion>,
) -> Option<String> {
    let reference = schema.reference.as_ref()?;
    let name = reference_model_name(reference, model_names);
    unions.contains_key(&name).then_some(name)
}

/// The union type name a property carries, if any: an inline `oneOf` union named
/// `<Model><Field>`, or a `$ref` to a named union model.
fn property_union_name(
    model_name: &str,
    json_name: &str,
    property: &Schema,
    unions: &BTreeMap<String, GoUnion>,
) -> Option<String> {
    if property.one_of.is_some() {
        let name = format!("{model_name}{}", go_union_field_suffix(json_name));
        return unions.contains_key(&name).then_some(name);
    }
    if let Some(reference) = &property.reference {
        let name = reference_model_name(reference, &BTreeMap::new());
        if unions.contains_key(&name) {
            return Some(name);
        }
    }
    None
}

/// Emits every union's sealed interface, marker/`Validate` methods, synthesized
/// wrapper types, and dispatch function.
fn render_go_unions(
    output: &mut String,
    unions: &BTreeMap<String, GoUnion>,
    models: &[&PlannedJsonType],
    model_names: &BTreeMap<String, String>,
) -> Result<()> {
    for union in unions.values() {
        let union_schema = models
            .iter()
            .find(|model| model.model_name == union.name)
            .map(|model| decode_schema(model))
            .transpose()?
            .unwrap_or_default();
        render_go_schema_doc(
            output,
            "",
            &union.name,
            &union_schema,
            "type",
            &format!("{} is one of: {}.", union.name, union.admissible()),
        );
        output.push_str("type ");
        output.push_str(&union.name);
        output.push_str(" interface {\n\t");
        output.push_str(&union.marker_method());
        output.push_str("()\n\tValidate() error\n}\n\n");

        for variant in &union.variants {
            if let Some(underlying) = &variant.synthesized {
                // The compiled-regex vars the wrapper's `Validate` references for
                // a `pattern`/`format` the branch declares, keyed by the wrapper
                // type itself (`fooStringPattern`).
                render_go_string_vars_recursive(
                    output,
                    &variant.go_type,
                    UNION_VARIANT_POSITION,
                    &variant.schema,
                );
                render_go_schema_doc(
                    output,
                    "",
                    &variant.go_type,
                    &variant.schema,
                    "type",
                    &format!(
                        "{} wraps a {underlying} value admissible in the {} union.",
                        variant.go_type, union.name
                    ),
                );
                output.push_str("type ");
                output.push_str(&variant.go_type);
                output.push(' ');
                output.push_str(underlying);
                output.push_str("\n\n");
            }
            // An inline object branch: declare the map-shaped struct the union
            // owns, with the same (de)serialize/validate surface a named
            // map-shaped model gets.
            if let Some(shape) = &variant.owned_map {
                render_go_schema_doc(
                    output,
                    "",
                    &variant.go_type,
                    &variant.schema,
                    "type",
                    &format!(
                        "{} wraps the object admissible in the {} union.",
                        variant.go_type, union.name
                    ),
                );
                output.push_str("type ");
                output.push_str(&variant.go_type);
                output.push_str(" struct {\n");
                render_go_map_field(output, shape);
                output.push_str("}\n\n");
            }
            // Marker method.
            output.push_str("func (");
            output.push_str(&variant.go_type);
            output.push_str(") ");
            output.push_str(&union.marker_method());
            output.push_str("() {}\n\n");
            // A synthesized wrapper needs its own Validate (a `$ref` object
            // branch already has one on its named model).
            if variant.synthesized.is_some() {
                render_go_variant_validate(output, variant, model_names, unions);
            }
            if let Some(shape) = &variant.owned_map {
                render_go_map_methods(
                    output,
                    &variant.go_type,
                    &variant.schema,
                    shape,
                    model_names,
                    unions,
                );
            }
        }
        render_go_union_dispatch(output, union, model_names, unions);
    }
    Ok(())
}

/// The position name a union's synthesized `<Union><Kind>` variant contributes to
/// a compiled-regex var: none, because the variant type name already identifies
/// the branch (`fooStringPattern`).
const UNION_VARIANT_POSITION: &str = "";

/// The wrapper `Validate` for a synthesized scalar/array variant: runs every
/// predicate the branch declares, so the branch's own constraints are enforced
/// on the way in (the dispatcher calls this) and again before emit (P12) — the
/// same predicates, with the same reasons, the property position runs for a value
/// of that type ([[oneOf]] §"Validator mapping").
fn render_go_variant_validate(
    output: &mut String,
    variant: &GoUnionVariant,
    model_names: &BTreeMap<String, String>,
    unions: &BTreeMap<String, GoUnion>,
) {
    output
        .push_str("// Validate checks v against every constraint and returns a *ValidationError\n");
    output.push_str("// listing any violations.\n");
    output.push_str("func (v ");
    output.push_str(&variant.go_type);
    output.push_str(") Validate() error {\n\tvar errs []Violation\n");
    let underlying = variant.synthesized.as_deref().unwrap_or("");
    // The wrapper is a defined type over `underlying`, so every predicate reads
    // the value through a conversion back to it.
    render_go_member_checks(
        output,
        "\t",
        &format!("{underlying}(v)"),
        "\"\"",
        &variant.go_type,
        UNION_VARIANT_POSITION,
        &variant.schema,
        true,
    );
    if variant.schema.ty.as_ref().and_then(Value::as_str) == Some("array") {
        render_go_array_items_validate(
            output,
            "\t",
            &format!("{underlying}(v)"),
            "\"\"",
            &variant.schema,
            &variant.go_type,
            UNION_VARIANT_POSITION,
            model_names,
            unions,
            0,
        );
    }
    output.push_str("\tif len(errs) > 0 {\n\t\treturn &ValidationError{Violations: errs}\n\t}\n\treturn nil\n}\n\n");
    if variant.schema.ty.as_ref().and_then(Value::as_str) == Some("array")
        && schema_requires_go_wire_conversion(&variant.schema)
    {
        output.push_str("// MarshalJSON validates v, converts nested values, and serializes it.\n");
        output.push_str("func (v ");
        output.push_str(&variant.go_type);
        output.push_str(") MarshalJSON() ([]byte, error) {\n\tif err := v.Validate(); err != nil {\n\t\treturn nil, err\n\t}\n");
        let wire = render_go_array_wire_value(
            output,
            "\t",
            &format!("{underlying}(v)"),
            &variant.schema,
            &variant.go_type,
            0,
        );
        output.push_str("\treturn json.Marshal(");
        output.push_str(&wire);
        output.push_str(")\n}\n\n");
    }
}

/// Emits `unmarshal<Union>`: peeks the wire token (then, for a tagged object
/// union, the discriminant) and routes to exactly one branch, or records a
/// Violation naming the admissible members.
fn render_go_union_dispatch(
    output: &mut String,
    union: &GoUnion,
    model_names: &BTreeMap<String, String>,
    unions: &BTreeMap<String, GoUnion>,
) {
    let admissible = union.admissible();
    output.push_str("func unmarshal");
    output.push_str(&union.name);
    output.push_str("(raw json.RawMessage, path string, errs *[]Violation) (");
    output.push_str(&union.name);
    output.push_str(", bool) {\n");
    output.push_str("\ttrimmed := bytes.TrimSpace(raw)\n");
    output.push_str(&format!(
        "\tif len(trimmed) == 0 {{\n\t\t*errs = append(*errs, Violation{{path, {}}})\n\t\treturn nil, false\n\t}}\n",
        go_string_literal(&format!("expected one of: {admissible}"))
    ));
    output.push_str("\tswitch trimmed[0] {\n");

    // Object branch(es).
    let object_variants: Vec<&GoUnionVariant> = union
        .variants
        .iter()
        .filter(|variant| variant.is_object)
        .collect();
    if !object_variants.is_empty() {
        output.push_str("\tcase '{':\n");
        if let Some(discriminant) = &union.discriminant {
            output.push_str("\t\tvar obj map[string]json.RawMessage\n");
            output.push_str("\t\tif err := json.Unmarshal(trimmed, &obj); err != nil {\n\t\t\t*errs = append(*errs, Violation{path, \"expected object\"})\n\t\t\treturn nil, false\n\t\t}\n");
            output.push_str(&format!(
                "\t\tdiscRaw, ok := obj[{}]\n",
                go_string_literal(discriminant)
            ));
            output.push_str(&format!(
                "\t\tif !ok {{\n\t\t\t*errs = append(*errs, Violation{{path, {}}})\n\t\t\treturn nil, false\n\t\t}}\n",
                go_string_literal(&format!("discriminator {discriminant:?} is required"))
            ));
            output.push_str("\t\tswitch string(bytes.TrimSpace(discRaw)) {\n");
            let mut values_display = Vec::new();
            for variant in &object_variants {
                let Some(value) = &variant.discriminant_value else {
                    continue;
                };
                let literal = serde_json::to_string(value).unwrap_or_default();
                values_display.push(literal.clone());
                output.push_str(&format!("\t\tcase {}:\n", go_string_literal(&literal)));
                output.push_str(&format!("\t\t\tvar v {}\n", variant.go_type));
                output.push_str("\t\t\tmergeNested(errs, path, json.Unmarshal(trimmed, &v))\n");
                output.push_str("\t\t\treturn v, true\n");
            }
            output.push_str("\t\tdefault:\n");
            output.push_str(&format!(
                "\t\t\t*errs = append(*errs, Violation{{path, fmt.Sprintf({}, string(bytes.TrimSpace(discRaw)))}})\n",
                go_string_literal(&format!(
                    "unknown discriminator {discriminant} %s: expected one of [{}]",
                    values_display.join(", ")
                ))
            ));
            output.push_str("\t\t\treturn nil, false\n");
            output.push_str("\t\t}\n");
        } else {
            let variant = object_variants[0];
            output.push_str(&format!("\t\tvar v {}\n", variant.go_type));
            output.push_str("\t\tmergeNested(errs, path, json.Unmarshal(trimmed, &v))\n");
            output.push_str("\t\treturn v, true\n");
        }
    }

    // Scalar / array branches.
    for variant in union.variants.iter().filter(|variant| !variant.is_object) {
        let cases = variant
            .tokens
            .iter()
            .map(|token| go_rune_literal(*token))
            .collect::<Vec<_>>()
            .join(", ");
        output.push_str(&format!("\tcase {cases}:\n"));
        render_go_scalar_variant_decode(output, variant, model_names, unions);
    }

    output.push_str("\t}\n");
    output.push_str(&format!(
        "\t*errs = append(*errs, Violation{{path, {}}})\n\treturn nil, false\n}}\n\n",
        go_string_literal(&format!("expected one of: {admissible}"))
    ));
}

/// A Go rune literal for a wire-token byte (`'{'`, `'"'`, `'-'`, digits). Only
/// the single-quote and backslash need escaping; the double-quote is literal in
/// a rune literal.
fn go_rune_literal(token: char) -> String {
    match token {
        '\'' => "'\\''".to_string(),
        '\\' => "'\\\\'".to_string(),
        other => format!("'{other}'"),
    }
}

fn render_go_scalar_variant_decode(
    output: &mut String,
    variant: &GoUnionVariant,
    model_names: &BTreeMap<String, String>,
    unions: &BTreeMap<String, GoUnion>,
) {
    let underlying = variant.synthesized.as_deref().unwrap_or("");
    match underlying {
        "string" => {
            output.push_str("\t\tvar s string\n");
            output.push_str("\t\tif err := json.Unmarshal(trimmed, &s); err != nil {\n\t\t\t*errs = append(*errs, Violation{path, \"expected string\"})\n\t\t\treturn nil, false\n\t\t}\n");
            output.push_str(&format!("\t\tv := {}(s)\n", variant.go_type));
        }
        "bool" => {
            output.push_str("\t\tvar b bool\n");
            output.push_str("\t\tif err := json.Unmarshal(trimmed, &b); err != nil {\n\t\t\t*errs = append(*errs, Violation{path, \"expected boolean\"})\n\t\t\treturn nil, false\n\t\t}\n");
            output.push_str(&format!("\t\tv := {}(b)\n", variant.go_type));
        }
        "int64" => {
            output.push_str("\t\tdec := json.NewDecoder(bytes.NewReader(trimmed))\n\t\tdec.UseNumber()\n\t\tvar n json.Number\n");
            output.push_str("\t\tif err := dec.Decode(&n); err != nil {\n\t\t\t*errs = append(*errs, Violation{path, \"expected integer\"})\n\t\t\treturn nil, false\n\t\t}\n");
            output.push_str("\t\tiv, err := parseSpecInteger(n)\n\t\tif err != nil {\n\t\t\t*errs = append(*errs, Violation{path, err.Error()})\n\t\t\treturn nil, false\n\t\t}\n");
            output.push_str(&format!("\t\tv := {}(iv)\n", variant.go_type));
        }
        "float64" => {
            output.push_str("\t\tdec := json.NewDecoder(bytes.NewReader(trimmed))\n\t\tdec.UseNumber()\n\t\tvar n json.Number\n");
            output.push_str("\t\tif err := dec.Decode(&n); err != nil {\n\t\t\t*errs = append(*errs, Violation{path, \"expected number\"})\n\t\t\treturn nil, false\n\t\t}\n");
            output.push_str("\t\tfv, err := n.Float64()\n\t\tif err != nil || math.IsNaN(fv) || math.IsInf(fv, 0) {\n\t\t\t*errs = append(*errs, Violation{path, \"expected finite number\"})\n\t\t\treturn nil, false\n\t\t}\n");
            output.push_str(&format!("\t\tv := {}(fv)\n", variant.go_type));
        }
        other if other.starts_with("[]") => {
            output.push_str(&format!("\t\tvar arr {other}\n"));
            render_go_array_position_unmarshal(
                output,
                "\t\t",
                "trimmed",
                "path",
                "arr",
                other,
                &variant.schema,
                &variant.go_type,
                UNION_VARIANT_POSITION,
                model_names,
                unions,
                0,
                &GoErrsBinding::BY_POINTER,
            );
            output.push_str(&format!("\t\tv := {}(arr)\n", variant.go_type));
        }
        _ => {
            output.push_str("\t\treturn nil, false\n");
            return;
        }
    }
    if underlying.starts_with("[]") {
        // Item validation renders against a value-owned accumulator. Adapt the
        // union dispatcher's pointer here without re-running array-level
        // constraints against the partially converted typed slice.
        output.push_str("\t\t{\n\t\t\tunionErrs := errs\n\t\t\terrs := *unionErrs\n");
        render_go_array_items_validate(
            output,
            "\t\t\t",
            "arr",
            "path",
            &variant.schema,
            &variant.go_type,
            UNION_VARIANT_POSITION,
            model_names,
            unions,
            0,
        );
        output.push_str("\t\t\t*unionErrs = errs\n\t\t}\n");
    } else {
        output.push_str("\t\tmergeNested(errs, path, v.Validate())\n");
    }
    output.push_str("\t\treturn v, true\n");
}

fn render_model(
    output: &mut String,
    model: &PlannedJsonType,
    _models: &[&PlannedJsonType],
    model_names: &BTreeMap<String, String>,
    unions: &BTreeMap<String, GoUnion>,
) -> Result<()> {
    let schema = decode_schema(model)?;
    // A named `oneOf` union model is emitted as a sealed interface by
    // `render_go_unions`, not as a struct.
    if schema.one_of.is_some() && unions.contains_key(&model.model_name) {
        return Ok(());
    }
    render_go_pattern_vars(output, model, &schema);
    render_go_schema_doc(
        output,
        "",
        &model.model_name,
        &schema,
        "type",
        &format!(
            "{} is generated from the corresponding JSON Schema definition.",
            model.model_name
        ),
    );
    output.push_str("type ");
    output.push_str(&model.model_name);
    output.push_str(" struct {\n");
    if let Some(shape) = go_map_shape(&schema, model_names, unions)? {
        render_go_map_field(output, &shape);
        output.push_str("}\n\n");
        render_go_map_methods(
            output,
            &model.model_name,
            &schema,
            &shape,
            model_names,
            unions,
        );
        return Ok(());
    }

    let required = required_fields(&schema);
    let properties = schema.properties.as_ref();
    if let Some(properties) = properties {
        for (json_name, property) in properties {
            let field_name = property.go_member_name(json_name);
            render_go_schema_doc(
                output,
                "\t",
                &field_name,
                property,
                "field",
                &format!("{field_name} corresponds to the {json_name:?} JSON property."),
            );
            output.push('\t');
            output.push_str(&field_name);
            output.push(' ');
            if let Some(union_name) =
                property_union_name(&model.model_name, json_name, property, unions)
            {
                output.push_str(&union_name);
            } else {
                output.push_str(&go_property_type(
                    &model.model_name,
                    json_name,
                    property,
                    required.contains(json_name),
                    model_names,
                )?);
            }
            output.push_str(" `json:\"");
            output.push_str(json_name);
            if !required.contains(json_name) {
                output.push_str(",omitempty");
            }
            output.push_str("\"`\n");
        }
    }
    if is_open_object(&schema) {
        output.push_str("\t// AdditionalProperties holds unknown members verbatim.\n");
        output.push_str("\tAdditionalProperties map[string]json.RawMessage `json:\"-\"`\n");
    }
    output.push_str("}\n\n");
    render_default_accessors(output, model, &schema)?;
    render_validate(output, model, &schema, model_names, unions)?;
    render_unmarshal_json(output, model, &schema, model_names, unions)?;
    render_marshal_json(output, model, &schema, unions)?;
    Ok(())
}

fn render_default_accessors(
    output: &mut String,
    model: &PlannedJsonType,
    schema: &Schema,
) -> Result<()> {
    let Some(properties) = &schema.properties else {
        return Ok(());
    };
    for (json_name, property) in properties {
        let Some(default) = &property.default else {
            continue;
        };
        // Materialize-on-read for every scalar kind (P9/P12): the bare field
        // stays `*T` (set-ness intact); the accessor returns the pointee when
        // set and the schema default literal when nil. Modeled on proto3's
        // `GetX()`. See specs/json-schema/features/default.md.
        let Some((return_type, literal)) = go_default_type_and_literal(property, default) else {
            continue;
        };
        let field = property.go_member_name(json_name);
        output.push_str("// ");
        output.push_str(&field);
        output.push_str("OrDefault returns ");
        output.push_str(&field);
        output.push_str(" when set, else the schema default.\n");
        output.push_str("func (m ");
        output.push_str(&model.model_name);
        output.push_str(") ");
        output.push_str(&field);
        output.push_str("OrDefault() ");
        output.push_str(&return_type);
        output.push_str(" {\n\tif m.");
        output.push_str(&field);
        output.push_str(" != nil {\n\t\treturn *m.");
        output.push_str(&field);
        output.push_str("\n\t}\n\treturn ");
        output.push_str(&literal);
        output.push_str("\n}\n\n");
    }
    Ok(())
}

/// The Go return type and literal for a scalar `default` on `property`. Returns
/// `None` for a composite default (loader-rejected). The type follows the
/// declared `type` (loader-enforced scalar-compatible), falling back to the
/// literal's own kind for a typeless (nullable `oneOf`) member.
fn go_default_type_and_literal(property: &Schema, default: &Value) -> Option<(String, String)> {
    let declared = property.ty.as_ref().and_then(Value::as_str);
    match default {
        Value::String(text) => Some(("string".to_string(), go_string_literal(text))),
        Value::Bool(flag) => Some(("bool".to_string(), flag.to_string())),
        Value::Number(number) => {
            let is_integer = declared == Some("integer")
                || (declared.is_none()
                    && number.as_f64().is_some_and(|value| value.fract() == 0.0));
            if is_integer {
                Some(("int64".to_string(), go_bound_literal(number, true)))
            } else {
                Some(("float64".to_string(), go_bound_literal(number, false)))
            }
        }
        _ => None,
    }
}

fn render_validate(
    output: &mut String,
    model: &PlannedJsonType,
    schema: &Schema,
    model_names: &BTreeMap<String, String>,
    unions: &BTreeMap<String, GoUnion>,
) -> Result<()> {
    output
        .push_str("// Validate checks m against every constraint and returns a *ValidationError\n");
    output.push_str("// listing any violations.\n");
    output.push_str("func (m ");
    output.push_str(&model.model_name);
    output.push_str(") Validate() error {\n\tvar errs []Violation\n");
    // A map-shaped model's `Validate` is emitted by `render_go_map_methods`; this
    // one only ever sees a struct.
    if let Some(properties) = &schema.properties {
        for (json_name, property) in properties {
            let field = format!("m.{}", property.go_member_name(json_name));
            if property_union_name(&model.model_name, json_name, property, unions).is_some() {
                // A union field is a sealed interface; re-run the held branch's
                // constraints (P12), and enforce presence for a required member.
                if required_fields(schema).contains(json_name) {
                    output.push_str("\tif ");
                    output.push_str(&field);
                    output.push_str(" == nil {\n\t\terrs = append(errs, Violation{");
                    output.push_str(&go_string_literal(json_name));
                    output.push_str(", \"required\"})\n\t} else {\n\t\tmergeNested(&errs, ");
                    output.push_str(&go_string_literal(json_name));
                    output.push_str(", ");
                    output.push_str(&field);
                    output.push_str(".Validate())\n\t}\n");
                } else {
                    output.push_str("\tif ");
                    output.push_str(&field);
                    output.push_str(" != nil {\n\t\tmergeNested(&errs, ");
                    output.push_str(&go_string_literal(json_name));
                    output.push_str(", ");
                    output.push_str(&field);
                    output.push_str(".Validate())\n\t}\n");
                }
                continue;
            }
            if matches!(
                temporal_kind(property),
                Some(
                    crate::json_schema::format::TemporalKind::DateTime
                        | crate::json_schema::format::TemporalKind::Date
                )
            ) {
                let required = required_fields(schema).contains(json_name);
                let pointer = !required || allows_null(property);
                if pointer {
                    output.push_str("\tif ");
                    output.push_str(&field);
                    output.push_str(" != nil {\n");
                }
                output.push_str(if pointer { "\t\tif " } else { "\tif " });
                if pointer {
                    output.push_str("(*");
                    output.push_str(&field);
                    output.push_str(").Year() < 1");
                } else {
                    output.push_str(&field);
                    output.push_str(".Year() < 1");
                }
                output.push_str(" {\n");
                output.push_str(if pointer { "\t\t\t" } else { "\t\t" });
                output.push_str("errs = append(errs, Violation{");
                output.push_str(&go_string_literal(json_name));
                output.push_str(", \"year must be >= 1\"})\n");
                output.push_str(if pointer { "\t\t}\n\t}\n" } else { "\t}\n" });
            }
            if is_closed_value_schema(property) {
                render_go_closed_validate(
                    output,
                    &model.model_name,
                    json_name,
                    property,
                    required_fields(schema).contains(json_name),
                );
            }
            if property.ty.as_ref().and_then(Value::as_str) == Some("integer")
                && !is_closed_value_schema(property)
            {
                let expr = if required_fields(schema).contains(json_name) {
                    field.clone()
                } else {
                    format!("*{field}")
                };
                let guard = if required_fields(schema).contains(json_name) {
                    String::new()
                } else {
                    format!("{field} != nil && ")
                };
                output.push_str("\tif ");
                output.push_str(&guard);
                output.push('(');
                output.push_str(&expr);
                output.push_str(" < -integerCap || ");
                output.push_str(&expr);
                output.push_str(" > integerCap) {\n\t\terrs = append(errs, Violation{");
                output.push_str(&go_string_literal(json_name));
                output.push_str(", \"exceeds ±(2^53-1) integer cap\"})\n\t}\n");
            }
            if property.ty.as_ref().and_then(Value::as_str) == Some("number") {
                let required = required_fields(schema).contains(json_name);
                let mut expr = if required {
                    field.clone()
                } else {
                    format!("*{field}")
                };
                if is_closed_value_schema(property) {
                    expr = format!("float64({expr})");
                }
                if !required {
                    output.push_str("\tif ");
                    output.push_str(&field);
                    output.push_str(" != nil {\n");
                }
                output.push_str(if required { "\tif " } else { "\t\tif " });
                output.push_str("math.IsNaN(");
                output.push_str(&expr);
                output.push_str(") || math.IsInf(");
                output.push_str(&expr);
                output.push_str(", 0) {\n");
                output.push_str(if required { "\t\t" } else { "\t\t\t" });
                output.push_str("errs = append(errs, Violation{");
                output.push_str(&go_string_literal(json_name));
                output.push_str(", fmt.Sprintf(\"must be a finite number, got %v\", ");
                output.push_str(&expr);
                output.push_str(")})\n");
                output.push_str(if required { "\t}\n" } else { "\t\t}\n\t}\n" });
            }
            if property.has_numeric_constraints()
                && matches!(
                    property.ty.as_ref().and_then(Value::as_str),
                    Some("integer" | "number")
                )
            {
                let required = required_fields(schema).contains(json_name);
                if required {
                    render_go_numeric_checks(
                        output,
                        &field,
                        &go_string_literal(json_name),
                        property,
                        "\t",
                    );
                } else {
                    output.push_str("\tif ");
                    output.push_str(&field);
                    output.push_str(" != nil {\n");
                    render_go_numeric_checks(
                        output,
                        &format!("*{field}"),
                        &go_string_literal(json_name),
                        property,
                        "\t\t",
                    );
                    output.push_str("\t}\n");
                }
            }
            if property.has_string_constraints()
                && property.ty.as_ref().and_then(Value::as_str) == Some("string")
                && temporal_kind(property).is_none()
            {
                let var_name = go_pattern_var_name(&model.model_name, json_name);
                let format_var = go_format_var_name(&model.model_name, json_name);
                if required_fields(schema).contains(json_name) {
                    render_go_string_checks(
                        output,
                        &field,
                        &go_string_literal(json_name),
                        property,
                        "\t",
                    );
                    if let Some(pattern) = &property.pattern {
                        render_go_pattern_check(
                            output,
                            &field,
                            &go_string_literal(json_name),
                            &var_name,
                            pattern,
                            "\t",
                        );
                    }
                    if let Some(format) = &property.format {
                        render_go_format_check(
                            output,
                            &field,
                            &go_string_literal(json_name),
                            &format_var,
                            format,
                            "\t",
                        );
                    }
                } else {
                    output.push_str("\tif ");
                    output.push_str(&field);
                    output.push_str(" != nil {\n");
                    render_go_string_checks(
                        output,
                        &format!("*{field}"),
                        &go_string_literal(json_name),
                        property,
                        "\t\t",
                    );
                    if let Some(pattern) = &property.pattern {
                        render_go_pattern_check(
                            output,
                            &format!("*{field}"),
                            &go_string_literal(json_name),
                            &var_name,
                            pattern,
                            "\t\t",
                        );
                    }
                    if let Some(format) = &property.format {
                        render_go_format_check(
                            output,
                            &format!("*{field}"),
                            &go_string_literal(json_name),
                            &format_var,
                            format,
                            "\t\t",
                        );
                    }
                    output.push_str("\t}\n");
                }
            }
            if property.has_array_constraints()
                && property.ty.as_ref().and_then(Value::as_str) == Some("array")
            {
                if required_fields(schema).contains(json_name) {
                    render_go_array_checks(
                        output,
                        &field,
                        &go_string_literal(json_name),
                        property,
                        "\t",
                    );
                } else {
                    output.push_str("\tif ");
                    output.push_str(&field);
                    output.push_str(" != nil {\n");
                    render_go_array_checks(
                        output,
                        &field,
                        &go_string_literal(json_name),
                        property,
                        "\t\t",
                    );
                    output.push_str("\t}\n");
                }
            }
            if property.ty.as_ref().and_then(Value::as_str) == Some("array") {
                render_go_array_items_validate(
                    output,
                    "\t",
                    &field,
                    &go_string_literal(json_name),
                    property,
                    &model.model_name,
                    json_name,
                    model_names,
                    unions,
                    0,
                );
            }
            if property.reference.is_some() {
                if required_fields(schema).contains(json_name) {
                    output.push_str("\tmergeNested(&errs, ");
                    output.push_str(&go_string_literal(json_name));
                    output.push_str(", ");
                    output.push_str(&field);
                    output.push_str(".Validate())\n");
                } else {
                    output.push_str("\tif ");
                    output.push_str(&field);
                    output.push_str(" != nil {\n\t\tmergeNested(&errs, ");
                    output.push_str(&go_string_literal(json_name));
                    output.push_str(", ");
                    output.push_str(&field);
                    output.push_str(".Validate())\n\t}\n");
                }
            }
            let _ = model_names;
        }
    }
    output.push_str("\tif len(errs) > 0 {\n\t\treturn &ValidationError{Violations: errs}\n\t}\n\treturn nil\n}\n\n");
    Ok(())
}

fn render_unmarshal_json(
    output: &mut String,
    model: &PlannedJsonType,
    schema: &Schema,
    model_names: &BTreeMap<String, String>,
    unions: &BTreeMap<String, GoUnion>,
) -> Result<()> {
    output.push_str("// UnmarshalJSON parses data into m and validates it, returning a\n");
    output.push_str("// *ValidationError listing any violations.\n");
    output.push_str("func (m *");
    output.push_str(&model.model_name);
    output.push_str(") UnmarshalJSON(data []byte) error {\n");
    output.push_str("\tvar all map[string]json.RawMessage\n\tif err := json.Unmarshal(data, &all); err != nil {\n\t\treturn err\n\t}\n\tvar errs []Violation\n");
    let required = required_fields(schema);
    if is_open_object(schema) {
        output.push_str("\tm.AdditionalProperties = map[string]json.RawMessage{}\n");
        output.push_str("\tfor k, v := range all {\n\t\tswitch k {\n");
    } else {
        output.push_str("\tfor k := range all {\n\t\tswitch k {\n");
    }
    let property_names = schema
        .properties
        .as_ref()
        .map(|properties| properties.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    if !property_names.is_empty() {
        output.push_str("\t\tcase ");
        output.push_str(
            &property_names
                .iter()
                .map(|name| go_string_literal(name))
                .collect::<Vec<_>>()
                .join(", "),
        );
        output.push_str(":\n");
    }
    output.push_str("\t\tdefault:\n");
    if is_open_object(schema) {
        output.push_str("\t\t\tm.AdditionalProperties[k] = v\n");
    } else {
        output.push_str("\t\t\terrs = append(errs, Violation{k, \"unknown field\"})\n");
    }
    output.push_str("\t\t}\n\t}\n");
    output.push_str("\tget := func(k string) *json.RawMessage {\n\t\tif v, ok := all[k]; ok {\n\t\t\treturn &v\n\t\t}\n\t\treturn nil\n\t}\n\t_ = get\n");
    if let Some(properties) = &schema.properties {
        for (json_name, property) in properties {
            if let Some(union_name) =
                property_union_name(&model.model_name, json_name, property, unions)
            {
                render_union_property_unmarshal(
                    output,
                    json_name,
                    &property.go_member_name(json_name),
                    &union_name,
                    required.contains(json_name),
                    unions,
                );
                continue;
            }
            render_property_unmarshal(
                output,
                model,
                json_name,
                property,
                required.contains(json_name),
                model_names,
                unions,
            )?;
        }
    }
    // Object member-count and cross-field constraints over the wire member set
    // (`all` holds every distinct wire key, before default population).
    render_go_property_count_checks(output, "len(all)", schema, "\t");
    render_go_dependent_required(output, "all", schema, "\t");
    output.push_str("\tif len(errs) > 0 {\n\t\treturn &ValidationError{Violations: errs}\n\t}\n\treturn nil\n}\n\n");
    Ok(())
}

/// Decodes a union-typed field: reject an absent required member / an explicit
/// `null` on a non-nullable union, otherwise dispatch on the wire token.
fn render_union_property_unmarshal(
    output: &mut String,
    json_name: &str,
    field_name: &str,
    union_name: &str,
    required: bool,
    unions: &BTreeMap<String, GoUnion>,
) {
    let nullable = unions.get(union_name).is_some_and(|union| union.nullable);
    output.push_str("\tif raw := get(");
    output.push_str(&go_string_literal(json_name));
    output.push_str("); raw == nil {\n");
    if required {
        output.push_str("\t\terrs = append(errs, Violation{");
        output.push_str(&go_string_literal(json_name));
        output.push_str(", \"required\"})\n");
    }
    output.push_str("\t} else if isNull(*raw) {\n");
    if !nullable {
        output.push_str("\t\terrs = append(errs, Violation{");
        output.push_str(&go_string_literal(json_name));
        output.push_str(", \"explicit null not allowed\"})\n");
    }
    output.push_str("\t} else if v, ok := unmarshal");
    output.push_str(union_name);
    output.push_str("(*raw, ");
    output.push_str(&go_string_literal(json_name));
    output.push_str(", &errs); ok {\n\t\tm.");
    output.push_str(field_name);
    output.push_str(" = v\n\t}\n");
}

fn render_property_unmarshal(
    output: &mut String,
    model: &PlannedJsonType,
    json_name: &str,
    property: &Schema,
    required: bool,
    model_names: &BTreeMap<String, String>,
    unions: &BTreeMap<String, GoUnion>,
) -> Result<()> {
    let field = property.go_member_name(json_name);
    // A materialized temporal: read the wire string, then parse into the native
    // construct via the parse adapter (regex + calendar/overflow over the wire).
    if let Some(kind) = temporal_kind(property) {
        render_temporal_property_unmarshal(
            output,
            json_name,
            &field,
            kind,
            required,
            allows_null(property),
        );
        return Ok(());
    }
    // A materialized `contentEncoding`: read the wire string, run the pinned
    // regex + any co-occurring wire-string constraints, then decode into `[]byte`
    // via the stdlib codec.
    if let Some(encoding) = content_encoding_kind(property) {
        render_content_encoding_property_unmarshal(
            output,
            &model.model_name,
            json_name,
            &field,
            encoding,
            required,
            allows_null(property),
            property,
        );
        return Ok(());
    }
    if is_closed_value_schema(property) {
        render_closed_value_unmarshal(output, &model.model_name, json_name, property, required);
        return Ok(());
    }
    if let Some(non_null) = nullable_non_null_schema(property)
        && non_null.reference.is_some()
    {
        let model_type = go_type_annotation(non_null, json_name, model_names)?;
        render_reference_property_unmarshal(output, json_name, &field, &model_type, required, true);
        return Ok(());
    }
    if property.reference.is_some() {
        let model_type = go_type_annotation(property, json_name, model_names)?;
        render_reference_property_unmarshal(
            output,
            json_name,
            &field,
            &model_type,
            required,
            false,
        );
        return Ok(());
    }
    if property.ty.as_ref().and_then(Value::as_str) == Some("string") {
        output.push_str("\tif v, ok := parseStringField(get(");
        output.push_str(&go_string_literal(json_name));
        output.push_str("), ");
        output.push_str(&go_string_literal(json_name));
        output.push_str(", ");
        output.push_str(if required { "true" } else { "false" });
        output.push_str(", ");
        output.push_str(if allows_null(property) {
            "true"
        } else {
            "false"
        });
        output.push_str(", &errs); ok {\n\t\tm.");
        output.push_str(&field);
        output.push_str(" = ");
        if required {
            output.push_str("v\n");
        } else {
            output.push_str("&v\n");
        }
        if property.has_string_constraints() {
            render_go_string_checks(output, "v", &go_string_literal(json_name), property, "\t\t");
        }
        if let Some(pattern) = &property.pattern {
            render_go_pattern_check(
                output,
                "v",
                &go_string_literal(json_name),
                &go_pattern_var_name(&model.model_name, json_name),
                pattern,
                "\t\t",
            );
        }
        if let Some(format) = &property.format {
            render_go_format_check(
                output,
                "v",
                &go_string_literal(json_name),
                &go_format_var_name(&model.model_name, json_name),
                format,
                "\t\t",
            );
        }
        output.push_str("\t}\n");
        return Ok(());
    }
    if property.ty.as_ref().and_then(Value::as_str) == Some("integer") {
        output.push_str("\tif v, ok := parseIntegerField(get(");
        output.push_str(&go_string_literal(json_name));
        output.push_str("), ");
        output.push_str(&go_string_literal(json_name));
        output.push_str(", ");
        output.push_str(if required { "true" } else { "false" });
        output.push_str(", false, &errs); ok {\n");
        if property.has_numeric_constraints() {
            render_go_numeric_checks(output, "v", &go_string_literal(json_name), property, "\t\t");
        }
        output.push_str("\t\tm.");
        output.push_str(&field);
        output.push_str(" = ");
        if required {
            output.push_str("v\n");
        } else {
            output.push_str("&v\n");
        }
        output.push_str("\t}\n");
        return Ok(());
    }
    if property.ty.as_ref().and_then(Value::as_str) == Some("number") {
        output.push_str("\tif v, ok := parseNumberField(get(");
        output.push_str(&go_string_literal(json_name));
        output.push_str("), ");
        output.push_str(&go_string_literal(json_name));
        output.push_str(", ");
        output.push_str(if required { "true" } else { "false" });
        output.push_str(", false, &errs); ok {\n");
        if property.has_numeric_constraints() {
            render_go_numeric_checks(output, "v", &go_string_literal(json_name), property, "\t\t");
        }
        output.push_str("\t\tm.");
        output.push_str(&field);
        output.push_str(" = ");
        if required {
            output.push_str("v\n");
        } else {
            output.push_str("&v\n");
        }
        output.push_str("\t}\n");
        return Ok(());
    }
    if property.ty.as_ref().and_then(Value::as_str) == Some("boolean") {
        output.push_str("\tif v, ok := parseBoolField(get(");
        output.push_str(&go_string_literal(json_name));
        output.push_str("), ");
        output.push_str(&go_string_literal(json_name));
        output.push_str(", ");
        output.push_str(if required { "true" } else { "false" });
        output.push_str(", ");
        output.push_str(if allows_null(property) {
            "true"
        } else {
            "false"
        });
        output.push_str(", &errs); ok {\n\t\tm.");
        output.push_str(&field);
        output.push_str(" = ");
        if required {
            output.push_str("v\n");
        } else {
            output.push_str("&v\n");
        }
        output.push_str("\t}\n");
        return Ok(());
    }
    if property.ty.as_ref().and_then(Value::as_str) == Some("array") {
        output.push_str("\tif raw := get(");
        output.push_str(&go_string_literal(json_name));
        output.push_str("); raw == nil {\n");
        if required {
            output.push_str("\t\terrs = append(errs, Violation{");
            output.push_str(&go_string_literal(json_name));
            output.push_str(", \"required\"})\n");
        }
        output.push_str("\t} else if isNull(*raw) {\n\t\terrs = append(errs, Violation{");
        output.push_str(&go_string_literal(json_name));
        output.push_str(", \"explicit null not allowed\"})\n");
        output.push_str("\t} else {\n");
        let element_type = go_type_annotation(property, json_name, model_names)?;
        render_go_array_position_unmarshal(
            output,
            "\t\t",
            "*raw",
            &go_string_literal(json_name),
            &format!("m.{field}"),
            &element_type,
            property,
            &model.model_name,
            json_name,
            model_names,
            unions,
            0,
            &GoErrsBinding::BY_VALUE,
        );
        output.push_str("\t}\n");
        return Ok(());
    }
    if allows_null(property) {
        output.push_str("\tif v, ok := parseStringField(get(");
        output.push_str(&go_string_literal(json_name));
        output.push_str("), ");
        output.push_str(&go_string_literal(json_name));
        output.push_str(", ");
        output.push_str(if required { "true" } else { "false" });
        output.push_str(", true, &errs); ok {\n\t\tm.");
        output.push_str(&field);
        output.push_str(" = &v\n\t}\n");
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn render_go_array_items_validate(
    output: &mut String,
    indent: &str,
    target: &str,
    path: &str,
    array: &Schema,
    model_name: &str,
    position: &str,
    model_names: &BTreeMap<String, String>,
    unions: &BTreeMap<String, GoUnion>,
    level: usize,
) {
    let Some(item) = array.items.as_deref() else {
        return;
    };
    if !schema_requires_go_validation(item) {
        return;
    }
    let non_null = nullable_non_null_schema(item).unwrap_or(item);
    let item_type = go_element_type_annotation(item, position, model_names)
        .expect("the loader produced a supported Go array item type");
    let index = format!("i{level}");
    let element = format!("v{level}");
    let element_path = format!("p{level}");
    let inner = format!("{indent}\t");
    output.push_str(&format!(
        "{indent}for {index}, {element} := range {target} {{\n{inner}{element_path} := fmt.Sprintf(\"%s[%d]\", {path}, {index})\n"
    ));
    let nullable_nilable =
        allows_null(item) && (item_type.starts_with('*') || item_type.starts_with("[]"));
    if nullable_nilable {
        output.push_str(&format!(
            "{inner}if {element} == nil {{\n{inner}\tcontinue\n{inner}}}\n"
        ));
    }
    let value_expr = if item_type.starts_with('*') {
        format!("(*{element})")
    } else {
        element.clone()
    };
    let item_position = format!("{position}Item");
    if union_reference_name(non_null, model_names, unions).is_some() {
        output.push_str(&format!("{inner}if isNilValue({element}) {{\n"));
        if allows_null(item) {
            output.push_str(&format!("{inner}\tcontinue\n"));
        } else {
            output.push_str(&format!(
                "{inner}\terrs = append(errs, Violation{{{element_path}, \"explicit null not allowed\"}})\n"
            ));
        }
        output.push_str(&format!(
            "{inner}}} else {{\n{inner}\tmergeNested(&errs, {element_path}, {value_expr}.Validate())\n{inner}}}\n"
        ));
    } else if non_null.reference.is_some() {
        output.push_str(&format!(
            "{inner}mergeNested(&errs, {element_path}, {value_expr}.Validate())\n"
        ));
    } else if non_null.ty.as_ref().and_then(Value::as_str) == Some("array") {
        let nested_indent = if allows_null(item) {
            inner.clone()
        } else {
            format!("{inner}\t")
        };
        if !allows_null(item) {
            output.push_str(&format!(
                "{inner}if {element} == nil {{\n{inner}\terrs = append(errs, Violation{{{element_path}, \"explicit null not allowed\"}})\n{inner}}} else {{\n"
            ));
        }
        render_go_array_checks(output, &value_expr, &element_path, non_null, &nested_indent);
        render_go_array_items_validate(
            output,
            &nested_indent,
            &value_expr,
            &element_path,
            non_null,
            model_name,
            &item_position,
            model_names,
            unions,
            level + 1,
        );
        if !allows_null(item) {
            output.push_str(&format!("{inner}}}\n"));
        }
    } else if let Some(encoding) = content_encoding_kind(non_null) {
        let wire = format!("wire{level}");
        output.push_str(&format!(
            "{inner}{wire} := {}({value_expr})\n",
            go_content_encoding_encode_fn(encoding)
        ));
        if non_null.min_length.is_some() || non_null.max_length.is_some() {
            render_go_string_checks(output, &wire, &element_path, non_null, &inner);
        }
        if let Some(pattern) = &non_null.pattern {
            render_go_pattern_check(
                output,
                &wire,
                &element_path,
                &go_pattern_var_name(model_name, &item_position),
                pattern,
                &inner,
            );
        }
    } else if matches!(
        temporal_kind(non_null),
        Some(
            crate::json_schema::format::TemporalKind::DateTime
                | crate::json_schema::format::TemporalKind::Date
        )
    ) {
        output.push_str(&format!(
            "{inner}if {value_expr}.Year() < 1 {{\n{inner}\terrs = append(errs, Violation{{{element_path}, \"year must be >= 1\"}})\n{inner}}}\n"
        ));
    } else if temporal_kind(non_null).is_none() {
        render_go_member_checks(
            output,
            &inner,
            &value_expr,
            &element_path,
            model_name,
            &item_position,
            non_null,
            true,
        );
    }
    output.push_str(&format!("{indent}}}\n"));
}

fn schema_requires_go_validation(schema: &Schema) -> bool {
    let schema = nullable_non_null_schema(schema).unwrap_or(schema);
    is_closed_value_schema(schema)
        || schema.reference.is_some()
        || schema.one_of.is_some()
        || schema.ty.as_ref().and_then(Value::as_str) == Some("integer")
        || schema.ty.as_ref().and_then(Value::as_str) == Some("number")
        || schema.has_numeric_constraints()
        || schema.has_string_constraints()
        || schema.has_array_constraints()
        || matches!(
            temporal_kind(schema),
            Some(
                crate::json_schema::format::TemporalKind::DateTime
                    | crate::json_schema::format::TemporalKind::Date
            )
        )
        || schema
            .items
            .as_deref()
            .is_some_and(schema_requires_go_validation)
}

fn schema_requires_go_wire_conversion(schema: &Schema) -> bool {
    let schema = nullable_non_null_schema(schema).unwrap_or(schema);
    temporal_kind(schema).is_some()
        || content_encoding_kind(schema).is_some()
        || schema
            .items
            .as_deref()
            .is_some_and(schema_requires_go_wire_conversion)
}

fn render_go_array_wire_value(
    output: &mut String,
    indent: &str,
    value_expr: &str,
    array: &Schema,
    position: &str,
    level: usize,
) -> String {
    let suffix = format!("{}{}", go_field_name(position), level);
    let wire = format!("wire{suffix}");
    let index = format!("item{suffix}");
    output.push_str(&format!(
        "{indent}{wire} := make([]any, 0, len({value_expr}))\n{indent}for _, {index} := range {value_expr} {{\n"
    ));
    let inner = format!("{indent}\t");
    let Some(item) = array.items.as_deref() else {
        output.push_str(&format!(
            "{inner}{wire} = append({wire}, {index})\n{indent}}}\n"
        ));
        return wire;
    };
    let non_null = nullable_non_null_schema(item).unwrap_or(item);
    let nilable = allows_null(item)
        && (go_type_annotation(item, position, &BTreeMap::new())
            .is_ok_and(|ty| ty.starts_with('*') || ty.starts_with("[]")));
    if nilable {
        output.push_str(&format!(
            "{inner}if {index} == nil {{\n{inner}\t{wire} = append({wire}, nil)\n{inner}\tcontinue\n{inner}}}\n"
        ));
    }
    let item_expr = if go_type_annotation(item, position, &BTreeMap::new())
        .is_ok_and(|ty| ty.starts_with('*'))
    {
        format!("(*{index})")
    } else {
        index.clone()
    };
    let item_position = format!("{position}Item");
    if non_null.ty.as_ref().and_then(Value::as_str) == Some("array") {
        let nested = render_go_array_wire_value(
            output,
            &inner,
            &item_expr,
            non_null,
            &item_position,
            level + 1,
        );
        output.push_str(&format!("{inner}{wire} = append({wire}, {nested})\n"));
    } else if let Some(kind) = temporal_kind(non_null) {
        output.push_str(&format!(
            "{inner}{wire} = append({wire}, {}({item_expr}))\n",
            go_temporal_format_fn(kind)
        ));
    } else if let Some(encoding) = content_encoding_kind(non_null) {
        output.push_str(&format!(
            "{inner}{wire} = append({wire}, {}({item_expr}))\n",
            go_content_encoding_encode_fn(encoding)
        ));
    } else {
        output.push_str(&format!("{inner}{wire} = append({wire}, {item_expr})\n"));
    }
    output.push_str(&format!("{indent}}}\n"));
    wire
}

/// How the enclosing scope names its violation accumulator: a model's
/// (de)serializer holds `errs` by value, a union dispatcher by pointer.
struct GoErrsBinding {
    /// The accumulator as an addressable `[]Violation` (`errs` / `*errs`).
    value: &'static str,
    /// The accumulator as a `*[]Violation` argument (`&errs` / `errs`).
    pointer: &'static str,
}

impl GoErrsBinding {
    const BY_VALUE: Self = Self {
        value: "errs",
        pointer: "&errs",
    };
    const BY_POINTER: Self = Self {
        value: "*errs",
        pointer: "errs",
    };
}

/// Decodes an ordinary array recursively instead of handing the whole slice to
/// `encoding/json`. Every item therefore uses the same spec-strict adapter as a
/// property (integer spellings, nullability, temporal/content conversion,
/// nested model/union dispatch), and failures retain every array index.
#[allow(clippy::too_many_arguments)]
fn render_go_array_position_unmarshal(
    output: &mut String,
    indent: &str,
    raw: &str,
    path: &str,
    target: &str,
    slice_type: &str,
    array: &Schema,
    model_name: &str,
    position: &str,
    model_names: &BTreeMap<String, String>,
    unions: &BTreeMap<String, GoUnion>,
    level: usize,
    errs: &GoErrsBinding,
) {
    let elems = format!("elems{level}");
    let index = format!("i{level}");
    let element = format!("e{level}");
    let element_path = format!("p{level}");
    let inner = format!("{indent}\t");
    let body = format!("{inner}\t");
    let (errs_value, errs_ref) = (errs.value, errs.pointer);
    output.push_str(&format!("{indent}var {elems} []json.RawMessage\n"));
    output.push_str(&format!(
        "{indent}if err := json.Unmarshal({raw}, &{elems}); err != nil {{\n{inner}{errs_value} = append({errs_value}, Violation{{{path}, \"expected array\"}})\n{indent}}} else {{\n"
    ));
    output.push_str(&format!(
        "{inner}{target} = make({slice_type}, 0, len({elems}))\n{inner}for {index}, {element} := range {elems} {{\n{body}{element_path} := fmt.Sprintf(\"%s[%d]\", {path}, {index})\n"
    ));
    let Some(item) = array.items.as_deref() else {
        output.push_str(&format!(
            "{body}var value{level} any\n{body}if err := json.Unmarshal({element}, &value{level}); err != nil {{\n{body}\tmergeNested({errs_ref}, {element_path}, err)\n{body}\tcontinue\n{body}}}\n{body}{target} = append({target}, value{level})\n{inner}}}\n{indent}}}\n"
        ));
        return;
    };
    let item_type = go_element_type_annotation(item, position, model_names)
        .expect("the loader produced a supported Go array item type");
    let non_null = nullable_non_null_schema(item).unwrap_or(item);
    if allows_null(item) {
        output.push_str(&format!(
            "{body}if isNull({element}) {{\n{body}\t{target} = append({target}, nil)\n{body}\tcontinue\n{body}}}\n"
        ));
    } else {
        output.push_str(&format!(
            "{body}if isNull({element}) {{\n{body}\t{errs_value} = append({errs_value}, Violation{{{element_path}, \"explicit null not allowed\"}})\n{body}\tcontinue\n{body}}}\n"
        ));
    }
    let append_value = |output: &mut String, value: &str| {
        output.push_str(&format!("{body}{target} = append({target}, "));
        if item_type.starts_with('*') {
            output.push('&');
        }
        output.push_str(value);
        output.push_str(")\n");
    };
    let item_position = format!("{position}Item");
    if let Some(union_name) = union_reference_name(non_null, model_names, unions) {
        output.push_str(&format!(
            "{body}if value{level}, ok := unmarshal{union_name}({element}, {element_path}, {errs_ref}); ok {{\n{body}\t{target} = append({target}, value{level})\n{body}}}\n"
        ));
    } else if non_null.ty.as_ref().and_then(Value::as_str) == Some("array") {
        let row = format!("value{level}");
        let row_type = item_type.trim_start_matches('*');
        output.push_str(&format!("{body}var {row} {row_type}\n"));
        render_go_array_position_unmarshal(
            output,
            &body,
            &element,
            &element_path,
            &row,
            row_type,
            non_null,
            model_name,
            &item_position,
            model_names,
            unions,
            level + 1,
            errs,
        );
        append_value(output, &row);
    } else if let Some(kind) = temporal_kind(non_null) {
        let parse_fn = go_temporal_parse_fn(kind);
        output.push_str(&format!(
            "{body}if s{level}, ok := parseStringField(&{element}, {element_path}, true, false, {errs_ref}); ok {{\n{body}\tif value{level}, ok := {parse_fn}({element_path}, s{level}, {errs_ref}); ok {{\n"
        ));
        output.push_str(&format!("{body}\t\t{target} = append({target}, "));
        if item_type.starts_with('*') {
            output.push('&');
        }
        output.push_str(&format!("value{level})\n{body}\t}}\n{body}}}\n"));
    } else if let Some(encoding) = content_encoding_kind(non_null) {
        let decode_fn = go_content_encoding_decode_fn(encoding);
        let re_var = go_content_encoding_var_name(model_name, &item_position);
        output.push_str(&format!(
            "{body}if s{level}, ok := parseStringField(&{element}, {element_path}, true, false, {errs_ref}); ok {{\n"
        ));
        if non_null.min_length.is_some() || non_null.max_length.is_some() {
            render_go_string_checks(
                output,
                &format!("s{level}"),
                &element_path,
                non_null,
                &format!("{body}\t"),
            );
        }
        if let Some(pattern) = &non_null.pattern {
            render_go_pattern_check(
                output,
                &format!("s{level}"),
                &element_path,
                &go_pattern_var_name(model_name, &item_position),
                pattern,
                &format!("{body}\t"),
            );
        }
        output.push_str(&format!(
            "{body}\tif value{level}, ok := {decode_fn}({element_path}, s{level}, {re_var}, {errs_ref}); ok {{\n{body}\t\t{target} = append({target}, value{level})\n{body}\t}}\n{body}}}\n"
        ));
    } else if non_null.reference.is_some() {
        let decoded_type = item_type.trim_start_matches('*');
        output.push_str(&format!(
            "{body}var value{level} {decoded_type}\n{body}if err := json.Unmarshal({element}, &value{level}); err != nil {{\n{body}\tmergeNested({errs_ref}, {element_path}, err)\n{body}}} else {{\n"
        ));
        output.push_str(&format!("{body}\t{target} = append({target}, "));
        if item_type.starts_with('*') {
            output.push('&');
        }
        output.push_str(&format!("value{level})\n{body}}}\n"));
    } else {
        let helper = match non_null.ty.as_ref().and_then(Value::as_str) {
            Some("string") => Some("parseStringField"),
            Some("integer") => Some("parseIntegerField"),
            Some("number") => Some("parseNumberField"),
            Some("boolean") => Some("parseBoolField"),
            _ => None,
        };
        if let Some(helper) = helper {
            output.push_str(&format!(
                "{body}if value{level}, ok := {helper}(&{element}, {element_path}, true, false, {errs_ref}); ok {{\n"
            ));
            // A union dispatcher holds the accumulator by pointer and validates
            // the completed array through its wrapper below. Ordinary model/map
            // decoders hold `errs` by value and run the item checks here.
            if errs.value == "errs" {
                render_go_member_checks(
                    output,
                    &format!("{body}\t"),
                    &format!("value{level}"),
                    &element_path,
                    model_name,
                    &item_position,
                    non_null,
                    false,
                );
            }
            output.push_str(&format!("{body}\t{target} = append({target}, "));
            if item_type.starts_with('*') {
                output.push('&');
            }
            output.push_str(&format!("value{level})\n{body}}}\n"));
        } else {
            let decoded_type = item_type.trim_start_matches('*');
            output.push_str(&format!(
                "{body}var value{level} {decoded_type}\n{body}if err := json.Unmarshal({element}, &value{level}); err != nil {{\n{body}\tmergeNested({errs_ref}, {element_path}, err)\n{body}}} else {{\n{body}\t{target} = append({target}, value{level})\n{body}}}\n"
            ));
        }
    }
    output.push_str(&format!("{inner}}}\n"));
    if array.has_array_constraints() {
        render_go_raw_array_checks(output, &elems, path, array, &inner, level, errs);
    }
    output.push_str(&format!("{indent}}}\n"));
}

/// Decodes a materialized temporal field: read the wire `string` (presence /
/// null handling via `parseStringField`), then parse it into the native
/// construct through the parse adapter. A required + non-nullable field is a
/// value; anything optional or nullable is a pointer.
fn render_temporal_property_unmarshal(
    output: &mut String,
    json_name: &str,
    field: &str,
    kind: crate::json_schema::format::TemporalKind,
    required: bool,
    nullable: bool,
) {
    let parse_fn = go_temporal_parse_fn(kind);
    let path = go_string_literal(json_name);
    output.push_str("\tif s, ok := parseStringField(get(");
    output.push_str(&path);
    output.push_str("), ");
    output.push_str(&path);
    output.push_str(", ");
    output.push_str(if required { "true" } else { "false" });
    output.push_str(", ");
    output.push_str(if nullable { "true" } else { "false" });
    output.push_str(", &errs); ok {\n\t\tif v, ok := ");
    output.push_str(parse_fn);
    output.push('(');
    output.push_str(&path);
    output.push_str(", s, &errs); ok {\n\t\t\tm.");
    output.push_str(field);
    output.push_str(" = ");
    if required && !nullable {
        output.push_str("v\n");
    } else {
        output.push_str("&v\n");
    }
    output.push_str("\t\t}\n\t}\n");
}

fn render_reference_property_unmarshal(
    output: &mut String,
    json_name: &str,
    field: &str,
    model_type: &str,
    required: bool,
    nullable: bool,
) {
    output.push_str("\tif raw := get(");
    output.push_str(&go_string_literal(json_name));
    output.push_str("); raw == nil {\n");
    if required {
        output.push_str("\t\terrs = append(errs, Violation{");
        output.push_str(&go_string_literal(json_name));
        output.push_str(", \"required\"})\n");
    }
    output.push_str("\t} else if isNull(*raw) {\n");
    if !nullable {
        output.push_str("\t\terrs = append(errs, Violation{");
        output.push_str(&go_string_literal(json_name));
        output.push_str(", \"explicit null not allowed\"})\n");
    }
    output.push_str("\t} else {\n");
    if required && !nullable {
        output.push_str("\t\tmergeNested(&errs, ");
        output.push_str(&go_string_literal(json_name));
        output.push_str(", json.Unmarshal(*raw, &m.");
        output.push_str(field);
        output.push_str("))\n");
    } else {
        output.push_str("\t\tvar tmp ");
        output.push_str(model_type);
        output.push_str(
            "\n\t\tif err := json.Unmarshal(*raw, &tmp); err != nil {\n\t\t\tmergeNested(&errs, ",
        );
        output.push_str(&go_string_literal(json_name));
        output.push_str(", err)\n\t\t} else {\n\t\t\tm.");
        output.push_str(field);
        output.push_str(" = &tmp\n\t\t}\n");
    }
    output.push_str("\t}\n");
}

fn render_marshal_json(
    output: &mut String,
    model: &PlannedJsonType,
    schema: &Schema,
    unions: &BTreeMap<String, GoUnion>,
) -> Result<()> {
    output.push_str("// MarshalJSON validates m, then serializes it to JSON, returning a\n");
    output.push_str("// *ValidationError if validation fails.\n");
    output.push_str("func (m ");
    output.push_str(&model.model_name);
    output.push_str(") MarshalJSON() ([]byte, error) {\n");
    output.push_str("\tvar errs []Violation\n\taddViolations(&errs, m.Validate())\n");
    output.push_str("\tout := map[string]json.RawMessage{}\n");
    if is_open_object(schema) {
        output.push_str("\tfor k, v := range m.AdditionalProperties {\n\t\tout[k] = v\n\t}\n");
    }
    let required = required_fields(schema);
    if let Some(properties) = &schema.properties {
        for (json_name, property) in properties {
            let field = format!("m.{}", property.go_member_name(json_name));
            if let Some(kind) = temporal_kind(property) {
                render_temporal_property_marshal(
                    output,
                    json_name,
                    &field,
                    kind,
                    required.contains(json_name),
                    allows_null(property),
                );
                continue;
            }
            if let Some(encoding) = content_encoding_kind(property) {
                render_content_encoding_property_marshal(
                    output,
                    &model.model_name,
                    json_name,
                    &field,
                    encoding,
                    required.contains(json_name),
                    property,
                );
                continue;
            }
            if let Some(union_name) =
                property_union_name(&model.model_name, json_name, property, unions)
            {
                // A union field marshals its held branch (interface dispatch);
                // an absent optional union is omitted, a required+nullable one
                // emits `null`.
                let nullable = required.contains(json_name)
                    && unions.get(&union_name).is_some_and(|union| union.nullable);
                output.push_str("\tif ");
                output.push_str(&field);
                output.push_str(" != nil {\n\t\tmarshalField(out, ");
                output.push_str(&go_string_literal(json_name));
                output.push_str(", ");
                output.push_str(&field);
                output.push_str(", &errs)\n\t}");
                if nullable {
                    output.push_str(" else {\n\t\tout[");
                    output.push_str(&go_string_literal(json_name));
                    output.push_str("] = json.RawMessage(\"null\")\n\t}");
                }
                output.push('\n');
                continue;
            }
            if property.ty.as_ref().and_then(Value::as_str) == Some("array")
                && schema_requires_go_wire_conversion(property)
            {
                if required.contains(json_name) {
                    let wire =
                        render_go_array_wire_value(output, "\t", &field, property, json_name, 0);
                    output.push_str("\tmarshalField(out, ");
                    output.push_str(&go_string_literal(json_name));
                    output.push_str(", ");
                    output.push_str(&wire);
                    output.push_str(", &errs)\n");
                } else {
                    output.push_str("\tif ");
                    output.push_str(&field);
                    output.push_str(" != nil {\n");
                    let wire =
                        render_go_array_wire_value(output, "\t\t", &field, property, json_name, 0);
                    output.push_str("\t\tmarshalField(out, ");
                    output.push_str(&go_string_literal(json_name));
                    output.push_str(", ");
                    output.push_str(&wire);
                    output.push_str(", &errs)\n\t}\n");
                }
                continue;
            }
            if required.contains(json_name) {
                if allows_null(property) {
                    output.push_str("\tif ");
                    output.push_str(&field);
                    output.push_str(" != nil {\n\t\tmarshalField(out, ");
                    output.push_str(&go_string_literal(json_name));
                    output.push_str(", *");
                    output.push_str(&field);
                    output.push_str(", &errs)\n\t} else {\n\t\tout[");
                    output.push_str(&go_string_literal(json_name));
                    output.push_str("] = json.RawMessage(\"null\")\n\t}\n");
                } else {
                    output.push_str("\tmarshalField(out, ");
                    output.push_str(&go_string_literal(json_name));
                    output.push_str(", ");
                    output.push_str(&field);
                    output.push_str(", &errs)\n");
                }
            } else {
                output.push_str("\tif ");
                output.push_str(&field);
                output.push_str(" != nil {\n\t\tmarshalField(out, ");
                output.push_str(&go_string_literal(json_name));
                output.push_str(", ");
                if property.ty.as_ref().and_then(Value::as_str) != Some("array") {
                    output.push('*');
                }
                output.push_str(&field);
                output.push_str(", &errs)\n\t}\n");
            }
        }
    }
    // Object member-count and cross-field constraints over the to-be-emitted
    // member set (`out` holds every key that will actually be written).
    render_go_property_count_checks(output, "len(out)", schema, "\t");
    render_go_dependent_required(output, "out", schema, "\t");
    output.push_str("\tif len(errs) > 0 {\n\t\treturn nil, &ValidationError{Violations: errs}\n\t}\n\treturn json.Marshal(out)\n}\n\n");
    Ok(())
}

/// Serializes a materialized temporal field through the generator-owned
/// serializer (never the native `time.Time`/`Duration` `MarshalJSON`, which
/// would emit a full RFC 3339 datetime for date/time and diverge on duration).
fn render_temporal_property_marshal(
    output: &mut String,
    json_name: &str,
    field: &str,
    kind: crate::json_schema::format::TemporalKind,
    required: bool,
    nullable: bool,
) {
    let format_fn = go_temporal_format_fn(kind);
    let path = go_string_literal(json_name);
    if required && !nullable {
        output.push_str("\tmarshalField(out, ");
        output.push_str(&path);
        output.push_str(", ");
        output.push_str(format_fn);
        output.push('(');
        output.push_str(field);
        output.push_str("), &errs)\n");
        return;
    }
    // Optional or nullable: a pointer. Absent → omitted; a required+nullable
    // field that is nil emits an explicit `null`.
    output.push_str("\tif ");
    output.push_str(field);
    output.push_str(" != nil {\n\t\tmarshalField(out, ");
    output.push_str(&path);
    output.push_str(", ");
    output.push_str(format_fn);
    output.push_str("(*");
    output.push_str(field);
    output.push_str("), &errs)\n\t}");
    if required && nullable {
        output.push_str(" else {\n\t\tout[");
        output.push_str(&path);
        output.push_str("] = json.RawMessage(\"null\")\n\t}");
    }
    output.push('\n');
}

/// Emits the `AdditionalProperties map[string]T` member of a map-shaped model.
/// The caller has already opened the struct declaration.
fn render_go_map_field(output: &mut String, shape: &GoMapShape) {
    output.push_str("\t// AdditionalProperties holds every member of this map-shaped object.\n");
    output.push_str("\tAdditionalProperties map[string]");
    output.push_str(&shape.element_type);
    output.push('\n');
}

/// Emits `Validate`/`UnmarshalJSON`/`MarshalJSON` for a map-shaped model: the
/// member-count and key-shape constraints apply to the whole map, and each
/// member decodes through its element type's parse adapter (P12).
fn render_go_map_methods(
    output: &mut String,
    type_name: &str,
    schema: &Schema,
    shape: &GoMapShape,
    model_names: &BTreeMap<String, String>,
    unions: &BTreeMap<String, GoUnion>,
) {
    let element_type = shape.element_type.as_str();
    output
        .push_str("// Validate checks m against every constraint and returns a *ValidationError\n");
    output.push_str("// listing any violations.\n");
    output.push_str("func (m ");
    output.push_str(type_name);
    output.push_str(") Validate() error {\n\tvar errs []Violation\n");
    render_go_property_count_checks(output, "len(m.AdditionalProperties)", schema, "\t");
    if let Some(subschema) = &schema.property_names {
        render_go_property_name_checks(output, "m.AdditionalProperties", subschema, "\t");
    }
    // Every member is re-checked against `T` before emit (P12): a named member
    // type through its own `Validate`, a scalar or array member through the same
    // predicates the property position runs.
    if let Some(value) = &shape.value_schema
        && schema_requires_go_validation(value)
    {
        output.push_str("\tfor k, v := range m.AdditionalProperties {\n");
        let (subject, indent) = if shape.nullable {
            output.push_str("\t\tif v == nil {\n\t\t\tcontinue\n\t\t}\n");
            ("(*v)".to_string(), "\t\t")
        } else {
            ("v".to_string(), "\t\t")
        };
        let non_null = nullable_non_null_schema(value).unwrap_or(value);
        if union_reference_name(non_null, model_names, unions).is_some() {
            output.push_str(indent);
            output.push_str("if isNilValue(");
            output.push_str(&subject);
            output.push_str(") {\n");
            if !allows_null(value) {
                output.push_str(indent);
                output.push_str(
                    "\terrs = append(errs, Violation{k, \"explicit null not allowed\"})\n",
                );
            }
            output.push_str(indent);
            output.push_str("} else {\n");
            output.push_str(indent);
            output.push_str("\tmergeNested(&errs, k, ");
            output.push_str(&subject);
            output.push_str(".Validate())\n");
            output.push_str(indent);
            output.push_str("}\n");
        } else if shape.element == GoMapElement::Model {
            output.push_str(indent);
            output.push_str("mergeNested(&errs, k, ");
            output.push_str(&subject);
            output.push_str(".Validate())\n");
        } else if non_null.ty.as_ref().and_then(Value::as_str) == Some("array") {
            render_go_member_checks(
                output,
                indent,
                &subject,
                "k",
                type_name,
                MAP_MEMBER_POSITION,
                non_null,
                true,
            );
            render_go_array_items_validate(
                output,
                indent,
                &subject,
                "k",
                non_null,
                type_name,
                MAP_MEMBER_POSITION,
                model_names,
                unions,
                0,
            );
        } else if let Some(encoding) = content_encoding_kind(non_null) {
            output.push_str(indent);
            output.push_str("wire := ");
            output.push_str(go_content_encoding_encode_fn(encoding));
            output.push('(');
            output.push_str(&subject);
            output.push_str(")\n");
            if non_null.min_length.is_some() || non_null.max_length.is_some() {
                render_go_string_checks(output, "wire", "k", non_null, indent);
            }
            if let Some(pattern) = &non_null.pattern {
                render_go_pattern_check(
                    output,
                    "wire",
                    "k",
                    &go_pattern_var_name(type_name, MAP_MEMBER_POSITION),
                    pattern,
                    indent,
                );
            }
        } else if matches!(
            temporal_kind(non_null),
            Some(
                crate::json_schema::format::TemporalKind::DateTime
                    | crate::json_schema::format::TemporalKind::Date
            )
        ) {
            output.push_str(indent);
            output.push_str("if ");
            output.push_str(&subject);
            output.push_str(".Year() < 1 {\n");
            output.push_str(indent);
            output.push_str("\terrs = append(errs, Violation{k, \"year must be >= 1\"})\n");
            output.push_str(indent);
            output.push_str("}\n");
        } else if temporal_kind(non_null).is_none() {
            render_go_member_checks(
                output,
                indent,
                &subject,
                "k",
                type_name,
                MAP_MEMBER_POSITION,
                non_null,
                true,
            );
        }
        output.push_str("\t}\n");
    }
    output.push_str("\tif len(errs) > 0 {\n\t\treturn &ValidationError{Violations: errs}\n\t}\n\treturn nil\n}\n\n");

    output.push_str("// UnmarshalJSON parses data into m and validates it, returning a\n");
    output.push_str("// *ValidationError listing any violations.\n");
    output.push_str("func (m *");
    output.push_str(type_name);
    output.push_str(") UnmarshalJSON(data []byte) error {\n");
    output.push_str("\tvar raw map[string]json.RawMessage\n\tif err := json.Unmarshal(data, &raw); err != nil {\n\t\treturn err\n\t}\n\tvar errs []Violation\n");
    output.push_str("\tm.AdditionalProperties = make(map[string]");
    output.push_str(element_type);
    output.push_str(", len(raw))\n");
    output.push_str("\tfor k, v := range raw {\n");
    // A `null` member of a nullable map decodes to `nil` rather than being
    // dropped from the map, so the key survives the round trip ([[nullability]]).
    if shape.nullable && shape.element != GoMapElement::Raw {
        output.push_str(
            "\t\tif isNull(v) {\n\t\t\tm.AdditionalProperties[k] = nil\n\t\t\tcontinue\n\t\t}\n",
        );
    }
    let store = |output: &mut String, expr: &str| {
        output.push_str(if shape.nullable { "&" } else { "" });
        output.push_str(expr);
    };
    match shape.element {
        // Untyped members are preserved byte-for-byte, `null` included (P13).
        GoMapElement::Raw => {
            output.push_str("\t\tm.AdditionalProperties[k] = v\n");
        }
        GoMapElement::String
        | GoMapElement::Integer
        | GoMapElement::Number
        | GoMapElement::Boolean => {
            let helper = match shape.element {
                GoMapElement::String => "parseStringField",
                GoMapElement::Integer => "parseIntegerField",
                GoMapElement::Number => "parseNumberField",
                _ => "parseBoolField",
            };
            output.push_str("\t\tif value, ok := ");
            output.push_str(helper);
            output.push_str("(&v, k, true, false, &errs); ok {\n");
            if let Some(value) = &shape.value_schema {
                render_go_member_checks(
                    output,
                    "\t\t\t",
                    "value",
                    "k",
                    type_name,
                    MAP_MEMBER_POSITION,
                    value,
                    false,
                );
            }
            output.push_str("\t\t\tm.AdditionalProperties[k] = ");
            store(output, "value");
            output.push_str("\n\t\t}\n");
        }
        GoMapElement::Model | GoMapElement::Other => {
            if !shape.nullable {
                output.push_str("\t\tif isNull(v) {\n\t\t\terrs = append(errs, Violation{k, \"explicit null not allowed\"})\n\t\t\tcontinue\n\t\t}\n");
            }
            let decoded_type = element_type.trim_start_matches('*');
            let value_schema = shape
                .value_schema
                .as_ref()
                .map(|value| nullable_non_null_schema(value).unwrap_or(value));
            // A union member is a sealed interface `encoding/json` cannot
            // allocate; it decodes through the union's dispatcher instead.
            if unions.contains_key(decoded_type) {
                output.push_str("\t\tif value, ok := unmarshal");
                output.push_str(decoded_type);
                output.push_str(
                    "(v, k, &errs); ok {\n\t\t\tm.AdditionalProperties[k] = value\n\t\t}\n",
                );
            } else if let Some(value) = value_schema
                && value.ty.as_ref().and_then(Value::as_str) == Some("array")
            {
                output.push_str("\t\tvar value ");
                output.push_str(decoded_type);
                output.push('\n');
                render_go_array_position_unmarshal(
                    output,
                    "\t\t",
                    "v",
                    "k",
                    "value",
                    decoded_type,
                    value,
                    type_name,
                    MAP_MEMBER_POSITION,
                    model_names,
                    unions,
                    0,
                    &GoErrsBinding::BY_VALUE,
                );
                output.push_str("\t\tm.AdditionalProperties[k] = ");
                store(output, "value");
                output.push('\n');
            } else if let Some(value) = value_schema
                && let Some(kind) = temporal_kind(value)
            {
                output.push_str("\t\tif s, ok := parseStringField(&v, k, true, false, &errs); ok {\n\t\t\tif value, ok := ");
                output.push_str(go_temporal_parse_fn(kind));
                output.push_str("(k, s, &errs); ok {\n\t\t\t\tm.AdditionalProperties[k] = ");
                store(output, "value");
                output.push_str("\n\t\t\t}\n\t\t}\n");
            } else if let Some(value) = value_schema
                && let Some(encoding) = content_encoding_kind(value)
            {
                output.push_str(
                    "\t\tif s, ok := parseStringField(&v, k, true, false, &errs); ok {\n",
                );
                if value.min_length.is_some() || value.max_length.is_some() {
                    render_go_string_checks(output, "s", "k", value, "\t\t\t");
                }
                if let Some(pattern) = &value.pattern {
                    render_go_pattern_check(
                        output,
                        "s",
                        "k",
                        &go_pattern_var_name(type_name, MAP_MEMBER_POSITION),
                        pattern,
                        "\t\t\t",
                    );
                }
                output.push_str("\t\t\tif value, ok := ");
                output.push_str(go_content_encoding_decode_fn(encoding));
                output.push_str("(k, s, ");
                output.push_str(&go_content_encoding_var_name(
                    type_name,
                    MAP_MEMBER_POSITION,
                ));
                output.push_str(", &errs); ok {\n\t\t\t\tm.AdditionalProperties[k] = ");
                store(output, "value");
                output.push_str("\n\t\t\t}\n\t\t}\n");
            } else {
                output.push_str("\t\tvar value ");
                output.push_str(decoded_type);
                output.push_str("\n\t\tif err := json.Unmarshal(v, &value); err != nil {\n\t\t\tmergeNested(&errs, k, err)\n\t\t\tcontinue\n\t\t}\n");
                if let Some(value) = &shape.value_schema {
                    render_go_member_checks(
                        output,
                        "\t\t",
                        "value",
                        "k",
                        type_name,
                        MAP_MEMBER_POSITION,
                        value,
                        false,
                    );
                }
                output.push_str("\t\tm.AdditionalProperties[k] = ");
                store(output, "value");
                output.push('\n');
            }
        }
    }
    output.push_str("\t}\n");
    // Object member-count and key-shape constraints over the wire member set.
    render_go_property_count_checks(output, "len(raw)", schema, "\t");
    if let Some(subschema) = &schema.property_names {
        render_go_property_name_checks(output, "raw", subschema, "\t");
    }
    output.push_str("\tif len(errs) > 0 {\n\t\treturn &ValidationError{Violations: errs}\n\t}\n\treturn nil\n}\n\n");

    output.push_str("// MarshalJSON validates m, then serializes it to JSON, returning a\n");
    output.push_str("// *ValidationError if validation fails.\n");
    output.push_str("func (m ");
    output.push_str(type_name);
    output.push_str(") MarshalJSON() ([]byte, error) {\n\tif err := m.Validate(); err != nil {\n\t\treturn nil, err\n\t}\n");
    let needs_conversion = shape
        .value_schema
        .as_ref()
        .is_some_and(schema_requires_go_wire_conversion);
    if !needs_conversion {
        output.push_str("\tout := make(map[string]");
        output.push_str(element_type);
        output.push_str(", len(m.AdditionalProperties))\n\tfor k, v := range m.AdditionalProperties {\n\t\tout[k] = v\n\t}\n\treturn json.Marshal(out)\n}\n\n");
        return;
    }
    output.push_str("\tout := make(map[string]any, len(m.AdditionalProperties))\n\tfor k, v := range m.AdditionalProperties {\n");
    if shape.nullable {
        output.push_str("\t\tif v == nil {\n\t\t\tout[k] = nil\n\t\t\tcontinue\n\t\t}\n");
    }
    let subject = if shape.nullable { "(*v)" } else { "v" };
    let value_schema = shape
        .value_schema
        .as_ref()
        .map(|value| nullable_non_null_schema(value).unwrap_or(value));
    if let Some(value) = value_schema
        && value.ty.as_ref().and_then(Value::as_str) == Some("array")
    {
        let wire =
            render_go_array_wire_value(output, "\t\t", subject, value, MAP_MEMBER_POSITION, 0);
        output.push_str("\t\tout[k] = ");
        output.push_str(&wire);
        output.push('\n');
    } else if let Some(value) = value_schema
        && let Some(kind) = temporal_kind(value)
    {
        output.push_str("\t\tout[k] = ");
        output.push_str(go_temporal_format_fn(kind));
        output.push('(');
        output.push_str(subject);
        output.push_str(")\n");
    } else if let Some(value) = value_schema
        && let Some(encoding) = content_encoding_kind(value)
    {
        output.push_str("\t\tout[k] = ");
        output.push_str(go_content_encoding_encode_fn(encoding));
        output.push('(');
        output.push_str(subject);
        output.push_str(")\n");
    } else {
        output.push_str("\t\tout[k] = v\n");
    }
    output.push_str("\t}\n\treturn json.Marshal(out)\n}\n\n");
}

/// Emits the predicates a value schema declares, over `value_expr` (the decoded
/// value, in scope) and keyed by `key_expr` (the violation path). These are the
/// same predicates — and the same reasons — the property position runs for a
/// value of that type, per [[additionalProperties]] §"Validator mapping"
/// (per-member `T` validation) and [[oneOf]] §"Validator mapping" (a branch's own
/// constraints). Two positions share it: a typed map's member (keyed by the
/// member's own key) and a union's synthesized `<Union><Kind>` variant.
///
/// `type_name`/`position` name the package-level compiled-regex vars a `pattern`
/// or `format` references — the map's model + `value`, or the variant type itself.
///
/// `serialize_side` selects the checks the parse adapter has already made:
/// the integer cap is enforced by `parseIntegerField` on the way in, so it is
/// re-checked only before emit (P12), where an in-memory `int64` can hold a
/// magnitude the cap forbids.
///
/// A closed value set (`const`/`enum`) is checked against the wire literals
/// rather than through a synthesized defined type: neither a map member nor a
/// union variant has a field to hang Go's value constants off, so the closedness
/// lives in the validator alone. The accepted value set is identical to every
/// other target's.
fn render_go_member_checks(
    output: &mut String,
    indent: &str,
    value_expr: &str,
    key_expr: &str,
    type_name: &str,
    position: &str,
    value: &Schema,
    serialize_side: bool,
) {
    let ty = value.ty.as_ref().and_then(Value::as_str);
    if is_closed_value_schema(value) {
        let values = closed_values(value);
        let literals = values
            .iter()
            .map(|entry| go_scalar_literal(entry, ty))
            .collect::<Vec<_>>();
        let reason = go_closed_reason(&values, value_expr);
        output.push_str(indent);
        if literals.len() == 1 {
            output.push_str("if ");
            output.push_str(value_expr);
            output.push_str(" != ");
            output.push_str(&literals[0]);
            output.push_str(" {\n");
        } else {
            output.push_str("switch ");
            output.push_str(value_expr);
            output.push_str(" {\n");
            output.push_str(indent);
            output.push_str("case ");
            output.push_str(&literals.join(", "));
            output.push_str(":\n");
            output.push_str(indent);
            output.push_str("default:\n");
        }
        output.push_str(indent);
        output.push_str("\terrs = append(errs, Violation{");
        output.push_str(key_expr);
        output.push_str(", ");
        output.push_str(&reason);
        output.push_str("})\n");
        output.push_str(indent);
        output.push_str("}\n");
        return;
    }
    if serialize_side && ty == Some("integer") {
        output.push_str(indent);
        output.push_str("if ");
        output.push_str(value_expr);
        output.push_str(" < -integerCap || ");
        output.push_str(value_expr);
        output.push_str(" > integerCap {\n");
        output.push_str(indent);
        output.push_str("\terrs = append(errs, Violation{");
        output.push_str(key_expr);
        output.push_str(", \"exceeds ±(2^53-1) integer cap\"})\n");
        output.push_str(indent);
        output.push_str("}\n");
    }
    if ty == Some("number") {
        output.push_str(indent);
        output.push_str("if math.IsNaN(");
        output.push_str(value_expr);
        output.push_str(") || math.IsInf(");
        output.push_str(value_expr);
        output.push_str(", 0) {\n");
        output.push_str(indent);
        output.push_str("\terrs = append(errs, Violation{");
        output.push_str(key_expr);
        output.push_str(", fmt.Sprintf(\"must be a finite number, got %v\", ");
        output.push_str(value_expr);
        output.push_str(")})\n");
        output.push_str(indent);
        output.push_str("}\n");
    }
    if value.has_numeric_constraints() && matches!(ty, Some("integer" | "number")) {
        render_go_numeric_checks(output, value_expr, key_expr, value, indent);
    }
    if ty == Some("string") {
        if value.has_string_constraints() {
            render_go_string_checks(output, value_expr, key_expr, value, indent);
        }
        if let Some(pattern) = &value.pattern {
            render_go_pattern_check(
                output,
                value_expr,
                key_expr,
                &go_pattern_var_name(type_name, position),
                pattern,
                indent,
            );
        }
        if let Some(format) = &value.format {
            render_go_format_check(
                output,
                value_expr,
                key_expr,
                &go_format_var_name(type_name, position),
                format,
                indent,
            );
        }
    }
    if value.has_array_constraints() && ty == Some("array") {
        render_go_array_checks(output, value_expr, key_expr, value, indent);
    }
}

/// The position name a typed map's member contributes to a synthesized
/// identifier — the same `Value` suffix the loader uses when it names an inline
/// member shape (`<Enclosing>Value`).
const MAP_MEMBER_POSITION: &str = "value";

/// True when the model has any materialized temporal `format` property.
fn schema_uses_temporal(schema: &Schema) -> bool {
    temporal_kind(schema).is_some()
        || schema
            .properties
            .as_ref()
            .is_some_and(|properties| properties.values().any(schema_uses_temporal))
        || schema.items.as_deref().is_some_and(schema_uses_temporal)
        || schema
            .one_of
            .as_ref()
            .is_some_and(|branches| branches.iter().any(schema_uses_temporal))
        || typed_map_value_schema(schema)
            .ok()
            .flatten()
            .as_ref()
            .is_some_and(schema_uses_temporal)
}

fn model_uses_temporal(model: &PlannedJsonType) -> bool {
    let Ok(schema) = decode_schema(model) else {
        return false;
    };
    schema_uses_temporal(&schema)
}

/// Emits the package-level materialized-temporal runtime: the pinned narrowed
/// regexes, the Gregorian calendar predicate, and the parse/serialize adapters
/// for `date-time` / `date` / `time` / `duration`. The serializer is
/// generator-owned (RFC 3339, original offset preserved, `+00:00`/`-00:00` → `Z`,
/// trailing fractional zeros trimmed; duration canonicalized to time-only). See
/// `specs/json-schema/features/format.md`.
fn render_go_temporal_helpers(output: &mut String) {
    use crate::json_schema::format::TemporalKind;
    output.push_str(&format!(
        "var jsonTemporalDateTimeRE = regexp.MustCompile(`{}`)\n",
        TemporalKind::DateTime.pattern()
    ));
    output.push_str(&format!(
        "var jsonTemporalDateRE = regexp.MustCompile(`{}`)\n",
        TemporalKind::Date.pattern()
    ));
    output.push_str(&format!(
        "var jsonTemporalTimeRE = regexp.MustCompile(`{}`)\n",
        TemporalKind::Time.pattern()
    ));
    output.push_str(&format!(
        "var jsonTemporalDurationRE = regexp.MustCompile(`{}`)\n\n",
        TemporalKind::Duration.pattern()
    ));
    output.push_str(TEMPORAL_HELPER_BODY);
}

/// The schema-independent body of the materialized-temporal runtime.
const TEMPORAL_HELPER_BODY: &str = r####"// daysInTemporalMonth returns the Gregorian day count of a 1-based month.
func daysInTemporalMonth(year, month int) int {
	switch month {
	case 1, 3, 5, 7, 8, 10, 12:
		return 31
	case 4, 6, 9, 11:
		return 30
	case 2:
		if (year%4 == 0 && year%100 != 0) || year%400 == 0 {
			return 29
		}
		return 28
	}
	return 0
}

// validTemporalCalendar checks day-in-month over a YYYY-MM-DD prefix; the regex
// has already fixed the digit shape and the month/day ranges.
func validTemporalCalendar(s string) bool {
	if len(s) < 10 {
		return false
	}
	year, err1 := strconv.Atoi(s[0:4])
	month, err2 := strconv.Atoi(s[5:7])
	day, err3 := strconv.Atoi(s[8:10])
	if err1 != nil || err2 != nil || err3 != nil {
		return false
	}
	if year < 1 {
		return false
	}
	max := daysInTemporalMonth(year, month)
	return max > 0 && day >= 1 && day <= max
}

func temporalFracNanos(nanos int) string {
	if nanos == 0 {
		return ""
	}
	return "." + strings.TrimRight(fmt.Sprintf("%09d", nanos), "0")
}

func temporalOffset(secs int) string {
	if secs == 0 {
		return "Z"
	}
	sign := "+"
	if secs < 0 {
		sign = "-"
		secs = -secs
	}
	return fmt.Sprintf("%s%02d:%02d", sign, secs/3600, (secs%3600)/60)
}

// parseDateTime validates the wire string (narrowed regex + calendar) then parses
// it into a time.Time, uppercasing the case-insensitive T/Z first (Go's parser
// rejects lowercase). Offset and nanoseconds are preserved; no truncation.
func parseDateTime(path, s string, errs *[]Violation) (time.Time, bool) {
	if !jsonTemporalDateTimeRE.MatchString(s) || !validTemporalCalendar(s) {
		*errs = append(*errs, Violation{path, fmt.Sprintf("must be a valid date-time, got %q", s)})
		return time.Time{}, false
	}
	t, err := time.Parse(time.RFC3339Nano, strings.ToUpper(s))
	if err != nil {
		*errs = append(*errs, Violation{path, fmt.Sprintf("must be a valid date-time, got %q", s)})
		return time.Time{}, false
	}
	return t, true
}

// formatDateTime re-serializes via time.RFC3339Nano, which matches the
// generator-owned form exactly (offset preserved, Z for zero offset, trailing
// fractional zeros trimmed).
func formatDateTime(t time.Time) string {
	return t.Format(time.RFC3339Nano)
}

func parseDate(path, s string, errs *[]Violation) (time.Time, bool) {
	if !jsonTemporalDateRE.MatchString(s) || !validTemporalCalendar(s) {
		*errs = append(*errs, Violation{path, fmt.Sprintf("must be a valid date, got %q", s)})
		return time.Time{}, false
	}
	t, _ := time.Parse("2006-01-02", s)
	return t, true
}

func formatDate(t time.Time) string {
	return t.Format("2006-01-02")
}

// Go has no time-of-day type; time.Time carries a phantom date. An offset-less
// value is stored with a sentinel year so the serializer can distinguish it from
// an explicit Z/offset (whose offset parse yields year 0).
const temporalNoOffsetYear = 1

func parseTime(path, s string, errs *[]Violation) (time.Time, bool) {
	if !jsonTemporalTimeRE.MatchString(s) {
		*errs = append(*errs, Violation{path, fmt.Sprintf("must be a valid time, got %q", s)})
		return time.Time{}, false
	}
	w := strings.ToUpper(s)
	if t, err := time.Parse("15:04:05.999999999Z07:00", w); err == nil {
		return t, true
	}
	t, err := time.Parse("15:04:05.999999999", w)
	if err != nil {
		*errs = append(*errs, Violation{path, fmt.Sprintf("must be a valid time, got %q", s)})
		return time.Time{}, false
	}
	return time.Date(temporalNoOffsetYear, 1, 1, t.Hour(), t.Minute(), t.Second(), t.Nanosecond(), time.UTC), true
}

func formatTime(t time.Time) string {
	s := fmt.Sprintf("%02d:%02d:%02d%s", t.Hour(), t.Minute(), t.Second(), temporalFracNanos(t.Nanosecond()))
	if t.Year() != temporalNoOffsetYear {
		_, off := t.Zone()
		s += temporalOffset(off)
	}
	return s
}

// parseDuration validates the time-only wire string then sums it into a
// time.Duration, rejecting an overflow of the int64-nanosecond capacity.
func parseDuration(path, s string, errs *[]Violation) (time.Duration, bool) {
	fail := func() (time.Duration, bool) {
		*errs = append(*errs, Violation{path, fmt.Sprintf("must be a valid duration, got %q", s)})
		return 0, false
	}
	if !jsonTemporalDurationRE.MatchString(s) {
		return fail()
	}
	const maxSeconds = int64(9223372036854775807) / 1000000000
	var totalSeconds int64
	num := ""
	for _, ch := range s[2:] {
		if ch >= '0' && ch <= '9' {
			num += string(ch)
			continue
		}
		mag, err := strconv.ParseInt(num, 10, 64)
		if err != nil {
			return fail()
		}
		num = ""
		var unit int64
		switch ch {
		case 'H':
			unit = 3600
		case 'M':
			unit = 60
		case 'S':
			unit = 1
		}
		prod := mag * unit
		if unit != 0 && prod/unit != mag {
			return fail()
		}
		totalSeconds += prod
		if totalSeconds < 0 || totalSeconds > maxSeconds {
			return fail()
		}
	}
	return time.Duration(totalSeconds) * time.Second, true
}

// formatDuration canonicalizes to time-only PT…H…M…S (PT0S for zero); a
// non-canonical input collapses (PT90M → PT1H30M).
func formatDuration(d time.Duration) string {
	total := int64(d / time.Second)
	if total == 0 {
		return "PT0S"
	}
	out := "PT"
	if h := total / 3600; h != 0 {
		out += fmt.Sprintf("%dH", h)
	}
	if m := (total % 3600) / 60; m != 0 {
		out += fmt.Sprintf("%dM", m)
	}
	if sec := total % 60; sec != 0 {
		out += fmt.Sprintf("%dS", sec)
	}
	return out
}

"####;

/// True when the model has any materialized `contentEncoding` property.
fn schema_uses_content_encoding(schema: &Schema) -> bool {
    content_encoding_kind(schema).is_some()
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
        || typed_map_value_schema(schema)
            .ok()
            .flatten()
            .as_ref()
            .is_some_and(schema_uses_content_encoding)
}

fn model_uses_content_encoding(model: &PlannedJsonType) -> bool {
    let Ok(schema) = decode_schema(model) else {
        return false;
    };
    schema_uses_content_encoding(&schema)
}

/// Emits the package-level `contentEncoding` codec: the stdlib base64 /
/// base64url decode (regex-gated) and canonical encode adapters. The regex is
/// passed in (compiled once per field at package init); the decoder runs only
/// after it passes, so no language's lenient decoder can diverge (P1). See
/// `specs/json-schema/features/contentEncoding.md`.
fn render_go_content_encoding_helpers(output: &mut String) {
    output.push_str(CONTENT_ENCODING_HELPER_BODY);
}

const CONTENT_ENCODING_HELPER_BODY: &str = r####"// decodeBase64 validates the wire string against the pinned canonical base64
// regex, then decodes it via the standard (padded) alphabet into bytes.
func decodeBase64(path, s string, re *regexp.Regexp, errs *[]Violation) ([]byte, bool) {
	if !re.MatchString(s) {
		*errs = append(*errs, Violation{path, fmt.Sprintf("must be base64-encoded, got %q", s)})
		return nil, false
	}
	b, err := base64.StdEncoding.DecodeString(s)
	if err != nil {
		*errs = append(*errs, Violation{path, fmt.Sprintf("must be base64-encoded, got %q", s)})
		return nil, false
	}
	return b, true
}

// encodeBase64 re-encodes bytes to canonical padded standard base64.
func encodeBase64(b []byte) string {
	return base64.StdEncoding.EncodeToString(b)
}

// decodeBase64URL validates the wire string against the pinned canonical
// (unpadded) base64url regex, then decodes it via the URL-safe alphabet.
func decodeBase64URL(path, s string, re *regexp.Regexp, errs *[]Violation) ([]byte, bool) {
	if !re.MatchString(s) {
		*errs = append(*errs, Violation{path, fmt.Sprintf("must be base64url-encoded, got %q", s)})
		return nil, false
	}
	b, err := base64.RawURLEncoding.DecodeString(s)
	if err != nil {
		*errs = append(*errs, Violation{path, fmt.Sprintf("must be base64url-encoded, got %q", s)})
		return nil, false
	}
	return b, true
}

// encodeBase64URL re-encodes bytes to canonical unpadded URL-safe base64.
func encodeBase64URL(b []byte) string {
	return base64.RawURLEncoding.EncodeToString(b)
}

"####;

fn decode_schema(model: &PlannedJsonType) -> Result<Schema> {
    serde_json::from_value(model.schema.clone()).map_err(|error| Error::InvalidJsonSchema {
        path: PathBuf::from("<go-json-generator>"),
        reason: format!(
            "failed to read planned JSON schema `{}`: {error}",
            model.full_name
        ),
    })
}

fn typed_map_value_schema(schema: &Schema) -> Result<Option<Schema>> {
    if schema
        .properties
        .as_ref()
        .is_some_and(|properties| !properties.is_empty())
    {
        return Ok(None);
    }
    match &schema.additional_properties {
        Some(Value::Object(value)) => serde_json::from_value(Value::Object(value.clone()))
            .map(Some)
            .map_err(|error| Error::InvalidJsonSchema {
                path: PathBuf::from("<go-json-generator>"),
                reason: format!("failed to read `additionalProperties`: {error}"),
            }),
        _ => Ok(None),
    }
}

/// How a map-shaped model's members decode: the wire kind of its
/// `additionalProperties` element type, which selects the parse helper and the
/// `Validate` recursion.
#[derive(Debug, Clone, PartialEq)]
enum GoMapElement {
    /// Untyped members (`additionalProperties: true`), kept verbatim as
    /// `json.RawMessage` so numbers and precision survive a round-trip (P13).
    Raw,
    String,
    Integer,
    Number,
    Boolean,
    /// A `$ref` to a named model: its own `UnmarshalJSON`/`Validate` carry the
    /// member's constraints.
    Model,
    /// Any other typed element (an array, a nested map): decoded by
    /// `encoding/json` into the element type.
    Other,
}

/// A map-shaped object model — no declared `properties`, members governed by
/// `additionalProperties` — emitted as a struct wrapping a single
/// `AdditionalProperties map[string]T` member (specs/json-schema/features/additionalProperties.md).
#[derive(Debug, Clone)]
struct GoMapShape {
    /// The member schema, with any nullability `oneOf` wrapper looked through;
    /// `None` for untyped members.
    value_schema: Option<Schema>,
    /// Whether an explicit `null` member is admitted (the nullability `oneOf`).
    nullable: bool,
    /// The Go element type `T` of `AdditionalProperties map[string]T` — `*T` when
    /// a `null` member is admitted, so it decodes to `nil` rather than landing on
    /// `T`'s zero value, the same rule [[items]] applies to an element.
    element_type: String,
    element: GoMapElement,
}

/// Classifies a schema as map-shaped, i.e. an object with no declared
/// `properties` whose members are open (`additionalProperties` either typed or
/// `true`). A closed empty object (`additionalProperties: false`) is not
/// map-shaped — it admits no members at all and stays an empty struct.
fn go_map_shape(
    schema: &Schema,
    model_names: &BTreeMap<String, String>,
    unions: &BTreeMap<String, GoUnion>,
) -> Result<Option<GoMapShape>> {
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
    if let Some(declared) = typed_map_value_schema(schema)? {
        // `go_element_type_annotation` adds the pointer a nullable member needs,
        // exactly as it does for an array element — except over a union, whose
        // sealed interface holds `nil` itself.
        let mut element_type = go_element_type_annotation(&declared, "", model_names)?;
        if unions.contains_key(element_type.trim_start_matches('*')) {
            element_type = element_type.trim_start_matches('*').to_string();
        }
        let nullable = allows_null(&declared);
        let value_schema = nullable_non_null_schema(&declared)
            .cloned()
            .unwrap_or(declared);
        let element = if value_schema.reference.is_some() {
            GoMapElement::Model
        } else {
            match value_schema.ty.as_ref().and_then(Value::as_str) {
                Some("string")
                    if temporal_kind(&value_schema).is_none()
                        && content_encoding_kind(&value_schema).is_none() =>
                {
                    GoMapElement::String
                }
                Some("integer") => GoMapElement::Integer,
                Some("number") => GoMapElement::Number,
                Some("boolean") => GoMapElement::Boolean,
                _ => GoMapElement::Other,
            }
        };
        return Ok(Some(GoMapShape {
            value_schema: Some(value_schema),
            nullable,
            element_type,
            element,
        }));
    }
    if schema.additional_properties.as_ref() == Some(&Value::Bool(false)) {
        return Ok(None);
    }
    Ok(Some(GoMapShape {
        value_schema: None,
        nullable: false,
        element_type: "json.RawMessage".to_string(),
        element: GoMapElement::Raw,
    }))
}

fn go_property_type(
    model_name: &str,
    json_name: &str,
    schema: &Schema,
    required: bool,
    model_names: &BTreeMap<String, String>,
) -> Result<String> {
    if is_closed_value_schema(schema) {
        let type_name = const_type_name(model_name, &schema.go_member_name(json_name));
        return Ok(if required {
            type_name
        } else {
            format!("*{type_name}")
        });
    }
    let mut annotation = go_type_annotation(schema, json_name, model_names)?;
    if !required && !annotation.starts_with("[]") {
        annotation = format!("*{annotation}");
    }
    if required && allows_null(schema) && !annotation.starts_with('*') {
        annotation = format!("*{annotation}");
    }
    Ok(annotation)
}

fn go_type_annotation(
    schema: &Schema,
    json_name: &str,
    model_names: &BTreeMap<String, String>,
) -> Result<String> {
    // A materialized temporal `format` replaces the `string` field type with a
    // native construct (looking through the `oneOf[…, null]` nullable wrapper).
    if let Some(kind) = temporal_kind(schema) {
        return Ok(go_temporal_type(kind).to_string());
    }
    // A materialized `contentEncoding` replaces the `string` field type with the
    // native `[]byte` construct (looking through the `oneOf[…, null]` wrapper).
    if content_encoding_kind(schema).is_some() {
        return Ok("[]byte".to_string());
    }
    if let Some(reference) = &schema.reference {
        return Ok(reference_model_name(reference, model_names));
    }
    if let Some(branches) = &schema.one_of
        && let Some(non_null) = branches
            .iter()
            .find(|branch| !schema_type_includes(branch, "null"))
    {
        return go_type_annotation(non_null, json_name, model_names);
    }
    match schema.ty.as_ref().and_then(Value::as_str) {
        Some("string") => Ok("string".to_string()),
        Some("integer") => Ok("int64".to_string()),
        Some("number") => Ok("float64".to_string()),
        Some("boolean") => Ok("bool".to_string()),
        Some("array") => {
            let item = schema
                .items
                .as_ref()
                .map(|item| go_element_type_annotation(item, json_name, model_names))
                .transpose()?
                .unwrap_or_else(|| "any".to_string());
            Ok(format!("[]{item}"))
        }
        Some("object") => Ok("map[string]json.RawMessage".to_string()),
        _ => Ok("any".to_string()),
    }
}

/// An array element's Go type. Element nullability is the element's own
/// concern ([[items]]): a `oneOf:[T, null]` element is a `*T`, so a wire `null`
/// decodes to `nil` instead of silently landing on `T`'s zero value. A slice
/// element is already nil-able, and a sealed interface (a nullable union) holds
/// `nil` itself.
fn go_element_type_annotation(
    schema: &Schema,
    json_name: &str,
    model_names: &BTreeMap<String, String>,
) -> Result<String> {
    let annotation = go_type_annotation(schema, json_name, model_names)?;
    if allows_null(schema)
        && schema.one_of.is_some()
        && !annotation.starts_with('*')
        && !annotation.starts_with("[]")
    {
        return Ok(format!("*{annotation}"));
    }
    Ok(annotation)
}

fn reference_model_name(reference: &str, model_names: &BTreeMap<String, String>) -> String {
    if let Some(model_name) = model_names.get(reference) {
        return model_name.clone();
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

fn required_fields(schema: &Schema) -> BTreeSet<String> {
    schema
        .required
        .iter()
        .flatten()
        .cloned()
        .collect::<BTreeSet<_>>()
}

/// The materialized `TemporalKind` of a property schema — looking through the
/// `oneOf[…, null]` nullable wrapper — or `None` when it is not a materialized
/// temporal string field. See `specs/json-schema/features/format.md` (Materialization).
fn temporal_kind(schema: &Schema) -> Option<crate::json_schema::format::TemporalKind> {
    if let Some(non_null) = nullable_non_null_schema(schema) {
        return temporal_kind(non_null);
    }
    if schema.ty.as_ref().and_then(Value::as_str) != Some("string") {
        return None;
    }
    schema
        .format
        .as_deref()
        .and_then(crate::json_schema::format::TemporalKind::from_name)
}

/// The materialized `contentEncoding` of a property schema — looking through the
/// `oneOf[…, null]` nullable wrapper — or `None` when it is not a materialized
/// bytes string field. See `specs/json-schema/features/contentEncoding.md`.
fn content_encoding_kind(
    schema: &Schema,
) -> Option<crate::json_schema::content_encoding::Encoding> {
    if let Some(non_null) = nullable_non_null_schema(schema) {
        return content_encoding_kind(non_null);
    }
    if schema.ty.as_ref().and_then(Value::as_str) != Some("string") {
        return None;
    }
    schema
        .content_encoding
        .as_deref()
        .and_then(crate::json_schema::content_encoding::Encoding::from_name)
}

/// The generator-owned decode / encode function names for a `contentEncoding`.
fn go_content_encoding_decode_fn(
    encoding: crate::json_schema::content_encoding::Encoding,
) -> &'static str {
    match encoding {
        crate::json_schema::content_encoding::Encoding::Base64 => "decodeBase64",
        crate::json_schema::content_encoding::Encoding::Base64Url => "decodeBase64URL",
    }
}

fn go_content_encoding_encode_fn(
    encoding: crate::json_schema::content_encoding::Encoding,
) -> &'static str {
    match encoding {
        crate::json_schema::content_encoding::Encoding::Base64 => "encodeBase64",
        crate::json_schema::content_encoding::Encoding::Base64Url => "encodeBase64URL",
    }
}

/// Decodes a materialized `contentEncoding` field: read the wire `string`
/// (presence / null via `parseStringField`), run the pinned regex + co-occurring
/// wire-string constraints, then decode into `[]byte` via the stdlib codec. A
/// `[]byte` is nil-able, so an optional field needs no pointer.
#[allow(clippy::too_many_arguments)]
fn render_content_encoding_property_unmarshal(
    output: &mut String,
    model_name: &str,
    json_name: &str,
    field: &str,
    encoding: crate::json_schema::content_encoding::Encoding,
    required: bool,
    nullable: bool,
    property: &Schema,
) {
    let decode_fn = go_content_encoding_decode_fn(encoding);
    let re_var = go_content_encoding_var_name(model_name, json_name);
    let path = go_string_literal(json_name);
    output.push_str("\tif s, ok := parseStringField(get(");
    output.push_str(&path);
    output.push_str("), ");
    output.push_str(&path);
    output.push_str(", ");
    output.push_str(if required { "true" } else { "false" });
    output.push_str(", ");
    output.push_str(if nullable { "true" } else { "false" });
    output.push_str(", &errs); ok {\n");
    // Co-occurring wire-string constraints re-run over the encoded wire string.
    if property.min_length.is_some() || property.max_length.is_some() {
        render_go_string_checks(output, "s", &go_string_literal(json_name), property, "\t\t");
    }
    if let Some(pattern) = &property.pattern {
        render_go_pattern_check(
            output,
            "s",
            &go_string_literal(json_name),
            &go_pattern_var_name(model_name, json_name),
            pattern,
            "\t\t",
        );
    }
    output.push_str("\t\tif v, ok := ");
    output.push_str(decode_fn);
    output.push('(');
    output.push_str(&path);
    output.push_str(", s, ");
    output.push_str(&re_var);
    output.push_str(", &errs); ok {\n\t\t\tm.");
    output.push_str(field);
    output.push_str(" = v\n\t\t}\n\t}\n");
}

/// Serializes a materialized `contentEncoding` field through the generator-owned
/// canonical encoder (bytes → canonical base64 / base64url). A nil `[]byte`
/// optional field is omitted.
fn render_content_encoding_property_marshal(
    output: &mut String,
    model_name: &str,
    json_name: &str,
    field: &str,
    encoding: crate::json_schema::content_encoding::Encoding,
    required: bool,
    property: &Schema,
) {
    let encode_fn = go_content_encoding_encode_fn(encoding);
    let path = go_string_literal(json_name);
    let emit_checks = property.min_length.is_some()
        || property.max_length.is_some()
        || property.pattern.is_some();
    let emit_encode = |output: &mut String, indent: &str, value_expr: &str| {
        if emit_checks {
            output.push_str(indent);
            output.push_str("wire := ");
            output.push_str(encode_fn);
            output.push('(');
            output.push_str(value_expr);
            output.push_str(")\n");
            if property.min_length.is_some() || property.max_length.is_some() {
                render_go_string_checks(
                    output,
                    "wire",
                    &go_string_literal(json_name),
                    property,
                    indent,
                );
            }
            if let Some(pattern) = &property.pattern {
                render_go_pattern_check(
                    output,
                    "wire",
                    &go_string_literal(json_name),
                    &go_pattern_var_name(model_name, json_name),
                    pattern,
                    indent,
                );
            }
            output.push_str(indent);
            output.push_str("marshalField(out, ");
            output.push_str(&path);
            output.push_str(", wire, &errs)\n");
        } else {
            output.push_str(indent);
            output.push_str("marshalField(out, ");
            output.push_str(&path);
            output.push_str(", ");
            output.push_str(encode_fn);
            output.push('(');
            output.push_str(value_expr);
            output.push_str("), &errs)\n");
        }
    };
    if required {
        // A required `[]byte` is emitted unconditionally (a nil slice encodes to
        // the empty string, the canonical zero-byte wire).
        if emit_checks {
            output.push_str("\t{\n");
            emit_encode(output, "\t\t", field);
            output.push_str("\t}\n");
        } else {
            emit_encode(output, "\t", field);
        }
        return;
    }
    // Optional: a nil slice is absent (omitted).
    output.push_str("\tif ");
    output.push_str(field);
    output.push_str(" != nil {\n");
    emit_encode(output, "\t\t", field);
    output.push_str("\t}\n");
}

/// The Go native type a temporal `format` materializes into.
fn go_temporal_type(kind: crate::json_schema::format::TemporalKind) -> &'static str {
    match kind {
        crate::json_schema::format::TemporalKind::Duration => "time.Duration",
        _ => "time.Time",
    }
}

/// The generator-owned serializer function name for a temporal kind.
fn go_temporal_format_fn(kind: crate::json_schema::format::TemporalKind) -> &'static str {
    match kind {
        crate::json_schema::format::TemporalKind::DateTime => "formatDateTime",
        crate::json_schema::format::TemporalKind::Date => "formatDate",
        crate::json_schema::format::TemporalKind::Time => "formatTime",
        crate::json_schema::format::TemporalKind::Duration => "formatDuration",
    }
}

/// The parse-adapter function name for a temporal kind.
fn go_temporal_parse_fn(kind: crate::json_schema::format::TemporalKind) -> &'static str {
    match kind {
        crate::json_schema::format::TemporalKind::DateTime => "parseDateTime",
        crate::json_schema::format::TemporalKind::Date => "parseDate",
        crate::json_schema::format::TemporalKind::Time => "parseTime",
        crate::json_schema::format::TemporalKind::Duration => "parseDuration",
    }
}

fn allows_null(schema: &Schema) -> bool {
    schema.const_value.as_ref() == Some(&Value::Null)
        || schema_type_includes(schema, "null")
        || schema
            .one_of
            .as_ref()
            .is_some_and(|branches| branches.iter().any(allows_null))
}

fn nullable_non_null_schema(schema: &Schema) -> Option<&Schema> {
    schema.one_of.as_ref()?.iter().find(|branch| {
        !schema_type_includes(branch, "null") && branch.const_value.as_ref() != Some(&Value::Null)
    })
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

fn is_open_object(schema: &Schema) -> bool {
    schema.ty.as_ref().and_then(Value::as_str) == Some("object")
        && schema
            .properties
            .as_ref()
            .is_some_and(|properties| !properties.is_empty())
        && schema.additional_properties.as_ref() != Some(&Value::Bool(false))
}

/// The Go closed-value defined type, `<Model><Member>`, built from the **emitted
/// member identifier** so an `x-go-name` override on the declaring property moves
/// it: a name synthesized *from the member* follows the member (P15). This matches
/// Java, whose nested value class is already named off the emitted member, and is
/// what makes the collision fix-it in [[const]] actually resolve a clash — while
/// the name derived from the JSON key, the override moved the field and left this
/// type behind.
fn const_type_name(model_name: &str, member_ident: &str) -> String {
    format!("{model_name}{member_ident}")
}

/// True when the schema is a scalar closed value set (`const` or `enum`) that
/// synthesizes a Go defined type + value constant(s).
fn is_closed_value_schema(schema: &Schema) -> bool {
    schema.const_value.is_some() || schema.enum_values.is_some()
}

/// The scalar values of a `const`/`enum` schema (one for `const`, many for
/// `enum`).
fn closed_values(schema: &Schema) -> Vec<Value> {
    if let Some(value) = &schema.const_value {
        vec![value.clone()]
    } else if let Some(values) = &schema.enum_values {
        values.clone()
    } else {
        Vec::new()
    }
}

/// The Go underlying primitive for a closed-value defined type.
fn go_closed_underlying(schema: &Schema) -> &'static str {
    match schema.ty.as_ref().and_then(Value::as_str) {
        Some("integer") => "int64",
        Some("number") => "float64",
        Some("boolean") => "bool",
        _ => "string",
    }
}

/// Encodes a scalar value to its Go identifier suffix (Stage 1-4 → PascalCase):
/// strings word-split + camel-cased, digits kept, `.` → `_`, a leading sign →
/// `Neg`, booleans `True`/`False`.
fn go_value_suffix(value: &Value) -> String {
    match value {
        Value::String(text) => text.to_upper_camel_case(),
        Value::Bool(flag) => if *flag { "True" } else { "False" }.to_string(),
        Value::Number(number) => number.to_string().replace('-', "Neg").replace('.', "_"),
        _ => String::new(),
    }
}

/// The verbatim value-constant override for a `const`/`enum` value, if the
/// schema carries one: `x-go-const-name` replaces the single `const`'s constant,
/// and an `x-go-enum-names` entry (keyed by the wire value's string form)
/// replaces an enum member's constant. Mirrors `value_constant_override` in
/// `src/parser/json_schema.rs` so the P15 collision pass and emission agree —
/// keep the two lookups identical (const gates on `const`, else enum by string
/// key).
fn go_value_constant_override<'a>(schema: &'a Schema, value: &Value) -> Option<&'a str> {
    if schema.const_value.is_some() {
        schema.x_go_const_name.as_deref()
    } else if let (Some(map), Value::String(key)) = (&schema.x_go_enum_names, value) {
        map.get(key).map(String::as_str)
    } else {
        None
    }
}

fn go_closed_value_name(
    schema: &Schema,
    model_name: &str,
    field_name: &str,
    value: &Value,
) -> String {
    if let Some(name) = go_value_constant_override(schema, value) {
        return name.to_string();
    }
    format!(
        "{}{}",
        const_type_name(model_name, &schema.go_member_name(field_name)),
        go_value_suffix(value)
    )
}

/// The Go literal for a scalar value in the defined type's underlying kind.
fn go_closed_value_literal(value: &Value) -> String {
    match value {
        Value::String(text) => go_string_literal(text),
        Value::Bool(flag) => flag.to_string(),
        Value::Number(number) => number.to_string(),
        _ => "nil".to_string(),
    }
}

/// The `printf` verb for a value kind (quoted for strings, `%v` otherwise).
fn go_closed_verb(value: &Value) -> &'static str {
    match value {
        Value::String(_) => "%q",
        _ => "%v",
    }
}

/// The comma-joined display of a value set for a reason string.
fn go_closed_set_display(values: &[Value]) -> String {
    values
        .iter()
        .map(|value| match value {
            Value::String(text) => format!("{text:?}"),
            Value::Bool(flag) => flag.to_string(),
            Value::Number(number) => number.to_string(),
            _ => String::new(),
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// The Go expression producing the violation reason for an off-value: a static
/// `must equal <v>` for a single-value `const`, or an `fmt.Sprintf(... got ...)`
/// for a multi-value `enum`. `got_expr` is the Go expression holding the
/// offending value.
fn go_closed_reason(values: &[Value], got_expr: &str) -> String {
    if values.len() == 1 {
        let value = &values[0];
        let text = match value {
            Value::String(string) => format!("must equal {string:?}"),
            Value::Bool(flag) => format!("must equal {flag}"),
            Value::Number(number) => format!("must equal {number}"),
            _ => "must equal <const>".to_string(),
        };
        go_string_literal(&text)
    } else {
        let verb = values.first().map(go_closed_verb).unwrap_or("%v");
        let template = format!(
            "must be one of [{}], got {verb}",
            go_closed_set_display(values)
        );
        format!("fmt.Sprintf({}, {got_expr})", go_string_literal(&template))
    }
}

/// Emits the shared-`Validate` closed-value check (const equality / enum
/// membership) over the in-memory field. Optional fields are guarded on nil.
fn render_go_closed_validate(
    output: &mut String,
    model_name: &str,
    json_name: &str,
    property: &Schema,
    required: bool,
) {
    let field = format!("m.{}", property.go_member_name(json_name));
    let values = closed_values(property);
    let names = values
        .iter()
        .map(|value| go_closed_value_name(property, model_name, json_name, value))
        .collect::<Vec<_>>();
    let (subject, indent) = if required {
        (field.clone(), "\t".to_string())
    } else {
        output.push_str("\tif ");
        output.push_str(&field);
        output.push_str(" != nil {\n");
        (format!("(*{field})"), "\t\t".to_string())
    };
    let reason = go_closed_reason(&values, &subject);
    if values.len() == 1 {
        output.push_str(&indent);
        output.push_str("if ");
        output.push_str(&subject);
        output.push_str(" != ");
        output.push_str(&names[0]);
        output.push_str(" {\n");
        output.push_str(&indent);
        output.push_str("\terrs = append(errs, Violation{");
        output.push_str(&go_string_literal(json_name));
        output.push_str(", ");
        output.push_str(&reason);
        output.push_str("})\n");
        output.push_str(&indent);
        output.push_str("}\n");
    } else {
        output.push_str(&indent);
        output.push_str("switch ");
        output.push_str(&subject);
        output.push_str(" {\n");
        output.push_str(&indent);
        output.push_str("case ");
        output.push_str(&names.join(", "));
        output.push_str(":\n");
        output.push_str(&indent);
        output.push_str("default:\n");
        output.push_str(&indent);
        output.push_str("\terrs = append(errs, Violation{");
        output.push_str(&go_string_literal(json_name));
        output.push_str(", ");
        output.push_str(&reason);
        output.push_str("})\n");
        output.push_str(&indent);
        output.push_str("}\n");
    }
    if !required {
        output.push_str("\t}\n");
    }
}

/// Emits the closed-value (`const`/`enum`) unmarshal: parse the underlying
/// scalar, convert to the defined type, check equality/membership, and assign.
fn render_closed_value_unmarshal(
    output: &mut String,
    model_name: &str,
    json_name: &str,
    property: &Schema,
    required: bool,
) {
    let field = property.go_member_name(json_name);
    let type_name = const_type_name(model_name, &field);
    let underlying = go_closed_underlying(property);
    let parser = match underlying {
        "int64" => "parseIntegerField",
        "float64" => "parseNumberField",
        "bool" => "parseBoolField",
        _ => "parseStringField",
    };
    let values = closed_values(property);
    let names = values
        .iter()
        .map(|value| go_closed_value_name(property, model_name, json_name, value))
        .collect::<Vec<_>>();
    let assign = if required { "typed" } else { "&typed" };
    let reason = go_closed_reason(&values, "typed");
    output.push_str("\tif v, ok := ");
    output.push_str(parser);
    output.push_str("(get(");
    output.push_str(&go_string_literal(json_name));
    output.push_str("), ");
    output.push_str(&go_string_literal(json_name));
    output.push_str(", ");
    output.push_str(if required { "true" } else { "false" });
    output.push_str(", false, &errs); ok {\n");
    output.push_str("\t\ttyped := ");
    output.push_str(&type_name);
    output.push_str("(v)\n");
    if values.len() == 1 {
        output.push_str("\t\tif typed != ");
        output.push_str(&names[0]);
        output.push_str(" {\n\t\t\terrs = append(errs, Violation{");
        output.push_str(&go_string_literal(json_name));
        output.push_str(", ");
        output.push_str(&reason);
        output.push_str("})\n\t\t} else {\n\t\t\tm.");
        output.push_str(&field);
        output.push_str(" = ");
        output.push_str(assign);
        output.push_str("\n\t\t}\n");
    } else {
        output.push_str("\t\tswitch typed {\n\t\tcase ");
        output.push_str(&names.join(", "));
        output.push_str(":\n\t\t\tm.");
        output.push_str(&field);
        output.push_str(" = ");
        output.push_str(assign);
        output.push_str("\n\t\tdefault:\n\t\t\terrs = append(errs, Violation{");
        output.push_str(&go_string_literal(json_name));
        output.push_str(", ");
        output.push_str(&reason);
        output.push_str("})\n\t\t}\n");
    }
    output.push_str("\t}\n");
}

/// Renders `doc` (the envelope's `description`, per specs/json-schema/services.md)
/// as a Go doc comment, falling back to `fallback` (already name-led) when
/// `doc` is absent — every exported declaration must carry a doc comment
/// (specs/json-schema/PRINCIPLES.md, Go §1).
fn render_go_doc_comment(output: &mut String, indent: &str, doc: Option<&str>, fallback: &str) {
    let text = doc
        .map(str::trim)
        .filter(|doc| !doc.is_empty())
        .unwrap_or(fallback);
    for line in text.lines() {
        output.push_str(indent);
        output.push_str("// ");
        output.push_str(line.trim());
        output.push('\n');
    }
}

/// Renders the doc comment for a schema declaration: the `title` summary line
/// (name-led per the Go godoc convention — `// <Name> <title>`), the
/// `description` body, and — when `deprecated: true` — a `// Deprecated:`
/// paragraph (godoc convention; a generic reason, the rationale lives in the
/// body). See specs/json-schema/features/{title,description,deprecated}.md. `kind` is
/// "type" or "field", used only in the deprecation reason. When neither
/// `title` nor `description` is present, falls back to `fallback` (already
/// name-led) so the declaration still carries a comment — every exported
/// identifier must (specs/json-schema/PRINCIPLES.md, Go §1).
fn render_go_schema_doc(
    output: &mut String,
    indent: &str,
    name: &str,
    schema: &Schema,
    kind: &str,
    fallback: &str,
) {
    // Name-led opening line (golint): don't double the name if the text
    // already begins with it (case-insensitively).
    let name_led = |text: &str| {
        if text.to_lowercase().starts_with(&name.to_lowercase()) {
            text.to_string()
        } else {
            format!("{name} {text}")
        }
    };
    let title = schema
        .title
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty());
    let description = schema
        .description
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty());
    let mut lines: Vec<String> = Vec::new();
    match (title, description) {
        (Some(title), description) => {
            lines.push(name_led(title));
            for line in description.into_iter().flat_map(str::lines) {
                lines.push(line.trim().to_string());
            }
        }
        (None, Some(description)) => {
            // No title: the identifier leads the first line of the
            // description instead (specs/json-schema/features/description.md).
            let mut desc_lines = description.lines();
            if let Some(first) = desc_lines.next() {
                lines.push(name_led(first.trim()));
            }
            for line in desc_lines {
                lines.push(line.trim().to_string());
            }
        }
        (None, None) => {}
    }
    if lines.is_empty() {
        lines.push(fallback.to_string());
    }
    if schema.deprecated == Some(true) {
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.push(format!("Deprecated: This {kind} is deprecated."));
    }
    for line in lines {
        output.push_str(indent);
        if line.is_empty() {
            output.push_str("//\n");
        } else {
            output.push_str("// ");
            output.push_str(&line);
            output.push('\n');
        }
    }
}
