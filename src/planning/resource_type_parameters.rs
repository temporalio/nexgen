//! Infer the type parameters of each resource from the types that it uses, as records and
//! variants infer theirs. A resource that uses a type parameter, also through records, protos or
//! another generic resource, is generic in it.

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

/// The type parameters of the resource's fields, of its methods, and of the operations that they
/// bind, in the order that they first appear.
fn used_type_parameters(
    spec: &ApiSpec<SelectedFamily>,
    service: &ServiceSpec<SelectedFamily>,
    resource: &ResourceSpec<SelectedFamily>,
) -> Vec<TypeParameterSpec> {
    let mut values = Vec::<&TypeSpec<SelectedFamily>>::new();
    values.extend(resource.fields.iter().map(|field| &field.field_type));
    for method in &resource.methods {
        values.extend(method.params.iter().map(|param| &param.field_type));
        values.extend(method.result.as_ref().map(|result| &result.result_type));
        if let Some(operation) = method
            .operation_name
            .as_deref()
            .and_then(|name| service.operation(name))
        {
            values.extend(operation.input_type());
            values.extend(operation.output_type());
        }
    }
    let mut parameters = Vec::<TypeParameterSpec>::new();
    for value in values {
        let returned = match value.without_option() {
            TypeSpec::Resource(returned) => service
                .resource(returned.as_str())
                .map(|returned| returned.type_parameters.clone())
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        for parameter in spec
            .type_parameters_through_protos(value)
            .into_iter()
            .map(|usage| usage.parameter)
            .chain(returned)
        {
            if !parameters.contains(&parameter) {
                parameters.push(parameter);
            }
        }
    }
    parameters
}
