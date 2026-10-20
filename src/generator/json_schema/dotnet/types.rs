//! Classification of planned JSON Schema models into C# shapes: the type
//! lattice, field plans, `oneOf` unions and closed value sets. Rendering lives
//! in `emit`; this module only decides *what* is emitted.

use std::collections::{BTreeMap, BTreeSet};

use heck::ToUpperCamelCase;
use indexmap::IndexMap;
use serde_json::Value;

use crate::error::Result;
use crate::json_schema::content_encoding::Encoding;
use crate::json_schema::format::TemporalKind;
use crate::planning::PlannedJsonType;

use super::schema::{
    ArrayConstraints, Schema, StringConstraints, additional_properties_schema, allows_null,
    compose_doc, content_encoding_direct, decode_schema, is_union_schema, nullable_non_null_schema,
    shape_of, temporal_kind_direct, typed_map_value,
};

pub(super) const LIST: &str = "global::System.Collections.Generic.List";
pub(super) const READ_ONLY_LIST: &str = "global::System.Collections.Generic.IReadOnlyList";
pub(super) const DICTIONARY: &str = "global::System.Collections.Generic.Dictionary";
pub(super) const READ_ONLY_DICTIONARY: &str =
    "global::System.Collections.Generic.IReadOnlyDictionary";
pub(super) const JSON_ELEMENT: &str = "global::System.Text.Json.JsonElement";
pub(super) const JSON_VALUE_KIND: &str = "global::System.Text.Json.JsonValueKind";
/// The violation list every generated `Parse`/`Validate` collects into.
pub(super) fn violations_type() -> String {
    format!(
        "global::System.Collections.Generic.List<{}Violation>",
        super::runtime_prefix()
    )
}

/// Resolution context for one input module. Every model reference resolves
/// through `registry` (planned full name → namespace and type name) so a type
/// rename lands at every use site.
pub(super) struct CsContext<'a> {
    pub(super) namespace: String,
    pub(super) registry: &'a BTreeMap<String, (String, String)>,
    pub(super) all_models: &'a BTreeMap<String, PlannedJsonType>,
}

pub(super) fn strip_ref(reference: &str) -> &str {
    reference.strip_prefix("#/$defs/").unwrap_or(reference)
}

impl CsContext<'_> {
    /// The `(namespace, type)` a `$ref` names.
    pub(super) fn resolve_ref(&self, reference: &str) -> (String, String) {
        let key = strip_ref(reference);
        if let Some(resolved) = self
            .registry
            .get(key)
            .or_else(|| self.registry.get(reference))
        {
            return resolved.clone();
        }
        let name = key.rsplit(['/', '#']).next().unwrap_or(key);
        (self.namespace.clone(), name.to_upper_camel_case())
    }

    /// The C# spelling of a type in `namespace` as seen from this module.
    pub(super) fn qualify(&self, namespace: &str, name: &str) -> String {
        if namespace == self.namespace {
            name.to_string()
        } else {
            format!("global::{namespace}.{name}")
        }
    }

    pub(super) fn ref_type(&self, reference: &str) -> String {
        let (namespace, name) = self.resolve_ref(reference);
        self.qualify(&namespace, &name)
    }

    pub(super) fn model_schema(&self, reference: &str) -> Option<Schema> {
        self.all_models
            .get(strip_ref(reference))
            .and_then(|model| decode_schema(model).ok())
    }
}

#[derive(Debug, Clone)]
pub(super) enum CsType {
    Long,
    Double,
    Boolean,
    String,
    /// A reference to another generated model or named `oneOf` interface;
    /// either parses, validates and writes through its own `Converter`.
    Ref {
        name: String,
    },
    /// A list of the element type; the flag is set when an element may be
    /// `null`, so a nested list keeps its elements' nullability.
    List(Box<CsType>, bool),
    /// An inline property-level union, nested in its declaring model.
    Union {
        name: String,
    },
    Temporal(TemporalKind),
    Bytes(Encoding),
    /// A `const`/`enum` member rendered as a nested closed value class.
    ClosedValue {
        name: String,
        wire: Box<CsType>,
    },
}

