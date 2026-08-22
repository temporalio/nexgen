use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use heck::{ToShoutySnakeCase, ToSnakeCase, ToUpperCamelCase};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::Value;

use crate::error::{Error, Result};
use crate::generator::ExternalModelBackend;
use crate::generator::json_schema::build_json_name_manifest;
use crate::generator::json_schema::register_cross_module_ref_names;
use crate::generator::python::{
    PythonImports, PythonModelHoists, RenderedModelFragments, WireValueConversion,
    module_common_prefix_len, python_field_name, python_string_literal,
    render_generated_file_header, render_named_python_import, render_optional_python_imports,
    render_python_docstring,
};
use crate::json_schema::scalar::{ScalarKind, ScalarMatcher};
use crate::language::Language;
use crate::parser::NameManifest;
use crate::planning::{PlannedFamily, PlannedJsonType, PlannedSpec};
use crate::spec::{ApiSpecBranch, ApiSpecNode};
use crate::spec::{ExternalTypeSpec, ModulePath, RecordSpec};

const JSON_PUBLIC_RUNTIME_NAMES: &[&str] = &["ValidationError", "Violation"];

#[derive(Debug, Clone, Deserialize, Default)]
struct Schema {
    #[serde(rename = "$ref")]
    reference: Option<String>,
    #[serde(rename = "type")]
    ty: Option<Value>,
    title: Option<String>,
    description: Option<String>,
    deprecated: Option<bool>,
    properties: Option<IndexMap<String, Schema>>,
    required: Option<Vec<String>>,
    #[serde(rename = "additionalProperties")]
    additional_properties: Option<Value>,
    items: Option<Box<Schema>>,
    #[serde(rename = "oneOf")]
    one_of: Option<Vec<Schema>>,
    default: Option<Value>,
    #[serde(rename = "const")]
    const_value: Option<Value>,
    #[serde(rename = "maxProperties")]
    max_properties: Option<usize>,
    #[serde(rename = "minProperties")]
    min_properties: Option<usize>,
    #[serde(rename = "propertyNames")]
    property_names: Option<Box<Schema>>,
    #[serde(rename = "dependentRequired")]
    dependent_required: Option<IndexMap<String, Vec<String>>>,
    minimum: Option<serde_json::Number>,
    maximum: Option<serde_json::Number>,
    #[serde(rename = "exclusiveMinimum")]
    exclusive_minimum: Option<serde_json::Number>,
    #[serde(rename = "exclusiveMaximum")]
    exclusive_maximum: Option<serde_json::Number>,
    #[serde(rename = "multipleOf")]
    multiple_of: Option<serde_json::Number>,
    #[serde(rename = "minLength")]
    min_length: Option<u64>,
    #[serde(rename = "maxLength")]
    max_length: Option<u64>,
    pattern: Option<String>,
    format: Option<String>,
    #[serde(rename = "contentEncoding")]
    content_encoding: Option<String>,
    #[serde(rename = "minItems")]
    min_items: Option<u64>,
    #[serde(rename = "maxItems")]
    max_items: Option<u64>,
    #[serde(rename = "uniqueItems")]
    unique_items: Option<bool>,
    contains: Option<Box<Schema>>,
    #[serde(rename = "minContains")]
    min_contains: Option<u64>,
    #[serde(rename = "maxContains")]
    max_contains: Option<u64>,
    #[serde(rename = "enum")]
    enum_values: Option<Vec<Value>>,
    #[serde(rename = "x-py-name")]
    x_py_name: Option<String>,
}

impl Schema {
    /// The emitted Python attribute identifier for a property: the `x-py-name`
    /// override if present (used verbatim), otherwise the snake-cased JSON name.
    /// The wire name (`json_name`) is unaffected — the `Field(alias=...)` pin
    /// keeps the contract stable. See specs/json-schema/features/properties.md.
    fn py_member_name(&self, json_name: &str) -> String {
        self.x_py_name
            .clone()
            .unwrap_or_else(|| python_field_name(json_name))
    }
}

fn py_bound_literal(number: &serde_json::Number, is_integer: bool) -> String {
    if is_integer && let Some(value) = number.as_f64() {
        return (value.trunc() as i64).to_string();
    }
    number.to_string()
}

thread_local! {
    /// Resolved type identifiers keyed by both `full_name` and the `#/$defs/<full_name>`
    /// `$ref` form, so `reference_model_name` follows the same name manifest as the
    /// declaration (honoring `x-py-name` overrides) instead of recasing the ref segment.
    /// Generation is single-threaded per file, so a thread-local avoids threading the map
    /// through every recursive `annotation` call that resolves a `$ref`.
    static REF_NAMES: RefCell<BTreeMap<String, String>> = const { RefCell::new(BTreeMap::new()) };
    /// The emitted type identifiers declared in the module currently rendering.
    /// A `$ref` at one of them reaches its converter class directly; anything
    /// else is only available as the imported class, so its converter is read
    /// off the class attribute the SDK decorator set.
    static LOCAL_MODELS: RefCell<BTreeSet<String>> = const { RefCell::new(BTreeSet::new()) };
    /// The emitted type identifiers that are `oneOf` sum types. A union carries
    /// no converter class — its conversion is a pair of module-private free
    /// functions — so a `$ref` at one dispatches differently.
    static UNION_NAMES: RefCell<BTreeSet<String>> = const { RefCell::new(BTreeSet::new()) };
}

fn set_ref_names(ref_names: &BTreeMap<String, String>) {
    REF_NAMES.with(|cell| cell.borrow_mut().clone_from(ref_names));
}

/// Records the module-scoped facts the value emitters resolve `$ref`s against.
fn set_module_context(json_models: &[&PlannedJsonType]) -> Result<()> {
    let locals = json_models
        .iter()
        .map(|model| model.model_name.clone())
        .collect::<BTreeSet<_>>();
    let unions = json_models
        .iter()
        .filter(|model| is_python_union_model(model))
        .map(|model| model.model_name.clone())
        .collect::<BTreeSet<_>>();
    LOCAL_MODELS.with(|cell| *cell.borrow_mut() = locals);
    UNION_NAMES.with(|cell| *cell.borrow_mut() = unions);
    Ok(())
}

fn is_local_model(name: &str) -> bool {
    LOCAL_MODELS.with(|cell| cell.borrow().contains(name))
}

fn is_union_type_name(name: &str) -> bool {
    UNION_NAMES.with(|cell| cell.borrow().contains(name))
}

/// The private converter class a model's wire contract lives in.
pub(crate) fn converter_class_name(model_name: &str) -> String {
    format!("_{model_name}TransferTypeConverter")
}

/// The converter body's local holding one declared property's parsed value,
/// until the final keyword-argument construction reads it back.
///
/// The name is **not** the member identifier: `from_transfer_type` is one Python
/// scope, so a property-derived local shares it with the converter's own locals
/// (`violations`, `raw`), with the runtime helpers and modules the body calls
/// (`_collect`, `typing`, `re`, `math`), and with the builtins it calls
/// (`isinstance`, `len`, `int`). A member identifier used verbatim would shadow
/// any of them — a property named `violations` silently rebound the violation
/// accumulator and dropped every collected violation, which is exactly the
/// silently-wrong output P15 exists to prevent.
///
/// The `_value` suffix makes that structurally impossible instead of
/// blocklisting names: no fixed local, builtin, imported module, or synthesized
/// module-level identifier (`_PATTERN_<HEX>`, `_<MODEL>_DECLARED`,
/// `_<base>_{from,to}_transfer_type`,
/// `_<Model>TransferTypeConverter`) ends in `_value`. It stays collision-free
/// *within* the property family too: every temporary this position needs appends
/// a further suffix (`_raw`, `_parsed`, `_list`, `_index`, `_element`, `_item`,
/// `_path`), none of which ends in `_value`, so no property's slot can be
/// another property's temporary and distinct members (already one P15 scope)
/// stay distinct here.
fn parse_slot_local(field_name: &str) -> String {
    format!("{field_name}_value")
}

/// The expression that reaches a referenced model's converter. A model declared
/// in this module is reached through its own converter class (fully typed); one
/// imported from a sibling module is reached through the class attribute the SDK
/// decorator set, because the converter itself is module-private there.
fn converter_expr(model_name: &str) -> String {
    if is_local_model(model_name) {
        format!("{}()", converter_class_name(model_name))
    } else {
        format!("getattr({model_name}, \"__temporal_transfer_type_converter\")")
    }
}

/// The `_<base>_{from,to}_transfer_type` function-name base for a named union.
pub(crate) fn union_fn_base(model_name: &str) -> String {
    model_name.to_snake_case()
}

/// The function-name base for an **inline** (property-level) union, mirroring the
/// `<Model><Property>` synthesized-name rule. `member_ident` is the member's
/// *emitted* identifier, so a `x-py-name` override moves this name with it — P15's
/// escape hatch has to reach every name synthesized from the property.
pub(crate) fn inline_union_fn_base(model_name: &str, member_ident: &str) -> String {
    format!(
        "{}_{}",
        model_name.to_snake_case(),
        member_ident.to_snake_case()
    )
}

pub(crate) fn union_parse_fn(base: &str) -> String {
    format!("_{base}_from_transfer_type")
}

pub(crate) fn union_serialize_fn(base: &str) -> String {
    format!("_{base}_to_transfer_type")
}

/// The module-level `frozenset` of declared wire keys an open object splits its
/// catch-all on, mirroring TypeScript's `<MODEL>_DECLARED`.
pub(crate) fn declared_fields_const_name(model_name: &str) -> String {
    format!("_{}_DECLARED", model_name.to_shouty_snake_case())
}

pub(in crate::generator) fn model_type_ref(json_type: &PlannedJsonType) -> String {
    json_type.model_name.clone()
}

#[derive(Debug, Default)]
pub(in crate::generator) struct ModelBackend {
    json_models: Vec<PlannedJsonType>,
    hoisted_json_models: Vec<PlannedJsonType>,
    tree_leaf: bool,
    runtime_import_module: String,
    /// Resolved emitted identifiers (with `x-py-name` overrides applied).
    manifest: NameManifest,
    /// Resolved type names keyed by `full_name` and the `#/$defs/<full_name>` ref form.
    ref_names: BTreeMap<String, String>,
}

impl ExternalModelBackend<PlannedJsonType> for ModelBackend {
    type ModelFragments = RenderedModelFragments;
    type WireConversion = WireValueConversion;

    fn prepare(&mut self, api_plan: &PlannedSpec) -> Result<()> {
        self.tree_leaf = !api_plan.module_path.is_root();
        self.runtime_import_module = if self.tree_leaf {
            root_python_runtime_module(&api_plan.module_path)
        } else {
            "._definitions".to_string()
        };
        // Resolve every emitted identifier once (overrides applied), then adopt the
        // resolved type name as each model's `model_name` so every downstream derivation
        // (class decl, union `TypeAlias`, `model_type_ref`) follows the same identifier.
        // `$ref` targets are resolved via `ref_names` below.
        self.manifest = build_json_name_manifest(Language::Python, api_plan)?;
        let mut json_models: Vec<PlannedJsonType> = api_plan
            .external_types()
            .map(|(_, binding)| binding)
            .filter_map(|binding| match &binding.external_type {
                ExternalTypeSpec::Json(json_type) => Some(json_type.clone()),
                _ => None,
            })
            .map(|mut json_type| {
                if let Some(resolved) = self.manifest.type_name(&json_type.full_name) {
                    json_type.model_name = resolved.to_string();
                }
                json_type
            })
            .collect();
        self.json_models = std::mem::take(&mut json_models);
        self.hoisted_json_models = Vec::new();
        self.ref_names.clear();
        register_cross_module_ref_names(api_plan, &mut self.ref_names);
        for model in &self.json_models {
            // A resolved `$ref` is `#/$defs/<full_name>`; register that form (plus the
            // bare `full_name`) so `reference_model_name` resolves through the manifest
            // instead of recasing the ref segment (which would drop a type override).
            self.ref_names
                .insert(model.full_name.clone(), model.model_name.clone());
            self.ref_names.insert(
                format!("#/$defs/{}", model.full_name),
                model.model_name.clone(),
            );
        }
        Ok(())
    }

    fn render_models(&self) -> Result<RenderedModelFragments> {
        set_ref_names(&self.ref_names);
        let json_models = self.json_models.iter().collect::<Vec<_>>();
        let mut fragments =
            render_external_models(json_models.as_slice(), &self.runtime_import_module)?;
        if !self.json_models.is_empty() || !self.hoisted_json_models.is_empty() {
            // Validation failures are part of the JSON backend's public runtime surface.
            // Keep the request even when every local model was moved to `_recursive`.
            fragments.root_package_imports.insert(
                "._definitions".to_string(),
                JSON_PUBLIC_RUNTIME_NAMES
                    .iter()
                    .map(|name| (*name).to_string())
                    .collect(),
            );
        }
        Ok(fragments)
    }

    fn render_support_files(&self) -> Result<BTreeMap<PathBuf, String>> {
        if self.tree_leaf || self.json_models.is_empty() {
            return Ok(BTreeMap::new());
        }

        Ok(BTreeMap::from([(
            PathBuf::from("_definitions.py"),
            render_support_file(),
        )]))
    }

    fn model_type_annotation(&self, json_type: &PlannedJsonType) -> Option<String> {
        Some(model_type_ref(json_type))
    }

    fn wire_type_identifier(&self, json_type: &PlannedJsonType) -> Option<String> {
        Some(json_type.full_name.clone())
    }

    fn wire_conversion(
        &self,
        json_type: &PlannedJsonType,
        _planned_record: Option<&RecordSpec<PlannedFamily>>,
    ) -> Option<WireValueConversion> {
        Some(WireValueConversion {
            annotation: model_type_ref(json_type),
            from_wire: "{wire}".to_string(),
            to_wire: "{value}".to_string(),
            imports: PythonImports::default(),
            supports_unpacked_input: false,
        })
    }
}

impl ModelBackend {
    pub(in crate::generator) fn prepare_with_hoists(
        &mut self,
        api_plan: &PlannedSpec,
        hoists: &PythonModelHoists,
    ) -> Result<()> {
        self.prepare(api_plan)?;
        let mut local_models = Vec::new();
        let mut hoisted_models = Vec::new();
        for model in std::mem::take(&mut self.json_models) {
            if hoists.is_hoisted(&api_plan.module_path, &model.model_name) {
                hoisted_models.push(model);
            } else {
                local_models.push(model);
            }
        }
        self.json_models = local_models;
        self.hoisted_json_models = hoisted_models;
        Ok(())
    }

    pub(in crate::generator) fn is_hoisted(&self, json_type: &PlannedJsonType) -> bool {
        self.hoisted_json_models
            .iter()
            .any(|model| model.full_name == json_type.full_name)
    }
}

#[derive(Debug, Default)]
struct JsonModelHoistPlan {
    hoisted: BTreeMap<ModulePath, BTreeSet<String>>,
    hoisted_models: Vec<PlannedJsonType>,
    dependency_imports: BTreeMap<ModulePath, BTreeSet<String>>,
}

impl JsonModelHoistPlan {
    fn for_tree(branch: &ApiSpecBranch<PlannedFamily>) -> Self {
        let mut models = BTreeMap::<String, (ModulePath, PlannedJsonType)>::new();
        collect_tree_json_models(branch, &mut models);

        let mut graph = BTreeMap::<String, BTreeSet<String>>::new();
        for (full_name, (_, model)) in &models {
            let mut refs = BTreeSet::new();
            collect_json_schema_model_refs(&model.schema, &models, &mut refs);
            graph.insert(full_name.clone(), refs);
        }

        let mut hoisted_full_names = BTreeSet::new();
        for (source_name, (source_module, _)) in &models {
            for target_name in graph.get(source_name).into_iter().flatten() {
                let Some((target_module, _)) = models.get(target_name) else {
                    continue;
                };
                if source_module == target_module {
                    continue;
                }
                if json_model_can_reach(target_name, source_name, &graph, &mut BTreeSet::new()) {
                    hoisted_full_names.insert(source_name.clone());
                    hoisted_full_names.insert(target_name.clone());
                }
            }
        }

        let mut hoisted = BTreeMap::<ModulePath, BTreeSet<String>>::new();
        let mut hoisted_models = Vec::new();
        for full_name in &hoisted_full_names {
            let Some((module_path, model)) = models.get(full_name) else {
                continue;
            };
            hoisted
                .entry(module_path.clone())
                .or_default()
                .insert(model.model_name.clone());
            hoisted_models.push(model.clone());
        }

        let mut dependency_imports = BTreeMap::<ModulePath, BTreeSet<String>>::new();
        for full_name in &hoisted_full_names {
            for target_name in graph.get(full_name).into_iter().flatten() {
                if hoisted_full_names.contains(target_name) {
                    continue;
                }
                let Some((module_path, model)) = models.get(target_name) else {
                    continue;
                };
                dependency_imports
                    .entry(module_path.clone())
                    .or_default()
                    .insert(model.model_name.clone());
            }
        }

        Self {
            hoisted,
            hoisted_models,
            dependency_imports,
        }
    }

    fn is_empty(&self) -> bool {
        self.hoisted_models.is_empty()
    }
}

pub(in crate::generator) fn tree_model_hoists(
    branch: &ApiSpecBranch<PlannedFamily>,
) -> Result<PythonModelHoists> {
    let plan = JsonModelHoistPlan::for_tree(branch);
    let mut hoists = PythonModelHoists::default();
    if plan.is_empty() {
        return Ok(hoists);
    }
    for (module_path, names) in &plan.hoisted {
        hoists.add_module_hoists(module_path.clone(), names.clone());
    }
    hoists.add_file(
        PathBuf::from("_recursive.py"),
        render_hoisted_models_module(&plan)?,
    );
    hoists.add_exported_names(
        plan.hoisted_models
            .iter()
            .map(|model| model.model_name.clone())
            .collect(),
    );
    Ok(hoists)
}

fn render_hoisted_models_module(hoists: &JsonModelHoistPlan) -> Result<String> {
    let models = hoists.hoisted_models.iter().collect::<Vec<_>>();
    let model_fragments = render_external_models(models.as_slice(), "._definitions")?;
    let mut body = model_fragments.body.clone();
    if !model_fragments.post_model_statements.is_empty() {
        if !body.is_empty() {
            body.push_str("\n\n");
        }
        body.push_str(&model_fragments.post_model_statements);
    }

    let mut output = String::new();
    render_generated_file_header(&mut output);
    output.push('\n');
    let wrote_imports =
        render_optional_python_imports(&mut output, &body, &model_fragments.module_imports, &[]);
    let mut wrote_relative_imports = false;
    for (module, names) in &model_fragments.relative_imports {
        if names.is_empty() {
            continue;
        }
        if wrote_imports || wrote_relative_imports {
            output.push('\n');
        }
        render_named_python_import(
            &mut output,
            module,
            &names.iter().cloned().collect::<Vec<_>>(),
        );
        wrote_relative_imports = true;
    }
    let mut wrote_dependency_imports = false;
    for (module_path, names) in &hoists.dependency_imports {
        if names.is_empty() {
            continue;
        }
        if wrote_imports || wrote_relative_imports || wrote_dependency_imports {
            output.push('\n');
        }
        render_named_python_import(
            &mut output,
            &python_relative_models_module(&ModulePath::default(), module_path),
            &names.iter().cloned().collect::<Vec<_>>(),
        );
        wrote_dependency_imports = true;
    }

    if !body.is_empty() {
        output.push('\n');
        output.push('\n');
        output.push_str(&body);
    }
    output.push_str("\n\n__all__ = [\n");
    for name in hoists
        .hoisted_models
        .iter()
        .map(|model| &model.model_name)
        .collect::<BTreeSet<_>>()
    {
        output.push_str("    ");
        output.push_str(&python_string_literal(name));
        output.push_str(",\n");
    }
    output.push_str("]\n");
    Ok(output)
}

fn collect_tree_json_models(
    branch: &ApiSpecBranch<PlannedFamily>,
    models: &mut BTreeMap<String, (ModulePath, PlannedJsonType)>,
) {
    for node in branch.children.values() {
        match node {
            ApiSpecNode::Leaf(leaf) => {
                for binding in leaf.spec.external_types().map(|(_, binding)| binding) {
                    if let ExternalTypeSpec::Json(json_type) = &binding.external_type {
                        models.insert(
                            json_type.full_name.clone(),
                            (leaf.module_path.clone(), json_type.clone()),
                        );
                    }
                }
            }
            ApiSpecNode::Branch(branch) => collect_tree_json_models(branch, models),
        }
    }
}

fn collect_json_schema_model_refs(
    value: &serde_json::Value,
    models: &BTreeMap<String, (ModulePath, PlannedJsonType)>,
    refs: &mut BTreeSet<String>,
) {
    match value {
        serde_json::Value::Object(object) => {
            if let Some(reference) = object.get("$ref").and_then(serde_json::Value::as_str)
                && let Some(full_name) = json_schema_ref_full_name(reference)
                && models.contains_key(&full_name)
            {
                refs.insert(full_name);
            }
            for value in object.values() {
                collect_json_schema_model_refs(value, models, refs);
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                collect_json_schema_model_refs(value, models, refs);
            }
        }
        _ => {}
    }
}

fn json_schema_ref_full_name(reference: &str) -> Option<String> {
    let fragment = reference
        .split_once('#')
        .map(|(_, fragment)| fragment)
        .unwrap_or(reference);
    let name = fragment.strip_prefix("/$defs/")?;
    Some(name.replace("~1", "/").replace("~0", "~"))
}

fn json_model_can_reach(
    source: &str,
    target: &str,
    graph: &BTreeMap<String, BTreeSet<String>>,
    visited: &mut BTreeSet<String>,
) -> bool {
    if source == target {
        return true;
    }
    if !visited.insert(source.to_string()) {
        return false;
    }
    graph.get(source).is_some_and(|next| {
        next.iter()
            .any(|name| json_model_can_reach(name, target, graph, visited))
    })
}

fn python_relative_models_module(from: &ModulePath, to: &ModulePath) -> String {
    let common = module_common_prefix_len(&from.0, &to.0);
    let dot_count = from.0.len().saturating_sub(common) + 1;
    let mut module = ".".repeat(dot_count);
    let rest = to.0[common..]
        .iter()
        .map(|segment| segment.replace('-', "_"))
        .chain(std::iter::once("models".to_string()))
        .collect::<Vec<_>>();
    module.push_str(&rest.join("."));
    module
}

pub(in crate::generator) fn render_support_file() -> String {
    render_json_runtime_module()
}

