use std::cell::{Ref, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use indexmap::IndexMap;

use crate::error::{Error, Result};
use crate::generator::ExternalModelBackend;
use crate::language::Language;
use crate::planning::ResolvedResourceBindingSource;
use crate::planning::{
    PlannedFamily, PlannedProtoType, PlannedResource, PlannedResourceField, PlannedSpec,
    PlannedType, PlannedWireFieldBinding,
};
use crate::spec::{ExternalTypeSpec, RecordFieldSpec, RecordFieldVisibility, RecordSpec};

use crate::generator::go::{
    GoPackageContext, PlannedEnumType, PlannedFieldKind, PlannedMessageSource, PlannedMessageType,
    PlannedOperationOutput, PlannedOperationResourceReturn, PlannedScalarType, PlannedTypeInfo,
    PlannedValueType, RenderedModel, RenderedService, go_authored_type_annotation, go_field_name,
    go_replacement_type_name, go_string_literal, go_unexported_name, new_nexus_client_expr,
    operation_output, planned_field, planned_field_kind, planned_message_type,
    public_default_punning_zero_for_field, record_for_model_key, render_operation_future_adapter,
    render_operation_future_return_type, resolve_resource_field_kind, split_go_type_decl_name,
};

#[derive(Debug, Default)]
pub(in crate::generator) struct ModelFragments;

/// Describes how a single value (a field element, map value, etc.) converts
/// between its native Go representation and its proto representation.
///
/// Each direction is expressed as a closure over an input expression so the
/// same logic can be reused for singular fields, slice elements, and map
/// values. Conversions never touch raw bytes -- they translate between the
/// native value and a Go proto struct that the Temporal SDK serializes.
pub(in crate::generator) struct GoValueConversion {
    /// The structural shape of the conversion, which determines how the line
    /// builders handle pointers and dereferencing.
    pub(in crate::generator) kind: GoConversionKind,
    /// Produces the native expression from a proto expression.
    pub(in crate::generator) from_proto: Box<dyn Fn(&str) -> String>,
    /// Produces the proto expression from a native expression.
    pub(in crate::generator) to_proto: Box<dyn Fn(&str) -> String>,
    /// Whether the conversion expression returns `(value, error)`.
    pub(in crate::generator) fallible: bool,
    /// Whether the native argument to `to_proto` is pointer-shaped.
    pub(in crate::generator) to_proto_takes_pointer: bool,
    /// Whether the native result from `from_proto` is pointer-shaped.
    pub(in crate::generator) from_proto_returns_pointer: bool,
}

/// Classifies a value conversion so the line builders know how to bridge
/// pointer/value mismatches between the native field and the converter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::generator) enum GoConversionKind {
    /// No conversion: native and proto share the same scalar Go type. The
    /// `from_proto`/`to_proto` closures are the identity.
    Scalar,
    /// A cast between a named Go type and a proto enum type (value to value),
    /// e.g. `enums.WorkflowIdReusePolicy(x)`.
    Enum,
    /// A hand-written override converter pair that is pointer-based on both the
    /// native and proto sides: `FromProto(*Proto) *Native` and
    /// `ToProto(*Native) *Proto`, each returning `nil` for `nil` input. The
    /// converter owns nil passthrough; the caller supplies/consumes pointers.
    OverrideConverter,
    /// A generated model converter: `FromProto(*Proto) Model` (value result)
    /// and `(Model) toProto(ctx) *Proto` (value receiver). The native side is a
    /// value, the proto side is a pointer.
    ModelConverter,
}

/// The result of building a Go proto conversion: either the conversion, or a
/// human-readable reason why the value cannot be converted. Callers attach
/// model/field context and fail generation -- unconvertible values must never
/// be silently omitted from the wire format.
pub(in crate::generator) type GoConversionResult<T> = std::result::Result<T, String>;

#[derive(Debug)]
pub(in crate::generator) struct ModelBackend {
    package: GoPackageContext,
    imports: RefCell<GoImportCollector>,
    proto_models: BTreeMap<String, PlannedTypeInfo>,
    wire_models: RefCell<BTreeMap<String, RenderedModelWire>>,
}

impl ModelBackend {
    pub(in crate::generator) fn new(package: GoPackageContext) -> Self {
        Self {
            package,
            imports: RefCell::new(GoImportCollector::default()),
            proto_models: BTreeMap::new(),
            wire_models: RefCell::new(BTreeMap::new()),
        }
    }

    pub(in crate::generator) fn imports(&self) -> Ref<'_, GoImportCollector> {
        self.imports.borrow()
    }

    pub(in crate::generator) fn message_proto_type(
        &self,
        message: &PlannedMessageType,
    ) -> Option<String> {
        self.proto_type(&message.info)
            .map(|type_name| format!("*{type_name}"))
    }

    pub(in crate::generator) fn has_message_wire_type(&self, message: &PlannedMessageType) -> bool {
        descriptor_has_go_package(&message.info)
    }

    pub(in crate::generator) fn value_proto_type(
        &self,
        value: &PlannedValueType,
    ) -> GoConversionResult<String> {
        match value {
            PlannedValueType::Scalar(scalar) => Ok(go_scalar_type_expr(scalar).to_string()),
            PlannedValueType::Enum(enum_type) => {
                let info = enum_type
                    .info
                    .as_ref()
                    .ok_or_else(|| "the enum has no proto descriptor info".to_string())?;
                self.proto_type(info).ok_or_else(|| {
                    format!(
                        "proto enum `{}` has no `go_package` option in its descriptor",
                        info.full_name
                    )
                })
            }
            PlannedValueType::Message(message) => {
                self.message_proto_type(message).ok_or_else(|| {
                    format!(
                        "proto message `{}` has no `go_package` option in its descriptor",
                        message.info.full_name
                    )
                })
            }
            _ => Err("the value has no proto-side Go type".to_string()),
        }
    }

    fn proto_type(&self, info: &PlannedTypeInfo) -> Option<String> {
        let proto_ref = go_proto_ref(info)?;
        let alias = self.imports.borrow_mut().register(&proto_ref);
        Some(proto_ref.qualified(&alias))
    }

    pub(in crate::generator) fn record_model(&mut self, key: &str, message: &PlannedMessageType) {
        if let Some(info) = proto_backed_info(message) {
            self.proto_models.insert(key.to_string(), info);
        }
    }

    pub(in crate::generator) fn model_proto_info(&self, key: &str) -> Option<PlannedTypeInfo> {
        self.proto_models.get(key).cloned()
    }

    pub(in crate::generator) fn set_model_wire(&self, key: String, wire: RenderedModelWire) {
        self.wire_models.borrow_mut().insert(key, wire);
    }

    pub(in crate::generator) fn render_model_wire_methods(
        &self,
        output: &mut String,
        key: &str,
        model: &RenderedModel,
    ) {
        let wire_models = self.wire_models.borrow();
        let Some(wire) = wire_models.get(key) else {
            return;
        };
        render_model_wire_methods(output, model, wire, &self.package);
    }
}

impl ExternalModelBackend<PlannedValueType> for ModelBackend {
    type ModelFragments = ModelFragments;
    type WireConversion = GoConversionResult<GoValueConversion>;

    fn prepare(&mut self, _api_plan: &PlannedSpec) -> Result<()> {
        Ok(())
    }

    fn render_models(&self) -> Result<ModelFragments> {
        Ok(ModelFragments)
    }

    fn render_support_files(&self) -> Result<BTreeMap<PathBuf, String>> {
        Ok(BTreeMap::new())
    }

    fn model_type_annotation(&self, model_type: &PlannedValueType) -> Option<String> {
        match model_type {
            PlannedValueType::Enum(enum_type) => enum_type
                .replacement
                .as_ref()
                .and_then(|replacement| go_replacement_type_name(replacement, &self.package))
                .or_else(|| enum_type.name.clone()),
            PlannedValueType::Message(message) => {
                if let Some(replacement) = &message.replacement {
                    return go_replacement_type_name(replacement, &self.package);
                }
                if let Some(authored_type) = &message.authored_type {
                    return Some(go_authored_type_annotation(authored_type));
                }
                Some(message.model_name.clone())
            }
            _ => None,
        }
    }

    fn wire_type_identifier(&self, model_type: &PlannedValueType) -> Option<String> {
        match model_type {
            PlannedValueType::Message(message) => Some(message.info.full_name.clone()),
            PlannedValueType::Enum(_) => None,
            _ => None,
        }
    }

