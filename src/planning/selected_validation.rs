//! Validate the chosen generic surface after selection and before binding.
//! Authored descriptor checks remain in `AuthoredValidationPass`; this pass
//! consumes selected IR without cloning or selecting authored alternatives.

use std::collections::BTreeMap;
use std::path::Path;

use crate::error::{Error, Result};
use crate::nexgen_config::GenerationTarget;
use crate::spec::{
    ApiSpec, ApiSpecLeaf, CompilerPass, ExternalTypeSpec, FieldOrigin, FunctionArgsSpec,
    FunctionResultSpec, OperationSpec, RecordFieldSpec, RecordSpec, SelectedFamily, TypeSpec,
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
    }

    for (_, record) in spec.records() {
        for (field_name, field) in &record.fields {
            if field.flatten_in_api {
                validate_field_flatten(spec, record, field_name, field, &invalid)?;
            }
        }
        validate_flattened_field_names(spec, record, &invalid)?;
    }
    // Flattening spreads a request's fields as call arguments, so the request can't
    // also carry a default that would let the whole request be omitted.
    for service in &spec.services {
        for operation in &service.operations {
            let Some(record) = operation_input_record(spec, operation) else {
                continue;
            };
            let flattens = record.fields.values().any(|field| field.flatten_in_api);
            let has_default = record
                .fields
                .values()
                .any(|field| field.default_for(GenerationTarget::Workflow).is_some());
            if flattens && has_default {
                return Err(Error::UnsupportedFlatten {
                    operation: operation.name.clone(),
                    reason: "its request has a `@nexus.default` too".to_string(),
                });
            }
        }
    }
    // A resource method builds the whole request, so it can't bind a flattened call.
    for service in &spec.services {
        for method in service
            .resources
            .iter()
            .flat_map(|resource| &resource.methods)
        {
            let Some(operation) = method
                .operation_name
                .as_deref()
                .and_then(|name| service.operation(name))
            else {
                continue;
            };
            let flattens = operation_input_record(spec, operation)
                .is_some_and(|record| record.fields.values().any(|field| field.flatten_in_api));
            if flattens {
                return Err(invalid(format!(
                    "resource method `{}` binds operation `{}`, whose request flattens a field",
                    method.name, operation.name
                )));
            }
        }
    }

    Ok(())
}

fn operation_input_record<'a>(
    spec: &'a ApiSpec<SelectedFamily>,
    operation: &OperationSpec<SelectedFamily>,
) -> Option<&'a RecordSpec<SelectedFamily>> {
    operation
        .input_type()
        .and_then(|input| match input.without_option() {
            TypeSpec::Record(record) => spec.record(record.as_str()),
            TypeSpec::External(ExternalTypeSpec::Proto(proto)) => {
                spec.record_for_proto(proto.as_str())
            }
            _ => None,
        })
}

fn flattened_record<'a>(
    spec: &'a ApiSpec<SelectedFamily>,
    field: &RecordFieldSpec<SelectedFamily>,
) -> Option<&'a RecordSpec<SelectedFamily>> {
    match field.field_type.without_option() {
        TypeSpec::Record(record) => spec.record(record.as_str()),
        TypeSpec::External(ExternalTypeSpec::Proto(proto)) => spec.record_for_proto(proto.as_str()),
        _ => None,
    }
}

/// A flattened field needs a proto-backed record.
fn validate_field_flatten(
    spec: &ApiSpec<SelectedFamily>,
    parent: &RecordSpec<SelectedFamily>,
    field_name: &str,
    field: &RecordFieldSpec<SelectedFamily>,
    invalid: &dyn Fn(String) -> Error,
) -> Result<()> {
    let context = format!("record `{}` field `{field_name}`", parent.full_name);
    let Some(record) = flattened_record(spec, field).filter(|record| record.source.is_some())
    else {
        return Err(invalid(format!(
            "{context} flattens its record, but its type isn't a proto-backed record"
        )));
    };
    if record.flatten_in_api {
        return Err(invalid(format!(
            "{context} flattens `{}`, which flattens in every call already",
            record.full_name
        )));
    }
    if field.origin.is_some() {
        return Err(invalid(format!(
            "{context} flattens its record, so it can't have a source, a default or an api-omit too"
        )));
    }
    Ok(())
}

