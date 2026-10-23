//! The C# (.NET) JSON Schema model backend.
//!
//! Each planned JSON Schema model becomes a `sealed record` (or, for a `oneOf`
//! sum type, an interface) with a nested `JsonConverter` that parses, validates
//! and writes it. The design is pinned in `specs/json-schema/PRINCIPLES.md`
//! §C#; its structure follows the Java backend (`json_schema/java.rs`): a
//! collecting deserializer, spec-strict number parsing, and validate-then-write.
//!
//! The backend plugs into the C# generator through [`ExternalModelBackend`],
//! which renders services and file layout. Models need the whole tree, not one
//! leaf, because a `$ref` resolves across input files; [`JsonModelTree`] is
//! built once per run and shared by every leaf.

mod checks;
mod emit;
mod runtime;
mod schema;
mod types;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;

use crate::error::Result;
use crate::generator::ExternalModelBackend;
use crate::generator::dotnet::{OBSOLETE_WARNING_PRAGMA, WireValueConversion};
use crate::generator::json_schema::bare_ref_target;
use crate::language::Language;
use crate::parser::{ManifestModel, build_name_manifest};
use crate::planning::{PlannedFamily, PlannedJsonType, PlannedSpec};
use crate::spec::{ModulePath, RecordSpec};

pub(crate) use types::{
    closed_value_class_name, closed_value_constant_name, default_accessor_name, inline_union_name,
};

pub(in crate::generator) const GENERATED_CODE_ATTRIBUTE: &str =
    "[global::System.CodeDom.Compiler.GeneratedCode(\"nexgen\", null)]";

thread_local! {
    /// The namespace the runtime support types live in for the run currently
    /// rendering. Generated model code names them fully qualified
    /// (`global::<namespace>.Violation`), which no member, nested type, or local
    /// can capture. Model rendering is single-threaded per file, so a
    /// thread-local avoids threading the namespace through every emitter.
    static RUNTIME_NAMESPACE: RefCell<String> = const { RefCell::new(String::new()) };
}

/// The `global::`-qualified prefix generated model code spells a runtime
/// support type with: `global::Nexgen.JsonSchema.Chat.` before `Violation`.
pub(super) fn runtime_prefix() -> String {
    RUNTIME_NAMESPACE.with(|namespace| {
        let namespace = namespace.borrow();
        assert!(
            !namespace.is_empty(),
            "the C# runtime namespace is set while models render"
        );
        format!("global::{namespace}.")
    })
}

/// The runtime support types. They are reserved identifiers in every C# module
/// scope (P15): the runtime namespace declares them alongside a module's types.
pub(crate) const RUNTIME_TYPES: &[&str] = &[
    "Violation",
    "SpecJson",
    "ValueEquality",
    "TemporalSupport",
    "Base64Support",
];

/// The members every generated `JsonConverter<T>` declares or inherits. A
/// converter names the same-module types it parses, validates and writes by
/// simple name, so a type spelled like one of these would bind to the member
/// instead (CS0119/CS0120). They are reserved type names in every C# module
/// scope (P15). A *member* may still use most of them, being always accessed
/// through an instance; [`OBJECT_MEMBERS`] are the exceptions.
pub(crate) const CONVERTER_MEMBERS: &[&str] = &[
    "CanConvert",
    "GetType",
    "HandleNull",
    "MemberwiseClone",
    "Parse",
    "Read",
    "ReadAsPropertyName",
    "ReferenceEquals",
    "Type",
    "Validate",
    "Write",
    "WriteAsPropertyName",
    "WriteValue",
];

/// The non-overridden members every C# type inherits from `object`. A generated
/// record or closed-value class member spelled like one would hide it (CS0108,
/// an error under the generated project's warnings-as-errors), so they are
/// reserved in every model's member scope and every closed-value constant scope
/// (P15). The overridden `Equals`, `GetHashCode` and `ToString` are reserved as
/// generated members instead.
pub(crate) const OBJECT_MEMBERS: &[&str] = &["GetType", "MemberwiseClone", "ReferenceEquals"];

fn is_deprecated(schema: &serde_json::Value) -> bool {
    schema.get("deprecated") == Some(&serde_json::Value::Bool(true))
}

/// A C# string literal, escaping everything a regular literal cannot hold.
pub(in crate::generator) fn cs_string(value: &str) -> String {
    let mut output = String::with_capacity(value.len() + 2);
    output.push('"');
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\0' => output.push_str("\\0"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            other
                if (other as u32) < 0x20
                    || ('\u{7f}'..='\u{9f}').contains(&other)
                    || other == '\u{2028}'
                    || other == '\u{2029}' =>
            {
                output.push_str(&format!("\\u{:04X}", other as u32));
            }
            other => output.push(other),
        }
    }
    output.push('"');
    output
}

