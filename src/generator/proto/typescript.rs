use std::borrow::Cow;
use std::collections::BTreeMap;

use heck::ToLowerCamelCase;
use prost_types::FieldDescriptorProto;

use crate::error::Result;
use crate::generator::ExternalModelBackend;
use crate::generator::typescript::{
    RenderedExternalModelFragments, RenderedField, RenderedModel, ResolvedFieldType,
    WireFunctionNames, WireValueConversion, flattened_field_from_wire_expr,
    flattened_message_to_wire_expr, generic_model_annotation, map_value_from_wire_expr,
    map_value_to_wire_expr_from_expr, optional_from_wire_expr, optional_to_wire_expr,
    render_named_generic_function_start, repeated_from_wire_expr, repeated_to_wire_expr,
    required_field_expr, required_from_wire_expr, required_to_wire_expr, support_reference,
    typescript_authored_type_annotation, typescript_generated_field_name, typescript_ident,
};
use crate::planning::{
    PlannedFamily, PlannedProtoType, PlannedProtoTypeInfo, PlannedSpec, PlannedType,
    PlannedWireFieldBinding, relative_descriptor_name,
};
use crate::spec::{EnumSpec, ExternalTypeSpec, RecordFieldSpec, RecordSpec, TypeReplacementSpec};

#[derive(Debug, Default)]
pub(in crate::generator) struct ModelBackend {
    enum_conversions: BTreeMap<String, WireValueConversion>,
}

impl ExternalModelBackend for ModelBackend {
    type ModelFragments = RenderedExternalModelFragments;
    type WireConversion = WireValueConversion;

    fn prepare(&mut self, api_plan: &PlannedSpec) -> Result<()> {
        self.enum_conversions = api_plan
            .enums()
            .filter_map(|(name, model)| {
                generated_enum_wire_conversion(model)
                    .map(|conversion| (name.to_string(), conversion))
            })
            .collect();
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

    fn wire_conversion(
        &self,
        model_type: &PlannedType,
        planned_record: Option<&RecordSpec<PlannedFamily>>,
    ) -> Option<WireValueConversion> {
        let native_enum = match model_type {
            PlannedType::Enum(enumeration) => {
                self.enum_conversions.get(&enumeration.full_name).cloned()
            }
            _ => None,
        };
        native_enum
            .or_else(|| enum_wire_conversion(model_type))
            .or_else(|| message_override_conversion(model_type))
            .or_else(|| {
                planned_record.and_then(|record| generated_wire_conversion(model_type, record))
            })
    }
}

enum ModelWireField<'a> {
    Value(Cow<'a, RenderedField>),
    Oneof(OneofField<'a>),
}

impl ModelWireField<'_> {
    fn field(&self) -> &RenderedField {
        match self {
            Self::Value(field) => field,
            Self::Oneof(oneof) => oneof.field,
        }
    }
}

struct OneofField<'a> {
    field: &'a RenderedField,
    local_name: String,
    wire_group: String,
    required: bool,
    cases: Vec<OneofCase>,
}

fn oneof_wire_field<'a>(
    api_plan: &PlannedSpec,
    field: &RecordFieldSpec<PlannedFamily>,
    rendered: &'a RenderedField,
    local_name: String,
    resolve_type: &impl Fn(&PlannedType) -> ResolvedFieldType,
) -> Option<OneofField<'a>> {
    let PlannedWireFieldBinding::VariantMembers { wire_name, members } =
        field.data.wire_binding.as_ref()?
    else {
        return None;
    };
    let PlannedType::Variant(variant_type) = field.field_type.validation_type() else {
        unreachable!("oneof binding requires a variant");
    };
    let variant = api_plan
        .variant(&variant_type.full_name)
        .expect("planned oneof variant should exist");
    let mut cases = Vec::new();
    for (case, member) in variant.cases.iter().zip(members) {
        let payload = case
            .payload
            .as_ref()
            .expect("proto oneof cases have payloads");
        let mut value = resolve_type(payload);
        bind_nested_type_info(api_plan, payload, &mut value);
        let conversion = generic_carrier_conversion(payload, &member.wire_type, &value.annotation)
            .or_else(|| value.wire_conversion.clone());
        cases.push(OneofCase {
            tag: case.name.clone(),
            wire_name: typescript_generated_field_name(&member.wire_name),
            conversion,
        });
    }
    Some(OneofField {
        field: rendered,
        local_name,
        wire_group: typescript_generated_field_name(wire_name),
        required: field.required,
        cases,
    })
}

