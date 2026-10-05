use heck::ToLowerCamelCase;

use crate::error::{Error, Result};
use crate::generator::dotnet::{
    WireValueConversion, csharp_parameter_name, csharp_type_name, csharp_type_parameter_name,
    field_property_name, function_args_parameter_type, qualify_dotnet_support_reference,
};
use crate::language::Language;
use crate::planning::{
    PlannedFamily, PlannedProtoMessageType, PlannedProtoType, PlannedProtoTypeInfo, PlannedSpec,
    PlannedType, PlannedWireFieldBinding, PlannedWireVariantMember,
};
use crate::spec::{
    AliasTypeSpec, ExternalTypeSpec, RecordFieldSpec, RecordFieldVisibility, RecordSpec,
    TypeReplacementSpec,
};

#[derive(Debug, Default)]
pub(in crate::generator) struct ModelBackend;

impl ModelBackend {
    pub(in crate::generator) fn prepare(&mut self, api_plan: &PlannedSpec) -> Result<()> {
        for (_, record) in api_plan.records() {
            validate_record_conversion(api_plan, record)?;
        }
        Ok(())
    }

    pub(in crate::generator) fn render_models(&self) -> Result<()> {
        Ok(())
    }

    pub(in crate::generator) fn model_type_annotation(
        &self,
        proto_type: &PlannedProtoType,
    ) -> Option<String> {
        Some(match proto_type {
            PlannedProtoType::Message(message) => dotnet_message_type_for_proto_message(message),
            PlannedProtoType::Enum(enumeration) => enumeration
                .replacement
                .as_ref()
                .and_then(dotnet_replacement_type_name)
                .unwrap_or_else(|| {
                    dotnet_proto_or_local_type(&enumeration.proto, Some(&enumeration.name))
                }),
        })
    }

    pub(in crate::generator) fn wire_conversion(
        &self,
        model_type: &PlannedType,
        planned_record: Option<&RecordSpec<PlannedFamily>>,
    ) -> Option<WireValueConversion> {
        let annotation = match model_type {
            PlannedType::External(ExternalTypeSpec::Proto(proto_type)) => {
                self.model_type_annotation(proto_type)?
            }
            PlannedType::Record(record) => record.model_name.clone(),
            _ => return None,
        };
        Some(WireValueConversion {
            annotation,
            to_wire: self.value_to_wire_expr(
                model_type,
                "{value}",
                false,
                planned_record,
                None,
                None,
            ),
        })
    }

    pub(in crate::generator) fn support_references(&self, value: &PlannedType) -> Vec<String> {
        match value {
            model_type @ PlannedType::External(ExternalTypeSpec::Proto(
                PlannedProtoType::Message(_),
            )) => dotnet_to_proto_converter(model_type)
                .map(|reference| vec![reference.to_string()])
                .unwrap_or_default(),
            PlannedType::Record(_) | PlannedType::Resource(_) => Vec::new(),
            PlannedType::External(ExternalTypeSpec::Alias(AliasTypeSpec {
                target: fallback,
                ..
            })) => self.support_references(fallback),
            PlannedType::Option(inner) | PlannedType::List(inner) => self.support_references(inner),
            PlannedType::Map(key, value) => {
                let mut references = self.support_references(key);
                references.extend(self.support_references(value));
                references
            }
            PlannedType::Tuple(items) => items
                .iter()
                .flat_map(|item| self.support_references(item))
                .collect(),
            PlannedType::Result { ok, err } => {
                let mut references = ok
                    .as_deref()
                    .map(|ok| self.support_references(ok))
                    .unwrap_or_default();
                if let Some(err) = err {
                    references.extend(self.support_references(err));
                }
                references
            }
            _ => Vec::new(),
        }
    }

    pub(in crate::generator) fn function_args_authored_type<'a>(
        &self,
        field: &'a RecordFieldSpec<PlannedFamily>,
    ) -> Option<&'a PlannedType> {
        match &field.field_type {
            PlannedType::External(ExternalTypeSpec::Proto(PlannedProtoType::Message(proto))) => {
                proto.authored_type.as_deref()
            }
            _ => None,
        }
    }
}

impl ModelBackend {
    pub(in crate::generator) fn wire_type_annotation(
        &self,
        model_type: &PlannedType,
        planned_record: Option<&RecordSpec<PlannedFamily>>,
    ) -> Option<String> {
        if let Some(record) = planned_record
            && let Some(proto) = &record.data.proto
        {
            return Some(dotnet_proto_type_name_for_info(proto));
        }
        match model_type {
            PlannedType::External(ExternalTypeSpec::Proto(proto_type)) => {
                self.model_type_annotation(proto_type)
            }
            PlannedType::Record(record) => Some(csharp_type_name(&record.model_name)),
            _ => None,
        }
    }

    pub(in crate::generator) fn model_needs_wire_method(
        &self,
        model: &RecordSpec<PlannedFamily>,
    ) -> bool {
        model.data.proto.as_ref().is_some_and(|proto| {
            dotnet_proto_type_name_for_info(proto) != csharp_type_name(&model.name)
        })
    }

    pub(in crate::generator) fn model_transfer_converter_attribute(
        &self,
        model: &RecordSpec<PlannedFamily>,
        api_plan: &PlannedSpec,
    ) -> Option<String> {
        self.model_needs_wire_method(model).then(|| {
            let type_name = open_model_type_name(model, api_plan);
            format!(
                "[Temporalio.Converters.TemporalTransferTypeConverter(typeof({type_name}.TransferTypeConverter))]"
            )
        })
    }

    pub(in crate::generator) fn render_model_wire_methods(
        &self,
        output: &mut String,
        model: &RecordSpec<PlannedFamily>,
        api_plan: &PlannedSpec,
        support_namespace: Option<&str>,
    ) -> bool {
        if !self.model_needs_wire_method(model) {
            return false;
        }
        render_model_transfer_converter(self, output, model, api_plan, support_namespace);
        true
    }

