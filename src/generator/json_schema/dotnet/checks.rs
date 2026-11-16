//! Constraint predicates, emitted as C# statements that append `Violation`s.
//!
//! Every predicate here is shared by the parse direction (over the decoded
//! wire value) and the validate direction (over the in-memory model) so the
//! two cannot drift (P12). `path` arguments are C# expressions evaluating to the
//! violation path of the checked value.

use serde_json::Value;

use crate::json_schema::format::TemporalKind;

use super::cs_string;
use super::schema::{
    ArrayConstraints, NumericConstraints, Schema, StringConstraints, element_shape,
};
use super::types::CsType;

pub(super) const REGEX: &str = "global::System.Text.RegularExpressions.Regex";
pub(super) const REGEX_OPTIONS: &str =
    "global::System.Text.RegularExpressions.RegexOptions.CultureInvariant";

/// Renders a numeric literal for an `integer` (`long`) or `number` (`double`)
/// position. A fractional bound over an integer keeps its `double` spelling so
/// `>= 1.5` still means what it says.
pub(super) fn bound_literal(number: &serde_json::Number, is_integer: bool) -> String {
    if is_integer
        && let Some(value) = number.as_f64()
        && value.fract() == 0.0
    {
        return format!("{}L", value as i64);
    }
    let text = number.to_string();
    if text.contains(['.', 'e', 'E']) {
        text
    } else {
        format!("{text}.0")
    }
}

/// The human-readable bound in a reason string (`10`, not `10L`).
fn display_bound(literal: &str) -> &str {
    literal
        .strip_suffix('L')
        .or_else(|| literal.strip_suffix(".0"))
        .unwrap_or(literal)
}

fn push_violation(out: &mut String, indent: &str, condition: &str, path: &str, reason: &str) {
    let rt = super::runtime_prefix();
    out.push_str(&format!(
        "{indent}if ({condition})\n{indent}{{\n{indent}    violations.Add(new {rt}Violation({path}, {reason}));\n{indent}}}\n"
    ));
}

/// The numeric-constraint predicates over a decoded `long`/`double`.
pub(super) fn numeric_checks(
    out: &mut String,
    value: &str,
    path: &str,
    constraints: &NumericConstraints,
    is_integer: bool,
    indent: &str,
) {
    let mut emit = |bound: &serde_json::Number, op: &str, reason: &str| {
        let literal = bound_literal(bound, is_integer);
        push_violation(
            out,
            indent,
            &format!("{value} {op} {literal}"),
            path,
            &format!(
                "{} + {value}",
                cs_string(&format!("{reason} {}, got ", display_bound(&literal)))
            ),
        );
    };
    if let Some(bound) = &constraints.minimum {
        emit(bound, "<", "must be >=");
    }
    if let Some(bound) = &constraints.maximum {
        emit(bound, ">", "must be <=");
    }
    if let Some(bound) = &constraints.exclusive_minimum {
        emit(bound, "<=", "must be >");
    }
    if let Some(bound) = &constraints.exclusive_maximum {
        emit(bound, ">=", "must be <");
    }
    if let Some(divisor) = &constraints.multiple_of {
        let literal = bound_literal(divisor, is_integer);
        push_violation(
            out,
            indent,
            &format!("{value} % {literal} != 0"),
            path,
            &format!(
                "{} + {value}",
                cs_string(&format!(
                    "must be a multiple of {}, got ",
                    display_bound(&literal)
                ))
            ),
        );
    }
}

/// Where a string check finds its compiled `Regex`: a static field on the
/// enclosing converter (named after a stable position), or compiled through
/// the shared `Regex` cache at the use site.
#[derive(Clone, Copy)]
pub(super) enum RegexSite<'a> {
    Static(&'a str),
    Inline,
}

pub(super) fn pattern_field(position: &str) -> String {
    format!("{position}Pattern")
}

pub(super) fn format_field(position: &str) -> String {
    format!("{position}Format")
}

fn contains_pattern_field(position: &str) -> String {
    format!("{position}ContainsPattern")
}

fn contains_format_field(position: &str) -> String {
    format!("{position}ContainsFormat")
}

fn regex_match(site: RegexSite, field: &str, pattern: &str, value: &str) -> String {
    match site {
        RegexSite::Static(_) => format!("{field}.IsMatch({value})"),
        RegexSite::Inline => format!(
            "{REGEX}.IsMatch({value}, {}, {REGEX_OPTIONS})",
            cs_string(pattern)
        ),
    }
}