fn generic_carrier_expressions(
    record: &RecordSpec<PlannedFamily>,
    field: &RecordFieldSpec<PlannedFamily>,
    generated_field_name: &str,
    wire_field_expr: &str,
) -> Option<(String, String)> {
    let PlannedWireFieldBinding::Value { wire_type, .. } = field.data.wire_binding.as_ref()? else {
        return None;
    };
    let PlannedType::TypeParameter(parameter) = field.field_type.validation_type() else {
        return None;
    };
    let conversion = generic_carrier_conversion(&field.field_type, wire_type, &parameter.name)?;
    // Required presence applies to the envelope. A generic application value
    // can itself be null or undefined.
    let value_expr = format!("model.{generated_field_name}");
    let from_wire_expr = if field.required {
        conversion.from_wire_expr(&required_field_expr(
            wire_field_expr,
            &record.name,
            generated_field_name,
        ))
    } else {
        format!(
            "{wire_field_expr} == null ? undefined : {}",
            conversion.from_wire_expr(wire_field_expr)
        )
    };
    let to_wire_expr = if field.required {
        conversion.to_wire_expr(&value_expr)
    } else {
        format!(
            "{value_expr} == null ? undefined : {}",
            conversion.to_wire_expr(&value_expr)
        )
    };
    Some((from_wire_expr, to_wire_expr))
}

/// Bind only the parameters used by the nested record. Parameter identity is
/// shared across records, but the nested converter may use a subset or order.
fn bind_nested_type_info(
    api_plan: &PlannedSpec,
    kind: &PlannedType,
    resolved: &mut ResolvedFieldType,
) {
    if let PlannedType::Record(record) = kind.validation_type()
        && let Some(conversion) = &mut resolved.wire_conversion
        && let Some(names) = &conversion.wire_function_names
    {
        let parameters = api_plan.record_type_parameters(&record.full_name);
        if !parameters.is_empty() {
            let bindings = parameters
                .iter()
                .map(|usage| {
                    let name = usage.parameter.name.to_lower_camel_case();
                    format!("{name}: typeInfo.{name}")
                })
                .collect::<Vec<_>>()
                .join(", ");
            conversion.from_wire = format!("{}({{wire}}, {{ {bindings} }})", names.from_wire);
            conversion.to_wire = format!("{}({{value}}, {{ {bindings} }})", names.to_wire);
        }
    }
}

fn nested_generic_expressions(
    record: &RecordSpec<PlannedFamily>,
    field: &RecordFieldSpec<PlannedFamily>,
    name: &str,
    wire: &str,
    api_plan: &PlannedSpec,
    resolve_type: &impl Fn(&PlannedType) -> ResolvedFieldType,
) -> Option<(String, String)> {
    let kind = match &field.field_type {
        PlannedType::List(inner) | PlannedType::Map(_, inner) => inner,
        kind => kind,
    };
    let PlannedType::Record(nested) = kind.validation_type() else {
        return None;
    };
    if api_plan
        .record_type_parameters(&nested.full_name)
        .is_empty()
    {
        return None;
    }
    let mut resolved = resolve_type(kind);
    bind_nested_type_info(api_plan, kind, &mut resolved);
    resolved
        .wire_conversion
        .as_ref()?
        .wire_function_names
        .as_ref()?;
    Some(match &field.field_type {
        PlannedType::List(_) => (
            repeated_from_wire_expr(&resolved, wire),
            repeated_to_wire_expr(&resolved, name),
        ),
        PlannedType::Map(_, _) => (
            map_value_from_wire_expr(&resolved, wire),
            map_value_to_wire_expr_from_expr(&resolved, &format!("model.{name}")),
        ),
        _ if field.required => (
            required_from_wire_expr(&resolved, &record.name, wire, name, field),
            required_to_wire_expr(&resolved, &record.name, name),
        ),
        _ => (
            optional_from_wire_expr(&resolved, wire),
            optional_to_wire_expr(&resolved, name),
        ),
    })
}