    pub(in crate::generator) fn model_uses_support_extensions(
        &self,
        model: &RecordSpec<PlannedFamily>,
        api_plan: &PlannedSpec,
    ) -> bool {
        model.sourced_fields().any(|(_, field, _)| {
            self.field_kind_uses_support_extensions(&field.field_type, api_plan)
        }) || model.model_fields().any(|(field_name, field)| {
            function_args_field_uses_logical_storage(model, field_name, field)
                || self.field_kind_uses_support_extensions(&field.field_type, api_plan)
        })
    }

    pub(in crate::generator) fn field_kind_to_wire_expr(
        &self,
        kind: &PlannedType,
        source_expr: &str,
        optional: bool,
        api_plan: &PlannedSpec,
        support_namespace: Option<&str>,
    ) -> String {
        match kind {
            PlannedType::List(_) | PlannedType::Map(_, _) => source_expr.to_string(),
            value => self.value_to_wire_expr(
                value,
                source_expr,
                optional,
                match value {
                    PlannedType::Record(record) => api_plan.record(&record.full_name),
                    _ => None,
                },
                Some(api_plan),
                support_namespace,
            ),
        }
    }

    fn value_to_wire_expr(
        &self,
        value: &PlannedType,
        source_expr: &str,
        optional: bool,
        planned_record: Option<&RecordSpec<PlannedFamily>>,
        api_plan: Option<&PlannedSpec>,
        support_namespace: Option<&str>,
    ) -> String {
        let _ = optional;
        match value {
            model_type @ (PlannedType::External(ExternalTypeSpec::Proto(
                PlannedProtoType::Message(_),
            ))
            | PlannedType::Record(_)) => {
                if planned_record.is_some_and(|model| self.model_needs_wire_method(model)) {
                    let raw_type = dotnet_proto_type_name_for_info(
                        planned_record
                            .and_then(|model| model.data.proto.as_ref())
                            .expect("wire method model should have proto backing"),
                    );
                    format!("({raw_type}){source_expr}.ToTransferType()")
                } else {
                    self.message_to_wire_expr(model_type, source_expr, support_namespace)
                }
            }
            PlannedType::Resource(_) => source_expr.to_string(),
            PlannedType::External(ExternalTypeSpec::Alias(AliasTypeSpec {
                target: fallback,
                ..
            })) => self.value_to_wire_expr(
                fallback,
                source_expr,
                optional,
                match fallback.as_ref() {
                    PlannedType::Record(record) => {
                        api_plan.and_then(|api_plan| api_plan.record(&record.full_name))
                    }
                    _ => None,
                },
                api_plan,
                support_namespace,
            ),
            _ => source_expr.to_string(),
        }
    }

    fn message_to_wire_expr(
        &self,
        model_type: &PlannedType,
        source_expr: &str,
        support_namespace: Option<&str>,
    ) -> String {
        if matches!(
            model_type,
            PlannedType::External(ExternalTypeSpec::Proto(PlannedProtoType::Message(_)))
        ) && dotnet_message_type(model_type) != dotnet_proto_type_name_for_message(model_type)
        {
            if let Some(converter) = dotnet_to_proto_converter(model_type) {
                let converter = qualify_dotnet_support_reference(converter, support_namespace);
                return format!("{converter}({source_expr})");
            }
            format!("{source_expr}.ToProto()")
        } else {
            source_expr.to_string()
        }
    }

    fn field_kind_uses_support_extensions(
        &self,
        kind: &PlannedType,
        api_plan: &PlannedSpec,
    ) -> bool {
        match kind {
            PlannedType::List(value) => self.value_uses_support_extensions(value, api_plan),
            PlannedType::Map(key, value) => {
                self.value_uses_support_extensions(key, api_plan)
                    || self.value_uses_support_extensions(value, api_plan)
            }
            value => self.value_uses_support_extensions(value, api_plan),
        }
    }

    fn value_uses_support_extensions(&self, value: &PlannedType, api_plan: &PlannedSpec) -> bool {
        match value {
            model_type @ PlannedType::External(ExternalTypeSpec::Proto(
                PlannedProtoType::Message(_),
            )) => dotnet_message_type(model_type) != dotnet_proto_type_name_for_message(model_type),
            PlannedType::Record(record) => api_plan
                .record(&record.full_name)
                .is_some_and(|model| self.model_needs_wire_method(model)),
            PlannedType::Resource(_) => false,
            PlannedType::List(inner) => self.value_uses_support_extensions(inner, api_plan),
            PlannedType::Map(key, value) => {
                self.value_uses_support_extensions(key, api_plan)
                    || self.value_uses_support_extensions(value, api_plan)
            }
            PlannedType::External(ExternalTypeSpec::Alias(AliasTypeSpec {
                target: fallback,
                ..
            })) => self.value_uses_support_extensions(fallback, api_plan),
            PlannedType::Result { ok, err } => {
                ok.as_deref()
                    .is_some_and(|ok| self.value_uses_support_extensions(ok, api_plan))
                    || err
                        .as_deref()
                        .is_some_and(|err| self.value_uses_support_extensions(err, api_plan))
            }
            PlannedType::Tuple(items) => items
                .iter()
                .any(|item| self.value_uses_support_extensions(item, api_plan)),
            _ => false,
        }
    }
}

fn validate_record_conversion(
    api_plan: &PlannedSpec,
    record: &RecordSpec<PlannedFamily>,
) -> Result<()> {
    let Some(proto) = &record.data.proto else {
        return Ok(());
    };
    for (_, field) in record
        .fields
        .iter()
        .filter(|(_, field)| field.visibility != RecordFieldVisibility::Omitted)
    {
        if let Some(PlannedWireFieldBinding::VariantMembers { wire_name, members }) =
            &field.data.wire_binding
        {
            oneof_cases(api_plan, &proto.full_name, field, wire_name, members)?;
        }
    }
    Ok(())
}

