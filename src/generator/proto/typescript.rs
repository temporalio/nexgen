use heck::ToLowerCamelCase;
use prost_types::FieldDescriptorProto;

use crate::error::{Error, Result};
use crate::generator::ExternalModelBackend;
use crate::generator::typescript::{
    RenderedExternalModelFragments, RenderedModel, WireFunctionNames, WireValueConversion,
    generic_model_annotation, render_named_generic_function_start,
    typescript_authored_type_annotation, typescript_generated_field_name, typescript_ident,
};
use crate::language::Language;
use crate::planning::{
    PlannedFamily, PlannedProtoType, PlannedProtoTypeInfo, PlannedSpec, PlannedType,
    PlannedWireFieldBinding, relative_descriptor_name,
};
use crate::spec::{ExternalTypeSpec, RecordFieldVisibility, RecordSpec, TypeReplacementSpec};

#[derive(Debug, Default)]
pub(in crate::generator) struct ModelBackend;

impl ExternalModelBackend for ModelBackend {
    type ModelFragments = RenderedExternalModelFragments;
    type WireConversion = WireValueConversion;

    fn prepare(&mut self, api_plan: &PlannedSpec) -> Result<()> {
        for (_, record) in api_plan.records() {
            validate_record_conversion(record)?;
        }
        Ok(())
    }

    fn render_models(&self) -> Result<RenderedExternalModelFragments> {
        Ok(RenderedExternalModelFragments::default())
    }

    fn model_type_annotation(&self, model_type: &PlannedType) -> Option<String> {
        match model_type {
            PlannedType::External(ExternalTypeSpec::Proto(PlannedProtoType::Message(proto))) => {
                Some(message_typescript_interface_ref(&proto.proto))
            }
            PlannedType::External(ExternalTypeSpec::Proto(PlannedProtoType::Enum(enum_type))) => {
                enum_type
                    .replacement
                    .as_ref()
                    .and_then(typescript_replacement_type_name)
                    .or_else(|| Some(enum_type.name.clone()))
            }
            _ => None,
        }
    }

    fn wire_type_identifier(&self, model_type: &PlannedType) -> Option<String> {
        match model_type {
            PlannedType::External(ExternalTypeSpec::Proto(PlannedProtoType::Message(proto))) => {
                Some(proto.proto.full_name.clone())
            }
            PlannedType::External(ExternalTypeSpec::Proto(PlannedProtoType::Enum(_))) => None,
            _ => None,
        }
    }

    fn wire_conversion(
        &self,
        model_type: &PlannedType,
        planned_record: Option<&RecordSpec<PlannedFamily>>,
    ) -> Option<WireValueConversion> {
        enum_wire_conversion(model_type)
            .or_else(|| message_override_conversion(model_type))
            .or_else(|| {
                planned_record.and_then(|record| generated_wire_conversion(model_type, record))
            })
    }
}

fn validate_record_conversion(record: &RecordSpec<PlannedFamily>) -> Result<()> {
    let Some(proto) = &record.data.proto else {
        return Ok(());
    };
    for (field_name, field) in record
        .fields
        .iter()
        .filter(|(_, field)| field.visibility != RecordFieldVisibility::Omitted)
    {
        match &field.data.wire_binding {
            Some(PlannedWireFieldBinding::VariantMembers { wire_name, .. }) => {
                return Err(Error::UnsupportedProtoOneofConversion {
                    language: Language::TypeScript,
                    message: proto.full_name.clone(),
                    oneof: wire_name.clone(),
                });
            }
            Some(PlannedWireFieldBinding::Value { wire_type, .. })
                if matches!(
                    field.field_type.validation_type(),
                    PlannedType::TypeParameter(_)
                ) && is_proto_generic_carrier(wire_type) =>
            {
                return Err(Error::UnsupportedProtoGenericCarrierConversion {
                    language: Language::TypeScript,
                    message: proto.full_name.clone(),
                    field: field_name.clone(),
                });
            }
            _ => {}
        }
    }
    Ok(())
}

fn is_proto_generic_carrier(kind: &PlannedType) -> bool {
    let PlannedType::External(ExternalTypeSpec::Proto(PlannedProtoType::Message(message))) =
        kind.validation_type()
    else {
        return false;
    };
    matches!(
        message.proto.full_name.as_str(),
        "temporal.api.common.v1.Payload" | "temporal.api.common.v1.Payloads"
    )
}