/// Wire-only field expressions are prepared at emission. Public model fields
/// retain their ordinary TypeScript metadata; unchanged fields are borrowed.
fn model_wire_fields<'a>(
    model: &'a RenderedModel,
    record: &RecordSpec<PlannedFamily>,
    api_plan: &PlannedSpec,
    resolve_type: &impl Fn(&PlannedType) -> ResolvedFieldType,
) -> Vec<ModelWireField<'a>> {
    model
        .fields
        .iter()
        .enumerate()
        .map(|(index, rendered)| {
            let (_, field) = record
                .model_fields()
                .find(|(_, field)| typescript_generated_field_name(&field.name) == rendered.name)
                .expect("rendered model field should have a planned field");
            let local_name = if index == 0 {
                "result".to_string()
            } else {
                format!("result{index}")
            };
            if let Some(oneof) =
                oneof_wire_field(api_plan, field, rendered, local_name, resolve_type)
            {
                return ModelWireField::Oneof(oneof);
            }
            let wire_expr = format!("{{wire}}.{}", rendered.wire_name);
            if let Some((from_wire, to_wire)) =
                generic_carrier_expressions(record, field, &rendered.name, &wire_expr)
            {
                let mut wire_field = rendered.clone();
                wire_field.from_wire_expr = from_wire;
                wire_field.to_wire_expr = to_wire;
                return ModelWireField::Value(Cow::Owned(wire_field));
            }
            // Flattened fields still belong to their nested protobuf record.
            let mut wire_field = Cow::Borrowed(rendered);
            if rendered.flattened_fields.is_empty()
                && let Some((from_wire, to_wire)) = nested_generic_expressions(
                    record,
                    field,
                    &rendered.name,
                    &wire_expr,
                    api_plan,
                    resolve_type,
                )
            {
                let target = wire_field.to_mut();
                target.from_wire_expr = from_wire;
                target.to_wire_expr = to_wire;
            }
            if !rendered.flattened_fields.is_empty()
                && let PlannedType::Record(nested) = &field.field_type
            {
                let nested = api_plan
                    .record(&nested.full_name)
                    .expect("flattened record should exist");
                for (index, flattened) in rendered.flattened_fields.iter().enumerate() {
                    let (_, nested_field) = nested
                        .public_fields()
                        .find(|(_, field)| {
                            typescript_generated_field_name(&field.name) == flattened.name
                        })
                        .expect("flattened field should have a planned field");
                    let nested_wire_expr = format!("{wire_expr}.{}", flattened.wire_name);
                    if let Some((from_wire, to_wire)) = generic_carrier_expressions(
                        nested,
                        nested_field,
                        &flattened.name,
                        &nested_wire_expr,
                    )
                    .or_else(|| {
                        nested_generic_expressions(
                            nested,
                            nested_field,
                            &flattened.name,
                            &nested_wire_expr,
                            api_plan,
                            resolve_type,
                        )
                    }) {
                        let nested_annotation = resolve_type(&nested_field.field_type).annotation;
                        let target = &mut wire_field.to_mut().flattened_fields[index];
                        target.from_wire_expr = flattened_field_from_wire_expr(
                            &wire_expr,
                            &from_wire,
                            &nested_annotation,
                            &flattened.annotation,
                        );
                        target.to_wire_expr = to_wire;
                    }
                }
                if let Cow::Owned(wire_field) = &mut wire_field {
                    wire_field.to_wire_expr = flattened_message_to_wire_expr(
                        field.required,
                        &wire_field.flattened_fields,
                    );
                }
            }
            ModelWireField::Value(wire_field)
        })
        .collect()
}

fn generic_carrier_conversion(
    api_type: &PlannedType,
    kind: &PlannedType,
    annotation: &str,
) -> Option<WireValueConversion> {
    if !matches!(api_type.validation_type(), PlannedType::TypeParameter(_)) {
        return None;
    }
    let PlannedType::External(ExternalTypeSpec::Proto(PlannedProtoType::Message(message))) =
        kind.validation_type()
    else {
        return None;
    };
    let type_info = format!("typeInfo.{}", annotation.to_lower_camel_case());
    let (from_wire, to_wire) = match message.proto.full_name.as_str() {
        "temporal.api.common.v1.Payload" => (
            format!(
                "{}<{annotation}>({{wire}}, {type_info})",
                support_reference("payloadToValue")
            ),
            format!(
                "{}({{value}}, {type_info})",
                support_reference("valueToPayload")
            ),
        ),
        "temporal.api.common.v1.Payloads" => (
            format!(
                "{}<{annotation}>({{wire}}, {type_info})",
                support_reference("singleValueFromPayloads")
            ),
            format!(
                "{}([{{value}}], [{type_info} ?? {{}}])",
                support_reference("payloadsToProto")
            ),
        ),
        _ => return None,
    };
    Some(WireValueConversion {
        annotation: annotation.to_string(),
        from_wire,
        to_wire,
        function_name_to_wire: None,
        wire_function_names: None,
        uses_rendered_model_annotation: false,
    })
}