/// Protobuf messages that carry a type parameter's value through the payload converter.
#[derive(Debug, Clone, Copy)]
enum ProtoGenericCarrier {
    Payload,
    Payloads,
}

/// A WIT variant case bound to one member of a protobuf oneof.
struct OneofCase<'a> {
    case_name: String,
    member: &'a PlannedWireVariantMember,
    payload: &'a PlannedType,
}

fn oneof_cases<'a>(
    api_plan: &'a PlannedSpec,
    message_name: &str,
    field: &RecordFieldSpec<PlannedFamily>,
    wire_name: &str,
    members: &'a [PlannedWireVariantMember],
) -> Result<Vec<OneofCase<'a>>> {
    let invalid = |reason: String| Error::InvalidTypeOverrideField {
        message: message_name.to_string(),
        field: wire_name.to_string(),
        property: "type",
        reason,
    };
    let PlannedType::Variant(variant_type) = field.field_type.validation_type() else {
        return Err(invalid(
            "wire variant members do not resolve to a planned variant".to_string(),
        ));
    };
    let variant = api_plan.variant(&variant_type.full_name).ok_or_else(|| {
        invalid(format!(
            "planned variant `{}` is unavailable",
            variant_type.full_name
        ))
    })?;
    members
        .iter()
        .map(|member| {
            let case = variant
                .cases
                .iter()
                .find(|case| case.wire_name == member.wire_name)
                .ok_or_else(|| {
                    invalid(format!(
                        "planned variant `{}` is missing wire case `{}`",
                        variant.name, member.wire_name
                    ))
                })?;
            let payload = case.payload.as_ref().ok_or_else(|| {
                invalid(format!(
                    "planned variant case `{}` has no payload",
                    case.name
                ))
            })?;
            Ok(OneofCase {
                case_name: csharp_type_name(&case.name),
                member,
                payload,
            })
        })
        .collect()
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

/// Returns the type parameter and carrier when a type-parameter value is stored in a
/// Payload-shaped protobuf message.
fn generic_carrier(
    value: &PlannedType,
    wire_type: &PlannedType,
) -> Option<(String, ProtoGenericCarrier)> {
    let PlannedType::TypeParameter(parameter) = value.validation_type() else {
        return None;
    };
    proto_generic_carrier(wire_type)
        .map(|carrier| (csharp_type_parameter_name(&parameter.name), carrier))
}

fn generic_carrier_from_wire_expr(
    carrier: ProtoGenericCarrier,
    wire_type: &PlannedType,
    type_parameter: &str,
    source_expr: &str,
    support_namespace: Option<&str>,
) -> String {
    let (default_converter, suffix) = match carrier {
        ProtoGenericCarrier::Payload => ("ProtoExtensions.FromPayload", ""),
        ProtoGenericCarrier::Payloads => ("ProtoExtensions.FromPayloads", "[0]"),
    };
    let converter = qualify_dotnet_support_reference(
        dotnet_from_proto_converter(wire_type.validation_type()).unwrap_or(default_converter),
        support_namespace,
    );
    format!("{converter}<{type_parameter}>({source_expr}){suffix}")
}

fn generic_carrier_to_wire_expr(
    carrier: ProtoGenericCarrier,
    wire_type: &PlannedType,
    source_expr: &str,
    support_namespace: Option<&str>,
) -> String {
    let (default_converter, argument) = match carrier {
        ProtoGenericCarrier::Payload => ("ProtoExtensions.ToPayload", source_expr.to_string()),
        ProtoGenericCarrier::Payloads => (
            "ProtoExtensions.ToPayloads",
            format!("new object?[] {{ {source_expr} }}"),
        ),
    };
    let converter = qualify_dotnet_support_reference(
        dotnet_to_proto_converter(wire_type.validation_type()).unwrap_or(default_converter),
        support_namespace,
    );
    format!("{converter}({argument})")
}