fn regex_static(out: &mut String, indent: &str, field: &str, pattern: &str) {
    out.push_str(&format!(
        "{indent}private static readonly {REGEX} {field} = new({}, {REGEX_OPTIONS});\n",
        cs_string(pattern)
    ));
}

/// Emits the compiled-once statics a position's string and `contains` checks
/// reference. Returns true when anything was written.
pub(super) fn regex_statics(
    out: &mut String,
    indent: &str,
    position: &str,
    string: &StringConstraints,
    array: &ArrayConstraints,
) -> bool {
    let mut wrote = false;
    if let Some(pattern) = &string.pattern {
        regex_static(out, indent, &pattern_field(position), pattern);
        wrote = true;
    }
    if let Some(format) = &string.format {
        regex_static(out, indent, &format_field(position), &format.pattern);
        wrote = true;
    }
    if let Some(matcher) = array.contains.as_deref() {
        let matcher_string = StringConstraints::from_schema(matcher);
        if let Some(pattern) = &matcher_string.pattern {
            regex_static(out, indent, &contains_pattern_field(position), pattern);
            wrote = true;
        }
        if let Some(format) = &matcher_string.format {
            regex_static(
                out,
                indent,
                &contains_format_field(position),
                &format.pattern,
            );
            wrote = true;
        }
    }
    wrote
}

/// The code-point length of a C# string (never `.Length`, which counts UTF-16
/// units).
pub(super) fn code_points(value: &str) -> String {
    let rt = super::runtime_prefix();
    format!("{rt}SpecJson.CodePoints({value})")
}

/// `minLength`/`maxLength`/`pattern`/`format` over a `string` value.
pub(super) fn string_checks(
    out: &mut String,
    value: &str,
    path: &str,
    site: RegexSite,
    constraints: &StringConstraints,
    indent: &str,
    depth: usize,
) {
    if constraints.has_length() {
        let length = format!("length{depth}");
        out.push_str(&format!("{indent}var {length} = {};\n", code_points(value)));
        if let Some(min) = constraints.min_length {
            push_violation(
                out,
                indent,
                &count_comparison(&length, "<", min),
                path,
                &format!(
                    "{} + {length}",
                    cs_string(&format!("must have length >= {min}, got "))
                ),
            );
        }
        if let Some(max) = constraints.max_length {
            push_violation(
                out,
                indent,
                &count_comparison(&length, ">", max),
                path,
                &format!(
                    "{} + {length}",
                    cs_string(&format!("must have length <= {max}, got "))
                ),
            );
        }
    }
    let position = match site {
        RegexSite::Static(position) => position,
        RegexSite::Inline => "",
    };
    if let Some(pattern) = &constraints.pattern {
        push_violation(
            out,
            indent,
            &format!(
                "!{}",
                regex_match(site, &pattern_field(position), pattern, value)
            ),
            path,
            &format!(
                "{} + {} + \", got \" + {value}",
                cs_string("must match pattern "),
                cs_string(pattern)
            ),
        );
    }
    if let Some(format) = &constraints.format {
        let mut condition = String::new();
        if let Some(max) = format.max_code_points {
            condition.push_str(&format!("{} > {max} || ", code_points(value)));
        }
        condition.push_str(&format!(
            "!{}",
            regex_match(site, &format_field(position), &format.pattern, value)
        ));
        push_violation(
            out,
            indent,
            &condition,
            path,
            &format!(
                "{} + {value}",
                cs_string(&format!("must be a valid {}, got ", format.name))
            ),
        );
    }
}

/// The C# literal for a scalar closed/matcher value in the operand's type.
pub(super) fn scalar_literal(value: &Value, ty: &CsType) -> String {
    match value {
        Value::String(text) => cs_string(text),
        Value::Bool(flag) => flag.to_string(),
        Value::Number(number) => bound_literal(number, matches!(ty, CsType::Long)),
        _ => "null".to_string(),
    }
}

fn equals_term(value: &Value, operand: &str, ty: &CsType) -> String {
    let literal = scalar_literal(value, ty);
    match ty {
        CsType::String => {
            format!("string.Equals({operand}, {literal}, global::System.StringComparison.Ordinal)")
        }
        _ => format!("{operand} == {literal}"),
    }
}

