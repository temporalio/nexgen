//! Resource resolution resolves resource methods and operation-return field sources.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use heck::{ToKebabCase, ToUpperCamelCase};
use prost_types::FieldDescriptorProto;
use prost_types::field_descriptor_proto::Type;

use crate::descriptors::{DescriptorIndex, real_oneof_groups};
use crate::error::{Error, Result};
use crate::nexgen_config::GenerationTarget;
use crate::spec::{
    ApiSpec, ApiSpecTransform, ExternalTypeSpec, FieldPolicy, IntSpec, JsonModelSpec,
    OperationSpec, RecordFieldSpec, RecordSpec, ResourceFieldSpec, ResourceMethodSpec,
    ResourceResultSpec, ResourceSpec, SelectedFamily, SelectedResourceType, SelectedTextSpec,
    ServiceSpec, Symbol, TypeFamily, TypeSpec,
};
use crate::spec::{ApiSpecLeaf, CompilerPass};

mod type_parameters;

/// Selected IR with validated resource metadata and resolved operation-return bindings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ResourceBoundFamily;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ResourceBoundData {
    pub(crate) operation_returns: BTreeMap<String, BTreeMap<String, ResolvedResourceReturnSpec>>,
}

impl TypeFamily for ResourceBoundFamily {
    type Origin = Option<crate::spec::FieldPolicy>;
    type SpecData = ResourceBoundData;
    type Record = Symbol;
    type Enum = Symbol;
    type Flags = Symbol;
    type Variant = Symbol;
    type Resource = SelectedResourceType;
    type Proto = Symbol;
    type Json = crate::spec::JsonModelSpec<Symbol>;
    type Alias = Symbol;
    type ServiceData = ();
    type RecordData = ();
    type ResourceData = ResolvedResourceSpec;
    type OperationData = ();
    type FieldData = ();
    type Text = SelectedTextSpec;
}

struct ResourceBindingMapper {
    data: ResourceBoundData,
    resources: BTreeMap<String, ResolvedResourceSpec>,
}