    fn wire_conversion(
        &self,
        model_type: &PlannedValueType,
        _planned_record: Option<&RecordSpec<PlannedFamily>>,
    ) -> Option<GoConversionResult<GoValueConversion>> {
        match model_type {
            PlannedValueType::Scalar(_) => Some(Ok(GoValueConversion {
                kind: GoConversionKind::Scalar,
                from_proto: Box::new(|expr| expr.to_string()),
                to_proto: Box::new(|expr| expr.to_string()),
                fallible: false,
                to_proto_takes_pointer: false,
                from_proto_returns_pointer: false,
            })),
            PlannedValueType::Enum(enum_type) => Some(go_enum_conversion(enum_type, self)),
            PlannedValueType::Message(message_type) => {
                Some(go_message_conversion(message_type, &self.package))
            }
            _ => None,
        }
    }
}

/// Collects and de-duplicates Go proto imports, assigning a unique alias to
/// each import path. When two distinct import paths request the same alias
/// (e.g. two `v1` packages both aliased `enums`), later ones are suffixed with
/// a numeric disambiguator.
#[derive(Debug, Default)]
pub(in crate::generator) struct GoImportCollector {
    by_path: IndexMap<String, String>,
    used_aliases: BTreeSet<String>,
}

impl GoImportCollector {
    fn register(&mut self, proto_ref: &GoProtoRef) -> String {
        if let Some(existing) = self.by_path.get(&proto_ref.import_path) {
            return existing.clone();
        }
        let mut alias = proto_ref.alias.clone();
        if self.used_aliases.contains(&alias) {
            let mut counter = 2;
            loop {
                let candidate = format!("{}{counter}", proto_ref.alias);
                if !self.used_aliases.contains(&candidate) {
                    alias = candidate;
                    break;
                }
                counter += 1;
            }
        }
        self.used_aliases.insert(alias.clone());
        self.by_path
            .insert(proto_ref.import_path.clone(), alias.clone());
        alias
    }

    pub(in crate::generator) fn paths(&self) -> impl Iterator<Item = (&String, &String)> {
        self.by_path.iter()
    }
}

/// A reference to a Go protobuf type, derived from a proto descriptor's
/// `go_package` file option and the type's fully-qualified name.
#[derive(Debug, Clone)]
struct GoProtoRef {
    import_path: String,
    alias: String,
    type_name: String,
}

impl GoProtoRef {
    fn qualified(&self, alias: &str) -> String {
        format!("{alias}.{}", self.type_name)
    }
}

fn go_proto_ref(info: &PlannedTypeInfo) -> Option<GoProtoRef> {
    let go_package = info
        .file_options
        .as_ref()
        .and_then(|options| options.go_package.as_deref())?;
    let (import_path, alias) = parse_go_package_option(go_package);
    let relative = if info.package.is_empty() {
        info.full_name.as_str()
    } else {
        info.full_name
            .strip_prefix(&format!("{}.", info.package))
            .unwrap_or(&info.full_name)
    };
    Some(GoProtoRef {
        import_path,
        alias,
        type_name: relative.replace('.', "_"),
    })
}

fn parse_go_package_option(go_package: &str) -> (String, String) {
    if let Some((path, alias)) = go_package.split_once(';') {
        (path.to_string(), alias.to_string())
    } else {
        let alias = go_package
            .rsplit('/')
            .next()
            .unwrap_or(go_package)
            .to_string();
        (go_package.to_string(), alias)
    }
}

fn go_scalar_type_expr(scalar: &PlannedScalarType) -> &'static str {
    match scalar {
        PlannedScalarType::Float => "float64",
        PlannedScalarType::Int32 => "int32",
        PlannedScalarType::Int64 => "int64",
        PlannedScalarType::Bool => "bool",
        PlannedScalarType::String => "string",
        PlannedScalarType::Bytes => "[]byte",
    }
}

fn go_enum_conversion(
    enum_type: &PlannedEnumType,
    backend: &ModelBackend,
) -> GoConversionResult<GoValueConversion> {
    let native_type = if let Some(replacement) = &enum_type.replacement {
        go_replacement_type_name(replacement, &backend.package).ok_or_else(|| {
            "the type-replaced enum has no `go=` annotation in its `@nexus.type` directive"
                .to_string()
        })?
    } else {
        enum_type
            .name
            .clone()
            .ok_or_else(|| "the enum has no generated Go name".to_string())?
    };

    let info = enum_type
        .info
        .as_ref()
        .ok_or_else(|| "the enum has no proto descriptor info".to_string())?;
    let proto_type = backend.proto_type(info).ok_or_else(|| {
        format!(
            "proto enum `{}` has no `go_package` option in its descriptor",
            info.full_name
        )
    })?;

    let native_for_cast = native_type.clone();
    Ok(GoValueConversion {
        kind: GoConversionKind::Enum,
        from_proto: Box::new(move |expr| format!("{native_for_cast}(int32({expr}))")),
        to_proto: Box::new(move |expr| format!("{proto_type}({expr})")),
        fallible: false,
        to_proto_takes_pointer: false,
        from_proto_returns_pointer: false,
    })
}

fn go_message_conversion(
    message: &PlannedMessageType,
    package: &GoPackageContext,
) -> GoConversionResult<GoValueConversion> {
    if let Some(replacement) = &message.replacement {
        if let Some(type_name) = go_replacement_type_name(replacement, package) {
            let from = go_from_proto_converter(&message.info.full_name, replacement);
            let to = go_to_proto_converter(&message.info.full_name, replacement);
            let native_is_nilable_value = crate::generator::go::go_type_is_nilable(&type_name);
            return Ok(GoValueConversion {
                kind: GoConversionKind::OverrideConverter,
                from_proto: Box::new(move |expr| format!("{from}(ctx, {expr})")),
                to_proto: Box::new(move |expr| format!("{to}(ctx, {expr})")),
                fallible: true,
                to_proto_takes_pointer: !native_is_nilable_value,
                from_proto_returns_pointer: !native_is_nilable_value,
            });
        }
    }

    if message.authored_type.is_some() {
        let from = go_default_from_proto_name(&message.info.full_name);
        let to = go_default_to_proto_name(&message.info.full_name);
        return Ok(GoValueConversion {
            kind: GoConversionKind::OverrideConverter,
            from_proto: Box::new(move |expr| format!("{from}(ctx, {expr})")),
            to_proto: Box::new(move |expr| format!("{to}(ctx, {expr})")),
            fallible: true,
            to_proto_takes_pointer: true,
            from_proto_returns_pointer: true,
        });
    }

    if message.replacement.is_some() {
        return Err(format!(
            "type-replaced message `{}` has no `go=` annotation in its `@nexus.type` directive",
            message.info.full_name
        ));
    }

    if message.source == PlannedMessageSource::Proto {
        let from_proto = format!("{}FromProto", go_unexported_name(&message.model_name));
        return Ok(GoValueConversion {
            kind: GoConversionKind::ModelConverter,
            from_proto: Box::new(move |expr| format!("{from_proto}(ctx, {expr})")),
            to_proto: Box::new(|expr| format!("{expr}.toProto(ctx)")),
            fallible: true,
            to_proto_takes_pointer: false,
            from_proto_returns_pointer: false,
        });
    }

    Err(format!(
        "WIT-native record `{}` has no proto conversion",
        message.model_name
    ))
}

fn leaf_lower_camel(full_name: &str) -> String {
    use heck::ToLowerCamelCase;
    full_name
        .rsplit('.')
        .next()
        .unwrap_or(full_name)
        .to_lower_camel_case()
}

fn go_default_from_proto_name(full_name: &str) -> String {
    format!("{}FromProto", leaf_lower_camel(full_name))
}

fn go_default_to_proto_name(full_name: &str) -> String {
    format!("{}ToProto", leaf_lower_camel(full_name))
}

fn go_from_proto_converter(
    full_name: &str,
    replacement: &crate::spec::TypeReplacementSpec,
) -> String {
    replacement
        .from_proto
        .for_language(Language::Go)
        .map(str::to_string)
        .unwrap_or_else(|| go_default_from_proto_name(full_name))
}

fn go_to_proto_converter(
    full_name: &str,
    replacement: &crate::spec::TypeReplacementSpec,
) -> String {
    replacement
        .to_proto
        .for_language(Language::Go)
        .map(str::to_string)
        .unwrap_or_else(|| go_default_to_proto_name(full_name))
}

/// Builds the [`GoValueConversion`] for a single planned value type,
/// registering any proto imports it requires.
fn go_value_conversion(
    value: &PlannedValueType,
    api_plan: &PlannedSpec,
    backend: &ModelBackend,
    package: &GoPackageContext,
) -> GoConversionResult<GoValueConversion> {
    let _ = (api_plan, package);
    backend
        .wire_conversion(value, None)
        .ok_or_else(|| match value {
            PlannedValueType::Flags(_) => "flags values have no Go proto conversion".to_string(),
            PlannedValueType::Variant(_) => {
                "variant values have no Go proto conversion".to_string()
            }
            PlannedValueType::Tuple(_) => "tuple values have no Go proto conversion".to_string(),
            PlannedValueType::Result { .. } => {
                "result values have no Go proto conversion".to_string()
            }
            PlannedValueType::External { .. } => {
                "externally-typed values (via `@nexus.type`) have no Go proto conversion"
                    .to_string()
            }
            PlannedValueType::Unknown => {
                "the proto type could not be resolved from the provided descriptors".to_string()
            }
            _ => "the value has no Go proto conversion".to_string(),
        })?
}