/// The call takes the record's fields next to the others, so no two names can match in any case.
fn validate_flattened_field_names(
    spec: &ApiSpec<SelectedFamily>,
    record: &RecordSpec<SelectedFamily>,
    invalid: &dyn Fn(String) -> Error,
) -> Result<()> {
    let normalized = |name: &str| {
        name.chars()
            .filter(|character| !matches!(character, '-' | '_'))
            .flat_map(char::to_lowercase)
            .collect::<String>()
    };
    let omitted = |field: &RecordFieldSpec<SelectedFamily>| {
        matches!(field.origin, Some(FieldOrigin::Omitted))
    };
    let parameters = record
        .fields
        .iter()
        .filter(|(_, field)| !omitted(field) && !field.flatten_in_api)
        .map(|(field_name, field)| (normalized(&field.name), field_name))
        .collect::<BTreeMap<_, _>>();
    let mut flattened_names = BTreeMap::<String, String>::new();
    for (field_name, field) in record
        .fields
        .iter()
        .filter(|(_, field)| field.flatten_in_api)
    {
        let Some(nested) = flattened_record(spec, field) else {
            continue;
        };
        for (key, nested_field) in nested.fields.iter().filter(|(_, field)| !omitted(field)) {
            let name = normalized(&nested_field.name);
            if let Some(sibling) = parameters.get(&name) {
                return Err(invalid(format!(
                    "record `{}` field `{field_name}` flattens `{key}`, whose name `{}` is also \
                     the name of field `{sibling}`",
                    record.full_name, nested_field.name
                )));
            }
            let owner = format!("`{key}` of field `{field_name}`");
            if let Some(first) = flattened_names.insert(name, owner.clone()) {
                return Err(invalid(format!(
                    "record `{}` flattens {first} and {owner}, which have one parameter name",
                    record.full_name
                )));
            }
        }
    }
    let flattened_list = |value: &TypeSpec<SelectedFamily>| {
        let TypeSpec::List(item) = value.without_option() else {
            return None;
        };
        let TypeSpec::Record(record) = item.as_ref() else {
            return None;
        };
        let record = spec
            .record(record.as_str())
            .filter(|record| record.flatten_in_api)?;
        let fields = record
            .fields
            .values()
            .filter(|field| !matches!(field.origin, Some(FieldOrigin::Omitted)))
            .count();
        // TypeScript sends a record with no proto as it is, so only a proto converter can wrap.
        let reason = if fields != 1 {
            format!(
                "which flattens with {fields} fields. A list flattens a record with one field only"
            )
        } else if record.source.is_none() {
            "which flattens with no `@nexus.proto`. A list flattens a proto-backed record only"
                .to_string()
        } else {
            return None;
        };
        Some((record.full_name.clone(), reason))
    };
    let lists = spec
        .records()
        .flat_map(|(_, record)| {
            record.fields.iter().map(move |(field_name, field)| {
                (
                    format!("record `{}` field `{field_name}`", record.full_name),
                    &field.field_type,
                )
            })
        })
        .chain(spec.services.iter().flat_map(|service| {
            service.resources.iter().flat_map(|resource| {
                resource.methods.iter().flat_map(move |method| {
                    method.params.iter().map(move |param| {
                        (
                            format!(
                                "resource `{}` method `{}` parameter `{}`",
                                resource.name, method.name, param.name
                            ),
                            &param.field_type,
                        )
                    })
                })
            })
        }));
    for (context, value) in lists {
        if let Some((record, reason)) = flattened_list(value) {
            return Err(invalid(format!(
                "{context} is a list of `{record}`, {reason}"
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

    fn selected(language: Language, wit: &str) -> Result<ApiSpecTree<SelectedFamily>> {
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
        Ok(selected)
    }

    fn validate(language: Language, wit: &str) -> Result<()> {
        SelectedValidationPass
            .apply(selected(language, wit)?)
            .map(|_| ())
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