fn root_python_runtime_module(module_path: &ModulePath) -> String {
    format!("{}{}", ".".repeat(module_path.0.len() + 1), "_definitions")
}

pub(in crate::generator) fn render_external_models(
    json_models: &[&PlannedJsonType],
    runtime_import_module: &str,
) -> Result<RenderedModelFragments> {
    if json_models.is_empty() {
        return Ok(RenderedModelFragments::default());
    }

    // Partition class models from `oneOf` sum-type union defs. Union defs are
    // emitted as `typing.Union[...]` TypeAliases *after* all classes so their
    // eager `Union[...]` expression sees every member class defined.
    let class_models: Vec<&PlannedJsonType> = json_models
        .iter()
        .copied()
        .filter(|model| !is_python_union_model(model))
        .collect();
    let union_models: Vec<&PlannedJsonType> = json_models
        .iter()
        .copied()
        .filter(|model| is_python_union_model(model))
        .collect();

    set_module_context(json_models)?;

    let mut body = String::new();
    // Module-level constants first: the shared compiled `pattern`/`format`
    // regexes and the declared-key sets an open object splits its catch-all on.
    render_pattern_regexes(&mut body, json_models)?;
    for model in &class_models {
        let schema = decode_schema(model)?;
        if is_open_object(&schema) {
            push_section(&mut body);
            render_declared_field_set(&mut body, model, &schema);
        }
    }

    // Each model is a plain dataclass plus a private off-model converter owning
    // both wire directions. The converter is emitted first and refers to the
    // model by forward-ref string, so the dataclass can carry the
    // `transfer_type_convertible` decorator that registers it.
    for model in &class_models {
        let schema = decode_schema(model)?;
        push_section(&mut body);
        render_model_converter(&mut body, model, &schema, json_models)?;
        push_section(&mut body);
        render_model_dataclass(&mut body, model, &schema)?;
    }

    // A `TypeAlias` cannot be decorated and `type[A | B]` is not a valid
    // annotation, so a union's conversion is emitted as module-private free
    // functions instead.
    render_union_transfer_functions(&mut body, json_models)?;

    for model in &union_models {
        let schema = decode_schema(model)?;
        push_section(&mut body);
        body.push_str(&model.model_name);
        body.push_str(": typing.TypeAlias = ");
        body.push_str(&annotation(&schema)?);
        body.push('\n');
        // A module-level variable docstring *follows* its assignment — the same
        // placement a dataclass member's docstring takes. Emitted before it, the
        // string would document whatever statement precedes the alias.
        render_python_docstring(
            &mut body,
            "",
            schema.description.as_deref(),
            &[],
            None,
            false,
        );
    }

    // Each is emitted only when the rendered body actually references the module
    // (a materialized temporal field, a `multipleOf` on a number, a hoisted
    // `pattern`/`format` regex); the shared import writer does that filtering.
    let module_imports = BTreeSet::from([
        "temporalio.converter".to_string(),
        "datetime".to_string(),
        "math".to_string(),
        "re".to_string(),
    ]);
    let mut relative_imports = BTreeMap::<String, BTreeSet<String>>::new();
    // Import exactly the runtime symbols the emitted body references. Longer
    // names are checked as whole identifiers so `_parse_base64url` does not drag
    // `_parse_base64` in with it.
    let runtime_imports = JSON_RUNTIME_SYMBOLS
        .iter()
        .filter(|symbol| body_references_symbol(&body, symbol))
        .map(|symbol| (*symbol).to_string())
        .collect::<BTreeSet<_>>();
    if !runtime_imports.is_empty() {
        relative_imports.insert(runtime_import_module.to_string(), runtime_imports);
    }
    Ok(RenderedModelFragments {
        body,
        post_model_statements: String::new(),
        module_imports,
        relative_imports,
        root_package_imports: BTreeMap::new(),
        exported_names: json_models
            .iter()
            .map(|model| model.model_name.clone())
            .collect(),
        allows_private_wire_access: false,
    })
}

/// The runtime symbols a generated model module may import from `_definitions`.
const JSON_RUNTIME_SYMBOLS: &[&str] = &[
    "ValidationError",
    "Violation",
    "_check_contains",
    "_check_date_time",
    "_check_duration",
    "_check_time",
    "_check_unique_items",
    "_collect",
    "_json_values_equal",
    "_format_base64",
    "_format_base64url",
    "_format_date",
    "_format_date_time",
    "_format_duration",
    "_format_time",
    "_parse_base64",
    "_parse_base64url",
    "_parse_date",
    "_parse_date_time",
    "_parse_duration",
    "_parse_spec_integer",
    "_parse_time",
    "_quote",
    "_transfer_type_convertible",
];

/// True when `body` references `symbol` as a whole Python identifier (not as the
/// prefix or suffix of a longer one, and not as an attribute of something else).
fn body_references_symbol(body: &str, symbol: &str) -> bool {
    let is_ident = |character: char| character.is_ascii_alphanumeric() || character == '_';
    body.match_indices(symbol).any(|(index, _)| {
        let previous = body[..index].chars().next_back();
        let next = body[index + symbol.len()..].chars().next();
        !previous.is_some_and(|character| is_ident(character) || character == '.')
            && !next.is_some_and(is_ident)
    })
}

/// Starts a new top-level section, separated from the previous one by a blank
/// line. `ruff format` normalizes the exact count.
fn push_section(body: &mut String) {
    if !body.is_empty() {
        body.push_str("\n\n");
    }
}

fn push_indented(output: &mut String, body: &str, indent: &str) {
    for line in body.lines() {
        if line.is_empty() {
            output.push('\n');
            continue;
        }
        output.push_str(indent);
        output.push_str(line);
        output.push('\n');
    }
}

/// True when a model's schema is a `oneOf` sum type (two or more non-null
/// branches) — emitted as a `typing.TypeAlias` over the branch union, not a
/// dataclass.
fn is_python_union_model(model: &PlannedJsonType) -> bool {
    decode_schema(model).is_ok_and(|schema| {
        schema.one_of.as_ref().is_some_and(|branches| {
            branches
                .iter()
                .filter(|branch| branch.ty.as_ref().and_then(Value::as_str) != Some("null"))
                .count()
                >= 2
        })
    })
}

fn render_json_runtime_module() -> String {
    let mut output = String::new();
    output.push_str(crate::generator::python::GENERATED_HEADER);
    output.push_str("\n\n");
    output.push_str("from __future__ import annotations\n\n");
    output.push_str("import base64\n");
    output.push_str("import collections.abc\n");
    output.push_str("import dataclasses\n");
    output.push_str("import datetime\n");
    output.push_str("import json\n");
    output.push_str("import re\n");
    output.push_str("import typing\n");
    output.push_str("import temporalio.converter\n\n\n");
    // The underscore-prefixed helpers below are imported by sibling generated
    // modules (e.g. `models.py`); listing them keeps type checkers from flagging
    // them as unused private symbols.
    output.push_str("__all__ = [\n");
    for name in [
        "ValidationError",
        "Violation",
        "_check_contains",
        "_check_date_time",
        "_check_duration",
        "_check_time",
        "_check_unique_items",
        "_collect",
        "_json_values_equal",
        "_format_base64",
        "_format_base64url",
        "_format_date",
        "_format_date_time",
        "_format_duration",
        "_format_time",
        "_parse_base64",
        "_parse_base64url",
        "_parse_date",
        "_parse_date_time",
        "_parse_duration",
        "_parse_spec_integer",
        "_parse_time",
        "_quote",
        "_transfer_type_convertible",
    ] {
        output.push_str("    \"");
        output.push_str(name);
        output.push_str("\",\n");
    }
    output.push_str("]\n\n\n");
    render_validator_core(&mut output);
    output.push_str("\n\n");
    render_transfer_type_convertible_helper(&mut output);
    output.push_str("\n\n");
    render_spec_int_helper(&mut output);
    output.push_str("\n\n");
    render_unique_items_helper(&mut output);
    output.push_str("\n\n");
    render_contains_helper(&mut output);
    output.push_str("\n\n");
    render_temporal_helpers(&mut output);
    output.push_str("\n\n");
    render_content_encoding_helpers(&mut output);
    output
}

/// Emits the error-aggregation core: the `Violation` record, the single
/// aggregating `ValidationError`, and the `_collect` re-pather that lifts a
/// nested model's violations under the parent's path. One error type carrying
/// every violation, structurally identical to Go/TypeScript/Java (P11).
fn render_validator_core(output: &mut String) {
    output.push_str(VALIDATOR_CORE_BODY);
}

const VALIDATOR_CORE_BODY: &str = r#"@dataclasses.dataclass(frozen=True, slots=True)
class Violation:
    """A single constraint failure, located by JSON path."""

    path: str
    reason: str


class ValidationError(Exception):
    """Every constraint failure found in one (de)serialization pass."""

    violations: list[Violation]

    def __init__(self, violations: list[Violation]) -> None:
        self.violations = violations
        detail = "; ".join(f"{item.path}: {item.reason}" for item in violations)
        super().__init__(f"{len(violations)} validation error(s): {detail}")


def _quote(value: object) -> str:
    """Renders a value in the JSON form every target quotes offending values in."""

    try:
        return json.dumps(value, ensure_ascii=False)
    except (TypeError, ValueError):
        return repr(value)


def _collect(violations: list[Violation], path: str, error: ValidationError) -> None:
    """Re-paths a nested model's violations under `path` and appends them."""

    for inner in error.violations:
        # A nested violation about the value *itself* carries no path of its own
        # (a union branch's own constraint, an element-level check), so the
        # prefix is the whole path -- never a dangling separator (P11).
        nested = f"{path}.{inner.path}" if inner.path else path
        violations.append(Violation(path=nested, reason=inner.reason))
"#;

/// Emits the decorator shim every model registers its converter through. It
/// exists purely to erase the converter's value-type parameter: binding it on the
/// decorated class is circular for a static type checker, which degrades the
/// model to `Unknown` and poisons every annotation naming it.
fn render_transfer_type_convertible_helper(output: &mut String) {
    output.push_str(TRANSFER_TYPE_CONVERTIBLE_BODY);
}

const TRANSFER_TYPE_CONVERTIBLE_BODY: &str = r#"_ModelT = typing.TypeVar("_ModelT")


def _transfer_type_convertible(
    converter: type[temporalio.converter.TransferTypeConverter[typing.Any, typing.Any]],
) -> collections.abc.Callable[[type[_ModelT]], type[_ModelT]]:
    """Registers a transfer type converter on a model class.

    Wraps `temporalio.converter.transfer_type_convertible` to erase the
    converter's value-type parameter. Binding it directly on the decorated class
    is circular for a static type checker -- the class's type depends on the
    decorator, whose value type depends on the class -- which degrades the model
    to `Unknown`. Erasing it here keeps the decorator idiomatic at each model and
    resolves the cycle.
    """

    return temporalio.converter.transfer_type_convertible(converter)
"#;

/// Emits the materialized-temporal runtime: the pinned narrowed regexes, the
/// Gregorian calendar predicate, the violation-collecting parse helpers, the
/// serialize-side representability predicates, and the canonical formatters the
/// converters call for each of the four kinds. See
/// `specs/json-schema/features/format.md`. The parse is generator-owned rather
/// than `datetime.fromisoformat` alone, which accepts a missing offset and
/// normalizes differently from the narrowed grammar; every value the regex admits
/// is normalized into a spelling `fromisoformat` cannot raise on, so a rejection
/// is always an aggregated `Violation` and never a `ValueError` (P11).
fn render_temporal_helpers(output: &mut String) {
    use crate::json_schema::format::TemporalKind;
    output.push_str(&format!(
        "_TEMPORAL_DATE_TIME_RE = re.compile(r\"{}\")\n",
        TemporalKind::DateTime.pattern()
    ));
    output.push_str(&format!(
        "_TEMPORAL_DATE_RE = re.compile(r\"{}\")\n",
        TemporalKind::Date.pattern()
    ));
    output.push_str(&format!(
        "_TEMPORAL_TIME_RE = re.compile(r\"{}\")\n",
        TemporalKind::Time.pattern()
    ));
    output.push_str(&format!(
        "_TEMPORAL_DURATION_RE = re.compile(r\"{}\")\n",
        TemporalKind::Duration.pattern()
    ));
    output.push_str(TEMPORAL_HELPER_BODY);
}

const TEMPORAL_HELPER_BODY: &str = r#"_TEMPORAL_MAX_DURATION_SECONDS = ((1 << 63) - 1) // 1_000_000_000
# A duration component with more digits than the cap itself is over the cap
# whatever those digits are, which is how the magnitude is bounded before `int()`
# sees it: CPython refuses to convert a string of more than 4300 digits.
_TEMPORAL_MAX_DURATION_DIGITS = len(str(_TEMPORAL_MAX_DURATION_SECONDS))
# `datetime` resolves to microseconds, and `fromisoformat` before Python 3.11
# parses only the fraction widths `isoformat` writes.
_TEMPORAL_FRACTION_DIGITS = 6


def _days_in_month(year: int, month: int) -> int:
    if month in (1, 3, 5, 7, 8, 10, 12):
        return 31
    if month in (4, 6, 9, 11):
        return 30
    if month == 2:
        return 29 if (year % 4 == 0 and year % 100 != 0) or year % 400 == 0 else 28
    return 0


def _valid_temporal_calendar(value: str) -> bool:
    if len(value) < 10:
        return False
    try:
        year, month, day = int(value[0:4]), int(value[5:7]), int(value[8:10])
    except ValueError:
        return False
    # `datetime.MINYEAR` is 1, which is also the shared cross-language floor.
    # Year 0000 is rejected rather than shifted into range, and
    # `_temporal_reason` says so.
    if year < datetime.MINYEAR:
        return False
    maximum = _days_in_month(year, month)
    return maximum > 0 and 1 <= day <= maximum


def _temporal_reason(name: str, value: str) -> str:
    """The reason a rejected temporal string is reported under.

    Year 0000 earns its own clause so the caller sees the shared calendar floor
    rather than only a generic malformed-timestamp reason.
    """

    if value[0:4] == "0000":
        return (
            f"must be a valid {name}, got {_quote(value)}: year 0000 is not"
            f" representable (datetime.MINYEAR is {datetime.MINYEAR})"
        )
    return f"must be a valid {name}, got {_quote(value)}"


def _temporal_isoformat(value: str) -> str:
    """Rewrites a wire temporal into the spelling `fromisoformat` accepts.

    `Z` becomes `+00:00`, and the fractional second is padded or truncated to
    exactly `_TEMPORAL_FRACTION_DIGITS`: before Python 3.11 `fromisoformat`
    parses only what `isoformat` writes, so an RFC 3339 `.1` or `.1234567` --
    which every other target accepts -- would otherwise raise. Digits past the
    sixth are dropped, the loss at `datetime`'s own resolution that P1 allows;
    the canonical output re-trims the padding, so `.1` still writes as `.1`.
    """

    normalized = value.upper()
    if normalized.endswith("Z"):
        normalized = normalized[:-1] + "+00:00"
    dot = normalized.find(".")
    if dot < 0:
        return normalized
    end = dot + 1
    while end < len(normalized) and normalized[end].isdigit():
        end += 1
    fraction = normalized[dot + 1 : end].ljust(_TEMPORAL_FRACTION_DIGITS, "0")
    return (
        normalized[: dot + 1]
        + fraction[:_TEMPORAL_FRACTION_DIGITS]
        + normalized[end:]
    )


def _parse_date_time(
    value: str, path: str, violations: list[Violation]
) -> datetime.datetime | None:
    if _TEMPORAL_DATE_TIME_RE.match(value) is None or not _valid_temporal_calendar(value):
        violations.append(
            Violation(path=path, reason=_temporal_reason("date-time", value))
        )
        return None
    return datetime.datetime.fromisoformat(_temporal_isoformat(value))


def _parse_date(
    value: str, path: str, violations: list[Violation]
) -> datetime.date | None:
    if _TEMPORAL_DATE_RE.match(value) is None or not _valid_temporal_calendar(value):
        violations.append(Violation(path=path, reason=_temporal_reason("date", value)))
        return None
    return datetime.date.fromisoformat(value)


def _parse_time(
    value: str, path: str, violations: list[Violation]
) -> datetime.time | None:
    if _TEMPORAL_TIME_RE.match(value) is None:
        violations.append(Violation(path=path, reason=_temporal_reason("time", value)))
        return None
    return datetime.time.fromisoformat(_temporal_isoformat(value))


def _parse_duration(
    value: str, path: str, violations: list[Violation]
) -> datetime.timedelta | None:
    if _TEMPORAL_DURATION_RE.match(value) is None:
        violations.append(
            Violation(path=path, reason=_temporal_reason("duration", value))
        )
        return None
    total = 0
    number = ""
    for char in value[2:]:
        if char.isdigit():
            number += char
            continue
        digits = number.lstrip("0")
        number = ""
        if len(digits) > _TEMPORAL_MAX_DURATION_DIGITS:
            # Over the cap by digit count alone (see the constant), so the
            # conversion `int()` would refuse is never attempted.
            total = _TEMPORAL_MAX_DURATION_SECONDS + 1
            break
        total += int(digits or "0") * {"H": 3600, "M": 60, "S": 1}[char]
        if total > _TEMPORAL_MAX_DURATION_SECONDS:
            break
    if total > _TEMPORAL_MAX_DURATION_SECONDS:
        violations.append(
            Violation(path=path, reason=_temporal_reason("duration", value))
        )
        return None
    return datetime.timedelta(seconds=total)


def _check_temporal_offset(
    name: str,
    value: datetime.datetime | datetime.time,
    offset: datetime.timedelta,
    path: str,
    violations: list[Violation],
) -> None:
    """Asserts a UTC offset is a whole number of minutes, the finest the wire
    form spells (`tzinfo` allows seconds, which the offset would silently lose).
    """

    if offset % datetime.timedelta(minutes=1):
        violations.append(
            Violation(
                path=path,
                reason=(
                    f"must be a valid {name}, got {_quote(str(value))}: "
                    f"the UTC offset {offset} is not a whole number of minutes"
                ),
            )
        )


def _check_date_time(
    value: datetime.datetime, path: str, violations: list[Violation]
) -> None:
    """Asserts a datetime is writable as a wire date-time (P12).

    A dataclass is constructed unchecked, so a naive datetime -- with no offset
    the required wire form could carry -- reaches serialize; without this it
    would emit a value this module's own parser rejects.
    """

    offset = value.utcoffset()
    if offset is None:
        violations.append(
            Violation(
                path=path,
                reason=(
                    f"must be a valid date-time, got {_quote(str(value))}: "
                    "a naive datetime carries no UTC offset"
                ),
            )
        )
        return
    _check_temporal_offset("date-time", value, offset, path, violations)


def _check_time(value: datetime.time, path: str, violations: list[Violation]) -> None:
    """Asserts a time is writable as a wire time (P12). The offset is optional in
    the grammar, so only its precision is held to anything."""

    offset = value.utcoffset()
    if offset is not None:
        _check_temporal_offset("time", value, offset, path, violations)


def _check_duration(
    value: datetime.timedelta, path: str, violations: list[Violation]
) -> None:
    """Asserts a timedelta is writable as a wire duration (P12): the grammar is
    unsigned, whole-second and capped, and a `timedelta` is none of those."""

    if value < datetime.timedelta(0):
        reason = "a duration cannot be negative"
    elif value % datetime.timedelta(seconds=1):
        reason = "a duration cannot carry a fraction of a second"
    elif value.total_seconds() > _TEMPORAL_MAX_DURATION_SECONDS:
        reason = f"a duration cannot exceed {_TEMPORAL_MAX_DURATION_SECONDS} seconds"
    else:
        return
    violations.append(
        Violation(
            path=path,
            reason=f"must be a valid duration, got {_quote(str(value))}: {reason}",
        )
    )


def _temporal_frac(microsecond: int) -> str:
    if microsecond == 0:
        return ""
    return "." + f"{microsecond:06d}".rstrip("0")


def _temporal_offset(value: datetime.datetime | datetime.time) -> str:
    offset = value.utcoffset()
    if offset is None:
        return ""
    total = int(offset.total_seconds())
    if total == 0:
        return "Z"
    sign = "+" if total > 0 else "-"
    total = abs(total)
    return f"{sign}{total // 3600:02d}:{(total % 3600) // 60:02d}"


def _format_date_time(value: datetime.datetime) -> str:
    return (
        f"{value.year:04d}-{value.month:02d}-{value.day:02d}"
        f"T{value.hour:02d}:{value.minute:02d}:{value.second:02d}"
        f"{_temporal_frac(value.microsecond)}{_temporal_offset(value)}"
    )


def _format_date(value: datetime.date) -> str:
    return f"{value.year:04d}-{value.month:02d}-{value.day:02d}"


def _format_time(value: datetime.time) -> str:
    return (
        f"{value.hour:02d}:{value.minute:02d}:{value.second:02d}"
        f"{_temporal_frac(value.microsecond)}{_temporal_offset(value)}"
    )


def _format_duration(value: datetime.timedelta) -> str:
    total = int(value.total_seconds())
    if total == 0:
        return "PT0S"
    hours, remainder = divmod(total, 3600)
    minutes, seconds = divmod(remainder, 60)
    out = "PT"
    if hours:
        out += f"{hours}H"
    if minutes:
        out += f"{minutes}M"
    if seconds:
        out += f"{seconds}S"
    return out
"#;

/// Emits the materialized-`contentEncoding` runtime: the pinned canonical
/// base64 / base64url regexes (the validity oracle) plus the violation-collecting
/// decode and canonical encode helpers the converters call. The codec is
/// generator-owned so the accept/reject line and the canonical output are the
/// same in every target. See `specs/json-schema/features/contentEncoding.md`.
fn render_content_encoding_helpers(output: &mut String) {
    use crate::json_schema::content_encoding::Encoding;
    output.push_str(&format!(
        "_BASE64_RE = re.compile({}, re.ASCII)\n",
        python_string_literal(&crate::json_schema::pattern::rewrite_end_anchor(
            Encoding::Base64.pattern(),
            r"\Z"
        ))
    ));
    output.push_str(&format!(
        "_BASE64URL_RE = re.compile({}, re.ASCII)\n",
        python_string_literal(&crate::json_schema::pattern::rewrite_end_anchor(
            Encoding::Base64Url.pattern(),
            r"\Z"
        ))
    ));
    output.push_str(CONTENT_ENCODING_HELPER_BODY);
}