/// Every JSON Schema model in one generation run, with the C# namespace and
/// type name each resolves to.
#[derive(Debug, Default)]
pub(in crate::generator) struct JsonModelTree {
    models: BTreeMap<String, PlannedJsonType>,
    /// Planned full name → (namespace, type name), with bare-ref aliases
    /// resolved to the model that owns the type.
    registry: BTreeMap<String, (String, String)>,
    runtime_namespace: String,
    uses_temporal: bool,
    uses_content_encoding: bool,
}

impl JsonModelTree {
    pub(in crate::generator) fn new(
        models: BTreeMap<String, PlannedJsonType>,
        runtime_namespace: String,
        namespace_for: &dyn Fn(&ModulePath) -> String,
    ) -> Result<Self> {
        let manifest_models: Vec<ManifestModel> = models
            .values()
            .map(|json| ManifestModel {
                full_name: json.full_name.clone(),
                local_name: json
                    .full_name
                    .rsplit(['#', '/'])
                    .next()
                    .unwrap_or(&json.full_name)
                    .to_string(),
                model_name: json.model_name.clone(),
                module_key: json
                    .module_path
                    .as_ref()
                    .map(ModulePath::as_module_key)
                    .unwrap_or_default(),
                schema: json.schema.clone(),
            })
            .collect();
        let manifest = build_name_manifest(Language::Dotnet, &manifest_models, &[])?;
        let root = ModulePath::default();
        let mut registry = BTreeMap::new();
        for (full_name, model) in &models {
            let target = resolve_bare_ref_alias(model, &models);
            let namespace = namespace_for(target.module_path.as_ref().unwrap_or(&root));
            let name = manifest
                .type_name(&target.full_name)
                .unwrap_or(&target.model_name)
                .to_string();
            registry.insert(full_name.clone(), (namespace, name));
        }
        let uses = |direct: fn(&schema::Schema) -> bool| {
            models.values().any(|model| {
                schema::decode_schema(model)
                    .is_ok_and(|schema| schema::schema_uses_feature(&schema, direct))
            })
        };
        let uses_temporal = uses(|schema| schema::temporal_kind_direct(schema).is_some());
        let uses_content_encoding =
            uses(|schema| schema::content_encoding_direct(schema).is_some());
        Ok(Self {
            models,
            registry,
            runtime_namespace,
            uses_temporal,
            uses_content_encoding,
        })
    }

    pub(in crate::generator) fn is_empty(&self) -> bool {
        self.models.is_empty()
    }

    /// The runtime support files, emitted once at the output root.
    pub(in crate::generator) fn runtime_files(&self) -> Vec<(PathBuf, String)> {
        if self.is_empty() {
            return Vec::new();
        }
        runtime::files(
            &self.runtime_namespace,
            self.uses_temporal,
            self.uses_content_encoding,
        )
    }

    fn context(&self, namespace: &str) -> types::CsContext<'_> {
        types::CsContext {
            namespace: namespace.to_string(),
            registry: &self.registry,
            all_models: &self.models,
        }
    }
}

/// Resolves an exact bare-ref model alias to the model that owns the type. C#
/// emits no alias declaration, so references use the final target directly.
fn resolve_bare_ref_alias<'a>(
    model: &'a PlannedJsonType,
    all_models: &'a BTreeMap<String, PlannedJsonType>,
) -> &'a PlannedJsonType {
    let mut current = model;
    for _ in 0..=all_models.len() {
        let Some(reference) = bare_ref_target(current) else {
            break;
        };
        let Some(target) = all_models.get(types::strip_ref(reference)) else {
            break;
        };
        current = target;
    }
    current
}

/// The JSON Schema model backend for one C# module (input file).
#[derive(Debug, Default)]
pub(in crate::generator) struct ModelBackend {
    tree: Rc<JsonModelTree>,
    namespace: String,
    models: Vec<PlannedJsonType>,
}

#[derive(Debug, Default)]
pub(in crate::generator) struct RenderedModelFragments {
    pub(in crate::generator) body: String,
}

impl RenderedModelFragments {
    pub(in crate::generator) fn has_models(&self) -> bool {
        !self.body.is_empty()
    }
}

impl ModelBackend {
    pub(in crate::generator) fn new(tree: Rc<JsonModelTree>, namespace: String) -> Self {
        Self {
            tree,
            namespace,
            models: Vec::new(),
        }
    }

    /// Whether any model in the run is `deprecated`. A file that names models
    /// it does not declare (the service file) then has to suppress the
    /// obsolete warnings its own references raise, as the model files do.
    pub(in crate::generator) fn has_deprecated_models(&self) -> bool {
        self.tree
            .models
            .values()
            .any(|model| is_deprecated(&model.schema))
    }

