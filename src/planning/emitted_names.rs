//! `EmittedNameResolutionPass` fixes the final emitted JSON model identifiers.
//!
//! It runs after reachability so its manifest covers exactly the declarations a
//! backend will render. The pass owns this planning decision; generators only
//! consult the already-resolved planned graph.
//!
//! The manifest is built over the **whole tree**, not one leaf. One input file
//! is one module (P14), so a `$ref` that crosses files names a model whose
//! `x-<lang>-name` override is declared in the *other* file; a leaf-local
//! manifest cannot see that override, and the consuming module would emit the
//! pre-override identifier — a reference and an import that name nothing.
//! Collision scoping is unchanged by the widening: [`build_name_manifest`]
//! groups the models it is handed by module key, so a foreign model never joins
//! a local module's namespace (P15).

use std::collections::{BTreeMap, BTreeSet};

use crate::error::{Error, Result};
use crate::language::Language;
use crate::parser::{ManifestModel, ManifestService, NameManifest, build_name_manifest};
use crate::spec::{ApiSpecLeaf, ApiSpecNode, ApiSpecTransform, ApiSpecTree, CompilerPass};
use crate::spec::{LanguageStringSpec, ModulePath};

use super::{
    PlannedAliasType, PlannedEnumType, PlannedFamily, PlannedFieldData, PlannedFlagsType,
    PlannedJsonType, PlannedOperationData, PlannedProtoType, PlannedRecordData, PlannedRecordType,
    PlannedResource, PlannedResourceType, PlannedSpec, PlannedSpecData, PlannedVariantType,
};

pub(crate) struct EmittedNameResolutionPass {
    /// Resolved identifiers for every JSON model in the tree, keyed by full name.
    manifest: NameManifest,
    /// Every JSON model in the tree, carrying what the per-leaf rewrites need
    /// beyond the manifest itself.
    tree_models: Vec<TreeModel>,
}

/// One tree-wide JSON model, as the per-leaf rewrites see it.
struct TreeModel {
    module_key: String,
    full_name: String,
    /// The planned (pre-override) identifier. This is the language-agnostic name
    /// `module_imports` carries, so it is the key a stale import resolves under.
    planned_name: String,
}

impl EmittedNameResolutionPass {
    pub(crate) fn new(language: Language, tree: &ApiSpecTree<PlannedFamily>) -> Result<Self> {
        let mut models = Vec::new();
        let mut services = Vec::new();
        collect_tree_manifest_inputs(language, &tree.root, &mut models, &mut services);
        let manifest = build_name_manifest(language, &models, &services)?;
        let tree_models = models
            .iter()
            .map(|model| TreeModel {
                module_key: model.module_key.clone(),
                full_name: model.full_name.clone(),
                planned_name: model.model_name.clone(),
            })
            .collect();
        Ok(Self {
            manifest,
            tree_models,
        })
    }

    /// The resolved identifier for a model, keyed by full name.
    fn model_name(&self, full_name: &str) -> Option<&str> {
        self.manifest.type_name(full_name)
    }

    /// The resolved identifiers of every model `module_key` does *not* declare,
    /// keyed by full name. Generators seed their `$ref` registries with these so
    /// a cross-module `$ref` resolves through the manifest instead of being
    /// recased from the reference text (which drops the foreign override).
    fn cross_module_model_names(&self, module_key: &str) -> BTreeMap<String, String> {
        self.tree_models
            .iter()
            .filter(|model| model.module_key != module_key)
            .filter_map(|model| {
                let resolved = self.model_name(&model.full_name)?;
                Some((model.full_name.clone(), resolved.to_string()))
            })
            .collect()
    }

    /// The resolved identifier of the model `module_path` declares under the
    /// planned name `planned_name`. `module_imports` records planned names, so
    /// this is how a stale import name becomes the emitted one.
    fn imported_model_name(&self, module_path: &ModulePath, planned_name: &str) -> Option<&str> {
        let module_key = module_path.as_module_key();
        self.tree_models
            .iter()
            .find(|model| model.module_key == module_key && model.planned_name == planned_name)
            .and_then(|model| self.model_name(&model.full_name))
    }
}

impl CompilerPass<PlannedFamily, PlannedFamily> for EmittedNameResolutionPass {
    type Error = Error;

    fn transform_leaf(
        &mut self,
        leaf: ApiSpecLeaf<PlannedFamily>,
    ) -> Result<ApiSpecLeaf<PlannedFamily>> {
        let module_key = leaf.spec.module_path.as_module_key();
        let spec = leaf.spec.map_names(EmittedNameMapper {
            pass: self,
            module_key,
        });
        Ok(ApiSpecLeaf { spec, ..leaf })
    }
}

/// Rewrites every emitted JSON model identifier in one leaf from the tree-wide
/// manifest. Resolving the leaf's declarations is not enough: every *reference*
/// to a model — an operation input/output, a record field type — carries its own
/// [`PlannedJsonType`] clone with the pre-override name, and a reference to
/// another module's model is the only place a foreign override can be applied.
/// Routing the rewrite through [`ApiSpec::map_names`] reaches all of them.
struct EmittedNameMapper<'a> {
    pass: &'a EmittedNameResolutionPass,
    /// The module key of the leaf being rewritten.
    module_key: String,
}

