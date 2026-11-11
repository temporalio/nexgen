//! Resource resolution infers and validates generic resources before binding their returns.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::error::{Error, Result};
use crate::spec::{
    ApiSpec, ExternalTypeSpec, ResourceSpec, SelectedFamily, ServiceSpec, TypeParameterSpec,
    TypeSpec,
};

use super::{ResolvedResourceMethodBinding, ResolvedServiceResources};

pub(super) fn infer_and_validate(
    spec: &ApiSpec<SelectedFamily>,
    bindings: &mut BTreeMap<String, ResolvedServiceResources>,
    path: &Path,
) -> Result<()> {
    infer_resource_type_parameters(spec, bindings);
    validate_generic_resources(spec, bindings, path)
}

/// A resource takes the type parameters of a resource that it returns, so the loop repeats.
fn infer_resource_type_parameters(
    spec: &ApiSpec<SelectedFamily>,
    bindings: &mut BTreeMap<String, ResolvedServiceResources>,
) {
    loop {
        let mut changed = false;
        for service in &spec.services {
            let resolved = bindings
                .get_mut(&service.name)
                .expect("resource resolution includes every service");
            for (index, resource) in service.resources.iter().enumerate() {
                let parameters = used_type_parameters(spec, service, resource, resolved, index);
                if resolved.resources[index].type_parameters != parameters {
                    resolved.resources[index].type_parameters = parameters;
                    changed = true;
                }
            }
        }
        if !changed {
            return;
        }
    }
}

/// Only a method result or an operation output gives the type parameters of a returned resource.
fn used_type_parameters(
    spec: &ApiSpec<SelectedFamily>,
    service: &ServiceSpec<SelectedFamily>,
    resource: &ResourceSpec<SelectedFamily>,
    resolved: &ResolvedServiceResources,
    resource_index: usize,
) -> Vec<TypeParameterSpec> {
    let mut values = Vec::<&TypeSpec<SelectedFamily>>::new();
    let mut results = Vec::<&TypeSpec<SelectedFamily>>::new();
    values.extend(resource.fields.iter().map(|field| &field.field_type));
    for method in &resource.methods {
        values.extend(method.params.iter().map(|param| &param.field_type));
        results.extend(method.result.as_ref().map(|result| &result.result_type));
    }
    // Use the resolved binding for explicit and implicit methods, on every target.
    let operations = resolved.resources[resource_index]
        .methods
        .iter()
        .filter_map(|method| match &method.binding {
            ResolvedResourceMethodBinding::Operation { operation_name, .. } => {
                service.operation(operation_name)
            }
            ResolvedResourceMethodBinding::Stub => None,
        });
    for operation in operations {
        values.extend(operation.input_type());
        results.extend(operation.output_type());
    }
    let returned = results
        .iter()
        .filter_map(|result| match result.without_option() {
            TypeSpec::Resource(returned) => resolved
                .resources
                .iter()
                .find(|resource| resource.name == returned.as_str()),
            _ => None,
        })
        .flat_map(|returned| returned.type_parameters.iter().cloned());
    let mut parameters = values
        .iter()
        .chain(&results)
        .flat_map(|value| spec.type_parameters_through_protos(value))
        .map(|usage| usage.parameter)
        .chain(returned)
        .collect::<Vec<_>>();
    // `full_name` is a type parameter's identity, so this order stays stable when a method moves.
    parameters.sort_by(|left, right| left.full_name.cmp(&right.full_name));
    parameters.dedup();
    parameters
}

fn validate_generic_resources(
    spec: &ApiSpec<SelectedFamily>,
    bindings: &BTreeMap<String, ResolvedServiceResources>,
    path: &Path,
) -> Result<()> {
    let invalid = |reason: String| Error::InvalidWit {
        path: path.to_path_buf(),
        reason,
    };
    let generic_resources = bindings
        .values()
        .flat_map(|service| &service.resources)
        .filter(|resource| !resource.type_parameters.is_empty())
        .map(|resource| resource.name.as_str())
        .collect::<BTreeSet<_>>();
    for service in &spec.services {
        for operation in &service.operations {
            let Some(TypeSpec::Resource(returned)) = operation.output_type() else {
                continue;
            };
            if returned.wire_type.is_none()
                && service.resource(returned.as_str()).is_some()
                && generic_resources.contains(returned.as_str())
            {
                return Err(invalid(format!(
                    "operation `{}` returns the generic resource `{}` with no `@nexus.proto` \
                     message, which isn't supported yet",
                    operation.name, returned
                )));
            }
        }
    }
    // A record builds its held resource from data, with no runtime type arguments.
    for (_, record) in spec.records() {
        for (field_name, field) in &record.fields {
            if let Some(resource) = held_generic_resource(
                spec,
                &field.field_type,
                &generic_resources,
                &mut BTreeSet::new(),
            ) {
                return Err(invalid(format!(
                    "record `{}` field `{field_name}` holds the generic resource `{resource}`, \
                     which is not supported",
                    record.full_name
                )));
            }
        }
    }
    Ok(())
}

/// The loop over the records checks a nested record, so the walk stops at one.
fn held_generic_resource(
    spec: &ApiSpec<SelectedFamily>,
    value: &TypeSpec<SelectedFamily>,
    generic_resources: &BTreeSet<&str>,
    visiting: &mut BTreeSet<String>,
) -> Option<String> {
    let mut held = |value: &TypeSpec<SelectedFamily>| {
        held_generic_resource(spec, value, generic_resources, visiting)
    };
    match value {
        TypeSpec::Resource(resource) => generic_resources
            .contains(resource.as_str())
            .then(|| resource.as_str().to_string()),
        TypeSpec::Option(inner) | TypeSpec::List(inner) => held(inner),
        TypeSpec::Tuple(items) => items.iter().find_map(held),
        TypeSpec::Map(key, value) => held(key).or_else(|| held(value)),
        TypeSpec::Result { ok, err } => ok.iter().chain(err).find_map(|value| held(value)),
        TypeSpec::External(ExternalTypeSpec::Alias(alias)) => held(&alias.target),
        TypeSpec::Variant(variant) => {
            if !visiting.insert(variant.as_ref().to_string()) {
                return None;
            }
            let resource = spec
                .variant(variant.as_ref())
                .into_iter()
                .flat_map(|variant| &variant.cases)
                .filter_map(|case| case.payload.as_ref())
                .find_map(|payload| {
                    held_generic_resource(spec, payload, generic_resources, visiting)
                });
            visiting.remove(variant.as_ref());
            resource
        }
        _ => None,
    }
}