impl CsType {
    pub(super) fn name(&self) -> String {
        match self {
            CsType::Long => "long".to_string(),
            CsType::Double => "double".to_string(),
            CsType::Boolean => "bool".to_string(),
            CsType::String => "string".to_string(),
            CsType::Ref { name } | CsType::Union { name } | CsType::ClosedValue { name, .. } => {
                name.clone()
            }
            CsType::List(element, nullable) => {
                format!("{READ_ONLY_LIST}<{}>", element.element_name(*nullable))
            }
            CsType::Temporal(kind) => temporal_type(*kind).to_string(),
            CsType::Bytes(_) => "byte[]".to_string(),
        }
    }

    /// True when the C# type is a struct, so absence needs `Nullable<T>`.
    pub(super) fn is_value_type(&self) -> bool {
        match self {
            CsType::Long | CsType::Double | CsType::Boolean => true,
            CsType::Temporal(kind) => *kind != TemporalKind::Time,
            _ => false,
        }
    }

    /// The type as a nullable slot (`long?`, `string?`).
    pub(super) fn nullable_name(&self) -> String {
        format!("{}?", self.name())
    }

    /// An element type written inside a collection.
    pub(super) fn element_name(&self, nullable: bool) -> String {
        if nullable {
            self.nullable_name()
        } else {
            self.name()
        }
    }

    /// The concrete collection type a parse builds for `List(element)`.
    pub(super) fn list_builder(element: &CsType, nullable: bool) -> String {
        format!("{LIST}<{}>", element.element_name(nullable))
    }
}

/// The C# type a materialized temporal `format` maps to. `time` stays a
/// validated, canonicalized `string`: no BCL type holds both an
/// offset-bearing and an offset-less time of day.
pub(super) fn temporal_type(kind: TemporalKind) -> &'static str {
    match kind {
        TemporalKind::DateTime => "global::System.DateTimeOffset",
        TemporalKind::Date => "global::System.DateOnly",
        TemporalKind::Time => "string",
        TemporalKind::Duration => "global::System.TimeSpan",
    }
}

pub(super) fn cs_type_for(schema: &Schema, context: &CsContext) -> CsType {
    if let Some(reference) = &schema.reference {
        return CsType::Ref {
            name: context.ref_type(reference),
        };
    }
    if !is_union_schema(schema)
        && let Some(non_null) = nullable_non_null_schema(schema)
    {
        return cs_type_for(non_null, context);
    }
    if let Some(kind) = temporal_kind_direct(schema) {
        return CsType::Temporal(kind);
    }
    if let Some(encoding) = content_encoding_direct(schema) {
        return CsType::Bytes(encoding);
    }
    match schema.type_name() {
        Some("integer") => CsType::Long,
        Some("number") => CsType::Double,
        Some("boolean") => CsType::Boolean,
        Some("array") => match schema.items.as_deref() {
            Some(item) => CsType::List(Box::new(cs_type_for(item, context)), allows_null(item)),
            None => CsType::List(Box::new(CsType::String), false),
        },
        _ => CsType::String,
    }
}

/// The C# member name for a JSON property: the `x-cs-name` override, or the
/// PascalCase spelling of the wire name.
pub(super) fn member_name(json_name: &str, property: &Schema) -> String {
    property
        .x_cs_name
        .clone()
        .unwrap_or_else(|| json_name.to_upper_camel_case())
}

// ---------------------------------------------------------------------------
// Closed value sets (`const` / `enum`)
// ---------------------------------------------------------------------------

/// The verbatim value-constant overrides (`x-cs-const-name` /
/// `x-cs-enum-names`) a closed-value property carries.
#[derive(Debug, Clone, Default)]
pub(super) struct ClosedNameOverrides {
    is_const: bool,
    const_name: Option<String>,
    enum_names: Option<IndexMap<String, String>>,
}

impl ClosedNameOverrides {
    pub(super) fn from_property(schema: &Schema) -> Self {
        Self {
            is_const: schema.const_value.is_some(),
            const_name: schema.x_cs_const_name.clone(),
            enum_names: schema.x_cs_enum_names.clone(),
        }
    }

    fn get(&self, value: &Value) -> Option<&str> {
        if self.is_const {
            return self.const_name.as_deref();
        }
        let map = self.enum_names.as_ref()?;
        map.get(&enum_names_lookup_key(value)?).map(String::as_str)
    }
}

