//! `LanguageSelectionPass` converts authored API metadata into target-selected
//! IR by resolving language defaults and overrides exactly once.
//!
//! It consumes `ApiSpecTree<AuthoredFamily>` and produces
//! `ApiSpecTree<SelectedFamily>`.
//!
//! Target selection is the boundary between authored API metadata and the
//! language-specific compiler pipeline.
//!
//! Parsing preserves authored defaults and overrides. This pass applies the
//! existing override precedence once, before planning or emission.

use crate::language::Language;
use crate::spec::{
    ApiSpec, ApiSpecTransform, AuthoredFamily, AuthoredResourceType, FunctionTypeDescriptorSpec,
    JsonModelSpec, LanguageStringSpec, SelectedFamily, SelectedResourceType, SelectedTextSpec,
    Symbol,
};
use crate::spec::{ApiSpecLeaf, CompilerPass};

/// Select target text and normalize descriptors for the selected surface.
pub(crate) fn select_spec(spec: ApiSpec, language: Language) -> ApiSpec<SelectedFamily> {
    spec.map_names(Selector { language })
}

struct Selector {
    language: Language,
}

impl ApiSpecTransform<AuthoredFamily, SelectedFamily> for Selector {
    fn map_spec_data(&mut self, _data: ()) {}
    fn map_record(&mut self, name: Symbol) -> Symbol {
        name
    }
    fn map_enum(&mut self, name: Symbol) -> Symbol {
        name
    }
    fn map_flags(&mut self, name: Symbol) -> Symbol {
        name
    }
    fn map_variant(&mut self, name: Symbol) -> Symbol {
        name
    }
    fn map_resource(&mut self, resource: AuthoredResourceType) -> SelectedResourceType {
        resource.map_names_with(self)
    }
    fn map_proto(&mut self, name: Symbol) -> Symbol {
        name
    }
    fn map_json(&mut self, name: JsonModelSpec<Symbol>) -> JsonModelSpec<Symbol> {
        name
    }
    fn map_alias(&mut self, name: Symbol) -> Symbol {
        name
    }
    fn map_service_data(&mut self, _name: &str, _data: ()) {}
    fn map_record_data(&mut self, _full_name: &str, _data: ()) {}
    fn map_resource_data(&mut self, _name: &str, _data: ()) {}
    fn map_operation_data(&mut self, _name: &str, _data: ()) {}
    fn map_field_data(&mut self, _record: &str, _field: &str, _data: ()) {}

    fn map_function_type_descriptor(
        &mut self,
        descriptor: FunctionTypeDescriptorSpec<AuthoredFamily>,
    ) -> Option<FunctionTypeDescriptorSpec<SelectedFamily>> {
        let descriptor = FunctionTypeDescriptorSpec::<SelectedFamily> {
            value_type: self.map_text(descriptor.value_type),
            args_type: self.map_text(descriptor.args_type),
        };
        (descriptor.value_type.value().is_some() && descriptor.args_type.value().is_some())
            .then_some(descriptor)
    }

    fn map_text(&mut self, text: LanguageStringSpec) -> SelectedTextSpec {
        SelectedTextSpec {
            value: text.for_language(self.language).map(ToOwned::to_owned),
            import: text
                .import_for_language(self.language)
                .map(ToOwned::to_owned),
        }
    }
}

pub(crate) struct LanguageSelectionPass {
    language: Language,
}

impl LanguageSelectionPass {
    pub(crate) fn new(language: Language) -> Self {
        Self { language }
    }
}

impl CompilerPass<AuthoredFamily, SelectedFamily> for LanguageSelectionPass {
    type Error = std::convert::Infallible;