impl ModelBackend {
    pub(in crate::generator) fn render_model_wire_functions(
        &self,
        output: &mut String,
        model: &RenderedModel,
        planned_record: &RecordSpec<PlannedFamily>,
    ) -> bool {
        let Some(names) = &model.wire_function_names else {
            return false;
        };
        render_model_from_proto_function(output, model, planned_record, &names.from_wire);
        output.push('\n');
        render_model_to_proto_function(output, model, planned_record, &names.to_wire);
        output.push('\n');
        render_model_transfer_type_converter(output, model, planned_record, names);
        true
    }
}

/// Render the transfer-type adapter for a WIT model backed by a protobuf
/// message. This keeps the public operation input in its generated TypeScript
/// model form while letting the payload converter transport its protobuf
/// envelope representation.
fn render_model_transfer_type_converter(
    output: &mut String,
    model: &RenderedModel,
    planned_record: &RecordSpec<PlannedFamily>,
    names: &WireFunctionNames,
) {
    let proto = planned_record
        .data
        .proto
        .as_ref()
        .expect("wire functions require a protobuf-backed record");
    let converter_name = format!("{}TransferTypeConverter", model.name.to_lower_camel_case());
    let proto_type = message_typescript_interface_ref(proto);
    output.push_str("export const ");
    output.push_str(&converter_name);
    output.push_str(" = {\n  fromTransferType(value: ");
    output.push_str(&proto_type);
    output.push_str("): ");
    output.push_str(&model.name);
    output.push_str(" {\n    return ");
    output.push_str(&names.from_wire);
    output.push_str("(value)!;\n  },\n\n  toTransferType(value: ");
    output.push_str(&model.name);
    output.push_str("): ");
    output.push_str(&proto_type);
    output.push_str(" {\n    return ");
    output.push_str(&names.to_wire);
    output.push_str("(value) ?? {};\n  },\n};");
}

pub(crate) fn message_typescript_interface_ref(proto: &PlannedProtoTypeInfo) -> String {
    let relative_name = relative_descriptor_name(&proto.full_name, &proto.package);
    let mut parts = relative_name.split('.').collect::<Vec<_>>();
    let leaf = parts
        .pop()
        .expect("descriptor names should not be empty")
        .to_string();
    if parts.is_empty() {
        format!("{}.I{leaf}", proto.package)
    } else {
        format!("{}.{}.I{leaf}", proto.package, parts.join("."))
    }
}

pub(crate) fn model_typescript_interface_ref(
    model_type: &PlannedType,
    api_plan: &PlannedSpec,
) -> Option<String> {
    match model_type {
        PlannedType::External(ExternalTypeSpec::Proto(PlannedProtoType::Message(proto))) => {
            Some(message_typescript_interface_ref(&proto.proto))
        }
        PlannedType::Record(record) => record_proto_info(model_type, api_plan)
            .map(message_typescript_interface_ref)
            .or_else(|| Some(record.model_name.clone())),
        _ => None,
    }
}

pub(crate) fn model_typescript_type_id(
    model_type: &PlannedType,
    api_plan: &PlannedSpec,
) -> Option<String> {
    match model_type {
        PlannedType::External(ExternalTypeSpec::Proto(PlannedProtoType::Message(proto))) => {
            Some(proto.proto.full_name.clone())
        }
        PlannedType::Record(record) => record_proto_info(model_type, api_plan)
            .map(|proto| proto.full_name.clone())
            .or_else(|| Some(record.full_name.clone())),
        _ => None,
    }
}

pub(crate) fn record_proto_info<'a>(
    model_type: &PlannedType,
    api_plan: &'a PlannedSpec,
) -> Option<&'a PlannedProtoTypeInfo> {
    let PlannedType::Record(record) = model_type else {
        return None;
    };
    api_plan
        .record(&record.full_name)
        .and_then(|record| record.data.proto.as_ref())
}

pub(crate) fn typescript_default_from_proto_name(name: &str) -> String {
    format!(
        "{}FromProto",
        name.rsplit('.')
            .next()
            .expect("converter name source should have a final segment")
            .to_lower_camel_case()
    )
}

pub(crate) fn typescript_default_to_proto_name(name: &str) -> String {
    format!(
        "{}ToProto",
        name.rsplit('.')
            .next()
            .expect("converter name source should have a final segment")
            .to_lower_camel_case()
    )
}

pub(crate) fn typescript_from_proto_converter(
    name: &str,
    replacement: &TypeReplacementSpec,
) -> String {
    replacement
        .from_proto
        .for_language(Language::TypeScript)
        .map(str::to_string)
        .unwrap_or_else(|| typescript_default_from_proto_name(name))
}

