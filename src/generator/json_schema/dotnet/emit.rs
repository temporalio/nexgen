//! Rendering of classified models into C# source.
//!
//! Every model is a `sealed record` (or, for a `oneOf`, an interface) carrying
//! a nested `Converter : JsonConverter<T>`. The converter's static `Parse`,
//! `Validate` and `WriteValue` methods take the violation path of the value
//! they handle, so a nested model aggregates into its parent's violation list
//! directly; only the outermost `Read`/`Write` throws the one
//! `PayloadValidationError` (P11). `Write` validates the complete value tree
//! before writing its first token (P11.1/P12).

use serde_json::Value;

use crate::json_schema::format::TemporalKind;

use super::checks::{
    self, ADDITIONAL_PROPERTIES_POSITION, MAP_MEMBER_POSITION, PROPERTY_NAME_POSITION, RegexSite,
    closed_reason_prefix, content_encoding_parse_fn, materialized_wire, raw_array_checks,
    regex_statics, representability_checks, scalar_literal, temporal_parse_fn, value_checks,
};
use super::schema::{ArrayConstraints, Schema, StringConstraints, allows_null, element_shape};
use super::types::{
    ClosedNameOverrides, CsType, CsUnion, DICTIONARY, FieldPlan, JSON_ELEMENT, JSON_VALUE_KIND,
    ModelKind, READ_ONLY_DICTIONARY, TypedAdditionalPlan, UnionVariant, closed_constant_name,
    default_accessor_name,
};
use super::{GENERATED_CODE_ATTRIBUTE, cs_string};
use crate::generator::dotnet::obsolete_attribute;

const JSON_CONVERTER: &str = "global::System.Text.Json.Serialization.JsonConverter";
const UTF8_JSON_READER: &str = "global::System.Text.Json.Utf8JsonReader";
const UTF8_JSON_WRITER: &str = "global::System.Text.Json.Utf8JsonWriter";
const SERIALIZER_OPTIONS: &str = "global::System.Text.Json.JsonSerializerOptions";

fn member_path(json_name: &str) -> String {
    let rt = super::runtime_prefix();
    format!(
        "{rt}Violation.Join(path, {})",
        cs_string(&crate::generator::json_schema::violation_member_segment(
            json_name
        ))
    )
}

// ---------------------------------------------------------------------------
// Documentation
// ---------------------------------------------------------------------------

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Writes an XML doc comment for a public declaration: the authored
/// `title`/`description` text, or `fallback` (a short name-led sentence) when
/// the schema carries none, so every public generated member is documented.
pub(super) fn doc_comment(
    out: &mut String,
    indent: &str,
    doc: Option<&str>,
    fallback: &str,
    deprecated: Option<&str>,
) {
    let doc = match doc.map(str::trim).filter(|doc| !doc.is_empty()) {
        Some(authored) => xml_escape(authored),
        None => fallback.to_string(),
    };
    if !doc.is_empty() {
        out.push_str(&format!("{indent}/// <summary>\n"));
        for line in doc.lines() {
            let line = line.trim();
            if line.is_empty() {
                out.push_str(&format!("{indent}/// <para/>\n"));
            } else {
                out.push_str(&format!("{indent}/// {line}\n"));
            }
        }
        out.push_str(&format!("{indent}/// </summary>\n"));
    }
    if let Some(kind) = deprecated {
        out.push_str(&format!("{indent}{}\n", obsolete_attribute(kind)));
    }
}

// ---------------------------------------------------------------------------
// Object models
// ---------------------------------------------------------------------------

pub(super) struct ObjectModel<'a> {
    pub(super) name: &'a str,
    pub(super) schema: &'a Schema,
    pub(super) implements: &'a [String],
}

pub(super) fn render_object(out: &mut String, model: &ObjectModel, kind: &ModelKind) {
    let schema = model.schema;
    doc_comment(
        out,
        "",
        super::schema::compose_doc(schema.title.as_deref(), schema.description.as_deref())
            .as_deref(),
        &format!("The {} model.", model.name),
        (schema.deprecated == Some(true)).then_some("type"),
    );
    out.push_str(&format!("{GENERATED_CODE_ATTRIBUTE}\n"));
    out.push_str(&format!(
        "[{JSON_CONVERTER}(typeof({}.Converter))]\n",
        model.name
    ));
    let implements = if model.implements.is_empty() {
        String::new()
    } else {
        format!(" : {}", model.implements.join(", "))
    };
    out.push_str(&format!(
        "public sealed record {}{implements}\n{{\n",
        model.name
    ));
    let mut body = String::new();
    match kind {
        ModelKind::Object {
            open,
            fields,
            typed_additional,
        } => render_object_body(&mut body, model, *open, fields, typed_additional.as_ref()),
        ModelKind::TypedMap(additional) => render_typed_map_body(&mut body, model, additional),
    }
    out.push_str(body.trim_end());
    out.push_str("\n}\n");
}

fn additional_value_type(typed_additional: Option<&TypedAdditionalPlan>) -> String {
    typed_additional
        .map(TypedAdditionalPlan::value_type)
        .unwrap_or_else(|| JSON_ELEMENT.to_string())
}

fn render_object_body(
    out: &mut String,
    model: &ObjectModel,
    open: bool,
    fields: &[FieldPlan],
    typed_additional: Option<&TypedAdditionalPlan>,
) {
    // Nested declarations: inline unions and closed value classes.
    for field in fields {
        if field.nested_union
            && let Some(union) = &field.union
        {
            render_union_interface(out, union, "    ", None, false);
            out.push('\n');
        }
        if let CsType::ClosedValue { name, wire } = &field.ty {
            render_closed_value_class(
                out,
                name,
                wire,
                &field.closed_values,
                &field.closed_overrides,
            );
            out.push('\n');
        }
    }

    for field in fields {
        doc_comment(
            out,
            "    ",
            field.doc.as_deref(),
            &format!("The <c>{}</c> member.", xml_escape(&field.json_name)),
            field.deprecated.then_some("field"),
        );
        let required = if field.required { "required " } else { "" };
        out.push_str(&format!(
            "    public {required}{} {} {{ get; init; }}\n\n",
            field.declared_type(),
            field.name
        ));
    }
    if open {
        let value_type = additional_value_type(typed_additional);
        out.push_str(&format!(
            "    /// <summary>\n    /// Members not declared by the schema, preserved verbatim.\n    /// </summary>\n    public {READ_ONLY_DICTIONARY}<string, {value_type}> AdditionalProperties {{ get; init; }} = new {DICTIONARY}<string, {value_type}>();\n\n"
        ));
    }
    for field in fields {
        if field.required {
            continue;
        }
        let Some(default) = &field.default else {
            continue;
        };
        let Some(expr) = default_expr(field, default) else {
            continue;
        };
        let ty = match &field.ty {
            CsType::List(..) => continue,
            other => other.name(),
        };
        out.push_str(&format!(
            "    /// <summary>\n    /// <see cref=\"{}\"/>, or its schema default when absent.\n    /// </summary>\n",
            field.name
        ));
        if field.deprecated {
            out.push_str(&format!("    {}\n", obsolete_attribute("field")));
        }
        out.push_str(&format!(
            "    public {ty} {} => this.{} ?? {expr};\n\n",
            default_accessor_name(&field.name),
            field.name
        ));
    }

    let mut members: Vec<String> = fields.iter().map(|field| field.name.clone()).collect();
    if open {
        members.push("AdditionalProperties".to_string());
    }
    render_equality(out, model.name, &members);

    // The converter.
    out.push_str(&format!(
        "    /// <summary>\n    /// Converts <see cref=\"{name}\"/> to and from JSON, enforcing the schema in both directions.\n    /// </summary>\n    internal sealed class Converter : {JSON_CONVERTER}<{name}>\n    {{\n",
        name = model.name
    ));
    let mut statics = String::new();
    for field in fields {
        regex_statics(
            &mut statics,
            "        ",
            &field.name,
            &field.string,
            &field.array,
        );
    }
    if let Some(additional) = typed_additional {
        regex_statics(
            &mut statics,
            "        ",
            ADDITIONAL_PROPERTIES_POSITION,
            &StringConstraints::from_schema(&additional.schema),
            &ArrayConstraints::from_schema(&additional.schema),
        );
    }
    if let Some(names) = &model.schema.property_names {
        regex_statics(
            &mut statics,
            "        ",
            PROPERTY_NAME_POSITION,
            &StringConstraints::from_schema(names),
            &ArrayConstraints::default(),
        );
    }
    if !statics.is_empty() {
        out.push_str(&statics);
        out.push('\n');
    }
    render_converter_entry_points(out, model.name, false);
    render_object_parse(out, model, open, fields, typed_additional);
    render_object_validate(out, model, open, fields, typed_additional);
    render_object_write(out, model.name, open, fields, typed_additional);
    out.push_str("    }\n");
}