const CONTENT_ENCODING_HELPER_BODY: &str = r#"

def _parse_base64(value: str, path: str, violations: list[Violation]) -> bytes | None:
    if _BASE64_RE.match(value) is None:
        violations.append(
            Violation(path=path, reason=f"must be base64-encoded, got {_quote(value)}")
        )
        return None
    return base64.b64decode(value, validate=True)


def _format_base64(value: bytes) -> str:
    return base64.b64encode(value).decode("ascii")


def _parse_base64url(value: str, path: str, violations: list[Violation]) -> bytes | None:
    if _BASE64URL_RE.match(value) is None:
        violations.append(
            Violation(path=path, reason=f"must be base64url-encoded, got {_quote(value)}")
        )
        return None
    return base64.urlsafe_b64decode(value + "=" * (-len(value) % 4))


def _format_base64url(value: bytes) -> str:
    return base64.urlsafe_b64encode(value).rstrip(b"=").decode("ascii")
"#;

/// Emits the `_check_unique_items` runtime helper: asserts pairwise-distinct
/// elements, reporting the first duplicate's index and the index it repeats --
/// the same reason every other target emits. Comparison follows JSON value
/// equality (notably, booleans are distinct from numbers) over a list rather
/// than hashing; arrays are small and correctness beats the O(n^2) (P2).
/// See `specs/json-schema/features/uniqueItems.md`.
fn render_unique_items_helper(output: &mut String) {
    output.push_str(UNIQUE_ITEMS_HELPER_BODY);
}

const UNIQUE_ITEMS_HELPER_BODY: &str = r#"def _json_values_equal(left: typing.Any, right: typing.Any) -> bool:
    """Compares JSON values without Python's bool-as-int equality leak."""

    left_is_number = isinstance(left, (int, float)) and not isinstance(left, bool)
    right_is_number = isinstance(right, (int, float)) and not isinstance(right, bool)
    if left_is_number or right_is_number:
        return left_is_number and right_is_number and left == right
    if type(left) is not type(right):
        return False
    if isinstance(left, list):
        left_list = typing.cast("list[typing.Any]", left)
        right_list = typing.cast("list[typing.Any]", right)
        return len(left_list) == len(right_list) and all(
            _json_values_equal(a, b) for a, b in zip(left_list, right_list)
        )
    if isinstance(left, dict):
        left_dict = typing.cast("dict[typing.Any, typing.Any]", left)
        right_dict = typing.cast("dict[typing.Any, typing.Any]", right)
        return left_dict.keys() == right_dict.keys() and all(
            _json_values_equal(left_dict[key], right_dict[key]) for key in left_dict
        )
    return left == right


def _check_unique_items(
    value: list[typing.Any], path: str, violations: list[Violation]
) -> None:
    """Asserts an array's elements are pairwise distinct."""

    seen: list[typing.Any] = []
    for index, element in enumerate(value):
        for earlier, previous in enumerate(seen):
            if _json_values_equal(previous, element):
                violations.append(
                    Violation(
                        path=path,
                        reason=(
                            f"duplicate items: element at index {index} "
                            f"equals index {earlier}"
                        ),
                    )
                )
                break
        seen.append(element)
"#;

/// Emits the `_check_contains` runtime helper: asserts the number of elements
/// matching a predicate falls in `[min_contains, max_contains]`, with the same
/// reasons every other target emits. See
/// `specs/json-schema/features/contains.md`.
fn render_contains_helper(output: &mut String) {
    output.push_str(CONTAINS_HELPER_BODY);
}

const CONTAINS_HELPER_BODY: &str = r#"def _check_contains(
    value: list[typing.Any],
    matches: typing.Callable[[typing.Any], bool],
    min_contains: int,
    max_contains: int | None,
    bounded_min: bool,
    path: str,
    violations: list[Violation],
) -> None:
    """Asserts how many of an array's elements match the `contains` schema."""

    match_count = sum(1 for element in value if matches(element))
    if match_count < min_contains:
        if bounded_min:
            violations.append(
                Violation(
                    path=path,
                    reason=(
                        f"too few matching items: at least {min_contains}, "
                        f"got {match_count}"
                    ),
                )
            )
        else:
            violations.append(
                Violation(path=path, reason="no element matches the required schema")
            )
    if max_contains is not None and match_count > max_contains:
        violations.append(
            Violation(
                path=path,
                reason=(
                    f"too many matching items: at most {max_contains}, "
                    f"got {match_count}"
                ),
            )
        )
"#;

// ---------------------------------------------------------------------------
// Shared constraint checks (P12 layer 2)
//
// One set of emitters, called by both converter directions, so a value is held
// to identical predicates on the way in and on the way out. Every check appends
// a `Violation` and keeps going; the caller raises the single aggregated
// `ValidationError` (P11). The emitted lines are deliberately unwrapped —
// `ruff format` reflows them to the 88-column budget.
// ---------------------------------------------------------------------------

/// The module-level compiled-regex const name for a `pattern`, keyed by the
/// (normalized) pattern text so identical patterns share one compiled instance
/// per module. Stable FNV-1a hash → a valid Python identifier.
pub(crate) fn py_pattern_const_name(pattern: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in pattern.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("_PATTERN_{hash:016X}")
}

/// The JSON rendering of a closed-value literal, for the *message* half of a
/// `const`/`enum` violation. Deliberately JSON and not a Python literal so the
/// reason reads identically to every other target (`true`, not `True`); the
/// *comparison* half uses [`python_value_literal`].
fn py_reason_literal(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".to_string())
}

/// Emits `if <condition>: violations.append(Violation(path=…, reason=…))`.
/// `reason` is a complete Python string expression — usually an f-string whose
/// interpolation names the offending value.
fn render_py_violation_if(
    output: &mut String,
    indent: &str,
    condition: &str,
    path_expr: &str,
    reason: &str,
) {
    output.push_str(indent);
    output.push_str("if ");
    output.push_str(condition);
    output.push_str(":\n");
    output.push_str(indent);
    output.push_str("    violations.append(Violation(path=");
    output.push_str(path_expr);
    output.push_str(", reason=");
    output.push_str(reason);
    output.push_str("))\n");
}

/// The largest finite IEEE-754 binary64 magnitude, the range a JSON `number`
/// carries in every other target (Go `float64`, TS `number`, Java `double`).
const PY_BINARY64_MAX: &str = "1.7976931348623157e308";

/// Emits the numeric-constraint predicates over `value_expr` (an in-scope
/// `int`/`float`). `value_expr` is always a bare or dotted name, never a
/// subscript, so it is safe to interpolate inside a double-quoted f-string.
///
/// A `number` is guarded for finiteness first, and every other predicate hangs
/// off that guard's `else`. Python is the only target that can hold a
/// non-finite `number` at all: `json.loads` accepts the `Infinity`,
/// `-Infinity` and `NaN` literals its dialect adds (Go's `json.Unmarshal` and
/// `JSON.parse` reject the bytes outright, and Jackson rejects them by
/// default), and it decodes an over-range literal as `inf` or as an unbounded
/// `int` where Go's `json.Number.Float64` reports a range error. Left
/// unchecked the value re-serializes as `Infinity`/`NaN` — bytes no other
/// target can read (P1) — so it is rejected in both directions (P12). It also
/// has to be rejected *before* the other predicates: `nan` compares false
/// against every bound, and `math.fmod` raises rather than returns
/// (`ValueError` on `inf`, `OverflowError` on an int past the binary64 range),
/// which would escape the aggregated `ValidationError` (P11). An `integer`
/// needs no guard — `_parse_spec_integer` rejects a non-finite wire value and
/// caps the magnitude, and an in-memory `int` is always finite.
fn render_py_numeric_checks(
    output: &mut String,
    value_expr: &str,
    path_expr: &str,
    schema: &Schema,
    indent: &str,
) {
    let is_integer = schema.ty.as_ref().and_then(Value::as_str) == Some("integer");
    let body_indent = if is_integer {
        indent.to_string()
    } else {
        format!("{indent}    ")
    };
    let mut body = String::new();
    let mut emit = |condition: String, reason: String| {
        render_py_violation_if(&mut body, &body_indent, &condition, path_expr, &reason);
    };
    if let Some(min) = &schema.minimum {
        let bound = py_bound_literal(min, is_integer);
        emit(
            format!("{value_expr} < {bound}"),
            format!("f\"must be >= {bound}, got {{{value_expr}}}\""),
        );
    }
    if let Some(max) = &schema.maximum {
        let bound = py_bound_literal(max, is_integer);
        emit(
            format!("{value_expr} > {bound}"),
            format!("f\"must be <= {bound}, got {{{value_expr}}}\""),
        );
    }
    if let Some(min) = &schema.exclusive_minimum {
        let bound = py_bound_literal(min, is_integer);
        emit(
            format!("{value_expr} <= {bound}"),
            format!("f\"must be > {bound}, got {{{value_expr}}}\""),
        );
    }
    if let Some(max) = &schema.exclusive_maximum {
        let bound = py_bound_literal(max, is_integer);
        emit(
            format!("{value_expr} >= {bound}"),
            format!("f\"must be < {bound}, got {{{value_expr}}}\""),
        );
    }
    if let Some(divisor) = &schema.multiple_of {
        let bound = py_bound_literal(divisor, is_integer);
        // An integer field divides exactly; a number field goes through
        // `math.fmod` so divisibility is bit-identical across all four targets
        // rather than merely close. See features/multipleOf.md.
        let condition = if is_integer {
            format!("{value_expr} % {bound} != 0")
        } else {
            format!("math.fmod({value_expr}, {bound}) != 0")
        };
        emit(
            condition,
            format!("f\"must be a multiple of {bound}, got {{{value_expr}}}\""),
        );
    }
    if is_integer {
        output.push_str(&body);
        return;
    }
    // The chained comparison is the one finiteness test that never raises:
    // `math.isfinite` overflows on an int past the binary64 range, while an
    // `int`/`float` comparison against a float bound is exact for any
    // magnitude, and `nan` fails it the way every other value out of range
    // does.
    render_py_violation_if(
        output,
        indent,
        &format!("not (-{PY_BINARY64_MAX} <= {value_expr} <= {PY_BINARY64_MAX})"),
        path_expr,
        &format!("f\"must be a finite number, got {{{value_expr}}}\""),
    );
    if !body.is_empty() {
        output.push_str(indent);
        output.push_str("else:\n");
        output.push_str(&body);
    }
}

/// Emits the string predicates over `value_expr` (an in-scope `str`).
/// `len()` on a `str` is the Unicode code-point count, which is what the spec
/// means — no surrogate correction needed as in TypeScript.
fn render_py_string_checks(
    output: &mut String,
    value_expr: &str,
    path_expr: &str,
    schema: &Schema,
    indent: &str,
) {
    let length = format!("len({value_expr})");
    if let Some(min) = schema.min_length {
        render_py_violation_if(
            output,
            indent,
            &format!("{length} < {min}"),
            path_expr,
            &format!("f\"must have length >= {min}, got {{{length}}}\""),
        );
    }
    if let Some(max) = schema.max_length {
        render_py_violation_if(
            output,
            indent,
            &format!("{length} > {max}"),
            path_expr,
            &format!("f\"must have length <= {max}, got {{{length}}}\""),
        );
    }
    if let Some(pattern) = &schema.pattern {
        render_py_pattern_check(output, value_expr, path_expr, pattern, indent);
    }
    if let Some(format) = &schema.format {
        render_py_format_check(output, value_expr, path_expr, format, indent);
    }
}

/// Emits the `pattern` predicate. The message reads the pattern text back off
/// the compiled object (`.pattern`) rather than embedding it in the f-string,
/// which sidesteps escaping a regex inside a Python string literal entirely.
/// `re.search` is unanchored — never `match` (anchors the start) or `fullmatch`.
fn render_py_pattern_check(
    output: &mut String,
    value_expr: &str,
    path_expr: &str,
    pattern: &str,
    indent: &str,
) {
    let rewritten = crate::json_schema::pattern::rewrite_end_anchor(pattern, r"\Z");
    let const_name = py_pattern_const_name(&rewritten);
    render_py_violation_if(
        output,
        indent,
        &format!("{const_name}.search({value_expr}) is None"),
        path_expr,
        &format!("f\"must match pattern {{{const_name}.pattern}}, got {{_quote({value_expr})}}\""),
    );
}

/// Emits the `format` predicate: the length guard (when the format has one)
/// short-circuits **before** the pinned regex, so one combined condition
/// produces a single violation naming the format and the value.
fn render_py_format_check(
    output: &mut String,
    value_expr: &str,
    path_expr: &str,
    format: &str,
    indent: &str,
) {
    let Some(check) = crate::json_schema::format::check_for(format) else {
        return;
    };
    let rewritten = crate::json_schema::pattern::rewrite_end_anchor(&check.pattern, r"\Z");
    let const_name = py_pattern_const_name(&rewritten);
    let mut condition = String::new();
    if let Some(max) = check.max_code_points {
        condition.push_str(&format!("len({value_expr}) > {max} or "));
    }
    condition.push_str(&format!("{const_name}.search({value_expr}) is None"));
    render_py_violation_if(
        output,
        indent,
        &condition,
        path_expr,
        &format!(
            "f\"must be a valid {}, got {{_quote({value_expr})}}\"",
            check.name
        ),
    );
}

/// Emits the array predicates over `array_expr` (an in-scope `list`).
fn render_py_array_checks(
    output: &mut String,
    array_expr: &str,
    path_expr: &str,
    schema: &Schema,
    indent: &str,
) -> Result<()> {
    let length = format!("len({array_expr})");
    if let Some(min) = schema.min_items {
        render_py_violation_if(
            output,
            indent,
            &format!("{length} < {min}"),
            path_expr,
            &format!("f\"must have at least {min} items, got {{{length}}}\""),
        );
    }
    if let Some(max) = schema.max_items {
        render_py_violation_if(
            output,
            indent,
            &format!("{length} > {max}"),
            path_expr,
            &format!("f\"must have at most {max} items, got {{{length}}}\""),
        );
    }
    if schema.unique_items == Some(true) {
        output.push_str(indent);
        output.push_str(&format!(
            "_check_unique_items({array_expr}, {path_expr}, violations)\n"
        ));
    }
    if let Some(matcher) = &schema.contains {
        let condition = py_matcher_condition(matcher, schema.items.as_deref(), "element")?;
        let effective_min = schema.min_contains.unwrap_or(1);
        let max_arg = match schema.max_contains {
            Some(max) => max.to_string(),
            None => "None".to_string(),
        };
        let bounded_min = if schema.min_contains.is_some() {
            "True"
        } else {
            "False"
        };
        output.push_str(indent);
        output.push_str(&format!(
            "_check_contains({array_expr}, lambda element: {condition}, {effective_min}, {max_arg}, {bounded_min}, {path_expr}, violations)\n"
        ));
    }
    Ok(())
}

/// Emits the object member-count predicates over `count_expr` (the number of
/// distinct wire member keys). These are whole-object constraints, so the path
/// is the empty string.
fn render_py_property_count_checks(
    output: &mut String,
    count_expr: &str,
    schema: &Schema,
    indent: &str,
) {
    if let Some(min) = schema.min_properties {
        render_py_violation_if(
            output,
            indent,
            &format!("{count_expr} < {min}"),
            "\"\"",
            &format!("f\"must have at least {min} properties, got {{{count_expr}}}\""),
        );
    }
    if let Some(max) = schema.max_properties {
        render_py_violation_if(
            output,
            indent,
            &format!("{count_expr} > {max}"),
            "\"\"",
            &format!("f\"must have at most {max} properties, got {{{count_expr}}}\""),
        );
    }
}

/// Emits the `propertyNames` key-shape predicate over `keys_expr`, applying the
/// supported string matcher vocabulary to each key.
fn render_py_property_name_checks(
    output: &mut String,
    keys_expr: &str,
    subschema: &Schema,
    indent: &str,
) {
    if subschema.min_length.is_none()
        && subschema.max_length.is_none()
        && subschema.pattern.is_none()
        && subschema.format.is_none()
        && subschema.enum_values.is_none()
    {
        return;
    }
    output.push_str(indent);
    output.push_str(&format!("for key in {keys_expr}:\n"));
    let inner = format!("{indent}    ");
    if let Some(min) = subschema.min_length {
        render_py_violation_if(
            output,
            &inner,
            &format!("len(key) < {min}"),
            "key",
            &format!(
                "f\"invalid property name {{_quote(key)}}: must have length >= {min}, got {{len(key)}}\""
            ),
        );
    }
    if let Some(max) = subschema.max_length {
        render_py_violation_if(
            output,
            &inner,
            &format!("len(key) > {max}"),
            "key",
            &format!(
                "f\"invalid property name {{_quote(key)}}: must have length <= {max}, got {{len(key)}}\""
            ),
        );
    }
    if let Some(pattern) = &subschema.pattern {
        let rewritten = crate::json_schema::pattern::rewrite_end_anchor(pattern, r"\Z");
        let const_name = py_pattern_const_name(&rewritten);
        render_py_violation_if(
            output,
            &inner,
            &format!("{const_name}.search(key) is None"),
            "key",
            &format!(
                "f'invalid property name {{_quote(key)}}: must match pattern {{{const_name}.pattern}}'"
            ),
        );
    }
    if let Some(values) = &subschema.enum_values {
        let allowed = values
            .iter()
            .filter_map(Value::as_str)
            .map(python_string_literal)
            .collect::<Vec<_>>();
        if !allowed.is_empty() {
            render_py_violation_if(
                output,
                &inner,
                &format!("key not in {}", py_value_tuple(&allowed)),
                "key",
                "f'invalid property name {_quote(key)}: must equal an allowed value'",
            );
        }
    }
    if let Some(format) = &subschema.format
        && let Some(check) = crate::json_schema::format::check_for(format)
    {
        let rewritten = crate::json_schema::pattern::rewrite_end_anchor(&check.pattern, r"\Z");
        let const_name = py_pattern_const_name(&rewritten);
        let length_guard = check
            .max_code_points
            .map(|max| format!("len(key) > {max} or "))
            .unwrap_or_default();
        render_py_violation_if(
            output,
            &inner,
            &format!("{length_guard}{const_name}.search(key) is None"),
            "key",
            &format!(
                "f'invalid property name {{_quote(key)}}: must be a valid {}'",
                check.name
            ),
        );
    }
}

/// Emits the `dependentRequired` cross-field presence predicate over the
/// presence mapping `obj_expr`: for each present trigger key, each dependent
/// key must also be present.
fn render_py_dependent_required(
    output: &mut String,
    obj_expr: &str,
    schema: &Schema,
    indent: &str,
) {
    let Some(dependent_required) = &schema.dependent_required else {
        return;
    };
    for (trigger, deps) in dependent_required {
        output.push_str(indent);
        output.push_str(&format!(
            "if {} in {obj_expr}:\n",
            python_string_literal(trigger)
        ));
        let inner = format!("{indent}    ");
        for dep in deps {
            render_py_violation_if(
                output,
                &inner,
                &format!("{} not in {obj_expr}", python_string_literal(dep)),
                &python_string_literal(dep),
                &python_string_literal(&format!(
                    "property \"{dep}\" is required when \"{trigger}\" is present"
                )),
            );
        }
    }
}

/// Emits the closed value-set membership predicate over an in-memory
/// `value_expr` for the serialize path, producing the same informative reason
/// the parse path does. `compare_exprs` are the admissible Python literals.
fn render_py_closed_value_check(
    output: &mut String,
    compare_exprs: &[String],
    value_expr: &str,
    path_expr: &str,
    indent: &str,
    reason: &str,
) {
    // The member is typed by the closed set it belongs to, so a direct comparison
    // against each admissible value is statically dead code. Widening to `object`
    // keeps the runtime check — a value mutated past the type system still has to
    // fail before it reaches the wire (P12).
    let membership = format!(
        "typing.cast(\"object\", {value_expr}) not in {}",
        py_value_tuple(compare_exprs)
    );
    render_py_violation_if(output, indent, &membership, path_expr, reason);
}

/// The Python tuple of admissible literals a closed value set is tested against,
/// with the comma a one-member tuple needs. Membership against a tuple is the
/// one shape both directions and both keywords use — a `const` is the one-member
/// `enum` — and it is what keeps the emitted test out of the `!= True` /
/// `!= None` comparisons a per-member `!=` chain would produce, which read as
/// unidiomatic Python and are lint errors (ruff E712/E711) in the generated
/// output (P2).
fn py_value_tuple(compare_exprs: &[String]) -> String {
    match compare_exprs {
        [single] => format!("({single},)"),
        many => format!("({})", many.join(", ")),
    }
}

/// True when a field schema carries a constraint the serialize path must
/// re-check over the in-memory value (P12, both directions). Mirrors the
/// dispatch in [`render_py_field_checks`].
fn py_field_needs_serialize_check(schema: &Schema) -> bool {
    // A nullability wrapper declares nothing itself; its non-null branch carries
    // the constraints, checked under a `is not None` guard.
    if let Some(non_null) = nullable_member_schema(schema) {
        return py_field_needs_serialize_check(non_null);
    }
    if schema.const_value.is_some() || schema.enum_values.is_some() {
        return true;
    }
    // A sum type always has something to check: even with no constraint on any
    // branch, the member is held to matching *some* branch (see
    // `render_py_union_value_checks`).
    if is_py_union(schema) {
        return true;
    }
    // An inline sum type: any branch that declares something is re-checked
    // against the member it holds. A `$ref` branch validates through its own
    // converter, so only the non-reference branches count.
    if schema.one_of.is_some() {
        return schema
            .one_of
            .iter()
            .flatten()
            .filter(|branch| branch.reference.is_none())
            .any(py_field_needs_serialize_check);
    }
    match schema.ty.as_ref().and_then(Value::as_str) {
        Some("string") => {
            schema.min_length.is_some()
                || schema.max_length.is_some()
                || schema.pattern.is_some()
                || schema.format.is_some()
        }
        // A `number` always carries one: the finiteness guard applies whether or
        // not a bound is declared, because `json.dumps` would otherwise write an
        // in-memory `inf`/`nan` out as bytes no other target can read (see
        // [`render_py_numeric_checks`]).
        Some("number") => true,
        Some("integer") => {
            schema.minimum.is_some()
                || schema.maximum.is_some()
                || schema.exclusive_minimum.is_some()
                || schema.exclusive_maximum.is_some()
                || schema.multiple_of.is_some()
        }
        Some("array") => {
            schema.min_items.is_some()
                || schema.max_items.is_some()
                || schema.unique_items == Some(true)
                || schema.contains.is_some()
                || schema
                    .items
                    .as_deref()
                    .is_some_and(py_field_needs_serialize_check)
        }
        _ => false,
    }
}