/// The `x-cs-enum-names` key for one closed value: its canonical JSON
/// spelling. Mirrors the loader's `enum_names_lookup_key`.
fn enum_names_lookup_key(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Bool(flag) => Some(flag.to_string()),
        Value::Number(number) => Some(crate::json_schema::scalar::value_token_decimal(number)),
        _ => None,
    }
}

/// The PascalCase token a scalar value contributes to its constant name.
fn closed_value_token(value: &Value) -> String {
    match value {
        Value::String(text) => text.to_upper_camel_case(),
        Value::Bool(flag) => if *flag { "True" } else { "False" }.to_string(),
        Value::Number(number) => crate::json_schema::scalar::value_token_decimal(number)
            .replace('-', "Neg")
            .replace('.', "_"),
        _ => String::new(),
    }
}

/// The class-scoped constant name for a closed value: the value's own token,
/// with no member-derived component (the value class already carries the
/// member). A token that does not begin with an ASCII letter — every number —
/// takes a `V` prefix.
pub(crate) fn closed_value_constant_name(override_name: Option<&str>, value: &Value) -> String {
    if let Some(name) = override_name {
        return name.to_string();
    }
    let token = closed_value_token(value);
    if matches!(value, Value::Number(_)) || !token.starts_with(|c: char| c.is_ascii_alphabetic()) {
        format!("V{token}")
    } else {
        token
    }
}

pub(super) fn closed_constant_name(overrides: &ClosedNameOverrides, value: &Value) -> String {
    closed_value_constant_name(overrides.get(value), value)
}

/// The nested value-class name for a closed-value member.
pub(crate) fn closed_value_class_name(member: &str) -> String {
    format!("{member}Value")
}

/// The nested interface name for an inline property-level union.
pub(crate) fn inline_union_name(member: &str) -> String {
    format!("{member}Union")
}

/// The computed accessor a default-bearing optional member synthesizes.
pub(crate) fn default_accessor_name(member: &str) -> String {
    format!("{member}OrDefault")
}

// ---------------------------------------------------------------------------
// `oneOf` unions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub(super) struct UnionVariant {
    /// The C# type of this branch: a wrapper (nested in a named union's
    /// interface, or beside an inline union's interface in the declaring
    /// model), a member model, or the owned free-form object wrapper.
    pub(super) class: String,
    pub(super) is_object: bool,
    /// The underlying value a wrapper holds; `None` for a model member.
    pub(super) underlying: Option<CsType>,
    /// The JSON value kind test that selects this branch.
    pub(super) kind_test: &'static str,
    pub(super) discriminant_value: Option<Value>,
    pub(super) member_full_name: Option<String>,
    /// True for the inline free-form object member, which the union wraps.
    pub(super) owned_object: bool,
    pub(super) nullable_items: bool,
    pub(super) label: String,
    pub(super) schema: Schema,
}

#[derive(Debug, Clone)]
pub(super) struct CsUnion {
    pub(super) interface: String,
    pub(super) nullable: bool,
    pub(super) discriminant: Option<String>,
    pub(super) variants: Vec<UnionVariant>,
}