/// The `Read`/`Write` overrides shared by every converter: one parse or one
/// validate over the whole value, and one aggregated failure.
fn render_converter_entry_points(out: &mut String, name: &str, nullable: bool) {
    let rt = super::runtime_prefix();
    let violations_type = super::types::violations_type();
    let result = if nullable {
        format!("{name}?")
    } else {
        name.to_string()
    };
    out.push_str("        /// <inheritdoc/>\n        public override bool HandleNull => true;\n\n");
    out.push_str(&format!(
        "        /// <inheritdoc/>\n        public override {result} Read(ref {UTF8_JSON_READER} reader, global::System.Type typeToConvert, {SERIALIZER_OPTIONS} options)\n        {{\n            var node = {JSON_ELEMENT}.ParseValue(ref reader);\n            var violations = new {violations_type}();\n            var value = Parse(node, \"\", violations);\n"
    ));
    if nullable {
        out.push_str(
            &format!("            if (violations.Count > 0)\n            {{\n                throw {rt}Violation.Failure(violations);\n            }}\n            return value;\n        }}\n\n"),
        );
    } else {
        out.push_str(
            &format!("            if (value is null || violations.Count > 0)\n            {{\n                throw {rt}Violation.Failure(violations);\n            }}\n            return value;\n        }}\n\n"),
        );
    }
    out.push_str(&format!(
        "        /// <inheritdoc/>\n        public override void Write({UTF8_JSON_WRITER} writer, {name} value, {SERIALIZER_OPTIONS} options)\n        {{\n            var violations = new {violations_type}();\n"
    ));
    if nullable {
        out.push_str("            if (value is null)\n            {\n                writer.WriteNullValue();\n                return;\n            }\n");
    } else {
        out.push_str(&format!("            if (value is null)\n            {{\n                violations.Add(new {rt}Violation(\"\", \"expected object\"));\n                throw {rt}Violation.Failure(violations);\n            }}\n"));
    }
    out.push_str(
        &format!("            Validate(value, \"\", violations);\n            if (violations.Count > 0)\n            {{\n                throw {rt}Violation.Failure(violations);\n            }}\n            WriteValue(writer, value);\n        }}\n\n"),
    );
}

fn render_equality(out: &mut String, name: &str, members: &[String]) {
    let rt = super::runtime_prefix();
    out.push_str(&format!(
        "    /// <inheritdoc/>\n    public bool Equals({name}? other)\n    {{\n        if (other is null)\n        {{\n            return false;\n        }}\n        if (ReferenceEquals(this, other))\n        {{\n            return true;\n        }}\n"
    ));
    if members.is_empty() {
        out.push_str("        return true;\n    }\n\n");
    } else {
        out.push_str("        return ");
        out.push_str(
            &members
                .iter()
                .map(|member| format!("{rt}ValueEquality.Equal(this.{member}, other.{member})"))
                .collect::<Vec<_>>()
                .join("\n            && "),
        );
        out.push_str(";\n    }\n\n");
    }
    out.push_str("    /// <inheritdoc/>\n    public override int GetHashCode()\n    {\n        var hash = new global::System.HashCode();\n");
    for member in members {
        out.push_str(&format!(
            "        hash.Add({rt}ValueEquality.Hash(this.{member}));\n"
        ));
    }
    out.push_str("        return hash.ToHashCode();\n    }\n\n");
}

fn render_object_parse(
    out: &mut String,
    model: &ObjectModel,
    open: bool,
    fields: &[FieldPlan],
    typed_additional: Option<&TypedAdditionalPlan>,
) {
    let rt = super::runtime_prefix();
    let violations_type = super::types::violations_type();
    let name = model.name;
    let indent = "            ";
    out.push_str(&format!(
        "        internal static {name}? Parse({JSON_ELEMENT} node, string path, {violations_type} violations)\n        {{\n"
    ));
    out.push_str(&format!(
        "{indent}if (node.ValueKind != {JSON_VALUE_KIND}.Object)\n{indent}{{\n{indent}    violations.Add(new {rt}Violation(path, \"expected object\"));\n{indent}    return null;\n{indent}}}\n"
    ));
    out.push_str(&format!(
        "{indent}var start = violations.Count;\n{indent}var members = {rt}SpecJson.Members(node);\n"
    ));
    if open {
        out.push_str(&format!(
            "{indent}var additionalProperties = new {DICTIONARY}<string, {}>();\n",
            additional_value_type(typed_additional)
        ));
    }
    // Undeclared members: rejected for a closed model, preserved for an open
    // one (P13), and validated against a typed catch-all.
    let known: Vec<&str> = fields
        .iter()
        .map(|field| field.json_name.as_str())
        .collect();
    out.push_str(&format!(
        "{indent}foreach (var member in members)\n{indent}{{\n"
    ));
    if !known.is_empty() {
        out.push_str(&format!(
            "{indent}    if ({})\n{indent}    {{\n{indent}        continue;\n{indent}    }}\n",
            known
                .iter()
                .map(|name| format!("member.Key == {}", cs_string(name)))
                .collect::<Vec<_>>()
                .join(" || ")
        ));
    }
    let inner = format!("{indent}    ");
    if !open {
        out.push_str(&format!(
            "{inner}violations.Add(new {rt}Violation({rt}Violation.Member(path, member.Key), \"unknown field\"));\n"
        ));
    } else if let Some(additional) = typed_additional {
        out.push_str(&format!(
            "{inner}var memberPath = {rt}Violation.Member(path, member.Key);\n"
        ));
        out.push_str(&format!(
            "{inner}if (member.Value.ValueKind == {JSON_VALUE_KIND}.Null)\n{inner}{{\n"
        ));
        if additional.nullable {
            out.push_str(&format!(
                "{inner}    additionalProperties[member.Key] = null;\n{inner}    continue;\n"
            ));
        } else {
            out.push_str(&format!(
                "{inner}    violations.Add(new {rt}Violation(memberPath, \"explicit null not allowed\"));\n{inner}    continue;\n"
            ));
        }
        out.push_str(&format!("{inner}}}\n"));
        parse_into(
            out,
            &ParseTarget {
                ty: &additional.ty,
                schema: Some(&additional.schema),
                node: "member.Value",
                path: "memberPath",
                site: RegexSite::Static(ADDITIONAL_PROPERTIES_POSITION),
                closed: None,
                kind_known: false,
            },
            &|value| format!("additionalProperties[member.Key] = {value};"),
            &inner,
            0,
        );
    } else {
        out.push_str(&format!(
            "{inner}additionalProperties[member.Key] = member.Value;\n"
        ));
    }
    out.push_str(&format!("{indent}}}\n"));

    for field in fields {
        render_field_parse(out, field, indent);
    }

    checks::property_count_checks(out, "members.Count", "path", model.schema, indent);
    if let Some(names) = &model.schema.property_names {
        checks::property_name_checks(out, "members.Keys", "path", names, indent);
    }
    checks::dependent_required_checks(out, model.schema, "path", indent, &|key| {
        format!("members.ContainsKey({})", cs_string(key))
    });

    out.push_str(&format!(
        "{indent}if (violations.Count > start)\n{indent}{{\n{indent}    return null;\n{indent}}}\n"
    ));
    out.push_str(&format!("{indent}return new {name}\n{indent}{{\n"));
    for field in fields {
        let value = if field.always_present_value() {
            format!("parsed{} ?? default", field.name)
        } else if field.required && !field.nullable {
            format!("parsed{}!", field.name)
        } else {
            format!("parsed{}", field.name)
        };
        out.push_str(&format!("{indent}    {} = {value},\n", field.name));
    }
    if open {
        out.push_str(&format!(
            "{indent}    AdditionalProperties = additionalProperties,\n"
        ));
    }
    out.push_str(&format!("{indent}}};\n        }}\n\n"));
}

