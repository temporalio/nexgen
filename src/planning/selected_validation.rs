//! Validate the chosen generic surface after selection and before binding.
//! Authored descriptor checks remain in `AuthoredValidationPass`; this pass
//! consumes selected IR without cloning or selecting authored alternatives.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::error::{Error, Result};
use crate::spec::{
    ApiSpec, ApiSpecLeaf, CompilerPass, FunctionArgsSpec, FunctionResultSpec, ResourceSpec,
    SelectedFamily, ServiceSpec, TypeSpec,
};

pub(crate) struct SelectedValidationPass;

impl CompilerPass<SelectedFamily, SelectedFamily> for SelectedValidationPass {
    type Error = Error;

    fn transform_leaf(
        &mut self,
        leaf: ApiSpecLeaf<SelectedFamily>,
    ) -> Result<ApiSpecLeaf<SelectedFamily>> {
        validate_generic_model_semantics(&leaf.spec, &leaf.source_path)?;
        Ok(leaf)
    }
}

fn validate_generic_model_semantics(spec: &ApiSpec<SelectedFamily>, path: &Path) -> Result<()> {
    let invalid = |reason: String| Error::InvalidWit {
        path: path.to_path_buf(),
        reason,
    };
    for (_, record) in spec.records() {
        let parameters = spec.record_type_parameters(&record.full_name);
        let mut generated_names = BTreeMap::<String, String>::new();
        for usage in &parameters {
            if let Some(previous) = generated_names.insert(
                usage.parameter.name.clone(),
                usage.parameter.full_name.clone(),
            ) && previous != usage.parameter.full_name
            {
                return Err(invalid(format!(
                    "type parameters `{previous}` and `{}` both generate the name `{}`",
                    usage.parameter.full_name, usage.parameter.name
                )));
            }
        }
        for (field_name, function) in record.functions() {
            let uses_parameter = match &function.result {
                FunctionResultSpec::Authored(result) => !spec.type_parameters(result).is_empty(),
                FunctionResultSpec::Annotation(_) => false,
            } || match &function.args {
                FunctionArgsSpec::Varargs { prefix, .. } | FunctionArgsSpec::Fixed(prefix) => {
                    prefix
                        .iter()
                        .any(|arg| !spec.type_parameters(&arg.field_type).is_empty())
                }
            } || function
                .alternate_type
                .as_ref()
                .is_some_and(|alternate| !spec.type_parameters(alternate).is_empty());
            if uses_parameter {
                return Err(invalid(format!(
                    "record `{}` field `{field_name}` uses a type parameter in function-signature metadata, which is not supported",
                    record.full_name
                )));
            }
        }
    }

    for (_, variant) in spec.variants() {
        let parameters = spec.variant_type_parameters(&variant.full_name);
        let mut generated_names = BTreeMap::<String, String>::new();
        for usage in parameters {
            if let Some(previous) = generated_names.insert(
                usage.parameter.name.clone(),
                usage.parameter.full_name.clone(),
            ) && previous != usage.parameter.full_name
            {
                return Err(invalid(format!(
                    "variant `{}` uses type parameters `{previous}` and `{}` that both generate the name `{}`",
                    variant.full_name, usage.parameter.full_name, usage.parameter.name
                )));
            }
        }
    }

    for service in &spec.services {
        for operation in &service.operations {
            let mut generated_names = BTreeMap::<String, String>::new();
            let operation_parameters = operation
                .input_type()
                .into_iter()
                .chain(operation.output_type())
                .flat_map(|value| spec.type_parameters(value));
            for usage in operation_parameters {
                if let Some(previous) = generated_names.insert(
                    usage.parameter.name.clone(),
                    usage.parameter.full_name.clone(),
                ) && previous != usage.parameter.full_name
                {
                    return Err(invalid(format!(
                        "operation `{}` uses type parameters `{previous}` and `{}` that both generate the name `{}`",
                        operation.name, usage.parameter.full_name, usage.parameter.name
                    )));
                }
            }
        }
        for resource in &service.resources {
            validate_resource_type_parameters(spec, service, resource, &invalid)?;
        }
        for operation in &service.operations {
            let Some(TypeSpec::Resource(returned)) = operation.output_type() else {
                continue;
            };
            if returned.wire_type.is_none()
                && service
                    .resource(returned.as_str())
                    .is_some_and(|resource| !resource.type_parameters.is_empty())
            {
                return Err(invalid(format!(
                    "operation `{}` returns the generic resource `{}` with no `@nexus.proto` \
                     message to hold its type arguments",
                    operation.name, returned
                )));
            }
        }
    }

    // A record that holds a resource builds it from the record's message, with no type argument.
    let generic_resources = spec
        .services
        .iter()
        .flat_map(|service| &service.resources)
        .filter(|resource| !resource.type_parameters.is_empty())
        .map(|resource| resource.name.as_str())
        .collect::<BTreeSet<_>>();
    for (_, record) in spec.records() {
        for (field_name, field) in &record.fields {
            if let TypeSpec::Resource(resource) = field.field_type.without_option()
                && generic_resources.contains(resource.as_str())
            {
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

/// A resource may use only the type parameters that it declares, and it must use each of them.
/// The operations that its methods bind may use only these too.
fn validate_resource_type_parameters(
    spec: &ApiSpec<SelectedFamily>,
    service: &ServiceSpec<SelectedFamily>,
    resource: &ResourceSpec<SelectedFamily>,
    invalid: &dyn Fn(String) -> Error,
) -> Result<()> {
    let declared = resource
        .type_parameters
        .iter()
        .map(|parameter| parameter.full_name.as_str())
        .collect::<BTreeSet<_>>();
    // A returned generic resource takes its type arguments from the resource that returns it.
    let parameters = |value: &TypeSpec<SelectedFamily>| {
        let returned = match value.without_option() {
            TypeSpec::Resource(returned) => service
                .resource(returned.as_str())
                .map(|returned| returned.type_parameters.clone())
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        spec.type_parameters_through_protos(value)
            .into_iter()
            .map(|usage| usage.parameter)
            .chain(returned)
            .collect::<Vec<_>>()
    };
    let undeclared = |value: &TypeSpec<SelectedFamily>| {
        parameters(value)
            .into_iter()
            .find(|parameter| !declared.contains(parameter.full_name.as_str()))
    };
    let resource_types = resource
        .fields
        .iter()
        .map(|field| &field.field_type)
        .chain(resource.methods.iter().flat_map(|method| {
            method
                .params
                .iter()
                .map(|param| &param.field_type)
                .chain(method.result.as_ref().map(|result| &result.result_type))
        }))
        .collect::<Vec<_>>();
    if let Some(parameter) = resource_types.iter().find_map(|value| undeclared(value)) {
        return Err(invalid(format!(
            "resource `{}` uses the type parameter `{}`, which it doesn't declare. Add \
             `@nexus.type-parameter \"<alias>\"` to the resource for it",
            resource.name, parameter.full_name
        )));
    }
    let used = resource_types
        .iter()
        .flat_map(|value| parameters(value))
        .map(|parameter| parameter.full_name)
        .collect::<BTreeSet<_>>();
    if let Some(unused) = resource
        .type_parameters
        .iter()
        .find(|parameter| !used.contains(&parameter.full_name))
    {
        return Err(invalid(format!(
            "resource `{}` declares the type parameter `{}`, which none of its fields, \
             parameters or results uses",
            resource.name, unused.full_name
        )));
    }

    for method in &resource.methods {
        let Some(operation_name) = &method.operation_name else {
            continue;
        };
        let Some(operation) = service.operation(operation_name) else {
            continue;
        };
        if let Some(parameter) = operation
            .input_type()
            .into_iter()
            .chain(operation.output_type())
            .find_map(undeclared)
        {
            return Err(invalid(format!(
                "resource-bound operation `{operation_name}` uses the type parameter `{}`, which \
                 resource `{}` doesn't declare",
                parameter.full_name, resource.name
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::descriptors::DescriptorIndex;
    use crate::language::Language;
    use crate::planning::{AuthoredValidationPass, LanguageSelectionPass};
    use crate::spec::ApiSpecTree;

    fn validate(language: Language, wit: &str) -> Result<()> {
        let spec = crate::parser::parse_api_spec_from_wit_for_language_with_inputs(
            language,
            wit,
            PathBuf::from("inline.wit"),
            &[PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("advanced/samples/inputs/deps")],
        )?;
        let descriptors = DescriptorIndex::load_many(&[])?;
        let authored =
            AuthoredValidationPass::new(&descriptors, language).apply(ApiSpecTree::single(spec))?;
        let selected = LanguageSelectionPass::new(language)
            .apply(authored)
            .expect("language selection is infallible");
        SelectedValidationPass.apply(selected).map(|_| ())
    }

    const GENERIC_WIT: &str = r#"
package temporal:nexus@1.0.0;
world system { export generic-service; }
interface generic-service {
  use nexus:temporal-types/model@1.0.0.{placeholder};
  /// @nexus.type-parameter
  type context-t = placeholder;
  /// @nexus.type-parameter
  type output-t = placeholder;
  record inner { value: context-t, }
  record request { nested: inner, values: list<context-t>, }
  record response { context: context-t, output: output-t, }
  complete: func(request: request) -> response;
}
"#;

    #[test]
    fn accepts_generic_models() {
        validate(Language::Dotnet, GENERIC_WIT).unwrap();
    }
}
