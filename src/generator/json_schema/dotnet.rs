use std::collections::BTreeSet;
use std::path::PathBuf;

use heck::ToUpperCamelCase;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::Value;

use crate::error::{Error, Result};
use crate::generator::ExternalModelBackend;
use crate::generator::dotnet::{
    WireValueConversion, csharp_parameter_name, csharp_string_literal, csharp_type_name,
};
use crate::planning::{PlannedFamily, PlannedJsonType, PlannedSpec};
use crate::spec::RecordSpec;

const GENERATED_CODE_ATTRIBUTE: &str = "[GeneratedCode(\"nexgen\", null)]";

#[derive(Debug, Deserialize, Default)]
struct Schema {
    #[serde(rename = "$ref")]
    reference: Option<String>,
    #[serde(rename = "type")]
    ty: Option<Value>,
    description: Option<String>,
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
}

#[derive(Debug, Default)]
pub(in crate::generator) struct ModelBackend {
    json_models: Vec<PlannedJsonType>,
}

#[derive(Debug, Default)]
pub(in crate::generator) struct RenderedModelFragments {
    pub(in crate::generator) body: String,
}

impl RenderedModelFragments {
    pub(in crate::generator) fn has_models(&self) -> bool {
        !self.body.is_empty()
    }
}

impl ExternalModelBackend<PlannedJsonType> for ModelBackend {
    type ModelFragments = RenderedModelFragments;
    type WireConversion = WireValueConversion;

    fn prepare(&mut self, api_plan: &PlannedSpec) -> Result<()> {
        self.json_models = api_plan
            .external_types()
            .map(|(_, binding)| binding)
            .filter_map(|binding| binding.json_model().cloned())
            .collect();
        Ok(())
    }

    fn render_models(&self) -> Result<RenderedModelFragments> {
        render_external_models(&self.json_models)
    }

    fn model_type_annotation(&self, json_type: &PlannedJsonType) -> Option<String> {
        Some(model_type_ref(json_type))
    }

    fn wire_conversion(
        &self,
        json_type: &PlannedJsonType,
        _planned_record: Option<&RecordSpec<PlannedFamily>>,
    ) -> Option<WireValueConversion> {
        Some(WireValueConversion {
            annotation: model_type_ref(json_type),
            to_wire: "{value}".to_string(),
        })
    }
}

fn model_type_ref(json_type: &PlannedJsonType) -> String {
    csharp_type_name(&json_type.model_name)
}

fn render_external_models(json_models: &[PlannedJsonType]) -> Result<RenderedModelFragments> {
    if json_models.is_empty() {
        return Ok(RenderedModelFragments::default());
    }

    let mut output = String::new();
    for (index, model) in json_models.iter().enumerate() {
        if index > 0 {
            output.push('\n');
        }
        render_model(&mut output, model)?;
    }
    Ok(RenderedModelFragments { body: output })
}

fn render_model(output: &mut String, model: &PlannedJsonType) -> Result<()> {
    let schema = decode_schema(model)?;
    render_xml_summary(output, "", schema.description.as_deref());
    if !model_needs_extension_data(&schema)? && !is_open_object(&schema) {
        output.push_str("[JsonUnmappedMemberHandling(JsonUnmappedMemberHandling.Disallow)]\n");
    }
    output.push_str(GENERATED_CODE_ATTRIBUTE);
    output.push('\n');
    output.push_str("public class ");
    output.push_str(&model_type_ref(model));
    if model_needs_on_deserialized(&schema)? {
        output.push_str(" : IJsonOnDeserialized");
    }
    output.push_str("\n{\n");

    if typed_map_value_schema(&schema)?.is_none() {
        render_model_constructor(output, model, &schema)?;
        render_model_properties(output, &schema)?;
    }
    render_extension_data_property(output, &schema)?;
    render_model_validation(output, &schema)?;
    render_optional_helpers(output, &schema)?;

    output.push_str("}\n\n");
    Ok(())
}