fn local_type(field: &FieldPlan) -> String {
    let declared = field.declared_type();
    if declared.ends_with('?') {
        declared
    } else {
        format!("{declared}?")
    }
}

fn render_field_parse(out: &mut String, field: &FieldPlan, indent: &str) {
    let rt = super::runtime_prefix();
    let path = member_path(&field.json_name);
    let local = format!("parsed{}", field.name);
    let node = format!("node{}", field.name);
    out.push_str(&format!("{indent}{} {local} = null;\n", local_type(field)));
    out.push_str(&format!(
        "{indent}if (!members.TryGetValue({}, out var {node}))\n{indent}{{\n",
        cs_string(&field.json_name)
    ));
    if field.required {
        out.push_str(&format!(
            "{indent}    violations.Add(new {rt}Violation({path}, \"required\"));\n"
        ));
    }
    out.push_str(&format!(
        "{indent}}}\n{indent}else if ({node}.ValueKind == {JSON_VALUE_KIND}.Null)\n{indent}{{\n"
    ));
    if !field.nullable {
        out.push_str(&format!(
            "{indent}    violations.Add(new {rt}Violation({path}, \"explicit null not allowed\"));\n"
        ));
    }
    out.push_str(&format!("{indent}}}\n{indent}else\n{indent}{{\n"));
    let element_nullable = field.nullable_items;
    let ty = match &field.ty {
        CsType::List(element, nullable) => CsType::List(element.clone(), *nullable),
        other => other.clone(),
    };
    parse_into(
        out,
        &ParseTarget {
            ty: &ty,
            schema: Some(&field.schema),
            node: &node,
            path: &path,
            site: RegexSite::Static(&field.name),
            closed: Some((&field.closed_values, &field.closed_overrides)),
            kind_known: false,
        },
        &|value| format!("{local} = {value};"),
        &format!("{indent}    "),
        0,
    );
    let _ = element_nullable;
    out.push_str(&format!("{indent}}}\n"));
}

/// One value to decode from a `JsonElement`.
pub(super) struct ParseTarget<'a> {
    pub(super) ty: &'a CsType,
    /// The position's own schema (its constraints and `items`).
    pub(super) schema: Option<&'a Schema>,
    pub(super) node: &'a str,
    pub(super) path: &'a str,
    pub(super) site: RegexSite<'a>,
    pub(super) closed: Option<(&'a [Value], &'a ClosedNameOverrides)>,
    /// True when the caller already dispatched on the JSON value kind.
    pub(super) kind_known: bool,
}

/// Opens `if (<mismatch>) { violation } else {`, the caller closing the
/// `else`. When the dispatcher already selected the value kind, opens a bare
/// block instead.
fn kind_check(out: &mut String, target: &ParseTarget, kind: &str, expected: &str, indent: &str) {
    let rt = super::runtime_prefix();
    if target.kind_known {
        out.push_str(&format!("{indent}{{\n"));
        return;
    }
    let node = target.node;
    let mismatch = match kind {
        "Boolean" => {
            format!("{node}.ValueKind is not ({JSON_VALUE_KIND}.True or {JSON_VALUE_KIND}.False)")
        }
        kind => format!("{node}.ValueKind != {JSON_VALUE_KIND}.{kind}"),
    };
    out.push_str(&format!(
        "{indent}if ({mismatch})\n{indent}{{\n{indent}    violations.Add(new {rt}Violation({}, \"expected {expected}\"));\n{indent}}}\n{indent}else\n{indent}{{\n",
        target.path
    ));
}

/// Decodes `target.node` into `sink(value)`, running the position's own
/// constraints on the way in.
pub(super) fn parse_into(
    out: &mut String,
    target: &ParseTarget,
    sink: &dyn Fn(&str) -> String,
    indent: &str,
    depth: usize,
) {
    let rt = super::runtime_prefix();
    let node = target.node;
    let path = target.path;
    let inner = format!("{indent}    ");
    let empty = Schema::default();
    let schema = target.schema.unwrap_or(&empty);
    match target.ty {
        CsType::ClosedValue { name, wire } => {
            let (values, overrides) = target
                .closed
                .expect("closed value parses with its value set");
            let raw = format!("closed{depth}");
            let parsed = parse_scalar(out, target, wire, &raw, indent);
            let Some(parsed) = parsed else { return };
            let body_indent = format!("{indent}    ");
            for (index, value) in values.iter().enumerate() {
                let keyword = if index == 0 { "if" } else { "else if" };
                let test = match wire.as_ref() {
                    CsType::String => format!(
                        "string.Equals({parsed}, {}, global::System.StringComparison.Ordinal)",
                        scalar_literal(value, wire)
                    ),
                    _ => format!("{parsed} == {}", scalar_literal(value, wire)),
                };
                out.push_str(&format!(
                    "{body_indent}{keyword} ({test})\n{body_indent}{{\n{body_indent}    {}\n{body_indent}}}\n",
                    sink(&format!("{name}.{}", closed_constant_name(overrides, value)))
                ));
            }
            out.push_str(&format!(
                "{body_indent}else\n{body_indent}{{\n{body_indent}    violations.Add(new {rt}Violation({path}, {} + {parsed}));\n{body_indent}}}\n",
                cs_string(&closed_reason_prefix(values))
            ));
            out.push_str(&format!("{indent}}}\n"));
        }
        CsType::String | CsType::Long | CsType::Double | CsType::Boolean => {
            let raw = format!("value{depth}");
            let Some(parsed) = parse_scalar(out, target, target.ty, &raw, indent) else {
                return;
            };
            value_checks(
                out,
                target.ty,
                schema,
                &parsed,
                path,
                target.site,
                &inner,
                depth,
            );
            out.push_str(&format!("{inner}{}\n{indent}}}\n", sink(&parsed)));
        }
        CsType::Temporal(kind) => {
            kind_check(out, target, "String", "string", indent);
            let text = format!("text{depth}");
            out.push_str(&format!("{inner}var {text} = {node}.GetString()!;\n"));
            checks::string_checks(
                out,
                &text,
                path,
                target.site,
                &StringConstraints::from_schema(schema),
                &inner,
                depth,
            );
            checks::closed_scalar_checks(out, &text, path, schema, &CsType::String, &inner);
            let parsed = format!("temporal{depth}");
            out.push_str(&format!(
                "{inner}if ({rt}TemporalSupport.{}({text}, {path}, violations) is {{ }} {parsed})\n{inner}{{\n{inner}    {}\n{inner}}}\n{indent}}}\n",
                temporal_parse_fn(*kind),
                sink(&parsed)
            ));
        }
        CsType::Bytes(encoding) => {
            kind_check(out, target, "String", "string", indent);
            let text = format!("text{depth}");
            out.push_str(&format!("{inner}var {text} = {node}.GetString()!;\n"));
            checks::string_checks(
                out,
                &text,
                path,
                target.site,
                &StringConstraints::from_schema(schema),
                &inner,
                depth,
            );
            checks::closed_scalar_checks(out, &text, path, schema, &CsType::String, &inner);
            let parsed = format!("bytes{depth}");
            out.push_str(&format!(
                "{inner}if ({rt}Base64Support.{}({text}, {path}, violations) is {{ }} {parsed})\n{inner}{{\n{inner}    {}\n{inner}}}\n{indent}}}\n",
                content_encoding_parse_fn(*encoding),
                sink(&parsed)
            ));
        }
        CsType::Ref { name } | CsType::Union { name } => {
            let parsed = format!("model{depth}");
            out.push_str(&format!(
                "{indent}if ({name}.Converter.Parse({node}, {path}, violations) is {{ }} {parsed})\n{indent}{{\n{indent}    {}\n{indent}}}\n",
                sink(&parsed)
            ));
        }
        CsType::List(element, _) => {
            let nullable_elements = schema.items.as_deref().is_some_and(allows_null);
            let items = format!("items{depth}");
            let index = format!("index{depth}");
            let item = format!("item{depth}");
            let item_path = format!("itemPath{depth}");
            kind_check(out, target, "Array", "array", indent);
            out.push_str(&format!(
                "{inner}var {items} = new {}();\n{inner}var {index} = 0;\n{inner}foreach (var {item} in {node}.EnumerateArray())\n{inner}{{\n",
                CsType::list_builder(element, nullable_elements)
            ));
            let loop_indent = format!("{inner}    ");
            out.push_str(&format!(
                "{loop_indent}var {item_path} = {rt}Violation.Index({path}, {index}++);\n"
            ));
            out.push_str(&format!(
                "{loop_indent}if ({item}.ValueKind == {JSON_VALUE_KIND}.Null)\n{loop_indent}{{\n"
            ));
            if nullable_elements {
                out.push_str(&format!("{loop_indent}    {items}.Add(null);\n"));
            } else {
                out.push_str(&format!(
                    "{loop_indent}    violations.Add(new {rt}Violation({item_path}, \"explicit null not allowed\"));\n"
                ));
            }
            out.push_str(&format!("{loop_indent}    continue;\n{loop_indent}}}\n"));
            let add = |value: &str| format!("{items}.Add({value});");
            parse_into(
                out,
                &ParseTarget {
                    ty: element,
                    schema: element_shape(schema),
                    node: &item,
                    path: &item_path,
                    site: RegexSite::Inline,
                    closed: None,
                    kind_known: false,
                },
                &add,
                &loop_indent,
                depth + 1,
            );
            out.push_str(&format!("{inner}}}\n"));
            let constraints = ArrayConstraints::from_schema(schema);
            if !constraints.is_empty() {
                let position = match target.site {
                    RegexSite::Static(position) => Some(position),
                    RegexSite::Inline => None,
                };
                raw_array_checks(
                    out,
                    node,
                    path,
                    element,
                    &constraints,
                    position,
                    &inner,
                    depth,
                );
            }
            out.push_str(&format!("{inner}{}\n{indent}}}\n", sink(&items)));
        }
    }
}