pub(super) fn closed_set_display(values: &[Value]) -> String {
    values
        .iter()
        .map(|value| match value {
            Value::String(text) => format!("{text:?}"),
            Value::Bool(flag) => flag.to_string(),
            Value::Number(number) => number.to_string(),
            _ => String::new(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The reason prefix for an off-set closed value.
pub(super) fn closed_reason_prefix(values: &[Value]) -> String {
    if values.len() == 1 {
        format!(
            "must equal {}, got ",
            serde_json::to_string(&values[0]).unwrap()
        )
    } else {
        format!("must be one of [{}], got ", closed_set_display(values))
    }
}

/// The `const`/`enum` assertion over a scalar operand (used where the value is
/// not materialized as a closed value class).
pub(super) fn closed_scalar_checks(
    out: &mut String,
    value: &str,
    path: &str,
    schema: &Schema,
    ty: &CsType,
    indent: &str,
) {
    let values = schema.closed_values();
    if values.is_empty() {
        return;
    }
    let condition = values
        .iter()
        .map(|candidate| format!("!({})", equals_term(candidate, value, ty)))
        .collect::<Vec<_>>()
        .join(" && ");
    push_violation(
        out,
        indent,
        &condition,
        path,
        &format!("{} + {value}", cs_string(&closed_reason_prefix(&values))),
    );
}

// ---------------------------------------------------------------------------
// Arrays
// ---------------------------------------------------------------------------

fn matcher_kind<'a>(matcher: &'a Schema, fallback: &CsType) -> &'a str {
    matcher.type_name().unwrap_or(match fallback {
        CsType::String => "string",
        CsType::Boolean => "boolean",
        CsType::Long => "integer",
        CsType::Double => "number",
        _ => "unsupported",
    })
}

/// The boolean match expression for a scalar `contains` matcher over `element`.
fn matcher_condition(
    matcher: &Schema,
    element: &str,
    ty: &CsType,
    position: Option<&str>,
) -> String {
    let is_integer = matches!(ty, CsType::Long);
    let mut parts = Vec::new();
    if let Some(value) = &matcher.const_value {
        parts.push(equals_term(value, element, ty));
    }
    if let Some(values) = &matcher.enum_values {
        let alternatives = values
            .iter()
            .map(|value| equals_term(value, element, ty))
            .collect::<Vec<_>>()
            .join(" || ");
        if !alternatives.is_empty() {
            parts.push(format!("({alternatives})"));
        }
    }
    for (bound, op) in [
        (&matcher.minimum, ">="),
        (&matcher.maximum, "<="),
        (&matcher.exclusive_minimum, ">"),
        (&matcher.exclusive_maximum, "<"),
    ] {
        if let Some(bound) = bound {
            parts.push(format!(
                "{element} {op} {}",
                bound_literal(bound, is_integer)
            ));
        }
    }
    if let Some(divisor) = &matcher.multiple_of {
        parts.push(format!(
            "{element} % {} == 0",
            bound_literal(divisor, is_integer)
        ));
    }
    if let Some(min) = matcher.min_length {
        parts.push(count_comparison(&code_points(element), ">=", min));
    }
    if let Some(max) = matcher.max_length {
        parts.push(count_comparison(&code_points(element), "<=", max));
    }
    let string = StringConstraints::from_schema(matcher);
    let site = match position {
        Some(position) => RegexSite::Static(position),
        None => RegexSite::Inline,
    };
    if let Some(pattern) = &string.pattern {
        parts.push(regex_match(
            site,
            &contains_pattern_field(position.unwrap_or_default()),
            pattern,
            element,
        ));
    }
    if let Some(format) = &string.format {
        if let Some(max) = format.max_code_points {
            parts.push(format!("{} <= {max}", code_points(element)));
        }
        parts.push(regex_match(
            site,
            &contains_format_field(position.unwrap_or_default()),
            &format.pattern,
            element,
        ));
    }
    if parts.is_empty() {
        "true".to_string()
    } else {
        parts.join(" && ")
    }
}

/// The type guard a typed `contains` matcher puts on an in-memory element.
fn typed_matcher_guard(matcher: &Schema, element: &str, ty: &CsType) -> String {
    let rt = super::runtime_prefix();
    match (matcher_kind(matcher, ty), ty) {
        ("string", CsType::String) | ("boolean", CsType::Boolean) => "true".to_string(),
        ("integer" | "number", CsType::Long) => "true".to_string(),
        ("number", CsType::Double) => format!("double.IsFinite({element})"),
        ("integer", CsType::Double) => format!(
            "double.IsFinite({element}) && {element} == global::System.Math.Round({element}) && global::System.Math.Abs({element}) <= {rt}SpecJson.IntegerCap"
        ),
        _ => "false".to_string(),
    }
}

/// True when an in-memory element needs a derived `uniqueItems` key.
fn needs_unique_key(ty: &CsType) -> bool {
    match ty {
        CsType::Double | CsType::Temporal(_) | CsType::Bytes(_) => true,
        CsType::List(inner, _) => needs_unique_key(inner),
        _ => false,
    }
}

/// The serialize-side `uniqueItems` key: a materialized value compares on its
/// canonical wire string (the operand the parse side compares), a `number`
/// folds `-0.0` onto `0.0`.
pub(super) fn unique_key(ty: &CsType, element: &str, depth: usize) -> String {
    let rt = super::runtime_prefix();
    let bound = format!("keyed{depth}");
    match ty {
        CsType::Double => format!("{rt}SpecJson.NumberKey({element})"),
        CsType::Temporal(kind) => format!(
            "({element} is {{ }} {bound} ? (object){rt}TemporalSupport.{}({bound}) : null)",
            temporal_format_fn(*kind)
        ),
        CsType::Bytes(encoding) => format!(
            "({element} is {{ }} {bound} ? (object){rt}Base64Support.{}({bound}) : null)",
            content_encoding_format_fn(*encoding)
        ),
        CsType::List(inner, _) if needs_unique_key(inner) => {
            let parameter = format!("uniqueKeyElement{depth}");
            format!(
                "({element} is {{ }} {bound} ? (object)global::System.Linq.Enumerable.ToList(global::System.Linq.Enumerable.Select({bound}, {parameter} => {})) : null)",
                unique_key(inner, &parameter, depth + 1)
            )
        }
        _ => element.to_string(),
    }
}

pub(super) fn temporal_parse_fn(kind: TemporalKind) -> &'static str {
    match kind {
        TemporalKind::DateTime => "ParseDateTime",
        TemporalKind::Date => "ParseDate",
        TemporalKind::Time => "ParseTime",
        TemporalKind::Duration => "ParseDuration",
    }
}

pub(super) fn temporal_format_fn(kind: TemporalKind) -> &'static str {
    match kind {
        TemporalKind::DateTime => "FormatDateTime",
        TemporalKind::Date => "FormatDate",
        TemporalKind::Time => "FormatTime",
        TemporalKind::Duration => "FormatDuration",
    }
}