struct OneofCase {
    tag: String,
    wire_name: String,
    conversion: Option<WireValueConversion>,
}

fn render_oneof_from_proto(output: &mut String, owner: &str, oneof: &OneofField<'_>) {
    let name = &oneof.local_name;
    let field = &oneof.field.name;
    output.push_str(&format!(
        "  let {name}: {}{};\n  switch (proto.{}) {{\n",
        oneof.field.annotation,
        if oneof.required { "" } else { " | undefined" },
        oneof.wire_group,
    ));
    for case in &oneof.cases {
        let wire =
            required_field_expr(&format!("proto.{}", case.wire_name), owner, &case.wire_name);
        let decoded = case
            .conversion
            .as_ref()
            .map(|c| c.from_wire_expr(&wire))
            .unwrap_or(wire.clone());
        output.push_str(&format!(
            "    case {:?}:\n      {name} = {{ tag: {:?}, value: {decoded} }};\n      break;\n",
            case.wire_name, case.tag
        ));
    }
    if oneof.required {
        output.push_str(&format!(
            "    case undefined:\n      throw new Error(\"missing required field {owner}.{field}\");\n"
        ));
    } else {
        output.push_str(&format!(
            "    case undefined:\n      {name} = undefined;\n      break;\n"
        ));
    }
    output.push_str(&format!(
        "    default:\n      proto.{} satisfies never;\n      throw new Error(\"invalid oneof discriminator for {owner}.{field}\");\n  }}\n",
        oneof.wire_group,
    ));
}

fn render_oneof_to_proto(output: &mut String, owner: &str, oneof: &OneofField<'_>) {
    let name = &oneof.local_name;
    let field = &oneof.field.name;
    let value = format!("model.{field}");
    if oneof.required {
        output.push_str(&format!(
            "  const {name} = {};\n",
            required_field_expr(&value, owner, field)
        ));
    } else {
        output.push_str(&format!(
            "  const {name} = {value};\n  if ({name} != null) {{\n"
        ));
    }
    let indent = if oneof.required { "  " } else { "    " };
    output.push_str(&format!("{indent}switch ({name}.tag) {{\n"));
    for case in &oneof.cases {
        let encoded = case
            .conversion
            .as_ref()
            .map(|c| c.to_wire_expr(&format!("{name}.value")))
            .unwrap_or_else(|| format!("{name}.value"));
        output.push_str(&format!(
            "{indent}  case {:?}:\n{indent}    proto.{} = {:?};\n{indent}    proto.{} = {encoded};\n{indent}    break;\n",
            case.tag, oneof.wire_group, case.wire_name, case.wire_name,
        ));
    }
    output.push_str(&format!(
        "{indent}  default:\n{indent}    {name} satisfies never;\n{indent}    throw new Error(\"invalid oneof tag for {owner}.{field}\");\n{indent}}}\n"
    ));
    if !oneof.required {
        output.push_str("  }\n");
    }
}

impl ModelBackend {
    /// Report the proto converter values that the Workflow API barrel exposes.
    /// Definitions-only barrels already re-export every model value.
    pub(in crate::generator) fn model_value_exports(
        &self,
        models: &[&RenderedModel],
        include_default_converters: bool,
    ) -> Vec<String> {
        let mut exports = Vec::new();
        if include_default_converters {
            exports.extend(
                models
                    .iter()
                    .filter(|model| model.wire_function_names.is_some())
                    .map(|model| transfer_converter_name(model)),
            );
        }
        exports.extend(
            models
                .iter()
                .filter_map(|model| transfer_converter_factory_name(model)),
        );
        exports
    }

    pub(in crate::generator) fn render_model_wire_functions(
        &self,
        output: &mut String,
        model: &RenderedModel,
        planned_record: &RecordSpec<PlannedFamily>,
        api_plan: &PlannedSpec,
        resolve_type: &impl Fn(&PlannedType) -> ResolvedFieldType,
    ) -> bool {
        let Some(names) = &model.wire_function_names else {
            return false;
        };
        let fields = model_wire_fields(model, planned_record, api_plan, resolve_type);
        render_model_from_proto_function(output, model, planned_record, &fields, &names.from_wire);
        output.push('\n');
        render_model_to_proto_function(output, model, planned_record, &fields, &names.to_wire);
        output.push('\n');
        render_model_transfer_type_converter(output, model, planned_record, names);
        true
    }
}

fn transfer_converter_name(model: &RenderedModel) -> String {
    format!("{}TransferTypeConverter", model.name.to_lower_camel_case())
}