/// Opens the kind check for a scalar and binds the decoded value. Returns the
/// C# expression holding it, inside an open `else {` block the caller closes;
/// `None` when nothing was opened.
fn parse_scalar(
    out: &mut String,
    target: &ParseTarget,
    ty: &CsType,
    local: &str,
    indent: &str,
) -> Option<String> {
    let rt = super::runtime_prefix();
    let inner = format!("{indent}    ");
    let node = target.node;
    let path = target.path;
    match ty {
        CsType::String => {
            kind_check(out, target, "String", "string", indent);
            out.push_str(&format!("{inner}var {local} = {node}.GetString()!;\n"));
            Some(local.to_string())
        }
        CsType::Boolean => {
            kind_check(out, target, "Boolean", "boolean", indent);
            out.push_str(&format!("{inner}var {local} = {node}.GetBoolean();\n"));
            Some(local.to_string())
        }
        CsType::Long | CsType::Double => {
            let function = if matches!(ty, CsType::Long) {
                "SpecLong"
            } else {
                "SpecDouble"
            };
            out.push_str(&format!(
                "{indent}if ({rt}SpecJson.{function}({node}, {path}, violations) is {{ }} {local})\n{indent}{{\n"
            ));
            Some(local.to_string())
        }
        _ => None,
    }
}

fn render_object_validate(
    out: &mut String,
    model: &ObjectModel,
    open: bool,
    fields: &[FieldPlan],
    typed_additional: Option<&TypedAdditionalPlan>,
) {
    let rt = super::runtime_prefix();
    let violations_type = super::types::violations_type();
    let indent = "            ";
    out.push_str(&format!(
        "        internal static void Validate({} value, string path, {violations_type} violations)\n        {{\n",
        model.name
    ));
    for field in fields {
        render_field_validate(out, field, indent);
    }
    let present = |field: &FieldPlan| -> String {
        if field.always_present_value() || (field.required && field.nullable) {
            "true".to_string()
        } else {
            format!("value.{} is not null", field.name)
        }
    };
    if open {
        out.push_str(&format!(
            "{indent}var wireKeys = new global::System.Collections.Generic.HashSet<string>(global::System.StringComparer.Ordinal);\n"
        ));
        for field in fields {
            let key = cs_string(&field.json_name);
            match present(field).as_str() {
                "true" => out.push_str(&format!("{indent}wireKeys.Add({key});\n")),
                condition => out.push_str(&format!(
                    "{indent}if ({condition})\n{indent}{{\n{indent}    wireKeys.Add({key});\n{indent}}}\n"
                )),
            }
        }
        out.push_str(&format!(
            "{indent}if (value.AdditionalProperties is null)\n{indent}{{\n{indent}    violations.Add(new {rt}Violation({rt}Violation.Join(path, \"AdditionalProperties\"), \"required\"));\n{indent}}}\n{indent}else\n{indent}{{\n"
        ));
        let inner = format!("{indent}    ");
        // The catch-all can only hold members the declared set does not name,
        // whether or not the declared member is set: writing one would emit a
        // member the declared type must parse (or emit it twice).
        let declared = fields
            .iter()
            .map(|field| cs_string(&field.json_name))
            .collect::<Vec<_>>();
        let collides = if declared.is_empty() {
            "!wireKeys.Add(entry.Key)".to_string()
        } else {
            format!(
                "entry.Key is {} || !wireKeys.Add(entry.Key)",
                declared.join(" or ")
            )
        };
        out.push_str(&format!(
            "{inner}foreach (var entry in value.AdditionalProperties)\n{inner}{{\n{inner}    if ({collides})\n{inner}    {{\n{inner}        violations.Add(new {rt}Violation({rt}Violation.Member(path, entry.Key), \"declared property key collision\"));\n{inner}    }}\n"
        ));
        if let Some(additional) = typed_additional {
            render_map_entry_validate(
                out,
                additional,
                ADDITIONAL_PROPERTIES_POSITION,
                &format!("{inner}    "),
            );
        }
        out.push_str(&format!("{inner}}}\n{indent}}}\n"));
        checks::property_count_checks(out, "wireKeys.Count", "path", model.schema, indent);
        if let Some(names) = &model.schema.property_names {
            checks::property_name_checks(out, "wireKeys", "path", names, indent);
        }
        checks::dependent_required_checks(out, model.schema, "path", indent, &|key| {
            format!("wireKeys.Contains({})", cs_string(key))
        });
    } else {
        if model.schema.min_properties.is_some() || model.schema.max_properties.is_some() {
            out.push_str(&format!("{indent}var wireKeyCount = 0;\n"));
            for field in fields {
                match present(field).as_str() {
                    "true" => out.push_str(&format!("{indent}wireKeyCount++;\n")),
                    condition => out.push_str(&format!(
                        "{indent}if ({condition})\n{indent}{{\n{indent}    wireKeyCount++;\n{indent}}}\n"
                    )),
                }
            }
            checks::property_count_checks(out, "wireKeyCount", "path", model.schema, indent);
        }
        checks::dependent_required_checks(out, model.schema, "path", indent, &|key| {
            fields
                .iter()
                .find(|field| field.json_name == key)
                .map(present)
                .unwrap_or_else(|| "false".to_string())
        });
    }
    out.push_str("        }\n\n");
}