pub(super) fn temporal_check_fn(kind: TemporalKind) -> &'static str {
    match kind {
        TemporalKind::DateTime => "CheckDateTime",
        TemporalKind::Date => "CheckDate",
        TemporalKind::Time => "CheckTime",
        TemporalKind::Duration => "CheckDuration",
    }
}

pub(super) fn content_encoding_parse_fn(
    encoding: crate::json_schema::content_encoding::Encoding,
) -> &'static str {
    match encoding {
        crate::json_schema::content_encoding::Encoding::Base64 => "ParseBase64",
        crate::json_schema::content_encoding::Encoding::Base64Url => "ParseBase64Url",
    }
}

pub(super) fn content_encoding_format_fn(
    encoding: crate::json_schema::content_encoding::Encoding,
) -> &'static str {
    match encoding {
        crate::json_schema::content_encoding::Encoding::Base64 => "FormatBase64",
        crate::json_schema::content_encoding::Encoding::Base64Url => "FormatBase64Url",
    }
}

fn count_checks(
    out: &mut String,
    count: &str,
    path: &str,
    constraints: &ArrayConstraints,
    indent: &str,
) {
    if let Some(min) = constraints.min_items {
        push_violation(
            out,
            indent,
            &count_comparison(count, "<", min),
            path,
            &format!(
                "{} + {count}",
                cs_string(&format!("must have at least {min} items, got "))
            ),
        );
    }
    if let Some(max) = constraints.max_items {
        push_violation(
            out,
            indent,
            &count_comparison(count, ">", max),
            path,
            &format!(
                "{} + {count}",
                cs_string(&format!("must have at most {max} items, got "))
            ),
        );
    }
}