/// True when a model's `to_transfer_type` must run collecting validation before
/// building the wire object: any constrained declared field, a constrained
/// typed-map member, or an object-level count/name/dependency constraint.
fn py_model_needs_serialize_validation(schema: &Schema) -> Result<bool> {
    // A mixed declared/catch-all object must reject a programmatically-created
    // catch-all entry whose wire key is already owned by a declared field.
    if is_open_object(schema) {
        return Ok(true);
    }
    if schema.min_properties.is_some()
        || schema.max_properties.is_some()
        || schema.dependent_required.is_some()
        || schema.property_names.is_some()
    {
        return Ok(true);
    }
    if let Some(value_schema) = typed_additional_properties_schema(schema)?
        && py_field_needs_serialize_check(&value_schema)
    {
        return Ok(true);
    }
    if let Some(properties) = &schema.properties {
        for property in properties.values() {
            if py_field_needs_serialize_check(property) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Emits the per-value constraint checks over an in-memory `value_expr`,
/// reusing the same emitters the parse path calls. References and
/// contentEncoding carry no check here — a nested converter validates its own
/// value, and `bytes` re-encodes losslessly. A materialized temporal does carry
/// one: the native type is wider than the narrowed wire grammar, so it is held to
/// what that grammar can spell. `models` resolves a union branch's `$ref` to the
/// shape it names; a position that cannot hold a union passes none.
fn render_py_field_checks(
    output: &mut String,
    schema: &Schema,
    models: &[&PlannedJsonType],
    value_expr: &str,
    path_expr: &str,
    indent: &str,
) -> Result<()> {
    // A nullability wrapper's constraints live on its non-null branch; the
    // caller has already guarded the value against `None`.
    if let Some(non_null) = nullable_member_schema(schema) {
        return render_py_field_checks(output, non_null, models, value_expr, path_expr, indent);
    }
    // An inline sum type narrows to the branch it holds and runs that branch's
    // own checks. An object branch validates through its own converter instead,
    // and contributes only its arm of the no-branch-matched test.
    if is_py_union(schema) {
        if let Some(union) = classify_py_union(schema, models)? {
            render_py_union_value_checks(output, &union, models, value_expr, path_expr, indent)?;
        }
        return Ok(());
    }
    // A materialized temporal is held to what the wire grammar can spell: a
    // `datetime` may be naive and a `timedelta` may be negative, sub-second, or
    // past the cap, none of which has a wire form. Construction is unchecked, so
    // without this the formatter would emit bytes this module's own parser
    // rejects (P12).
    if let Some(kind) = temporal_kind_direct(schema) {
        if let Some(check) = python_temporal_check_fn(kind) {
            output.push_str(indent);
            output.push_str(&format!("{check}({value_expr}, {path_expr}, violations)\n"));
        }
        render_py_materialized_wire_checks(
            output,
            schema,
            &format!("{}({value_expr})", python_temporal_format_fn(kind)),
            path_expr,
            indent,
        )?;
        return Ok(());
    }
    if let Some(encoding) = content_encoding_direct(schema) {
        render_py_materialized_wire_checks(
            output,
            schema,
            &format!(
                "{}({value_expr})",
                python_content_encoding_format_fn(encoding)
            ),
            path_expr,
            indent,
        )?;
        return Ok(());
    }
    if let Some(const_value) = &schema.const_value {
        let literal = python_value_literal(const_value)?;
        let reason =
            python_string_literal(&format!("must equal {}", py_reason_literal(const_value)));
        render_py_closed_value_check(
            output,
            std::slice::from_ref(&literal),
            value_expr,
            path_expr,
            indent,
            &reason,
        );
        return Ok(());
    }
    if let Some(values) = &schema.enum_values {
        let literals = values
            .iter()
            .map(python_value_literal)
            .collect::<Result<Vec<_>>>()?;
        let reason = py_enum_reason(values, value_expr);
        render_py_closed_value_check(output, &literals, value_expr, path_expr, indent, &reason);
        return Ok(());
    }
    match schema.ty.as_ref().and_then(Value::as_str) {
        Some("string") => render_py_string_checks(output, value_expr, path_expr, schema, indent),
        Some("number") | Some("integer") => {
            render_py_numeric_checks(output, value_expr, path_expr, schema, indent)
        }
        Some("array") => {
            render_py_array_checks(output, value_expr, path_expr, schema, indent)?;
            if let Some(items) = schema.items.as_deref()
                && py_field_needs_serialize_check(items)
            {
                // Validate every materialized element before the enclosing value
                // is emitted. The depth-derived names stay distinct for nested
                // arrays, so an inner loop cannot overwrite the outer index used
                // to build paths such as `numberGrid[0][1]`.
                let depth = indent.len();
                let index = format!("item_index_{depth}");
                let element = format!("item_element_{depth}");
                let loop_indent = format!("{indent}    ");
                let item_path = py_indexed_path(path_expr, &index);
                let nullable = allows_null(items);
                let check_indent = if nullable {
                    format!("{loop_indent}    ")
                } else {
                    loop_indent.clone()
                };
                let mut checks = String::new();
                render_py_field_checks(
                    &mut checks,
                    items,
                    models,
                    &element,
                    &item_path,
                    &check_indent,
                )?;
                // Some materialized schemas (for example `date`) report that
                // they need serialize validation but their native helper proves
                // sufficient and emits no inline predicate. Do not leave an
                // empty loop behind in that case.
                if !checks.is_empty() {
                    output.push_str(indent);
                    output.push_str(&format!(
                        "for {index}, {element} in enumerate({value_expr}):\n"
                    ));
                    if nullable {
                        output.push_str(&loop_indent);
                        output.push_str(&format!("if {element} is not None:\n"));
                    }
                    output.push_str(&checks);
                }
            }
        }
        _ => {}
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Reason-string composition
//
// Every reason that quotes a JSON value is an f-string delimited by single
// quotes, so the double quotes JSON (and therefore every other target) reports
// offending values in can appear verbatim. A double-quoted f-string could not
// carry them: nesting the delimiter inside an f-string only became legal in
// Python 3.12, and the emitted floor is 3.10.
// ---------------------------------------------------------------------------

/// Escapes text for the *literal* portion of a single-quoted Python f-string.
/// Backslash escapes are legal there on every supported version (only the
/// expression portion forbids them before 3.12).
fn py_fstring_text(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('\'', "\\'")
        .replace('{', "{{")
        .replace('}', "}}")
}

/// The `enum` membership reason, naming the admissible set in its JSON form and
/// the offending value through `_quote`.
fn py_enum_reason(values: &[Value], value_expr: &str) -> String {
    let rendered = values
        .iter()
        .map(py_reason_literal)
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "f'must be one of [{}], got {{_quote({value_expr})}}'",
        py_fstring_text(&rendered)
    )
}

/// The text of a Python double-quoted string literal, when it carries no escape
/// that would have to be re-escaped for an f-string.
fn py_literal_text(expr: &str) -> Option<String> {
    let inner = expr.strip_prefix('"')?.strip_suffix('"')?;
    if inner.contains('\\') || inner.contains('"') {
        return None;
    }
    Some(inner.to_string())
}

/// The path expression for an array element: `tags[0]`, or `<parent>[0]` when the
/// parent path is itself a runtime expression. The parent is interpolated by
/// *name* rather than nested as an f-string, because a nested f-string is 3.12+.
fn py_indexed_path(path_expr: &str, index_var: &str) -> String {
    match py_literal_text(path_expr) {
        Some(text) => format!("f'{}[{{{index_var}}}]'", py_fstring_text(&text)),
        // Concatenate a new suffix instead of interpolating the already-dynamic
        // expression. This remains valid on Python 3.10 even when `path_expr`
        // is itself an f-string from an outer array level.
        None => format!("{path_expr} + f'[{{{index_var}}}]'"),
    }
}

// ---------------------------------------------------------------------------
// Materialized value types
// ---------------------------------------------------------------------------

/// The native Python type a materialized temporal `format` field carries.
fn python_temporal_type(kind: crate::json_schema::format::TemporalKind) -> &'static str {
    use crate::json_schema::format::TemporalKind;
    match kind {
        TemporalKind::DateTime => "datetime.datetime",
        TemporalKind::Date => "datetime.date",
        TemporalKind::Time => "datetime.time",
        TemporalKind::Duration => "datetime.timedelta",
    }
}

fn python_temporal_parse_fn(kind: crate::json_schema::format::TemporalKind) -> &'static str {
    use crate::json_schema::format::TemporalKind;
    match kind {
        TemporalKind::DateTime => "_parse_date_time",
        TemporalKind::Date => "_parse_date",
        TemporalKind::Time => "_parse_time",
        TemporalKind::Duration => "_parse_duration",
    }
}

fn python_temporal_format_fn(kind: crate::json_schema::format::TemporalKind) -> &'static str {
    use crate::json_schema::format::TemporalKind;
    match kind {
        TemporalKind::DateTime => "_format_date_time",
        TemporalKind::Date => "_format_date",
        TemporalKind::Time => "_format_time",
        TemporalKind::Duration => "_format_duration",
    }
}

/// The predicate a materialized temporal value is held to before it is written,
/// or `None` for `date` — every `datetime.date` Python can hold writes a valid
/// wire date, so there is nothing to assert. See `_check_date_time` in the
/// runtime for why the other three do have something to assert.
fn python_temporal_check_fn(
    kind: crate::json_schema::format::TemporalKind,
) -> Option<&'static str> {
    use crate::json_schema::format::TemporalKind;
    match kind {
        TemporalKind::DateTime => Some("_check_date_time"),
        TemporalKind::Time => Some("_check_time"),
        TemporalKind::Duration => Some("_check_duration"),
        TemporalKind::Date => None,
    }
}

fn python_content_encoding_parse_fn(
    encoding: crate::json_schema::content_encoding::Encoding,
) -> &'static str {
    use crate::json_schema::content_encoding::Encoding;
    match encoding {
        Encoding::Base64 => "_parse_base64",
        Encoding::Base64Url => "_parse_base64url",
    }
}

fn python_content_encoding_format_fn(
    encoding: crate::json_schema::content_encoding::Encoding,
) -> &'static str {
    use crate::json_schema::content_encoding::Encoding;
    match encoding {
        Encoding::Base64 => "_format_base64",
        Encoding::Base64Url => "_format_base64url",
    }
}

/// Emits one compiled regex per distinct `pattern` / `format` source across the
/// module's schemas, so a check reads a shared pre-compiled object rather than
/// recompiling per call. `re.ASCII` pins the character classes the loader
/// normalized to their ASCII meaning.
fn render_pattern_regexes(output: &mut String, models: &[&PlannedJsonType]) -> Result<()> {
    let mut patterns = Vec::new();
    for model in models {
        collect_schema_patterns(&decode_schema(model)?, &mut patterns);
    }
    let mut seen = BTreeSet::new();
    let mut emitted = false;
    for pattern in patterns {
        let name = py_pattern_const_name(&pattern);
        if !seen.insert(name.clone()) {
            continue;
        }
        if !emitted {
            push_section(output);
            emitted = true;
        }
        output.push_str(&format!(
            "{name} = re.compile({}, re.ASCII)\n",
            python_string_literal(&pattern)
        ));
    }
    Ok(())
}

/// Collects every compiled-regex source the module's checks reference, in each
/// string position one can occur: a declared property, an array element at any
/// depth, a typed map's member, a key-shape subschema, and a nullability
/// wrapper's branch. The stored form is the emitted one (`$` already rewritten to
/// `\Z`), so the const name matches what the check emitters compute.
fn collect_schema_patterns(schema: &Schema, patterns: &mut Vec<String>) {
    if let Some(pattern) = &schema.pattern {
        patterns.push(crate::json_schema::pattern::rewrite_end_anchor(
            pattern, r"\Z",
        ));
    }
    if let Some(format) = &schema.format
        && let Some(check) = crate::json_schema::format::check_for(format)
    {
        patterns.push(crate::json_schema::pattern::rewrite_end_anchor(
            &check.pattern,
            r"\Z",
        ));
    }
    for property in schema
        .properties
        .iter()
        .flat_map(|entries| entries.values())
    {
        collect_schema_patterns(property, patterns);
    }
    if let Some(items) = &schema.items {
        collect_schema_patterns(items, patterns);
    }
    if let Some(contains) = &schema.contains {
        collect_schema_patterns(contains, patterns);
    }
    for branch in schema.one_of.iter().flatten() {
        collect_schema_patterns(branch, patterns);
    }
    if let Some(names) = &schema.property_names {
        collect_schema_patterns(names, patterns);
    }
    if let Some(Value::Object(members)) = &schema.additional_properties
        && let Ok(member) = serde_json::from_value::<Schema>(Value::Object(members.clone()))
    {
        collect_schema_patterns(&member, patterns);
    }
}

/// Emits the module-level declared-key set an open object splits its catch-all
/// on, mirroring TypeScript's `<MODEL>_DECLARED`.
fn render_declared_field_set(output: &mut String, model: &PlannedJsonType, schema: &Schema) {
    let fields = schema
        .properties
        .as_ref()
        .map(|properties| {
            properties
                .keys()
                .map(|field| python_string_literal(field))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    output.push_str(&declared_fields_const_name(&model.model_name));
    output.push_str(": frozenset[str] = ");
    if fields.is_empty() {
        output.push_str("frozenset()\n");
    } else {
        output.push_str("frozenset({");
        output.push_str(&fields.join(", "));
        output.push_str("})\n");
    }
}

// ---------------------------------------------------------------------------
// Object shape classification
// ---------------------------------------------------------------------------

fn required_fields(schema: &Schema) -> BTreeSet<String> {
    schema.required.iter().flatten().cloned().collect()
}

/// True when a declared-property object stays open to unknown members, which is
/// what gives it an explicit `additional_properties` catch-all.
fn is_open_object(schema: &Schema) -> bool {
    schema.ty.as_ref().and_then(Value::as_str) == Some("object")
        && schema
            .properties
            .as_ref()
            .is_some_and(|properties| !properties.is_empty())
        && schema.additional_properties.as_ref() != Some(&Value::Bool(false))
}

/// The annotation of an open object's catch-all member.
fn additional_properties_annotation(schema: &Schema) -> Result<String> {
    match &schema.additional_properties {
        Some(Value::Object(members)) => {
            let member: Schema =
                serde_json::from_value(Value::Object(members.clone())).map_err(|error| {
                    Error::InvalidJsonSchema {
                        path: PathBuf::from("<json-generator>"),
                        reason: format!("failed to read `additionalProperties`: {error}"),
                    }
                })?;
            annotation(&member)
        }
        _ => Ok("typing.Any".to_string()),
    }
}

/// A map-shaped model — no declared `properties`, members governed by
/// `additionalProperties` — emitted as a dataclass whose only member is the
/// catch-all (specs/json-schema/features/additionalProperties.md).
#[derive(Debug, Clone)]
struct PyMapShape {
    /// The declared member schema; `None` for free-form members, which are
    /// carried verbatim as `typing.Any`.
    value_schema: Option<Schema>,
    value_annotation: String,
}

fn py_map_shape(schema: &Schema) -> Result<Option<PyMapShape>> {
    if !is_python_map_model(schema) {
        return Ok(None);
    }
    if let Some(value_schema) = typed_map_value_schema(schema)? {
        let value_annotation = annotation(&value_schema)?;
        return Ok(Some(PyMapShape {
            value_schema: Some(value_schema),
            value_annotation,
        }));
    }
    Ok(Some(PyMapShape {
        value_schema: None,
        value_annotation: "typing.Any".to_string(),
    }))
}

/// The annotation of a model's catch-all member, whichever open shape it is.
fn catch_all_annotation(schema: &Schema) -> Result<String> {
    match py_map_shape(schema)? {
        Some(shape) => Ok(shape.value_annotation),
        None => additional_properties_annotation(schema),
    }
}

// ---------------------------------------------------------------------------
// `oneOf` closed sum types (specs/json-schema/features/oneOf.md)
// ---------------------------------------------------------------------------

/// The ±(2^53−1) spec-integer cap, inlined into the union's integer selector so
/// the branch is chosen by exactly the predicate a declared integer field is
/// parsed with.
const PY_INTEGER_CAP: &str = "9007199254740991";

/// The JSON token a union branch is selected by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PyToken {
    Object,
    Array,
    String,
    Integer,
    Number,
    Boolean,
}

impl PyToken {
    /// The narrowing guard over a raw wire value. `expr` is always a bare or
    /// dotted name, so repeating it is free of side effects.
    fn wire_guard(self, expr: &str) -> String {
        match self {
            Self::Object => format!("isinstance({expr}, dict)"),
            Self::Array => format!("isinstance({expr}, list)"),
            Self::String => format!("isinstance({expr}, str)"),
            Self::Boolean => format!("isinstance({expr}, bool)"),
            // `bool` is an `int` subclass, so it is excluded before the numeric
            // test or `True` would select a numeric branch.
            Self::Integer => format!(
                "not isinstance({expr}, bool) and isinstance({expr}, (int, float)) and abs({expr}) <= {PY_INTEGER_CAP} and float({expr}).is_integer()"
            ),
            Self::Number => {
                format!("not isinstance({expr}, bool) and isinstance({expr}, (int, float))")
            }
        }
    }

    /// The narrowing guard over the in-memory value, which selects the same
    /// branch on the way out. An integer member is an `int` by then, so the wire
    /// form's `1.0`-accepting predicate would be wrong here.
    fn memory_guard(self, expr: &str) -> String {
        match self {
            Self::Integer => format!("not isinstance({expr}, bool) and isinstance({expr}, int)"),
            other => other.wire_guard(expr),
        }
    }
}

/// A member of a Python union (a native `A | B | …` annotation).
#[derive(Debug, Clone)]
struct PyUnionVariant {
    /// The member's own annotation, so a `const`/`enum` branch narrows to the
    /// closed literal set it declares rather than the wider primitive.
    py_type: String,
    is_object: bool,
    /// The referenced model's converter expression, for a `$ref` object branch.
    converter: Option<String>,
    /// The referenced union's free-function base, for a `$ref` at a named union.
    union_base: Option<String>,
    /// The runtime type an in-memory member is recognized by, when the token
    /// alone cannot say so (a model class, a materialized temporal, `bytes`).
    memory_type: Option<String>,
    /// The runtime helpers a materialized branch converts through.
    parse_fn: Option<String>,
    serialize_fn: Option<String>,
    discriminant_value: Option<Value>,
    token: PyToken,
    /// True when the branch's declared type is narrower than the JSON token it is
    /// selected by (a `const`/`enum` literal set, a typed array), so the selected
    /// value has to be cast to it.
    narrowed: bool,
    /// The name this branch is reported under in `expected one of: …`.
    label: String,
    /// The branch's own schema, whose constraints the narrowed value is held to.
    schema: Schema,
}

impl PyUnionVariant {
    fn memory_guard(&self, expr: &str) -> String {
        match &self.memory_type {
            Some(ty) => format!("isinstance({expr}, {ty})"),
            None => self.token.memory_guard(expr),
        }
    }

    fn serialize_expr(&self, value_expr: &str) -> String {
        if let Some(converter) = &self.converter {
            return format!("{converter}.to_transfer_type({value_expr})");
        }
        if let Some(base) = &self.union_base {
            return format!("{}({value_expr})", union_serialize_fn(base));
        }
        if let Some(function) = &self.serialize_fn {
            return format!("{function}({value_expr})");
        }
        value_expr.to_string()
    }

    fn needs_transform(&self) -> bool {
        self.converter.is_some() || self.union_base.is_some() || self.serialize_fn.is_some()
    }
}

#[derive(Debug, Clone)]
struct PyUnion {
    nullable: bool,
    discriminant: Option<String>,
    variants: Vec<PyUnionVariant>,
}

impl PyUnion {
    /// The admissible branch names, as `expected one of: …` reports them.
    fn admissible(&self) -> String {
        self.variants
            .iter()
            .map(|variant| variant.label.clone())
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn needs_serializer(&self) -> bool {
        self.variants
            .iter()
            .any(|variant| variant.needs_transform())
    }
}

/// True when a schema is a `oneOf` sum type (two or more non-null branches),
/// rather than the degenerate nullability pattern.
fn is_py_union(schema: &Schema) -> bool {
    schema.one_of.as_ref().is_some_and(|branches| {
        branches
            .iter()
            .filter(|branch| !schema_type_includes(branch, "null"))
            .count()
            >= 2
    })
}

fn py_discriminator_const(property: &Schema) -> Option<Value> {
    if let Some(value) = &property.const_value {
        return Some(value.clone());
    }
    if let Some(values) = &property.enum_values
        && values.len() == 1
    {
        return Some(values[0].clone());
    }
    None
}

fn py_branch_discriminator_tags(object: &Schema) -> BTreeMap<String, Value> {
    let required = required_fields(object);
    let mut tags = BTreeMap::new();
    if let Some(properties) = &object.properties {
        for (name, property) in properties {
            if required.contains(name)
                && let Some(value) = py_discriminator_const(property)
            {
                tags.insert(name.clone(), value);
            }
        }
    }
    tags
}

fn find_ref_model<'a>(
    reference: &str,
    models: &'a [&PlannedJsonType],
) -> Option<&'a PlannedJsonType> {
    let target = reference_model_name(reference);
    models
        .iter()
        .copied()
        .find(|model| model.model_name == target || model.full_name == target)
}