/// Computes the proto field accessor name on a Go proto struct from the WIT
/// proto field name (snake_case -> UpperCamelCase, e.g. `task_queue` ->
/// `TaskQueue`).
fn go_proto_field_name(proto_name: &str) -> String {
    use heck::ToUpperCamelCase;
    proto_name.to_upper_camel_case()
}

fn proto_backed_info(message: &PlannedMessageType) -> Option<PlannedTypeInfo> {
    if message.replacement.is_some() || message.authored_type.is_some() {
        return None;
    }
    if message.source != PlannedMessageSource::Proto {
        return None;
    }
    if message
        .info
        .file_options
        .as_ref()
        .and_then(|options| options.go_package.as_ref())
        .is_none()
    {
        return None;
    }
    Some(message.info.clone())
}

fn descriptor_has_go_package(info: &PlannedTypeInfo) -> bool {
    info.file_options
        .as_ref()
        .and_then(|options| options.go_package.as_ref())
        .is_some()
}

/// Proto serialization metadata for a rendered Go model.
#[derive(Debug)]
pub(in crate::generator) struct RenderedModelWire {
    /// The Go proto type expression (e.g. `"common.ActivityOptions"`) the
    /// model converts to/from, qualified with the resolved import alias.
    pub(in crate::generator) proto_type: String,
    /// Field conversions in rendered model field order.
    pub(in crate::generator) field_conversions: Vec<RenderedFieldConversion>,
    /// Sourced fields (write-only, value derived from a source expression)
    /// emitted in `ToProto` after the regular fields.
    pub(in crate::generator) sourced_fields: Vec<RenderedSourcedField>,
}

/// Per-field proto conversion metadata, carrying the lines/expressions needed
/// to populate the field in `ToProto` and read it in `FromProto`.
#[derive(Debug)]
pub(in crate::generator) struct RenderedFieldConversion {
    /// Lines that assign this field into the `message` proto value, given a
    /// receiver `m`.
    pub(in crate::generator) to_proto_lines: Vec<String>,
    /// Lines that compute this field from `proto`, binding the result to the
    /// struct field of the value under construction.
    pub(in crate::generator) from_proto_lines: Vec<String>,
}

/// A sourced field rendered for `ToProto`.
#[derive(Debug)]
pub(in crate::generator) struct RenderedSourcedField {
    pub(in crate::generator) to_proto_lines: Vec<String>,
}

/// Wire serialization binding for an operation's request and response,
/// allowing the generated operation function to convert native values to/from
/// proto before/after the SDK call.
#[derive(Debug)]
pub(in crate::generator) struct OperationBinding {
    /// Expression converting the native `request` value to its proto form.
    input_to_proto: String,
    /// Proto type expression for the response, or `None` for void operations.
    output_proto_type: Option<String>,
    /// Expression converting the proto response (bound to `&result`) to its
    /// native form. `None` for void operations.
    output_from_proto: Option<String>,
    /// Whether `output_from_proto` already evaluates to a pointer (override
    /// converters return `*Native`) or to a value that must be addressed
    /// (generated model converters return `Model`).
    output_returns_pointer: bool,
    /// Resource construction binding for proto-backed resource-returning
    /// operations.
    resource_return: Option<RenderedResourceReturn>,
}

#[derive(Debug)]
struct RenderedResourceReturn {
    resource_type_name: String,
    local_lines: Vec<String>,
    field_initializers: Vec<RenderedResourceFieldInitializer>,
}

#[derive(Debug)]
struct RenderedResourceFieldInitializer {
    expr: String,
}

impl OperationBinding {
    pub(in crate::generator) fn requires_fmt(&self) -> bool {
        self.output_returns_pointer
    }
    pub(in crate::generator) fn returned_resource_type_name(&self) -> Option<&str> {
        self.resource_return
            .as_ref()
            .map(|resource| resource.resource_type_name.as_str())
    }
}

impl ModelBackend {
    /// Populates backend wire conversion metadata for every rendered model this
    /// backend recognizes. Runs after all rendered types are collected and
    /// visibility is applied so conversion logic can reference final model
    /// names without threading backend state through type resolution.
    pub(in crate::generator) fn populate_model_wire_conversions(
        &self,
        api_plan: &PlannedSpec,
        models: &IndexMap<String, RenderedModel>,
    ) -> Result<()> {
        let full_names: Vec<String> = models.keys().cloned().collect();
        for full_name in full_names {
            let Some(proto_info) = self.model_proto_info(&full_name) else {
                continue;
            };

            let planned_record = record_for_model_key(api_plan, &full_name);
            if let Some(planned_record) = planned_record {
                validate_record_conversion(planned_record)?;
            }

            let proto_type = self
                .message_proto_type(&PlannedMessageType {
                    info: proto_info.clone(),
                    model_name: String::new(),
                    replacement: None,
                    authored_type: None,
                    source: PlannedMessageSource::Proto,
                })
                .ok_or_else(|| Error::UnsupportedGoProtoConversion {
                    context: format!("model `{}`", proto_info.full_name),
                    reason: "its proto descriptor has no `go_package` option".to_string(),
                })?;

            let native_field_types: Vec<String> = models
                .get(&full_name)
                .map(|model| model.fields.iter().map(|f| f.go_type.clone()).collect())
                .unwrap_or_default();
            let (field_conversions, sourced_fields) = match planned_record {
                Some(planned_model) => (
                    build_field_conversions(planned_model, &native_field_types, api_plan, self)?,
                    build_sourced_conversions(planned_model, api_plan, self)?,
                ),
                None => (Vec::new(), Vec::new()),
            };

            self.set_model_wire(
                full_name,
                RenderedModelWire {
                    proto_type,
                    field_conversions,
                    sourced_fields,
                },
            );
        }
        Ok(())
    }