fn contains_verdict(
    out: &mut String,
    match_count: &str,
    path: &str,
    constraints: &ArrayConstraints,
    indent: &str,
) {
    let effective_min = constraints.min_contains.unwrap_or(1);
    if effective_min > 0 {
        let reason = if constraints.min_contains.is_some() {
            format!(
                "{} + {match_count}",
                cs_string(&format!(
                    "too few matching items: at least {effective_min}, got "
                ))
            )
        } else {
            cs_string("no element matches the required schema")
        };
        push_violation(
            out,
            indent,
            &count_comparison(match_count, "<", effective_min),
            path,
            &reason,
        );
    }
    if let Some(max) = constraints.max_contains {
        push_violation(
            out,
            indent,
            &count_comparison(match_count, ">", max),
            path,
            &format!(
                "{} + {match_count}",
                cs_string(&format!("too many matching items: at most {max}, got "))
            ),
        );
    }
}

/// An `int` count compared against a portable count bound. A bound past
/// `int.MaxValue` widens the count to `long`: C# flags comparing an `int` with
/// an out-of-range constant as useless (CS0652, an error under warnings as
/// errors), although the verdict it reaches is the right one.
fn count_comparison(count: &str, operator: &str, bound: u64) -> String {
    if bound > i32::MAX as u64 {
        format!("(long){count} {operator} {bound}L")
    } else {
        format!("{count} {operator} {bound}")
    }
}

/// Array-level constraints over an in-memory list.
pub(super) fn array_checks(
    out: &mut String,
    list: &str,
    path: &str,
    element: &CsType,
    constraints: &ArrayConstraints,
    position: Option<&str>,
    indent: &str,
    depth: usize,
) {
    let rt = super::runtime_prefix();
    count_checks(out, &format!("{list}.Count"), path, constraints, indent);
    if constraints.unique_items {
        let seen = format!("seen{depth}");
        let index = format!("uniqueIndex{depth}");
        out.push_str(&format!(
            "{indent}var {seen} = new global::System.Collections.Generic.Dictionary<object, int>({rt}ValueEquality.Comparer);\n"
        ));
        out.push_str(&format!(
            "{indent}for (var {index} = 0; {index} < {list}.Count; {index}++)\n{indent}{{\n"
        ));
        out.push_str(&format!(
            "{indent}    var key = (object?){} ?? {rt}ValueEquality.NullKey;\n",
            unique_key(element, &format!("{list}[{index}]"), 0)
        ));
        out.push_str(&format!(
            "{indent}    if ({seen}.TryGetValue(key, out var priorIndex))\n{indent}    {{\n{indent}        violations.Add(new {rt}Violation({path}, \"duplicate items: element at index \" + {index} + \" equals index \" + priorIndex));\n{indent}    }}\n{indent}    else\n{indent}    {{\n{indent}        {seen}[key] = {index};\n{indent}    }}\n{indent}}}\n"
        ));
    }
    if let Some(matcher) = &constraints.contains {
        let (element_expr, matcher_ty) =
            if matches!(element, CsType::Temporal(_) | CsType::Bytes(_)) {
                (
                    format!("((string){}!)", unique_key(element, "candidate", 0)),
                    CsType::String,
                )
            } else {
                ("candidate".to_string(), element.clone())
            };
        let condition = matcher_condition(matcher, &element_expr, &matcher_ty, position);
        let guard = typed_matcher_guard(matcher, &element_expr, &matcher_ty);
        let match_count = format!("matchCount{depth}");
        out.push_str(&format!("{indent}var {match_count} = 0;\n"));
        out.push_str(&format!(
            "{indent}foreach (var item in {list})\n{indent}{{\n{indent}    if (item is {{ }} candidate && ({guard}) && ({condition}))\n{indent}    {{\n{indent}        {match_count}++;\n{indent}    }}\n{indent}}}\n"
        ));
        contains_verdict(out, &match_count, path, constraints, indent);
    }
}