/// Classifies a `oneOf` schema into a Python union, or `None` for the degenerate
/// nullability pattern.
fn classify_py_union(schema: &Schema, models: &[&PlannedJsonType]) -> Result<Option<PyUnion>> {
    if !is_py_union(schema) {
        return Ok(None);
    }
    let Some(branches) = schema.one_of.as_ref() else {
        return Ok(None);
    };
    let mut nullable = false;
    let mut variants: Vec<PyUnionVariant> = Vec::new();
    let mut object_schemas: Vec<Schema> = Vec::new();
    for branch in branches {
        let resolved = if let Some(reference) = &branch.reference {
            find_ref_model(reference, models)
                .and_then(|model| decode_schema(model).ok())
                .unwrap_or_else(|| branch.clone())
        } else {
            branch.clone()
        };
        let scalar = |token: PyToken, primitive: &str, label: &str| -> PyUnionVariant {
            let py_type = annotation(&resolved).unwrap_or_else(|_| primitive.to_string());
            let py_type_is_narrowed = py_type != primitive;
            let (memory_type, parse_fn, serialize_fn) =
                if let Some(kind) = temporal_kind_direct(&resolved) {
                    (
                        Some(python_temporal_type(kind).to_string()),
                        Some(python_temporal_parse_fn(kind).to_string()),
                        Some(python_temporal_format_fn(kind).to_string()),
                    )
                } else if let Some(encoding) = content_encoding_direct(&resolved) {
                    (
                        Some("bytes".to_string()),
                        Some(python_content_encoding_parse_fn(encoding).to_string()),
                        Some(python_content_encoding_format_fn(encoding).to_string()),
                    )
                } else {
                    (None, None, None)
                };
            PyUnionVariant {
                py_type,
                is_object: false,
                converter: None,
                union_base: None,
                memory_type,
                parse_fn,
                serialize_fn,
                discriminant_value: None,
                token,
                narrowed: py_type_is_narrowed,
                label: label.to_string(),
                schema: resolved.clone(),
            }
        };
        match resolved.ty.as_ref().and_then(Value::as_str) {
            Some("null") => nullable = true,
            Some("object") => {
                // A `$ref` branch is the named model (converted by its own
                // converter); an inline branch is the free-form object
                // (loader-enforced), carried structurally as a mapping — Python
                // needs no synthesized name to narrow on the object token.
                let (py_type, converter, union_base, memory_type, label) = match &branch.reference {
                    Some(reference) => {
                        let name = reference_model_name(reference);
                        if is_union_type_name(&name) {
                            (
                                name.clone(),
                                None,
                                Some(union_fn_base(&name)),
                                Some(name.clone()),
                                name,
                            )
                        } else {
                            (
                                name.clone(),
                                Some(converter_expr(&name)),
                                None,
                                Some(name.clone()),
                                name,
                            )
                        }
                    }
                    None => (
                        object_annotation(&resolved)?,
                        None,
                        None,
                        None,
                        "object".to_string(),
                    ),
                };
                object_schemas.push(resolved.clone());
                variants.push(PyUnionVariant {
                    py_type,
                    is_object: true,
                    converter,
                    union_base,
                    memory_type,
                    parse_fn: None,
                    serialize_fn: None,
                    discriminant_value: None,
                    token: PyToken::Object,
                    narrowed: true,
                    label,
                    schema: resolved.clone(),
                });
            }
            Some("string") => variants.push(scalar(PyToken::String, "str", "string")),
            Some("integer") => variants.push(scalar(PyToken::Integer, "int", "integer")),
            Some("number") => variants.push(scalar(PyToken::Number, "float", "number")),
            Some("boolean") => variants.push(scalar(PyToken::Boolean, "bool", "boolean")),
            Some("array") => {
                let py_type =
                    annotation(&resolved).unwrap_or_else(|_| "list[typing.Any]".to_string());
                variants.push(PyUnionVariant {
                    py_type: py_type.clone(),
                    is_object: false,
                    converter: None,
                    union_base: None,
                    memory_type: None,
                    parse_fn: None,
                    serialize_fn: None,
                    discriminant_value: None,
                    token: PyToken::Array,
                    narrowed: true,
                    // An array branch has no definition to take a name from, so
                    // it reports under Python's own type spelling.
                    label: py_type,
                    schema: resolved.clone(),
                });
            }
            _ => {}
        }
    }

    let mut discriminant = None;
    if object_schemas.len() >= 2 {
        let mut shared: Option<BTreeMap<String, Value>> = None;
        for object in &object_schemas {
            let tags = py_branch_discriminator_tags(object);
            shared = Some(match shared {
                None => tags,
                Some(existing) => existing
                    .into_iter()
                    .filter(|(name, _)| tags.contains_key(name))
                    .collect(),
            });
        }
        let shared = shared.unwrap_or_default();
        let name = shared
            .keys()
            .find(|name| {
                let values: Vec<Value> = object_schemas
                    .iter()
                    .filter_map(|object| py_branch_discriminator_tags(object).get(*name).cloned())
                    .collect();
                values
                    .iter()
                    .enumerate()
                    .all(|(index, value)| !values[..index].iter().any(|existing| existing == value))
            })
            .cloned();
        if let Some(name) = &name {
            let mut object_index = 0;
            for variant in variants.iter_mut().filter(|variant| variant.is_object) {
                variant.discriminant_value =
                    py_branch_discriminator_tags(&object_schemas[object_index])
                        .get(name)
                        .cloned();
                object_index += 1;
            }
        }
        discriminant = name;
    }

    Ok(Some(PyUnion {
        nullable,
        discriminant,
        variants,
    }))
}

/// Emits the body of a union's `_<base>_from_transfer_type`: token /
/// discriminant selection, returning the selected member or `None` after
/// recording why nothing matched.
fn render_py_union_parse(
    output: &mut String,
    union: &PyUnion,
    value_expr: &str,
    path_expr: &str,
    indent: &str,
) -> Result<()> {
    let object_variants: Vec<&PyUnionVariant> = union
        .variants
        .iter()
        .filter(|variant| variant.is_object)
        .collect();
    if !object_variants.is_empty() {
        output.push_str(indent);
        output.push_str(&format!("if isinstance({value_expr}, dict):\n"));
        let inner = format!("{indent}    ");
        if let Some(discriminant) = &union.discriminant {
            output.push_str(&inner);
            output.push_str(&format!(
                "tagged = typing.cast(\"dict[str, typing.Any]\", {value_expr})\n"
            ));
            output.push_str(&inner);
            output.push_str(&format!(
                "tag = tagged.get({})\n",
                python_string_literal(discriminant)
            ));
            let mut values_display = Vec::new();
            for variant in &object_variants {
                let Some(value) = &variant.discriminant_value else {
                    continue;
                };
                values_display.push(py_reason_literal(value));
                output.push_str(&inner);
                output.push_str(&format!("if tag == {}:\n", python_value_literal(value)?));
                render_py_union_object_branch(
                    output,
                    variant,
                    value_expr,
                    path_expr,
                    &format!("{inner}    "),
                );
            }
            output.push_str(&inner);
            output.push_str(&format!(
                "violations.append(Violation(path={path_expr}, reason=f'unknown discriminator {} {{tag}}: expected one of [{}]'))\n",
                py_fstring_text(discriminant),
                py_fstring_text(&values_display.join(", "))
            ));
            output.push_str(&inner);
            output.push_str("return None\n");
        } else {
            let variant = object_variants[0];
            render_py_union_object_branch(output, variant, value_expr, path_expr, &inner);
        }
    }

    for variant in union.variants.iter().filter(|variant| !variant.is_object) {
        output.push_str(indent);
        output.push_str(&format!("if {}:\n", variant.token.wire_guard(value_expr)));
        let inner = format!("{indent}    ");
        // The token has selected the branch; the value is now held to everything
        // the branch declares (P12 — the same predicates a property of that type
        // runs). A branch whose declared type is narrower than its token is cast
        // to it once, and the checks run over the narrowed name. A branch is a
        // scalar or array shape, never a union of its own, so its checks need no
        // model list to resolve branch `$ref`s with.
        let selected = match (&variant.parse_fn, variant.token) {
            // A materialized branch parses through its runtime helper; the token
            // guard has already established the wire is a string.
            (Some(parse_fn), _) => {
                output.push_str(&inner);
                output.push_str(&format!(
                    "parsed = {parse_fn}({value_expr}, {path_expr}, violations)\n"
                ));
                "parsed".to_string()
            }
            // The token accepted `1.0` as an integer, so the member is normalized
            // before its own bounds are checked.
            (None, PyToken::Integer) => {
                output.push_str(&inner);
                output.push_str(&format!("number = int({value_expr})\n"));
                render_py_field_checks(output, &variant.schema, &[], "number", path_expr, &inner)?;
                if variant.narrowed {
                    output.push_str(&inner);
                    output.push_str(&format!(
                        "narrowed = typing.cast({}, number)\n",
                        python_string_literal(&variant.py_type)
                    ));
                    "narrowed".to_string()
                } else {
                    "number".to_string()
                }
            }
            // An array branch is decoded elementwise, exactly as a declared array
            // member is: the `list` token selects the branch, but it says nothing
            // about what the elements are, and `list[float]` admits only numbers
            // (P1 — Go and Java decode into a typed list and reject a bad
            // element). The array-level predicates then run over the built list.
            (None, PyToken::Array) => render_py_array_elements(
                output,
                &variant.schema,
                value_expr,
                path_expr,
                &inner,
                "items",
            )?,
            _ if variant.narrowed => {
                output.push_str(&inner);
                output.push_str(&format!(
                    "narrowed = typing.cast({}, {value_expr})\n",
                    python_string_literal(&variant.py_type)
                ));
                render_py_field_checks(
                    output,
                    &variant.schema,
                    &[],
                    "narrowed",
                    path_expr,
                    &inner,
                )?;
                "narrowed".to_string()
            }
            _ => {
                render_py_field_checks(
                    output,
                    &variant.schema,
                    &[],
                    value_expr,
                    path_expr,
                    &inner,
                )?;
                value_expr.to_string()
            }
        };
        output.push_str(&inner);
        output.push_str(&format!("return {selected}\n"));
    }

    if union.nullable {
        output.push_str(indent);
        output.push_str(&format!("if {value_expr} is None:\n"));
        output.push_str(indent);
        output.push_str("    return None\n");
    }

    output.push_str(indent);
    output.push_str(&format!(
        "violations.append(Violation(path={path_expr}, reason={}))\n",
        python_string_literal(&format!("expected one of: {}", union.admissible()))
    ));
    output.push_str(indent);
    output.push_str("return None\n");
    Ok(())
}

/// Emits the object-branch arm of a union's parse: the member converts through
/// its own converter (or the nested union's dispatcher), with its violations
/// re-pathed under the union's path.
fn render_py_union_object_branch(
    output: &mut String,
    variant: &PyUnionVariant,
    value_expr: &str,
    path_expr: &str,
    indent: &str,
) {
    if let Some(base) = &variant.union_base {
        output.push_str(indent);
        output.push_str(&format!(
            "return {}({value_expr}, {path_expr}, violations)\n",
            union_parse_fn(base)
        ));
        return;
    }
    let Some(converter) = &variant.converter else {
        // A free-form object branch: the wire object is already the member.
        output.push_str(indent);
        output.push_str(&format!(
            "return typing.cast({}, {value_expr})\n",
            python_string_literal(&variant.py_type)
        ));
        return;
    };
    output.push_str(indent);
    output.push_str("try:\n");
    output.push_str(indent);
    output.push_str(&format!(
        "    return {converter}.from_transfer_type({value_expr}, {})\n",
        variant.py_type
    ));
    output.push_str(indent);
    output.push_str("except ValidationError as error:\n");
    output.push_str(indent);
    output.push_str(&format!("    _collect(violations, {path_expr}, error)\n"));
    output.push_str(indent);
    output.push_str("    return None\n");
}

/// The local a union's in-memory value is widened through before the
/// no-branch-matched test. See [`render_py_union_value_checks`].
const PY_UNION_CANDIDATE: &str = "candidate";

/// Emits the constraint checks a union's **in-memory** value is held to, narrowed
/// to the branch it holds: one guarded block per non-object branch that declares
/// anything (P12), then the terminal test that *some* branch matched at all.
/// Object branches carry their own validation in their model's converter, so they
/// contribute no constraint block — only their arm of that terminal test.
fn render_py_union_value_checks(
    output: &mut String,
    union: &PyUnion,
    models: &[&PlannedJsonType],
    value_expr: &str,
    path_expr: &str,
    indent: &str,
) -> Result<()> {
    for variant in union.variants.iter().filter(|variant| !variant.is_object) {
        let mut body = String::new();
        render_py_field_checks(
            &mut body,
            &variant.schema,
            models,
            value_expr,
            path_expr,
            &format!("{indent}    "),
        )?;
        if body.is_empty() {
            continue;
        }
        output.push_str(indent);
        output.push_str(&format!("if {}:\n", variant.memory_guard(value_expr)));
        output.push_str(&body);
    }

    // Nothing enforces a Python annotation at runtime, so a member holding a value
    // admitted by *no* branch is a real state — and one the per-branch blocks above
    // say nothing about, since each is guarded by its own kind test. Left
    // unreported it would serialize verbatim, emitting bytes every parser
    // (Python's own included) rejects, so it is the same aggregated violation the
    // parse side reports for an inadmissible wire token (P12: both directions run
    // the same checks). The value is widened to `object` first: read through the
    // declared union a closed set of guards can be provably exhaustive, which puts
    // the violation in code pyright reports as unreachable — and the widening
    // costs nothing, because the guards are the runtime tests either way. This
    // test is also what makes the dispatch's unguarded last branch safe, so a
    // union that has a serializer runs it inside that function rather than at the
    // enclosing member (see `render_union_serialize_function`).
    let mut guards: Vec<String> = union
        .variants
        .iter()
        .map(|variant| py_negatable(&variant.memory_guard(PY_UNION_CANDIDATE)))
        .collect();
    if union.nullable {
        guards.push(format!("{PY_UNION_CANDIDATE} is None"));
    }
    if guards.is_empty() {
        return Ok(());
    }
    output.push_str(indent);
    output.push_str(&format!(
        "{PY_UNION_CANDIDATE} = typing.cast(\"object\", {value_expr})\n"
    ));
    output.push_str(indent);
    output.push_str(&format!("if not ({}):\n", guards.join(" or ")));
    output.push_str(indent);
    output.push_str(&format!(
        "    violations.append(Violation(path={path_expr}, reason={}))\n",
        python_string_literal(&format!("expected one of: {}", union.admissible()))
    ));
    Ok(())
}

/// Emits the dispatch of a union's `_<base>_to_transfer_type`. Unlike the parse
/// side, which is handed an untyped wire value, this direction runs only after
/// the checks above it have established that the value matches *some* branch, so
/// each guard narrows a set already known to be inhabited and the final branch is
/// whatever is left over. Guarding that one as well would be provably redundant —
/// and would put an unreachable `expected one of` raise behind it — so it is
/// emitted as the fallthrough instead. When no branch transforms its value at all
/// the dispatch collapses to returning it unchanged.
fn render_py_union_serialize(output: &mut String, union: &PyUnion, value_expr: &str, indent: &str) {
    if union.nullable {
        output.push_str(indent);
        output.push_str(&format!("if {value_expr} is None:\n"));
        output.push_str(indent);
        output.push_str("    return None\n");
    }
    let Some((last, leading)) = union.variants.split_last() else {
        output.push_str(indent);
        output.push_str(&format!("return {value_expr}\n"));
        return;
    };
    if union.needs_serializer() {
        for variant in leading {
            output.push_str(indent);
            output.push_str(&format!("if {}:\n", variant.memory_guard(value_expr)));
            output.push_str(indent);
            output.push_str(&format!(
                "    return {}\n",
                variant.serialize_expr(value_expr)
            ));
        }
    }
    output.push_str(indent);
    output.push_str(&format!("return {}\n", last.serialize_expr(value_expr)));
}

/// Emits the module-private free functions a union's conversion lives in. A
/// `typing.TypeAlias` cannot be decorated and `type[A | B]` is not a valid
/// annotation, so a union is never registered with the SDK; it is only ever
/// reached from an enclosing model's converter.
fn render_union_transfer_functions(output: &mut String, models: &[&PlannedJsonType]) -> Result<()> {
    for model in models {
        let schema = decode_schema(model)?;
        if !is_python_union_model(model) {
            continue;
        }
        let Some(union) = classify_py_union(&schema, models)? else {
            continue;
        };
        let base = union_fn_base(&model.model_name);
        render_union_parse_function(output, &base, &model.model_name, &union)?;
        render_union_serialize_function(output, &base, &model.model_name, &union, models)?;
    }
    for model in models {
        let schema = decode_schema(model)?;
        let Some(properties) = &schema.properties else {
            continue;
        };
        for (json_name, property) in properties {
            let Some(union) = classify_py_union(property, models)? else {
                continue;
            };
            let base = inline_union_fn_base(&model.model_name, &property.py_member_name(json_name));
            let member_type = annotation(property)?;
            render_union_parse_function(output, &base, &member_type, &union)?;
            // Only needed when some branch's in-memory form differs from its wire
            // form; otherwise the member is emitted as-is and the enclosing
            // property runs its checks inline.
            if union.needs_serializer() {
                render_union_serialize_function(output, &base, &member_type, &union, models)?;
            }
        }
    }
    Ok(())
}

fn render_union_parse_function(
    output: &mut String,
    base: &str,
    member_type: &str,
    union: &PyUnion,
) -> Result<()> {
    push_section(output);
    output.push_str(&format!(
        "def {}(\n    value: typing.Any, path: str, violations: list[Violation]\n) -> {}:\n",
        union_parse_fn(base),
        optional_annotation(member_type)
    ));
    render_py_union_parse(output, union, "value", "path", "    ")
}

/// Emits a union's `_<base>_to_transfer_type`: the value's checks, the raise that
/// aggregates them, then the dispatch. The order is the point — the dispatch's
/// last branch is unguarded, so a value matching no branch has to fail here, as
/// the union's own aggregated `ValidationError`, rather than reach a converter
/// that would raise whatever its first attribute access raises. Callers report the
/// checks' violations under the member's path through `_collect` (P11), which is
/// what the checks' empty `path` is for.
fn render_union_serialize_function(
    output: &mut String,
    base: &str,
    member_type: &str,
    union: &PyUnion,
    models: &[&PlannedJsonType],
) -> Result<()> {
    push_section(output);
    output.push_str(&format!(
        "def {}(value: {member_type}) -> typing.Any:\n",
        union_serialize_fn(base)
    ));
    let mut checks = String::new();
    render_py_union_value_checks(&mut checks, union, models, "value", "\"\"", "    ")?;
    if !checks.is_empty() {
        output.push_str("    violations: list[Violation] = []\n");
        output.push_str(&checks);
        output.push_str("    if violations:\n");
        output.push_str("        raise ValidationError(violations)\n");
    }
    render_py_union_serialize(output, union, "value", "    ");
    Ok(())
}

// ---------------------------------------------------------------------------
// Model + converter emission
// ---------------------------------------------------------------------------

/// Emits the dataclass: a plain data carrier with no conversion of its own,
/// registered with the SDK so the **default** data converter finds its
/// converter. Decorator order is fixed — `transfer_type_convertible` above
/// `dataclass`, because `slots=True` returns a new class object and the attribute
/// must land on the final one.
fn render_model_dataclass(
    output: &mut String,
    model: &PlannedJsonType,
    schema: &Schema,
) -> Result<()> {
    // Native deprecation marker (PEP 702) on the type; `category=None` is the
    // no-runtime-warning form. See specs/json-schema/features/deprecated.md.
    if schema.deprecated == Some(true) {
        output.push_str(
            "@typing_extensions.deprecated(\"This type is deprecated.\", category=None)\n",
        );
    }
    output.push_str(&format!(
        "@_transfer_type_convertible({})\n",
        converter_class_name(&model.model_name)
    ));
    let has_defaults = schema.properties.as_ref().is_some_and(|properties| {
        properties
            .values()
            .any(|property| property.default.is_some())
    });
    // Every member is keyword-only: JSON Schema interleaves required and
    // optional properties freely, so a positional order is never safe. Models
    // with schema defaults need a handwritten initializer so the public keyword
    // can initialize its private presence-bearing slot.
    if has_defaults {
        output.push_str("@dataclasses.dataclass(slots=True, kw_only=True, init=False)\n");
    } else {
        output.push_str("@dataclasses.dataclass(slots=True, kw_only=True)\n");
    }
    output.push_str("class ");
    output.push_str(&model.model_name);
    output.push_str(":\n");
    render_python_docstring(
        output,
        "    ",
        compose_python_doc(schema.title.as_deref(), schema.description.as_deref()).as_deref(),
        &[],
        None,
        false,
    );

    let required = required_fields(schema);
    let mut members = 0usize;
    if let Some(properties) = &schema.properties {
        for (json_name, property) in properties {
            output.push('\n');
            members += 1;
            let field_name = property.py_member_name(json_name);
            let mut member_type = annotation(property)?;
            // A PEP 702 decorator cannot apply to a field, so a deprecated
            // member carries the marker inside its annotation instead.
            if property.deprecated == Some(true) {
                member_type = format!(
                    "typing.Annotated[{member_type}, typing_extensions.deprecated(\"This field is deprecated.\", category=None)]"
                );
            }
            let storage_name = if property.default.is_some() {
                format!("_{field_name}")
            } else {
                field_name.clone()
            };
            output.push_str("    ");
            output.push_str(&storage_name);
            output.push_str(": ");
            if property.default.is_some() {
                output.push_str(&optional_annotation(&member_type));
                output.push_str(" = dataclasses.field(default=None, repr=False)");
            } else if let Some(const_value) = &property.const_value {
                // The only admissible value, so it is the field's default — a
                // schema `default`, being a suggestion, is not.
                if required.contains(json_name) {
                    output.push_str(&member_type);
                } else {
                    output.push_str(&optional_annotation(&member_type));
                }
                output.push_str(" = ");
                output.push_str(&python_typed_value_literal(property, const_value)?);
            } else if required.contains(json_name) {
                // Required and nullable keeps the `| None` (an explicit null is
                // the value) but takes no default: the member must be supplied.
                if allows_null(property) {
                    output.push_str(&optional_annotation(&member_type));
                } else {
                    output.push_str(&member_type);
                }
            } else {
                output.push_str(&optional_annotation(&member_type));
                output.push_str(" = None");
            }
            output.push('\n');
            render_python_docstring(
                output,
                "    ",
                compose_python_doc(property.title.as_deref(), property.description.as_deref())
                    .as_deref(),
                &[],
                None,
                false,
            );
        }
    }

    if is_python_map_model(schema) || is_open_object(schema) {
        output.push('\n');
        members += 1;
        output.push_str(&format!(
            "    additional_properties: dict[str, {}] = dataclasses.field(default_factory=dict)\n",
            catch_all_annotation(schema)?
        ));
    }
    if has_defaults {
        render_model_init(output, schema)?;
        render_default_properties(output, schema)?;
    }
    if members == 0 {
        output.push_str("\n    pass\n");
    }
    Ok(())
}