    /// Populates wire bindings on operations whose input/output messages are
    /// handled by this backend, registering required imports.
    pub(in crate::generator) fn populate_operation_bindings(
        &self,
        api_plan: &PlannedSpec,
        services: &mut [RenderedService<'_>],
    ) -> Result<()> {
        for (service, planned_service) in services.iter_mut().zip(api_plan.services.iter()) {
            for (rendered_op, planned_op) in service
                .operations
                .iter_mut()
                .zip(planned_service.operations.iter())
            {
                let Some(input) = planned_op
                    .input
                    .as_ref()
                    .and_then(|input| planned_message_type(input, api_plan))
                else {
                    continue;
                };
                let output = operation_output(planned_op, api_plan);
                let Some((_input_wire_type, input_conv)) =
                    operation_message_binding(&input, &planned_op.name, "input", self)?
                else {
                    continue;
                };
                // Override converters take a pointer to the native value; generated
                // model converters use a value receiver (`request.toProto(ctx)`).
                let input_arg = match input_conv.kind {
                    GoConversionKind::OverrideConverter => "&request".to_string(),
                    _ => "request".to_string(),
                };
                let input_to_proto = (input_conv.to_proto)(&input_arg);
                let has_go_output_transform =
                    planned_op
                        .output_transform
                        .as_ref()
                        .is_some_and(|transform| {
                            transform.type_name.for_language(Language::Go).is_some()
                                && transform.transform.for_language(Language::Go).is_some()
                        });
                let resource_return = if let Some(resource_return) =
                    &planned_op.data.output_resource_return
                {
                    let resource = service
                        .resources
                        .iter()
                        .find(|resource| resource.type_name == resource_return.resource_type_name)
                        .ok_or_else(|| Error::InvalidResource {
                            service: planned_service.name.clone(),
                            resource: resource_return.resource_type_name.clone(),
                            reason: "resource-returning operation references an unknown resource"
                                .to_string(),
                        })?;
                    Some(build_rendered_resource_return(
                        planned_service.name.as_str(),
                        resource_return,
                        resource,
                        api_plan,
                        self,
                        &self.package,
                    )?)
                } else {
                    None
                };

                let (output_proto_type, output_from_proto, output_returns_pointer) = match &output {
                    PlannedOperationOutput::Message(output) => {
                        if planned_op.data.output_resource_return.is_some() {
                            let Some(proto_type) = operation_message_proto_type(output, self)
                            else {
                                continue;
                            };
                            (Some(proto_type), None, false)
                        } else if has_go_output_transform {
                            (operation_message_proto_type(output, self), None, false)
                        } else {
                            match operation_message_binding(
                                output,
                                &planned_op.name,
                                "output",
                                self,
                            )? {
                                Some((proto_type, conv)) => {
                                    // `result` is declared as a proto value;
                                    // converters take a pointer to the proto message.
                                    let from = (conv.from_proto)("&result");
                                    let returns_pointer = conv.from_proto_returns_pointer;
                                    (Some(proto_type), Some(from), returns_pointer)
                                }
                                None => continue,
                            }
                        }
                    }
                    PlannedOperationOutput::Resource { .. } => continue,
                    PlannedOperationOutput::None => (None, None, false),
                };

                rendered_op.wire_binding = Some(OperationBinding {
                    input_to_proto,
                    output_proto_type,
                    output_from_proto,
                    output_returns_pointer,
                    resource_return,
                });
            }
        }
        Ok(())
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
                    language: Language::Go,
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
                    language: Language::Go,
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

/// Renders an operation function that serializes its request to proto before
/// the SDK call and deserializes the proto response afterwards.
pub(in crate::generator) fn render_operation_function_proto(
    output: &mut String,
    service: &crate::generator::go::RenderedService<'_>,
    operation: &crate::generator::go::RenderedOperation<'_>,
    binding: &OperationBinding,
    package: &GoPackageContext,
) {
    let operation_name = go_string_literal(operation.wire_name);

    output.push_str("func ");
    output.push_str(&operation.func_name);
    output.push_str("(ctx ");
    output.push_str(&package.workflow_context_type());
    output.push_str(", request ");
    output.push_str(&operation.input_type);
    output.push_str(") ");
    render_operation_future_return_type(output, package);
    output.push_str(" {\n");

    output.push_str("\trequestProto, err := ");
    output.push_str(&binding.input_to_proto);
    output.push('\n');
    output.push_str("\tif err != nil {\n");
    output.push_str("\t\tresult, resultSettable := ");
    output.push_str(&package.new_future());
    output.push_str("(ctx)\n");
    output.push_str("\t\tresultSettable.SetError(err)\n");
    output.push_str("\t\treturn result\n");
    output.push_str("\t}\n");
    let endpoint = service
        .endpoint
        .as_deref()
        .expect("operations require endpoint");
    output.push_str("\tc := ");
    output.push_str(&new_nexus_client_expr(endpoint, service.wire_name, package));
    output.push('\n');
    output.push_str("\tfut := c.ExecuteOperation(ctx, ");
    output.push_str(&operation_name);
    output.push_str(", requestProto");
    output.push_str(", ");
    output.push_str(&package.nexus_operation_options());
    output.push_str(")\n");

    if let (Some(transform_expr), Some(transform_type)) = (
        operation.output_transform_expr,
        operation.output_transform_type.as_ref(),
    ) {
        render_operation_future_adapter(output, package, transform_type, false, |output| {
            if let Some(proto_value_type) = binding
                .output_proto_type
                .as_deref()
                .map(|value| value.trim_start_matches('*'))
            {
                output.push_str("\t\tvar result ");
                output.push_str(proto_value_type);
                output.push('\n');
                output.push_str("\t\tif err := fut.Get(ctx, &result); err != nil {\n");
            } else if let Some(raw_result_type) = &operation.raw_output_type {
                output.push_str("\t\tvar result ");
                output.push_str(raw_result_type);
                output.push('\n');
                output.push_str("\t\tif err := fut.Get(ctx, &result); err != nil {\n");
            } else {
                output.push_str("\t\tif err := fut.Get(ctx, nil); err != nil {\n");
            }
            output.push_str("\t\t\tresultSettable.SetError(err)\n");
            output.push_str("\t\t\treturn\n");
            output.push_str("\t\t}\n");
            output.push_str("\t\tvalue, err := ");
            output.push_str(transform_expr);
            output.push('\n');
            output.push_str("\t\tif err != nil {\n");
            output.push_str("\t\t\tresultSettable.SetError(err)\n");
            output.push_str("\t\t\treturn\n");
            output.push_str("\t\t}\n");
        });
    } else if let (Some(output_type), Some(proto_value_type), Some(from_proto)) = (
        operation.output_type.as_ref(),
        binding
            .output_proto_type
            .as_deref()
            .map(|value| value.trim_start_matches('*')),
        binding.output_from_proto.as_deref(),
    ) {
        render_operation_future_adapter(
            output,
            package,
            output_type,
            binding.output_returns_pointer,
            |output| {
                output.push_str("\t\tvar result ");
                output.push_str(proto_value_type);
                output.push('\n');
                output.push_str("\t\tif err := fut.Get(ctx, &result); err != nil {\n");
                output.push_str("\t\t\tresultSettable.SetError(err)\n");
                output.push_str("\t\t\treturn\n");
                output.push_str("\t\t}\n");
                output.push_str("\t\tvalue, err := ");
                output.push_str(from_proto);
                output.push('\n');
                output.push_str("\t\tif err != nil {\n");
                output.push_str("\t\t\tresultSettable.SetError(err)\n");
                output.push_str("\t\t\treturn\n");
                output.push_str("\t\t}\n");
            },
        );
    } else if let (Some(resource_return), Some(proto_value_type)) = (
        binding.resource_return.as_ref(),
        binding
            .output_proto_type
            .as_deref()
            .map(|value| value.trim_start_matches('*')),
    ) {
        render_operation_future_adapter(
            output,
            package,
            &resource_return.resource_type_name,
            false,
            |output| {
                output.push_str("\t\tvar result ");
                output.push_str(proto_value_type);
                output.push('\n');
                output.push_str("\t\tif err := fut.Get(ctx, &result); err != nil {\n");
                output.push_str("\t\t\tresultSettable.SetError(err)\n");
                output.push_str("\t\t\treturn\n");
                output.push_str("\t\t}\n");
                for line in &resource_return.local_lines {
                    output.push_str("\t\t");
                    output.push_str(line);
                    output.push('\n');
                }
                output.push_str("\t\tvalue := New");
                output.push_str(&resource_return.resource_type_name);
                output.push('(');
                for (index, initializer) in resource_return.field_initializers.iter().enumerate() {
                    if index > 0 {
                        output.push_str(", ");
                    }
                    output.push_str(&initializer.expr);
                }
                output.push_str(")\n");
            },
        );
    } else {
        output.push_str("\treturn fut\n");
    }
    output.push_str("}\n");
}

/// Computes the backend binding for an operation message: the wire type
/// expression and the native<->wire conversion expressions. Returns `None`
/// when the message is not handled by this backend.
fn operation_message_binding(
    message: &PlannedMessageType,
    operation_name: &str,
    direction: &str,
    backend: &ModelBackend,
) -> Result<Option<(String, GoValueConversion)>> {
    let Some(proto_type) = backend.message_proto_type(message) else {
        return Ok(None);
    };
    let conversion = backend
        .wire_conversion(&PlannedValueType::Message(message.clone()), None)
        .ok_or_else(|| Error::UnsupportedGoProtoConversion {
            context: format!("operation `{operation_name}` {direction}"),
            reason: "message values have no Go proto conversion".to_string(),
        })?
        .map_err(|reason| Error::UnsupportedGoProtoConversion {
            context: format!("operation `{operation_name}` {direction}"),
            reason,
        })?;
    Ok(Some((proto_type, conversion)))
}

fn operation_message_proto_type(
    message: &PlannedMessageType,
    backend: &ModelBackend,
) -> Option<String> {
    backend.message_proto_type(message)
}

fn build_rendered_resource_return(
    service_name: &str,
    resource_return: &PlannedOperationResourceReturn,
    resource: &PlannedResource,
    api_plan: &PlannedSpec,
    backend: &ModelBackend,
    package: &GoPackageContext,
) -> Result<RenderedResourceReturn> {
    let mut local_lines = Vec::new();
    let mut field_initializers = Vec::new();

    for binding in &resource_return.bindings {
        let field = resource
            .fields
            .iter()
            .find(|field| field.name == binding.field_name)
            .ok_or_else(|| Error::InvalidResource {
                service: service_name.to_string(),
                resource: resource_return.resource_type_name.clone(),
                reason: format!(
                    "resource-returning operation references unknown field `{}`",
                    binding.field_name
                ),
            })?;
        let (mut expr, constructor_needs_pointer_unpack) = match &binding.source {
            ResolvedResourceBindingSource::RequestField {
                field_name,
                proto_field_name,
                hidden,
            } => {
                if *hidden {
                    let (lines, expr) = resource_return_proto_field_source(
                        field,
                        "requestProto",
                        proto_field_name,
                        api_plan,
                        backend,
                        package,
                    )
                    .map_err(|reason| Error::UnsupportedGoProtoConversion {
                        context: format!(
                            "resource return field `{}.{}`",
                            resource_return.resource_type_name, binding.field_name
                        ),
                        reason,
                    })?;
                    local_lines.extend(lines);
                    (expr, false)
                } else {
                    (format!("request.{}", go_field_name(field_name)), true)
                }
            }
            ResolvedResourceBindingSource::ResultField {
                proto_field_name, ..
            } => {
                let (lines, expr) = resource_return_proto_field_source(
                    field,
                    "result",
                    proto_field_name,
                    api_plan,
                    backend,
                    package,
                )
                .map_err(|reason| Error::UnsupportedGoProtoConversion {
                    context: format!(
                        "resource return field `{}.{}`",
                        resource_return.resource_type_name, binding.field_name
                    ),
                    reason,
                })?;
                local_lines.extend(lines);
                (expr, false)
            }
        };

        let field_kind = planned_field_kind(&field.kind, api_plan);
        let internal_type = resolve_resource_field_kind(&field_kind, field.optional, package);
        if constructor_needs_pointer_unpack
            && field.optional
            && public_default_punning_zero_for_field(&field_kind, &internal_type).is_some()
        {
            let value_type = internal_type.trim_start_matches('*');
            let constructor_value = format!(
                "{}ConstructorValue",
                resource_return_local_name(&field.name)
            );
            local_lines.push(format!("var {constructor_value} {value_type}"));
            local_lines.push(format!("if {expr} != nil {{"));
            local_lines.push(format!("\t{constructor_value} = *{expr}"));
            local_lines.push("}".to_string());
            expr = constructor_value;
        }

        field_initializers.push(RenderedResourceFieldInitializer { expr });
    }

    Ok(RenderedResourceReturn {
        resource_type_name: resource_return.resource_type_name.clone(),
        local_lines,
        field_initializers,
    })
}

fn resource_return_proto_field_source(
    field: &PlannedResourceField,
    source: &str,
    proto_field_name: &str,
    api_plan: &PlannedSpec,
    backend: &ModelBackend,
    package: &GoPackageContext,
) -> GoConversionResult<(Vec<String>, String)> {
    let local = resource_return_local_name(&field.name);
    let proto_field = go_proto_field_name(proto_field_name);
    let getter = format!("{source}.Get{proto_field}()");
    let field_kind = planned_field_kind(&field.kind, api_plan);
    let mut native_type = resolve_resource_field_kind(&field_kind, field.optional, package);
    if field.optional && public_default_punning_zero_for_field(&field_kind, &native_type).is_some()
    {
        native_type = native_type.trim_start_matches('*').to_string();
    }

    match &field_kind {
        PlannedFieldKind::Singular(value) => {
            let conversion = go_value_conversion(value, api_plan, backend, package)?;
            Ok(resource_return_singular_proto_source(
                field,
                value,
                &conversion,
                &getter,
                &local,
                &native_type,
                "return nil, err",
            ))
        }
        PlannedFieldKind::Repeated(value) => {
            let conversion = go_value_conversion(value, api_plan, backend, package)?;
            let converted = (conversion.from_proto)("item");
            let mut lines = vec![
                format!("var {local} {native_type}"),
                format!("for _, item := range {getter} {{"),
            ];
            match conversion.kind {
                GoConversionKind::OverrideConverter => {
                    if !conversion.from_proto_returns_pointer {
                        if conversion.fallible {
                            lines.push(format!("\tconverted, err := {converted}"));
                            lines.push("\tif err != nil {".to_string());
                            lines.push("\t\treturn nil, err".to_string());
                            lines.push("\t}".to_string());
                            lines.push(format!("\t{local} = append({local}, converted)"));
                        } else {
                            lines.push(format!("\t{local} = append({local}, {converted})"));
                        }
                    } else {
                        if conversion.fallible {
                            lines.push(format!("\tconverted, err := {converted}"));
                            lines.push("\tif err != nil {".to_string());
                            lines.push("\t\treturn nil, err".to_string());
                            lines.push("\t}".to_string());
                            lines.push("\tif converted != nil {".to_string());
                        } else {
                            lines.push(format!(
                                "\tif converted := {converted}; converted != nil {{"
                            ));
                        }
                        lines.push(format!("\t\t{local} = append({local}, *converted)"));
                        lines.push("\t}".to_string());
                    }
                }
                _ => {
                    if conversion.fallible {
                        lines.push(format!("\tconverted, err := {converted}"));
                        lines.push("\tif err != nil {".to_string());
                        lines.push("\t\treturn nil, err".to_string());
                        lines.push("\t}".to_string());
                        lines.push(format!("\t{local} = append({local}, converted)"));
                    } else {
                        lines.push(format!("\t{local} = append({local}, {converted})"));
                    }
                }
            }
            lines.push("}".to_string());
            Ok((lines, local))
        }
        PlannedFieldKind::Map { key: _, value } => {
            let conversion = go_value_conversion(value, api_plan, backend, package)?;
            let converted = (conversion.from_proto)("v");
            let mut lines = vec![
                format!("var {local} {native_type}"),
                format!("if len({getter}) > 0 {{"),
                format!("\t{local} = make({native_type}, len({getter}))"),
                format!("\tfor k, v := range {getter} {{"),
            ];
            match conversion.kind {
                GoConversionKind::OverrideConverter => {
                    if !conversion.from_proto_returns_pointer {
                        if conversion.fallible {
                            lines.push(format!("\t\tconverted, err := {converted}"));
                            lines.push("\t\tif err != nil {".to_string());
                            lines.push("\t\t\treturn nil, err".to_string());
                            lines.push("\t\t}".to_string());
                            lines.push(format!("\t\t{local}[k] = converted"));
                        } else {
                            lines.push(format!("\t\t{local}[k] = {converted}"));
                        }
                    } else {
                        if conversion.fallible {
                            lines.push(format!("\t\tconverted, err := {converted}"));
                            lines.push("\t\tif err != nil {".to_string());
                            lines.push("\t\t\treturn nil, err".to_string());
                            lines.push("\t\t}".to_string());
                            lines.push("\t\tif converted != nil {".to_string());
                        } else {
                            lines.push(format!(
                                "\t\tif converted := {converted}; converted != nil {{"
                            ));
                        }
                        lines.push(format!("\t\t\t{local}[k] = *converted"));
                        lines.push("\t\t}".to_string());
                    }
                }
                _ => {
                    if conversion.fallible {
                        lines.push(format!("\t\tconverted, err := {converted}"));
                        lines.push("\t\tif err != nil {".to_string());
                        lines.push("\t\t\treturn nil, err".to_string());
                        lines.push("\t\t}".to_string());
                        lines.push(format!("\t\t{local}[k] = converted"));
                    } else {
                        lines.push(format!("\t\t{local}[k] = {converted}"));
                    }
                }
            }
            lines.push("\t}".to_string());
            lines.push("}".to_string());
            Ok((lines, local))
        }
    }
}

fn resource_return_singular_proto_source(
    field: &PlannedResourceField,
    value: &PlannedValueType,
    conversion: &GoValueConversion,
    getter: &str,
    local: &str,
    native_type: &str,
    error_return: &str,
) -> (Vec<String>, String) {
    let converted = (conversion.from_proto)(getter);
    let uses_pointer = field.optional && native_type.starts_with('*');

    if matches!(
        conversion.kind,
        GoConversionKind::Scalar | GoConversionKind::Enum
    ) && !uses_pointer
        && !conversion.fallible
    {
        return (Vec::new(), converted);
    }

    match conversion.kind {
        GoConversionKind::OverrideConverter => {
            if !conversion.from_proto_returns_pointer {
                if uses_pointer {
                    if conversion.fallible {
                        (
                            vec![
                                format!("converted, err := {converted}"),
                                "if err != nil {".to_string(),
                                format!("\t{error_return}"),
                                "}".to_string(),
                                format!("{local} := &converted"),
                            ],
                            local.to_string(),
                        )
                    } else {
                        (
                            vec![
                                format!("converted := {converted}"),
                                format!("{local} := &converted"),
                            ],
                            local.to_string(),
                        )
                    }
                } else if conversion.fallible {
                    (
                        vec![
                            format!("{local}, err := {converted}"),
                            "if err != nil {".to_string(),
                            format!("\t{error_return}"),
                            "}".to_string(),
                        ],
                        local.to_string(),
                    )
                } else {
                    (vec![format!("{local} := {converted}")], local.to_string())
                }
            } else if uses_pointer {
                if conversion.fallible {
                    (
                        vec![
                            format!("{local}, err := {converted}"),
                            "if err != nil {".to_string(),
                            format!("\t{error_return}"),
                            "}".to_string(),
                        ],
                        local.to_string(),
                    )
                } else {
                    (vec![format!("{local} := {converted}")], local.to_string())
                }
            } else {
                let mut lines = vec![format!("var {local} {native_type}")];
                if conversion.fallible {
                    lines.push(format!("converted, err := {converted}"));
                    lines.push("if err != nil {".to_string());
                    lines.push(format!("\t{error_return}"));
                    lines.push("}".to_string());
                    lines.push("if converted != nil {".to_string());
                } else {
                    lines.push(format!("if converted := {converted}; converted != nil {{"));
                }
                lines.push(format!("\t{local} = *converted"));
                lines.push("}".to_string());
                (lines, local.to_string())
            }
        }
        GoConversionKind::ModelConverter => {
            if uses_pointer {
                let mut lines = vec![
                    format!("var {local} {native_type}"),
                    format!("if {getter} != nil {{"),
                ];
                if conversion.fallible {
                    lines.push(format!("\tconverted, err := {converted}"));
                    lines.push("\tif err != nil {".to_string());
                    lines.push(format!("\t\t{error_return}"));
                    lines.push("\t}".to_string());
                } else {
                    lines.push(format!("\tconverted := {converted}"));
                }
                lines.push(format!("\t{local} = &converted"));
                lines.push("}".to_string());
                (lines, local.to_string())
            } else if conversion.fallible {
                (
                    vec![
                        format!("{local}, err := {converted}"),
                        "if err != nil {".to_string(),
                        format!("\t{error_return}"),
                        "}".to_string(),
                    ],
                    local.to_string(),
                )
            } else {
                (vec![format!("{local} := {converted}")], local.to_string())
            }
        }
        GoConversionKind::Scalar | GoConversionKind::Enum => {
            if uses_pointer {
                let value_local = format!("{local}Value");
                let zero = resource_return_zero_value_expr(value);
                (
                    vec![
                        format!("{value_local} := {converted}"),
                        format!("var {local} {native_type}"),
                        format!("if {value_local} != {zero} {{"),
                        format!("\t{local} = &{value_local}"),
                        "}".to_string(),
                    ],
                    local.to_string(),
                )
            } else {
                (vec![format!("{local} := {converted}")], local.to_string())
            }
        }
    }
}

fn resource_return_zero_value_expr(value: &PlannedValueType) -> &'static str {
    match value {
        PlannedValueType::Scalar(PlannedScalarType::Bool) => "false",
        PlannedValueType::Scalar(PlannedScalarType::String) => "\"\"",
        PlannedValueType::Scalar(PlannedScalarType::Bytes) => "nil",
        PlannedValueType::Scalar(_) | PlannedValueType::Enum(_) => "0",
        _ => "nil",
    }
}

fn resource_return_local_name(field_name: &str) -> String {
    go_unexported_name(&go_field_name(field_name))
}

/// Builds per-field conversion metadata for a proto-backed model, in field
/// declaration order (matching the rendered struct fields).
fn build_field_conversions(
    planned_model: &RecordSpec<PlannedFamily>,
    native_field_types: &[String],
    api_plan: &PlannedSpec,
    backend: &ModelBackend,
) -> Result<Vec<RenderedFieldConversion>> {
    planned_model
        .model_fields()
        .enumerate()
        .map(|(index, (field_name, field))| {
            let planned_field = planned_field(planned_model, field_name, field, api_plan);
            let native_go_type = native_field_types
                .get(index)
                .map(String::as_str)
                .unwrap_or("");
            build_field_conversion(&planned_field, native_go_type, api_plan, backend).map_err(
                |reason| Error::UnsupportedGoProtoConversion {
                    context: format!(
                        "field `{}.{}`",
                        planned_model.name, planned_field.authored_name
                    ),
                    reason,
                },
            )
        })
        .collect()
}

fn build_field_conversion(
    field: &crate::generator::go::PlannedField,
    native_go_type: &str,
    api_plan: &PlannedSpec,
    backend: &ModelBackend,
) -> GoConversionResult<RenderedFieldConversion> {
    let proto_field = go_proto_field_name(&field.proto_name);
    let go_field = go_field_name(&field.authored_name);
    let receiver = format!("m.{go_field}");

    match &field.kind {
        PlannedFieldKind::Singular(value) => {
            let conversion = go_value_conversion(value, api_plan, backend, &backend.package)?;
            let field_is_pointer = native_go_type.starts_with('*');
            let to_lines = singular_to_proto_lines(
                &conversion,
                &receiver,
                &proto_field,
                field_is_pointer,
                "return nil, err",
            );
            let mut from_lines = singular_from_proto_lines(
                &conversion,
                &proto_field,
                &go_field,
                field_is_pointer,
                "return value, err",
            );
            if field
                .flattened_annotation_override
                .as_ref()
                .and_then(|annotation| annotation.for_language(Language::Go))
                .is_some()
                && !conversion.from_proto_returns_pointer
            {
                from_lines = vec![
                    format!("if proto.Get{proto_field}() != nil {{"),
                    format!(
                        "\tconverted, err := {}",
                        (conversion.from_proto)(&format!("proto.Get{proto_field}()"))
                    ),
                    "\tif err != nil {".to_string(),
                    "\t\treturn value, err".to_string(),
                    "\t}".to_string(),
                    format!("\ttyped, ok := converted.({native_go_type})"),
                    "\tif !ok {".to_string(),
                    format!(
                        "\t\treturn value, fmt.Errorf(\"nexgen decoded field {go_field} has unexpected type %T\", converted)"
                    ),
                    "\t}".to_string(),
                    format!("\tvalue.{go_field} = typed"),
                    "}".to_string(),
                ];
            }
            Ok(RenderedFieldConversion {
                to_proto_lines: to_lines,
                from_proto_lines: from_lines,
            })
        }
        PlannedFieldKind::Repeated(value) => {
            let conversion = go_value_conversion(value, api_plan, backend, &backend.package)?;
            let to_lines =
                repeated_to_proto_lines(&conversion, &receiver, &proto_field, "return nil, err");
            let from_lines = repeated_from_proto_lines(
                &conversion,
                &proto_field,
                &go_field,
                "return value, err",
            );
            Ok(RenderedFieldConversion {
                to_proto_lines: to_lines,
                from_proto_lines: from_lines,
            })
        }
        PlannedFieldKind::Map { key, value } => {
            let conversion = go_value_conversion(value, api_plan, backend, &backend.package)?;
            let key_type = backend
                .value_proto_type(key)
                .map_err(|reason| format!("map key: {reason}"))?;
            let value_type = backend
                .value_proto_type(value)
                .map_err(|reason| format!("map value: {reason}"))?;
            let proto_map_type = format!("map[{key_type}]{value_type}");
            let to_lines = map_to_proto_lines(
                &conversion,
                &proto_map_type,
                &receiver,
                &proto_field,
                "return nil, err",
            );
            let from_lines = map_from_proto_lines(
                &conversion,
                native_go_type,
                &proto_field,
                &go_field,
                "return value, err",
            );
            Ok(RenderedFieldConversion {
                to_proto_lines: to_lines,
                from_proto_lines: from_lines,
            })
        }
    }
}

/// Builds `ToProto` lines for sourced (write-only) fields.
fn build_sourced_conversions(
    planned_model: &RecordSpec<PlannedFamily>,
    api_plan: &PlannedSpec,
    backend: &ModelBackend,
) -> Result<Vec<RenderedSourcedField>> {
    planned_model
        .sourced_fields()
        .map(|(field_name, field, source_expr)| {
            build_sourced_conversion(field_name, field, source_expr, api_plan, backend).map_err(
                |reason| Error::UnsupportedGoProtoConversion {
                    context: format!("sourced field `{}.{}`", planned_model.name, field_name),
                    reason,
                },
            )
        })
        .collect()
}

fn build_sourced_conversion(
    proto_name: &str,
    field: &RecordFieldSpec<PlannedFamily>,
    source_expr: &str,
    api_plan: &PlannedSpec,
    backend: &ModelBackend,
) -> GoConversionResult<RenderedSourcedField> {
    let proto_field = go_proto_field_name(proto_name);
    match &planned_field_kind(&field.field_type, api_plan) {
        PlannedFieldKind::Singular(value) => {
            let conversion = go_value_conversion(value, api_plan, backend, &backend.package)?;
            match conversion.kind {
                GoConversionKind::OverrideConverter => {
                    let arg = if conversion.to_proto_takes_pointer {
                        "&sourced"
                    } else {
                        "sourced"
                    };
                    let converted = (conversion.to_proto)(arg);
                    let mut to_proto_lines = vec![format!("sourced := {source_expr}")];
                    if conversion.fallible {
                        to_proto_lines.extend([
                            format!("converted, err := {converted}"),
                            "if err != nil {".to_string(),
                            "\treturn nil, err".to_string(),
                            "}".to_string(),
                            format!("message.{proto_field} = converted"),
                        ]);
                    } else {
                        to_proto_lines.push(format!("message.{proto_field} = {converted}"));
                    }
                    Ok(RenderedSourcedField { to_proto_lines })
                }
                _ => {
                    let converted = (conversion.to_proto)(source_expr);
                    if conversion.fallible {
                        Ok(RenderedSourcedField {
                            to_proto_lines: vec![
                                format!("converted, err := {converted}"),
                                "if err != nil {".to_string(),
                                "\treturn nil, err".to_string(),
                                "}".to_string(),
                                format!("message.{proto_field} = converted"),
                            ],
                        })
                    } else {
                        Ok(RenderedSourcedField {
                            to_proto_lines: vec![format!("message.{proto_field} = {converted}")],
                        })
                    }
                }
            }
        }
        PlannedFieldKind::Repeated(value) => {
            let conversion = go_value_conversion(value, api_plan, backend, &backend.package)?;
            let converted = match conversion.kind {
                GoConversionKind::OverrideConverter if conversion.to_proto_takes_pointer => {
                    (conversion.to_proto)("&item")
                }
                _ => (conversion.to_proto)("item"),
            };
            let mut to_proto_lines = vec![format!("for _, item := range {source_expr} {{")];
            if conversion.fallible {
                to_proto_lines.push(format!("\tconverted, err := {converted}"));
                to_proto_lines.push("\tif err != nil {".to_string());
                to_proto_lines.push("\t\treturn nil, err".to_string());
                to_proto_lines.push("\t}".to_string());
                to_proto_lines.push(format!(
                    "\tmessage.{proto_field} = append(message.{proto_field}, converted)"
                ));
            } else {
                to_proto_lines.push(format!(
                    "\tmessage.{proto_field} = append(message.{proto_field}, {converted})"
                ));
            }
            to_proto_lines.push("}".to_string());
            Ok(RenderedSourcedField { to_proto_lines })
        }
        PlannedFieldKind::Map { key, value } => {
            let conversion = go_value_conversion(value, api_plan, backend, &backend.package)?;
            let key_type = backend
                .value_proto_type(key)
                .map_err(|reason| format!("map key: {reason}"))?;
            let value_type = backend
                .value_proto_type(value)
                .map_err(|reason| format!("map value: {reason}"))?;
            let proto_map_type = format!("map[{key_type}]{value_type}");
            let local = format!("sourced{proto_field}");
            let mut to_proto_lines = vec![format!("{local} := {source_expr}")];
            to_proto_lines.extend(map_to_proto_lines(
                &conversion,
                &proto_map_type,
                &local,
                &proto_field,
                "return nil, err",
            ));
            Ok(RenderedSourcedField { to_proto_lines })
        }
    }
}

/// `ToProto` lines for a singular field.
///
/// `field_is_pointer` indicates whether the native struct field is rendered as
/// a pointer (an optional field). The generated code bridges the pointer/value
/// mismatch with the converter while delegating all nil handling either to a
/// guard here or to the converter's nil passthrough.
fn singular_to_proto_lines(
    conversion: &GoValueConversion,
    receiver: &str,
    proto_field: &str,
    field_is_pointer: bool,
    error_return: &str,
) -> Vec<String> {
    let assign = |converted: &str| format!("message.{proto_field} = {converted}");
    let checked_assign = |call: String| {
        vec![
            "{".to_string(),
            format!("\tconverted, err := {call}"),
            "\tif err != nil {".to_string(),
            format!("\t\t{error_return}"),
            "\t}".to_string(),
            format!("\t{}", assign("converted")),
            "}".to_string(),
        ]
    };
    match conversion.kind {
        GoConversionKind::OverrideConverter => {
            let arg = if conversion.to_proto_takes_pointer {
                if field_is_pointer {
                    receiver.to_string()
                } else {
                    format!("&{receiver}")
                }
            } else {
                receiver.to_string()
            };
            let converted = (conversion.to_proto)(&arg);
            if conversion.fallible {
                checked_assign(converted)
            } else {
                vec![assign(&converted)]
            }
        }
        GoConversionKind::Scalar | GoConversionKind::Enum | GoConversionKind::ModelConverter => {
            if field_is_pointer {
                let converted = (conversion.to_proto)(&format!("(*{receiver})"));
                if conversion.fallible {
                    vec![
                        format!("if {receiver} != nil {{"),
                        format!("\tconverted, err := {converted}"),
                        "\tif err != nil {".to_string(),
                        format!("\t\t{error_return}"),
                        "\t}".to_string(),
                        format!("\t{}", assign("converted")),
                        "}".to_string(),
                    ]
                } else {
                    vec![
                        format!("if {receiver} != nil {{"),
                        format!("\t{}", assign(&converted)),
                        "}".to_string(),
                    ]
                }
            } else {
                let converted = (conversion.to_proto)(receiver);
                if conversion.fallible {
                    checked_assign(converted)
                } else {
                    vec![assign(&converted)]
                }
            }
        }
    }
}

/// `FromProto` lines for a singular field, assigning into `value.<go_field>`.
fn singular_from_proto_lines(
    conversion: &GoValueConversion,
    proto_field: &str,
    go_field: &str,
    field_is_pointer: bool,
    error_return: &str,
) -> Vec<String> {
    let getter = format!("proto.Get{proto_field}()");
    match conversion.kind {
        GoConversionKind::OverrideConverter => {
            let converted = (conversion.from_proto)(&getter);
            if !conversion.from_proto_returns_pointer {
                if conversion.fallible {
                    vec![
                        "{".to_string(),
                        format!("\tconverted, err := {converted}"),
                        "\tif err != nil {".to_string(),
                        format!("\t\t{error_return}"),
                        "\t}".to_string(),
                        if field_is_pointer {
                            format!("\tvalue.{go_field} = &converted")
                        } else {
                            format!("\tvalue.{go_field} = converted")
                        },
                        "}".to_string(),
                    ]
                } else {
                    if field_is_pointer {
                        vec![
                            "{".to_string(),
                            format!("\tconverted := {converted}"),
                            format!("\tvalue.{go_field} = &converted"),
                            "}".to_string(),
                        ]
                    } else {
                        vec![format!("value.{go_field} = {converted}")]
                    }
                }
            } else if field_is_pointer {
                if conversion.fallible {
                    vec![
                        "{".to_string(),
                        format!("\tconverted, err := {converted}"),
                        "\tif err != nil {".to_string(),
                        format!("\t\t{error_return}"),
                        "\t}".to_string(),
                        format!("\tvalue.{go_field} = converted"),
                        "}".to_string(),
                    ]
                } else {
                    vec![format!("value.{go_field} = {converted}")]
                }
            } else if conversion.fallible {
                vec![
                    "{".to_string(),
                    format!("\tconverted, err := {converted}"),
                    "\tif err != nil {".to_string(),
                    format!("\t\t{error_return}"),
                    "\t}".to_string(),
                    "\tif converted != nil {".to_string(),
                    format!("\t\tvalue.{go_field} = *converted"),
                    "\t}".to_string(),
                    "}".to_string(),
                ]
            } else {
                vec![
                    format!("if converted := {converted}; converted != nil {{"),
                    format!("\tvalue.{go_field} = *converted"),
                    "}".to_string(),
                ]
            }
        }
        GoConversionKind::ModelConverter => {
            let converted = (conversion.from_proto)(&getter);
            if field_is_pointer {
                if conversion.fallible {
                    vec![
                        format!("if {getter} != nil {{"),
                        format!("\tconverted, err := {converted}"),
                        "\tif err != nil {".to_string(),
                        format!("\t\t{error_return}"),
                        "\t}".to_string(),
                        format!("\tvalue.{go_field} = &converted"),
                        "}".to_string(),
                    ]
                } else {
                    vec![
                        format!("if {getter} != nil {{"),
                        format!("\tconverted := {converted}"),
                        format!("\tvalue.{go_field} = &converted"),
                        "}".to_string(),
                    ]
                }
            } else if conversion.fallible {
                vec![
                    "{".to_string(),
                    format!("\tconverted, err := {converted}"),
                    "\tif err != nil {".to_string(),
                    format!("\t\t{error_return}"),
                    "\t}".to_string(),
                    format!("\tvalue.{go_field} = converted"),
                    "}".to_string(),
                ]
            } else {
                vec![format!("value.{go_field} = {converted}")]
            }
        }
        GoConversionKind::Scalar | GoConversionKind::Enum => {
            let converted = (conversion.from_proto)(&getter);
            if field_is_pointer {
                vec![
                    "{".to_string(),
                    format!("\tconverted := {converted}"),
                    format!("\tvalue.{go_field} = &converted"),
                    "}".to_string(),
                ]
            } else {
                vec![format!("value.{go_field} = {converted}")]
            }
        }
    }
}

/// `ToProto` lines for a repeated field.
fn repeated_to_proto_lines(
    conversion: &GoValueConversion,
    receiver: &str,
    proto_field: &str,
    error_return: &str,
) -> Vec<String> {
    let converted = match conversion.kind {
        GoConversionKind::OverrideConverter if conversion.to_proto_takes_pointer => {
            (conversion.to_proto)("&item")
        }
        _ => (conversion.to_proto)("item"),
    };
    let mut lines = vec![format!("for _, item := range {receiver} {{")];
    if conversion.fallible {
        lines.push(format!("\tconverted, err := {converted}"));
        lines.push("\tif err != nil {".to_string());
        lines.push(format!("\t\t{error_return}"));
        lines.push("\t}".to_string());
        lines.push(format!(
            "\tmessage.{proto_field} = append(message.{proto_field}, converted)"
        ));
    } else {
        lines.push(format!(
            "\tmessage.{proto_field} = append(message.{proto_field}, {converted})"
        ));
    }
    lines.push("}".to_string());
    lines
}

/// `FromProto` lines for a repeated field.
fn repeated_from_proto_lines(
    conversion: &GoValueConversion,
    proto_field: &str,
    go_field: &str,
    error_return: &str,
) -> Vec<String> {
    let converted = (conversion.from_proto)("item");
    let mut lines = vec![format!("for _, item := range proto.Get{proto_field}() {{")];
    if conversion.kind == GoConversionKind::OverrideConverter
        && conversion.from_proto_returns_pointer
        && conversion.fallible
    {
        lines.push(format!("\tconverted, err := {converted}"));
        lines.push("\tif err != nil {".to_string());
        lines.push(format!("\t\t{error_return}"));
        lines.push("\t}".to_string());
        lines.push("\tif converted != nil {".to_string());
        lines.push(format!(
            "\t\tvalue.{go_field} = append(value.{go_field}, *converted)"
        ));
        lines.push("\t}".to_string());
    } else if conversion.kind == GoConversionKind::OverrideConverter
        && conversion.from_proto_returns_pointer
    {
        lines.push(format!(
            "\tif converted := {converted}; converted != nil {{"
        ));
        lines.push(format!(
            "\t\tvalue.{go_field} = append(value.{go_field}, *converted)"
        ));
        lines.push("\t}".to_string());
    } else if conversion.fallible {
        lines.push(format!("\tconverted, err := {converted}"));
        lines.push("\tif err != nil {".to_string());
        lines.push(format!("\t\t{error_return}"));
        lines.push("\t}".to_string());
        lines.push(format!(
            "\tvalue.{go_field} = append(value.{go_field}, converted)"
        ));
    } else {
        lines.push(format!(
            "\tvalue.{go_field} = append(value.{go_field}, {converted})"
        ));
    }
    lines.push("}".to_string());
    lines
}

/// `ToProto` lines for a map field. `proto_map_type` is the Go type expression
/// of the proto struct's map field (e.g. `map[string]string`).
fn map_to_proto_lines(
    conversion: &GoValueConversion,
    proto_map_type: &str,
    receiver: &str,
    proto_field: &str,
    error_return: &str,
) -> Vec<String> {
    let converted = match conversion.kind {
        GoConversionKind::OverrideConverter if conversion.to_proto_takes_pointer => {
            (conversion.to_proto)("&v")
        }
        _ => (conversion.to_proto)("v"),
    };
    let mut lines = vec![
        format!("if len({receiver}) > 0 {{"),
        format!("\tmessage.{proto_field} = make({proto_map_type}, len({receiver}))"),
        format!("\tfor k, v := range {receiver} {{"),
    ];
    if conversion.fallible {
        lines.push(format!("\t\tconverted, err := {converted}"));
        lines.push("\t\tif err != nil {".to_string());
        lines.push(format!("\t\t\t{error_return}"));
        lines.push("\t\t}".to_string());
        lines.push(format!("\t\tmessage.{proto_field}[k] = converted"));
    } else {
        lines.push(format!("\t\tmessage.{proto_field}[k] = {converted}"));
    }
    lines.push("\t}".to_string());
    lines.push("}".to_string());
    lines
}

/// `FromProto` lines for a map field. `native_map_type` is the Go type
/// expression of the native struct's map field (e.g. `map[string]string`).
fn map_from_proto_lines(
    conversion: &GoValueConversion,
    native_map_type: &str,
    proto_field: &str,
    go_field: &str,
    error_return: &str,
) -> Vec<String> {
    let getter = format!("proto.Get{proto_field}()");
    let converted = (conversion.from_proto)("v");
    let mut lines = vec![
        format!("if len({getter}) > 0 {{"),
        format!("\tvalue.{go_field} = make({native_map_type}, len({getter}))"),
        format!("\tfor k, v := range {getter} {{"),
    ];
    match conversion.kind {
        GoConversionKind::OverrideConverter => {
            if !conversion.from_proto_returns_pointer {
                if conversion.fallible {
                    lines.push(format!("\t\tconverted, err := {converted}"));
                    lines.push("\t\tif err != nil {".to_string());
                    lines.push(format!("\t\t\t{error_return}"));
                    lines.push("\t\t}".to_string());
                    lines.push(format!("\t\tvalue.{go_field}[k] = converted"));
                } else {
                    lines.push(format!("\t\tvalue.{go_field}[k] = {converted}"));
                }
            } else {
                if conversion.fallible {
                    lines.push(format!("\t\tconverted, err := {converted}"));
                    lines.push("\t\tif err != nil {".to_string());
                    lines.push(format!("\t\t\t{error_return}"));
                    lines.push("\t\t}".to_string());
                    lines.push("\t\tif converted != nil {".to_string());
                } else {
                    lines.push(format!(
                        "\t\tif converted := {converted}; converted != nil {{"
                    ));
                }
                lines.push(format!("\t\t\tvalue.{go_field}[k] = *converted"));
                lines.push("\t\t}".to_string());
            }
        }
        _ => {
            if conversion.fallible {
                lines.push(format!("\t\tconverted, err := {converted}"));
                lines.push("\t\tif err != nil {".to_string());
                lines.push(format!("\t\t\t{error_return}"));
                lines.push("\t\t}".to_string());
                lines.push(format!("\t\tvalue.{go_field}[k] = converted"));
            } else {
                lines.push(format!("\t\tvalue.{go_field}[k] = {converted}"));
            }
        }
    }
    lines.push("\t}".to_string());
    lines.push("}".to_string());
    lines
}

/// Renders the `toProto` method and unexported from-proto constructor for a
/// proto-backed model.
fn render_model_wire_methods(
    output: &mut String,
    model: &RenderedModel,
    wire: &RenderedModelWire,
    package: &GoPackageContext,
) {
    let proto_value_type = wire.proto_type.trim_start_matches('*');

    output.push('\n');
    output.push_str("func (m ");
    output.push_str(&model.name);
    output.push_str(") toProto(ctx ");
    output.push_str(&package.workflow_context_type());
    output.push_str(") (");
    output.push_str(&wire.proto_type);
    output.push_str(", error) {\n");
    output.push_str("\tmessage := &");
    output.push_str(proto_value_type);
    output.push_str("{}\n");
    for conversion in &wire.field_conversions {
        for line in &conversion.to_proto_lines {
            output.push('\t');
            output.push_str(line);
            output.push('\n');
        }
    }
    for sourced in &wire.sourced_fields {
        for line in &sourced.to_proto_lines {
            output.push('\t');
            output.push_str(line);
            output.push('\n');
        }
    }
    output.push_str("\treturn message, nil\n");
    output.push_str("}\n");

    output.push('\n');
    output.push_str("func ");
    let (model_ident, _) = split_go_type_decl_name(&model.name);
    output.push_str(&go_unexported_name(model_ident));
    output.push_str("FromProto(ctx ");
    output.push_str(&package.workflow_context_type());
    output.push_str(", proto ");
    output.push_str(&wire.proto_type);
    output.push_str(") (");
    output.push_str(&model.name);
    output.push_str(", error) {\n");
    output.push_str("\tvalue := ");
    output.push_str(&model.name);
    output.push_str("{}\n");
    for conversion in &wire.field_conversions {
        for line in &conversion.from_proto_lines {
            output.push('\t');
            output.push_str(line);
            output.push('\n');
        }
    }
    output.push_str("\treturn value, nil\n");
    output.push_str("}\n");
}