/// Array-level constraints over the original wire array. Element conversion
/// may drop a bad element from the decoded list; these checks must not see
/// that shortened list.
pub(super) fn raw_array_checks(
    out: &mut String,
    node: &str,
    path: &str,
    element: &CsType,
    constraints: &ArrayConstraints,
    position: Option<&str>,
    indent: &str,
    depth: usize,
) {
    let rt = super::runtime_prefix();
    count_checks(
        out,
        &format!("{node}.GetArrayLength()"),
        path,
        constraints,
        indent,
    );
    if constraints.unique_items {
        let seen = format!("rawSeen{depth}");
        let index = format!("rawIndex{depth}");
        out.push_str(&format!(
            "{indent}var {seen} = new global::System.Collections.Generic.Dictionary<string, int>(global::System.StringComparer.Ordinal);\n{indent}var {index} = 0;\n"
        ));
        out.push_str(&format!(
            "{indent}foreach (var rawElement in {node}.EnumerateArray())\n{indent}{{\n{indent}    var rawKey = {rt}SpecJson.ValueKey(rawElement);\n{indent}    if ({seen}.TryGetValue(rawKey, out var priorIndex))\n{indent}    {{\n{indent}        violations.Add(new {rt}Violation({path}, \"duplicate items: element at index \" + {index} + \" equals index \" + priorIndex));\n{indent}    }}\n{indent}    else\n{indent}    {{\n{indent}        {seen}[rawKey] = {index};\n{indent}    }}\n{indent}    {index}++;\n{indent}}}\n"
        ));
    }
    if let Some(matcher) = &constraints.contains {
        let (guard, value, ty) = match matcher_kind(matcher, element) {
            "string" => (
                format!(
                    "rawElement.ValueKind == {}.String",
                    super::types::JSON_VALUE_KIND
                ),
                "rawElement.GetString()!".to_string(),
                CsType::String,
            ),
            "boolean" => (
                format!(
                    "rawElement.ValueKind is {kind}.True or {kind}.False",
                    kind = super::types::JSON_VALUE_KIND
                ),
                "rawElement.GetBoolean()".to_string(),
                CsType::Boolean,
            ),
            "integer" => (
                format!("{rt}SpecJson.IsSpecLong(rawElement)"),
                format!("{rt}SpecJson.DoubleOf(rawElement)"),
                CsType::Double,
            ),
            "number" => (
                format!("{rt}SpecJson.IsFiniteNumber(rawElement)"),
                format!("{rt}SpecJson.DoubleOf(rawElement)"),
                CsType::Double,
            ),
            _ => (
                "false".to_string(),
                "rawElement".to_string(),
                element.clone(),
            ),
        };
        let condition = matcher_condition(matcher, &value, &ty, position);
        let match_count = format!("rawMatchCount{depth}");
        out.push_str(&format!(
            "{indent}var {match_count} = 0;\n{indent}foreach (var rawElement in {node}.EnumerateArray())\n{indent}{{\n{indent}    if ({guard} && ({condition}))\n{indent}    {{\n{indent}        {match_count}++;\n{indent}    }}\n{indent}}}\n"
        ));
        contains_verdict(out, &match_count, path, constraints, indent);
    }
}

// ---------------------------------------------------------------------------
// Objects
// ---------------------------------------------------------------------------

/// `minProperties`/`maxProperties` over one member count.
pub(super) fn property_count_checks(
    out: &mut String,
    count: &str,
    path: &str,
    schema: &Schema,
    indent: &str,
) {
    if let Some(min) = schema.min_properties {
        push_violation(
            out,
            indent,
            &count_comparison(count, "<", min),
            path,
            &format!(
                "{} + {count}",
                cs_string(&format!("must have at least {min} properties, got "))
            ),
        );
    }
    if let Some(max) = schema.max_properties {
        push_violation(
            out,
            indent,
            &count_comparison(count, ">", max),
            path,
            &format!(
                "{} + {count}",
                cs_string(&format!("must have at most {max} properties, got "))
            ),
        );
    }
}