/// Emits the public keyword-only constructor for a dataclass whose defaulted
/// properties are backed by private presence-bearing fields.
fn render_model_init(output: &mut String, schema: &Schema) -> Result<()> {
    let required = required_fields(schema);
    output.push_str("\n    def __init__(\n        self,\n        *,\n");
    if let Some(properties) = &schema.properties {
        for (json_name, property) in properties {
            let field_name = property.py_member_name(json_name);
            let mut member_type = annotation(property)?;
            if property.deprecated == Some(true) {
                member_type = format!(
                    "typing.Annotated[{member_type}, typing_extensions.deprecated(\"This field is deprecated.\", category=None)]"
                );
            }
            output.push_str("        ");
            output.push_str(&field_name);
            output.push_str(": ");
            if property.default.is_some() || !required.contains(json_name) || allows_null(property)
            {
                output.push_str(&optional_annotation(&member_type));
            } else {
                output.push_str(&member_type);
            }
            if let Some(const_value) = &property.const_value {
                output.push_str(" = ");
                output.push_str(&python_typed_value_literal(property, const_value)?);
            } else if !required.contains(json_name) {
                output.push_str(" = None");
            }
            output.push_str(",\n");
        }
    }
    if is_python_map_model(schema) || is_open_object(schema) {
        output.push_str(&format!(
            "        additional_properties: dict[str, {}] | None = None,\n",
            catch_all_annotation(schema)?
        ));
    }
    // `dataclasses.replace()` reconstructs from declared dataclass field names,
    // which are the private backing names here. Accept those private keywords so
    // replacing an unrelated field preserves raw default presence/value state.
    if let Some(properties) = &schema.properties {
        for (json_name, property) in properties {
            if property.default.is_none() {
                continue;
            }
            let field_name = property.py_member_name(json_name);
            let mut member_type = annotation(property)?;
            if property.deprecated == Some(true) {
                member_type = format!(
                    "typing.Annotated[{member_type}, typing_extensions.deprecated(\"This field is deprecated.\", category=None)]"
                );
            }
            output.push_str(&format!(
                "        _{field_name}: {} = None,\n",
                optional_annotation(&member_type),
            ));
        }
    }
    output.push_str("    ) -> None:\n");
    let mut assignments = 0usize;
    if let Some(properties) = &schema.properties {
        for (json_name, property) in properties {
            assignments += 1;
            let field_name = property.py_member_name(json_name);
            let target = if property.default.is_some() {
                format!("_{field_name}")
            } else {
                field_name.clone()
            };
            if property.default.is_some() {
                output.push_str(&format!(
                    "        self.{target} = _{field_name} if _{field_name} is not None else {field_name}\n"
                ));
            } else {
                output.push_str(&format!("        self.{target} = {field_name}\n"));
            }
        }
    }
    if is_python_map_model(schema) || is_open_object(schema) {
        assignments += 1;
        output.push_str(
            "        self.additional_properties = (\n            {} if additional_properties is None else additional_properties\n        )\n",
        );
    }
    if assignments == 0 {
        output.push_str("        pass\n");
    }
    Ok(())
}

fn render_default_properties(output: &mut String, schema: &Schema) -> Result<()> {
    let Some(properties) = &schema.properties else {
        return Ok(());
    };
    for (json_name, property) in properties {
        let Some(default) = &property.default else {
            continue;
        };
        let field_name = property.py_member_name(json_name);
        let mut member_type = annotation(nullable_member_schema(property).unwrap_or(property))?;
        if property.deprecated == Some(true) {
            member_type = format!(
                "typing.Annotated[{member_type}, typing_extensions.deprecated(\"This field is deprecated.\", category=None)]"
            );
        }
        output.push_str(&format!(
            "\n    @property\n    def {field_name}(self) -> {member_type}:\n"
        ));
        render_python_docstring(
            output,
            "        ",
            compose_python_doc(property.title.as_deref(), property.description.as_deref())
                .as_deref(),
            &[],
            None,
            false,
        );
        output.push_str(&format!(
            "        return self._{field_name} if self._{field_name} is not None else {}\n",
            python_typed_value_literal(property, default)?
        ));
        output.push_str(&format!(
            "\n    @{field_name}.setter\n    def {field_name}(self, value: {member_type}) -> None:\n        self._{field_name} = value\n"
        ));
        output.push_str(&format!(
            "\n    @{field_name}.deleter\n    def {field_name}(self) -> None:\n        self._{field_name} = None\n"
        ));
    }
    Ok(())
}

/// Emits the private `TransferTypeConverter` a model's whole wire contract lives
/// in. `transfer_type` is left at its inherited `None`, which is what makes the
/// inner payload converter hand us the raw `json.loads` value.
fn render_model_converter(
    output: &mut String,
    model: &PlannedJsonType,
    schema: &Schema,
    models: &[&PlannedJsonType],
) -> Result<()> {
    let name = &model.model_name;
    output.push_str(&format!(
        "class {}(\n    temporalio.converter.TransferTypeConverter[\"{name}\", typing.Any]\n):\n",
        converter_class_name(name)
    ));
    output.push_str("    @typing_extensions.override\n");
    output.push_str(&format!(
        "    def from_transfer_type(\n        self, value: typing.Any, type_hint: type[\"{name}\"]\n    ) -> \"{name}\":\n"
    ));
    let mut parser = String::new();
    render_model_parser_body(&mut parser, model, schema, models)?;
    push_indented(output, &parser, "        ");
    output.push('\n');
    output.push_str("    @typing_extensions.override\n");
    output.push_str(&format!(
        "    def to_transfer_type(self, value: \"{name}\") -> typing.Any:\n"
    ));
    let mut serializer = String::new();
    render_model_serializer_body(&mut serializer, model, schema, models)?;
    push_indented(output, &serializer, "        ");
    Ok(())
}

fn render_model_parser_body(
    output: &mut String,
    model: &PlannedJsonType,
    schema: &Schema,
    models: &[&PlannedJsonType],
) -> Result<()> {
    output.push_str("violations: list[Violation] = []\n");
    // The one non-aggregating failure: without an object there is no member to
    // report a violation against.
    output.push_str("if not isinstance(value, dict):\n");
    output.push_str(
        "    raise ValidationError([Violation(path=\"\", reason=\"expected object\")])\n",
    );
    output.push_str("raw = typing.cast(\"dict[str, typing.Any]\", value)\n");

    if let Some(shape) = py_map_shape(schema)? {
        render_map_parser_body(output, model, schema, &shape)?;
        return Ok(());
    }

    let required = required_fields(schema);
    let mut fields: Vec<String> = Vec::new();
    if let Some(properties) = &schema.properties {
        for (json_name, property) in properties {
            output.push('\n');
            render_property_parser(
                output,
                model,
                models,
                json_name,
                property,
                required.contains(json_name),
            )?;
            fields.push(property.py_member_name(json_name));
        }
    }

    let open = is_open_object(schema);
    output.push('\n');
    if schema.additional_properties.as_ref() == Some(&Value::Bool(false)) {
        render_closed_object_unknown_key_check(output, schema);
    } else if open {
        render_open_object_collection(output, model, schema)?;
    }

    // Object member-count and cross-field constraints over the wire member set
    // (`raw` holds every distinct wire key).
    render_py_property_count_checks(output, "len(raw)", schema, "");
    if let Some(subschema) = &schema.property_names {
        render_py_property_name_checks(output, "raw", subschema, "");
    }
    render_py_dependent_required(output, "raw", schema, "");

    output.push_str("if violations:\n");
    output.push_str("    raise ValidationError(violations)\n");
    if fields.is_empty() && !open {
        output.push_str(&format!("return {}()\n", model.model_name));
        return Ok(());
    }
    output.push_str(&format!("return {}(\n", model.model_name));
    for field_name in &fields {
        // The member identifier names the keyword; the value comes off the
        // property's slot local (see `parse_slot_local`).
        output.push_str(&format!(
            "    {field_name}={},\n",
            parse_slot_local(field_name)
        ));
    }
    if open {
        output.push_str("    additional_properties=additional_properties,\n");
    }
    output.push_str(")\n");
    Ok(())
}

/// Emits the parse body of a map-shaped model: the member-count and key-shape
/// checks over the wire keys, then every member through its declared type into
/// the catch-all.
fn render_map_parser_body(
    output: &mut String,
    model: &PlannedJsonType,
    schema: &Schema,
    shape: &PyMapShape,
) -> Result<()> {
    render_py_property_count_checks(output, "len(raw)", schema, "");
    if let Some(subschema) = &schema.property_names {
        render_py_property_name_checks(output, "raw", subschema, "");
    }
    output.push_str(&format!(
        "additional_properties: dict[str, {}] = {{}}\n",
        shape.value_annotation
    ));
    output.push_str("for key in raw:\n");
    match &shape.value_schema {
        // Free-form members are carried verbatim, `null` included (P13).
        None => output.push_str("    additional_properties[key] = raw[key]\n"),
        Some(value_schema) => {
            render_py_slot_declaration(output, "    ", "member", &shape.value_annotation);
            output.push_str("    member_raw = raw[key]\n");
            render_value_parser(
                output,
                value_schema,
                "member_raw",
                "member",
                "key",
                "    ",
                "member",
            )?;
            // A member that failed to parse has already recorded a violation, so
            // the value stored here never reaches the caller.
            output.push_str("    additional_properties[key] = member\n");
        }
    }
    output.push_str("if violations:\n");
    output.push_str("    raise ValidationError(violations)\n");
    output.push_str(&format!(
        "return {}(additional_properties=additional_properties)\n",
        model.model_name
    ));
    Ok(())
}

fn render_model_serializer_body(
    output: &mut String,
    model: &PlannedJsonType,
    schema: &Schema,
    models: &[&PlannedJsonType],
) -> Result<()> {
    // Serialize-side (P12): re-run the shared field validation over the
    // in-memory model and raise the aggregated `ValidationError` before emitting
    // the wire object — both directions over one set of check emitters. A nested
    // conversion aggregates into the same list, so the violations declared here
    // also hold everything the members below report (P11).
    let needs_violations =
        py_model_needs_serialize_validation(schema)? || py_model_serialize_can_raise(schema)?;
    if needs_violations {
        output.push_str("violations: list[Violation] = []\n");
    }
    output.push_str("out: dict[str, typing.Any] = {}\n");

    if let Some(shape) = py_map_shape(schema)? {
        output.push_str("for key, entry in value.additional_properties.items():\n");
        if let Some(value_schema) = &shape.value_schema {
            render_py_member_check(output, value_schema, models, "entry", "key", "    ")?;
        }
        match &shape.value_schema {
            Some(value_schema) => render_py_serialize_value(
                output,
                value_schema,
                PySerializeSink::Assign("out[key]"),
                "entry",
                "key",
                "    ",
                "entry",
            )?,
            // Free-form members carry no declared shape to convert through.
            None => output.push_str("    out[key] = entry\n"),
        }
        if needs_violations {
            render_py_property_count_checks(output, "len(out)", schema, "");
            if let Some(subschema) = &schema.property_names {
                render_py_property_name_checks(output, "out", subschema, "");
            }
            output.push_str("if violations:\n");
            output.push_str("    raise ValidationError(violations)\n");
        }
        output.push_str("return out\n");
        return Ok(());
    }

    let required = required_fields(schema);
    if let Some(properties) = &schema.properties {
        for (json_name, property) in properties {
            let field_name = property.py_member_name(json_name);
            let value_expr = if property.default.is_some() {
                format!("value._{field_name}")
            } else {
                format!("value.{field_name}")
            };
            let key = python_string_literal(json_name);
            let target = format!("out[{key}]");
            let path_expr = python_string_literal(json_name);
            // An optional member is emitted under an `is not None` guard, so the
            // nullability wrapper's own `None` branch is already ruled out and the
            // transform is taken straight from the member's non-null shape.
            let emitted = match (
                required.contains(json_name),
                nullable_member_schema(property),
            ) {
                (false, Some(non_null)) => non_null,
                _ => property,
            };
            // A union whose members need a transform goes through the module's
            // union serializer, which is the one conversion not derivable from the
            // schema alone (it is named after this property's position).
            let inline_union = match classify_py_union(property, models)? {
                Some(union) if union.needs_serializer() => Some(format!(
                    "{}({value_expr})",
                    union_serialize_fn(&inline_union_fn_base(
                        &model.model_name,
                        &property.py_member_name(json_name)
                    ))
                )),
                _ => None,
            };
            let guarded = !required.contains(json_name);
            let indent = if guarded {
                // Absent and explicit `null` collapsed to `None` on the way in,
                // so both re-serialize as omitted.
                output.push_str(&format!("if {value_expr} is not None:\n"));
                "    "
            } else {
                ""
            };
            render_py_serialize_property_check(
                output, json_name, property, models, guarded, indent,
            )?;
            match inline_union {
                // Every union serializer validates before dispatching, so the
                // call always needs its violations re-pathed under this member.
                Some(call) => render_py_serialize_call(
                    output,
                    PySerializeSink::Assign(&target),
                    &call,
                    &path_expr,
                    indent,
                ),
                None => render_py_serialize_value(
                    output,
                    emitted,
                    PySerializeSink::Assign(&target),
                    &value_expr,
                    &path_expr,
                    indent,
                    &field_name,
                )?,
            }
        }
    }
    if is_open_object(schema) {
        output.push_str("for key, entry in value.additional_properties.items():\n");
        output.push_str(&format!(
            "    if key in {}:\n",
            declared_fields_const_name(&model.model_name)
        ));
        output.push_str(
            "        violations.append(Violation(path=key, reason=\"additional property collides with declared property\"))\n",
        );
        output.push_str("    else:\n");
        match typed_additional_properties_schema(schema)? {
            Some(value_schema) => {
                render_py_member_check(output, &value_schema, models, "entry", "key", "        ")?;
                render_py_serialize_value(
                    output,
                    &value_schema,
                    PySerializeSink::Assign("out[key]"),
                    "entry",
                    "key",
                    "        ",
                    "entry",
                )?;
            }
            None => output.push_str("        out[key] = entry\n"),
        }
    }
    if needs_violations {
        // Object member-count and cross-field constraints over the to-be-emitted
        // wire key set (`out` holds every distinct wire key, JSON-named).
        render_py_property_count_checks(output, "len(out)", schema, "");
        if let Some(subschema) = &schema.property_names {
            render_py_property_name_checks(output, "out", subschema, "");
        }
        render_py_dependent_required(output, "out", schema, "");
        output.push_str("if violations:\n");
        output.push_str("    raise ValidationError(violations)\n");
    }
    output.push_str("return out\n");
    Ok(())
}

/// Where one value's wire form goes: assigned to a target, or appended to the
/// list an enclosing array is building.
#[derive(Debug, Clone, Copy)]
enum PySerializeSink<'a> {
    Assign(&'a str),
    Append(&'a str),
}

impl PySerializeSink<'_> {
    fn statement(self, expr: &str) -> String {
        match self {
            Self::Assign(target) => format!("{target} = {expr}"),
            Self::Append(list) => format!("{list}.append({expr})"),
        }
    }
}

/// True when any of a model's members converts through a call that can raise, so
/// its serializer needs the violation list even with no constraint of its own.
fn py_model_serialize_can_raise(schema: &Schema) -> Result<bool> {
    if let Some(value_schema) = typed_additional_properties_schema(schema)?
        && py_serialize_can_raise(&value_schema)
    {
        return Ok(true);
    }
    Ok(schema
        .properties
        .iter()
        .flatten()
        .any(|(_, property)| py_serialize_can_raise(property)))
}

/// True when a value's wire form is produced by a nested converter or a union
/// dispatcher — the calls that raise their own `ValidationError`, whose violations
/// are relative to the nested value and so have to be re-pathed and merged into
/// the caller's list rather than left to propagate (P11; Go's `mergeNested`).
///
/// A property routing through a `_<base>_to_transfer_type` is not decided here —
/// `render_model_serializer_body` sees the union's own classification and always
/// re-paths that call, because every union serializer validates before
/// dispatching.
fn py_serialize_can_raise(schema: &Schema) -> bool {
    if schema.reference.is_some() {
        return true;
    }
    if is_py_union(schema) {
        // Only an object (or nested-union) branch converts through a call; a union
        // of plain scalars is emitted as-is, and its checks run at this level.
        return schema
            .one_of
            .iter()
            .flatten()
            .any(|branch| branch.reference.is_some());
    }
    if let Some(non_null) = nullable_member_schema(schema) {
        return py_serialize_can_raise(non_null);
    }
    match schema.items.as_deref() {
        Some(items) => py_serialize_can_raise(items),
        None => false,
    }
}

/// Emits the statements that put one value's wire form into `sink`, descending
/// into arrays so every nested conversion runs under its own `try` and reports at
/// its own path (`segments[1]`, `location.city`). A value that needs no converting
/// call is a plain assignment, exactly as before.
#[allow(clippy::too_many_arguments)]
fn render_py_serialize_value(
    output: &mut String,
    schema: &Schema,
    sink: PySerializeSink<'_>,
    value_expr: &str,
    path_expr: &str,
    indent: &str,
    slot: &str,
) -> Result<()> {
    if !py_serialize_can_raise(schema) {
        output.push_str(indent);
        output.push_str(&sink.statement(&serialize_expr(schema, value_expr, 0)));
        output.push('\n');
        return Ok(());
    }
    // A nullability wrapper: `None` is the wire value, and the non-null branch
    // carries the conversion.
    if let Some(non_null) = nullable_member_schema(schema) {
        output.push_str(indent);
        output.push_str(&format!("if {value_expr} is None:\n"));
        output.push_str(indent);
        output.push_str("    ");
        output.push_str(&sink.statement("None"));
        output.push('\n');
        output.push_str(indent);
        output.push_str("else:\n");
        return render_py_serialize_value(
            output,
            non_null,
            sink,
            value_expr,
            path_expr,
            &format!("{indent}    "),
            slot,
        );
    }
    if schema.ty.as_ref().and_then(Value::as_str) == Some("array")
        && let Some(items) = schema.items.as_deref()
    {
        // Elementwise, so a bad element is reported at its own index and the rest
        // of the list is still converted (P11).
        let list_local = format!("{slot}_out");
        let index_local = format!("{slot}_index");
        let element_local = format!("{slot}_element");
        output.push_str(indent);
        output.push_str(&format!("{list_local}: list[typing.Any] = []\n"));
        output.push_str(indent);
        output.push_str(&format!(
            "for {index_local}, {element_local} in enumerate({value_expr}):\n"
        ));
        let loop_body = format!("{indent}    ");
        render_py_serialize_value(
            output,
            items,
            PySerializeSink::Append(&list_local),
            &element_local,
            &py_indexed_path(path_expr, &index_local),
            &loop_body,
            &format!("{slot}_item"),
        )?;
        output.push_str(indent);
        output.push_str(&sink.statement(&list_local));
        output.push('\n');
        return Ok(());
    }
    render_py_serialize_call(
        output,
        sink,
        &serialize_expr(schema, value_expr, 0),
        path_expr,
        indent,
    );
    Ok(())
}

/// Emits one converting call under a `try`, re-pathing its violations under
/// `path_expr` and merging them into the caller's list — the analogue of Go's
/// `mergeNested`, so a nested failure neither escapes alone nor discards the
/// violations already collected (P11/P12).
fn render_py_serialize_call(
    output: &mut String,
    sink: PySerializeSink<'_>,
    call_expr: &str,
    path_expr: &str,
    indent: &str,
) {
    output.push_str(indent);
    output.push_str("try:\n");
    output.push_str(indent);
    output.push_str("    ");
    output.push_str(&sink.statement(call_expr));
    output.push('\n');
    output.push_str(indent);
    output.push_str("except ValidationError as error:\n");
    output.push_str(indent);
    output.push_str(&format!("    _collect(violations, {path_expr}, error)\n"));
}

/// Emits the serialize-side validation of one declared property, guarding a
/// nullable member so the checks only fire on a materialized value. `guarded` says
/// the caller has already established the member is not `None` — as the optional
/// members' emit guard does — in which case repeating the test here would be a
/// comparison pyright reports as unnecessary (and basedpyright fails the build
/// over), so only a *required* nullable member guards itself.
///
/// A union the member converts through a `_<base>_to_transfer_type` is the one
/// exception: that function runs the union's checks itself, ahead of a dispatch
/// its last branch leaves unguarded, and the caller re-paths what it raises. Any
/// check emitted here would be that same check reported twice.
fn render_py_serialize_property_check(
    output: &mut String,
    json_name: &str,
    property: &Schema,
    models: &[&PlannedJsonType],
    guarded: bool,
    indent: &str,
) -> Result<()> {
    if classify_py_union(property, models)?.is_some_and(|union| union.needs_serializer()) {
        return Ok(());
    }
    let value_expr = format!("value.{}", property.py_member_name(json_name));
    let path_expr = python_string_literal(json_name);
    let guard_null = allows_null(property) && !guarded;
    let body_indent = if guard_null {
        format!("{indent}    ")
    } else {
        indent.to_string()
    };
    let mut body = String::new();
    render_py_field_checks(
        &mut body,
        property,
        models,
        &value_expr,
        &path_expr,
        &body_indent,
    )?;
    if body.is_empty() {
        return Ok(());
    }
    if guard_null {
        output.push_str(indent);
        output.push_str(&format!("if {value_expr} is not None:\n"));
    }
    output.push_str(&body);
    Ok(())
}

/// The member counterpart of [`render_py_serialize_property_check`]: same
/// predicates, same nullable guard, keyed by the member's own key — a catch-all
/// mutated to an invalid value fails serialization rather than emitting bad data.
fn render_py_member_check(
    output: &mut String,
    value_schema: &Schema,
    models: &[&PlannedJsonType],
    value_expr: &str,
    path_expr: &str,
    indent: &str,
) -> Result<()> {
    let guard_null = allows_null(value_schema);
    let body_indent = if guard_null {
        format!("{indent}    ")
    } else {
        indent.to_string()
    };
    let mut body = String::new();
    render_py_field_checks(
        &mut body,
        value_schema,
        models,
        value_expr,
        path_expr,
        &body_indent,
    )?;
    if body.is_empty() {
        return Ok(());
    }
    if guard_null {
        output.push_str(indent);
        output.push_str(&format!("if {value_expr} is not None:\n"));
    }
    output.push_str(&body);
    Ok(())
}