pub(crate) fn typescript_to_proto_converter(
    name: &str,
    replacement: &TypeReplacementSpec,
) -> String {
    replacement
        .to_proto
        .for_language(Language::TypeScript)
        .map(str::to_string)
        .unwrap_or_else(|| typescript_default_to_proto_name(name))
}

pub(crate) fn typescript_replacement_type_name(
    replacement: &TypeReplacementSpec,
) -> Option<String> {
    replacement
        .type_name
        .for_language(Language::TypeScript)
        .map(str::to_string)
}

pub(in crate::generator) fn message_override_conversion(
    model_type: &PlannedType,
) -> Option<WireValueConversion> {
    let PlannedType::External(ExternalTypeSpec::Proto(PlannedProtoType::Message(proto))) =
        model_type
    else {
        return None;
    };
    if let Some(language_override) = &proto.replacement
        && let Some(type_name) = typescript_replacement_type_name(language_override)
    {
        return Some(WireValueConversion {
            annotation: type_name,
            from_wire: format!(
                "{}({{wire}})",
                typescript_from_proto_converter(&proto.proto.full_name, language_override)
            ),
            to_wire: format!(
                "{}({{value}})",
                typescript_to_proto_converter(&proto.proto.full_name, language_override)
            ),
            function_name_to_wire: Some(format!(
                "{}({{name}})",
                typescript_to_proto_converter(&proto.proto.full_name, language_override)
            )),
            wire_function_names: None,
            uses_rendered_model_annotation: false,
        });
    }
    if let Some(authored_type) = &proto.authored_type {
        return Some(WireValueConversion {
            annotation: typescript_authored_type_annotation(authored_type),
            from_wire: format!(
                "{}({{wire}})",
                typescript_default_from_proto_name(&proto.proto.full_name)
            ),
            to_wire: format!(
                "{}({{value}})",
                typescript_default_to_proto_name(&proto.proto.full_name)
            ),
            function_name_to_wire: Some(format!(
                "{}({{name}})",
                typescript_default_to_proto_name(&proto.proto.full_name)
            )),
            wire_function_names: None,
            uses_rendered_model_annotation: false,
        });
    }
    None
}

fn generated_wire_conversion(
    model_type: &PlannedType,
    planned_model: &RecordSpec<PlannedFamily>,
) -> Option<WireValueConversion> {
    let model_name = if planned_model.data.proto.is_some() {
        planned_model.name.clone()
    } else {
        let PlannedType::External(ExternalTypeSpec::Proto(PlannedProtoType::Message(proto))) =
            model_type
        else {
            return None;
        };
        proto.model_name.clone()
    };
    let from_wire_function_name = model_from_proto_function_name(&model_name);
    let to_wire_function_name = model_to_proto_function_name(&model_name);
    Some(WireValueConversion {
        annotation: model_name.clone(),
        from_wire: format!("{from_wire_function_name}({{wire}})"),
        to_wire: format!("{to_wire_function_name}({{value}}) ?? {{}}"),
        function_name_to_wire: Some(format!(
            "{to_wire_function_name}({{ name: {{name}} }}) ?? {{}}"
        )),
        wire_function_names: Some(WireFunctionNames {
            from_wire: from_wire_function_name,
            to_wire: to_wire_function_name,
        }),
        uses_rendered_model_annotation: true,
    })
}

fn model_to_proto_function_name(model_name: &str) -> String {
    format!("{}ToProto", model_name.to_lower_camel_case())
}

fn model_from_proto_function_name(model_name: &str) -> String {
    format!("{}FromProto", model_name.to_lower_camel_case())
}

pub(crate) fn field_name(field: &FieldDescriptorProto, explicit_name: Option<&str>) -> String {
    match explicit_name {
        Some(name) => typescript_generated_field_name(name),
        None => {
            let name = field
                .json_name
                .as_deref()
                .or_else(|| field.name.as_deref())
                .expect("descriptor fields should be named");
            typescript_ident(name)
        }
    }
}