impl ApiSpecTransform<PlannedFamily, PlannedFamily> for EmittedNameMapper<'_> {
    fn map_spec_data(&mut self, data: PlannedSpecData) -> PlannedSpecData {
        PlannedSpecData {
            module_imports: data
                .module_imports
                .into_iter()
                .map(|(module_path, names)| {
                    let names: BTreeSet<String> = names
                        .into_iter()
                        .map(|name| {
                            self.pass
                                .imported_model_name(&module_path, &name)
                                .unwrap_or(name.as_str())
                                .to_string()
                        })
                        .collect();
                    (module_path, names)
                })
                .collect(),
            cross_module_model_names: self.pass.cross_module_model_names(&self.module_key),
        }
    }

    fn map_json(&mut self, mut value: PlannedJsonType) -> PlannedJsonType {
        if let Some(resolved) = self.pass.model_name(&value.full_name) {
            value.model_name = resolved.to_string();
        }
        value
    }

    fn map_record(&mut self, value: PlannedRecordType) -> PlannedRecordType {
        value
    }
    fn map_enum(&mut self, value: PlannedEnumType) -> PlannedEnumType {
        value
    }
    fn map_flags(&mut self, value: PlannedFlagsType) -> PlannedFlagsType {
        value
    }
    fn map_variant(&mut self, value: PlannedVariantType) -> PlannedVariantType {
        value
    }
    fn map_resource(&mut self, value: PlannedResourceType) -> PlannedResourceType {
        value
    }
    fn map_proto(&mut self, value: PlannedProtoType) -> PlannedProtoType {
        value
    }
    fn map_alias(&mut self, value: PlannedAliasType) -> PlannedAliasType {
        value
    }
    fn map_service_data(&mut self, _: &str, _: ()) {}
    fn map_record_data(&mut self, _: &str, value: PlannedRecordData) -> PlannedRecordData {
        value
    }
    /// Identity: a resource's field and method types are WIT-authored (a JSON
    /// Schema input declares no resources), so they name no JSON model.
    fn map_resource_data(&mut self, _: &str, value: PlannedResource) -> PlannedResource {
        value
    }
    fn map_operation_data(&mut self, _: &str, value: PlannedOperationData) -> PlannedOperationData {
        value
    }
    fn map_field_data(&mut self, _: &str, _: &str, value: PlannedFieldData) -> PlannedFieldData {
        value
    }
    fn map_text(&mut self, value: LanguageStringSpec) -> LanguageStringSpec {
        value
    }
}

/// Builds the manifest over one leaf's post-reachability API surface. The
/// generators use this for the models they render; cross-module identifiers
/// reach them through [`PlannedSpecData::cross_module_model_names`], which this
/// pass resolves tree-wide.
pub(crate) fn build_json_name_manifest(
    language: Language,
    api_plan: &PlannedSpec,
) -> Result<NameManifest> {
    let models = manifest_models(api_plan);
    let services = manifest_services(language, api_plan);
    build_name_manifest(language, &models, &services)
}

fn collect_tree_manifest_inputs(
    language: Language,
    node: &ApiSpecNode<PlannedFamily>,
    models: &mut Vec<ManifestModel>,
    services: &mut Vec<ManifestService>,
) {
    match node {
        ApiSpecNode::Leaf(leaf) => {
            models.extend(manifest_models(&leaf.spec));
            services.extend(manifest_services(language, &leaf.spec));
        }
        ApiSpecNode::Branch(branch) => {
            for child in branch.children.values() {
                collect_tree_manifest_inputs(language, child, models, services);
            }
        }
    }
}

fn manifest_models(api_plan: &PlannedSpec) -> Vec<ManifestModel> {
    api_plan
        .external_types()
        .filter_map(|(_full_name, binding)| binding.json_model().map(manifest_model))
        .collect()
}

fn manifest_model(json: &PlannedJsonType) -> ManifestModel {
    let module_key = json
        .module_path
        .as_ref()
        .map(ModulePath::as_module_key)
        .unwrap_or_default();
    let local_name = json
        .full_name
        .rsplit(['#', '/'])
        .next()
        .unwrap_or(&json.full_name)
        .to_string();
    ManifestModel {
        full_name: json.full_name.clone(),
        local_name,
        model_name: json.model_name.clone(),
        module_key,
        schema: json.schema.clone(),
    }
}

fn manifest_services(language: Language, api_plan: &PlannedSpec) -> Vec<ManifestService> {
    api_plan
        .services
        .iter()
        .map(|service| ManifestService {
            name: service.name.clone(),
            code_name: service.code_name.for_language(language).map(str::to_string),
            module_key: api_plan.module_path.as_module_key(),
            io_type_refs: service
                .operations
                .iter()
                .flat_map(|operation| [operation.input.as_ref(), operation.output.as_ref()])
                .flatten()
                .filter_map(|ty| match ty {
                    crate::spec::TypeSpec::External(crate::spec::ExternalTypeSpec::Json(json)) => {
                        Some(json.full_name.clone())
                    }
                    _ => None,
                })
                .collect(),
        })
        .collect()
}