/// The member-level predicates over an in-memory member value `member`.
fn member_checks(
    out: &mut String,
    ty: &CsType,
    schema: &Schema,
    value: &str,
    path: &str,
    site: RegexSite,
    indent: &str,
) {
    representability_checks(out, ty, Some(schema), value, path, indent, 0);
    value_checks(out, ty, schema, value, path, site, indent, 0);
}

fn render_field_validate(out: &mut String, field: &FieldPlan, indent: &str) {
    let rt = super::runtime_prefix();
    let path = member_path(&field.json_name);
    let inner = format!("{indent}    ");
    let mut body = String::new();
    if !matches!(field.ty, CsType::ClosedValue { .. }) {
        let mut schema = field.schema.clone();
        if !matches!(field.ty, CsType::List(..)) {
            schema.items = None;
        }
        member_checks(
            &mut body,
            &field.ty,
            &schema,
            "member",
            &path,
            RegexSite::Static(&field.name),
            &inner,
        );
    }
    if field.always_present_value() {
        if !body.is_empty() {
            out.push_str(&format!(
                "{indent}{{\n{inner}var member = value.{};\n{body}{indent}}}\n",
                field.name
            ));
        }
        return;
    }
    let required_violation = field.required && !field.nullable;
    if body.is_empty() && !required_violation {
        return;
    }
    if body.is_empty() {
        out.push_str(&format!(
            "{indent}if (value.{} is null)\n{indent}{{\n{indent}    violations.Add(new {rt}Violation({path}, \"required\"));\n{indent}}}\n",
            field.name
        ));
        return;
    }
    out.push_str(&format!(
        "{indent}{{\n{inner}if (value.{} is {{ }} member)\n{inner}{{\n",
        field.name
    ));
    let nested = format!("{inner}    ");
    out.push_str(&reindent(&body, &inner, &nested));
    out.push_str(&format!("{inner}}}\n"));
    if required_violation {
        out.push_str(&format!(
            "{inner}else\n{inner}{{\n{inner}    violations.Add(new {rt}Violation({path}, \"required\"));\n{inner}}}\n"
        ));
    }
    out.push_str(&format!("{indent}}}\n"));
}

/// Re-indents generated lines from `from` to `to`.
fn reindent(body: &str, from: &str, to: &str) -> String {
    body.lines()
        .map(|line| match line.strip_prefix(from) {
            Some(rest) => format!("{to}{rest}\n"),
            None => format!("{line}\n"),
        })
        .collect()
}

/// The checks one typed map entry (`entry.Key` / `entry.Value`) runs before it
/// is written.
fn render_map_entry_validate(
    out: &mut String,
    additional: &TypedAdditionalPlan,
    position: &str,
    indent: &str,
) {
    let rt = super::runtime_prefix();
    let path = &format!("{rt}Violation.Member(path, entry.Key)");
    out.push_str(&format!(
        "{indent}if (entry.Value is {{ }} member)\n{indent}{{\n"
    ));
    let mut schema = additional.schema.clone();
    if !matches!(additional.ty, CsType::List(..)) {
        schema.items = None;
    }
    member_checks(
        out,
        &additional.ty,
        &schema,
        "member",
        path,
        RegexSite::Static(position),
        &format!("{indent}    "),
    );
    out.push_str(&format!("{indent}}}\n"));
    if !additional.nullable && !additional.ty.is_value_type() {
        out.push_str(&format!(
            "{indent}else\n{indent}{{\n{indent}    violations.Add(new {rt}Violation({path}, \"explicit null not allowed\"));\n{indent}}}\n"
        ));
    }
}

fn render_object_write(
    out: &mut String,
    name: &str,
    open: bool,
    fields: &[FieldPlan],
    typed_additional: Option<&TypedAdditionalPlan>,
) {
    let indent = "            ";
    out.push_str(&format!(
        "        internal static void WriteValue({UTF8_JSON_WRITER} writer, {name} value)\n        {{\n{indent}writer.WriteStartObject();\n"
    ));
    for field in fields {
        let property = format!("writer.WritePropertyName({});", cs_string(&field.json_name));
        if field.always_present_value() {
            out.push_str(&format!("{indent}{property}\n"));
            write_value(
                out,
                &field.ty,
                &format!("value.{}", field.name),
                field.nullable_items,
                indent,
                0,
            );
            continue;
        }
        out.push_str(&format!(
            "{indent}if (value.{} is {{ }} {})\n{indent}{{\n{indent}    {property}\n",
            field.name,
            local_member(field)
        ));
        write_value(
            out,
            &field.ty,
            &local_member(field),
            field.nullable_items,
            &format!("{indent}    "),
            0,
        );
        out.push_str(&format!("{indent}}}\n"));
        if field.required && field.nullable {
            out.push_str(&format!(
                "{indent}else\n{indent}{{\n{indent}    {property}\n{indent}    writer.WriteNullValue();\n{indent}}}\n"
            ));
        }
    }
    if open {
        out.push_str(&format!(
            "{indent}foreach (var entry in value.AdditionalProperties)\n{indent}{{\n{indent}    writer.WritePropertyName(entry.Key);\n"
        ));
        match typed_additional {
            Some(additional) => write_map_entry(out, additional, &format!("{indent}    ")),
            None => out.push_str(&format!("{indent}    entry.Value.WriteTo(writer);\n")),
        }
        out.push_str(&format!("{indent}}}\n"));
    }
    out.push_str(&format!("{indent}writer.WriteEndObject();\n        }}\n"));
}

fn local_member(field: &FieldPlan) -> String {
    format!("member{}", field.name)
}

fn write_map_entry(out: &mut String, additional: &TypedAdditionalPlan, indent: &str) {
    if additional.nullable || !additional.ty.is_value_type() {
        out.push_str(&format!(
            "{indent}if (entry.Value is {{ }} entryValue)\n{indent}{{\n"
        ));
        write_value(
            out,
            &additional.ty,
            "entryValue",
            false,
            &format!("{indent}    "),
            0,
        );
        out.push_str(&format!(
            "{indent}}}\n{indent}else\n{indent}{{\n{indent}    writer.WriteNullValue();\n{indent}}}\n"
        ));
    } else {
        write_value(out, &additional.ty, "entry.Value", false, indent, 0);
    }
}

/// Writes one already-validated value (no property name).
pub(super) fn write_value(
    out: &mut String,
    ty: &CsType,
    value: &str,
    nullable_elements: bool,
    indent: &str,
    depth: usize,
) {
    match ty {
        CsType::String => out.push_str(&format!("{indent}writer.WriteStringValue({value});\n")),
        CsType::Long | CsType::Double => {
            out.push_str(&format!("{indent}writer.WriteNumberValue({value});\n"))
        }
        CsType::Boolean => out.push_str(&format!("{indent}writer.WriteBooleanValue({value});\n")),
        CsType::Temporal(_) | CsType::Bytes(_) => out.push_str(&format!(
            "{indent}writer.WriteStringValue({});\n",
            materialized_wire(ty, value)
        )),
        CsType::Ref { name } | CsType::Union { name } => out.push_str(&format!(
            "{indent}{name}.Converter.WriteValue(writer, {value});\n"
        )),
        CsType::ClosedValue { wire, .. } => {
            write_value(out, wire, &format!("{value}.Value"), false, indent, depth)
        }
        CsType::List(element, _) => {
            let item = format!("item{depth}");
            out.push_str(&format!(
                "{indent}writer.WriteStartArray();\n{indent}foreach (var {item} in {value})\n{indent}{{\n"
            ));
            let inner = format!("{indent}    ");
            if nullable_elements || !element.is_value_type() {
                let present = format!("present{depth}");
                out.push_str(&format!(
                    "{inner}if ({item} is {{ }} {present})\n{inner}{{\n"
                ));
                write_value(
                    out,
                    element,
                    &present,
                    false,
                    &format!("{inner}    "),
                    depth + 1,
                );
                out.push_str(&format!(
                    "{inner}}}\n{inner}else\n{inner}{{\n{inner}    writer.WriteNullValue();\n{inner}}}\n"
                ));
            } else {
                write_value(out, element, &item, false, &inner, depth + 1);
            }
            out.push_str(&format!("{indent}}}\n{indent}writer.WriteEndArray();\n"));
        }
    }
}

