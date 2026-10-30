//! Shared rendering of resolved resource-method request plans.

use crate::error::{Error, Result};
use crate::planning::{
    PlannedFamily, PlannedProtoType, PlannedResourceMethodBindingSpec, PlannedSpec, RequestPlan,
    RequestPlanField, RequestPlanSource,
};
use crate::spec::{ExternalTypeSourceSpec, FieldOrigin, RecordSpec};

pub(crate) fn render_request_plan<FName, FAssign, FConstruct, FResource, FParam>(
    plan: &RequestPlan,
    member_name: FName,
    render_assignment: FAssign,
    render_construct: FConstruct,
    render_resource_field_source: FResource,
    render_method_param_source: FParam,
) -> String
where
    FName: Fn(&str) -> String + Copy,
    FAssign: Fn(String, String) -> String + Copy,
    FConstruct: Fn(&str, Vec<String>, &[RequestPlanField]) -> String + Copy,
    FResource: Fn(&str) -> String + Copy,
    FParam: Fn(&str) -> String + Copy,
{
    match plan {
        RequestPlan::Source(RequestPlanSource::ResourceField(name)) => {
            render_resource_field_source(name)
        }
        RequestPlan::Source(RequestPlanSource::MethodParam(name)) => {
            render_method_param_source(name)
        }
        RequestPlan::Construct {
            message_name,
            fields,
        } => {
            let rendered_fields = fields
                .iter()
                .map(|field| {
                    render_assignment(
                        member_name(&field.field_name),
                        render_request_plan(
                            &field.value,
                            member_name,
                            render_assignment,
                            render_construct,
                            render_resource_field_source,
                            render_method_param_source,
                        ),
                    )
                })
                .collect();
            render_construct(message_name, rendered_fields, fields)
        }
    }
}

/// The record of a request plan's message, by its WIT name or its proto message name.
pub(crate) fn record_for_request_name<'a>(
    api_plan: &'a PlannedSpec,
    name: &str,
) -> Option<&'a RecordSpec<PlannedFamily>> {
    api_plan.record(name).or_else(|| {
        let name = name.trim_start_matches('.');
        api_plan.records().map(|(_, record)| record).find(|record| {
            matches!(
                record.source.as_ref().and_then(ExternalTypeSourceSpec::proto_type),
                Some(PlannedProtoType::Message(message))
                    if message.proto.full_name == name
            )
        })
    })
}

/// Whether a request plan fills a top-level request field that has a source in `target`. The
/// resource's value wins over the source, so such a resource method must not call an operation
/// wrapper that fills the field from the source.
pub(crate) fn request_plan_fills_source(
    api_plan: &PlannedSpec,
    plan: &RequestPlan,
    target: crate::nexgen_config::GenerationTarget,
) -> bool {
    let RequestPlan::Construct {
        message_name,
        fields,
    } = plan
    else {
        return false;
    };
    let Some(record) = record_for_request_name(api_plan, message_name) else {
        return false;
    };
    fields.iter().any(|field| {
        record.fields.values().any(|candidate| {
            candidate.name == field.field_name && candidate.source_for(target).is_some()
        })
    })
}

/// Whether a resource method of the service binds the operation with a plan that fills one of
/// the operation's sourced fields.
pub(crate) fn resource_method_fills_source(
    api_plan: &PlannedSpec,
    service_name: &str,
    operation_name: &str,
    target: crate::nexgen_config::GenerationTarget,
) -> bool {
    api_plan
        .services
        .iter()
        .filter(|service| service.name == service_name)
        .flat_map(|service| service.resources.iter().map(|resource| &resource.data))
        .flat_map(|resource| &resource.methods)
        .any(|method| {
            matches!(
                &method.binding,
                PlannedResourceMethodBindingSpec::Operation {
                    operation_name: name,
                    request_plan,
                    ..
                } if name == operation_name
                    && request_plan_fills_source(api_plan, request_plan, target)
            )
        })
}

/// Check each API independently, preserving the shared request and source policy.
pub(crate) fn validate_resource_origins(
    api_plan: &PlannedSpec,
    target: crate::nexgen_config::GenerationTarget,
) -> Result<()> {
    for service in &api_plan.services {
        for resource in service.resources.iter().map(|resource| &resource.data) {
            for method in &resource.methods {
                let PlannedResourceMethodBindingSpec::Operation { request_plan, .. } =
                    &method.binding
                else {
                    continue;
                };
                if let Some((field, kind)) = missing_origin_field(api_plan, request_plan, target) {
                    return Err(Error::InvalidResourceMethod {
                        service: service.name.clone(),
                        resource: resource.type_name.clone(),
                        method: method.name.clone(),
                        reason: format!(
                            "the {kind} field `{field}` has no value for the `{}` target; \
                             supply it from a resource field or method parameter, or include this target in its origin",
                            match target {
                                crate::nexgen_config::GenerationTarget::Workflow => "workflow",
                                crate::nexgen_config::GenerationTarget::Client => "client",
                            },
                        ),
                    });
                }
            }
        }
    }
    Ok(())
}

fn missing_origin_field(
    api_plan: &PlannedSpec,
    plan: &RequestPlan,
    target: crate::nexgen_config::GenerationTarget,
) -> Option<(String, &'static str)> {
    let RequestPlan::Construct {
        message_name,
        fields,
        ..
    } = plan
    else {
        return None;
    };
    if let Some(record) = record_for_request_name(api_plan, message_name) {
        for field in record.fields.values() {
            let kind = match field.origin.as_ref() {
                Some(FieldOrigin::Source { .. }) => "sourced",
                Some(FieldOrigin::Default { .. }) => "defaulted",
                _ => continue,
            };
            if field.required
                && field.source_for(target).is_none()
                && field.default_for(target).is_none()
                && fields
                    .iter()
                    .all(|planned| planned.field_name != field.name)
            {
                return Some((field.name.clone(), kind));
            }
        }
    }
    fields
        .iter()
        .find_map(|field| missing_origin_field(api_plan, &field.value, target))
}