/// Returns the closed generic C# name for a generated model, such as `Model<T1, T2>`.
fn model_type_name(model: &RecordSpec<PlannedFamily>, api_plan: &PlannedSpec) -> String {
    let base = csharp_type_name(&model.name);
    let parameters = api_plan.record_type_parameters(&model.full_name, Language::Dotnet);
    if parameters.is_empty() {
        base
    } else {
        format!(
            "{base}<{}>",
            parameters
                .iter()
                .map(|usage| csharp_type_parameter_name(&usage.parameter.name))
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

/// Returns the open generic C# name for a generated model, such as `Model<,>`.
fn open_model_type_name(model: &RecordSpec<PlannedFamily>, api_plan: &PlannedSpec) -> String {
    let base = csharp_type_name(&model.name);
    let parameter_count = api_plan
        .record_type_parameters(&model.full_name, Language::Dotnet)
        .len();
    if parameter_count == 0 {
        base
    } else {
        format!("{base}<{}>", ",".repeat(parameter_count - 1))
    }
}

fn variant_type_name(variant_type: &PlannedType, api_plan: &PlannedSpec) -> String {
    let PlannedType::Variant(variant) = variant_type.validation_type() else {
        panic!("oneof field should resolve to a variant");
    };
    let base = csharp_type_name(&variant.name);
    let parameters = api_plan.variant_type_parameters(&variant.full_name, Language::Dotnet);
    if parameters.is_empty() {
        base
    } else {
        format!(
            "{base}<{}>",
            parameters
                .iter()
                .map(|usage| csharp_type_parameter_name(&usage.parameter.name))
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

pub(crate) fn dotnet_message_type(model_type: &PlannedType) -> String {
    match model_type {
        PlannedType::External(ExternalTypeSpec::Proto(PlannedProtoType::Message(proto))) => proto
            .replacement
            .as_ref()
            .and_then(dotnet_replacement_type_name)
            .unwrap_or_else(|| dotnet_proto_type_name_for_info(&proto.proto)),
        PlannedType::Record(record) => csharp_type_name(&record.model_name),
        PlannedType::Resource(resource) => csharp_type_name(&resource.type_name),
        _ => panic!("dotnet message type should be model-shaped"),
    }
}

fn dotnet_message_type_for_proto_message(proto: &PlannedProtoMessageType) -> String {
    proto
        .replacement
        .as_ref()
        .and_then(dotnet_replacement_type_name)
        .unwrap_or_else(|| dotnet_proto_type_name_for_info(&proto.proto))
}

pub(crate) fn dotnet_replacement_type_name(replacement: &TypeReplacementSpec) -> Option<String> {
    replacement
        .type_name
        .for_language(Language::Dotnet)
        .map(str::to_string)
}

pub(crate) fn dotnet_proto_or_local_type(
    info: &PlannedProtoTypeInfo,
    local_name: Option<&str>,
) -> String {
    if info.file_name.is_some() {
        dotnet_proto_type_name_for_info(info)
    } else {
        csharp_type_name(local_name.unwrap_or(&info.full_name))
    }
}

pub(crate) fn dotnet_proto_type_name_for_message(model_type: &PlannedType) -> String {
    let PlannedType::External(ExternalTypeSpec::Proto(PlannedProtoType::Message(proto))) =
        model_type
    else {
        panic!("dotnet proto type name should receive a proto message");
    };
    dotnet_proto_type_name_for_info(&proto.proto)
}

pub(crate) fn dotnet_proto_type_name_for_info(info: &PlannedProtoTypeInfo) -> String {
    info.file_options
        .as_ref()
        .and_then(|options| options.csharp_namespace.as_deref())
        .filter(|namespace| !namespace.is_empty())
        .map(|namespace| format!("{namespace}.{}", dotnet_proto_relative_type_name(info)))
        .or_else(|| {
            info.type_name
                .for_language(Language::Dotnet)
                .map(str::to_string)
        })
        .unwrap_or_else(|| dotnet_proto_type_name_fallback(&info.full_name))
}

pub(crate) fn dotnet_to_proto_converter(model_type: &PlannedType) -> Option<&str> {
    let PlannedType::External(ExternalTypeSpec::Proto(PlannedProtoType::Message(proto))) =
        model_type
    else {
        return None;
    };
    proto
        .replacement
        .as_ref()
        .and_then(|replacement| replacement.to_proto.for_language(Language::Dotnet))
}

pub(crate) fn dotnet_from_proto_converter(model_type: &PlannedType) -> Option<&str> {
    let PlannedType::External(ExternalTypeSpec::Proto(PlannedProtoType::Message(proto))) =
        model_type
    else {
        return None;
    };
    proto
        .replacement
        .as_ref()
        .and_then(|replacement| replacement.from_proto.for_language(Language::Dotnet))
}

fn render_model_transfer_converter(
    backend: &ModelBackend,
    output: &mut String,
    model: &RecordSpec<PlannedFamily>,
    api_plan: &PlannedSpec,
    support_namespace: Option<&str>,
) {
    let raw_type = dotnet_proto_type_name_for_info(
        model
            .data
            .proto
            .as_ref()
            .expect("model to proto method requires proto backing"),
    );
    render_model_from_wire_method(output, model, api_plan, support_namespace, &raw_type);
    output.push_str("    internal ");
    output.push_str(&raw_type);
    output.push_str(" ToTransferType()\n    {\n");
    output.push_str("        var proto = new ");
    output.push_str(&raw_type);
    output.push_str("();\n");
    for (field_name, sourced_field, _source_expr) in model.sourced_fields() {
        output.push_str("        proto.");
        output.push_str(&csharp_type_name(field_name));
        output.push_str(" = ");
        let source_expr = crate::generator::dotnet::field_property_name(sourced_field);
        output.push_str(&backend.field_kind_to_wire_expr(
            &sourced_field.field_type,
            &source_expr,
            false,
            api_plan,
            support_namespace,
        ));
        output.push_str(";\n");
    }
    for (field_name, field) in model.model_fields() {
        render_field_to_proto_assignment(
            backend,
            output,
            model,
            field_name,
            field,
            api_plan,
            support_namespace,
        );
    }
    output.push_str("        return proto;\n");
    output.push_str("    }\n\n");
    let type_name = model_type_name(model, api_plan);
    output.push_str("    /// <summary>\n");
    output.push_str("    /// Converts this model to and from its generated transfer type.\n");
    output.push_str("    /// </summary>\n");
    if model.experimental {
        output.push_str("    /// <remarks>WARNING: This API is experimental and may change in the future.</remarks>\n");
    }
    output.push_str("    public sealed class TransferTypeConverter : Temporalio.Converters.ITemporalTransferTypeConverter\n    {\n");
    output.push_str("        /// <summary>Gets the generated transfer type.</summary>\n");
    output.push_str("        public System.Type TransferType => typeof(");
    output.push_str(&raw_type);
    output.push_str(");\n\n");
    output
        .push_str("        /// <summary>Converts a model value to its transfer type.</summary>\n");
    output.push_str(
        "        public object? ToTransferType(object? value) => value is null ? null : ((",
    );
    output.push_str(&type_name);
    output.push_str(")value).ToTransferType();\n\n");
    output
        .push_str("        /// <summary>Converts a transfer-type value to this model.</summary>\n");
    output.push_str("        public object? FromTransferType(object? transferType) => transferType is null ? null : ");
    output.push_str(&type_name);
    output.push_str(".FromTransferType((");
    output.push_str(&raw_type);
    output.push_str(")transferType);\n");
    output.push_str("    }\n\n");
}

fn render_model_from_wire_method(
    output: &mut String,
    model: &RecordSpec<PlannedFamily>,
    api_plan: &PlannedSpec,
    support_namespace: Option<&str>,
    raw_type: &str,
) {
    let type_name = model_type_name(model, api_plan);
    output.push_str("    internal static ");
    output.push_str(&type_name);
    output.push_str(" FromTransferType(");
    output.push_str(raw_type);
    output.push_str(" wire)\n    {\n");
    for (field_name, field) in model
        .fields
        .iter()
        .filter(|(_, field)| field.visibility != RecordFieldVisibility::Omitted)
    {
        render_field_from_wire_setup(
            output,
            model,
            field_name,
            field,
            raw_type,
            api_plan,
            support_namespace,
        );
    }
    let required_fields = model
        .model_fields()
        .filter(|(_, field)| field.required)
        .collect::<Vec<_>>();
    output.push_str("        return new ");
    output.push_str(&type_name);
    output.push('(');
    for (index, (field_name, field)) in required_fields.iter().enumerate() {
        if index > 0 {
            output.push_str(", ");
        }
        output.push_str(&field_from_wire_expr(
            model,
            field_name,
            field,
            &format!("wire.{}", csharp_type_name(field_name)),
            api_plan,
            support_namespace,
        ));
    }
    let mut has_constructor_argument = !required_fields.is_empty();
    for (field_name, field, _) in model.sourced_fields() {
        if has_constructor_argument {
            output.push_str(", ");
        }
        has_constructor_argument = true;
        output.push_str(&field_from_wire_expr(
            model,
            field_name,
            field,
            &format!("wire.{}", csharp_type_name(field_name)),
            api_plan,
            support_namespace,
        ));
    }
    output.push(')');
    let init_fields = model
        .fields
        .iter()
        .filter(|(_, field)| {
            !field.required || matches!(field.visibility, RecordFieldVisibility::Sourced { .. })
        })
        .filter(|(_, field)| {
            !matches!(
                field.visibility,
                RecordFieldVisibility::Omitted | RecordFieldVisibility::Sourced { .. }
            )
        })
        .collect::<Vec<_>>();
    if init_fields.is_empty() {
        output.push_str(";\n");
    } else {
        output.push_str("\n        {\n");
        for (field_name, field) in init_fields {
            output.push_str("            ");
            output.push_str(&field_property_name(field));
            output.push_str(" = ");
            output.push_str(&field_from_wire_expr(
                model,
                field_name,
                field,
                &format!("wire.{}", csharp_type_name(field_name)),
                api_plan,
                support_namespace,
            ));
            output.push_str(",\n");
        }
        output.push_str("        };\n");
    }
    output.push_str("    }\n\n");
}

fn field_from_wire_expr(
    model: &RecordSpec<PlannedFamily>,
    field_name: &str,
    field: &RecordFieldSpec<PlannedFamily>,
    source_expr: &str,
    api_plan: &PlannedSpec,
    support_namespace: Option<&str>,
) -> String {
    let optional = !field.required;
    if function_args_field_uses_logical_storage(model, field_name, field) {
        let converter = function_args_from_proto_converter(field)
            .map(|converter| qualify_dotnet_support_reference(converter, support_namespace))
            .unwrap_or_else(|| {
                panic!(
                    "function args field `{}` missing .NET from-proto converter",
                    field_name
                )
            });
        return optional_message_from_wire_expr(
            source_expr,
            &format!("{converter}({{value}})"),
            optional,
        );
    }
    match &field.data.wire_binding {
        Some(PlannedWireFieldBinding::VariantMembers { .. }) => {
            // The setup statements decode the oneof into this local.
            return oneof_local_name(field_name);
        }
        Some(PlannedWireFieldBinding::Value { wire_type, .. }) => {
            if let Some((type_parameter, carrier)) = generic_carrier(&field.field_type, wire_type) {
                let converted = generic_carrier_from_wire_expr(
                    carrier,
                    wire_type,
                    &type_parameter,
                    source_expr,
                    support_namespace,
                );
                // A required carrier is checked for presence by the setup statements.
                return if optional {
                    format!("{source_expr} == null ? default : {converted}")
                } else {
                    converted
                };
            }
        }
        None => {}
    }
    value_from_wire_expr(
        &field.field_type,
        source_expr,
        optional,
        field.data.has_presence,
        api_plan,
        support_namespace,
    )
}

fn missing_required_field_message(model: &RecordSpec<PlannedFamily>, field_name: &str) -> String {
    format!(
        "\"missing required field {}.{}\"",
        csharp_type_name(&model.name),
        csharp_type_name(field_name)
    )
}

fn oneof_local_name(field_name: &str) -> String {
    format!("{}Oneof", field_name.to_lower_camel_case())
}

/// Renders the statements that `FromTransferType` runs before it constructs the model: a
/// presence check for each required Payload carrier, a count check for each Payloads
/// carrier, and a switch that decodes each oneof into a local.
fn render_field_from_wire_setup(
    output: &mut String,
    model: &RecordSpec<PlannedFamily>,
    field_name: &str,
    field: &RecordFieldSpec<PlannedFamily>,
    raw_type: &str,
    api_plan: &PlannedSpec,
    support_namespace: Option<&str>,
) {
    match &field.data.wire_binding {
        Some(PlannedWireFieldBinding::VariantMembers { wire_name, members }) => {
            render_oneof_from_wire_setup(
                output,
                model,
                field_name,
                field,
                wire_name,
                members,
                raw_type,
                api_plan,
                support_namespace,
            );
        }
        Some(PlannedWireFieldBinding::Value { wire_type, .. }) => {
            let Some((_, carrier)) = generic_carrier(&field.field_type, wire_type) else {
                return;
            };
            let source_expr = format!("wire.{}", csharp_type_name(field_name));
            if field.required {
                output.push_str(&format!(
                    "        if ({source_expr} == null)\n        {{\n            throw new System.InvalidOperationException({});\n        }}\n\n",
                    missing_required_field_message(model, field_name)
                ));
            }
            if matches!(carrier, ProtoGenericCarrier::Payloads) {
                render_single_payload_check(
                    output,
                    "        ",
                    &source_expr,
                    !field.required,
                    &format!(
                        "{}.{}",
                        csharp_type_name(&model.name),
                        csharp_type_name(field_name)
                    ),
                );
                output.push('\n');
            }
        }
        None => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn render_oneof_from_wire_setup(
    output: &mut String,
    model: &RecordSpec<PlannedFamily>,
    field_name: &str,
    field: &RecordFieldSpec<PlannedFamily>,
    wire_name: &str,
    members: &[PlannedWireVariantMember],
    raw_type: &str,
    api_plan: &PlannedSpec,
    support_namespace: Option<&str>,
) {
    let proto = model
        .data
        .proto
        .as_ref()
        .expect("oneof field requires proto backing");
    let cases = oneof_cases(api_plan, &proto.full_name, field, wire_name, members)
        .expect("oneof cases are validated during prepare");
    let variant_type = variant_type_name(&field.field_type, api_plan);
    let local_name = oneof_local_name(field_name);
    let oneof_case_type = format!("{raw_type}.{}OneofCase", csharp_type_name(wire_name));
    output.push_str(&format!(
        "        {variant_type}{} {local_name};\n",
        if field.required { "" } else { "?" }
    ));
    output.push_str(&format!(
        "        switch (wire.{}Case)\n        {{\n",
        csharp_type_name(wire_name)
    ));
    for case in &cases {
        let member_expr = format!("wire.{}", csharp_type_name(&case.member.wire_name));
        let value_expr = match generic_carrier(case.payload, &case.member.wire_type) {
            Some((type_parameter, carrier)) => generic_carrier_from_wire_expr(
                carrier,
                &case.member.wire_type,
                &type_parameter,
                &member_expr,
                support_namespace,
            ),
            None => value_from_wire_expr(
                case.payload,
                &member_expr,
                false,
                None,
                api_plan,
                support_namespace,
            ),
        };
        output.push_str(&format!(
            "            case {oneof_case_type}.{}:\n",
            csharp_type_name(&case.member.wire_name)
        ));
        if let Some((_, ProtoGenericCarrier::Payloads)) =
            generic_carrier(case.payload, &case.member.wire_type)
        {
            render_single_payload_check(
                output,
                "                ",
                &member_expr,
                false,
                &format!(
                    "{}.{}",
                    csharp_type_name(&model.name),
                    csharp_type_name(&case.member.wire_name)
                ),
            );
        }
        output.push_str(&format!(
            "                {local_name} = new {variant_type}.{}({value_expr});\n",
            case.case_name
        ));
        output.push_str("                break;\n");
    }
    output.push_str("            default:\n");
    if field.required {
        output.push_str(&format!(
            "                throw new System.InvalidOperationException({});\n",
            missing_required_field_message(model, field_name)
        ));
    } else {
        output.push_str(&format!("                {local_name} = null;\n"));
        output.push_str("                break;\n");
    }
    output.push_str("        }\n\n");
}

/// Renders a check that a Payloads carrier holds exactly one payload. The decode takes the
/// first payload, so this check replaces an unclear index error with a message that names
/// the field.
fn render_single_payload_check(
    output: &mut String,
    indent: &str,
    source_expr: &str,
    optional: bool,
    field_description: &str,
) {
    let count_expr = format!("{source_expr}.Payloads_.Count");
    let condition = if optional {
        format!("{source_expr} != null && {count_expr} != 1")
    } else {
        format!("{count_expr} != 1")
    };
    output.push_str(&format!(
        "{indent}if ({condition})\n{indent}{{\n{indent}    throw new System.InvalidOperationException($\"expected exactly one payload in {field_description}, found {{{count_expr}}}\");\n{indent}}}\n"
    ));
}

fn value_from_wire_expr(
    value: &PlannedType,
    source_expr: &str,
    optional: bool,
    has_presence: Option<bool>,
    api_plan: &PlannedSpec,
    support_namespace: Option<&str>,
) -> String {
    match value {
        PlannedType::Bool => optional_scalar_from_wire_expr(
            source_expr,
            source_expr,
            optional,
            has_presence,
            "false",
        ),
        PlannedType::Int(_) => {
            optional_scalar_from_wire_expr(source_expr, source_expr, optional, has_presence, "0")
        }
        PlannedType::Float => {
            optional_scalar_from_wire_expr(source_expr, source_expr, optional, has_presence, "0")
        }
        PlannedType::String => {
            if optional {
                if has_presence == Some(true) {
                    optional_presence_from_wire_expr(source_expr, source_expr)
                } else {
                    format!("string.IsNullOrEmpty({source_expr}) ? null : {source_expr}")
                }
            } else {
                source_expr.to_string()
            }
        }
        PlannedType::Bytes => source_expr.to_string(),
        PlannedType::Enum(_)
        | PlannedType::External(ExternalTypeSpec::Proto(PlannedProtoType::Enum(_))) => {
            if optional {
                if has_presence == Some(true) {
                    optional_presence_from_wire_expr(source_expr, source_expr)
                } else {
                    format!("(int){source_expr} == 0 ? null : {source_expr}")
                }
            } else {
                source_expr.to_string()
            }
        }
        PlannedType::External(ExternalTypeSpec::Proto(PlannedProtoType::Message(proto))) => {
            proto_message_from_wire_expr(proto, source_expr, optional, support_namespace)
        }
        PlannedType::Record(record) => {
            let model_name = api_plan
                .record(&record.full_name)
                .map(|record| model_type_name(record, api_plan))
                .unwrap_or_else(|| csharp_type_name(&record.model_name));
            optional_message_from_wire_expr(
                source_expr,
                &format!("{model_name}.FromTransferType({{value}})"),
                optional,
            )
        }
        PlannedType::External(ExternalTypeSpec::Alias(AliasTypeSpec { target, .. })) => {
            value_from_wire_expr(
                target,
                source_expr,
                optional,
                has_presence,
                api_plan,
                support_namespace,
            )
        }
        _ => source_expr.to_string(),
    }
}

fn proto_message_from_wire_expr(
    proto: &PlannedProtoMessageType,
    source_expr: &str,
    optional: bool,
    support_namespace: Option<&str>,
) -> String {
    let conversion = proto
        .replacement
        .as_ref()
        .and_then(|replacement| replacement.from_proto.for_language(Language::Dotnet))
        .map(|converter| {
            let converter = qualify_dotnet_support_reference(converter, support_namespace);
            format!("{converter}({{value}})")
        })
        .unwrap_or_else(|| "{value}".to_string());
    optional_message_from_wire_expr(source_expr, &conversion, optional)
}

fn optional_message_from_wire_expr(source_expr: &str, conversion: &str, optional: bool) -> String {
    let converted = conversion.replace("{value}", source_expr);
    if optional {
        format!("{source_expr} == null ? null : {converted}")
    } else {
        converted
    }
}

fn optional_scalar_from_wire_expr(
    source_expr: &str,
    converted_expr: &str,
    optional: bool,
    has_presence: Option<bool>,
    default_expr: &str,
) -> String {
    if !optional {
        return converted_expr.to_string();
    }
    if has_presence == Some(true) {
        optional_presence_from_wire_expr(source_expr, converted_expr)
    } else {
        format!("{source_expr} == {default_expr} ? null : {converted_expr}")
    }
}

fn optional_presence_from_wire_expr(source_expr: &str, converted_expr: &str) -> String {
    let Some((prefix, property_name)) = source_expr.rsplit_once('.') else {
        return converted_expr.to_string();
    };
    format!(
        "{}.Has{} ? {} : null",
        prefix,
        csharp_type_name(property_name),
        converted_expr
    )
}

fn render_field_to_proto_assignment(
    backend: &ModelBackend,
    output: &mut String,
    model: &RecordSpec<PlannedFamily>,
    field_name: &str,
    field: &RecordFieldSpec<PlannedFamily>,
    api_plan: &PlannedSpec,
    support_namespace: Option<&str>,
) {
    let property_name = field_property_name(field);
    let source_expr = property_name.to_string();
    if let Some(PlannedWireFieldBinding::VariantMembers { wire_name, members }) =
        &field.data.wire_binding
    {
        render_oneof_to_proto_assignment(
            backend,
            output,
            model,
            field_name,
            field,
            wire_name,
            members,
            &source_expr,
            api_plan,
            support_namespace,
        );
        return;
    }
    let target = format!("proto.{}", csharp_type_name(field_name));
    if field.required {
        output.push_str("        ");
        output.push_str(&target);
        output.push_str(" = ");
        output.push_str(&field_to_proto_expr(
            backend,
            model,
            field_name,
            field,
            &source_expr,
            api_plan,
            support_namespace,
        ));
        output.push_str(";\n");
    } else {
        output.push_str("        if (");
        output.push_str(&source_expr);
        output.push_str(" is { } ");
        output.push_str(&csharp_parameter_name(&field.name));
        output.push_str(")\n        {\n");
        output.push_str("            ");
        output.push_str(&target);
        output.push_str(" = ");
        output.push_str(&field_to_proto_expr(
            backend,
            model,
            field_name,
            field,
            &csharp_parameter_name(&field.name),
            api_plan,
            support_namespace,
        ));
        output.push_str(";\n");
        output.push_str("        }\n");
    }
}

#[allow(clippy::too_many_arguments)]
fn render_oneof_to_proto_assignment(
    backend: &ModelBackend,
    output: &mut String,
    model: &RecordSpec<PlannedFamily>,
    field_name: &str,
    field: &RecordFieldSpec<PlannedFamily>,
    wire_name: &str,
    members: &[PlannedWireVariantMember],
    source_expr: &str,
    api_plan: &PlannedSpec,
    support_namespace: Option<&str>,
) {
    let proto = model
        .data
        .proto
        .as_ref()
        .expect("oneof field requires proto backing");
    let cases = oneof_cases(api_plan, &proto.full_name, field, wire_name, members)
        .expect("oneof cases are validated during prepare");
    let variant_type = variant_type_name(&field.field_type, api_plan);
    output.push_str("        switch (");
    output.push_str(source_expr);
    output.push_str(")\n        {\n");
    for case in &cases {
        let case_variable = format!("{}Case", case.case_name.to_lower_camel_case());
        let value_expr = format!("{case_variable}.Value");
        let converted = match generic_carrier(case.payload, &case.member.wire_type) {
            Some((_, carrier)) => generic_carrier_to_wire_expr(
                carrier,
                &case.member.wire_type,
                &value_expr,
                support_namespace,
            ),
            None => backend.field_kind_to_wire_expr(
                case.payload,
                &value_expr,
                false,
                api_plan,
                support_namespace,
            ),
        };
        output.push_str(&format!(
            "            case {variant_type}.{} {case_variable}:\n",
            case.case_name
        ));
        output.push_str(&format!(
            "                proto.{} = {converted};\n",
            csharp_type_name(&case.member.wire_name)
        ));
        output.push_str("                break;\n");
    }
    // A required oneof has no null value. A forced null (for example `null!`) must not
    // encode as an unset oneof.
    if field.required {
        output.push_str("            default:\n");
        output.push_str(&format!(
            "                throw new System.InvalidOperationException({});\n",
            missing_required_field_message(model, field_name)
        ));
    }
    output.push_str("        }\n");
}

fn field_to_proto_expr(
    backend: &ModelBackend,
    model: &RecordSpec<PlannedFamily>,
    field_name: &str,
    field: &RecordFieldSpec<PlannedFamily>,
    source_expr: &str,
    api_plan: &PlannedSpec,
    support_namespace: Option<&str>,
) -> String {
    if function_args_field_uses_logical_storage(model, field_name, field) {
        let converter = function_args_to_proto_converter(field)
            .map(|converter| qualify_dotnet_support_reference(converter, support_namespace))
            .unwrap_or_else(|| {
                panic!(
                    "function args field `{}` missing .NET to-proto converter",
                    field_name
                )
            });
        return format!("{converter}({source_expr})");
    }
    if let Some(PlannedWireFieldBinding::Value { wire_type, .. }) = &field.data.wire_binding
        && let Some((_, carrier)) = generic_carrier(&field.field_type, wire_type)
    {
        return generic_carrier_to_wire_expr(carrier, wire_type, source_expr, support_namespace);
    }
    backend.field_kind_to_wire_expr(
        &field.field_type,
        source_expr,
        !field.required,
        api_plan,
        support_namespace,
    )
}

fn function_args_field_uses_logical_storage(
    model: &RecordSpec<PlannedFamily>,
    field_name: &str,
    field: &RecordFieldSpec<PlannedFamily>,
) -> bool {
    model.function_for_args_field(field_name).is_some()
        && function_args_field_stores_proto(field)
        && function_args_parameter_type(
            model,
            field_name,
            ModelBackend::default().function_args_authored_type(field),
        )
        .is_some()
}

fn function_args_field_stores_proto(field: &RecordFieldSpec<PlannedFamily>) -> bool {
    matches!(
        &field.field_type,
        PlannedType::External(ExternalTypeSpec::Proto(PlannedProtoType::Message(_)))
    )
}

fn function_args_to_proto_converter(field: &RecordFieldSpec<PlannedFamily>) -> Option<&str> {
    match &field.field_type {
        model_type @ PlannedType::External(ExternalTypeSpec::Proto(PlannedProtoType::Message(
            _,
        ))) => dotnet_to_proto_converter(model_type),
        _ => None,
    }
}

fn function_args_from_proto_converter(field: &RecordFieldSpec<PlannedFamily>) -> Option<&str> {
    match &field.field_type {
        model_type @ PlannedType::External(ExternalTypeSpec::Proto(PlannedProtoType::Message(
            _,
        ))) => dotnet_from_proto_converter(model_type),
        _ => None,
    }
}

fn dotnet_proto_relative_type_name(info: &PlannedProtoTypeInfo) -> String {
    let relative_name = info
        .full_name
        .strip_prefix(&format!("{}.", info.package))
        .unwrap_or(&info.full_name);
    let mut parts = relative_name.split('.');
    let Some(first) = parts.next() else {
        return String::new();
    };
    let mut type_name = csharp_type_name(first);
    for part in parts {
        type_name.push_str(".Types.");
        type_name.push_str(&csharp_type_name(part));
    }
    type_name
}

pub(crate) fn dotnet_proto_type_name_fallback(full_name: &str) -> String {
    full_name
        .split('.')
        .map(csharp_type_name)
        .collect::<Vec<_>>()
        .join(".")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use prost_types::FileOptions;

    use super::{dotnet_proto_type_name_fallback, dotnet_proto_type_name_for_info};
    use crate::language::Language;
    use crate::planning::PlannedProtoTypeInfo;
    use crate::spec::LanguageStringSpec;

    #[test]
    fn proto_type_name_fallback_pascal_cases_dotted_parts() {
        assert_eq!(
            dotnet_proto_type_name_fallback("acme.foo.v1.LocalRetryPolicy"),
            "Acme.Foo.V1.LocalRetryPolicy"
        );
        assert_eq!(
            dotnet_proto_type_name_fallback("company.widgets.v1.Widget"),
            "Company.Widgets.V1.Widget"
        );
    }

    #[test]
    fn proto_type_name_uses_csharp_namespace_file_option() {
        let info = PlannedProtoTypeInfo {
            full_name: "temporal.api.workflow.v1.VersioningOverride.PinnedOverride".to_string(),
            package: "temporal.api.workflow.v1".to_string(),
            file_name: Some("temporal/api/workflow/v1/message.proto".to_string()),
            file_options: Some(FileOptions {
                csharp_namespace: Some("Temporalio.Api.Workflow.V1".to_string()),
                ..Default::default()
            }),
            reference: LanguageStringSpec::default(),
            type_name: LanguageStringSpec::default(),
        };

        assert_eq!(
            dotnet_proto_type_name_for_info(&info),
            "Temporalio.Api.Workflow.V1.VersioningOverride.Types.PinnedOverride"
        );
    }

    #[test]
    fn proto_type_name_prefers_csharp_namespace_file_option_over_wit_override() {
        let info = PlannedProtoTypeInfo {
            full_name: "temporal.api.common.v1.Payload".to_string(),
            package: "temporal.api.common.v1".to_string(),
            file_name: Some("temporal/api/common/v1/message.proto".to_string()),
            file_options: Some(FileOptions {
                csharp_namespace: Some("Temporalio.Api.Common.V1".to_string()),
                ..Default::default()
            }),
            reference: LanguageStringSpec::default(),
            type_name: LanguageStringSpec {
                default: None,
                by_language: BTreeMap::from([(
                    Language::Dotnet,
                    "Should.Not.Be.Used.Payload".to_string(),
                )]),
                default_import: None,
                imports: BTreeMap::new(),
            },
        };

        assert_eq!(
            dotnet_proto_type_name_for_info(&info),
            "Temporalio.Api.Common.V1.Payload"
        );
    }
}
