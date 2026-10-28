//! Shared rendering of resolved resource-method request plans.

use crate::error::{Error, Result};
use crate::planning::{
    PlannedFamily, PlannedProtoType, PlannedResource, PlannedResourceMethodBindingSpec,
    PlannedSpec, RequestPlan, RequestPlanField, RequestPlanSource,
};
use crate::spec::{ExternalTypeSourceSpec, RecordSpec};

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

/// A Workflow source has no value outside a Workflow, so a client can't call a resource method
/// whose request leaves a sourced field to its source.
pub(crate) fn validate_client_resource_sources<'a>(
    api_plan: &PlannedSpec,
    resources: impl IntoIterator<Item = &'a PlannedResource>,
) -> Result<()> {
    for resource in resources {
        for method in &resource.methods {
            let PlannedResourceMethodBindingSpec::Operation {
                request_plan:
                    RequestPlan::Construct {
                        message_name,
                        fields,
                        ..
                    },
                ..
            } = &method.binding
            else {
                continue;
            };
            let Some(record) = record_for_request_name(api_plan, message_name) else {
                continue;
            };
            let left_to_source = record
                .sourced_fields()
                .map(|(name, _, _)| name)
                .find(|name| fields.iter().all(|field| field.field_name != *name));
            if let Some(field_name) = left_to_source {
                return Err(Error::UnsupportedClientTarget {
                    reason: format!(
                        "can't call resource method `{}.{}`, whose request leaves the sourced \
                         field `{field_name}` to its Workflow source",
                        resource.type_name, method.name
                    ),
                });
            }
        }
    }
    Ok(())
}