fn enum_wire_conversion(value_type: &PlannedType) -> Option<WireValueConversion> {
    let PlannedType::External(ExternalTypeSpec::Proto(PlannedProtoType::Enum(enum_type))) =
        value_type
    else {
        return None;
    };
    if let Some(replacement) = &enum_type.replacement
        && let Some(type_name) = typescript_replacement_type_name(replacement)
    {
        return Some(WireValueConversion {
            annotation: type_name.clone(),
            from_wire: format!(
                "{}({{wire}})",
                typescript_from_proto_converter(&enum_type.proto.full_name, replacement)
            ),
            to_wire: format!(
                "{}({{value}})",
                typescript_to_proto_converter(&enum_type.proto.full_name, replacement)
            ),
            function_name_to_wire: None,
            wire_function_names: None,
            uses_rendered_model_annotation: false,
        });
    }

    None
}

fn rendered_model_wire_annotation(
    model: &RenderedModel,
    planned_record: &RecordSpec<PlannedFamily>,
) -> String {
    planned_record
        .data
        .proto
        .as_ref()
        .map(|proto| message_typescript_interface_ref(proto))
        .unwrap_or_else(|| model.name.clone())
}

fn render_model_to_proto_function(
    output: &mut String,
    model: &RenderedModel,
    planned_record: &RecordSpec<PlannedFamily>,
    function_name: &str,
) {
    let wire_annotation = rendered_model_wire_annotation(model, planned_record);
    if model.type_parameters.is_empty() {
        output.push_str("export function ");
        output.push_str(function_name);
        output.push_str("(\n");
        output.push_str("  model: ");
        output.push_str(&model.name);
        output.push_str(" | null | undefined,\n");
    } else {
        render_named_generic_function_start(
            output,
            &format!("export function {function_name}"),
            &model.type_parameters,
            0,
        );
        output.push_str("  model: ");
        output.push_str(&generic_model_annotation(
            &model.name,
            &model.type_parameters,
        ));
        output.push_str(" | null | undefined,\n");
    }
    output.push_str("): ");
    output.push_str(&wire_annotation);
    output.push_str(" | undefined {\n");
    output.push_str("  if (model == null) {\n");
    output.push_str("    return undefined;\n");
    output.push_str("  }\n");
    if model.fields.is_empty() && model.sourced_fields.is_empty() {
        output.push_str("  return {};\n");
    } else {
        output.push_str("  return {\n");
        for field in &model.fields {
            output.push_str("    ");
            output.push_str(&field.wire_name);
            output.push_str(": ");
            output.push_str(&field.to_wire_expr);
            output.push_str(",\n");
        }
        for field in &model.sourced_fields {
            output.push_str("    ");
            output.push_str(&field.name);
            output.push_str(": ");
            output.push_str(&field.to_wire_expr);
            output.push_str(",\n");
        }
        output.push_str("  };\n");
    }
    output.push_str("}\n");
}

fn render_model_from_proto_function(
    output: &mut String,
    model: &RenderedModel,
    planned_record: &RecordSpec<PlannedFamily>,
    function_name: &str,
) {
    let wire_annotation = rendered_model_wire_annotation(model, planned_record);
    if model.type_parameters.is_empty() {
        output.push_str("export function ");
        output.push_str(function_name);
        output.push_str("(\n");
        output.push_str("  proto: ");
        output.push_str(&wire_annotation);
        output.push_str(" | null | undefined,\n");
    } else {
        render_named_generic_function_start(
            output,
            &format!("export function {function_name}"),
            &model.type_parameters,
            0,
        );
        output.push_str("  proto: ");
        output.push_str(&wire_annotation);
        output.push_str(" | null | undefined,\n");
    }
    output.push_str("): ");
    output.push_str(&generic_model_annotation(
        &model.name,
        &model.type_parameters,
    ));
    output.push_str(" | undefined {\n");
    output.push_str("  if (proto == null) {\n");
    output.push_str("    return undefined;\n");
    output.push_str("  }\n");
    if model.fields.is_empty() {
        output.push_str("  return {};\n");
    } else {
        output.push_str("  return {\n");
        for field in &model.fields {
            if field.flattened_fields.is_empty() {
                output.push_str("    ");
                output.push_str(&field.name);
                output.push_str(": ");
                output.push_str(&from_proto_expr(&field.from_wire_expr));
                output.push_str(",\n");
            } else {
                for flattened_field in &field.flattened_fields {
                    output.push_str("    ");
                    output.push_str(&flattened_field.name);
                    output.push_str(": ");
                    output.push_str(&from_proto_expr(&flattened_field.from_wire_expr));
                    output.push_str(",\n");
                }
            }
        }
        output.push_str("  };\n");
    }
    output.push_str("}\n");
}

fn from_proto_expr(field_expr: &str) -> String {
    field_expr.replace("{wire}", "proto")
}
