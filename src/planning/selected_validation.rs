//! Validate the chosen generic surface after selection and before binding.
//! Authored descriptor checks remain in `AuthoredValidationPass`; this pass
//! consumes selected IR without cloning or selecting authored alternatives.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::error::{Error, Result};
use crate::spec::{
    ApiSpec, ApiSpecLeaf, CompilerPass, ExternalTypeSpec, FunctionArgsSpec, FunctionResultSpec,
    SelectedFamily, TypeSpec,
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

/// The generic resource that a value holds, also in a list, map, tuple, result or variant case.
/// The loop over the records checks the fields of a nested record.
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::descriptors::DescriptorIndex;
    use crate::language::Language;
    use crate::planning::{
        AuthoredValidationPass, LanguageSelectionPass, ResourceTypeParameterPass,
    };
    use crate::spec::{ApiSpecNode, ApiSpecTree};

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
        ResourceTypeParameterPass.apply(selected)
    }

    fn validate(language: Language, wit: &str) -> Result<()> {
        SelectedValidationPass
            .apply(selected(language, wit)?)
            .map(|_| ())
    }

    fn resource_type_parameters(wit: &str, resource: &str) -> Vec<String> {
        let tree = selected(Language::Python, wit).unwrap();
        let ApiSpecNode::Leaf(leaf) = tree.root else {
            panic!("expected one module");
        };
        leaf.spec
            .services
            .iter()
            .find_map(|service| service.resource(resource))
            .expect("the service has the resource")
            .type_parameters
            .iter()
            .map(|parameter| parameter.name.clone())
            .collect()
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

    // The resource has no annotation. It holds the parameter only through its records.
    const RESOURCE_WIT: &str = r#"
package temporal:nexus@1.0.0;
world system { export handles; }
/// @nexus.endpoint "handles"
interface handles {
  use nexus:temporal-types/model@1.0.0.{placeholder};
  /// @nexus.type-parameter
  type body-t = placeholder;
  /// @nexus.type-parameter
  type key-t = placeholder;
  record item { body: body-t, }
  record page { items: list<item>, }
  record put-request { id: string, item: item, }
  record put-response {}
  record get-request { id: string, }
  resource handle {
    constructor(id: string);
    /// @nexus.operation "put"
    put: func(item: item) -> put-response;
    /// @nexus.operation "get"
    get: func() -> page;
  }
  resource plain {
    constructor(id: string, key: string);
  }
  put: func(request: put-request) -> put-response;
  get: func(request: get-request) -> page;
}
"#;

    #[test]
    fn a_resource_is_generic_in_the_parameters_of_its_records() {
        assert_eq!(resource_type_parameters(RESOURCE_WIT, "handle"), ["BodyT"]);
        assert!(resource_type_parameters(RESOURCE_WIT, "plain").is_empty());
    }

    #[test]
    fn a_resource_takes_its_type_parameters_in_the_order_of_their_declarations() {
        // The record and the constructor use value-t first.
        let wit = r#"
package test:pairs@1.0.0;
world system { export pairs; }
interface pairs {
  type placeholder = string;
  record entry { value: value-t, }
  /// @nexus.type-parameter
  type key-t = placeholder;
  /// @nexus.type-parameter
  type value-t = placeholder;
  resource pair {
    constructor(value: entry, key: key-t);
  }
}
"#;
        assert_eq!(resource_type_parameters(wit, "pair"), ["KeyT", "ValueT"]);
    }

    // `opener` comes before `holder`, so the first pass sees `holder` before it is generic.
    const RETURNED_RESOURCE_WIT: &str = r#"
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

    #[test]
    fn a_resource_takes_the_type_parameters_of_a_resource_that_it_returns() {
        assert_eq!(
            resource_type_parameters(RETURNED_RESOURCE_WIT, "opener"),
            ["ValueT"]
        );
    }

    #[test]
    fn a_resource_parameter_does_not_give_its_type_parameters() {
        assert!(resource_type_parameters(RETURNED_RESOURCE_WIT, "watcher").is_empty());
    }

    #[test]
    fn refuses_a_generic_resource_inside_a_record_field() {
        for field_type in [
            "list<job>",
            "option<tuple<string, job>>",
            "result<job, string>",
            "map<string, job>",
            "job-case",
        ] {
            let wit = format!(
                r#"
package test:jobs@1.0.0;
world system {{ export jobs; }}
interface jobs {{
  type placeholder = string;
  /// @nexus.type-parameter
  type value-t = placeholder;
  resource job {{
    constructor(value: value-t);
  }}
  variant job-case {{ none, some(job), }}
  record holder {{ jobs: {field_type}, }}
}}
"#
            );
            let error = validate(Language::Python, &wit).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("record `jobs.holder` field `jobs` holds the generic resource `job`"),
                "{field_type}: {error}"
            );
        }
    }
}