// ---------------------------------------------------------------------------
// Typed maps
// ---------------------------------------------------------------------------

fn render_typed_map_body(out: &mut String, model: &ObjectModel, additional: &TypedAdditionalPlan) {
    let rt = super::runtime_prefix();
    let violations_type = super::types::violations_type();
    let value_type = additional.value_type();
    out.push_str(&format!(
        "    /// <summary>\n    /// The map's members.\n    /// </summary>\n    public {READ_ONLY_DICTIONARY}<string, {value_type}> AdditionalProperties {{ get; init; }} = new {DICTIONARY}<string, {value_type}>();\n\n"
    ));
    render_equality(out, model.name, &["AdditionalProperties".to_string()]);
    out.push_str(&format!(
        "    /// <summary>\n    /// Converts <see cref=\"{name}\"/> to and from JSON, enforcing the schema in both directions.\n    /// </summary>\n    internal sealed class Converter : {JSON_CONVERTER}<{name}>\n    {{\n",
        name = model.name
    ));
    let mut statics = String::new();
    regex_statics(
        &mut statics,
        "        ",
        MAP_MEMBER_POSITION,
        &StringConstraints::from_schema(&additional.schema),
        &ArrayConstraints::from_schema(&additional.schema),
    );
    if let Some(names) = &model.schema.property_names {
        regex_statics(
            &mut statics,
            "        ",
            PROPERTY_NAME_POSITION,
            &StringConstraints::from_schema(names),
            &ArrayConstraints::default(),
        );
    }
    if !statics.is_empty() {
        out.push_str(&statics);
        out.push('\n');
    }
    render_converter_entry_points(out, model.name, false);

    // Parse.
    let indent = "            ";
    let inner = format!("{indent}    ");
    out.push_str(&format!(
        "        internal static {name}? Parse({JSON_ELEMENT} node, string path, {violations_type} violations)\n        {{\n",
        name = model.name
    ));
    out.push_str(&format!(
        "{indent}if (node.ValueKind != {JSON_VALUE_KIND}.Object)\n{indent}{{\n{indent}    violations.Add(new {rt}Violation(path, \"expected object\"));\n{indent}    return null;\n{indent}}}\n{indent}var start = violations.Count;\n{indent}var members = {rt}SpecJson.Members(node);\n{indent}var additionalProperties = new {DICTIONARY}<string, {value_type}>();\n{indent}foreach (var member in members)\n{indent}{{\n{inner}var memberPath = {rt}Violation.Member(path, member.Key);\n{inner}if (member.Value.ValueKind == {JSON_VALUE_KIND}.Null)\n{inner}{{\n"
    ));
    if additional.nullable {
        out.push_str(&format!(
            "{inner}    additionalProperties[member.Key] = null;\n{inner}    continue;\n"
        ));
    } else {
        out.push_str(&format!(
            "{inner}    violations.Add(new {rt}Violation(memberPath, \"explicit null not allowed\"));\n{inner}    continue;\n"
        ));
    }
    out.push_str(&format!("{inner}}}\n"));
    parse_into(
        out,
        &ParseTarget {
            ty: &additional.ty,
            schema: Some(&additional.schema),
            node: "member.Value",
            path: "memberPath",
            site: RegexSite::Static(MAP_MEMBER_POSITION),
            closed: None,
            kind_known: false,
        },
        &|value| format!("additionalProperties[member.Key] = {value};"),
        &inner,
        0,
    );
    out.push_str(&format!("{indent}}}\n"));
    checks::property_count_checks(out, "members.Count", "path", model.schema, indent);
    if let Some(names) = &model.schema.property_names {
        checks::property_name_checks(out, "members.Keys", "path", names, indent);
    }
    out.push_str(&format!(
        "{indent}if (violations.Count > start)\n{indent}{{\n{indent}    return null;\n{indent}}}\n{indent}return new {name} {{ AdditionalProperties = additionalProperties }};\n        }}\n\n",
        name = model.name
    ));

    // Validate.
    out.push_str(&format!(
        "        internal static void Validate({name} value, string path, {violations_type} violations)\n        {{\n{indent}if (value.AdditionalProperties is null)\n{indent}{{\n{indent}    violations.Add(new {rt}Violation(path, \"expected object\"));\n{indent}    return;\n{indent}}}\n",
        name = model.name
    ));
    checks::property_count_checks(
        out,
        "value.AdditionalProperties.Count",
        "path",
        model.schema,
        indent,
    );
    if let Some(names) = &model.schema.property_names {
        checks::property_name_checks(
            out,
            "value.AdditionalProperties.Keys",
            "path",
            names,
            indent,
        );
    }
    out.push_str(&format!(
        "{indent}foreach (var entry in value.AdditionalProperties)\n{indent}{{\n"
    ));
    render_map_entry_validate(out, additional, MAP_MEMBER_POSITION, &inner);
    out.push_str(&format!("{indent}}}\n        }}\n\n"));

    // Write.
    out.push_str(&format!(
        "        internal static void WriteValue({UTF8_JSON_WRITER} writer, {name} value)\n        {{\n{indent}writer.WriteStartObject();\n{indent}foreach (var entry in value.AdditionalProperties)\n{indent}{{\n{inner}writer.WritePropertyName(entry.Key);\n",
        name = model.name
    ));
    write_map_entry(out, additional, &inner);
    out.push_str(&format!(
        "{indent}}}\n{indent}writer.WriteEndObject();\n        }}\n    }}\n"
    ));
}

// ---------------------------------------------------------------------------
// Closed value sets
// ---------------------------------------------------------------------------

fn closed_const_type(wire: &CsType) -> &'static str {
    match wire {
        CsType::Long => "long",
        CsType::Double => "double",
        CsType::Boolean => "bool",
        _ => "string",
    }
}

/// A nested closed value class: one constant per member and a private
/// constructor, so a value outside the set cannot be constructed (P13.1).
fn render_closed_value_class(
    out: &mut String,
    name: &str,
    wire: &CsType,
    values: &[Value],
    overrides: &ClosedNameOverrides,
) {
    let value_type = closed_const_type(wire);
    out.push_str(&format!(
        "    /// <summary>\n    /// The closed set of values this member admits.\n    /// </summary>\n    {GENERATED_CODE_ATTRIBUTE}\n    public sealed class {name} : global::System.IEquatable<{name}>\n    {{\n"
    ));
    for value in values {
        out.push_str(&format!(
            "        /// <summary>The value <c>{}</c>.</summary>\n        public static readonly {name} {} = new({});\n",
            xml_escape(&serde_json::to_string(value).unwrap_or_default()),
            closed_constant_name(overrides, value),
            scalar_literal(value, wire)
        ));
    }
    out.push_str(&format!(
        "\n        private {name}({value_type} value)\n        {{\n            this.Value = value;\n        }}\n\n        /// <summary>\n        /// The wire value.\n        /// </summary>\n        public {value_type} Value {{ get; }}\n\n"
    ));
    let equals = if value_type == "string" {
        "string.Equals(this.Value, other.Value, global::System.StringComparison.Ordinal)"
    } else {
        "this.Value == other.Value"
    };
    out.push_str(&format!(
        "        /// <inheritdoc/>\n        public bool Equals({name}? other) => other is not null && {equals};\n\n        /// <inheritdoc/>\n        public override bool Equals(object? obj) => Equals(obj as {name});\n\n        /// <inheritdoc/>\n        public override int GetHashCode() => this.Value.GetHashCode();\n\n        /// <inheritdoc/>\n        public override string ToString() => this.Value.ToString()!;\n    }}\n"
    ));
}