fn render_model_constructor(
    output: &mut String,
    model: &PlannedJsonType,
    schema: &Schema,
) -> Result<()> {
    let required = required_fields(schema);
    let required_properties = schema
        .properties
        .as_ref()
        .map(|properties| {
            properties
                .iter()
                .filter(|(name, property)| {
                    required.contains(name.as_str()) && property.const_value.is_none()
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if required_properties.is_empty() {
        return Ok(());
    }

    output.push_str("    public ");
    output.push_str(&model_type_ref(model));
    output.push('(');
    for (index, (json_name, property)) in required_properties.iter().enumerate() {
        if index > 0 {
            output.push_str(", ");
        }
        output.push_str(&schema_type(property, false)?);
        output.push(' ');
        output.push_str(&csharp_parameter_name(json_name));
    }
    output.push_str(")\n    {\n");
    for (json_name, _) in required_properties {
        output.push_str("        ");
        output.push_str(&csharp_type_name(json_name));
        output.push_str(" = ");
        output.push_str(&csharp_parameter_name(json_name));
        output.push_str(";\n");
    }
    output.push_str("    }\n\n");
    Ok(())
}

fn render_model_properties(output: &mut String, schema: &Schema) -> Result<()> {
    let required = required_fields(schema);
    let Some(properties) = &schema.properties else {
        return Ok(());
    };
    for (json_name, property) in properties {
        render_xml_summary(output, "    ", property.description.as_deref());
        if !required.contains(json_name.as_str()) {
            render_optional_property(output, json_name, property)?;
            continue;
        }
        if property.const_value.is_some() {
            render_const_property(output, json_name, property)?;
            continue;
        }
        output.push_str("    [JsonPropertyName(");
        output.push_str(&csharp_string_literal(json_name));
        output.push_str(")]\n");
        output.push_str("    [JsonRequired]\n");
        output.push_str("    public ");
        output.push_str(&schema_type(property, false)?);
        output.push(' ');
        output.push_str(&csharp_type_name(json_name));
        output.push_str(" { get; init; }\n");
    }
    Ok(())
}

fn render_optional_property(output: &mut String, json_name: &str, property: &Schema) -> Result<()> {
    let property_type = schema_type(property, true)?;
    output.push_str("    [JsonIgnore]\n");
    output.push_str("    public ");
    output.push_str(&property_type);
    output.push(' ');
    output.push_str(&csharp_type_name(json_name));
    output.push_str("\n    {\n");
    output.push_str("        get => ReadOptionalValue<");
    output.push_str(&optional_read_type(property, &property_type)?);
    output.push_str(">(");
    output.push_str(&csharp_string_literal(json_name));
    if let Some(default_value) = property.default.as_ref().and_then(csharp_value_literal) {
        output.push_str(", ");
        output.push_str(&default_value);
    }
    output.push_str(");\n");
    output.push_str("        init\n        {\n");
    if !allows_null(property) {
        output.push_str("            RejectNull(");
        output.push_str(&csharp_string_literal(json_name));
        output.push_str(", value);\n");
    }
    output.push_str("            AdditionalProperties[");
    output.push_str(&csharp_string_literal(json_name));
    output.push_str("] = value;\n");
    output.push_str("        }\n");
    output.push_str("    }\n");
    Ok(())
}

fn render_const_property(output: &mut String, json_name: &str, property: &Schema) -> Result<()> {
    let const_value = property
        .const_value
        .as_ref()
        .and_then(csharp_value_literal)
        .expect("const property should have C# literal");
    let field_name = csharp_parameter_name(&format!("{json_name}-value"));
    output.push_str("    private ");
    output.push_str(&schema_type(property, false)?);
    output.push(' ');
    output.push_str(&field_name);
    output.push_str(" = ");
    output.push_str(&const_value);
    output.push_str(";\n\n");
    output.push_str("    [JsonPropertyName(");
    output.push_str(&csharp_string_literal(json_name));
    output.push_str(")]\n");
    output.push_str("    [JsonRequired]\n");
    output.push_str("    public ");
    output.push_str(&schema_type(property, false)?);
    output.push(' ');
    output.push_str(&csharp_type_name(json_name));
    output.push_str("\n    {\n");
    output.push_str("        get => ");
    output.push_str(&field_name);
    output.push_str(";\n");
    output.push_str("        init\n        {\n");
    output.push_str("            if (value != ");
    output.push_str(&const_value);
    output.push_str(")\n            {\n");
    output.push_str("                throw new JsonException(");
    output.push_str(&csharp_string_literal(&format!(
        "{json_name} must equal {const_value}"
    )));
    output.push_str(");\n");
    output.push_str("            }\n");
    output.push_str("            ");
    output.push_str(&field_name);
    output.push_str(" = value;\n");
    output.push_str("        }\n");
    output.push_str("    }\n");
    Ok(())
}

fn optional_read_type(schema: &Schema, property_type: &str) -> Result<String> {
    if matches!(schema.ty.as_ref().and_then(Value::as_str), Some("array")) {
        let item = schema
            .items
            .as_ref()
            .map(|item| schema_base_type(item))
            .transpose()?
            .unwrap_or_else(|| "object".to_string());
        Ok(format!("List<{item}>?"))
    } else {
        Ok(property_type.to_string())
    }
}

fn render_extension_data_property(output: &mut String, schema: &Schema) -> Result<()> {
    if !model_needs_extension_data(schema)? {
        return Ok(());
    }
    if schema
        .properties
        .as_ref()
        .is_some_and(|properties| !properties.is_empty())
    {
        output.push('\n');
    }
    output.push_str("    [JsonExtensionData]\n");
    output.push_str("    public Dictionary<string, object?> AdditionalProperties { get; set; } = new Dictionary<string, object?>();\n");
    Ok(())
}

fn render_model_validation(output: &mut String, schema: &Schema) -> Result<()> {
    if !model_needs_on_deserialized(schema)? {
        return Ok(());
    }
    output.push('\n');
    output.push_str("    void IJsonOnDeserialized.OnDeserialized()\n    {\n");
    if let Some(value_schema) = typed_map_value_schema(schema)? {
        if let Some(max_properties) = schema.max_properties {
            output.push_str("        if (AdditionalProperties.Count > ");
            output.push_str(&max_properties.to_string());
            output.push_str(")\n        {\n");
            output.push_str("            throw new JsonException(");
            output.push_str(&csharp_string_literal(&format!(
                "maxProperties: at most {max_properties} entries"
            )));
            output.push_str(");\n        }\n");
        }
        output.push_str("        foreach (var entry in AdditionalProperties)\n        {\n");
        render_extension_value_validation(output, "entry.Key", "entry.Value", &value_schema, 3)?;
        output.push_str("        }\n");
        output.push_str("    }\n");
        return Ok(());
    }

    let optional_fields = optional_fields(schema);
    if !optional_fields.is_empty() {
        if !is_open_object(schema) {
            output.push_str("        foreach (var key in AdditionalProperties.Keys)\n        {\n");
            output.push_str("            if (");
            for (index, (json_name, _)) in optional_fields.iter().enumerate() {
                if index > 0 {
                    output.push_str(" && ");
                }
                output.push_str("key != ");
                output.push_str(&csharp_string_literal(json_name));
            }
            output.push_str(")\n            {\n");
            output.push_str(
                "                throw new JsonException($\"Unknown field `{key}`.\");\n",
            );
            output.push_str("            }\n");
            output.push_str("        }\n");
        }
        for (json_name, property) in optional_fields {
            let value_name = csharp_parameter_name(&format!("{json_name}-value"));
            output.push_str("        if (AdditionalProperties.TryGetValue(");
            output.push_str(&csharp_string_literal(json_name));
            output.push_str(", out var ");
            output.push_str(&value_name);
            output.push_str("))\n        {\n");
            if !allows_null(property) {
                output.push_str("            RejectNull(");
                output.push_str(&csharp_string_literal(json_name));
                output.push_str(", ");
                output.push_str(&value_name);
                output.push_str(");\n");
            }
            render_extension_value_validation(
                output,
                &csharp_string_literal(json_name),
                &value_name,
                property,
                3,
            )?;
            output.push_str("        }\n");
        }
    }
    output.push_str("    }\n");
    Ok(())
}

fn render_extension_value_validation(
    output: &mut String,
    path_expr: &str,
    value_expr: &str,
    schema: &Schema,
    indent_level: usize,
) -> Result<()> {
    if allows_null(schema) {
        return Ok(());
    }
    let indent = "    ".repeat(indent_level);
    match schema.ty.as_ref().and_then(Value::as_str) {
        Some("string") => {
            let json_name = format!("json{indent_level}");
            output.push_str(&indent);
            output.push_str("if (");
            output.push_str(value_expr);
            output.push_str(" is JsonElement ");
            output.push_str(&json_name);
            output.push_str(" && ");
            output.push_str(&json_name);
            output.push_str(".ValueKind != JsonValueKind.String)\n");
            output.push_str(&indent);
            output.push_str("{\n");
            output.push_str(&indent);
            output.push_str("    throw new JsonException($\"{");
            output.push_str(path_expr);
            output.push_str("}: expected string\");\n");
            output.push_str(&indent);
            output.push_str("}\n");
            output.push_str(&indent);
            output.push_str("else if (");
            output.push_str(value_expr);
            output.push_str(" is not JsonElement && ");
            output.push_str(value_expr);
            output.push_str(" is not string)\n");
            output.push_str(&indent);
            output.push_str("{\n");
            output.push_str(&indent);
            output.push_str("    throw new JsonException($\"{");
            output.push_str(path_expr);
            output.push_str("}: expected string\");\n");
            output.push_str(&indent);
            output.push_str("}\n");
        }
        Some("integer") => {
            output.push_str(&indent);
            output.push_str("_ = ReadJsonValue<long?>(");
            output.push_str(value_expr);
            output.push_str(");\n");
        }
        Some("array") => {
            output.push_str(&indent);
            output.push_str("_ = ReadJsonValue<");
            output.push_str(&optional_read_type(schema, &schema_type(schema, true)?)?);
            output.push_str(">(");
            output.push_str(value_expr);
            output.push_str(");\n");
        }
        _ if schema.reference.is_some() => {
            output.push_str(&indent);
            output.push_str("_ = ReadJsonValue<");
            output.push_str(&optional_read_type(schema, &schema_type(schema, true)?)?);
            output.push_str(">(");
            output.push_str(value_expr);
            output.push_str(");\n");
        }
        _ => {}
    }
    Ok(())
}

fn render_optional_helpers(output: &mut String, schema: &Schema) -> Result<()> {
    if !model_needs_extension_data(schema)? {
        return Ok(());
    }
    output.push('\n');
    output.push_str(
        "    private T? ReadOptionalValue<T>(string name, T? defaultValue = default)\n    {\n",
    );
    output.push_str(
        "        if (!AdditionalProperties.TryGetValue(name, out var value))\n        {\n",
    );
    output.push_str("            return defaultValue;\n        }\n");
    output.push_str("        return ReadJsonValue<T>(value);\n");
    output.push_str("    }\n\n");
    output.push_str("    private static T? ReadJsonValue<T>(object? value)\n    {\n");
    output.push_str("        if (value is null)\n        {\n");
    output.push_str("            return default;\n        }\n");
    output.push_str(
        "        if (typeof(T) == typeof(long?) || typeof(T) == typeof(long))\n        {\n",
    );
    output.push_str("            return (T?)(object?)ReadJsonInteger(value);\n");
    output.push_str("        }\n");
    output.push_str("        if (value is JsonElement json)\n        {\n");
    output.push_str("            return json.Deserialize<T>();\n");
    output.push_str("        }\n");
    output.push_str("        if (value is T typed)\n        {\n");
    output.push_str("            return typed;\n        }\n");
    output.push_str("        return (T)value;\n");
    output.push_str("    }\n\n");
    output.push_str("    private static long? ReadJsonInteger(object? value)\n    {\n");
    output.push_str("        const double maxSafeInteger = 9007199254740991d;\n");
    output.push_str("        if (value is null)\n        {\n");
    output.push_str("            return default;\n        }\n");
    output.push_str("        double number;\n");
    output.push_str("        if (value is JsonElement json)\n        {\n");
    output.push_str("            if (json.ValueKind == JsonValueKind.Null)\n            {\n");
    output.push_str("                return default;\n");
    output.push_str("            }\n");
    output.push_str("            if (json.ValueKind != JsonValueKind.Number)\n            {\n");
    output.push_str("                throw new JsonException(\"expected integer\");\n");
    output.push_str("            }\n");
    output.push_str("            number = json.GetDouble();\n");
    output.push_str("        }\n");
    output.push_str("        else if (value is long longValue)\n        {\n");
    output.push_str("            number = longValue;\n");
    output.push_str("        }\n");
    output.push_str("        else if (value is int intValue)\n        {\n");
    output.push_str("            number = intValue;\n");
    output.push_str("        }\n");
    output.push_str("        else\n        {\n");
    output.push_str("            throw new JsonException(\"expected integer\");\n");
    output.push_str("        }\n");
    output.push_str("        if (double.IsNaN(number) || double.IsInfinity(number) || Math.Truncate(number) != number || Math.Abs(number) > maxSafeInteger)\n        {\n");
    output.push_str("            throw new JsonException(\"expected integer\");\n");
    output.push_str("        }\n");
    output.push_str("        return (long)number;\n");
    output.push_str("    }\n\n");
    output.push_str("    private static void RejectNull(string name, object? value)\n    {\n");
    output.push_str("        if (value is null || value is JsonElement { ValueKind: JsonValueKind.Null })\n        {\n");
    output
        .push_str("            throw new JsonException($\"{name}: explicit null not allowed\");\n");
    output.push_str("        }\n");
    output.push_str("    }\n");
    Ok(())
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

fn typed_map_value_schema(schema: &Schema) -> Result<Option<Schema>> {
    if schema
        .properties
        .as_ref()
        .is_some_and(|properties| !properties.is_empty())
    {
        return Ok(None);
    }

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

fn model_needs_extension_data(schema: &Schema) -> Result<bool> {
    Ok(typed_map_value_schema(schema)?.is_some()
        || is_open_object(schema)
        || !optional_fields(schema).is_empty())
}

fn model_needs_on_deserialized(schema: &Schema) -> Result<bool> {
    Ok(typed_map_value_schema(schema)?.is_some()
        || (!optional_fields(schema).is_empty()
            && (!is_open_object(schema)
                || optional_fields(schema)
                    .iter()
                    .any(|(_, property)| !allows_null(property)))))
}

fn optional_fields(schema: &Schema) -> Vec<(&str, &Schema)> {
    let required = required_fields(schema);
    schema
        .properties
        .as_ref()
        .map(|properties| {
            properties
                .iter()
                .filter(|(name, _)| !required.contains(name.as_str()))
                .map(|(name, property)| (name.as_str(), property))
                .collect()
        })
        .unwrap_or_default()
}

fn schema_type(schema: &Schema, optional: bool) -> Result<String> {
    let base = schema_base_type(schema)?;
    Ok(if optional { nullable_type(&base) } else { base })
}

fn schema_base_type(schema: &Schema) -> Result<String> {
    if let Some(reference) = &schema.reference {
        return Ok(reference_type_name(reference));
    }
    if let Some(one_of) = &schema.one_of {
        let non_null = one_of
            .iter()
            .filter(|branch| !schema_type_includes(branch, "null"))
            .collect::<Vec<_>>();
        if one_of
            .iter()
            .any(|branch| schema_type_includes(branch, "null"))
            && let Some(branch) = non_null.first()
        {
            return Ok(nullable_type(&schema_base_type(branch)?));
        }
        return Ok("object".to_string());
    }
    match schema.ty.as_ref().and_then(Value::as_str) {
        Some("string") => Ok("string".to_string()),
        Some("integer") => Ok("long".to_string()),
        Some("number") => Ok("double".to_string()),
        Some("boolean") => Ok("bool".to_string()),
        Some("array") => {
            let item = schema
                .items
                .as_ref()
                .map(|item| schema_base_type(item))
                .transpose()?
                .unwrap_or_else(|| "object".to_string());
            Ok(format!("IReadOnlyList<{item}>"))
        }
        Some("object") => {
            if let Some(value_schema) = typed_map_value_schema(schema)? {
                Ok(format!(
                    "IReadOnlyDictionary<string, {}>",
                    schema_base_type(&value_schema)?
                ))
            } else {
                Ok("object".to_string())
            }
        }
        Some("null") => Ok("object?".to_string()),
        _ => Ok("object".to_string()),
    }
}

fn nullable_type(base: &str) -> String {
    if base.ends_with('?') {
        base.to_string()
    } else {
        format!("{base}?")
    }
}

fn required_fields(schema: &Schema) -> BTreeSet<&str> {
    schema
        .required
        .as_deref()
        .unwrap_or_default()
        .iter()
        .map(String::as_str)
        .collect()
}

fn is_open_object(schema: &Schema) -> bool {
    !matches!(schema.additional_properties, Some(Value::Bool(false)))
}

fn allows_null(schema: &Schema) -> bool {
    schema_type_includes(schema, "null")
        || schema.one_of.as_ref().is_some_and(|branches| {
            branches
                .iter()
                .any(|branch| schema_type_includes(branch, "null"))
        })
}

fn schema_type_includes(schema: &Schema, ty: &str) -> bool {
    match &schema.ty {
        Some(Value::String(value)) => value == ty,
        Some(Value::Array(values)) => values
            .iter()
            .any(|value| value.as_str().is_some_and(|value| value == ty)),
        _ => false,
    }
}

fn reference_model_name(reference: &str) -> String {
    reference
        .rsplit('/')
        .next()
        .unwrap_or(reference)
        .rsplit('#')
        .next()
        .unwrap_or(reference)
        .replace("~1", "/")
        .replace("~0", "~")
}

fn reference_type_name(reference: &str) -> String {
    let target = reference
        .split_once('#')
        .map(|(_, fragment)| fragment)
        .unwrap_or(reference)
        .strip_prefix("/$defs/")
        .map(|name| name.replace("~1", "/").replace("~0", "~"));
    if let Some(target) = target
        && let Some((module_key, model_name)) = target.rsplit_once('#')
        && !module_key.is_empty()
    {
        let namespace = module_key
            .split('/')
            .map(|segment| csharp_type_name(&segment.to_upper_camel_case()))
            .collect::<Vec<_>>()
            .join(".");
        return format!(
            "global::Nexgen.Generated.{}.{}",
            namespace,
            csharp_type_name(model_name)
        );
    }
    csharp_type_name(&reference_model_name(reference))
}

fn csharp_value_literal(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(csharp_string_literal(value)),
        Value::Bool(value) => Some(if *value { "true" } else { "false" }.to_string()),
        Value::Number(number) => Some(number.to_string()),
        Value::Null => Some("null".to_string()),
        _ => None,
    }
}

fn render_xml_summary(output: &mut String, indent: &str, summary: Option<&str>) {
    let Some(summary) = summary else {
        return;
    };
    output.push_str(indent);
    output.push_str("/// <summary>\n");
    for line in summary.trim().lines() {
        output.push_str(indent);
        output.push_str("/// ");
        output.push_str(&xml_doc_escape(line.trim()));
        output.push('\n');
    }
    output.push_str(indent);
    output.push_str("/// </summary>\n");
}

fn xml_doc_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