/// Emits the three-way presence branch of one declared property, per the
/// field-encoding table: required rejects an absent (and, when non-nullable, a
/// null) member; optional non-nullable rejects an explicit null; optional
/// nullable collapses both to `None`.
fn render_property_parser(
    output: &mut String,
    model: &PlannedJsonType,
    models: &[&PlannedJsonType],
    json_name: &str,
    property: &Schema,
    required: bool,
) -> Result<()> {
    // Every local this position binds hangs off the property's slot name, never
    // off the member identifier itself (see `parse_slot_local`).
    let slot = parse_slot_local(&property.py_member_name(json_name));
    let member_type = annotation(property)?;
    let key = python_string_literal(json_name);
    let path_expr = python_string_literal(json_name);
    let raw_local = format!("{slot}_raw");
    let nullable = allows_null(property);

    // Not yet assigned; a failure to parse records a violation, so the
    // placeholder never escapes the converter.
    let declared_type = if required && !nullable {
        member_type.clone()
    } else {
        optional_annotation(&member_type)
    };
    render_py_slot_declaration(output, "", &slot, &declared_type);

    if required {
        if nullable {
            output.push_str(&format!("if {key} not in raw:\n"));
        } else {
            output.push_str(&format!("if {key} not in raw or raw[{key}] is None:\n"));
        }
        output.push_str(&format!(
            "    violations.append(Violation(path={path_expr}, reason=\"required\"))\n"
        ));
        output.push_str("else:\n");
        output.push_str(&format!("    {raw_local} = raw[{key}]\n"));
        render_property_value_parser(
            output, model, models, json_name, property, &slot, &raw_local, "    ",
        )?;
        return Ok(());
    }

    output.push_str(&format!("if {key} in raw:\n"));
    output.push_str(&format!("    {raw_local} = raw[{key}]\n"));
    if nullable {
        render_property_value_parser(
            output, model, models, json_name, property, &slot, &raw_local, "    ",
        )?;
        return Ok(());
    }
    output.push_str(&format!("    if {raw_local} is None:\n"));
    output.push_str(&format!(
        "        violations.append(Violation(path={path_expr}, reason=\"explicit null not allowed\"))\n"
    ));
    output.push_str("    else:\n");
    render_property_value_parser(
        output, model, models, json_name, property, &slot, &raw_local, "        ",
    )
}

fn render_property_value_parser(
    output: &mut String,
    model: &PlannedJsonType,
    models: &[&PlannedJsonType],
    json_name: &str,
    property: &Schema,
    target: &str,
    raw_expr: &str,
    indent: &str,
) -> Result<()> {
    let path_expr = python_string_literal(json_name);
    // An inline `oneOf` sum type dispatches through the module's union parser (a
    // `$ref` at a named union routes through the reference path below).
    if classify_py_union(property, models)?.is_some() {
        let base = inline_union_fn_base(&model.model_name, &property.py_member_name(json_name));
        let parsed = format!("{target}_parsed");
        output.push_str(indent);
        output.push_str(&format!(
            "{parsed} = {}({raw_expr}, {path_expr}, violations)\n",
            union_parse_fn(&base)
        ));
        output.push_str(indent);
        output.push_str(&format!("if {parsed} is not None:\n"));
        output.push_str(indent);
        output.push_str(&format!("    {target} = {parsed}\n"));
        return Ok(());
    }
    render_value_parser(
        output, property, raw_expr, target, &path_expr, indent, target,
    )
}

/// Parses one value into `target`. `raw_expr` is always a bare or dotted name, so
/// every reason f-string can interpolate it without the subscript that only
/// became legal inside a same-quoted f-string in Python 3.12. `slot` prefixes the
/// temporaries this position needs, keeping them distinct from every other
/// position's in the same (function-wide) Python scope.
fn render_value_parser(
    output: &mut String,
    schema: &Schema,
    raw_expr: &str,
    target: &str,
    path_expr: &str,
    indent: &str,
    slot: &str,
) -> Result<()> {
    if let Some(reference) = &schema.reference {
        let model_name = reference_model_name(reference);
        if is_union_type_name(&model_name) {
            let parsed = format!("{slot}_parsed");
            output.push_str(indent);
            output.push_str(&format!(
                "{parsed} = {}({raw_expr}, {path_expr}, violations)\n",
                union_parse_fn(&union_fn_base(&model_name))
            ));
            output.push_str(indent);
            output.push_str(&format!("if {parsed} is not None:\n"));
            output.push_str(indent);
            output.push_str(&format!("    {target} = {parsed}\n"));
            return Ok(());
        }
        output.push_str(indent);
        output.push_str("try:\n");
        output.push_str(indent);
        output.push_str(&format!(
            "    {target} = {}.from_transfer_type({raw_expr}, {model_name})\n",
            converter_expr(&model_name)
        ));
        output.push_str(indent);
        output.push_str("except ValidationError as error:\n");
        output.push_str(indent);
        output.push_str(&format!("    _collect(violations, {path_expr}, error)\n"));
        return Ok(());
    }

    // The nullability wrapper: an explicit null is the value, and the non-null
    // branch carries everything the member declares.
    if let Some(branches) = &schema.one_of
        && branches
            .iter()
            .any(|branch| schema_type_includes(branch, "null"))
        && !is_py_union(schema)
    {
        output.push_str(indent);
        output.push_str(&format!("if {raw_expr} is None:\n"));
        output.push_str(indent);
        output.push_str(&format!("    {target} = None\n"));
        let non_null = branches
            .iter()
            .find(|branch| !schema_type_includes(branch, "null"));
        match non_null {
            Some(branch) => {
                output.push_str(indent);
                output.push_str("else:\n");
                render_value_parser(
                    output,
                    branch,
                    raw_expr,
                    target,
                    path_expr,
                    &format!("{indent}    "),
                    slot,
                )?;
            }
            None => {}
        }
        return Ok(());
    }

    // A materialized temporal: the wire must be a string, which the runtime
    // helper then parses (returning `None` after recording its own violation).
    if let Some(kind) = temporal_kind_direct(schema) {
        render_py_materialized_parser(
            output,
            schema,
            python_temporal_parse_fn(kind),
            raw_expr,
            target,
            path_expr,
            indent,
            slot,
        )?;
        return Ok(());
    }
    // A materialized `contentEncoding`: the same shape, decoding to `bytes`.
    if let Some(encoding) = content_encoding_direct(schema) {
        render_py_materialized_parser(
            output,
            schema,
            python_content_encoding_parse_fn(encoding),
            raw_expr,
            target,
            path_expr,
            indent,
            slot,
        )?;
        return Ok(());
    }

    // A `const` is the one-member case of the closed value set an `enum`
    // declares, so both take the same parse.
    if let Some(values) = py_closed_value_set(schema) {
        let literals = values
            .iter()
            .map(python_value_literal)
            .collect::<Result<Vec<_>>>()?;
        let member_type = annotation(schema)?;
        if py_closed_set_holds_integer(values, &member_type) {
            // An integral closed set holds an `int`, so the wire number is
            // normalized through the shared spec-integer parse before the
            // comparison — the wire `1.0` *is* the integer `1`, and only an
            // `int` re-serializes as `1` rather than `1.0` (Go routes the same
            // value through `parseIntegerField`, Java through
            // `SpecNumbers.specLong`). Normalizing here is also what reinstates
            // the `1.5` reject and the integer cap for these fields, which a
            // bare membership test bypasses.
            let parsed = format!("{slot}_parsed");
            output.push_str(indent);
            output.push_str(&format!(
                "{parsed} = _parse_spec_integer({raw_expr}, {path_expr}, violations)\n"
            ));
            output.push_str(indent);
            output.push_str(&format!("if {parsed} is not None:\n"));
            // The membership test narrows the normalized `int` to the closed
            // literal type it declares, so the member takes it as it is — no
            // cast, unlike the pre-normalization form where the comparison ran
            // against an `int | float`.
            render_py_closed_value_membership(
                output,
                "if",
                &literals,
                &parsed,
                &parsed,
                target,
                path_expr,
                &format!("{indent}    "),
                &py_closed_value_reason(schema, values, &parsed),
            );
        } else {
            render_py_closed_value_parser(
                output,
                values,
                &literals,
                raw_expr,
                target,
                path_expr,
                indent,
                &py_closed_value_reason(schema, values, raw_expr),
            );
        }
        return Ok(());
    }

    match schema.ty.as_ref().and_then(Value::as_str) {
        Some("string") => {
            render_py_isinstance_parser(
                output,
                &format!("isinstance({raw_expr}, str)"),
                "expected string",
                raw_expr,
                target,
                path_expr,
                indent,
            );
            render_py_string_checks(
                output,
                raw_expr,
                path_expr,
                schema,
                &format!("{indent}    "),
            );
        }
        Some("boolean") => render_py_isinstance_parser(
            output,
            &format!("isinstance({raw_expr}, bool)"),
            "expected boolean",
            raw_expr,
            target,
            path_expr,
            indent,
        ),
        Some("number") => {
            render_py_isinstance_parser(
                output,
                &format!(
                    "not isinstance({raw_expr}, bool) and isinstance({raw_expr}, (int, float))"
                ),
                "expected number",
                raw_expr,
                target,
                path_expr,
                indent,
            );
            render_py_numeric_checks(
                output,
                raw_expr,
                path_expr,
                schema,
                &format!("{indent}    "),
            );
        }
        Some("integer") => {
            // `1.0` is an integer and `1.5` is not, so the parse is the shared
            // spec-integer helper rather than a bare type test.
            let parsed = format!("{slot}_parsed");
            output.push_str(indent);
            output.push_str(&format!(
                "{parsed} = _parse_spec_integer({raw_expr}, {path_expr}, violations)\n"
            ));
            output.push_str(indent);
            output.push_str(&format!("if {parsed} is not None:\n"));
            output.push_str(indent);
            output.push_str(&format!("    {target} = {parsed}\n"));
            render_py_numeric_checks(output, target, path_expr, schema, &format!("{indent}    "));
        }
        Some("array") => {
            render_array_parser(output, schema, raw_expr, target, path_expr, indent, slot)?
        }
        Some("null") => {
            output.push_str(indent);
            output.push_str(&format!("{target} = None\n"));
        }
        // A free-form or inline object, and anything untyped: the wire value is
        // already the member (P13).
        _ => {
            output.push_str(indent);
            output.push_str(&format!("{target} = {raw_expr}\n"));
        }
    }
    Ok(())
}

/// Emits the parse of a materialized value: the string guard the runtime helpers
/// require, then the helper itself.
fn render_py_materialized_parser(
    output: &mut String,
    schema: &Schema,
    parse_fn: &str,
    raw_expr: &str,
    target: &str,
    path_expr: &str,
    indent: &str,
    slot: &str,
) -> Result<()> {
    let parsed = format!("{slot}_parsed");
    output.push_str(indent);
    output.push_str(&format!("if not isinstance({raw_expr}, str):\n"));
    output.push_str(indent);
    output.push_str(&format!(
        "    violations.append(Violation(path={path_expr}, reason=\"expected string\"))\n"
    ));
    output.push_str(indent);
    output.push_str("else:\n");
    render_py_materialized_wire_checks(
        output,
        schema,
        raw_expr,
        path_expr,
        &format!("{indent}    "),
    )?;
    output.push_str(indent);
    output.push_str(&format!(
        "    {parsed} = {parse_fn}({raw_expr}, {path_expr}, violations)\n"
    ));
    output.push_str(indent);
    output.push_str(&format!("    if {parsed} is not None:\n"));
    output.push_str(indent);
    output.push_str(&format!("        {target} = {parsed}\n"));
    Ok(())
}

/// Validates constraints expressed over the JSON string of a materialized
/// temporal or byte value. The parse side calls this before native conversion;
/// the serialize side calls it over the canonical string produced by the
/// formatter.
fn render_py_materialized_wire_checks(
    output: &mut String,
    schema: &Schema,
    wire_expr: &str,
    path_expr: &str,
    indent: &str,
) -> Result<()> {
    let length = format!("len({wire_expr})");
    if let Some(min) = schema.min_length {
        render_py_violation_if(
            output,
            indent,
            &format!("{length} < {min}"),
            path_expr,
            &format!("f\"must have length >= {min}, got {{{length}}}\""),
        );
    }
    if let Some(max) = schema.max_length {
        render_py_violation_if(
            output,
            indent,
            &format!("{length} > {max}"),
            path_expr,
            &format!("f\"must have length <= {max}, got {{{length}}}\""),
        );
    }
    if let Some(pattern) = &schema.pattern {
        render_py_pattern_check(output, wire_expr, path_expr, pattern, indent);
    }
    // A temporal format is the materializer itself. For content-encoded values,
    // `format` remains an independently asserted string constraint.
    if content_encoding_direct(schema).is_some()
        && let Some(format) = &schema.format
    {
        render_py_format_check(output, wire_expr, path_expr, format, indent);
    }
    if let Some(values) = py_closed_value_set(schema) {
        let literals = values
            .iter()
            .map(python_value_literal)
            .collect::<Result<Vec<_>>>()?;
        render_py_closed_value_check(
            output,
            &literals,
            wire_expr,
            path_expr,
            indent,
            &py_closed_value_reason(schema, values, wire_expr),
        );
    }
    Ok(())
}

/// Emits `if not <guard>: <reason> else: <assign>`, the shape every scalar kind's
/// parse shares. Any per-kind constraint check is appended by the caller at the
/// `else` body's indent.
fn render_py_isinstance_parser(
    output: &mut String,
    guard: &str,
    reason: &str,
    raw_expr: &str,
    target: &str,
    path_expr: &str,
    indent: &str,
) {
    output.push_str(indent);
    output.push_str(&format!("if not {}:\n", py_negatable(guard)));
    output.push_str(indent);
    output.push_str(&format!(
        "    violations.append(Violation(path={path_expr}, reason={}))\n",
        python_string_literal(reason)
    ));
    output.push_str(indent);
    output.push_str("else:\n");
    output.push_str(indent);
    output.push_str(&format!("    {target} = {raw_expr}\n"));
}

/// Declares a local for a value that has not been parsed yet. The placeholder is
/// widened through `typing.Any` rather than cast from `None`, which would be a
/// type error on any annotation that does not admit it.
fn render_py_slot_declaration(output: &mut String, indent: &str, name: &str, member_type: &str) {
    output.push_str(indent);
    if member_type == "typing.Any" || admits_none(member_type) {
        output.push_str(&format!("{name}: {member_type} = None\n"));
    } else {
        output.push_str(&format!(
            "{name}: {member_type} = typing.cast(\"typing.Any\", None)\n"
        ));
    }
}

/// Parenthesizes a condition only when `not` would otherwise bind tighter than
/// its operators, so a single `isinstance(...)` call keeps its plain reading.
fn py_negatable(condition: &str) -> String {
    if condition.contains(" and ") || condition.contains(" or ") {
        format!("({condition})")
    } else {
        condition.to_string()
    }
}

/// The kind test and reason word for a closed value set, taken from its first
/// member's JSON kind.
fn py_closed_value_guard(value: &Value, raw_expr: &str) -> Option<(String, &'static str)> {
    match value {
        Value::String(_) => Some((format!("isinstance({raw_expr}, str)"), "expected string")),
        Value::Bool(_) => Some((format!("isinstance({raw_expr}, bool)"), "expected boolean")),
        Value::Number(_) => Some((
            format!("not isinstance({raw_expr}, bool) and isinstance({raw_expr}, (int, float))"),
            "expected number",
        )),
        _ => None,
    }
}

/// The closed value set a schema declares: the single `const` value, or the
/// `enum` members. Both are the same assertion — membership in a fixed set —
/// so both are emitted by one path.
fn py_closed_value_set(schema: &Schema) -> Option<&[Value]> {
    schema
        .const_value
        .as_ref()
        .map(std::slice::from_ref)
        .or(schema.enum_values.as_deref())
        .filter(|values| !values.is_empty())
}

/// True when a closed numeric value set holds an `int` at rest: every member is
/// an integral JSON number, which is exactly when the emitted annotation is a
/// numeric `typing.Literal[…]`. A float-valued set has no `Literal` form (PEP
/// 586 admits no float member), falls through to a plain `float`, and keeps the
/// wire value as it arrived. See `specs/json-schema/features/enum.md`.
fn py_closed_set_holds_integer(values: &[Value], member_type: &str) -> bool {
    member_type.starts_with("typing.Literal[") && values.iter().all(Value::is_number)
}

/// The membership violation reason for a closed value set: a `const` names its
/// single value, an `enum` names the admissible set and the offending value.
fn py_closed_value_reason(schema: &Schema, values: &[Value], value_expr: &str) -> String {
    match &schema.const_value {
        Some(const_value) => {
            python_string_literal(&format!("must equal {}", py_reason_literal(const_value)))
        }
        None => py_enum_reason(values, value_expr),
    }
}

/// Emits the closed-value (`const` single-value / `enum` multi-value) parse: a
/// kind test, a membership test against the fixed set, and the assignment on
/// success. See `specs/json-schema/features/{const,enum}.md`.
#[allow(clippy::too_many_arguments)]
fn render_py_closed_value_parser(
    output: &mut String,
    values: &[Value],
    compare_exprs: &[String],
    raw_expr: &str,
    target: &str,
    path_expr: &str,
    indent: &str,
    reason: &str,
) {
    let keyword = match values
        .first()
        .and_then(|value| py_closed_value_guard(value, raw_expr))
    {
        Some((guard, kind_reason)) => {
            output.push_str(indent);
            output.push_str(&format!("if not {}:\n", py_negatable(&guard)));
            output.push_str(indent);
            output.push_str(&format!(
                "    violations.append(Violation(path={path_expr}, reason={}))\n",
                python_string_literal(kind_reason)
            ));
            "elif"
        }
        None => "if",
    };
    // The value reaches the member as it arrived: a string, boolean or float
    // set narrows to the type it declares through the membership test itself.
    render_py_closed_value_membership(
        output,
        keyword,
        compare_exprs,
        raw_expr,
        raw_expr,
        target,
        path_expr,
        indent,
        reason,
    );
}

/// Emits the membership test of a closed value set and the assignment on
/// success. `keyword` chains the test onto a preceding kind test (`elif`) or
/// opens it (`if`); `compared` is the expression held to the set and
/// `assignment` the value stored once it passes.
#[allow(clippy::too_many_arguments)]
fn render_py_closed_value_membership(
    output: &mut String,
    keyword: &str,
    compare_exprs: &[String],
    compared: &str,
    assignment: &str,
    target: &str,
    path_expr: &str,
    indent: &str,
    reason: &str,
) {
    let membership = format!("{compared} not in {}", py_value_tuple(compare_exprs));
    output.push_str(indent);
    output.push_str(&format!("{keyword} {membership}:\n"));
    output.push_str(indent);
    output.push_str(&format!(
        "    violations.append(Violation(path={path_expr}, reason={reason}))\n"
    ));
    output.push_str(indent);
    output.push_str("else:\n");
    output.push_str(indent);
    output.push_str(&format!("    {target} = {assignment}\n"));
}

/// Emits the elementwise parse of an array. The typed list contains only
/// successfully converted elements and never escapes when any violation exists;
/// sibling array keywords are evaluated separately over the original wire list.
#[allow(clippy::too_many_arguments)]
fn render_array_parser(
    output: &mut String,
    schema: &Schema,
    raw_expr: &str,
    target: &str,
    path_expr: &str,
    indent: &str,
    slot: &str,
) -> Result<()> {
    output.push_str(indent);
    output.push_str(&format!("if not isinstance({raw_expr}, list):\n"));
    output.push_str(indent);
    output.push_str(&format!(
        "    violations.append(Violation(path={path_expr}, reason=\"expected array\"))\n"
    ));
    output.push_str(indent);
    output.push_str("else:\n");
    let list_local = render_py_array_elements(
        output,
        schema,
        raw_expr,
        path_expr,
        &format!("{indent}    "),
        slot,
    )?;
    output.push_str(indent);
    output.push_str(&format!("    {target} = {list_local}\n"));
    Ok(())
}

/// Emits the elementwise parse of a value the caller has *already* established is
/// a `list`: every element through its own declared type, then the array-level
/// predicates over the built list. Returns the name of the local holding it, so a
/// caller that only needs the value takes it without a copy. Split out of
/// [`render_array_parser`] so a union branch — whose token guard is that same
/// `isinstance` test — reuses it without re-testing (a redundant guard would trip
/// pyright's narrowing diagnostics).
fn render_py_array_elements(
    output: &mut String,
    schema: &Schema,
    raw_expr: &str,
    path_expr: &str,
    body: &str,
    slot: &str,
) -> Result<String> {
    let item_slot = format!("{slot}_item");
    let list_local = format!("{slot}_list");
    let index_local = format!("{slot}_index");
    let element_local = format!("{slot}_element");
    let item_path_local = format!("{item_slot}_path");
    let violation_count_local = format!("{item_slot}_violation_count");
    let item_type = schema
        .items
        .as_ref()
        .map(|item| annotation(item))
        .transpose()?
        .unwrap_or_else(|| "typing.Any".to_string());

    let body = body.to_string();
    output.push_str(&body);
    output.push_str(&format!("{list_local}: list[{item_type}] = []\n"));
    output.push_str(&body);
    output.push_str(&format!(
        "for {index_local}, {element_local} in enumerate(typing.cast(\"list[typing.Any]\", {raw_expr})):\n"
    ));
    let loop_body = format!("{body}    ");
    output.push_str(&loop_body);
    output.push_str(&format!(
        "{item_path_local} = {}\n",
        py_indexed_path(path_expr, &index_local)
    ));
    output.push_str(&loop_body);
    output.push_str(&format!("{violation_count_local} = len(violations)\n"));
    render_py_slot_declaration(output, &loop_body, &item_slot, &item_type);
    match &schema.items {
        // Every element kind takes the same parse the value in that position
        // would take anywhere else, so a mistyped element names the type it
        // failed to be (`expected string`) at its own index — see
        // `specs/json-schema/features/items.md`.
        Some(item_schema) => render_value_parser(
            output,
            item_schema,
            &element_local,
            &item_slot,
            &item_path_local,
            &loop_body,
            &item_slot,
        )?,
        None => {
            output.push_str(&loop_body);
            output.push_str(&format!("{item_slot} = {element_local}\n"));
        }
    }
    output.push_str(&loop_body);
    output.push_str(&format!("if len(violations) == {violation_count_local}:\n"));
    output.push_str(&loop_body);
    output.push_str(&format!("    {list_local}.append({item_slot})\n"));
    // Array-level keywords are siblings of `items`: they inspect the original
    // instance even when one or more elements fail conversion.
    let raw_array = format!("typing.cast(\"list[typing.Any]\", {raw_expr})");
    render_py_array_checks(output, &raw_array, path_expr, schema, &body)?;
    Ok(list_local)
}