fn transfer_converter_factory_name(model: &RenderedModel) -> Option<String> {
    (model.wire_function_names.is_some()
        && model
            .type_parameters
            .iter()
            .any(|parameter| parameter.explicit))
    .then(|| format!("create{}TransferTypeConverter", model.name))
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
    let converter_name = transfer_converter_name(model);
    let proto_type = message_typescript_interface_ref(proto);
    output.push_str("export const ");
    output.push_str(&converter_name);
    output.push_str(" = {\n");
    render_converter_method_start(output, "  fromTransferType", model);
    output.push_str("  value: ");
    output.push_str(&proto_type);
    output.push_str("): ");
    output.push_str(&generic_model_annotation(
        &model.name,
        &model.type_parameters,
    ));
    output.push_str(" {\n    return ");
    output.push_str(&names.from_wire);
    output.push_str(&format!("{}(value);\n  }},\n\n", type_arguments(model)));
    render_converter_method_start(output, "  toTransferType", model);
    output.push_str("  value: ");
    output.push_str(&generic_model_annotation(
        &model.name,
        &model.type_parameters,
    ));
    output.push_str("): ");
    output.push_str(&proto_type);
    output.push_str(" {\n    return ");
    output.push_str(&names.to_wire);
    output.push_str("(value);\n  },\n};");
    if let Some(factory_name) = transfer_converter_factory_name(model) {
        output.push_str("\n\n");
        render_named_generic_function_start(
            output,
            &format!("export function {factory_name}"),
            &model.type_parameters,
            0,
        );
        render_type_info_parameter(output, model);
        output.push_str(&format!(
            "): common.TransferTypeConverter<{}, {proto_type}> {{\n  return {{\n    fromTransferType(value) {{\n      return {}{}(value, typeInfo);\n    }},\n    toTransferType(value) {{\n      return {}(value, typeInfo);\n    }},\n  }};\n}}",
            generic_model_annotation(&model.name, &model.type_parameters),
            names.from_wire,
            type_arguments(model),
            names.to_wire,
        ));
    }
}

fn render_type_info_parameter(output: &mut String, model: &RenderedModel) {
    if !model
        .type_parameters
        .iter()
        .any(|parameter| parameter.explicit)
    {
        return;
    }
    output.push_str("  typeInfo: {\n");
    for parameter in model
        .type_parameters
        .iter()
        .filter(|parameter| parameter.explicit)
    {
        output.push_str(&format!(
            "    {}?: common.TypeInfo<{}, any>;\n",
            parameter.name.to_lower_camel_case(),
            parameter.name,
        ));
    }
    output.push_str("  } = {},\n");
}

fn type_arguments(model: &RenderedModel) -> String {
    generic_model_annotation("", &model.type_parameters)
}

fn render_converter_method_start(output: &mut String, name: &str, model: &RenderedModel) {
    if model.type_parameters.is_empty() {
        output.push_str(name);
        output.push_str("(\n");
    } else {
        render_named_generic_function_start(output, name, &model.type_parameters, 2);
    }
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
    replacement: &TypeReplacementSpec<PlannedFamily>,
) -> String {
    support_reference(
        &replacement
            .from_proto
            .value()
            .map(str::to_string)
            .unwrap_or_else(|| typescript_default_from_proto_name(name)),
    )
}

pub(crate) fn typescript_to_proto_converter(
    name: &str,
    replacement: &TypeReplacementSpec<PlannedFamily>,
) -> String {
    support_reference(
        &replacement
            .to_proto
            .value()
            .map(str::to_string)
            .unwrap_or_else(|| typescript_default_to_proto_name(name)),
    )
}

