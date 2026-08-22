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
    ApiSpec, ApiSpecTransform, AuthoredFamily, AuthoredResourceType, JsonModelSpec,
    LanguageStringSpec, SelectedFamily, SelectedSupportSpec, SelectedTextSpec, SupportSpec, Symbol,
};
use crate::spec::{ApiSpecLeaf, CompilerPass};

/// Produce the structurally identical selected IR.
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
    fn map_resource(&mut self, name: AuthoredResourceType) -> AuthoredResourceType {
        name
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

    fn map_text(&mut self, text: LanguageStringSpec) -> SelectedTextSpec {
        SelectedTextSpec {
            value: text.for_language(self.language).map(ToOwned::to_owned),
            import: text
                .import_for_language(self.language)
                .map(ToOwned::to_owned),
        }
    }

    fn map_support(&mut self, support: SupportSpec) -> SelectedSupportSpec {
        SelectedSupportSpec {
            fragments: support
                .fragments
                .get(&self.language)
                .cloned()
                .unwrap_or_default(),
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
    use crate::spec::{ApiSpec, LanguageStringSpec, ServiceSpec, SupportFragmentSpec, SupportSpec};
    use crate::spec::{ApiSpecNode, ApiSpecTree, CompilerPass};

    use super::{LanguageSelectionPass, select_spec};

    #[test]
    fn selects_default_or_target_override_once() {
        let mut overrides = BTreeMap::new();
        overrides.insert(Language::Python, "PythonService".to_string());
        let spec = ApiSpec {
            module_path: Default::default(),
            data: (),
            version: "1".to_string(),
            support: SupportSpec {
                fragments: BTreeMap::from([
                    (
                        Language::Python,
                        vec![SupportFragmentSpec {
                            path: "python.py".to_string(),
                            contents: String::new(),
                            namespace: None,
                        }],
                    ),
                    (
                        Language::Go,
                        vec![SupportFragmentSpec {
                            path: "go.go".to_string(),
                            contents: String::new(),
                            namespace: None,
                        }],
                    ),
                ]),
            },
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
        assert_eq!(leaf.spec.support.fragments.len(), 1);
        assert_eq!(leaf.spec.support.fragments[0].path, "python.py");
    }

    #[test]
    fn selected_ir_drops_language_maps() {
        let selected = select_spec(
            ApiSpec {
                module_path: Default::default(),
                data: (),
                version: "1".to_string(),
                support: SupportSpec::default(),
                services: vec![],
                types: BTreeMap::new(),
            },
            Language::Go,
        );
        assert!(selected.support.fragments.is_empty());
    }
}
