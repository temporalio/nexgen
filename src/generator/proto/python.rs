use heck::ToSnakeCase;

use crate::error::{Error, Result};
use crate::generator::ExternalModelBackend;
use crate::generator::python::{
    PythonFieldDefaultKind, PythonImports, RenderedField, RenderedModel, RenderedModelFragments,
    RenderedRecordWireBlock, ResolvedFieldKind, ResolvedFieldType, WireValueConversion,
    python_authored_type_annotation, python_string_literal, python_variant_case_class_name,
};
use crate::language::Language;
use crate::planning::{
    PlannedFamily, PlannedProtoType, PlannedProtoTypeInfo, PlannedSpec, PlannedType,
    PlannedWireFieldBinding, PlannedWireVariantMember, relative_descriptor_name,
};
use crate::spec::{ExternalTypeSpec, RecordFieldSpec, RecordSpec, TypeReplacementSpec};

#[derive(Debug)]
struct RenderedWireRead {
    setup_lines: Vec<String>,
    expr: String,
}

#[derive(Debug)]
struct RenderedWireWrite {
    lines: Vec<String>,
}

enum WireReadPolicy {
    Required { missing_error: String },
    Optional,
    Default { default_expr: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProtoGenericCarrier {
    Payload,
    Payloads,
}

#[derive(Debug, Clone)]
struct ProtoOneofCase {
    class_name: String,
    proto_name: String,
    payload_type: ResolvedFieldType,
    generic_carrier: Option<ProtoGenericCarrier>,
}

#[derive(Debug, Clone)]
struct ProtoOneof {
    name: String,
    cases: Vec<ProtoOneofCase>,
}

#[derive(Debug, Clone, Default)]
struct ProtoField {
    oneof: Option<ProtoOneof>,
    generic_carrier: Option<ProtoGenericCarrier>,
}

#[derive(Debug, Default)]
pub(in crate::generator) struct ModelBackend;

impl ExternalModelBackend for ModelBackend {
    type ModelFragments = RenderedModelFragments;
    type WireConversion = WireValueConversion;

    fn prepare(&mut self, _api_plan: &PlannedSpec) -> Result<()> {
        Ok(())
    }

    fn render_models(&self) -> Result<RenderedModelFragments> {
        Ok(RenderedModelFragments::default())
    }

    fn model_type_annotation(&self, model_type: &PlannedType) -> Option<String> {
        match model_type {
            PlannedType::External(ExternalTypeSpec::Proto(PlannedProtoType::Message(message))) => {
                message_python_ref(&message.proto).map(|reference| reference.type_ref)
            }
            PlannedType::External(ExternalTypeSpec::Proto(PlannedProtoType::Enum(enumeration))) => {
                enumeration
                    .replacement
                    .as_ref()
                    .and_then(python_replacement_type_name)
                    .or_else(|| Some(enumeration.name.clone()))
            }
            PlannedType::Record(record) => Some(record.model_name.clone()),
            _ => None,
        }
    }

    fn wire_type_identifier(&self, model_type: &PlannedType) -> Option<String> {
        match model_type {
            PlannedType::External(ExternalTypeSpec::Proto(PlannedProtoType::Message(message))) => {
                Some(message.proto.full_name.clone())
            }
            PlannedType::External(ExternalTypeSpec::Proto(PlannedProtoType::Enum(_))) => None,
            PlannedType::Record(record) => Some(record.full_name.clone()),
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

fn build_oneof(
    api_plan: &PlannedSpec,
    message_name: &str,
    field: &RecordFieldSpec<PlannedFamily>,
    wire_name: &str,
    members: &[PlannedWireVariantMember],
    resolve_type: &impl Fn(&PlannedType) -> Result<ResolvedFieldType>,
) -> Result<ProtoOneof> {
    let PlannedType::Variant(variant_type) = field.field_type.validation_type() else {
        return Err(Error::InvalidTypeOverrideField {
            message: message_name.to_string(),
            field: wire_name.to_string(),
            property: "type",
            reason: "wire variant members do not resolve to a planned variant".to_string(),
        });
    };
    let variant = api_plan.variant(&variant_type.full_name).ok_or_else(|| {
        Error::InvalidTypeOverrideField {
            message: message_name.to_string(),
            field: wire_name.to_string(),
            property: "type",
            reason: format!(
                "planned variant `{}` is unavailable",
                variant_type.full_name
            ),
        }
    })?;
    let mut cases = Vec::new();
    for member in members {
        let case = variant
            .cases
            .iter()
            .find(|case| case.wire_name == member.wire_name)
            .ok_or_else(|| Error::InvalidTypeOverrideField {
                message: message_name.to_string(),
                field: wire_name.to_string(),
                property: "type",
                reason: format!(
                    "planned variant `{}` is missing wire case `{}`",
                    variant.name, member.wire_name
                ),
            })?;
        let payload = case
            .payload
            .clone()
            .ok_or_else(|| Error::InvalidTypeOverrideField {
                message: message_name.to_string(),
                field: wire_name.to_string(),
                property: "type",
                reason: format!("planned variant case `{}` has no payload", case.name),
            })?;
        cases.push(ProtoOneofCase {
            class_name: python_variant_case_class_name(&variant.name, &case.name),
            proto_name: member.wire_name.clone(),
            payload_type: resolve_type(&payload)?,
            generic_carrier: matches!(payload.validation_type(), PlannedType::TypeParameter(_))
                .then(|| proto_generic_carrier(&member.wire_type))
                .flatten(),
        });
    }
    Ok(ProtoOneof {
        name: wire_name.to_string(),
        cases,
    })
}

fn proto_generic_carrier(wire_type: &PlannedType) -> Option<ProtoGenericCarrier> {
    let PlannedType::External(ExternalTypeSpec::Proto(PlannedProtoType::Message(message))) =
        wire_type.validation_type()
    else {
        return None;
    };
    match message.proto.full_name.as_str() {
        "temporal.api.common.v1.Payload" => Some(ProtoGenericCarrier::Payload),
        "temporal.api.common.v1.Payloads" => Some(ProtoGenericCarrier::Payloads),
        _ => None,
    }
}

impl ModelBackend {
    fn analyze_field(
        api_plan: &PlannedSpec,
        record: &RecordSpec<PlannedFamily>,
        field: &RecordFieldSpec<PlannedFamily>,
        resolve_type: &impl Fn(&PlannedType) -> Result<ResolvedFieldType>,
    ) -> Result<ProtoField> {
        let generic_carrier = match &field.data.wire_binding {
            Some(PlannedWireFieldBinding::Value { wire_type, .. })
                if matches!(
                    field.field_type.validation_type(),
                    PlannedType::TypeParameter(_)
                ) =>
            {
                proto_generic_carrier(wire_type)
            }
            _ => None,
        };
        let oneof = match (&record.data.proto, &field.data.wire_binding) {
            (Some(proto), Some(PlannedWireFieldBinding::VariantMembers { wire_name, members })) => {
                Some(build_oneof(
                    api_plan,
                    &proto.full_name,
                    field,
                    wire_name,
                    members,
                    resolve_type,
                )?)
            }
            _ => None,
        };
        Ok(ProtoField {
            oneof,
            generic_carrier,
        })
    }

    pub(in crate::generator) fn render_record_wire_block(
        &self,
        api_plan: &PlannedSpec,
        model: &RenderedModel,
        planned_model: &RecordSpec<PlannedFamily>,
        resolve_type: &impl Fn(&PlannedType) -> Result<ResolvedFieldType>,
    ) -> Result<Option<RenderedRecordWireBlock>> {
        render_record_wire_block(api_plan, model, planned_model, resolve_type)
    }

    pub(in crate::generator) fn service_wire_model_ref(
        &self,
        model_type: &PlannedType,
    ) -> Option<PythonReference> {
        external_message_python_ref(model_type)
    }

    pub(in crate::generator) fn enum_field_type(
        &self,
        value_type: &PlannedType,
    ) -> Option<ResolvedFieldType> {
        enum_field_type(value_type)
    }
}

#[derive(Debug)]
pub(crate) struct PythonReference {
    pub(crate) module_path: String,
    pub(crate) type_ref: String,
}

pub(crate) fn message_python_ref(type_info: &PlannedProtoTypeInfo) -> Option<PythonReference> {
    let module_path =
        python_module_path_for_file_name(type_info.file_name.as_deref(), &type_info.package)?;
    let relative_name = relative_descriptor_name(&type_info.full_name, &type_info.package);
    Some(PythonReference {
        type_ref: format!("{module_path}.{relative_name}"),
        module_path,
    })
}

pub(crate) fn enum_python_ref(type_info: &PlannedProtoTypeInfo) -> Option<PythonReference> {
    let module_path =
        python_module_path_for_file_name(type_info.file_name.as_deref(), &type_info.package)?;
    let relative_name = relative_descriptor_name(&type_info.full_name, &type_info.package);
    Some(PythonReference {
        type_ref: format!("{module_path}.{relative_name}.ValueType"),
        module_path,
    })
}

pub(crate) fn external_message_python_ref(model_type: &PlannedType) -> Option<PythonReference> {
    match model_type {
        PlannedType::External(ExternalTypeSpec::Proto(PlannedProtoType::Message(proto))) => {
            message_python_ref(&proto.proto)
        }
        _ => None,
    }
}

fn record_python_ref(planned_model: &RecordSpec<PlannedFamily>) -> Option<PythonReference> {
    planned_model
        .data
        .proto
        .as_ref()
        .and_then(message_python_ref)
}

fn message_override_conversion(model_type: &PlannedType) -> Option<WireValueConversion> {
    let PlannedType::External(ExternalTypeSpec::Proto(PlannedProtoType::Message(proto))) =
        model_type
    else {
        return None;
    };
    if let Some(language_override) = &proto.replacement
        && let Some(type_name) = python_replacement_type_name(language_override)
    {
        let from_proto = python_from_proto_converter(&proto.proto.full_name, language_override);
        let to_proto = python_to_proto_converter(&proto.proto.full_name, language_override);
        return Some(WireValueConversion {
            annotation: type_name,
            from_wire: format!("{from_proto}({{wire}})"),
            to_wire: format!("{to_proto}({{value}})"),
            imports: PythonImports::default(),
            supports_unpacked_input: false,
        });
    }
    if let Some(authored_type) = &proto.authored_type {
        let from_proto = python_default_from_proto_name(&proto.proto.full_name);
        let to_proto = python_default_to_proto_name(&proto.proto.full_name);
        return Some(WireValueConversion {
            annotation: python_authored_type_annotation(authored_type),
            from_wire: format!("{from_proto}({{wire}})"),
            to_wire: format!("{to_proto}({{value}})"),
            imports: PythonImports::default(),
            supports_unpacked_input: false,
        });
    }
    None
}

fn generated_message_model_name(
    model_type: &PlannedType,
    planned_model: &RecordSpec<PlannedFamily>,
) -> Option<String> {
    if planned_model.data.proto.is_some() {
        return Some(planned_model.name.clone());
    }
    let PlannedType::External(ExternalTypeSpec::Proto(PlannedProtoType::Message(proto))) =
        model_type
    else {
        return None;
    };
    Some(proto.model_name.clone())
}

fn generated_wire_conversion(
    model_type: &PlannedType,
    planned_model: &RecordSpec<PlannedFamily>,
) -> Option<WireValueConversion> {
    if let Some(model_name) = generated_message_model_name(model_type, planned_model) {
        return Some(WireValueConversion {
            annotation: model_name.clone(),
            from_wire: format!(
                "_{model_name}TransferTypeConverter().from_transfer_type({{wire}}, {{type_hint}})"
            ),
            to_wire: format!("_{model_name}TransferTypeConverter().to_transfer_type({{value}})"),
            imports: PythonImports::default(),
            supports_unpacked_input: true,
        });
    }
    match model_type {
        PlannedType::Record(record) => Some(WireValueConversion {
            annotation: record.model_name.clone(),
            from_wire: "{wire}".to_string(),
            to_wire: "{value}".to_string(),
            imports: PythonImports::default(),
            supports_unpacked_input: true,
        }),
        _ => None,
    }
}

pub(in crate::generator) struct PythonProtoEnumValue {
    pub(in crate::generator) annotation: String,
    pub(in crate::generator) module_import: Option<String>,
    pub(in crate::generator) conversion: Option<PythonProtoEnumConversion>,
}

pub(in crate::generator) struct PythonProtoEnumConversion {
    pub(in crate::generator) from_proto: String,
    pub(in crate::generator) to_proto: String,
}

pub(in crate::generator) fn enum_value(value_type: &PlannedType) -> Option<PythonProtoEnumValue> {
    let PlannedType::External(ExternalTypeSpec::Proto(PlannedProtoType::Enum(enum_type))) =
        value_type
    else {
        return None;
    };
    if let Some(replacement) = &enum_type.replacement
        && let Some(type_name) = python_replacement_type_name(replacement)
    {
        return Some(PythonProtoEnumValue {
            annotation: type_name,
            module_import: None,
            conversion: Some(PythonProtoEnumConversion {
                from_proto: python_from_proto_converter(&enum_type.proto.full_name, replacement),
                to_proto: python_to_proto_converter(&enum_type.proto.full_name, replacement),
            }),
        });
    }

    Some(PythonProtoEnumValue {
        annotation: enum_type.name.clone(),
        module_import: enum_python_ref(&enum_type.proto).map(|reference| reference.module_path),
        conversion: None,
    })
}

pub(in crate::generator) fn enum_field_type(value_type: &PlannedType) -> Option<ResolvedFieldType> {
    let proto_enum = enum_value(value_type)?;
    let mut imports = PythonImports::default();
    if let Some(module_import) = proto_enum.module_import {
        imports.module_imports.insert(module_import);
    }
    Some(ResolvedFieldType {
        annotation: proto_enum.annotation,
        imports,
        kind: ResolvedFieldKind::Enum,
        wire_conversion: enum_wire_conversion(value_type),
    })
}

fn enum_wire_conversion(value_type: &PlannedType) -> Option<WireValueConversion> {
    let proto_enum = enum_value(value_type)?;
    let conversion = proto_enum.conversion?;
    Some(WireValueConversion {
        annotation: proto_enum.annotation,
        from_wire: format!("{}({{wire}})", conversion.from_proto),
        to_wire: format!("{}({{value}})", conversion.to_proto),
        imports: PythonImports::default(),
        supports_unpacked_input: false,
    })
}

fn field_read(
    proto_name: &str,
    attr_name: &str,
    field: &RecordFieldSpec<PlannedFamily>,
    resolved_value_type: &ResolvedFieldType,
    generic_carrier: Option<ProtoGenericCarrier>,
    type_arguments: &[(String, String)],
    policy: WireReadPolicy,
) -> RenderedWireRead {
    let proto_expr = format!("value.{proto_name}");
    let value_expr = match generic_carrier {
        Some(carrier) => generic_carrier_from_proto_expr(
            carrier,
            resolved_value_type,
            &proto_expr,
            type_arguments,
        ),
        None => match &field.field_type {
            PlannedType::Map(_, _) => {
                map_value_from_proto_expr(resolved_value_type, proto_name, type_arguments)
            }
            PlannedType::List(_) => {
                repeated_from_proto_expr(resolved_value_type, proto_name, type_arguments)
            }
            _ => from_proto_value_expr(resolved_value_type, &proto_expr, type_arguments),
        },
    };

    match policy {
        WireReadPolicy::Required { missing_error } => {
            let mut setup_lines = Vec::new();
            if field_has_proto_presence(field) {
                setup_lines.push(format!("if not value.HasField(\"{proto_name}\"):"));
                setup_lines.push(format!("    raise ValueError({missing_error})"));
            } else if matches!(&field.field_type, PlannedType::String | PlannedType::Bytes) {
                setup_lines.push(format!("if not value.{proto_name}:"));
                setup_lines.push(format!("    raise ValueError({missing_error})"));
            }
            setup_lines.push(format!("{attr_name} = {value_expr}"));
            RenderedWireRead {
                setup_lines,
                expr: attr_name.to_string(),
            }
        }
        WireReadPolicy::Optional => RenderedWireRead {
            setup_lines: Vec::new(),
            expr: optional_from_proto_expr(field, resolved_value_type, proto_name, value_expr),
        },
        WireReadPolicy::Default { default_expr } => RenderedWireRead {
            setup_lines: Vec::new(),
            expr: defaulted_from_proto_expr(field, proto_name, value_expr, default_expr),
        },
    }
}

fn field_write(
    proto_name: &str,
    field: &RecordFieldSpec<PlannedFamily>,
    value_expr: &str,
    resolved_value_type: &ResolvedFieldType,
    generic_carrier: Option<ProtoGenericCarrier>,
    optional_guard: bool,
) -> RenderedWireWrite {
    let lines = match generic_carrier {
        Some(carrier) => {
            generic_carrier_to_proto_lines(carrier, value_expr, proto_name, optional_guard)
        }
        None => match &field.field_type {
            PlannedType::Map(_, _) => map_value_to_proto_lines(
                resolved_value_type,
                value_expr,
                proto_name,
                optional_guard,
            ),
            PlannedType::List(_) => {
                repeated_to_proto_lines(resolved_value_type, value_expr, proto_name, optional_guard)
            }
            _ => value_to_proto_lines(resolved_value_type, value_expr, proto_name, optional_guard),
        },
    };
    RenderedWireWrite { lines }
}

fn generic_carrier_from_proto_expr(
    carrier: ProtoGenericCarrier,
    resolved_type: &ResolvedFieldType,
    proto_expr: &str,
    type_arguments: &[(String, String)],
) -> String {
    match carrier {
        ProtoGenericCarrier::Payload => format!(
            "payload_from_proto({proto_expr}, {})",
            concrete_type_hint(&resolved_type.annotation, type_arguments)
        ),
        ProtoGenericCarrier::Payloads => format!(
            "payloads_from_proto({proto_expr}, [{}])[0]",
            concrete_type_hint(&resolved_type.annotation, type_arguments)
        ),
    }
}

fn concrete_type_hint(annotation: &str, type_arguments: &[(String, String)]) -> String {
    let mut output = String::new();
    let mut identifier = String::new();
    let flush_identifier = |output: &mut String, identifier: &mut String| {
        if identifier.is_empty() {
            return;
        }
        if let Some((_, replacement)) = type_arguments
            .iter()
            .find(|(parameter, _)| parameter == identifier)
        {
            output.push_str(replacement);
        } else {
            output.push_str(identifier);
        }
        identifier.clear();
    };

    for character in annotation.chars() {
        if character.is_ascii_alphanumeric() || character == '_' {
            identifier.push(character);
        } else {
            flush_identifier(&mut output, &mut identifier);
            output.push(character);
        }
    }
    flush_identifier(&mut output, &mut identifier);
    output
}

fn runtime_type_arguments(model: &RenderedModel) -> Vec<(String, String)> {
    let mut arguments = Vec::new();
    for parameter in &model.type_parameters {
        let stem = parameter
            .strip_suffix('T')
            .filter(|stem| !stem.is_empty())
            .unwrap_or(parameter);
        let base_name = format!("{}_type", stem.to_snake_case());
        let mut name = base_name.clone();
        let mut suffix = 1;
        while arguments
            .iter()
            .any(|(_, existing): &(String, String)| existing == &name)
            || model.fields.iter().any(|field| field.attr_name == name)
        {
            suffix += 1;
            name = format!("{base_name}_{suffix}");
        }
        arguments.push((parameter.clone(), name));
    }
    arguments
}

fn generic_carrier_to_proto_lines(
    carrier: ProtoGenericCarrier,
    value_expr: &str,
    proto_name: &str,
    optional_guard: bool,
) -> Vec<String> {
    let mut lines = Vec::new();
    if optional_guard {
        lines.push(format!("if {value_expr} is not None:"));
    }
    let indent = if optional_guard { "    " } else { "" };
    let converted = match carrier {
        ProtoGenericCarrier::Payload => format!("payload_to_proto({value_expr})"),
        ProtoGenericCarrier::Payloads => format!("payloads_to_proto([{value_expr}])"),
    };
    lines.push(format!(
        "{indent}message.{proto_name}.CopyFrom({converted})"
    ));
    lines
}

fn function_field_write(
    proto_name: &str,
    _field: &RecordFieldSpec<PlannedFamily>,
    value_expr: &str,
    converter: &str,
    resolved_type: &ResolvedFieldType,
    optional_guard: bool,
) -> RenderedWireWrite {
    let converted_value = format!("{converter}({value_expr})");
    let mut lines = Vec::new();
    if optional_guard {
        lines.push(format!("if {value_expr} is not None:"));
    }
    let indent = if optional_guard { "    " } else { "" };
    match resolved_type.kind {
        ResolvedFieldKind::Message => lines.push(format!(
            "{indent}message.{proto_name}.CopyFrom({converted_value})"
        )),
        _ => lines.push(format!("{indent}message.{proto_name} = {converted_value}")),
    }
    RenderedWireWrite { lines }
}

fn field_has_proto_presence(field: &RecordFieldSpec<PlannedFamily>) -> bool {
    field.data.has_presence.unwrap_or(!field.required)
}

fn optional_from_proto_expr(
    field: &RecordFieldSpec<PlannedFamily>,
    resolved_type: &ResolvedFieldType,
    proto_name: &str,
    value_expr: String,
) -> String {
    if field_has_proto_presence(field) {
        format!("{value_expr} if value.HasField(\"{proto_name}\") else None")
    } else if let Some(present_expr) =
        no_presence_default_value_present_expr(field, resolved_type, proto_name)
    {
        format!("{value_expr} if {present_expr} else None")
    } else {
        value_expr
    }
}

fn no_presence_default_value_present_expr(
    field: &RecordFieldSpec<PlannedFamily>,
    resolved_type: &ResolvedFieldType,
    proto_name: &str,
) -> Option<String> {
    match resolved_type.kind {
        ResolvedFieldKind::Enum => Some(format!("value.{proto_name} != 0")),
        ResolvedFieldKind::Scalar => match field.field_type.without_option() {
            PlannedType::Bool | PlannedType::String | PlannedType::Bytes => {
                Some(format!("bool(value.{proto_name})"))
            }
            PlannedType::Int(_) | PlannedType::Float => Some(format!("value.{proto_name} != 0")),
            _ => None,
        },
        _ => None,
    }
}

fn defaulted_from_proto_expr(
    field: &RecordFieldSpec<PlannedFamily>,
    proto_name: &str,
    value_expr: String,
    default_expr: String,
) -> String {
    if field_has_proto_presence(field) {
        format!("{value_expr} if value.HasField(\"{proto_name}\") else {default_expr}")
    } else {
        value_expr
    }
}

fn repeated_from_proto_expr(
    resolved_type: &ResolvedFieldType,
    proto_name: &str,
    type_arguments: &[(String, String)],
) -> String {
    match resolved_type.kind {
        ResolvedFieldKind::Message => format!(
            "[{} for item in value.{proto_name}]",
            resolved_type
                .wire_conversion
                .as_ref()
                .expect("message conversion should be present")
                .from_wire_expr_with_type_hint(
                    "item",
                    &concrete_type_hint(&resolved_type.annotation, type_arguments),
                )
        ),
        ResolvedFieldKind::Enum => format!(
            "[{} for item in value.{proto_name}]",
            enum_from_proto_expr(resolved_type, "item")
        ),
        _ => format!("list(value.{proto_name})"),
    }
}

fn map_value_from_proto_expr(
    map_value_type: &ResolvedFieldType,
    proto_name: &str,
    type_arguments: &[(String, String)],
) -> String {
    match map_value_type.kind {
        ResolvedFieldKind::Message => format!(
            "{{key: {} for key, item in value.{proto_name}.items()}}",
            map_value_type
                .wire_conversion
                .as_ref()
                .expect("message conversion should be present")
                .from_wire_expr_with_type_hint(
                    "item",
                    &concrete_type_hint(&map_value_type.annotation, type_arguments),
                )
        ),
        ResolvedFieldKind::Enum => format!(
            "{{key: {} for key, item in value.{proto_name}.items()}}",
            enum_from_proto_expr(map_value_type, "item")
        ),
        _ => format!("{{key: item for key, item in value.{proto_name}.items()}}"),
    }
}

fn from_proto_value_expr(
    resolved_type: &ResolvedFieldType,
    proto_expr: &str,
    type_arguments: &[(String, String)],
) -> String {
    match resolved_type.kind {
        ResolvedFieldKind::Message => resolved_type
            .wire_conversion
            .as_ref()
            .expect("message conversion should be present")
            .from_wire_expr_with_type_hint(
                proto_expr,
                &concrete_type_hint(&resolved_type.annotation, type_arguments),
            ),
        ResolvedFieldKind::Enum => enum_from_proto_expr(resolved_type, proto_expr),
        _ => proto_expr.to_string(),
    }
}

fn repeated_to_proto_lines(
    resolved_type: &ResolvedFieldType,
    value_expr: &str,
    proto_name: &str,
    optional_guard: bool,
) -> Vec<String> {
    let mut lines = Vec::new();
    if optional_guard {
        lines.push(format!("if {value_expr}:"));
    }
    let indent = if optional_guard { "    " } else { "" };
    match resolved_type.kind {
        ResolvedFieldKind::Message => {
            lines.push(format!("{indent}for value in {value_expr}:"));
            lines.push(format!("{indent}    item = message.{proto_name}.add()"));
            lines.push(format!(
                "{indent}    item.CopyFrom({})",
                resolved_type
                    .wire_conversion
                    .as_ref()
                    .expect("message conversion should be present")
                    .to_wire_expr("value")
            ));
        }
        ResolvedFieldKind::Enum => lines.push(format!(
            "{indent}message.{proto_name}.extend({} for value in {value_expr})",
            enum_to_proto_expr(resolved_type, "value")
        )),
        _ => lines.push(format!("{indent}message.{proto_name}.extend({value_expr})")),
    }
    lines
}

fn map_value_to_proto_lines(
    map_value_type: &ResolvedFieldType,
    value_expr: &str,
    proto_name: &str,
    optional_guard: bool,
) -> Vec<String> {
    let mut lines = Vec::new();
    if optional_guard {
        lines.push(format!("if {value_expr}:"));
    }
    let indent = if optional_guard { "    " } else { "" };
    match map_value_type.kind {
        ResolvedFieldKind::Message => {
            lines.push(format!("{indent}for key, value in {value_expr}.items():"));
            lines.push(format!(
                "{indent}    message.{proto_name}[key].CopyFrom({})",
                map_value_type
                    .wire_conversion
                    .as_ref()
                    .expect("message conversion should be present")
                    .to_wire_expr("value")
            ));
        }
        ResolvedFieldKind::Enum => lines.push(format!(
            "{indent}message.{proto_name}.update({{key: {} for key, value in {value_expr}.items()}})",
            enum_to_proto_expr(map_value_type, "value")
        )),
        _ => lines.push(format!("{indent}message.{proto_name}.update({value_expr})")),
    }
    lines
}

fn value_to_proto_lines(
    resolved_type: &ResolvedFieldType,
    value_expr: &str,
    proto_name: &str,
    optional_guard: bool,
) -> Vec<String> {
    let mut lines = Vec::new();
    if optional_guard {
        lines.push(format!("if {value_expr} is not None:"));
    }
    let indent = if optional_guard { "    " } else { "" };
    match resolved_type.kind {
        ResolvedFieldKind::Message => lines.push(format!(
            "{indent}message.{proto_name}.CopyFrom({})",
            resolved_type
                .wire_conversion
                .as_ref()
                .expect("message conversion should be present")
                .to_wire_expr(value_expr)
        )),
        ResolvedFieldKind::Enum => lines.push(format!(
            "{indent}message.{proto_name} = {}",
            enum_to_proto_expr(resolved_type, value_expr)
        )),
        _ => lines.push(format!("{indent}message.{proto_name} = {value_expr}")),
    }
    lines
}

fn enum_from_proto_expr(resolved_type: &ResolvedFieldType, expr: &str) -> String {
    if let Some(enum_conversion) = &resolved_type.wire_conversion {
        enum_conversion.from_wire_expr(expr)
    } else {
        format!("{}({expr})", resolved_type.annotation)
    }
}

fn enum_to_proto_expr(resolved_type: &ResolvedFieldType, expr: &str) -> String {
    if let Some(enum_conversion) = &resolved_type.wire_conversion {
        enum_conversion.to_wire_expr(expr)
    } else {
        format!("int({expr})")
    }
}

pub(crate) fn python_default_from_proto_name(name: &str) -> String {
    format!(
        "{}_from_proto",
        name.rsplit('.')
            .next()
            .expect("converter name source should have a final segment")
            .to_snake_case()
    )
}

pub(crate) fn python_default_to_proto_name(name: &str) -> String {
    format!(
        "{}_to_proto",
        name.rsplit('.')
            .next()
            .expect("converter name source should have a final segment")
            .to_snake_case()
    )
}

pub(crate) fn python_from_proto_converter(name: &str, replacement: &TypeReplacementSpec) -> String {
    replacement
        .from_proto
        .for_language(Language::Python)
        .map(str::to_string)
        .unwrap_or_else(|| python_default_from_proto_name(name))
}

pub(crate) fn python_to_proto_converter(name: &str, replacement: &TypeReplacementSpec) -> String {
    replacement
        .to_proto
        .for_language(Language::Python)
        .map(str::to_string)
        .unwrap_or_else(|| python_default_to_proto_name(name))
}

pub(crate) fn python_replacement_type_name(replacement: &TypeReplacementSpec) -> Option<String> {
    replacement
        .type_name
        .for_language(Language::Python)
        .map(str::to_string)
}

fn render_record_wire_block(
    api_plan: &PlannedSpec,
    model: &RenderedModel,
    planned_model: &RecordSpec<PlannedFamily>,
    resolve_type: &impl Fn(&PlannedType) -> Result<ResolvedFieldType>,
) -> Result<Option<RenderedRecordWireBlock>> {
    let Some(proto_ref) = record_python_ref(planned_model) else {
        return Ok(None);
    };
    let proto_fields = planned_model
        .fields
        .values()
        .filter(|field| field.visibility != crate::spec::RecordFieldVisibility::Omitted)
        .map(|field| ModelBackend::analyze_field(api_plan, planned_model, field, resolve_type))
        .collect::<Result<Vec<_>>>()?;
    let type_arguments = runtime_type_arguments(model);
    let converter_model_annotation = if model.type_parameters.is_empty() {
        format!("\"{}\"", model.name)
    } else {
        format!(
            "\"{}[{}]\"",
            model.name,
            std::iter::repeat_n("typing.Any", model.type_parameters.len())
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let mut output = String::new();
    let converter_name = format!("_{}TransferTypeConverter", model.name);
    let mut pre_class_lines = vec![
        format!(
            "class {converter_name}(temporalio.converter.TransferTypeConverter[{}, {}]):",
            converter_model_annotation, proto_ref.type_ref
        ),
        format!(
            "    transfer_type: type[{}] | None = {}",
            proto_ref.type_ref, proto_ref.type_ref
        ),
        String::new(),
    ];
    {
        if !model.fields.is_empty() {
            output.push('\n');
        }
        output.push_str("    @typing_extensions.override\n");
        output.push_str("    def from_transfer_type(\n");
        output.push_str("        self,\n");
        output.push_str("        value: ");
        output.push_str(&proto_ref.type_ref);
        output.push_str(",\n");
        output.push_str("        type_hint: type[");
        output.push_str(&converter_model_annotation);
        output.push_str("],\n");
        output.push_str("    ) -> ");
        output.push_str(&converter_model_annotation);
        output.push_str(":\n");
        if model.fields.is_empty() {
            output.push_str("        return ");
            output.push_str(&model.name);
            output.push_str("()\n");
        } else {
            if !type_arguments.is_empty() {
                output.push_str("        ");
                output.push_str(
                    &type_arguments
                        .iter()
                        .map(|(_, argument)| argument.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                );
                if type_arguments.len() == 1 {
                    output.push(',');
                }
                output.push_str(" = typing.get_args(type_hint) or (");
                output.push_str(
                    &std::iter::repeat_n("typing.Any", type_arguments.len())
                        .collect::<Vec<_>>()
                        .join(", "),
                );
                if type_arguments.len() == 1 {
                    output.push(',');
                }
                output.push_str(")\n");
            }
            for (((field_name, planned_field), rendered_field), proto_field) in planned_model
                .fields
                .iter()
                .filter(|(_, field)| {
                    field.visibility != crate::spec::RecordFieldVisibility::Omitted
                })
                .map(|(name, field)| (name.as_str(), field))
                .zip(model.fields.iter())
                .zip(proto_fields.iter())
            {
                let read = proto_field.oneof.as_ref().map_or_else(
                    || {
                        field_read(
                            field_name,
                            &rendered_field.attr_name,
                            planned_field,
                            &rendered_field.wire_value_type,
                            proto_field.generic_carrier,
                            &type_arguments,
                            field_read_policy(&model.name, rendered_field),
                        )
                    },
                    |oneof| {
                        oneof_field_read(
                            &model.name,
                            &rendered_field.attr_name,
                            oneof,
                            &type_arguments,
                            matches!(
                                rendered_field.default_kind,
                                PythonFieldDefaultKind::Required
                            ),
                        )
                    },
                );
                for line in &read.setup_lines {
                    output.push_str("        ");
                    output.push_str(line);
                    output.push('\n');
                }
            }

            output.push_str("        return ");
            output.push_str(&model.name);
            output.push_str("(\n");
            for (((field_name, planned_field), rendered_field), proto_field) in planned_model
                .fields
                .iter()
                .filter(|(_, field)| {
                    field.visibility != crate::spec::RecordFieldVisibility::Omitted
                })
                .map(|(name, field)| (name.as_str(), field))
                .zip(model.fields.iter())
                .zip(proto_fields.iter())
            {
                let read = proto_field.oneof.as_ref().map_or_else(
                    || {
                        field_read(
                            field_name,
                            &rendered_field.attr_name,
                            planned_field,
                            &rendered_field.wire_value_type,
                            proto_field.generic_carrier,
                            &type_arguments,
                            field_read_policy(&model.name, rendered_field),
                        )
                    },
                    |oneof| {
                        oneof_field_read(
                            &model.name,
                            &rendered_field.attr_name,
                            oneof,
                            &type_arguments,
                            matches!(
                                rendered_field.default_kind,
                                PythonFieldDefaultKind::Required
                            ),
                        )
                    },
                );
                output.push_str("            ");
                output.push_str(&rendered_field.attr_name);
                output.push_str("=");
                output.push_str(&read.expr);
                output.push_str(",\n");
            }
            output.push_str("        )\n");
        }
    }
    let wrote_method = true;
    {
        let generic_oneof = !model.type_parameters.is_empty()
            && proto_fields.iter().any(|field| field.oneof.is_some());
        if model.fields.is_empty() {
            if wrote_method {
                output.push('\n');
            }
        } else {
            output.push('\n');
            if wrote_method {
                output.push('\n');
            }
        }
        output.push_str("    @typing_extensions.override\n");
        output.push_str("    def to_transfer_type(\n");
        output.push_str("        self,\n");
        output.push_str("        value: ");
        output.push_str(&converter_model_annotation);
        output.push_str(",\n");
        output.push_str("    ) -> ");
        output.push_str(&proto_ref.type_ref);
        output.push_str(":\n");
        if generic_oneof {
            output.push_str("        runtime_value: typing.Any = value\n");
        }
        output.push_str("        message = ");
        output.push_str(&proto_ref.type_ref);
        output.push_str("()\n");
        for (((field_name, planned_field), rendered_field), proto_field) in planned_model
            .fields
            .iter()
            .filter(|(_, field)| field.visibility != crate::spec::RecordFieldVisibility::Omitted)
            .map(|(name, field)| (name.as_str(), field))
            .zip(model.fields.iter())
            .zip(proto_fields.iter())
        {
            let value_expr = format!(
                "{}.{}",
                if generic_oneof {
                    "runtime_value"
                } else {
                    "value"
                },
                rendered_field.attr_name
            );
            let write = field_write_for_rendered_field(
                &model.name,
                field_name,
                planned_field,
                rendered_field,
                proto_field,
                &value_expr,
                generic_oneof,
            );
            for line in &write.lines {
                output.push_str("        ");
                output.push_str(line);
                output.push('\n');
            }
        }
        output.push_str("        return message\n");
    }

    let mut imports = PythonImports {
        module_imports: [proto_ref.module_path, "temporalio.converter".to_string()]
            .into_iter()
            .collect(),
        ..PythonImports::default()
    };
    for field in &proto_fields {
        if let Some(oneof) = &field.oneof {
            for case in &oneof.cases {
                imports.extend(&case.payload_type.imports);
            }
        }
    }
    Ok(Some(RenderedRecordWireBlock {
        imports,
        pre_class_lines: {
            pre_class_lines.extend(output.lines().map(str::to_string));
            pre_class_lines
        },
        decorator: Some(if model.type_parameters.is_empty() {
            format!("@temporalio.converter.transfer_type_convertible({converter_name})")
        } else {
            format!(
                "@typing.cast(typing.Any, temporalio.converter.transfer_type_convertible({converter_name}))"
            )
        }),
        class_body_lines: Vec::new(),
    }))
}

fn field_read_policy(model_name: &str, rendered_field: &RenderedField) -> WireReadPolicy {
    match &rendered_field.default_kind {
        PythonFieldDefaultKind::Required => {
            let missing_error = python_string_literal(&format!(
                "missing required field {model_name}.{}",
                rendered_field.attr_name
            ));
            WireReadPolicy::Required { missing_error }
        }
        PythonFieldDefaultKind::None => WireReadPolicy::Optional,
        PythonFieldDefaultKind::EmptyDict => WireReadPolicy::Default {
            default_expr: "{}".to_string(),
        },
        PythonFieldDefaultKind::EmptyList => WireReadPolicy::Default {
            default_expr: "[]".to_string(),
        },
        PythonFieldDefaultKind::Expression(default_expr) => WireReadPolicy::Default {
            default_expr: default_expr.clone(),
        },
    }
}

fn field_write_for_rendered_field(
    model_name: &str,
    field_name: &str,
    planned_field: &RecordFieldSpec<PlannedFamily>,
    rendered_field: &RenderedField,
    proto_field: &ProtoField,
    value_expr: &str,
    value_is_any: bool,
) -> RenderedWireWrite {
    if let Some(oneof) = &proto_field.oneof {
        return oneof_field_write(
            model_name,
            &rendered_field.attr_name,
            oneof,
            value_expr,
            matches!(
                rendered_field.default_kind,
                PythonFieldDefaultKind::Required
            ),
            value_is_any,
        );
    }
    let optional_guard = matches!(
        rendered_field.default_kind,
        PythonFieldDefaultKind::None
            | PythonFieldDefaultKind::EmptyDict
            | PythonFieldDefaultKind::EmptyList
    );
    let converter = planned_field
        .function
        .as_ref()
        .and_then(|function| function.converter.as_deref())
        .filter(|_| {
            !matches!(
                planned_field.field_type,
                PlannedType::Map(_, _) | PlannedType::List(_)
            ) && planned_field.default_value.is_none()
        });
    match converter {
        Some(converter) => function_field_write(
            field_name,
            planned_field,
            value_expr,
            converter,
            &rendered_field.wire_value_type,
            optional_guard,
        ),
        None => field_write(
            field_name,
            planned_field,
            value_expr,
            &rendered_field.wire_value_type,
            proto_field.generic_carrier,
            optional_guard,
        ),
    }
}

fn oneof_field_read(
    model_name: &str,
    attr_name: &str,
    oneof: &ProtoOneof,
    type_arguments: &[(String, String)],
    required: bool,
) -> RenderedWireRead {
    let local_var = format!("_oneof_{attr_name}");
    let case_var = format!("{local_var}_case");
    let mut setup_lines = vec![
        format!(
            "{case_var} = value.WhichOneof({})",
            python_string_literal(&oneof.name)
        ),
        format!("if {case_var} is None:"),
    ];
    if required {
        setup_lines.push(format!(
            "    raise ValueError({})",
            python_string_literal(&format!("missing required field {model_name}.{attr_name}"))
        ));
    } else {
        setup_lines.push(format!("    {local_var} = None"));
    }
    for case in &oneof.cases {
        setup_lines.push(format!(
            "elif {case_var} == {}:",
            python_string_literal(&case.proto_name)
        ));
        setup_lines.push(format!(
            "    {local_var} = {}({})",
            case.class_name,
            match case.generic_carrier {
                Some(carrier) => generic_carrier_from_proto_expr(
                    carrier,
                    &case.payload_type,
                    &format!("value.{}", case.proto_name),
                    type_arguments,
                ),
                None => from_proto_value_expr(
                    &case.payload_type,
                    &format!("value.{}", case.proto_name),
                    type_arguments,
                ),
            }
        ));
    }
    setup_lines.push("else:".to_string());
    setup_lines.push(format!(
        "    raise ValueError(f\"unknown protobuf oneof case {model_name}.{}: {{{case_var}}}\")",
        oneof.name
    ));
    RenderedWireRead {
        setup_lines,
        expr: local_var,
    }
}

fn oneof_field_write(
    model_name: &str,
    attr_name: &str,
    oneof: &ProtoOneof,
    value_expr: &str,
    required: bool,
    value_is_any: bool,
) -> RenderedWireWrite {
    let mut lines = Vec::new();
    let case_indent = if required {
        lines.push(format!("if {value_expr} is None:"));
        lines.push(format!(
            "    raise ValueError({})",
            python_string_literal(&format!("missing required field {model_name}.{attr_name}"))
        ));
        ""
    } else {
        lines.push(format!("if {value_expr} is not None:"));
        "    "
    };
    let public_value_expr = format!("_oneof_{}_value", attr_name.to_snake_case());
    lines.push(if value_is_any {
        format!("{case_indent}{public_value_expr} = {value_expr}")
    } else {
        format!("{case_indent}{public_value_expr} = typing.cast(typing.Any, {value_expr})")
    });
    for (index, case) in oneof.cases.iter().enumerate() {
        let keyword = if index == 0 { "if" } else { "elif" };
        lines.push(format!(
            "{case_indent}{keyword} isinstance({public_value_expr}, {}):",
            case.class_name
        ));
        let case_value_expr = format!("{public_value_expr}.value");
        let case_lines = match case.generic_carrier {
            Some(carrier) => {
                generic_carrier_to_proto_lines(carrier, &case_value_expr, &case.proto_name, false)
            }
            None => value_to_proto_lines(
                &case.payload_type,
                &case_value_expr,
                &case.proto_name,
                false,
            ),
        };
        for line in case_lines {
            lines.push(format!("{case_indent}    {line}"));
        }
    }
    lines.push(format!("{case_indent}else:"));
    lines.push(format!(
        "{case_indent}    raise TypeError(f\"unsupported variant case {model_name}.{}: {{{public_value_expr}!r}}\")",
        oneof.name,
    ));
    RenderedWireWrite { lines }
}

fn python_module_path_for_file_name(file_name: Option<&str>, package: &str) -> Option<String> {
    if let Some(file_name) = file_name {
        let mut module_path = file_name.replace('/', ".");
        if let Some(stripped) = module_path.strip_suffix(".proto") {
            module_path = format!("{stripped}_pb2");
        }
        if let Some(suffix) = module_path.strip_prefix("temporal.") {
            module_path = format!("temporalio.{suffix}");
        }
        return Some(module_path);
    }

    if package.is_empty() {
        None
    } else if let Some(suffix) = package.strip_prefix("temporal.") {
        Some(format!("temporalio.{suffix}"))
    } else {
        Some(package.to_string())
    }
}