pub(crate) fn typescript_replacement_type_name(
    replacement: &TypeReplacementSpec<PlannedFamily>,
) -> Option<String> {
    replacement.type_name.value().map(str::to_string)
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
                support_reference(&typescript_default_from_proto_name(&proto.proto.full_name))
            ),
            to_wire: format!(
                "{}({{value}})",
                support_reference(&typescript_default_to_proto_name(&proto.proto.full_name))
            ),
            function_name_to_wire: Some(format!(
                "{}({{name}})",
                support_reference(&typescript_default_to_proto_name(&proto.proto.full_name))
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
        to_wire: format!("{to_wire_function_name}({{value}})"),
        function_name_to_wire: Some(format!("{to_wire_function_name}({{ name: {{name}} }})")),
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

/// The models module function that decodes a proto number as an enum with a fallback case.
pub(crate) fn enum_from_number_function_name(enum_name: &str) -> String {
    format!("{}FromNumber", enum_name.to_lower_camel_case())
}

fn generated_enum_wire_conversion(model: &EnumSpec<PlannedFamily>) -> Option<WireValueConversion> {
    let source = model.source.as_ref()?.proto_type()?;
    let PlannedProtoType::Enum(proto) = source else {
        return None;
    };
    let from_wire = if model.values.iter().any(|value| value.fallback) {
        format!(
            "{}({{wire}} as number)",
            enum_from_number_function_name(&model.name)
        )
    } else {
        format!("{{wire}} as number as {}", model.name)
    };
    Some(WireValueConversion {
        annotation: model.name.clone(),
        from_wire,
        to_wire: format!("{{value}} as number as {}", proto.proto.full_name),
        function_name_to_wire: None,
        wire_function_names: None,
        uses_rendered_model_annotation: true,
    })
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
    fields: &[ModelWireField<'_>],
    function_name: &str,
) {
    let wire_annotation = rendered_model_wire_annotation(model, planned_record);
    if model.type_parameters.is_empty() {
        output.push_str("function ");
        output.push_str(function_name);
        output.push_str("(\n");
    } else {
        render_named_generic_function_start(
            output,
            &format!("function {function_name}"),
            &model.type_parameters,
            0,
        );
    }
    output.push_str("  model: ");
    output.push_str(&generic_model_annotation(
        &model.name,
        &model.type_parameters,
    ));
    output.push_str(",\n");
    render_type_info_parameter(output, model);
    output.push_str("): ");
    output.push_str(&wire_annotation);
    output.push_str(" {\n");
    if fields
        .iter()
        .any(|field| matches!(field, ModelWireField::Oneof(_)))
    {
        output.push_str(&format!("  const proto: {wire_annotation} = {{}};\n"));
        for field in fields {
            match field {
                ModelWireField::Oneof(oneof) => {
                    render_oneof_to_proto(output, &planned_record.name, oneof)
                }
                ModelWireField::Value(field) => {
                    output.push_str(&format!(
                        "  proto.{} = {};\n",
                        field.wire_name, field.to_wire_expr
                    ));
                }
            }
        }
        for field in &model.sourced_fields {
            output.push_str(&format!(
                "  proto.{} = {};\n",
                field.name, field.to_wire_expr
            ));
        }
        output.push_str("  return proto;\n");
    } else if model.fields.is_empty() && model.sourced_fields.is_empty() {
        output.push_str("  return {};\n");
    } else {
        output.push_str("  return {\n");
        for field in fields {
            output.push_str("    ");
            let field = field.field();
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
    fields: &[ModelWireField<'_>],
    function_name: &str,
) {
    let wire_annotation = rendered_model_wire_annotation(model, planned_record);
    if model.type_parameters.is_empty() {
        output.push_str("function ");
        output.push_str(function_name);
        output.push_str("(\n");
    } else {
        render_named_generic_function_start(
            output,
            &format!("function {function_name}"),
            &model.type_parameters,
            0,
        );
    }
    output.push_str("  proto: ");
    output.push_str(&wire_annotation);
    output.push_str(",\n");
    render_type_info_parameter(output, model);
    output.push_str("): ");
    output.push_str(&generic_model_annotation(
        &model.name,
        &model.type_parameters,
    ));
    output.push_str(" {\n");
    if model.fields.is_empty() && model.sourced_fields.is_empty() {
        output.push_str("  return {};\n");
    } else {
        for field in fields {
            if let ModelWireField::Oneof(oneof) = field {
                render_oneof_from_proto(output, &planned_record.name, oneof);
            }
        }
        output.push_str("  return {\n");
        for field in fields {
            if let ModelWireField::Oneof(oneof) = field {
                if oneof.field.name == oneof.local_name {
                    output.push_str(&format!("    {},\n", oneof.local_name));
                } else {
                    output.push_str(&format!(
                        "    {}: {},\n",
                        oneof.field.name, oneof.local_name
                    ));
                }
                continue;
            }
            let field = field.field();
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
        for field in &model.sourced_fields {
            output.push_str("    ");
            output.push_str(&field.name);
            output.push_str(": ");
            output.push_str(&from_proto_expr(&field.from_wire_expr));
            output.push_str(",\n");
        }
        output.push_str("  };\n");
    }
    output.push_str("}\n");
}

fn from_proto_expr(field_expr: &str) -> String {
    field_expr.replace("{wire}", "proto")
}