fn default_expr(field: &FieldPlan, value: &Value) -> Option<String> {
    let rt = super::runtime_prefix();
    match (&field.ty, value) {
        (CsType::Long, Value::Number(number)) => Some(checks::bound_literal(number, true)),
        (CsType::Double, Value::Number(number)) => Some(checks::bound_literal(number, false)),
        (CsType::Boolean, Value::Bool(flag)) => Some(flag.to_string()),
        (CsType::String, Value::String(text)) => Some(cs_string(text)),
        (CsType::ClosedValue { name, .. }, _) => Some(format!(
            "{name}.{}",
            closed_constant_name(&field.closed_overrides, value)
        )),
        (CsType::Temporal(kind), Value::String(text)) => Some(match kind {
            TemporalKind::Time => cs_string(&text.to_ascii_uppercase()),
            _ => format!(
                "{rt}TemporalSupport.{}Literal({})",
                temporal_parse_fn(*kind),
                cs_string(text)
            ),
        }),
        (CsType::Bytes(encoding), Value::String(text)) => Some(format!(
            "{rt}Base64Support.{}Literal({})",
            content_encoding_parse_fn(*encoding),
            cs_string(text)
        )),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Unions
// ---------------------------------------------------------------------------

/// Renders a union interface (named, or nested in its declaring model at
/// `indent`) with its wrapper classes and converter.
///
/// A named union nests its wrappers in the interface (`Foo.FooString`). An
/// inline union's wrappers are siblings of the interface in the declaring model
/// (`Showcase.DetailUnionString` beside `Showcase.DetailUnion`), as in Java, so
/// call sites do not repeat the interface name.
pub(super) fn render_union_interface(
    out: &mut String,
    union: &CsUnion,
    indent: &str,
    doc: Option<(&Schema, &str)>,
    wrappers_inside: bool,
) {
    let inner = format!("{indent}    ");
    if let Some((schema, name)) = doc {
        doc_comment(
            out,
            indent,
            super::schema::compose_doc(schema.title.as_deref(), schema.description.as_deref())
                .as_deref(),
            &format!(
                "The {name} union: one of {}.",
                xml_escape(&union.admissible())
            ),
            (schema.deprecated == Some(true)).then_some("type"),
        );
    } else {
        out.push_str(&format!(
            "{indent}/// <summary>\n{indent}/// One of: {}.\n{indent}/// </summary>\n",
            xml_escape(&union.admissible())
        ));
    }
    out.push_str(&format!("{indent}{GENERATED_CODE_ATTRIBUTE}\n"));
    out.push_str(&format!(
        "{indent}[{JSON_CONVERTER}(typeof({}.Converter))]\n{indent}public interface {}\n{indent}{{\n",
        union.interface, union.interface
    ));
    if wrappers_inside {
        for variant in wrapped_variants(union) {
            render_union_wrapper(out, union, variant, &inner);
            out.push('\n');
        }
    }
    let mut converter = String::new();
    render_union_converter(&mut converter, union);
    for line in converter.lines() {
        if line.is_empty() {
            out.push('\n');
        } else {
            out.push_str(&format!("{inner}{line}\n"));
        }
    }
    out.push_str(&format!("{indent}}}\n"));
    if !wrappers_inside {
        for variant in wrapped_variants(union) {
            out.push('\n');
            render_union_wrapper(out, union, variant, indent);
        }
    }
}

/// The variants a union wraps in a class of its own: every non-model branch.
fn wrapped_variants(union: &CsUnion) -> impl Iterator<Item = &UnionVariant> {
    union
        .variants
        .iter()
        .filter(|variant| variant.underlying.is_some() || variant.owned_object)
}

fn wrapper_value_type(variant: &UnionVariant) -> String {
    match &variant.underlying {
        Some(CsType::List(element, _)) => format!(
            "{}<{}>",
            super::types::READ_ONLY_LIST,
            element.element_name(variant.nullable_items)
        ),
        Some(other) => other.name(),
        None => format!("{READ_ONLY_DICTIONARY}<string, {JSON_ELEMENT}>"),
    }
}

/// The predicates a wrapper runs over the value it holds.
fn wrapper_checks(variant: &UnionVariant, indent: &str) -> Option<String> {
    let ty = variant.underlying.as_ref()?;
    let mut body = String::new();
    let mut schema = variant.schema.clone();
    if !matches!(ty, CsType::List(..)) {
        schema.items = None;
    }
    representability_checks(&mut body, ty, Some(&schema), "Value", "path", indent, 0);
    value_checks(
        &mut body,
        ty,
        &schema,
        "Value",
        "path",
        RegexSite::Static(MAP_MEMBER_POSITION),
        indent,
        0,
    );
    (!body.is_empty()).then_some(body)
}

fn render_union_wrapper(out: &mut String, union: &CsUnion, variant: &UnionVariant, indent: &str) {
    let rt = super::runtime_prefix();
    let violations_type = super::types::violations_type();
    let class = &variant.class;
    let value_type = wrapper_value_type(variant);
    out.push_str(&format!(
        "{indent}/// <summary>\n{indent}/// The {} member of <see cref=\"{}\"/>.\n{indent}/// </summary>\n{indent}{GENERATED_CODE_ATTRIBUTE}\n{indent}public sealed class {class} : {}\n{indent}{{\n",
        variant.label, union.interface, union.interface
    ));
    let inner = format!("{indent}    ");
    let mut statics = String::new();
    regex_statics(
        &mut statics,
        &inner,
        MAP_MEMBER_POSITION,
        &StringConstraints::from_schema(&variant.schema),
        &ArrayConstraints::from_schema(&variant.schema),
    );
    if !statics.is_empty() {
        out.push_str(&statics);
        out.push('\n');
    }
    out.push_str(&format!(
        "{inner}/// <summary>\n{inner}/// Wraps a member value.\n{inner}/// </summary>\n{inner}public {class}({value_type} value)\n{inner}{{\n{inner}    this.Value = value;\n{inner}}}\n\n{inner}/// <summary>\n{inner}/// The member value.\n{inner}/// </summary>\n{inner}public {value_type} Value {{ get; }}\n\n"
    ));
    if let Some(checks) = wrapper_checks(variant, &format!("{inner}    ")) {
        out.push_str(&format!(
            "{inner}internal void Validate(string path, {violations_type} violations)\n{inner}{{\n{checks}{inner}}}\n\n"
        ));
    }
    out.push_str(&format!(
        "{inner}/// <inheritdoc/>\n{inner}public override bool Equals(object? obj) => obj is {class} other && {rt}ValueEquality.Equal(this.Value, other.Value);\n\n{inner}/// <inheritdoc/>\n{inner}public override int GetHashCode() => {rt}ValueEquality.Hash(this.Value);\n\n{inner}/// <inheritdoc/>\n{inner}public override string ToString() => \"{class}[\" + this.Value + \"]\";\n{indent}}}\n"
    ));
}

fn json_equals_literal(node: &str, value: &Value) -> String {
    let rt = super::runtime_prefix();
    match value {
        Value::String(text) => format!(
            "{node}.ValueKind == {JSON_VALUE_KIND}.String && {node}.GetString() == {}",
            cs_string(text)
        ),
        Value::Bool(true) => format!("{node}.ValueKind == {JSON_VALUE_KIND}.True"),
        Value::Bool(false) => format!("{node}.ValueKind == {JSON_VALUE_KIND}.False"),
        Value::Number(number) => format!(
            "{node}.ValueKind == {JSON_VALUE_KIND}.Number && {rt}SpecJson.DoubleOf({node}) == {}",
            checks::bound_literal(number, false)
        ),
        Value::Null => format!("{node}.ValueKind == {JSON_VALUE_KIND}.Null"),
        _ => "false".to_string(),
    }
}

fn render_union_converter(out: &mut String, union: &CsUnion) {
    let rt = super::runtime_prefix();
    let violations_type = super::types::violations_type();
    let name = &union.interface;
    out.push_str(&format!(
        "/// <summary>\n/// Converts <see cref=\"{name}\"/> to and from JSON, enforcing the schema in both directions.\n/// </summary>\ninternal sealed class Converter : {JSON_CONVERTER}<{name}>\n{{\n"
    ));
    let mut entry = String::new();
    render_converter_entry_points(&mut entry, name, union.nullable);
    for line in entry.lines() {
        out.push_str(line.strip_prefix("    ").unwrap_or(line));
        out.push('\n');
    }

    // Parse: select the branch by JSON value kind (and discriminant).
    let indent = "        ";
    out.push_str(&format!(
        "    internal static {name}? Parse({JSON_ELEMENT} node, string path, {violations_type} violations)\n    {{\n"
    ));
    let objects: Vec<&UnionVariant> = union
        .variants
        .iter()
        .filter(|variant| variant.is_object)
        .collect();
    if !objects.is_empty() {
        out.push_str(&format!(
            "{indent}if (node.ValueKind == {JSON_VALUE_KIND}.Object)\n{indent}{{\n"
        ));
        let inner = format!("{indent}    ");
        if let Some(discriminant) = &union.discriminant {
            out.push_str(&format!(
                "{inner}if (!node.TryGetProperty({}, out var discriminant) || discriminant.ValueKind is {JSON_VALUE_KIND}.Null or {JSON_VALUE_KIND}.Object or {JSON_VALUE_KIND}.Array)\n{inner}{{\n{inner}    violations.Add(new {rt}Violation(path, {}));\n{inner}    return null;\n{inner}}}\n",
                cs_string(discriminant),
                cs_string(&format!("discriminator {discriminant:?} is required"))
            ));
            let mut known = Vec::new();
            for variant in &objects {
                let Some(value) = &variant.discriminant_value else {
                    continue;
                };
                known.push(
                    value
                        .as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| value.to_string()),
                );
                out.push_str(&format!(
                    "{inner}if ({})\n{inner}{{\n",
                    json_equals_literal("discriminant", value)
                ));
                render_union_read_object(out, variant, &format!("{inner}    "));
                out.push_str(&format!("{inner}}}\n"));
            }
            out.push_str(&format!(
                "{inner}violations.Add(new {rt}Violation(path, {} + discriminant.GetRawText() + {}));\n{inner}return null;\n",
                cs_string(&format!("unknown discriminator {discriminant} ")),
                cs_string(&format!(": expected one of [{}]", known.join(", ")))
            ));
        } else {
            render_union_read_object(out, objects[0], &inner);
        }
        out.push_str(&format!("{indent}}}\n"));
    }
    for variant in union.variants.iter().filter(|variant| !variant.is_object) {
        let condition = match variant.kind_test {
            "Boolean" => {
                format!("node.ValueKind is {JSON_VALUE_KIND}.True or {JSON_VALUE_KIND}.False")
            }
            kind => format!("node.ValueKind == {JSON_VALUE_KIND}.{kind}"),
        };
        out.push_str(&format!("{indent}if ({condition})\n{indent}{{\n"));
        render_union_read_wrapper(out, variant, &format!("{indent}    "));
        out.push_str(&format!("{indent}}}\n"));
    }
    if union.nullable {
        out.push_str(&format!(
            "{indent}if (node.ValueKind == {JSON_VALUE_KIND}.Null)\n{indent}{{\n{indent}    return null;\n{indent}}}\n"
        ));
    }
    out.push_str(&format!(
        "{indent}violations.Add(new {rt}Violation(path, {}));\n{indent}return null;\n    }}\n\n",
        cs_string(&format!("expected one of: {}", union.admissible()))
    ));

    // Validate: dispatch on the runtime member type.
    out.push_str(&format!(
        "    internal static void Validate({name} value, string path, {violations_type} violations)\n    {{\n{indent}switch (value)\n{indent}{{\n"
    ));
    for (index, variant) in union.variants.iter().enumerate() {
        let binding = format!("variant{index}");
        out.push_str(&format!("{indent}    case {} {binding}:\n", variant.class));
        if variant.owned_object {
            // A free-form object holds verbatim members; nothing to check.
        } else if variant.underlying.is_none() {
            out.push_str(&format!(
                "{indent}        {}.Converter.Validate({binding}, path, violations);\n",
                variant.class
            ));
        } else if wrapper_checks(variant, "").is_some() {
            out.push_str(&format!(
                "{indent}        {binding}.Validate(path, violations);\n"
            ));
        }
        out.push_str(&format!("{indent}        break;\n"));
    }
    out.push_str(&format!(
        "{indent}    default:\n{indent}        violations.Add(new {rt}Violation(path, {}));\n{indent}        break;\n{indent}}}\n    }}\n\n",
        cs_string(&format!("expected one of: {}", union.admissible()))
    ));

    // WriteValue.
    out.push_str(&format!(
        "    internal static void WriteValue({UTF8_JSON_WRITER} writer, {name} value)\n    {{\n{indent}switch (value)\n{indent}{{\n"
    ));
    for (index, variant) in union.variants.iter().enumerate() {
        let binding = format!("variant{index}");
        out.push_str(&format!("{indent}    case {} {binding}:\n", variant.class));
        let inner = format!("{indent}        ");
        match &variant.underlying {
            None if variant.owned_object => out.push_str(&format!(
                "{inner}writer.WriteStartObject();\n{inner}foreach (var entry in {binding}.Value)\n{inner}{{\n{inner}    writer.WritePropertyName(entry.Key);\n{inner}    entry.Value.WriteTo(writer);\n{inner}}}\n{inner}writer.WriteEndObject();\n"
            )),
            None => out.push_str(&format!(
                "{inner}{}.Converter.WriteValue(writer, {binding});\n",
                variant.class
            )),
            Some(ty) => write_value(
                out,
                ty,
                &format!("{binding}.Value"),
                variant.nullable_items,
                &inner,
                0,
            ),
        }
        out.push_str(&format!("{inner}break;\n"));
    }
    out.push_str(&format!("{indent}}}\n    }}\n}}\n"));
}

fn render_union_read_object(out: &mut String, variant: &UnionVariant, indent: &str) {
    let rt = super::runtime_prefix();
    if variant.owned_object {
        out.push_str(&format!(
            "{indent}return new {}({rt}SpecJson.Members(node));\n",
            variant.class
        ));
    } else {
        out.push_str(&format!(
            "{indent}return {}.Converter.Parse(node, path, violations);\n",
            variant.class
        ));
    }
}

fn render_union_read_wrapper(out: &mut String, variant: &UnionVariant, indent: &str) {
    let Some(ty) = &variant.underlying else {
        return;
    };
    let checked = wrapper_checks(variant, "").is_some();
    let class = &variant.class;
    let mut schema = variant.schema.clone();
    if !matches!(ty, CsType::List(..)) {
        // Branch constraints run through the wrapper's own `Validate`.
        schema = Schema {
            items: schema.items.clone(),
            ..Schema::default()
        };
    }
    let start = "wrapperStart";
    out.push_str(&format!("{indent}var {start} = violations.Count;\n"));
    out.push_str(&format!("{indent}{class}? wrapped = null;\n"));
    let mut list_schema = variant.schema.clone();
    let target_schema = if matches!(ty, CsType::List(..)) {
        // The array wrapper's own constraints run over the wire array here;
        // element constraints run per element.
        list_schema.items = variant.schema.items.clone();
        Some(&list_schema)
    } else {
        Some(&schema)
    };
    parse_into(
        out,
        &ParseTarget {
            ty,
            schema: target_schema,
            node: "node",
            path: "path",
            site: RegexSite::Inline,
            closed: None,
            kind_known: true,
        },
        &|value| format!("wrapped = new {class}({value});"),
        indent,
        0,
    );
    if checked && !matches!(ty, CsType::List(..)) {
        out.push_str(&format!("{indent}wrapped?.Validate(path, violations);\n"));
    }
    out.push_str(&format!(
        "{indent}return violations.Count > {start} ? null : wrapped;\n"
    ));
}