/// `propertyNames` over a sequence of member keys.
pub(super) fn property_name_checks(
    out: &mut String,
    keys: &str,
    path: &str,
    subschema: &Schema,
    indent: &str,
) {
    let rt = super::runtime_prefix();
    let constraints = StringConstraints::from_schema(subschema);
    if constraints.is_empty() && subschema.const_value.is_none() && subschema.enum_values.is_none()
    {
        return;
    }
    let inner = format!("{indent}    ");
    let key_path = format!("{rt}Violation.Member({path}, propertyName)");
    out.push_str(&format!(
        "{indent}foreach (var propertyName in {keys})\n{indent}{{\n"
    ));
    // Every predicate records its own reason; the key's prefix is applied to
    // all of them at once below, so no check can forget it.
    out.push_str(&format!(
        "{inner}var nameViolationStart = violations.Count;\n"
    ));
    if constraints.has_length() {
        out.push_str(&format!(
            "{inner}var nameLength = {};\n",
            code_points("propertyName")
        ));
        for (bound, op, reason) in [
            (constraints.min_length, "<", "must have length >="),
            (constraints.max_length, ">", "must have length <="),
        ] {
            if let Some(bound) = bound {
                push_violation(
                    out,
                    &inner,
                    &count_comparison("nameLength", op, bound),
                    &key_path,
                    &format!(
                        "{} + nameLength",
                        cs_string(&format!("{reason} {bound}, got "))
                    ),
                );
            }
        }
    }
    string_checks(
        out,
        "propertyName",
        &key_path,
        RegexSite::Static(PROPERTY_NAME_POSITION),
        &constraints.without_length(),
        &inner,
        1,
    );
    closed_scalar_checks(
        out,
        "propertyName",
        &key_path,
        subschema,
        &CsType::String,
        &inner,
    );
    out.push_str(&format!(
        "{inner}for (var nameViolation = nameViolationStart; nameViolation < violations.Count; nameViolation++)\n{inner}{{\n{inner}    violations[nameViolation] = violations[nameViolation] with {{ Reason = \"invalid property name \\\"\" + propertyName + \"\\\": \" + violations[nameViolation].Reason }};\n{inner}}}\n"
    ));
    out.push_str(&format!("{indent}}}\n"));
}

/// `dependentRequired` over a presence predicate (`present(key)` is a C#
/// boolean expression).
pub(super) fn dependent_required_checks(
    out: &mut String,
    schema: &Schema,
    path: &str,
    indent: &str,
    present: &dyn Fn(&str) -> String,
) {
    let rt = super::runtime_prefix();
    let Some(dependent_required) = &schema.dependent_required else {
        return;
    };
    for (trigger, dependents) in dependent_required {
        if dependents.is_empty() {
            continue;
        }
        out.push_str(&format!("{indent}if ({})\n{indent}{{\n", present(trigger)));
        for dependent in dependents {
            let reason =
                format!("property \"{dependent}\" is required when \"{trigger}\" is present");
            push_violation(
                out,
                &format!("{indent}    "),
                &format!("!({})", present(dependent)),
                &format!(
                    "{rt}Violation.Join({path}, {})",
                    cs_string(&crate::generator::json_schema::violation_member_segment(
                        dependent
                    ))
                ),
                &cs_string(&reason),
            );
        }
        out.push_str(&format!("{indent}}}\n"));
    }
}

pub(super) const MAP_MEMBER_POSITION: &str = "Value";
pub(super) const ADDITIONAL_PROPERTIES_POSITION: &str = "AdditionalPropertiesValue";
pub(super) const PROPERTY_NAME_POSITION: &str = "PropertyName";

// ---------------------------------------------------------------------------
// In-memory representability (serialize side)
// ---------------------------------------------------------------------------