impl CsUnion {
    pub(super) fn admissible(&self) -> String {
        self.variants
            .iter()
            .map(|variant| variant.label.clone())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn discriminator_const(property: &Schema) -> Option<Value> {
    if let Some(value) = &property.const_value {
        return Some(value.clone());
    }
    match &property.enum_values {
        Some(values) if values.len() == 1 => Some(values[0].clone()),
        _ => None,
    }
}

fn branch_discriminator_tags(object: &Schema) -> BTreeMap<String, Value> {
    let required: BTreeSet<&String> = object.required.iter().flatten().collect();
    let mut tags = BTreeMap::new();
    for (name, property) in object.properties.iter().flatten() {
        if required.contains(name)
            && let Some(value) = discriminator_const(property)
        {
            tags.insert(name.clone(), value);
        }
    }
    tags
}

/// Classifies a `oneOf` schema into a C# union. Returns `None` for the
/// nullability pattern (fewer than two non-null branches).
pub(super) fn classify_union(
    interface: &str,
    schema: &Schema,
    context: &CsContext,
) -> Option<CsUnion> {
    let branches = schema.one_of.as_ref()?;
    let mut nullable = false;
    let mut variants = Vec::new();
    let mut object_schemas = Vec::new();
    for branch in branches {
        let (member_schema, object_class, member_full) = match &branch.reference {
            Some(reference) => (
                context
                    .model_schema(reference)
                    .unwrap_or_else(|| branch.clone()),
                Some(context.ref_type(reference)),
                Some(strip_ref(reference).to_string()),
            ),
            None => (branch.clone(), None, None),
        };
        let wrapper =
            |suffix: &str, underlying: CsType, kind_test: &'static str, label: &str| UnionVariant {
                class: format!("{interface}{suffix}"),
                is_object: false,
                underlying: Some(underlying),
                kind_test,
                discriminant_value: None,
                member_full_name: None,
                owned_object: false,
                nullable_items: false,
                label: label.to_string(),
                schema: member_schema.clone(),
            };
        match member_schema.type_name() {
            Some("null") => nullable = true,
            Some("object") => {
                let owned_object = object_class.is_none();
                let class = object_class.unwrap_or_else(|| format!("{interface}Object"));
                object_schemas.push(member_schema.clone());
                variants.push(UnionVariant {
                    label: if owned_object {
                        "object".to_string()
                    } else {
                        class.rsplit('.').next().unwrap_or(&class).to_string()
                    },
                    class,
                    is_object: true,
                    underlying: None,
                    kind_test: "Object",
                    discriminant_value: None,
                    member_full_name: member_full,
                    owned_object,
                    nullable_items: false,
                    schema: member_schema.clone(),
                });
            }
            Some("string") => variants.push(wrapper("String", CsType::String, "String", "string")),
            Some("integer") => variants.push(wrapper("Integer", CsType::Long, "Number", "integer")),
            Some("number") => variants.push(wrapper("Number", CsType::Double, "Number", "number")),
            Some("boolean") => {
                variants.push(wrapper("Boolean", CsType::Boolean, "Boolean", "boolean"))
            }
            Some("array") => {
                let item = member_schema
                    .items
                    .as_deref()
                    .map(|item| cs_type_for(item, context))
                    .unwrap_or(CsType::String);
                let nullable_items = member_schema.items.as_deref().is_some_and(allows_null);
                let mut variant = wrapper(
                    "Array",
                    CsType::List(Box::new(item), nullable_items),
                    "Array",
                    "array",
                );
                variant.nullable_items = nullable_items;
                variants.push(variant);
            }
            _ => {}
        }
    }
    if variants.len() < 2 {
        return None;
    }
    let mut discriminant = None;
    if object_schemas.len() >= 2 {
        let mut shared: Option<BTreeMap<String, Value>> = None;
        for object in &object_schemas {
            let tags = branch_discriminator_tags(object);
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
                    .filter_map(|object| branch_discriminator_tags(object).get(*name).cloned())
                    .collect();
                values
                    .iter()
                    .enumerate()
                    .all(|(index, value)| !values[..index].contains(value))
            })
            .cloned();
        if let Some(name) = &name {
            let mut object_index = 0;
            for variant in variants.iter_mut().filter(|variant| variant.is_object) {
                variant.discriminant_value =
                    branch_discriminator_tags(&object_schemas[object_index])
                        .get(name)
                        .cloned();
                object_index += 1;
            }
        }
        discriminant = name;
    }
    Some(CsUnion {
        interface: interface.to_string(),
        nullable,
        discriminant,
        variants,
    })
}

/// The union a schema `$ref`s, when its target is a named `oneOf` model.
pub(super) fn ref_union(schema: &Schema, context: &CsContext) -> Option<CsUnion> {
    let reference = schema.reference.as_ref()?;
    let target = context.model_schema(reference)?;
    if !is_union_schema(&target) {
        return None;
    }
    classify_union(&context.ref_type(reference), &target, context)
}

// ---------------------------------------------------------------------------
// Object models
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub(super) struct FieldPlan {
    pub(super) json_name: String,
    pub(super) name: String,
    pub(super) ty: CsType,
    pub(super) required: bool,
    pub(super) nullable: bool,
    pub(super) closed_values: Vec<Value>,
    pub(super) closed_overrides: ClosedNameOverrides,
    pub(super) default: Option<Value>,
    pub(super) doc: Option<String>,
    pub(super) deprecated: bool,
    pub(super) string: StringConstraints,
    pub(super) array: ArrayConstraints,
    /// The member's shape (nullability wrapper stripped).
    pub(super) schema: Schema,
    pub(super) nullable_items: bool,
    /// The inline union this member declares, nested in the declaring model.
    pub(super) union: Option<CsUnion>,
    /// True for an inline union nested in the declaring model.
    pub(super) nested_union: bool,
}

