//! A resource is generic in each type parameter that it uses, also through records, protos or a
//! generic resource that it returns. Records and variants get their type parameters the same way.

use crate::error::{Error, Result};
use crate::spec::{
    ApiSpec, ApiSpecLeaf, CompilerPass, ResourceSpec, SelectedFamily, ServiceSpec,
    TypeParameterSpec, TypeSpec,
};

pub(crate) struct ResourceTypeParameterPass;

impl CompilerPass<SelectedFamily, SelectedFamily> for ResourceTypeParameterPass {
    type Error = Error;

    fn transform_leaf(
        &mut self,
        mut leaf: ApiSpecLeaf<SelectedFamily>,
    ) -> Result<ApiSpecLeaf<SelectedFamily>> {
        infer_resource_type_parameters(&mut leaf.spec);
        Ok(leaf)
    }
}

/// Sets the type parameters of each resource. A resource that returns a generic resource takes its
/// parameters too, so the inference repeats until no resource changes.
fn infer_resource_type_parameters(spec: &mut ApiSpec<SelectedFamily>) {
    loop {
        let mut changed = false;
        for service_index in 0..spec.services.len() {
            for resource_index in 0..spec.services[service_index].resources.len() {
                let service = &spec.services[service_index];
                let parameters =
                    used_type_parameters(spec, service, &service.resources[resource_index]);
                let resource = &mut spec.services[service_index].resources[resource_index];
                if resource.type_parameters != parameters {
                    resource.type_parameters = parameters;
                    changed = true;
                }
            }
        }
        if !changed {
            return;
        }
    }
}

/// The type parameters that the resource uses, in the order of their declarations. A method result
/// or an operation output that is a generic resource gives the type parameters of that resource.
fn used_type_parameters(
    spec: &ApiSpec<SelectedFamily>,
    service: &ServiceSpec<SelectedFamily>,
    resource: &ResourceSpec<SelectedFamily>,
) -> Vec<TypeParameterSpec> {
    let mut values = Vec::<&TypeSpec<SelectedFamily>>::new();
    let mut results = Vec::<&TypeSpec<SelectedFamily>>::new();
    values.extend(resource.fields.iter().map(|field| &field.field_type));
    for method in &resource.methods {
        values.extend(method.params.iter().map(|param| &param.field_type));
        results.extend(method.result.as_ref().map(|result| &result.result_type));
        if let Some(operation) = method
            .operation_name
            .as_deref()
            .and_then(|name| service.operation(name))
        {
            values.extend(operation.input_type());
            results.extend(operation.output_type());
        }
    }
    let returned = results
        .iter()
        .filter_map(|result| match result.without_option() {
            TypeSpec::Resource(returned) => service.resource(returned.as_str()),
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
    parameters.sort_by(|left, right| {
        (left.declaration_index, &left.full_name).cmp(&(right.declaration_index, &right.full_name))
    });
    parameters.dedup();
    parameters
}