impl ApiSpecTransform<SelectedFamily, ResourceBoundFamily> for ResourceBindingMapper {
    fn map_origin(
        &mut self,
        origin: Option<crate::spec::FieldPolicy>,
    ) -> Option<crate::spec::FieldPolicy> {
        origin
    }
    fn map_spec_data(&mut self, _data: ()) -> ResourceBoundData {
        ResourceBoundData {
            operation_returns: std::mem::take(&mut self.data.operation_returns),
        }
    }
    fn map_record(&mut self, value: Symbol) -> Symbol {
        value
    }
    fn map_enum(&mut self, value: Symbol) -> Symbol {
        value
    }
    fn map_flags(&mut self, value: Symbol) -> Symbol {
        value
    }
    fn map_variant(&mut self, value: Symbol) -> Symbol {
        value
    }
    fn map_resource(&mut self, value: SelectedResourceType) -> SelectedResourceType {
        value
    }
    fn map_proto(&mut self, value: Symbol) -> Symbol {
        value
    }
    fn map_json(&mut self, value: JsonModelSpec<Symbol>) -> JsonModelSpec<Symbol> {
        value
    }
    fn map_alias(&mut self, value: Symbol) -> Symbol {
        value
    }
    fn map_service_data(&mut self, _: &str, _: ()) {}
    fn map_record_data(&mut self, _: &str, _: ()) {}
    fn map_resource_data(&mut self, name: &str, _: ()) -> ResolvedResourceSpec {
        self.resources
            .remove(name)
            .expect("resource resolution includes every resource")
    }
    fn map_operation_data(&mut self, _: &str, _: ()) {}
    fn map_field_data(&mut self, _: &str, _: &str, _: ()) {}
    fn map_text(&mut self, value: SelectedTextSpec) -> SelectedTextSpec {
        value
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ResolvedServiceResources {
    pub(crate) resources: Vec<ResolvedResourceSpec>,
    pub(crate) operation_returns: BTreeMap<String, ResolvedResourceReturnSpec>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ResolvedResourceSpec {
    pub(crate) name: String,
    pub(crate) doc: SelectedTextSpec,
    /// Inferred during resource resolution, in stable type-identity order.
    pub(crate) type_parameters: Vec<crate::spec::TypeParameterSpec>,
    pub(crate) fields: Vec<ResourceFieldSpec<SelectedFamily>>,
    pub(crate) methods: Vec<ResolvedResourceMethodSpec>,
    pub(crate) close: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ResolvedResourceMethodSpec {
    pub(crate) name: String,
    pub(crate) doc: SelectedTextSpec,
    pub(crate) params: Vec<ResourceFieldSpec<SelectedFamily>>,
    pub(crate) result: Option<ResourceResultSpec<SelectedFamily>>,
    pub(crate) varargs: Option<String>,
    pub(crate) api_omitted: BTreeSet<GenerationTarget>,
    pub(crate) binding: ResolvedResourceMethodBinding,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedResourceMethodBinding {
    Operation {
        operation_name: String,
        request_plan: RequestPlan,
    },
    Stub,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedResourceReturnSpec {
    pub resource_name: String,
    pub output_message_name: String,
    pub bindings: Vec<ResolvedResourceFieldBinding>,
    /// Set when the operation returns a record that holds the resource in one field.
    pub holder: Option<ResolvedResourceHolder>,
}

/// A result record with one resource field. The resource binds from the record's message,
/// and the other fields of the record come from the same message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedResourceHolder {
    pub record_name: String,
    pub field_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedResourceFieldBinding {
    pub field_name: String,
    pub optional: bool,
    pub source: ResolvedResourceBindingSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedResourceBindingSource {
    RequestField {
        field_name: String,
        proto_field_name: String,
        origin: Option<FieldPolicy>,
    },
    ResultField {
        field_name: String,
        proto_field_name: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestPlan {
    Source(RequestPlanSource),
    Construct {
        message_name: String,
        fields: Vec<RequestPlanField>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestPlanField {
    pub field_name: String,
    pub value: RequestPlan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestPlanSource {
    ResourceField(String),
    MethodParam(String),
}

#[derive(Debug, Clone)]
struct MessageFieldInfo {
    proto_name: String,
    api_name: String,
    required: bool,
    origin: Option<FieldPolicy>,
    message_name: Option<String>,
}

#[cfg(test)]
fn resolve_service_resources(
    spec: &ApiSpec<SelectedFamily>,
    service: &ServiceSpec<SelectedFamily>,
    descriptors: &DescriptorIndex,
    targets: &[crate::nexgen_config::GenerationTarget],
) -> Result<ResolvedServiceResources> {
    let mut resources = resolve_service_resource_methods(spec, service, descriptors)?;
    if targets.is_empty() {
        for resource in &mut resources {
            resource.methods.clear();
        }
    }
    resolve_service_resource_returns(spec, service, descriptors, resources)
}

fn resolve_service_resource_methods(
    spec: &ApiSpec<SelectedFamily>,
    service: &ServiceSpec<SelectedFamily>,
    descriptors: &DescriptorIndex,
) -> Result<Vec<ResolvedResourceSpec>> {
    service
        .resources
        .iter()
        .map(|resource| resolve_resource_methods(spec, service, resource, descriptors))
        .collect()
}

fn resolve_service_resource_returns(
    spec: &ApiSpec<SelectedFamily>,
    service: &ServiceSpec<SelectedFamily>,
    descriptors: &DescriptorIndex,
    resources: Vec<ResolvedResourceSpec>,
) -> Result<ResolvedServiceResources> {
    let mut operation_returns = BTreeMap::new();
    for operation in &service.operations {
        if let Some(TypeSpec::Resource(resource)) = operation.output_type() {
            ensure_local_resource_return(service, operation, resource.as_str())?;
        }
        let holder = operation_output_resource_holder(spec, service, operation)?;
        let resource_name = match &holder {
            Some((_, _, resource_name, _)) => *resource_name,
            None => {
                let Some(resource_name) = operation_output_resource_name(service, operation) else {
                    continue;
                };
                if operation_output_resource_wire_type(operation).is_none() {
                    continue;
                }
                resource_name
            }
        };
        let resource = service
            .resource(resource_name)
            .ok_or_else(|| Error::InvalidResource {
                service: service.name.clone(),
                resource: resource_name.to_string(),
                reason: format!(
                    "operation `{}` returns unknown resource `{resource_name}`",
                    operation.name
                ),
            })?;
        let Some(input_message_name) = operation_input_message_name(operation, descriptors) else {
            return Err(Error::InvalidResource {
                service: service.name.clone(),
                resource: resource.name.to_upper_camel_case(),
                reason: format!(
                    "operation `{}` returns a resource directly and does not need field bindings",
                    operation.name
                ),
            });
        };
        let Some(output_message_name) = holder
            .as_ref()
            .map(|(_, _, _, proto_name)| *proto_name)
            .or_else(|| operation_output_resource_message_name(operation))
        else {
            return Err(Error::InvalidResource {
                service: service.name.clone(),
                resource: resource.name.to_upper_camel_case(),
                reason: format!(
                    "operation `{}` returns a resource directly and does not need field bindings",
                    operation.name
                ),
            });
        };
        let bindings = resource
            .fields
            .iter()
            .map(|field| {
                let source = bind_resource_return_field(
                    spec,
                    &service.name,
                    &resource.name,
                    descriptors,
                    input_message_name,
                    output_message_name,
                    &field.name,
                )?;
                Ok(ResolvedResourceFieldBinding {
                    field_name: field.name.clone(),
                    optional: field.optional,
                    source,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        operation_returns.insert(
            operation.name.clone(),
            ResolvedResourceReturnSpec {
                resource_name: resource.name.clone(),
                output_message_name: output_message_name.to_string(),
                bindings,
                holder: holder.map(|(record_name, field_key, _, _)| ResolvedResourceHolder {
                    record_name: record_name.to_string(),
                    field_key: field_key.to_string(),
                }),
            },
        );
    }

    let mut bound_operation_resources = BTreeMap::new();
    for resource in &resources {
        for method in &resource.methods {
            let ResolvedResourceMethodBinding::Operation { operation_name, .. } = &method.binding
            else {
                continue;
            };
            if let Some(existing_resource) =
                bound_operation_resources.insert(operation_name.clone(), resource.name.clone())
                && existing_resource != resource.name
            {
                return Err(Error::InvalidResourceMethod {
                    service: service.name.clone(),
                    resource: resource.name.to_upper_camel_case(),
                    method: method.name.to_string(),
                    reason: format!(
                        "bound operation `{operation_name}` is already owned by resource `{}`",
                        existing_resource.to_upper_camel_case()
                    ),
                });
            }
        }
    }

    Ok(ResolvedServiceResources {
        resources,
        operation_returns,
    })
}

fn resolve_resource_methods(
    spec: &ApiSpec<SelectedFamily>,
    service: &ServiceSpec<SelectedFamily>,
    resource: &ResourceSpec<SelectedFamily>,
    descriptors: &DescriptorIndex,
) -> Result<ResolvedResourceSpec> {
    let methods = resource
        .methods
        .iter()
        .map(|method| resolve_resource_method(spec, service, resource, method, descriptors))
        .collect::<Result<Vec<_>>>()?;
    Ok(ResolvedResourceSpec {
        name: resource.name.clone(),
        doc: resource.doc.clone(),
        type_parameters: Vec::new(),
        fields: resource.fields.clone(),
        methods,
        close: resource.close.clone(),
    })
}

fn resolve_resource_method(
    spec: &ApiSpec<SelectedFamily>,
    service: &ServiceSpec<SelectedFamily>,
    resource: &ResourceSpec<SelectedFamily>,
    method: &ResourceMethodSpec<SelectedFamily>,
    descriptors: &DescriptorIndex,
) -> Result<ResolvedResourceMethodSpec> {
    let mut environment = BTreeMap::new();
    for field in &resource.fields {
        environment.insert(
            field.name.clone(),
            RequestPlanSource::ResourceField(field.name.clone()),
        );
    }
    for param in &method.params {
        environment.insert(
            param.name.clone(),
            RequestPlanSource::MethodParam(param.name.clone()),
        );
    }

    if let Some(operation_name) = &method.operation_name {
        let operation =
            service
                .operation(operation_name)
                .ok_or_else(|| Error::InvalidResourceMethod {
                    service: service.name.clone(),
                    resource: resource.name.to_upper_camel_case(),
                    method: method.name.to_string(),
                    reason: format!("bound operation `{operation_name}` was not found"),
                })?;
        let Some(request_plan) =
            synthesize_operation_request_plan(spec, descriptors, operation, &environment)?
        else {
            return Err(Error::InvalidResourceMethod {
                service: service.name.clone(),
                resource: resource.name.to_upper_camel_case(),
                method: method.name.to_string(),
                reason: format!(
                    "bound operation `{operation_name}` cannot be called from resource fields and method parameters"
                ),
            });
        };
        if !resource_method_result_matches_operation(service, method, operation) {
            return Err(Error::InvalidResourceMethod {
                service: service.name.clone(),
                resource: resource.name.to_upper_camel_case(),
                method: method.name.to_string(),
                reason: format!(
                    "bound operation `{operation_name}` result does not match the method result"
                ),
            });
        }
        return Ok(ResolvedResourceMethodSpec {
            name: method.name.clone(),
            doc: method.doc.clone(),
            params: bytes_bound_params(spec, &method.params, &request_plan),
            result: method.result.clone(),
            varargs: method.varargs.clone(),
            api_omitted: method.api_omitted.clone(),
            binding: ResolvedResourceMethodBinding::Operation {
                operation_name: operation.name.clone(),
                request_plan,
            },
        });
    }

    let implicit_operation_name = method.name.to_upper_camel_case();
    let mut matching_operations = Vec::new();
    for operation in &service.operations {
        if operation.name != implicit_operation_name {
            continue;
        }
        let Some(request_plan) =
            synthesize_operation_request_plan(spec, descriptors, operation, &environment)?
        else {
            continue;
        };

        if !resource_method_result_matches_operation(service, method, operation) {
            continue;
        }

        matching_operations.push((operation.name.clone(), request_plan));
    }

    let binding = match matching_operations.len() {
        // A stub close method would fail at the end of each scope.
        0 if resource.close.as_deref() == Some(method.name.as_str()) => {
            return Err(Error::InvalidResourceMethod {
                service: service.name.clone(),
                resource: resource.name.to_upper_camel_case(),
                method: method.name.to_string(),
                reason: "has `@nexus.close`, but binds to no operation".to_string(),
            });
        }
        0 => ResolvedResourceMethodBinding::Stub,
        1 => {
            let (operation_name, request_plan) = matching_operations.pop().expect("len checked");
            ResolvedResourceMethodBinding::Operation {
                operation_name,
                request_plan,
            }
        }
        _ => {
            let matches = matching_operations
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(Error::InvalidResourceMethod {
                service: service.name.clone(),
                resource: resource.name.to_upper_camel_case(),
                method: method.name.to_string(),
                reason: format!("matches multiple operations: {matches}"),
            });
        }
    };

    let params = match &binding {
        ResolvedResourceMethodBinding::Operation { request_plan, .. } => {
            bytes_bound_params(spec, &method.params, request_plan)
        }
        ResolvedResourceMethodBinding::Stub => method.params.clone(),
    };
    Ok(ResolvedResourceMethodSpec {
        name: method.name.clone(),
        doc: method.doc.clone(),
        params,
        result: method.result.clone(),
        varargs: method.varargs.clone(),
        api_omitted: method.api_omitted.clone(),
        binding,
    })
}

/// A proto-backed record reads a `list<u8>` field as the proto `bytes`. A method parameter that
/// fills such a field takes `bytes` too, or the method could not pass its value to the request.
fn bytes_bound_params(
    spec: &ApiSpec<SelectedFamily>,
    params: &[ResourceFieldSpec<SelectedFamily>],
    request_plan: &RequestPlan,
) -> Vec<ResourceFieldSpec<SelectedFamily>> {
    let mut bytes_params = BTreeSet::new();
    collect_bytes_params(spec, request_plan, &mut bytes_params);
    params
        .iter()
        .cloned()
        .map(|mut param| {
            if bytes_params.contains(param.name.as_str())
                && let Some(field_type) = list_as_bytes(&param.field_type)
            {
                param.field_type = field_type;
            }
            param
        })
        .collect()
}

fn collect_bytes_params<'a>(
    spec: &ApiSpec<SelectedFamily>,
    plan: &'a RequestPlan,
    bytes_params: &mut BTreeSet<&'a str>,
) {
    let RequestPlan::Construct {
        message_name,
        fields,
    } = plan
    else {
        return;
    };
    let record = spec
        .record(message_name)
        .or_else(|| spec.record_for_proto(message_name));
    for field in fields {
        match &field.value {
            RequestPlan::Source(RequestPlanSource::MethodParam(param_name)) => {
                let is_bytes = record
                    .and_then(|record| {
                        record
                            .fields
                            .values()
                            .find(|record_field| record_field.name == field.field_name)
                    })
                    .is_some_and(|record_field| {
                        matches!(record_field.field_type.without_option(), TypeSpec::Bytes)
                    });
                if is_bytes {
                    bytes_params.insert(param_name.as_str());
                }
            }
            RequestPlan::Source(RequestPlanSource::ResourceField(_)) => {}
            child @ RequestPlan::Construct { .. } => {
                collect_bytes_params(spec, child, bytes_params);
            }
        }
    }
}

/// A WIT `u8` reads as a `u32` integer, so this matches the list of integers that a `list<u8>`
/// parameter reads as.
fn list_as_bytes(param_type: &TypeSpec<SelectedFamily>) -> Option<TypeSpec<SelectedFamily>> {
    match param_type {
        TypeSpec::List(item) if matches!(**item, TypeSpec::Int(IntSpec::U32)) => {
            Some(TypeSpec::Bytes)
        }
        TypeSpec::Option(inner) => {
            list_as_bytes(inner).map(|bytes| TypeSpec::Option(Box::new(bytes)))
        }
        _ => None,
    }
}

fn resource_method_result_matches_operation(
    service: &ServiceSpec<SelectedFamily>,
    method: &ResourceMethodSpec<SelectedFamily>,
    operation: &OperationSpec<SelectedFamily>,
) -> bool {
    let Some(result) = &method.result else {
        return true;
    };
    match result.result_type.without_option() {
        TypeSpec::Resource(resource_name) => {
            operation_output_resource_name(service, operation) == Some(resource_name.as_str())
        }
        TypeSpec::External(ExternalTypeSpec::Proto(proto_name)) => {
            operation_output_ref(operation) == Some(proto_name.as_str())
                || operation_output_resource_message_name(operation) == Some(proto_name.as_str())
        }
        TypeSpec::Record(record) => {
            matches!(operation.output_type(), Some(TypeSpec::Record(output)) if output == record)
        }
        _ => false,
    }
}

fn synthesize_operation_request_plan(
    spec: &ApiSpec<SelectedFamily>,
    descriptors: &DescriptorIndex,
    operation: &OperationSpec<SelectedFamily>,
    environment: &BTreeMap<String, RequestPlanSource>,
) -> Result<Option<RequestPlan>> {
    if let Some(TypeSpec::External(ExternalTypeSpec::Proto(input_ref))) = operation.input_type() {
        synthesize_request_plan(spec, descriptors, input_ref.as_str(), environment)
    } else if let Some(TypeSpec::Record(input_ref)) = operation.input_type() {
        synthesize_record_request_plan(spec, descriptors, input_ref.as_str(), environment)
    } else {
        Ok(None)
    }
}

fn bind_resource_return_field(
    spec: &ApiSpec<SelectedFamily>,
    service_name: &str,
    resource_name: &str,
    descriptors: &DescriptorIndex,
    input_message_name: &str,
    output_message_name: &str,
    field_name: &str,
) -> Result<ResolvedResourceBindingSource> {
    if let Some(field) = find_message_field(spec, descriptors, input_message_name, field_name)? {
        return Ok(ResolvedResourceBindingSource::RequestField {
            field_name: field.api_name,
            proto_field_name: field.proto_name,
            origin: field.origin,
        });
    }
    if let Some(field) =
        find_visible_message_field(spec, descriptors, output_message_name, field_name)?
    {
        return Ok(ResolvedResourceBindingSource::ResultField {
            field_name: field.api_name,
            proto_field_name: field.proto_name,
        });
    }
    Err(Error::InvalidResource {
        service: service_name.to_string(),
        resource: resource_name.to_upper_camel_case(),
        reason: format!(
            "could not bind resource field `{field_name}` from operation input or output"
        ),
    })
}

fn synthesize_request_plan(
    spec: &ApiSpec<SelectedFamily>,
    descriptors: &DescriptorIndex,
    message_name: &str,
    environment: &BTreeMap<String, RequestPlanSource>,
) -> Result<Option<RequestPlan>> {
    let mut fields = Vec::new();
    for field in visible_message_fields(spec, descriptors, message_name)? {
        if let Some(source) = environment.get(&field.api_name) {
            fields.push(RequestPlanField {
                field_name: field.api_name,
                value: RequestPlan::Source(source.clone()),
            });
            continue;
        }

        if let Some(child_message_name) = field.message_name.as_deref() {
            if spec.record_for_proto(child_message_name).is_some()
                && let Some(value) =
                    synthesize_request_plan(spec, descriptors, child_message_name, environment)?
            {
                if !field.required && !request_plan_has_source(&value) {
                    continue;
                }
                fields.push(RequestPlanField {
                    field_name: field.api_name,
                    value,
                });
                continue;
            }
        }

        if field.required
            && !field
                .origin
                .as_ref()
                .is_some_and(FieldPolicy::can_initialize)
        {
            return Ok(None);
        }
    }

    Ok(Some(RequestPlan::Construct {
        message_name: message_name.to_string(),
        fields,
    }))
}

fn synthesize_record_request_plan(
    spec: &ApiSpec<SelectedFamily>,
    descriptors: &DescriptorIndex,
    record_name: &str,
    environment: &BTreeMap<String, RequestPlanSource>,
) -> Result<Option<RequestPlan>> {
    let mut fields = Vec::new();
    for field in visible_record_fields(spec, record_name)? {
        if let Some(source) = environment.get(&field.api_name) {
            fields.push(RequestPlanField {
                field_name: field.api_name,
                value: RequestPlan::Source(source.clone()),
            });
            continue;
        }

        if let Some(child_message_name) = field
            .message_name
            .as_deref()
            .and_then(|message_name| message_name.strip_prefix("proto:"))
            && let Some(value) =
                synthesize_request_plan(spec, descriptors, child_message_name, environment)?
        {
            if !field.required && !request_plan_has_source(&value) {
                continue;
            }
            fields.push(RequestPlanField {
                field_name: field.api_name,
                value,
            });
            continue;
        }

        if let Some(child_record_name) = field.message_name.as_deref()
            && let Some(value) =
                synthesize_record_request_plan(spec, descriptors, child_record_name, environment)?
        {
            if !field.required && !request_plan_has_source(&value) {
                continue;
            }
            fields.push(RequestPlanField {
                field_name: field.api_name,
                value,
            });
            continue;
        }

        if field.required
            && !field
                .origin
                .as_ref()
                .is_some_and(FieldPolicy::can_initialize)
        {
            return Ok(None);
        }
    }

    Ok(Some(RequestPlan::Construct {
        message_name: record_name.to_string(),
        fields,
    }))
}

fn request_plan_has_source(plan: &RequestPlan) -> bool {
    match plan {
        RequestPlan::Source(_) => true,
        RequestPlan::Construct { fields, .. } => fields
            .iter()
            .any(|field| request_plan_has_source(&field.value)),
    }
}

fn visible_message_fields(
    spec: &ApiSpec<SelectedFamily>,
    descriptors: &DescriptorIndex,
    message_name: &str,
) -> Result<Vec<MessageFieldInfo>> {
    Ok(all_message_fields(spec, descriptors, message_name)?
        .into_iter()
        .filter(|field| !field.origin.as_ref().is_some_and(FieldPolicy::is_omitted))
        .collect())
}

fn visible_record_fields(
    spec: &ApiSpec<SelectedFamily>,
    record_name: &str,
) -> Result<Vec<MessageFieldInfo>> {
    Ok(all_record_fields(spec, record_name)?
        .into_iter()
        .filter(|field| !field.origin.as_ref().is_some_and(FieldPolicy::is_omitted))
        .collect())
}

fn find_visible_message_field(
    spec: &ApiSpec<SelectedFamily>,
    descriptors: &DescriptorIndex,
    message_name: &str,
    field_name: &str,
) -> Result<Option<MessageFieldInfo>> {
    Ok(visible_message_fields(spec, descriptors, message_name)?
        .into_iter()
        .find(|field| field.api_name == field_name))
}

fn find_message_field(
    spec: &ApiSpec<SelectedFamily>,
    descriptors: &DescriptorIndex,
    message_name: &str,
    field_name: &str,
) -> Result<Option<MessageFieldInfo>> {
    Ok(all_message_fields(spec, descriptors, message_name)?
        .into_iter()
        .find(|field| field.api_name == field_name))
}

fn all_message_fields(
    spec: &ApiSpec<SelectedFamily>,
    descriptors: &DescriptorIndex,
    message_name: &str,
) -> Result<Vec<MessageFieldInfo>> {
    let message = descriptors
        .message(message_name)
        .ok_or_else(|| Error::UnknownTypeOverride {
            type_name: message_name.to_string(),
        })?;
    let record = spec.record_for_proto(message_name);
    // A record holds a proto oneof as one field named after the oneof, so a method parameter
    // binds to that field and not to the oneof's members.
    let mut grouped = BTreeMap::new();
    if let Some(record) = record {
        let groups =
            real_oneof_groups(message).map_err(|reason| Error::InvalidTypeOverrideField {
                message: message_name.to_string(),
                field: "<oneof>".to_string(),
                property: "oneof",
                reason,
            })?;
        for group in groups {
            if let Some(field) = record.fields.get(group.name) {
                for (index, _) in group.fields {
                    grouped.insert(index, (group.name, field));
                }
            }
        }
    }
    let mut emitted = BTreeSet::new();
    let mut fields = Vec::new();
    for (index, field) in message.descriptor.field.iter().enumerate() {
        match grouped.get(&index) {
            Some((oneof_name, record_field)) => {
                if emitted.insert(*oneof_name) {
                    fields.push(build_record_field_info(oneof_name, record_field, spec)?);
                }
            }
            None => fields.push(build_message_field_info(field, record, descriptors)?),
        }
    }
    Ok(fields)
}

fn all_record_fields(
    spec: &ApiSpec<SelectedFamily>,
    record_name: &str,
) -> Result<Vec<MessageFieldInfo>> {
    let record = spec
        .record(record_name)
        .ok_or_else(|| Error::UnknownTypeOverride {
            type_name: record_name.to_string(),
        })?;
    record
        .fields
        .iter()
        .filter(|(_, field)| !field.origin.as_ref().is_some_and(FieldPolicy::is_omitted))
        .map(|(field_name, field)| build_record_field_info(field_name, field, spec))
        .collect()
}

fn build_record_field_info(
    field_name: &str,
    field: &RecordFieldSpec<SelectedFamily>,
    spec: &ApiSpec<SelectedFamily>,
) -> Result<MessageFieldInfo> {
    let api_name = field.name.clone();
    let required = field.required;
    let message_name = api_field_message_name(&field.field_type, spec);
    Ok(MessageFieldInfo {
        proto_name: field_name.to_string(),
        api_name,
        required,
        origin: field.origin.clone(),
        message_name,
    })
}

fn api_field_message_name(
    field_type: &TypeSpec<SelectedFamily>,
    spec: &ApiSpec<SelectedFamily>,
) -> Option<String> {
    match field_type.without_option() {
        TypeSpec::Record(record_name) => Some(record_name.as_str().to_string()),
        TypeSpec::External(ExternalTypeSpec::Proto(proto_name))
            if spec.record_for_proto(proto_name.as_str()).is_some() =>
        {
            Some(format!("proto:{proto_name}"))
        }
        _ => None,
    }
}

fn build_message_field_info(
    field: &FieldDescriptorProto,
    record: Option<&RecordSpec<SelectedFamily>>,
    descriptors: &DescriptorIndex,
) -> Result<MessageFieldInfo> {
    let proto_name = field
        .name
        .as_deref()
        .expect("descriptor fields should be named");
    let api_name = record
        .and_then(|record| record.field_name_override(proto_name))
        .map(str::to_string)
        .unwrap_or_else(|| proto_name.to_kebab_case());
    let required = record.is_some_and(|record| {
        record
            .fields
            .get(proto_name)
            .is_some_and(|field| field.required)
    });
    let message_name = field_message_name(field, descriptors);
    Ok(MessageFieldInfo {
        proto_name: proto_name.to_string(),
        api_name,
        required,
        origin: record
            .and_then(|record| record.fields.get(proto_name))
            .and_then(|field| field.origin.clone()),
        message_name,
    })
}

fn field_message_name(
    field: &FieldDescriptorProto,
    descriptors: &DescriptorIndex,
) -> Option<String> {
    if field.r#type() != Type::Message {
        return None;
    }
    let type_name = field.type_name.as_deref()?.trim_start_matches('.');
    descriptors.message(type_name)?;
    Some(type_name.to_string())
}

fn operation_input_message_name<'a>(
    operation: &'a OperationSpec<SelectedFamily>,
    descriptors: &DescriptorIndex,
) -> Option<&'a str> {
    let Some(TypeSpec::External(ExternalTypeSpec::Proto(input_ref))) = operation.input_type()
    else {
        return None;
    };
    descriptors
        .message(input_ref.as_str())
        .map(|_| input_ref.as_str())
}

fn operation_output_resource_name<'a>(
    service: &'a ServiceSpec<SelectedFamily>,
    operation: &'a OperationSpec<SelectedFamily>,
) -> Option<&'a str> {
    let TypeSpec::Resource(resource_name) = operation.output_type()? else {
        return None;
    };
    service
        .resource(resource_name.as_str())
        .map(|_| resource_name.as_str())
}

fn ensure_local_resource_return(
    service: &ServiceSpec<SelectedFamily>,
    operation: &OperationSpec<SelectedFamily>,
    resource_name: &str,
) -> Result<()> {
    if service.resource(resource_name).is_none() {
        return Err(Error::InvalidResource {
            service: service.name.clone(),
            resource: resource_name.to_upper_camel_case(),
            reason: format!(
                "operation `{}` returns a resource declared in another service; resources must be returned by their declaring service",
                operation.name
            ),
        });
    }
    Ok(())
}

/// The proto-backed result record of an operation that holds a resource of the service, as
/// its record name, field key, resource name and proto message name.
fn operation_output_resource_holder<'a>(
    spec: &'a ApiSpec<SelectedFamily>,
    service: &'a ServiceSpec<SelectedFamily>,
    operation: &'a OperationSpec<SelectedFamily>,
) -> Result<Option<(&'a str, &'a str, &'a str, &'a str)>> {
    let record = match operation.output_type() {
        Some(TypeSpec::Record(record_name)) => spec.record(record_name.as_str()),
        Some(TypeSpec::External(ExternalTypeSpec::Proto(proto_name))) => {
            spec.record_for_proto(proto_name.as_str())
        }
        _ => None,
    };
    let Some(record) = record else {
        return Ok(None);
    };
    let resource_fields = record
        .fields
        .iter()
        .filter_map(|(key, field)| match field.field_type.without_option() {
            TypeSpec::Resource(resource) => Some((key.as_str(), resource.as_str())),
            _ => None,
        })
        .collect::<Vec<_>>();
    let [(field_key, resource_name)] = resource_fields.as_slice() else {
        if resource_fields.is_empty() {
            return Ok(None);
        }
        return Err(Error::InvalidResource {
            service: service.name.clone(),
            resource: resource_fields[0].1.to_upper_camel_case(),
            reason: format!(
                "result record `{}` of operation `{}` holds more than one resource",
                record.name, operation.name
            ),
        });
    };
    ensure_local_resource_return(service, operation, resource_name)?;
    let Some(proto_name) = record
        .source
        .as_ref()
        .and_then(|source| source.proto_type())
    else {
        return Err(Error::InvalidResource {
            service: service.name.clone(),
            resource: resource_name.to_upper_camel_case(),
            reason: format!(
                "result record `{}` of operation `{}` holds a resource, so it needs `@nexus.proto`",
                record.name, operation.name
            ),
        });
    };
    Ok(Some((
        record.full_name.as_str(),
        field_key,
        resource_name,
        proto_name.as_str(),
    )))
}

fn operation_output_ref(operation: &OperationSpec<SelectedFamily>) -> Option<&str> {
    operation.output_type()?.reference()
}

fn operation_output_resource_message_name(
    operation: &OperationSpec<SelectedFamily>,
) -> Option<&str> {
    let Some(ExternalTypeSpec::Proto(type_name)) = operation_output_resource_wire_type(operation)
    else {
        return None;
    };
    Some(type_name.as_ref())
}

fn operation_output_resource_wire_type(
    operation: &OperationSpec<SelectedFamily>,
) -> Option<&ExternalTypeSpec<SelectedFamily>> {
    let TypeSpec::Resource(resource_name) = operation.output_type()? else {
        return None;
    };
    resource_name.wire_type.as_ref()
}

/// Resolves and validates resource facts into resource data. Operation-return relationships
/// are retained for `OperationBindingPass` to attach to operations.
pub(crate) struct ResourceResolutionPass<'a> {
    descriptors: &'a DescriptorIndex,
    targets: Vec<crate::nexgen_config::GenerationTarget>,
}

impl<'a> ResourceResolutionPass<'a> {
    pub(crate) fn new(
        descriptors: &'a DescriptorIndex,
        targets: Vec<crate::nexgen_config::GenerationTarget>,
    ) -> Self {
        Self {
            descriptors,
            targets,
        }
    }

    pub(super) fn bind_spec(
        &self,
        spec: ApiSpec<SelectedFamily>,
        path: &Path,
    ) -> Result<ApiSpec<super::ResourceBoundFamily>> {
        ensure_unique_resource_names(&spec)?;
        let mut bindings = spec
            .services
            .iter()
            .map(|service| {
                resolve_service_resource_methods(&spec, service, self.descriptors).map(
                    |resources| {
                        (
                            service.name.clone(),
                            ResolvedServiceResources {
                                resources,
                                operation_returns: BTreeMap::new(),
                            },
                        )
                    },
                )
            })
            .collect::<Result<BTreeMap<_, _>>>()?;
        type_parameters::infer_and_validate(&spec, &mut bindings, path)?;
        // Return bindings determine shared wire models even when no callable API is emitted.
        for service in &spec.services {
            let resolved = bindings
                .get_mut(&service.name)
                .expect("resource resolution includes every service");
            if self.targets.is_empty() {
                for resource in &mut resolved.resources {
                    resource.methods.clear();
                }
            }
            *resolved = resolve_service_resource_returns(
                &spec,
                service,
                self.descriptors,
                std::mem::take(&mut resolved.resources),
            )?;
        }
        let mut resources = BTreeMap::new();
        let operation_returns = bindings
            .into_iter()
            .map(|(service, resolved)| {
                resources.extend(
                    resolved
                        .resources
                        .into_iter()
                        .map(|resource| (resource.name.clone(), resource)),
                );
                (service, resolved.operation_returns)
            })
            .collect();
        Ok(spec.map_names(ResourceBindingMapper {
            data: ResourceBoundData { operation_returns },
            resources,
        }))
    }
}

fn ensure_unique_resource_names(spec: &ApiSpec<SelectedFamily>) -> Result<()> {
    let mut seen_names = BTreeSet::new();
    for service in &spec.services {
        for resource in &service.resources {
            let generated_name = resource.name.to_upper_camel_case();
            if !seen_names.insert(generated_name.clone()) {
                return Err(Error::InvalidResource {
                    service: service.name.clone(),
                    resource: generated_name,
                    reason: "another resource uses the same generated name".to_string(),
                });
            }
        }
    }
    Ok(())
}

impl CompilerPass<SelectedFamily, super::ResourceBoundFamily> for ResourceResolutionPass<'_> {
    type Error = Error;

    fn transform_leaf(
        &mut self,
        leaf: ApiSpecLeaf<SelectedFamily>,
    ) -> Result<ApiSpecLeaf<super::ResourceBoundFamily>> {
        let spec = self.bind_spec(leaf.spec, &leaf.source_path)?;
        Ok(ApiSpecLeaf {
            module_path: leaf.module_path,
            source_root: leaf.source_root,
            source_path: leaf.source_path,
            spec,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use prost_types::FileDescriptorSet;

    use crate::descriptors::DescriptorIndex;
    use crate::language::Language;
    use crate::spec::{
        ApiSpec, ApiSpecLeaf, ApiSpecNode, ApiSpecTree, CompilerPass, SelectedFamily, TypeSpec,
    };

    use super::{
        RequestPlan, RequestPlanSource, ResolvedResourceMethodBinding, resolve_service_resources,
    };

    fn descriptors() -> DescriptorIndex {
        DescriptorIndex::load(
            &PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("advanced/samples/descriptors/temporal_api.bin"),
        )
        .unwrap()
    }

    fn empty_descriptors() -> DescriptorIndex {
        DescriptorIndex::from_descriptor_set(FileDescriptorSet { file: Vec::new() }).unwrap()
    }

    fn parse(language: Language, wit: &str) -> ApiSpec<SelectedFamily> {
        let temporal_types_input =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("advanced/samples/inputs/deps");
        let authored = crate::parser::parse_api_spec_from_wit_for_language_with_inputs(
            language,
            wit,
            PathBuf::from("inline.wit"),
            &[temporal_types_input],
        )
        .unwrap();
        crate::planning::select_spec(authored, language)
    }

    #[test]
    fn infers_resource_parameters_in_resource_data_for_every_target() {
        let wit = r#"
package test:pairs@1.0.0;
world system { export pairs; }
interface pairs {
  type placeholder = string;
  record entry { value: value-t, }
  /// @nexus.type-parameter
  type value-t = placeholder;
  /// @nexus.type-parameter
  type key-t = placeholder;
  resource pair {
    constructor(value: entry, key: key-t);
  }
}
"#;
        use crate::nexgen_config::GenerationTarget::{Client, Workflow};
        for targets in [vec![], vec![Workflow], vec![Client], vec![Workflow, Client]] {
            let descriptors = empty_descriptors();
            let spec = super::ResourceResolutionPass::new(&descriptors, targets)
                .bind_spec(
                    parse(Language::Python, wit),
                    std::path::Path::new("inline.wit"),
                )
                .unwrap();
            let resource = spec.services[0].resource("pair").unwrap();
            assert_eq!(
                resource
                    .data
                    .type_parameters
                    .iter()
                    .map(|parameter| parameter.name.as_str())
                    .collect::<Vec<_>>(),
                ["KeyT", "ValueT"],
            );
            let tree = ApiSpecTree {
                root: ApiSpecNode::Leaf(ApiSpecLeaf {
                    module_path: crate::spec::ModulePath::default(),
                    source_root: PathBuf::new(),
                    source_path: PathBuf::from("inline.wit"),
                    spec,
                }),
            };
            let bound = crate::planning::OperationBindingPass::new()
                .apply(tree)
                .unwrap();
            let lowered = crate::planning::OperationLoweringPass::new()
                .apply(bound)
                .unwrap();
            let crate::spec::ApiSpecNode::Leaf(leaf) = lowered.root else {
                panic!("expected one module");
            };
            assert_eq!(
                leaf.spec.services[0].resources[0]
                    .data
                    .type_parameters()
                    .iter()
                    .map(|parameter| parameter.name.as_str())
                    .collect::<Vec<_>>(),
                ["KeyT", "ValueT"],
            );
        }
    }

    #[test]
    fn propagates_parameters_through_returned_resources_to_a_fixed_point() {
        let wit = r#"
package test:holders@1.0.0;
world system { export holders; }
interface holders {
  type placeholder = string;
  /// @nexus.type-parameter
  type value-t = placeholder;
  resource opener {
    constructor(id: string);
    open: func() -> holder;
  }
  resource watcher {
    constructor(id: string);
    watch: func(holder: borrow<holder>);
  }
  resource holder {
    constructor(value: value-t);
  }
}
"#;
        let descriptors = empty_descriptors();
        for targets in [
            vec![],
            vec![crate::nexgen_config::GenerationTarget::Workflow],
        ] {
            let spec = super::ResourceResolutionPass::new(&descriptors, targets)
                .bind_spec(
                    parse(Language::Python, wit),
                    std::path::Path::new("inline.wit"),
                )
                .unwrap();
            let service = &spec.services[0];
            assert_eq!(
                service
                    .resource("opener")
                    .unwrap()
                    .data
                    .type_parameters
                    .iter()
                    .map(|parameter| parameter.name.as_str())
                    .collect::<Vec<_>>(),
                ["ValueT"],
            );
            assert!(
                service
                    .resource("watcher")
                    .unwrap()
                    .data
                    .type_parameters
                    .is_empty()
            );
        }
    }

    #[test]
    fn infers_parameters_from_implicit_bindings_even_without_an_api_target() {
        let wit = r#"
package test:senders@1.0.0;
world system { export senders; }
interface senders {
  type placeholder = string;
  /// @nexus.type-parameter
  type body-t = placeholder;
  record send-request {
    id: string,
    /// @nexus.source python="body()"
    body: body-t,
  }
  resource sender {
    constructor(id: string);
    send: func();
  }
  send: func(request: send-request);
}
"#;
        let descriptors = empty_descriptors();
        for targets in [
            vec![],
            vec![crate::nexgen_config::GenerationTarget::Workflow],
        ] {
            let emits_methods = !targets.is_empty();
            let spec = super::ResourceResolutionPass::new(&descriptors, targets)
                .bind_spec(
                    parse(Language::Python, wit),
                    std::path::Path::new("inline.wit"),
                )
                .unwrap();
            let resource = &spec.services[0].resources[0].data;
            assert_eq!(
                resource
                    .type_parameters
                    .iter()
                    .map(|parameter| parameter.name.as_str())
                    .collect::<Vec<_>>(),
                ["BodyT"],
            );
            assert_eq!(!resource.methods.is_empty(), emits_methods);
        }
    }

    #[test]
    fn binds_implicit_resource_method_to_same_named_operation() {
        let wit = r#"
package temporal:nexus@1.0.0;

world system {
  export workflow-service;
}

/// @nexus.endpoint "__temporal_system"
interface workflow-service {
  use nexus:temporal-types/model@1.0.0.{payloads, task-queue, workflow-function};

  resource started-workflow {
    constructor(namespace: string, workflow-id: string, run-id: option<string>);

    restart-workflow: func(
      workflow: workflow-function,
      task-queue: task-queue,
      input: option<payloads>,
    ) -> start-workflow-result;
  }

  /// @nexus.proto "temporal.api.workflowservice.v1.StartWorkflowExecutionRequest"
  record start-workflow-request {
    /// @nexus.proto-field "workflow_type"
    workflow: workflow-function,
    workflow-id: string,
    task-queue: task-queue,
    namespace: option<string>,
  }

  /// @nexus.proto "temporal.api.workflowservice.v1.StartWorkflowExecutionResponse"
  type start-workflow-result = own<started-workflow>;

  start-workflow: func(request: start-workflow-request) -> start-workflow-result;
  restart-workflow: func(request: start-workflow-request) -> start-workflow-result;
}
"#;
        let spec = parse(Language::Python, wit);
        let service = &spec.services[0];
        let resolved = resolve_service_resources(
            &spec,
            service,
            &descriptors(),
            &[crate::nexgen_config::GenerationTarget::Workflow],
        )
        .unwrap();
        let method = &resolved.resources[0].methods[0];

        assert_eq!(method.name, "restart-workflow");
        match &method.binding {
            ResolvedResourceMethodBinding::Operation { operation_name, .. } => {
                assert_eq!(operation_name, "RestartWorkflow");
            }
            ResolvedResourceMethodBinding::Stub => {
                panic!("restart-workflow should bind to RestartWorkflow");
            }
        }
    }

    #[test]
    fn resource_method_does_not_bind_operation_when_operation_name_differs_without_annotation() {
        let wit = r#"
package temporal:users@1.0.0;

world system {
  export user-service;
}

/// @nexus.endpoint "__user_service"
interface user-service {
  resource user {
    constructor(user-id: string, email: string);

    update-email: func(email: string) -> user-result;
  }

  type user-result = own<user>;

  record update-email-request {
    user-id: string,
    email: string,
  }

  updates-email: func(request: update-email-request) -> user-result;
}
"#;
        let spec = parse(Language::Python, wit);
        let service = &spec.services[0];
        let resolved = resolve_service_resources(
            &spec,
            service,
            &empty_descriptors(),
            &[crate::nexgen_config::GenerationTarget::Workflow],
        )
        .unwrap();
        let method = &resolved.resources[0].methods[0];

        assert_eq!(method.name, "update-email");
        assert!(matches!(
            method.binding,
            ResolvedResourceMethodBinding::Stub
        ));
    }

    #[test]
    fn binds_a_method_parameter_to_the_record_field_of_a_proto_oneof() {
        let wit = r#"
package test:oneof@1.0.0;

world system {
  export api;
}

interface api {
  /// @nexus.proto "temporal.api.workflowservice.v1.PauseActivityRequest.activity"
  variant activity-selection {
    id(string),
    %type(string),
  }

  /// @nexus.proto "temporal.api.common.v1.WorkflowExecution"
  record workflow-execution {
    workflow-id: string,
    run-id: string,
  }

  /// @nexus.proto "temporal.api.workflowservice.v1.PauseActivityRequest"
  record pause-activity-request {
    namespace: string,
    execution: option<workflow-execution>,
    identity: option<string>,
    activity: option<activity-selection>,
    reason: option<string>,
    request-id: option<string>,
  }

  resource activities {
    constructor(namespace: string);
    pause: func(activity: option<activity-selection>);
  }

  pause: func(request: pause-activity-request);
}
"#;
        let spec = parse(Language::Python, wit);
        let resolved = resolve_service_resources(
            &spec,
            &spec.services[0],
            &descriptors(),
            &[crate::nexgen_config::GenerationTarget::Workflow],
        )
        .unwrap();

        let ResolvedResourceMethodBinding::Operation { request_plan, .. } =
            &resolved.resources[0].methods[0].binding
        else {
            panic!("pause should bind to its operation");
        };
        let RequestPlan::Construct { fields, .. } = request_plan else {
            panic!("the request plan should construct the request");
        };
        assert_eq!(
            fields
                .iter()
                .map(|field| (field.field_name.as_str(), &field.value))
                .collect::<Vec<_>>(),
            [
                (
                    "namespace",
                    &RequestPlan::Source(RequestPlanSource::ResourceField("namespace".to_string()))
                ),
                (
                    "activity",
                    &RequestPlan::Source(RequestPlanSource::MethodParam("activity".to_string()))
                ),
            ]
        );
    }

    #[test]
    fn a_list_u8_parameter_that_fills_a_proto_bytes_field_is_bytes() {
        let wit = r#"
package test:bytes@1.0.0;

world system {
  export api;
}

interface api {
  /// @nexus.proto "temporal.api.common.v1.Callback.Internal"
  record callback-internal {
    data: option<list<u8>>,
  }

  resource callback {
    constructor();
    send: func(data: option<list<u8>>);
  }

  send: func(request: callback-internal);
}
"#;
        let spec = parse(Language::Python, wit);
        let resolved = resolve_service_resources(
            &spec,
            &spec.services[0],
            &descriptors(),
            &[crate::nexgen_config::GenerationTarget::Workflow],
        )
        .unwrap();

        let method = &resolved.resources[0].methods[0];
        assert!(matches!(
            method.binding,
            ResolvedResourceMethodBinding::Operation { .. }
        ));
        assert_eq!(
            method.params[0].field_type,
            TypeSpec::Option(Box::new(TypeSpec::Bytes))
        );
    }

    #[test]
    fn resource_method_can_bind_operation_when_operation_name_differs_with_annotation() {
        let wit = r#"
package temporal:users@1.0.0;

world system {
  export user-service;
}

/// @nexus.endpoint "__user_service"
interface user-service {
  resource user {
    constructor(user-id: string, email: string);

    /// @nexus.operation "updates-email"
    update-email: func(email: string) -> user-result;
  }

  type user-result = own<user>;

  record update-email-request {
    user-id: string,
    email: string,
  }

  updates-email: func(request: update-email-request) -> user-result;
}
"#;
        let spec = parse(Language::Python, wit);
        let service = &spec.services[0];
        let resolved = resolve_service_resources(
            &spec,
            service,
            &empty_descriptors(),
            &[crate::nexgen_config::GenerationTarget::Workflow],
        )
        .unwrap();
        let method = &resolved.resources[0].methods[0];

        assert_eq!(method.name, "update-email");
        match &method.binding {
            ResolvedResourceMethodBinding::Operation {
                operation_name,
                request_plan,
            } => {
                assert_eq!(operation_name, "UpdatesEmail");
                let RequestPlan::Construct {
                    message_name,
                    fields,
                } = request_plan
                else {
                    panic!("request plan should construct update-email-request");
                };
                assert_eq!(message_name, "user-service.update-email-request");
                assert_eq!(fields.len(), 2);
                assert!(fields.contains(&super::RequestPlanField {
                    field_name: "user-id".to_string(),
                    value: RequestPlan::Source(RequestPlanSource::ResourceField(
                        "user-id".to_string()
                    )),
                }));
                assert!(fields.contains(&super::RequestPlanField {
                    field_name: "email".to_string(),
                    value: RequestPlan::Source(RequestPlanSource::MethodParam("email".to_string())),
                }));
            }
            ResolvedResourceMethodBinding::Stub => {
                panic!("update-email should bind to UpdatesEmail");
            }
        }
    }

    #[test]
    fn resource_method_does_not_bind_operation_when_required_request_field_name_differs() {
        let wit = r#"
package temporal:users@1.0.0;

world system {
  export user-service;
}

/// @nexus.endpoint "__user_service"
interface user-service {
  resource user {
    constructor(user-id: string, email: string);

    update-email: func(email: string) -> user-result;
  }

  type user-result = own<user>;

  record update-email-request {
    users-id: string,
    email: string,
  }

  update-email: func(request: update-email-request) -> user-result;
}
"#;
        let spec = parse(Language::Python, wit);
        let service = &spec.services[0];
        let resolved = resolve_service_resources(
            &spec,
            service,
            &empty_descriptors(),
            &[crate::nexgen_config::GenerationTarget::Workflow],
        )
        .unwrap();
        let method = &resolved.resources[0].methods[0];

        assert_eq!(method.name, "update-email");
        assert!(matches!(
            method.binding,
            ResolvedResourceMethodBinding::Stub
        ));
    }
}