fn render_closed_object_unknown_key_check(output: &mut String, schema: &Schema) {
    let fields = schema
        .properties
        .as_ref()
        .map(|properties| {
            properties
                .keys()
                .map(|field| format!("key != {}", python_string_literal(field)))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    output.push_str("for key in raw:\n");
    if fields.is_empty() {
        // A closed object with no declared members admits nothing at all.
        output.push_str("    violations.append(Violation(path=key, reason=\"unknown field\"))\n");
        return;
    }
    output.push_str(&format!("    if {}:\n", fields.join(" and ")));
    output.push_str("        violations.append(Violation(path=key, reason=\"unknown field\"))\n");
}

fn render_open_object_collection(
    output: &mut String,
    model: &PlannedJsonType,
    schema: &Schema,
) -> Result<()> {
    let value_schema = typed_additional_properties_schema(schema)?;
    let value_annotation = additional_properties_annotation(schema)?;
    output.push_str(&format!(
        "additional_properties: dict[str, {}] = {{}}\n",
        additional_properties_annotation(schema)?
    ));
    output.push_str("for key in raw:\n");
    output.push_str(&format!(
        "    if key not in {}:\n",
        declared_fields_const_name(&model.model_name)
    ));
    match value_schema {
        None => output.push_str("        additional_properties[key] = raw[key]\n"),
        Some(value_schema) => {
            render_py_slot_declaration(output, "        ", "member", &value_annotation);
            output.push_str("        member_raw = raw[key]\n");
            output.push_str("        member_violation_count = len(violations)\n");
            render_value_parser(
                output,
                &value_schema,
                "member_raw",
                "member",
                "key",
                "        ",
                "member",
            )?;
            output.push_str("        if len(violations) == member_violation_count:\n");
            output.push_str("            additional_properties[key] = member\n");
        }
    }
    Ok(())
}

/// The wire form of an in-memory value. A Python dataclass is not its own wire
/// shape (snake_case attributes, an explicit catch-all, native temporals and
/// `bytes`), so a container whose elements transform is always descended into —
/// unlike TypeScript, where a closed interface can be copied verbatim.
fn serialize_expr(schema: &Schema, value_expr: &str, depth: usize) -> String {
    if let Some(reference) = &schema.reference {
        let model_name = reference_model_name(reference);
        if is_union_type_name(&model_name) {
            return format!(
                "{}({value_expr})",
                union_serialize_fn(&union_fn_base(&model_name))
            );
        }
        return format!(
            "{}.to_transfer_type({value_expr})",
            converter_expr(&model_name)
        );
    }
    if let Some(kind) = temporal_kind_direct(schema) {
        return format!("{}({value_expr})", python_temporal_format_fn(kind));
    }
    if let Some(encoding) = content_encoding_direct(schema) {
        return format!(
            "{}({value_expr})",
            python_content_encoding_format_fn(encoding)
        );
    }
    if let Some(branches) = &schema.one_of
        && branches
            .iter()
            .any(|branch| schema_type_includes(branch, "null"))
        && !is_py_union(schema)
        && let Some(non_null) = branches
            .iter()
            .find(|branch| !schema_type_includes(branch, "null"))
    {
        let inner = serialize_expr(non_null, value_expr, depth);
        if inner != value_expr {
            return format!("None if {value_expr} is None else {inner}");
        }
    }
    if schema.ty.as_ref().and_then(Value::as_str) == Some("array")
        && let Some(items) = schema.items.as_deref()
    {
        // Comprehension scopes nest, so each level names its own element.
        let element = if depth == 0 {
            "element".to_string()
        } else {
            format!("element{depth}")
        };
        let mapped = serialize_expr(items, &element, depth + 1);
        if mapped != element {
            return format!("[{mapped} for {element} in {value_expr}]");
        }
    }
    value_expr.to_string()
}

/// Emits the spec-integer parse helper: accepts `1` and `1.0`, rejects `1.5` and
/// anything beyond the ±(2^53−1) cap, and rejects `bool` (a Python `int`
/// subclass, so `isinstance(True, int)` is `True` and must be excluded
/// explicitly). Pushes a `Violation` and returns `None` on failure so the caller
/// keeps going and aggregates. See `specs/json-schema/features/type.md`.
fn render_spec_int_helper(output: &mut String) {
    output.push_str(SPEC_INT_HELPER_BODY);
}

const SPEC_INT_HELPER_BODY: &str = r#"_INTEGER_CAP = (1 << 53) - 1


def _parse_spec_integer(
    value: object, path: str, violations: list[Violation]
) -> int | None:
    """Parses a JSON number as a spec integer (`1.0` accepted, `1.5` rejected)."""

    # `bool` is a subclass of `int`, so it must be excluded before the int check.
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        violations.append(Violation(path=path, reason="expected integer"))
        return None
    if isinstance(value, float):
        if not value.is_integer():
            violations.append(Violation(path=path, reason="expected integer"))
            return None
        out = int(value)
    else:
        out = value
    if abs(out) > _INTEGER_CAP:
        violations.append(Violation(path=path, reason="expected integer"))
        return None
    return out
"#;

fn typed_map_value_schema(schema: &Schema) -> Result<Option<Schema>> {
    if schema
        .properties
        .as_ref()
        .is_some_and(|properties| !properties.is_empty())
    {
        return Ok(None);
    }

    typed_additional_properties_schema(schema)
}

/// The declared schema for catch-all members. Unlike `typed_map_value_schema`,
/// this also applies when an object has declared properties: mixed objects keep
/// their declared fields and expose all other keys through the typed map.
fn typed_additional_properties_schema(schema: &Schema) -> Result<Option<Schema>> {
    match &schema.additional_properties {
        Some(Value::Object(_)) => serde_json::from_value(
            schema
                .additional_properties
                .clone()
                .expect("additional properties presence checked"),
        )
        .map(Some)
        .map_err(|error| Error::InvalidJsonSchema {
            path: PathBuf::from("<json-generator>"),
            reason: format!("failed to read `additionalProperties`: {error}"),
        }),
        _ => Ok(None),
    }
}

/// True when the model is map-shaped: an object with no declared `properties`
/// whose members are open (typed via `additionalProperties`, or free-form for
/// `true`). A closed empty object (`additionalProperties: false`) admits no
/// members and is not map-shaped.
fn is_python_map_model(schema: &Schema) -> bool {
    schema.ty.as_ref().and_then(Value::as_str) == Some("object")
        && schema
            .properties
            .as_ref()
            .is_none_or(|properties| properties.is_empty())
        && schema.additional_properties.as_ref() != Some(&Value::Bool(false))
}

/// The non-null branch of a member schema that is the nullability `oneOf`
/// wrapper, which carries the member's own constraints.
fn nullable_member_schema(schema: &Schema) -> Option<&Schema> {
    let branches = schema.one_of.as_ref()?;
    let non_null: Vec<&Schema> = branches
        .iter()
        .filter(|branch| branch.ty.as_ref().and_then(Value::as_str) != Some("null"))
        .collect();
    match non_null.len() {
        1 => Some(non_null[0]),
        _ => None,
    }
}

/// Builds the boolean Python sub-conditions that define "match" for a scalar
/// `contains` matcher over `elem`. A type-only matcher matches every element, so
/// an empty condition set renders as the literal `True`.
fn py_matcher_condition(
    matcher: &Schema,
    element_schema: Option<&Schema>,
    elem: &str,
) -> Result<String> {
    let matcher = scalar_matcher(matcher);
    let kind = matcher.kind.or_else(|| {
        matcher
            .const_value
            .as_ref()
            .and_then(scalar_kind_for_value)
            .or_else(|| matcher.enum_values.first().and_then(scalar_kind_for_value))
            .or_else(|| {
                element_schema
                    .and_then(|schema| schema.ty.as_ref())
                    .and_then(Value::as_str)
                    .and_then(ScalarKind::from_name)
            })
    });
    let mut parts: Vec<String> = Vec::new();
    if let Some(kind) = kind {
        parts.push(match kind {
            ScalarKind::String => format!("isinstance({elem}, str)"),
            ScalarKind::Boolean => format!("isinstance({elem}, bool)"),
            ScalarKind::Number => format!(
                "not isinstance({elem}, bool) and isinstance({elem}, (int, float)) and -{PY_BINARY64_MAX} <= {elem} <= {PY_BINARY64_MAX}"
            ),
            ScalarKind::Integer => format!(
                "not isinstance({elem}, bool) and isinstance({elem}, (int, float)) and abs({elem}) <= {PY_INTEGER_CAP} and float({elem}).is_integer()"
            ),
        });
    }
    if let Some(value) = &matcher.const_value {
        parts.push(format!(
            "_json_values_equal({elem}, {})",
            python_value_literal(value)?
        ));
    }
    if !matcher.enum_values.is_empty() {
        let alternatives = matcher
            .enum_values
            .iter()
            .map(|value| {
                Ok(format!(
                    "_json_values_equal({elem}, {})",
                    python_value_literal(value)?
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        if !alternatives.is_empty() {
            parts.push(format!("({})", alternatives.join(" or ")));
        }
    }
    let is_integer = kind == Some(ScalarKind::Integer);
    if let Some(min) = &matcher.minimum {
        parts.push(format!("{elem} >= {}", py_bound_literal(min, is_integer)));
    }
    if let Some(max) = &matcher.maximum {
        parts.push(format!("{elem} <= {}", py_bound_literal(max, is_integer)));
    }
    if let Some(min) = &matcher.exclusive_minimum {
        parts.push(format!("{elem} > {}", py_bound_literal(min, is_integer)));
    }
    if let Some(max) = &matcher.exclusive_maximum {
        parts.push(format!("{elem} < {}", py_bound_literal(max, is_integer)));
    }
    if let Some(divisor) = &matcher.multiple_of {
        let divisor = py_bound_literal(divisor, is_integer);
        parts.push(if is_integer {
            format!("{elem} % {divisor} == 0")
        } else {
            format!("math.fmod({elem}, {divisor}) == 0")
        });
    }
    if let Some(min) = matcher.min_length {
        parts.push(format!("len({elem}) >= {min}"));
    }
    if let Some(max) = matcher.max_length {
        parts.push(format!("len({elem}) <= {max}"));
    }
    if let Some(pattern) = &matcher.pattern {
        let rewritten = crate::json_schema::pattern::rewrite_end_anchor(pattern, r"\Z");
        let const_name = py_pattern_const_name(&rewritten);
        parts.push(format!("{const_name}.search({elem}) is not None"));
    }
    if let Some(format) = &matcher.format
        && let Some(check) = crate::json_schema::format::check_for(format)
    {
        let rewritten = crate::json_schema::pattern::rewrite_end_anchor(&check.pattern, r"\Z");
        let const_name = py_pattern_const_name(&rewritten);
        if let Some(max) = check.max_code_points {
            parts.push(format!("len({elem}) <= {max}"));
        }
        parts.push(format!("{const_name}.search({elem}) is not None"));
    }
    if parts.is_empty() {
        Ok("True".to_string())
    } else {
        Ok(parts.join(" and "))
    }
}

fn scalar_matcher(schema: &Schema) -> ScalarMatcher {
    ScalarMatcher {
        kind: schema
            .ty
            .as_ref()
            .and_then(Value::as_str)
            .and_then(ScalarKind::from_name),
        const_value: schema.const_value.clone(),
        enum_values: schema.enum_values.clone().unwrap_or_default(),
        minimum: schema.minimum.clone(),
        maximum: schema.maximum.clone(),
        exclusive_minimum: schema.exclusive_minimum.clone(),
        exclusive_maximum: schema.exclusive_maximum.clone(),
        multiple_of: schema.multiple_of.clone(),
        min_length: schema.min_length,
        max_length: schema.max_length,
        pattern: schema.pattern.clone(),
        format: schema.format.clone(),
    }
}

fn scalar_kind_for_value(value: &Value) -> Option<ScalarKind> {
    match value {
        Value::String(_) => Some(ScalarKind::String),
        Value::Bool(_) => Some(ScalarKind::Boolean),
        Value::Number(number) if number.is_i64() || number.is_u64() => Some(ScalarKind::Integer),
        Value::Number(_) => Some(ScalarKind::Number),
        _ => None,
    }
}

/// Composes a docstring from a `title` (summary line) and `description` (body);
/// returns `None` when both are empty. See specs/json-schema/features/{title,description}.md.
fn compose_python_doc(title: Option<&str>, description: Option<&str>) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(title) = title.map(str::trim).filter(|t| !t.is_empty()) {
        parts.push(title.to_string());
    }
    if let Some(description) = description.map(str::trim).filter(|d| !d.is_empty()) {
        parts.push(description.to_string());
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n"))
    }
}

fn python_value_literal(value: &Value) -> Result<String> {
    match value {
        Value::Null => Ok("None".to_string()),
        Value::Bool(value) => Ok(if *value { "True" } else { "False" }.to_string()),
        Value::Number(value) => Ok(value.to_string()),
        Value::String(value) => Ok(python_string_literal(value)),
        Value::Array(values) => {
            let values = values
                .iter()
                .map(python_value_literal)
                .collect::<Result<Vec<_>>>()?;
            Ok(format!("[{}]", values.join(", ")))
        }
        Value::Object(values) => {
            let values = values
                .iter()
                .map(|(key, value)| {
                    Ok(format!(
                        "{}: {}",
                        python_string_literal(key),
                        python_value_literal(value)?
                    ))
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(format!("{{{}}}", values.join(", ")))
        }
    }
}

/// Renders a schema value in the same native representation as its public
/// annotation. Loader validation has already proved defaults and closed values
/// valid, so the generated helper call is a deterministic construction rather
/// than another user-visible validation boundary.
fn python_typed_value_literal(schema: &Schema, value: &Value) -> Result<String> {
    if value.is_null() {
        return Ok("None".to_string());
    }
    let schema = nullable_member_schema(schema).unwrap_or(schema);
    if let Some(text) = value.as_str() {
        let literal = python_string_literal(text);
        if let Some(kind) = temporal_kind_direct(schema) {
            return Ok(format!(
                "typing.cast(\"{}\", {}({}, \"\", []))",
                python_temporal_type(kind),
                python_temporal_parse_fn(kind),
                literal
            ));
        }
        if let Some(encoding) = content_encoding_direct(schema) {
            return Ok(format!(
                "typing.cast(\"bytes\", {}({}, \"\", []))",
                python_content_encoding_parse_fn(encoding),
                literal
            ));
        }
    }
    python_value_literal(value)
}

fn decode_schema(model: &PlannedJsonType) -> Result<Schema> {
    serde_json::from_value(model.schema.clone()).map_err(|error| Error::InvalidJsonSchema {
        path: PathBuf::from("<json-generator>"),
        reason: format!(
            "failed to read planned JSON schema `{}`: {error}",
            model.full_name
        ),
    })
}

/// The materialized `TemporalKind` of a schema that is directly a temporal
/// string (not looking through `oneOf`, which `annotation` handles by recursion).
fn temporal_kind_direct(schema: &Schema) -> Option<crate::json_schema::format::TemporalKind> {
    if schema.ty.as_ref().and_then(Value::as_str) != Some("string") {
        return None;
    }
    schema
        .format
        .as_deref()
        .and_then(crate::json_schema::format::TemporalKind::from_name)
}

/// The materialized `contentEncoding` of a schema that is directly a bytes string
/// (the `oneOf[…, null]` wrapper is handled by `annotation` recursion).
fn content_encoding_direct(
    schema: &Schema,
) -> Option<crate::json_schema::content_encoding::Encoding> {
    if schema.ty.as_ref().and_then(Value::as_str) != Some("string") {
        return None;
    }
    schema
        .content_encoding
        .as_deref()
        .and_then(crate::json_schema::content_encoding::Encoding::from_name)
}

fn annotation(schema: &Schema) -> Result<String> {
    // Materialization determines the public Python type even when the wire
    // schema also closes the value with `const` or `enum`.
    if let Some(kind) = temporal_kind_direct(schema) {
        return Ok(python_temporal_type(kind).to_string());
    }
    if content_encoding_direct(schema).is_some() {
        return Ok("bytes".to_string());
    }
    if let Some(const_value) = &schema.const_value
        && let Some(annotation) = python_literal_annotation(const_value)
    {
        return Ok(annotation);
    }
    // A scalar `enum` is a closed `Literal` union. Number(float) members are the
    // exception (PEP 586 forbids float literals): they fall through to the plain
    // `float` type and rest on the membership validator (see enum.md).
    if let Some(values) = &schema.enum_values
        && !values.is_empty()
    {
        let tokens = values
            .iter()
            .filter_map(python_literal_token)
            .collect::<Vec<_>>();
        if tokens.len() == values.len() {
            return Ok(format!("typing.Literal[{}]", tokens.join(", ")));
        }
    }
    if let Some(reference) = &schema.reference {
        return Ok(reference_model_name(reference));
    }
    if let Some(one_of) = &schema.one_of {
        let non_null = one_of
            .iter()
            .filter(|branch| branch.ty.as_ref().and_then(Value::as_str) != Some("null"))
            .collect::<Vec<_>>();
        let nullable = one_of
            .iter()
            .any(|branch| branch.ty.as_ref().and_then(Value::as_str) == Some("null"));
        // Two or more non-null branches form a closed sum type — a native
        // `A | B` union the converter selects a branch of by JSON token and
        // discriminant. One non-null branch is the degenerate nullability
        // pattern.
        if non_null.len() >= 2 {
            // A branch's own constraints are checked by the union's dispatcher,
            // not carried on the annotation, so the member type is the plain
            // branch type ([[oneOf]] §"Validator mapping").
            let mut members = non_null
                .iter()
                .map(|branch| annotation(branch))
                .collect::<Result<Vec<_>>>()?;
            if nullable {
                members.push("None".to_string());
            }
            return Ok(members.join(" | "));
        }
        let Some(branch) = non_null.first() else {
            return Ok("None".to_string());
        };
        return Ok(optional_annotation(&annotation(branch)?));
    }
    // A materialized temporal `format` replaces `str` with the native Python
    // type; the converter owns the parse and the canonical serialize. The
    // `oneOf[…, null]` nullable wrapper is handled above by recursing into the
    // non-null branch.
    if let Some(kind) = temporal_kind_direct(schema) {
        return Ok(python_temporal_type(kind).to_string());
    }
    // A materialized `contentEncoding` replaces `str` with `bytes`; the converter
    // owns the codec in both directions.
    if content_encoding_direct(schema).is_some() {
        return Ok("bytes".to_string());
    }
    match schema.ty.as_ref().and_then(Value::as_str) {
        Some("string") => Ok("str".to_string()),
        Some("integer") => Ok("int".to_string()),
        Some("number") => Ok("float".to_string()),
        Some("boolean") => Ok("bool".to_string()),
        Some("array") => {
            let item = schema
                .items
                .as_ref()
                .map(|item| annotation(item))
                .transpose()?
                .unwrap_or_else(|| "typing.Any".to_string());
            Ok(format!("list[{item}]"))
        }
        Some("object") => object_annotation(schema),
        Some("null") => Ok("None".to_string()),
        _ => Ok("typing.Any".to_string()),
    }
}

fn python_literal_annotation(value: &Value) -> Option<String> {
    python_literal_token(value).map(|token| format!("typing.Literal[{token}]"))
}

/// The inner `Literal[...]` member token for a scalar value, or `None` when the
/// value cannot be a `Literal` member (a float — PEP 586 — or a composite).
fn python_literal_token(value: &Value) -> Option<String> {
    match value {
        Value::Null => Some("None".to_string()),
        Value::Bool(value) => Some(if *value { "True" } else { "False" }.to_string()),
        Value::Number(value) if value.is_i64() || value.is_u64() => Some(value.to_string()),
        Value::String(value) => Some(python_string_literal(value)),
        _ => None,
    }
}

fn allows_null(schema: &Schema) -> bool {
    schema.const_value.as_ref() == Some(&Value::Null)
        || schema_type_includes(schema, "null")
        || schema
            .one_of
            .as_ref()
            .is_some_and(|branches| branches.iter().any(allows_null))
}

fn schema_type_includes(schema: &Schema, ty: &str) -> bool {
    match schema.ty.as_ref() {
        Some(Value::String(value)) => value == ty,
        Some(Value::Array(values)) => values
            .iter()
            .any(|value| value.as_str().is_some_and(|value| value == ty)),
        _ => false,
    }
}

fn object_annotation(schema: &Schema) -> Result<String> {
    if schema
        .properties
        .as_ref()
        .is_some_and(|properties| !properties.is_empty())
    {
        return Ok("dict[str, typing.Any]".to_string());
    }
    match &schema.additional_properties {
        Some(Value::Object(_)) => {
            let additional: Schema = serde_json::from_value(
                schema
                    .additional_properties
                    .clone()
                    .expect("additional properties presence checked"),
            )
            .map_err(|error| Error::InvalidJsonSchema {
                path: PathBuf::from("<json-generator>"),
                reason: format!("failed to read `additionalProperties`: {error}"),
            })?;
            Ok(format!("dict[str, {}]", annotation(&additional)?))
        }
        _ => Ok("dict[str, typing.Any]".to_string()),
    }
}

fn optional_annotation(annotation: &str) -> String {
    if admits_none(annotation) {
        annotation.to_string()
    } else {
        format!("{annotation} | None")
    }
}

/// True when the annotation itself already admits `None` — a `None` member of
/// the *top-level* union. A nested one does not count: in `list[str | None]`
/// the elements are nullable while the list is not, so an optional field of
/// that type still needs its own `| None` ([[items]] §"Element nullability is
/// the element's own concern").
fn admits_none(annotation: &str) -> bool {
    split_top_level_union(annotation).contains(&"None")
}

/// Splits a type annotation on its top-level `|`, ignoring any inside a
/// subscript (`list[str | None]` is one member, not two).
fn split_top_level_union(annotation: &str) -> Vec<&str> {
    let mut members = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (index, character) in annotation.char_indices() {
        match character {
            '[' | '(' => depth += 1,
            ']' | ')' => depth = depth.saturating_sub(1),
            '|' if depth == 0 => {
                members.push(annotation[start..index].trim());
                start = index + character.len_utf8();
            }
            _ => {}
        }
    }
    members.push(annotation[start..].trim());
    members
}

fn reference_model_name(reference: &str) -> String {
    if let Some(resolved) = REF_NAMES.with(|cell| cell.borrow().get(reference).cloned()) {
        return resolved;
    }
    let name = reference
        .split('#')
        .next_back()
        .unwrap_or(reference)
        .trim_start_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(reference);
    name.rsplit('#')
        .next()
        .unwrap_or(name)
        .to_upper_camel_case()
}