impl FieldPlan {
    /// The declared property type: absent or null needs a nullable slot.
    pub(super) fn declared_type(&self) -> String {
        let base = match &self.ty {
            CsType::List(element, _) => {
                format!(
                    "{READ_ONLY_LIST}<{}>",
                    element.element_name(self.nullable_items)
                )
            }
            other => other.name(),
        };
        if self.required && !self.nullable {
            base
        } else {
            format!("{base}?")
        }
    }

    /// True when the member is stored without a nullable wrapper and so is
    /// always present in memory.
    pub(super) fn always_present_value(&self) -> bool {
        self.required && !self.nullable && self.ty.is_value_type()
    }
}

#[derive(Debug, Clone)]
pub(super) struct TypedAdditionalPlan {
    pub(super) ty: CsType,
    pub(super) schema: Schema,
    pub(super) nullable: bool,
}

impl TypedAdditionalPlan {
    pub(super) fn value_type(&self) -> String {
        self.ty.element_name(self.nullable)
    }
}

pub(super) enum ModelKind {
    Object {
        open: bool,
        fields: Vec<FieldPlan>,
        typed_additional: Option<TypedAdditionalPlan>,
    },
    TypedMap(TypedAdditionalPlan),
}

fn additional_plan(authored: Schema, context: &CsContext) -> TypedAdditionalPlan {
    let shape = shape_of(&authored).clone();
    TypedAdditionalPlan {
        ty: cs_type_for(&shape, context),
        nullable: allows_null(&authored),
        schema: shape,
    }
}

pub(super) fn resolve_model_kind(schema: &Schema, context: &CsContext) -> Result<ModelKind> {
    if typed_map_value(schema).is_some() {
        let authored = additional_properties_schema(schema)?.unwrap_or_default();
        return Ok(ModelKind::TypedMap(additional_plan(authored, context)));
    }
    let open = schema.additional_properties.as_ref() != Some(&Value::Bool(false));
    let typed_additional =
        additional_properties_schema(schema)?.map(|authored| additional_plan(authored, context));
    let required: BTreeSet<&String> = schema.required.iter().flatten().collect();
    let mut fields = Vec::new();
    for (json_name, property) in schema.properties.iter().flatten() {
        let shape = shape_of(property);
        let closed_values = shape.closed_values();
        let name = member_name(json_name, property);
        let mut nested_union = false;
        let (ty, union) = if is_union_schema(property) {
            nested_union = true;
            let interface = inline_union_name(&name);
            (
                CsType::Union {
                    name: interface.clone(),
                },
                classify_union(&interface, property, context),
            )
        } else if ref_union(shape, context).is_some() {
            (cs_type_for(property, context), None)
        } else if !closed_values.is_empty() {
            let wire = cs_type_for(property, context);
            if matches!(wire, CsType::Temporal(_) | CsType::Bytes(_)) {
                (wire, None)
            } else {
                (
                    CsType::ClosedValue {
                        name: closed_value_class_name(&name),
                        wire: Box::new(wire),
                    },
                    None,
                )
            }
        } else {
            (cs_type_for(property, context), None)
        };
        fields.push(FieldPlan {
            json_name: json_name.clone(),
            name,
            ty,
            required: required.contains(json_name),
            nullable: allows_null(property),
            closed_values,
            closed_overrides: ClosedNameOverrides::from_property(shape),
            default: property.default.clone(),
            doc: compose_doc(property.title.as_deref(), property.description.as_deref()),
            deprecated: property.deprecated == Some(true),
            string: StringConstraints::from_schema(shape),
            array: ArrayConstraints::from_schema(shape),
            schema: shape.clone(),
            nullable_items: shape.items.as_deref().is_some_and(allows_null),
            union,
            nested_union,
        });
    }
    Ok(ModelKind::Object {
        open,
        fields,
        typed_additional,
    })
}