    fn transform_leaf(
        &mut self,
        leaf: ApiSpecLeaf<AuthoredFamily>,
    ) -> Result<ApiSpecLeaf<SelectedFamily>, Self::Error> {
        Ok(ApiSpecLeaf {
            module_path: leaf.module_path,
            source_root: leaf.source_root,
            source_path: leaf.source_path,
            spec: select_spec(leaf.spec, self.language),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::language::Language;
    use crate::spec::{
        AliasTypeSpec, ApiSpec, ApiSpecTransform, AuthoredResourceType, ExternalTypeSpec,
        FunctionArgsSpec, FunctionFieldSpec, FunctionResultSpec, FunctionTypeDescriptorSpec,
        LanguageStringSpec, RecordFieldSpec, RecordFieldVisibility, RecordSpec, ServiceSpec,
        Symbol, TypeDeclSpec, TypeSpec,
    };
    use crate::spec::{ApiSpecNode, ApiSpecTree, CompilerPass};

    use super::{LanguageSelectionPass, Selector, select_spec};

    struct Identity;

    impl ApiSpecTransform<crate::spec::AuthoredFamily, crate::spec::AuthoredFamily> for Identity {
        fn map_spec_data(&mut self, _data: ()) {}
        fn map_record(&mut self, name: Symbol) -> Symbol {
            name
        }
        fn map_enum(&mut self, name: Symbol) -> Symbol {
            name
        }
        fn map_flags(&mut self, name: Symbol) -> Symbol {
            name
        }
        fn map_variant(&mut self, name: Symbol) -> Symbol {
            name
        }
        fn map_resource(&mut self, name: AuthoredResourceType) -> AuthoredResourceType {
            name
        }
        fn map_proto(&mut self, name: Symbol) -> Symbol {
            name
        }
        fn map_json(
            &mut self,
            name: crate::spec::JsonModelSpec<Symbol>,
        ) -> crate::spec::JsonModelSpec<Symbol> {
            name
        }
        fn map_alias(&mut self, name: Symbol) -> Symbol {
            name
        }
        fn map_service_data(&mut self, _name: &str, _data: ()) {}
        fn map_record_data(&mut self, _name: &str, _data: ()) {}
        fn map_resource_data(&mut self, _name: &str, _data: ()) {}
        fn map_operation_data(&mut self, _name: &str, _data: ()) {}
        fn map_field_data(&mut self, _record: &str, _field: &str, _data: ()) {}
        fn map_text(&mut self, text: LanguageStringSpec) -> LanguageStringSpec {
            text
        }
    }

    #[test]
    fn selects_default_or_target_override_once() {
        let mut overrides = BTreeMap::new();
        overrides.insert(Language::Python, "PythonService".to_string());
        let spec = ApiSpec {
            module_path: Default::default(),
            data: (),
            version: "1".to_string(),
            services: vec![ServiceSpec {
                name: "service".to_string(),
                code_name: LanguageStringSpec {
                    default: Some("DefaultService".to_string()),
                    by_language: overrides,
                    ..Default::default()
                },
                wire_name: "service".to_string(),
                doc: Default::default(),
                namespace: Default::default(),
                operations_class: Default::default(),
                endpoint: None,
                experimental: false,
                deprecated: false,
                delay_load_temporalio_workflow: false,
                operations: Vec::new(),
                resources: Vec::new(),
                data: (),
            }],
            model_scope: None,
            types: BTreeMap::new(),
        };
        let tree = LanguageSelectionPass::new(Language::Python)
            .apply(ApiSpecTree::single(spec))
            .expect("language selection is infallible");
        let ApiSpecNode::Leaf(leaf) = tree.root else {
            panic!("single leaf");
        };
        assert_eq!(
            leaf.spec.services[0].code_name.value.as_deref(),
            Some("PythonService")
        );
    }

    #[test]
    fn selects_values_and_imports_independently() {
        let authored = LanguageStringSpec {
            default: Some("DefaultType".to_string()),
            default_import: Some("default.module".to_string()),
            by_language: BTreeMap::from([(Language::Python, "PythonType".to_string())]),
            imports: BTreeMap::from([(Language::Go, "go.module".to_string())]),
        };
        let python = Selector {
            language: Language::Python,
        }
        .map_text(authored.clone());
        assert_eq!(python.value(), Some("PythonType"));
        assert_eq!(python.import(), Some("default.module"));
        let go = Selector {
            language: Language::Go,
        }
        .map_text(authored.clone());
        assert_eq!(go.value(), Some("DefaultType"));
        assert_eq!(go.import(), Some("go.module"));
        let typescript = Selector {
            language: Language::TypeScript,
        }
        .map_text(authored);
        assert_eq!(typescript.value(), Some("DefaultType"));
        assert_eq!(typescript.import(), Some("default.module"));
    }

    #[test]
    fn selects_text_inside_resource_wire_types() {
        let resource = AuthoredResourceType {
            name: Symbol::new("resource"),
            wire_type: Some(ExternalTypeSpec::Alias(AliasTypeSpec {
                name: Symbol::new("wire-alias"),
                target: Box::new(TypeSpec::External(ExternalTypeSpec::Alias(AliasTypeSpec {
                    name: Symbol::new("nested-alias"),
                    target: Box::new(TypeSpec::String),
                    type_name: LanguageStringSpec {
                        by_language: BTreeMap::from([(Language::Python, "NestedType".to_string())]),
                        ..Default::default()
                    },
                }))),
                type_name: LanguageStringSpec {
                    default: Some("DefaultWireType".to_string()),
                    by_language: BTreeMap::from([(Language::Python, "WireType".to_string())]),
                    imports: BTreeMap::from([(Language::Python, "wire.module".to_string())]),
                    ..Default::default()
                },
            })),
            alias: None,
        };
        let selected = Selector {
            language: Language::Python,
        }
        .map_resource(resource);
        let Some(ExternalTypeSpec::Alias(alias)) = selected.wire_type else {
            panic!("resource wire alias");
        };
        assert_eq!(alias.type_name.value(), Some("WireType"));
        assert_eq!(alias.type_name.import(), Some("wire.module"));
        let TypeSpec::External(ExternalTypeSpec::Alias(nested)) = *alias.target else {
            panic!("nested wire alias");
        };
        assert_eq!(nested.type_name.value(), Some("NestedType"));
    }

    #[test]
    fn drops_function_descriptors_not_selected_for_the_target() {
        let text = |value: &str| LanguageStringSpec {
            by_language: BTreeMap::from([(Language::TypeScript, value.to_string())]),
            ..Default::default()
        };
        let spec = ApiSpec {
            module_path: Default::default(),
            data: (),
            version: "1".to_string(),
            services: vec![],
            model_scope: None,
            types: BTreeMap::from([(
                "request".to_string(),
                TypeDeclSpec::Record(RecordSpec {
                    name: "Request".to_string(),
                    full_name: "request".to_string(),
                    doc: Default::default(),
                    source: None,
                    experimental: false,
                    flatten_in_api: false,
                    fields: indexmap::IndexMap::from([(
                        "callback".to_string(),
                        RecordFieldSpec {
                            name: "callback".to_string(),
                            doc: None,
                            annotation: None,
                            flattened_annotation: None,
                            field_type: TypeSpec::String,
                            default_value: None,
                            required: true,
                            visibility: RecordFieldVisibility::Public,
                            function: Some(FunctionFieldSpec {
                                primary: false,
                                result: FunctionResultSpec::Authored(TypeSpec::String),
                                args_field: "args".to_string(),
                                arg_fields: vec![],
                                args: FunctionArgsSpec::Fixed(vec![]),
                                alternate_type: None,
                                converter: None,
                                name_extractor: None,
                                call_extractor: None,
                                result_type_parameter: None,
                                type_descriptor: Some(FunctionTypeDescriptorSpec {
                                    value_type: text("Callback"),
                                    args_type: text("CallbackArgs"),
                                }),
                            }),
                            data: (),
                        },
                    )]),
                    data: (),
                })
                .into(),
            )]),
        };
        // Universal mapping preserves even incomplete descriptors; only target
        // selection is allowed to drop them.
        let mut incomplete = spec.clone();
        let TypeDeclSpec::Record(record) =
            &mut incomplete.types.get_mut("request").unwrap().declaration
        else {
            panic!("request record");
        };
        record
            .fields
            .get_mut("callback")
            .unwrap()
            .function
            .as_mut()
            .unwrap()
            .type_descriptor
            .as_mut()
            .unwrap()
            .args_type = Default::default();
        let preserved = incomplete.map_names(Identity);
        assert!(
            preserved
                .record("request")
                .unwrap()
                .function("callback")
                .unwrap()
                .type_descriptor
                .is_some()
        );
        assert!(
            select_spec(preserved, Language::TypeScript)
                .record("request")
                .unwrap()
                .function("callback")
                .unwrap()
                .type_descriptor
                .is_none()
        );
        let python = select_spec(spec.clone(), Language::Python);
        assert!(
            python
                .record("request")
                .unwrap()
                .function("callback")
                .unwrap()
                .type_descriptor
                .is_none()
        );
        let typescript = select_spec(spec, Language::TypeScript);
        let descriptor = typescript
            .record("request")
            .unwrap()
            .function("callback")
            .unwrap()
            .type_descriptor
            .as_ref()
            .unwrap();
        assert_eq!(descriptor.value_type.value(), Some("Callback"));
        assert_eq!(descriptor.args_type.value(), Some("CallbackArgs"));
    }
}