/// Emits the predicates an in-memory value of `ty` needs before it can be
/// written: finiteness, the integer cap, temporal representability, and the
/// `items` constraints of nested arrays. Shared by declared members, map
/// members, and union wrappers.
pub(super) fn representability_checks(
    out: &mut String,
    ty: &CsType,
    schema: Option<&Schema>,
    value: &str,
    path: &str,
    indent: &str,
    depth: usize,
) {
    let rt = super::runtime_prefix();
    match ty {
        CsType::Double => push_violation(
            out,
            indent,
            &format!("!double.IsFinite({value})"),
            path,
            &format!("\"must be a finite number, got \" + {value}"),
        ),
        CsType::Long => push_violation(
            out,
            indent,
            &format!("{value} < -{rt}SpecJson.IntegerCap || {value} > {rt}SpecJson.IntegerCap"),
            path,
            "\"exceeds \\u00b1(2^53-1) integer cap\"",
        ),
        CsType::Temporal(kind) => out.push_str(&format!(
            "{indent}{rt}TemporalSupport.{}({value}, {path}, violations);\n",
            temporal_check_fn(*kind)
        )),
        // A nested model or union validates itself, re-rooted at this path.
        CsType::Ref { name } | CsType::Union { name } => out.push_str(&format!(
            "{indent}{name}.Converter.Validate({value}, {path}, violations);\n"
        )),
        CsType::List(element, _) => {
            let element_schema = schema.and_then(element_shape);
            let nullable_elements = schema
                .and_then(|schema| schema.items.as_deref())
                .is_some_and(super::schema::allows_null);
            let index = format!("index{depth}");
            let item = format!("item{depth}");
            let item_path = format!("{rt}Violation.Index({path}, {index})");
            // A reference element (or a nullable one) can be null in memory; a
            // non-nullable value-type element cannot.
            let null_check = !element.is_value_type() || nullable_elements;
            let null_violation = null_check && !nullable_elements;
            let inner = if null_check {
                format!("{indent}        ")
            } else {
                format!("{indent}    ")
            };
            let mut body = String::new();
            representability_checks(
                &mut body,
                element,
                element_schema,
                &item,
                &item_path,
                &inner,
                depth + 1,
            );
            if let Some(element_schema) = element_schema {
                value_checks(
                    &mut body,
                    element,
                    element_schema,
                    &item,
                    &item_path,
                    RegexSite::Inline,
                    &inner,
                    depth + 1,
                );
            }
            if !body.is_empty() || null_violation {
                out.push_str(&format!(
                    "{indent}for (var {index} = 0; {index} < {value}.Count; {index}++)\n{indent}{{\n"
                ));
                if null_check {
                    out.push_str(&format!(
                        "{indent}    if ({value}[{index}] is {{ }} {item})\n{indent}    {{\n{body}{indent}    }}\n"
                    ));
                    if null_violation {
                        out.push_str(&format!(
                            "{indent}    else\n{indent}    {{\n{indent}        violations.Add(new {rt}Violation({item_path}, \"explicit null not allowed\"));\n{indent}    }}\n"
                        ));
                    }
                } else {
                    out.push_str(&format!(
                        "{indent}    var {item} = {value}[{index}];\n{body}"
                    ));
                }
                out.push_str(&format!("{indent}}}\n"));
            }
            if let Some(schema) = schema {
                let constraints = ArrayConstraints::from_schema(schema);
                if !constraints.is_empty() {
                    array_checks(out, value, path, element, &constraints, None, indent, depth);
                }
            }
        }
        _ => {}
    }
}

/// The value-level constraints a nested position declares over an in-memory
/// value of `ty` (strings, numbers, closed sets, materialized wire strings).
pub(super) fn value_checks(
    out: &mut String,
    ty: &CsType,
    schema: &Schema,
    value: &str,
    path: &str,
    site: RegexSite,
    indent: &str,
    depth: usize,
) {
    match ty {
        CsType::String => {
            string_checks(
                out,
                value,
                path,
                site,
                &StringConstraints::from_schema(schema),
                indent,
                depth,
            );
            closed_scalar_checks(out, value, path, schema, ty, indent);
        }
        CsType::Long | CsType::Double => {
            numeric_checks(
                out,
                value,
                path,
                &NumericConstraints::from_schema(schema),
                matches!(ty, CsType::Long),
                indent,
            );
            closed_scalar_checks(out, value, path, schema, ty, indent);
        }
        CsType::Boolean => closed_scalar_checks(out, value, path, schema, ty, indent),
        CsType::Temporal(_) | CsType::Bytes(_) => {
            let constraints = StringConstraints::from_schema(schema);
            if constraints.is_empty() && schema.closed_values().is_empty() {
                return;
            }
            let wire = format!("wire{depth}");
            out.push_str(&format!(
                "{indent}var {wire} = {};\n",
                materialized_wire(ty, value)
            ));
            string_checks(out, &wire, path, site, &constraints, indent, depth);
            closed_scalar_checks(out, &wire, path, schema, &CsType::String, indent);
        }
        _ => {}
    }
}

/// The canonical wire string of a materialized (temporal / bytes) value.
pub(super) fn materialized_wire(ty: &CsType, value: &str) -> String {
    let rt = super::runtime_prefix();
    match ty {
        CsType::Temporal(kind) => {
            format!("{rt}TemporalSupport.{}({value})", temporal_format_fn(*kind))
        }
        CsType::Bytes(encoding) => {
            format!(
                "{rt}Base64Support.{}({value})",
                content_encoding_format_fn(*encoding)
            )
        }
        _ => value.to_string(),
    }
}