    /// Whether any model or model property in the run is deprecated. A model
    /// file refers to a deprecated member in its own converter and to a
    /// deprecated type from any module, so this is decided for the whole run.
    fn has_deprecations(&self) -> bool {
        self.tree.models.values().any(|model| {
            is_deprecated(&model.schema)
                || model
                    .schema
                    .get("properties")
                    .and_then(serde_json::Value::as_object)
                    .is_some_and(|properties| properties.values().any(is_deprecated))
        })
    }

    fn type_name(&self, json_type: &PlannedJsonType) -> String {
        let context = self.tree.context(&self.namespace);
        match self.tree.registry.get(&json_type.full_name) {
            Some((namespace, name)) => context.qualify(namespace, name),
            None => json_type.model_name.clone(),
        }
    }

    /// The unions (named, or inline on another model's property) that list
    /// `model` as an object member: the interfaces its record implements.
    fn implemented_unions(&self, model: &PlannedJsonType) -> Vec<String> {
        let context = self.tree.context(&self.namespace);
        let mut implements = Vec::new();
        for other in self.tree.models.values() {
            let Ok(other_schema) = schema::decode_schema(other) else {
                continue;
            };
            let holds = |union: &types::CsUnion| {
                union.variants.iter().any(|variant| {
                    variant.member_full_name.as_deref() == Some(model.full_name.as_str())
                })
            };
            let Some((namespace, other_name)) = self.tree.registry.get(&other.full_name) else {
                continue;
            };
            let other_type = context.qualify(namespace, other_name);
            if schema::is_union_schema(&other_schema) {
                if types::classify_union(&other_type, &other_schema, &context)
                    .is_some_and(|union| holds(&union))
                {
                    implements.push(other_type);
                }
                continue;
            }
            for (json_name, property) in other_schema.properties.iter().flatten() {
                if !schema::is_union_schema(property) {
                    continue;
                }
                let interface = types::inline_union_name(&types::member_name(json_name, property));
                if types::classify_union(&interface, property, &context)
                    .is_some_and(|union| holds(&union))
                {
                    implements.push(format!("{other_type}.{interface}"));
                }
            }
        }
        implements.sort();
        implements.dedup();
        implements
    }

    fn render_model(&self, out: &mut String, model: &PlannedJsonType) -> Result<()> {
        let schema = schema::decode_schema(model)?;
        let context = self.tree.context(&self.namespace);
        let name = self
            .tree
            .registry
            .get(&model.full_name)
            .map(|(_, name)| name.clone())
            .unwrap_or_else(|| model.model_name.clone());
        if schema::is_union_schema(&schema)
            && let Some(union) = types::classify_union(&name, &schema, &context)
        {
            emit::render_union_interface(out, &union, "", Some((&schema, &name)), true);
            return Ok(());
        }
        let kind = types::resolve_model_kind(&schema, &context)?;
        let implements = self.implemented_unions(model);
        emit::render_object(
            out,
            &emit::ObjectModel {
                name: &name,
                schema: &schema,
                implements: &implements,
            },
            &kind,
        );
        Ok(())
    }
}

impl ExternalModelBackend<PlannedJsonType> for ModelBackend {
    type ModelFragments = RenderedModelFragments;
    type WireConversion = WireValueConversion;

    fn prepare(&mut self, api_plan: &PlannedSpec) -> Result<()> {
        self.models = api_plan
            .external_types()
            .filter_map(|(_, binding)| binding.json_model().cloned())
            .filter(|model| {
                resolve_bare_ref_alias(model, &self.tree.models).full_name == model.full_name
            })
            .collect();
        Ok(())
    }

    fn render_models(&self) -> Result<RenderedModelFragments> {
        if self.models.is_empty() {
            return Ok(RenderedModelFragments::default());
        }
        RUNTIME_NAMESPACE.with(|namespace| {
            namespace
                .borrow_mut()
                .clone_from(&self.tree.runtime_namespace)
        });
        let mut body = String::new();
        if self.has_deprecations() {
            body.push_str(OBSOLETE_WARNING_PRAGMA);
        }
        for (index, model) in self.models.iter().enumerate() {
            if index > 0 {
                body.push('\n');
            }
            self.render_model(&mut body, model)?;
        }
        Ok(RenderedModelFragments { body })
    }

    fn model_type_annotation(&self, json_type: &PlannedJsonType) -> Option<String> {
        Some(self.type_name(json_type))
    }

    fn wire_conversion(
        &self,
        json_type: &PlannedJsonType,
        _planned_record: Option<&RecordSpec<PlannedFamily>>,
    ) -> Option<WireValueConversion> {
        Some(WireValueConversion {
            annotation: self.type_name(json_type),
            to_wire: "{value}".to_string(),
        })
    }
}
