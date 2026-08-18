use std::collections::BTreeMap;
use std::path::PathBuf;

use indexmap::IndexMap;

use crate::language::Language;

#[derive(Debug, Clone, PartialEq)]
pub struct ApiSpec<F: TypeFamily = AuthoredFamily> {
    pub module_path: ModulePath,
    pub data: F::SpecData,
    pub version: String,
    pub support: F::Support,
    pub services: Vec<ServiceSpec<F>>,
    pub types: BTreeMap<String, TypeDeclEntry<F>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ModulePath(pub Vec<String>);

impl ModulePath {
    pub fn child(&self, segment: impl Into<String>) -> Self {
        let mut segments = self.0.clone();
        segments.push(segment.into());
        Self(segments)
    }

    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    pub fn to_path_buf(&self) -> PathBuf {
        self.0.iter().collect()
    }

    pub fn as_module_key(&self) -> String {
        self.0.join("/")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Symbol {
    name: String,
    module_path: Option<ModulePath>,
    local_name: String,
}

impl Symbol {
    pub fn new(value: impl Into<String>) -> Self {
        let value = value.into();
        Self {
            name: value.clone(),
            module_path: None,
            local_name: value,
        }
    }

    pub fn qualified(
        module_path: ModulePath,
        name: impl Into<String>,
        local_name: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            module_path: Some(module_path),
            local_name: local_name.into(),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.name
    }

    pub fn local_name(&self) -> &str {
        &self.local_name
    }

    pub fn module_path(&self) -> Option<&ModulePath> {
        self.module_path.as_ref()
    }
}

impl AsRef<str> for Symbol {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Display for Symbol {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.name.fmt(formatter)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AuthoredResourceType {
    pub name: Symbol,
    pub wire_type: Option<ExternalTypeSpec<AuthoredFamily>>,
    /// The named type alias through which this resource is referenced.
    ///
    /// A resource return is lowered into its wire-facing result model. Keeping
    /// the alias here lets that model retain its authored name and be shared
    /// by every operation that returns the same alias.
    pub alias: Option<DeclaredTypeName>,
}

impl AuthoredResourceType {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: Symbol::new(name),
            wire_type: None,
            alias: None,
        }
    }

    pub fn as_str(&self) -> &str {
        self.name.as_str()
    }
}

/// The authored and canonical names of a declared type.
///
/// `name` is used for generated code while `full_name` is the stable identity
/// used to correlate references to the declaration.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DeclaredTypeName {
    pub name: String,
    pub full_name: String,
}

impl AsRef<str> for AuthoredResourceType {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Display for AuthoredResourceType {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.name.fmt(formatter)
    }
}

pub trait TypeFamily {
    type SpecData: std::fmt::Debug + Clone + PartialEq;
    type Record: std::fmt::Debug + Clone + PartialEq;
    type Enum: std::fmt::Debug + Clone + PartialEq;
    type Flags: std::fmt::Debug + Clone + PartialEq;
    type Variant: std::fmt::Debug + Clone + PartialEq;
    type Resource: std::fmt::Debug + Clone + PartialEq;
    type Proto: std::fmt::Debug + Clone + PartialEq;
    type Json: std::fmt::Debug + Clone + PartialEq;
    type Alias: std::fmt::Debug + Clone + PartialEq;
    type ServiceData: std::fmt::Debug + Clone + PartialEq;
    type RecordData: std::fmt::Debug + Clone + PartialEq;
    type ResourceData: std::fmt::Debug + Clone + PartialEq;
    type OperationData: std::fmt::Debug + Clone + PartialEq;
    type FieldData: std::fmt::Debug + Clone + PartialEq;
    type Text: TextSpec;
    type Support: SupportSpecFamily;
}

/// Text metadata carried by an API-spec stage. `for_language` remains a small
/// compatibility shim for emitters during the migration; selected text ignores
/// its argument because selection has already happened.
pub trait TextSpec: std::fmt::Debug + Clone + PartialEq {
    fn for_language(&self, language: Language) -> Option<&str>;
    fn import_for_language(&self, language: Language) -> Option<&str>;
    fn is_empty(&self) -> bool;
}

pub trait SupportSpecFamily: std::fmt::Debug + Clone + PartialEq {
    fn fragments_for_language(&self, language: Language) -> &[SupportFragmentSpec];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthoredFamily;

impl TypeFamily for AuthoredFamily {
    type SpecData = ();
    type Record = Symbol;
    type Enum = Symbol;
    type Flags = Symbol;
    type Variant = Symbol;
    type Resource = AuthoredResourceType;
    type Proto = Symbol;
    type Json = JsonModelSpec<Symbol>;
    type Alias = Symbol;
    type ServiceData = ();
    type RecordData = ();
    type ResourceData = ();
    type OperationData = ();
    type FieldData = ();
    type Text = LanguageStringSpec;
    type Support = SupportSpec;
}

/// Name family after target-language selection. It preserves the `ApiSpec<F>`
/// shape while making per-language maps unrepresentable in selected IR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SelectedFamily;

impl TypeFamily for SelectedFamily {
    type SpecData = ();
    type Record = Symbol;
    type Enum = Symbol;
    type Flags = Symbol;
    type Variant = Symbol;
    type Resource = AuthoredResourceType;
    type Proto = Symbol;
    type Json = JsonModelSpec<Symbol>;
    type Alias = Symbol;
    type ServiceData = ();
    type RecordData = ();
    type ResourceData = ();
    type OperationData = ();
    type FieldData = ();
    type Text = SelectedTextSpec;
    type Support = SelectedSupportSpec;
}

/// Maps one structural `ApiSpec` family into another.
///
/// This is intentionally broader than names: a family transform also maps the
/// pass-specific data attached to specs, declarations, operations, and fields.
pub trait ApiSpecTransform<From: TypeFamily, To: TypeFamily> {
    fn map_spec_data(&mut self, data: From::SpecData) -> To::SpecData;
    fn map_record(&mut self, name: From::Record) -> To::Record;
    fn map_enum(&mut self, name: From::Enum) -> To::Enum;
    fn map_flags(&mut self, name: From::Flags) -> To::Flags;
    fn map_variant(&mut self, name: From::Variant) -> To::Variant;
    fn map_resource(&mut self, name: From::Resource) -> To::Resource;
    fn map_proto(&mut self, name: From::Proto) -> To::Proto;
    fn map_json(&mut self, name: From::Json) -> To::Json;
    fn map_alias(&mut self, name: From::Alias) -> To::Alias;
    fn map_service_data(&mut self, name: &str, data: From::ServiceData) -> To::ServiceData;
    fn map_record_data(&mut self, full_name: &str, data: From::RecordData) -> To::RecordData;
    fn map_resource_data(&mut self, name: &str, data: From::ResourceData) -> To::ResourceData;
    fn map_operation_data(&mut self, name: &str, data: From::OperationData) -> To::OperationData;
    fn map_field_data(
        &mut self,
        record_full_name: &str,
        field_name: &str,
        data: From::FieldData,
    ) -> To::FieldData;
    fn map_text(&mut self, text: From::Text) -> To::Text;
    fn map_support(&mut self, support: From::Support) -> To::Support;
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct LanguageImportSpec {
    pub language: Language,
    pub reference: String,
    pub module: String,
    pub name: Option<String>,
    pub type_only: bool,
    pub import_style: LanguageImportStyle,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum LanguageImportStyle {
    Module,
    Namespace,
    Named,
}

pub type AuthoredApiSpec = ApiSpec<AuthoredFamily>;
pub type AuthoredTypeSpec = TypeSpec<AuthoredFamily>;

impl<F: TypeFamily> ApiSpec<F> {
    pub fn external_type_binding(&self, type_name: &str) -> Option<&ExternalTypeBindingSpec<F>> {
        match self.types.get(type_name.trim_start_matches('.')) {
            Some(TypeDeclEntry {
                declaration: TypeDeclSpec::External(binding),
                ..
            }) => Some(binding),
            _ => None,
        }
    }

    pub fn external_types(&self) -> impl Iterator<Item = (&str, &ExternalTypeBindingSpec<F>)> {
        self.types
            .iter()
            .filter_map(|(name, entry)| match &entry.declaration {
                TypeDeclSpec::External(binding) => Some((name.as_str(), binding)),
                _ => None,
            })
    }

    pub fn records(&self) -> impl Iterator<Item = (&str, &RecordSpec<F>)> {
        self.types
            .iter()
            .filter_map(|(name, entry)| match &entry.declaration {
                TypeDeclSpec::Record(record) => Some((name.as_str(), record)),
                _ => None,
            })
    }

    pub fn record(&self, name: &str) -> Option<&RecordSpec<F>> {
        match self.types.get(name) {
            Some(TypeDeclEntry {
                declaration: TypeDeclSpec::Record(record),
                ..
            }) => Some(record),
            _ => None,
        }
    }

    pub fn enums(&self) -> impl Iterator<Item = (&str, &EnumSpec)> {
        self.types
            .iter()
            .filter_map(|(name, entry)| match &entry.declaration {
                TypeDeclSpec::Enum(enumeration) => Some((name.as_str(), enumeration)),
                _ => None,
            })
    }

    pub fn enum_decl(&self, name: &str) -> Option<&EnumSpec> {
        match self.types.get(name) {
            Some(TypeDeclEntry {
                declaration: TypeDeclSpec::Enum(enumeration),
                ..
            }) => Some(enumeration),
            _ => None,
        }
    }

    pub fn flags(&self) -> impl Iterator<Item = (&str, &FlagsSpec)> {
        self.types
            .iter()
            .filter_map(|(name, entry)| match &entry.declaration {
                TypeDeclSpec::Flags(flags) => Some((name.as_str(), flags)),
                _ => None,
            })
    }

    pub fn flags_decl(&self, name: &str) -> Option<&FlagsSpec> {
        match self.types.get(name) {
            Some(TypeDeclEntry {
                declaration: TypeDeclSpec::Flags(flags),
                ..
            }) => Some(flags),
            _ => None,
        }
    }

    pub fn variants(&self) -> impl Iterator<Item = (&str, &VariantSpec<F>)> {
        self.types
            .iter()
            .filter_map(|(name, entry)| match &entry.declaration {
                TypeDeclSpec::Variant(variant) => Some((name.as_str(), variant)),
                _ => None,
            })
    }

    pub fn variant(&self, name: &str) -> Option<&VariantSpec<F>> {
        match self.types.get(name) {
            Some(TypeDeclEntry {
                declaration: TypeDeclSpec::Variant(variant),
                ..
            }) => Some(variant),
            _ => None,
        }
    }

    pub fn map_names<G, M>(self, mut map: M) -> ApiSpec<G>
    where
        G: TypeFamily,
        M: ApiSpecTransform<F, G>,
    {
        self.map_names_with(&mut map)
    }

    fn map_names_with<G, M>(self, map: &mut M) -> ApiSpec<G>
    where
        G: TypeFamily,
        M: ApiSpecTransform<F, G>,
    {
        ApiSpec {
            module_path: self.module_path,
            data: map.map_spec_data(self.data),
            version: self.version,
            support: map.map_support(self.support),
            services: self
                .services
                .into_iter()
                .map(|service| service.map_names_with(map))
                .collect(),
            types: self
                .types
                .into_iter()
                .map(|(name, entry)| (name, entry.map_names_with(map)))
                .collect(),
        }
    }
}

impl<F> ApiSpec<F>
where
    F: TypeFamily,
    F::Proto: AsRef<str>,
{
    pub fn record_for_proto(&self, proto_name: &str) -> Option<&RecordSpec<F>> {
        let proto_name = proto_name.trim_start_matches('.');
        self.types.values().find_map(|entry| {
            let TypeDeclSpec::Record(record) = &entry.declaration else {
                return None;
            };
            matches!(
                record.source_type.as_ref(),
                Some(ExternalTypeSpec::Proto(source_proto))
                    if source_proto.as_ref() == proto_name
            )
            .then_some(record)
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypeDeclEntry<F: TypeFamily = AuthoredFamily> {
    pub declaration: TypeDeclSpec<F>,
    /// Whether this declaration is a public root of its containing module.
    pub(crate) module_exported: bool,
}

impl<F: TypeFamily> TypeDeclEntry<F> {
    pub fn new(declaration: TypeDeclSpec<F>) -> Self {
        Self {
            declaration,
            module_exported: false,
        }
    }

    pub(crate) fn module_export(declaration: TypeDeclSpec<F>) -> Self {
        Self {
            declaration,
            module_exported: true,
        }
    }

    fn map_names_with<G, M>(self, map: &mut M) -> TypeDeclEntry<G>
    where
        G: TypeFamily,
        M: ApiSpecTransform<F, G>,
    {
        TypeDeclEntry {
            declaration: self.declaration.map_names_with(map),
            module_exported: self.module_exported,
        }
    }
}

impl<F: TypeFamily> From<TypeDeclSpec<F>> for TypeDeclEntry<F> {
    fn from(declaration: TypeDeclSpec<F>) -> Self {
        Self::new(declaration)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypeDeclSpec<F: TypeFamily = AuthoredFamily> {
    External(ExternalTypeBindingSpec<F>),
    Record(RecordSpec<F>),
    Enum(EnumSpec),
    Flags(FlagsSpec),
    Variant(VariantSpec<F>),
}

impl<F: TypeFamily> TypeDeclSpec<F> {
    fn map_names_with<G, M>(self, map: &mut M) -> TypeDeclSpec<G>
    where
        G: TypeFamily,
        M: ApiSpecTransform<F, G>,
    {
        match self {
            TypeDeclSpec::External(binding) => TypeDeclSpec::External(binding.map_names_with(map)),
            TypeDeclSpec::Record(record) => TypeDeclSpec::Record(record.map_names_with(map)),
            TypeDeclSpec::Enum(enumeration) => TypeDeclSpec::Enum(enumeration),
            TypeDeclSpec::Flags(flags) => TypeDeclSpec::Flags(flags),
            TypeDeclSpec::Variant(variant) => TypeDeclSpec::Variant(variant.map_names_with(map)),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ServiceSpec<F: TypeFamily = AuthoredFamily> {
    pub name: String,
    /// A per-language verbatim override of the emitted service code identifier
    /// (`x-<lang>-name` on a JSON-schema `services:` entry). Empty for a language
    /// with no override, in which case the identifier is derived as usual. Never
    /// affects `wire_name`.
    pub code_name: F::Text,
    pub wire_name: String,
    pub doc: F::Text,
    pub namespace: F::Text,
    pub operations_class: F::Text,
    pub endpoint: Option<String>,
    pub experimental: bool,
    pub delay_load_temporalio_workflow: bool,
    pub operations: Vec<OperationSpec<F>>,
    pub resources: Vec<ResourceSpec<F>>,
    pub data: F::ServiceData,
}

impl<F: TypeFamily> ServiceSpec<F> {
    pub fn operation(&self, name: &str) -> Option<&OperationSpec<F>> {
        self.operations
            .iter()
            .find(|operation| operation.name == name)
    }

    pub fn resource(&self, name: &str) -> Option<&ResourceSpec<F>> {
        self.resources.iter().find(|resource| resource.name == name)
    }

    fn map_names_with<G, M>(self, map: &mut M) -> ServiceSpec<G>
    where
        G: TypeFamily,
        M: ApiSpecTransform<F, G>,
    {
        let data = map.map_service_data(&self.name, self.data);
        ServiceSpec {
            name: self.name,
            code_name: map.map_text(self.code_name),
            wire_name: self.wire_name,
            doc: map.map_text(self.doc),
            namespace: map.map_text(self.namespace),
            operations_class: map.map_text(self.operations_class),
            endpoint: self.endpoint,
            experimental: self.experimental,
            delay_load_temporalio_workflow: self.delay_load_temporalio_workflow,
            operations: self
                .operations
                .into_iter()
                .map(|operation| operation.map_names_with(map))
                .collect(),
            resources: self
                .resources
                .into_iter()
                .map(|resource| resource.map_names_with(map))
                .collect(),
            data,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SupportSpec {
    pub fragments: BTreeMap<Language, Vec<SupportFragmentSpec>>,
}

impl SupportSpec {
    pub fn fragments_for_language(&self, language: Language) -> &[SupportFragmentSpec] {
        self.fragments
            .get(&language)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
}

impl SupportSpecFamily for SupportSpec {
    fn fragments_for_language(&self, language: Language) -> &[SupportFragmentSpec] {
        self.fragments_for_language(language)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SelectedSupportSpec {
    pub(crate) fragments: Vec<SupportFragmentSpec>,
}

impl SupportSpecFamily for SelectedSupportSpec {
    fn fragments_for_language(&self, _language: Language) -> &[SupportFragmentSpec] {
        &self.fragments
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupportFragmentSpec {
    pub path: String,
    pub contents: String,
    pub namespace: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OperationSpec<F: TypeFamily = AuthoredFamily> {
    pub name: String,
    /// A per-language verbatim override of the emitted operation code identifier
    /// (`x-<lang>-name` on a JSON-schema `operations:` entry). Empty for a
    /// language with no override, in which case the identifier is derived as
    /// usual. Never affects `wire_name`.
    pub code_name: F::Text,
    pub wire_name: String,
    pub experimental: bool,
    pub doc: F::Text,
    pub return_doc: F::Text,
    pub input: Option<TypeSpec<F>>,
    pub output: Option<TypeSpec<F>>,
    pub output_transform: Option<OperationOutputTransformSpec<F>>,
    pub serialization_context: F::Text,
    pub data: F::OperationData,
}

impl<F: TypeFamily> OperationSpec<F> {
    pub fn input_type(&self) -> Option<&TypeSpec<F>> {
        self.input.as_ref()
    }

    pub fn output_type(&self) -> Option<&TypeSpec<F>> {
        self.output.as_ref()
    }

    pub fn output_transform(&self) -> Option<&OperationOutputTransformSpec<F>> {
        self.output_transform.as_ref()
    }

    fn map_names_with<G, M>(self, map: &mut M) -> OperationSpec<G>
    where
        G: TypeFamily,
        M: ApiSpecTransform<F, G>,
    {
        let data = map.map_operation_data(&self.name, self.data);
        OperationSpec {
            name: self.name,
            code_name: map.map_text(self.code_name),
            wire_name: self.wire_name,
            experimental: self.experimental,
            doc: map.map_text(self.doc),
            return_doc: map.map_text(self.return_doc),
            input: self.input.map(|input| input.map_names_with(map)),
            output: self.output.map(|output| output.map_names_with(map)),
            output_transform: self
                .output_transform
                .map(|transform| transform.map_names_with(map)),
            serialization_context: map.map_text(self.serialization_context),
            data,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResourceSpec<F: TypeFamily = AuthoredFamily> {
    pub name: String,
    pub fields: Vec<ResourceFieldSpec<F>>,
    pub methods: Vec<ResourceMethodSpec<F>>,
    pub data: F::ResourceData,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResourceFieldSpec<F: TypeFamily = AuthoredFamily> {
    pub name: String,
    pub optional: bool,
    pub field_type: TypeSpec<F>,
    pub function: Option<FunctionFieldSpec<F>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResourceMethodSpec<F: TypeFamily = AuthoredFamily> {
    pub name: String,
    pub params: Vec<ResourceFieldSpec<F>>,
    pub result: Option<ResourceResultSpec<F>>,
    pub operation_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResourceResultSpec<F: TypeFamily = AuthoredFamily> {
    pub result_type: TypeSpec<F>,
}

impl<F: TypeFamily> ResourceSpec<F> {
    fn map_names_with<G, M>(self, map: &mut M) -> ResourceSpec<G>
    where
        G: TypeFamily,
        M: ApiSpecTransform<F, G>,
    {
        let data = map.map_resource_data(&self.name, self.data);
        ResourceSpec {
            name: self.name,
            fields: self
                .fields
                .into_iter()
                .map(|field| field.map_names_with(map))
                .collect(),
            methods: self
                .methods
                .into_iter()
                .map(|method| method.map_names_with(map))
                .collect(),
            data,
        }
    }
}

impl<F: TypeFamily> ResourceFieldSpec<F> {
    fn map_names_with<G, M>(self, map: &mut M) -> ResourceFieldSpec<G>
    where
        G: TypeFamily,
        M: ApiSpecTransform<F, G>,
    {
        ResourceFieldSpec {
            name: self.name,
            optional: self.optional,
            field_type: self.field_type.map_names_with(map),
            function: self.function.map(|function| function.map_names_with(map)),
        }
    }
}

impl<F: TypeFamily> ResourceMethodSpec<F> {
    fn map_names_with<G, M>(self, map: &mut M) -> ResourceMethodSpec<G>
    where
        G: TypeFamily,
        M: ApiSpecTransform<F, G>,
    {
        ResourceMethodSpec {
            name: self.name,
            params: self
                .params
                .into_iter()
                .map(|param| param.map_names_with(map))
                .collect(),
            result: self.result.map(|result| result.map_names_with(map)),
            operation_name: self.operation_name,
        }
    }
}

impl<F: TypeFamily> ResourceResultSpec<F> {
    fn map_names_with<G, M>(self, map: &mut M) -> ResourceResultSpec<G>
    where
        G: TypeFamily,
        M: ApiSpecTransform<F, G>,
    {
        ResourceResultSpec {
            result_type: self.result_type.map_names_with(map),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RecordSpec<F: TypeFamily = AuthoredFamily> {
    pub name: String,
    pub full_name: String,
    pub doc: F::Text,
    pub source_type: Option<ExternalTypeSpec<F>>,
    pub experimental: bool,
    pub flatten_in_api: bool,
    pub fields: IndexMap<String, RecordFieldSpec<F>>,
    pub data: F::RecordData,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RecordFieldSpec<F: TypeFamily = AuthoredFamily> {
    pub name: String,
    pub doc: Option<F::Text>,
    pub annotation: Option<F::Text>,
    pub flattened_annotation: Option<F::Text>,
    pub field_type: TypeSpec<F>,
    pub default_value: Option<FieldDefaultSpec>,
    pub required: bool,
    pub visibility: RecordFieldVisibility,
    pub function: Option<FunctionFieldSpec<F>>,
    pub data: F::FieldData,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordFieldVisibility {
    Public,
    Omitted,
    Sourced { source_expr: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumSpec {
    pub name: String,
    pub full_name: String,
    pub values: Vec<EnumValueSpec>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumValueSpec {
    pub wire_name: String,
    pub name: String,
    pub number: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlagsSpec {
    pub name: String,
    pub full_name: String,
    pub flags: Vec<FlagSpec>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlagSpec {
    pub name: String,
    pub bit: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VariantSpec<F: TypeFamily = AuthoredFamily> {
    pub name: String,
    pub full_name: String,
    pub cases: Vec<VariantCaseSpec<F>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VariantCaseSpec<F: TypeFamily = AuthoredFamily> {
    pub name: String,
    pub wire_name: String,
    pub payload: Option<TypeSpec<F>>,
}

impl<F: TypeFamily> RecordSpec<F> {
    fn map_names_with<G, M>(self, map: &mut M) -> RecordSpec<G>
    where
        G: TypeFamily,
        M: ApiSpecTransform<F, G>,
    {
        let full_name = self.full_name;
        let data = map.map_record_data(&full_name, self.data);
        RecordSpec {
            name: self.name,
            full_name: full_name.clone(),
            doc: map.map_text(self.doc),
            source_type: self
                .source_type
                .map(|source_type| source_type.map_names_with(map)),
            experimental: self.experimental,
            flatten_in_api: self.flatten_in_api,
            fields: self
                .fields
                .into_iter()
                .map(|(name, field)| {
                    let field = field.map_names_with(&full_name, &name, map);
                    (name, field)
                })
                .collect(),
            data,
        }
    }

    pub fn doc(&self) -> &F::Text {
        &self.doc
    }

    pub fn public_fields(&self) -> impl Iterator<Item = (&str, &RecordFieldSpec<F>)> {
        self.fields
            .iter()
            .filter(|(_, field)| field.visibility == RecordFieldVisibility::Public)
            .map(|(name, field)| (name.as_str(), field))
    }

    pub fn sourced_fields(&self) -> impl Iterator<Item = (&str, &RecordFieldSpec<F>, &str)> {
        self.fields.iter().filter_map(|(name, field)| {
            let RecordFieldVisibility::Sourced { source_expr } = &field.visibility else {
                return None;
            };
            Some((name.as_str(), field, source_expr.as_str()))
        })
    }

    pub fn functions(&self) -> impl Iterator<Item = (&str, &FunctionFieldSpec<F>)> {
        self.fields.iter().filter_map(|(name, field)| {
            field
                .function
                .as_ref()
                .map(|function| (name.as_str(), function))
        })
    }

    pub fn is_empty_model(&self) -> bool {
        self.doc.is_empty()
            && self
                .fields
                .values()
                .all(|field| field.is_empty_model_field())
    }

    pub fn field_name_override(&self, field_name: &str) -> Option<&str> {
        self.fields.get(field_name).map(|field| field.name.as_str())
    }

    pub fn field_doc(&self, field_name: &str) -> Option<&F::Text> {
        self.fields
            .get(field_name)
            .and_then(|field| field.doc.as_ref())
    }

    pub fn field_annotation(&self, field_name: &str) -> Option<&F::Text> {
        self.fields
            .get(field_name)
            .and_then(|field| field.annotation.as_ref())
    }

    pub fn field_flattened_annotation(&self, field_name: &str) -> Option<&F::Text> {
        self.fields
            .get(field_name)
            .and_then(|field| field.flattened_annotation.as_ref())
    }

    pub fn field_type(&self, field_name: &str) -> Option<&TypeSpec<F>> {
        self.fields.get(field_name).map(|field| &field.field_type)
    }

    pub fn field_default(&self, field_name: &str) -> Option<&FieldDefaultSpec> {
        self.fields
            .get(field_name)
            .and_then(|field| field.default_value.as_ref())
    }

    pub fn field_source(&self, field_name: &str) -> Option<&str> {
        self.fields.get(field_name).and_then(|field| {
            let RecordFieldVisibility::Sourced { source_expr } = &field.visibility else {
                return None;
            };
            Some(source_expr.as_str())
        })
    }

    pub fn field_required(&self, field_name: &str) -> bool {
        self.fields
            .get(field_name)
            .is_some_and(|field| field.required)
    }

    pub fn field_omitted(&self, field_name: &str) -> bool {
        self.fields
            .get(field_name)
            .is_some_and(|field| field.visibility == RecordFieldVisibility::Omitted)
    }

    pub fn function(&self, field_name: &str) -> Option<&FunctionFieldSpec<F>> {
        self.fields
            .get(field_name)
            .and_then(|field| field.function.as_ref())
    }

    pub fn function_for_args_field(&self, field_name: &str) -> Option<&FunctionFieldSpec<F>> {
        self.fields.values().find_map(|field| {
            field.function.as_ref().filter(|function| {
                function
                    .arg_fields
                    .iter()
                    .any(|arg_field| arg_field == field_name)
            })
        })
    }
}

impl<F: TypeFamily> RecordFieldSpec<F> {
    fn map_names_with<G, M>(
        self,
        record_full_name: &str,
        field_name: &str,
        map: &mut M,
    ) -> RecordFieldSpec<G>
    where
        G: TypeFamily,
        M: ApiSpecTransform<F, G>,
    {
        let data = map.map_field_data(record_full_name, field_name, self.data);
        RecordFieldSpec {
            name: self.name,
            doc: self.doc.map(|text| map.map_text(text)),
            annotation: self.annotation.map(|text| map.map_text(text)),
            flattened_annotation: self.flattened_annotation.map(|text| map.map_text(text)),
            field_type: self.field_type.map_names_with(map),
            default_value: self.default_value,
            required: self.required,
            visibility: self.visibility,
            function: self.function.map(|function| function.map_names_with(map)),
            data,
        }
    }

    fn is_empty_model_field(&self) -> bool {
        self.name.is_empty()
            && self.doc.is_none()
            && self.annotation.is_none()
            && self.flattened_annotation.is_none()
            && self.default_value.is_none()
            && !self.required
            && self.visibility == RecordFieldVisibility::Public
            && self.function.is_none()
    }
}

impl<F: TypeFamily> VariantSpec<F> {
    fn map_names_with<G, M>(self, map: &mut M) -> VariantSpec<G>
    where
        G: TypeFamily,
        M: ApiSpecTransform<F, G>,
    {
        VariantSpec {
            name: self.name,
            full_name: self.full_name,
            cases: self
                .cases
                .into_iter()
                .map(|case| case.map_names_with(map))
                .collect(),
        }
    }
}

impl<F: TypeFamily> VariantCaseSpec<F> {
    fn map_names_with<G, M>(self, map: &mut M) -> VariantCaseSpec<G>
    where
        G: TypeFamily,
        M: ApiSpecTransform<F, G>,
    {
        VariantCaseSpec {
            name: self.name,
            wire_name: self.wire_name,
            payload: self.payload.map(|payload| payload.map_names_with(map)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationOutputTransformSpec<F: TypeFamily = AuthoredFamily> {
    pub type_name: F::Text,
    pub transform: F::Text,
}

impl<F: TypeFamily> OperationOutputTransformSpec<F> {
    fn map_names_with<G: TypeFamily, M: ApiSpecTransform<F, G>>(
        self,
        map: &mut M,
    ) -> OperationOutputTransformSpec<G> {
        OperationOutputTransformSpec {
            type_name: map.map_text(self.type_name),
            transform: map.map_text(self.transform),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LanguageStringSpec {
    pub default: Option<String>,
    pub by_language: BTreeMap<Language, String>,
    pub default_import: Option<String>,
    pub imports: BTreeMap<Language, String>,
}

impl LanguageStringSpec {
    pub fn for_language(&self, language: Language) -> Option<&str> {
        self.by_language
            .get(&language)
            .or(self.default.as_ref())
            .map(String::as_str)
    }

    pub fn import_for_language(&self, language: Language) -> Option<&str> {
        self.imports
            .get(&language)
            .or(self.default_import.as_ref())
            .map(String::as_str)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.default.is_none() && self.by_language.is_empty()
    }
}

impl TextSpec for LanguageStringSpec {
    fn for_language(&self, language: Language) -> Option<&str> {
        LanguageStringSpec::for_language(self, language)
    }

    fn import_for_language(&self, language: Language) -> Option<&str> {
        LanguageStringSpec::import_for_language(self, language)
    }

    fn is_empty(&self) -> bool {
        LanguageStringSpec::is_empty(self)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SelectedTextSpec {
    pub(crate) value: Option<String>,
    pub(crate) import: Option<String>,
}

impl TextSpec for SelectedTextSpec {
    fn for_language(&self, _language: Language) -> Option<&str> {
        self.value.as_deref()
    }

    fn import_for_language(&self, _language: Language) -> Option<&str> {
        self.import.as_deref()
    }

    fn is_empty(&self) -> bool {
        self.value.is_none()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExternalTypeBindingSpec<F: TypeFamily = AuthoredFamily> {
    pub external_type: ExternalTypeSpec<F>,
    pub reference: F::Text,
    pub type_name: F::Text,
    pub replacement: Option<TypeReplacementSpec<F>>,
    pub authored_type: Option<TypeSpec<F>>,
}

impl<F: TypeFamily> ExternalTypeBindingSpec<F> {
    pub fn type_name(&self) -> &F::Text {
        &self.type_name
    }

    pub fn reference(&self) -> &F::Text {
        &self.reference
    }

    pub fn replacement(&self) -> Option<&TypeReplacementSpec<F>> {
        self.replacement.as_ref()
    }

    fn map_names_with<G, M>(self, map: &mut M) -> ExternalTypeBindingSpec<G>
    where
        G: TypeFamily,
        M: ApiSpecTransform<F, G>,
    {
        ExternalTypeBindingSpec {
            external_type: self.external_type.map_names_with(map),
            reference: map.map_text(self.reference),
            type_name: map.map_text(self.type_name),
            replacement: self
                .replacement
                .map(|replacement| replacement.map_names_with(map)),
            authored_type: self
                .authored_type
                .map(|authored_type| authored_type.map_names_with(map)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeReplacementSpec<F: TypeFamily = AuthoredFamily> {
    pub type_name: F::Text,
    pub from_proto: F::Text,
    pub to_proto: F::Text,
}

impl<F: TypeFamily> TypeReplacementSpec<F> {
    fn map_names_with<G, M>(self, map: &mut M) -> TypeReplacementSpec<G>
    where
        G: TypeFamily,
        M: ApiSpecTransform<F, G>,
    {
        TypeReplacementSpec {
            type_name: map.map_text(self.type_name),
            from_proto: map.map_text(self.from_proto),
            to_proto: map.map_text(self.to_proto),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct JsonModelSpec<N> {
    pub name: N,
    pub model_name: String,
    pub schema: serde_json::Value,
}

impl<N: AsRef<str>> AsRef<str> for JsonModelSpec<N> {
    fn as_ref(&self) -> &str {
        self.name.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldDefaultSpec {
    pub enum_case: String,
    pub enum_value: i32,
}

/// A WIT alias marked with `@nexus.type-parameter`.
///
/// `full_name` is the stable alias identity used when correlating occurrences
/// through nested models and operation inputs/outputs. `name` is the emitted
/// language identifier derived from the authored alias name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TypeParameterSpec {
    pub name: String,
    pub full_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeParameterUsage {
    pub parameter: TypeParameterSpec,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypeSpec<F: TypeFamily = AuthoredFamily> {
    Bool,
    Int(IntSpec),
    Float,
    String,
    Bytes,
    Option(Box<TypeSpec<F>>),
    List(Box<TypeSpec<F>>),
    Tuple(Vec<TypeSpec<F>>),
    Map(Box<TypeSpec<F>>, Box<TypeSpec<F>>),
    Result {
        ok: Option<Box<TypeSpec<F>>>,
        err: Option<Box<TypeSpec<F>>>,
    },
    TypeParameter(TypeParameterSpec),
    Record(F::Record),
    Enum(F::Enum),
    Flags(F::Flags),
    Variant(F::Variant),
    Resource(F::Resource),
    External(ExternalTypeSpec<F>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntSpec {
    I32,
    I64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExternalTypeSpec<F: TypeFamily = AuthoredFamily> {
    Proto(F::Proto),
    Json(F::Json),
    Alias {
        name: F::Alias,
        target: Box<TypeSpec<F>>,
        type_name: F::Text,
    },
}

impl<F: TypeFamily> TypeSpec<F> {
    pub fn map_names<G, M>(self, mut map: M) -> TypeSpec<G>
    where
        G: TypeFamily,
        M: ApiSpecTransform<F, G>,
    {
        self.map_names_with(&mut map)
    }

    fn map_names_with<G, M>(self, map: &mut M) -> TypeSpec<G>
    where
        G: TypeFamily,
        M: ApiSpecTransform<F, G>,
    {
        match self {
            TypeSpec::Bool => TypeSpec::Bool,
            TypeSpec::Int(int) => TypeSpec::Int(int),
            TypeSpec::Float => TypeSpec::Float,
            TypeSpec::String => TypeSpec::String,
            TypeSpec::Bytes => TypeSpec::Bytes,
            TypeSpec::Option(inner) => TypeSpec::Option(Box::new(inner.map_names_with(map))),
            TypeSpec::List(inner) => TypeSpec::List(Box::new(inner.map_names_with(map))),
            TypeSpec::Tuple(items) => TypeSpec::Tuple(
                items
                    .into_iter()
                    .map(|item| item.map_names_with(map))
                    .collect(),
            ),
            TypeSpec::Map(key, value) => TypeSpec::Map(
                Box::new(key.map_names_with(map)),
                Box::new(value.map_names_with(map)),
            ),
            TypeSpec::Result { ok, err } => TypeSpec::Result {
                ok: ok.map(|ok| Box::new(ok.map_names_with(map))),
                err: err.map(|err| Box::new(err.map_names_with(map))),
            },
            TypeSpec::TypeParameter(parameter) => TypeSpec::TypeParameter(parameter),
            TypeSpec::Record(type_name) => TypeSpec::Record(map.map_record(type_name)),
            TypeSpec::Enum(type_name) => TypeSpec::Enum(map.map_enum(type_name)),
            TypeSpec::Flags(type_name) => TypeSpec::Flags(map.map_flags(type_name)),
            TypeSpec::Variant(type_name) => TypeSpec::Variant(map.map_variant(type_name)),
            TypeSpec::Resource(type_name) => TypeSpec::Resource(map.map_resource(type_name)),
            TypeSpec::External(external) => TypeSpec::External(external.map_names_with(map)),
        }
    }

    pub(crate) fn without_option(&self) -> &TypeSpec<F> {
        match self {
            TypeSpec::Option(inner) => inner.without_option(),
            _ => self,
        }
    }

    pub(crate) fn validation_type(&self) -> &TypeSpec<F> {
        match self {
            TypeSpec::External(ExternalTypeSpec::Alias { target, .. }) => target.validation_type(),
            _ => self,
        }
    }
}

impl<F> TypeSpec<F>
where
    F: TypeFamily,
    F::Record: AsRef<str>,
    F::Enum: AsRef<str>,
    F::Flags: AsRef<str>,
    F::Variant: AsRef<str>,
    F::Resource: AsRef<str>,
    F::Proto: AsRef<str>,
    F::Json: AsRef<str>,
    F::Alias: AsRef<str>,
{
    pub fn reference(&self) -> Option<&str> {
        match self {
            TypeSpec::Record(type_name) => Some(type_name.as_ref()),
            TypeSpec::Enum(type_name) => Some(type_name.as_ref()),
            TypeSpec::Flags(type_name) => Some(type_name.as_ref()),
            TypeSpec::Variant(type_name) => Some(type_name.as_ref()),
            TypeSpec::Resource(type_name) => Some(type_name.as_ref()),
            TypeSpec::External(external) => external.reference(),
            _ => None,
        }
    }

    pub(crate) fn to_type_string(&self) -> String {
        match self {
            TypeSpec::Bool => "bool".to_string(),
            TypeSpec::Int(IntSpec::I32) => "s32".to_string(),
            TypeSpec::Int(IntSpec::I64) => "s64".to_string(),
            TypeSpec::Float => "float64".to_string(),
            TypeSpec::String => "string".to_string(),
            TypeSpec::Bytes => "bytes".to_string(),
            TypeSpec::Option(inner) => {
                format!("option<{}>", inner.to_type_string())
            }
            TypeSpec::List(inner) => format!("list<{}>", inner.to_type_string()),
            TypeSpec::Tuple(items) => format!(
                "tuple<{}>",
                items
                    .iter()
                    .map(TypeSpec::to_type_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            TypeSpec::Map(key, value) => {
                format!("map<{}, {}>", key.to_type_string(), value.to_type_string())
            }
            TypeSpec::Result { ok, err } => match (ok, err) {
                (Some(ok), Some(err)) => {
                    format!("result<{}, {}>", ok.to_type_string(), err.to_type_string())
                }
                (Some(ok), None) => format!("result<{}>", ok.to_type_string()),
                (None, Some(err)) => format!("result<_, {}>", err.to_type_string()),
                (None, None) => "result".to_string(),
            },
            TypeSpec::TypeParameter(parameter) => parameter.name.clone(),
            TypeSpec::Record(type_name) => type_name.as_ref().to_string(),
            TypeSpec::Enum(type_name) => type_name.as_ref().to_string(),
            TypeSpec::Flags(type_name) => type_name.as_ref().to_string(),
            TypeSpec::Variant(type_name) => type_name.as_ref().to_string(),
            TypeSpec::Resource(type_name) => type_name.as_ref().to_string(),
            TypeSpec::External(external) => external.to_type_string(),
        }
    }
}

impl<F: TypeFamily> ExternalTypeSpec<F> {
    fn map_names_with<G, M>(self, map: &mut M) -> ExternalTypeSpec<G>
    where
        G: TypeFamily,
        M: ApiSpecTransform<F, G>,
    {
        match self {
            ExternalTypeSpec::Proto(type_name) => ExternalTypeSpec::Proto(map.map_proto(type_name)),
            ExternalTypeSpec::Json(type_name) => ExternalTypeSpec::Json(map.map_json(type_name)),
            ExternalTypeSpec::Alias {
                name,
                target,
                type_name,
            } => ExternalTypeSpec::Alias {
                name: map.map_alias(name),
                target: Box::new(target.map_names_with(map)),
                type_name: map.map_text(type_name),
            },
        }
    }
}

impl<F> ExternalTypeSpec<F>
where
    F: TypeFamily,
    F::Proto: AsRef<str>,
    F::Json: AsRef<str>,
    F::Alias: AsRef<str>,
{
    pub fn reference(&self) -> Option<&str> {
        match self {
            ExternalTypeSpec::Proto(type_name) => Some(type_name.as_ref()),
            ExternalTypeSpec::Json(type_name) => Some(type_name.as_ref()),
            ExternalTypeSpec::Alias { name, .. } => Some(name.as_ref()),
        }
    }

    pub(crate) fn to_type_string(&self) -> String {
        self.reference().unwrap_or_default().to_string()
    }
}

impl<F> ApiSpec<F>
where
    F: TypeFamily,
    F::Record: AsRef<str>,
    F::Variant: AsRef<str>,
{
    /// Returns the model parameters in first-use order. Language-specific
    /// field annotations are authoritative, so an overridden field does not
    /// contribute parameters for that language.
    pub(crate) fn record_type_parameters(
        &self,
        record_name: &str,
        language: Language,
    ) -> Vec<TypeParameterUsage> {
        let mut parameters = IndexMap::<String, TypeParameterUsage>::new();
        let mut visiting = std::collections::BTreeSet::new();
        self.collect_record_type_parameters(record_name, language, &mut visiting, &mut parameters);
        parameters.into_values().collect()
    }

    /// Returns a variant's parameters in first-use order, recursively walking
    /// case payload containers and referenced records/variants.
    pub(crate) fn variant_type_parameters(
        &self,
        variant_name: &str,
        language: Language,
    ) -> Vec<TypeParameterUsage> {
        let mut parameters = IndexMap::<String, TypeParameterUsage>::new();
        let mut visiting = std::collections::BTreeSet::new();
        self.collect_variant_type_parameters(
            variant_name,
            language,
            &mut visiting,
            &mut parameters,
        );
        parameters.into_values().collect()
    }

    pub(crate) fn type_parameters(
        &self,
        value: &TypeSpec<F>,
        language: Language,
    ) -> Vec<TypeParameterUsage> {
        let mut parameters = IndexMap::<String, TypeParameterUsage>::new();
        let mut visiting = std::collections::BTreeSet::new();
        self.collect_type_parameters(value, language, &mut visiting, &mut parameters);
        parameters.into_values().collect()
    }

    fn collect_record_type_parameters(
        &self,
        record_name: &str,
        language: Language,
        visiting: &mut std::collections::BTreeSet<String>,
        parameters: &mut IndexMap<String, TypeParameterUsage>,
    ) {
        if !visiting.insert(record_name.to_string()) {
            return;
        }
        if let Some(record) = self.record(record_name) {
            for field in record.fields.values() {
                if field.visibility == RecordFieldVisibility::Omitted
                    || field
                        .annotation
                        .as_ref()
                        .is_some_and(|annotation| annotation.for_language(language).is_some())
                {
                    continue;
                }
                self.collect_type_parameters(&field.field_type, language, visiting, parameters);
            }
        }
        visiting.remove(record_name);
    }

    fn collect_variant_type_parameters(
        &self,
        variant_name: &str,
        language: Language,
        visiting: &mut std::collections::BTreeSet<String>,
        parameters: &mut IndexMap<String, TypeParameterUsage>,
    ) {
        if !visiting.insert(variant_name.to_string()) {
            return;
        }
        if let Some(variant) = self.variant(variant_name) {
            for case in &variant.cases {
                if let Some(payload) = &case.payload {
                    self.collect_type_parameters(payload, language, visiting, parameters);
                }
            }
        }
        visiting.remove(variant_name);
    }

    fn collect_type_parameters(
        &self,
        value: &TypeSpec<F>,
        language: Language,
        visiting: &mut std::collections::BTreeSet<String>,
        parameters: &mut IndexMap<String, TypeParameterUsage>,
    ) {
        match value {
            TypeSpec::TypeParameter(parameter) => {
                if !parameters.contains_key(&parameter.full_name) {
                    parameters.insert(
                        parameter.full_name.clone(),
                        TypeParameterUsage {
                            parameter: parameter.clone(),
                        },
                    );
                }
            }
            TypeSpec::Option(inner) | TypeSpec::List(inner) => {
                self.collect_type_parameters(inner, language, visiting, parameters)
            }
            TypeSpec::Tuple(items) => {
                for item in items {
                    self.collect_type_parameters(item, language, visiting, parameters);
                }
            }
            TypeSpec::Map(key, value) => {
                self.collect_type_parameters(key, language, visiting, parameters);
                self.collect_type_parameters(value, language, visiting, parameters);
            }
            TypeSpec::Result { ok, err } => {
                if let Some(ok) = ok {
                    self.collect_type_parameters(ok, language, visiting, parameters);
                }
                if let Some(err) = err {
                    self.collect_type_parameters(err, language, visiting, parameters);
                }
            }
            TypeSpec::Record(record) => {
                self.collect_record_type_parameters(record.as_ref(), language, visiting, parameters)
            }
            TypeSpec::Variant(variant) => self.collect_variant_type_parameters(
                variant.as_ref(),
                language,
                visiting,
                parameters,
            ),
            TypeSpec::External(ExternalTypeSpec::Alias { target, .. }) => {
                self.collect_type_parameters(target, language, visiting, parameters)
            }
            TypeSpec::Bool
            | TypeSpec::Int(_)
            | TypeSpec::Float
            | TypeSpec::String
            | TypeSpec::Bytes
            | TypeSpec::Enum(_)
            | TypeSpec::Flags(_)
            | TypeSpec::Resource(_)
            | TypeSpec::External(ExternalTypeSpec::Proto(_))
            | TypeSpec::External(ExternalTypeSpec::Json(_)) => {}
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FunctionFieldSpec<F: TypeFamily = AuthoredFamily> {
    pub primary: bool,
    pub result: FunctionResultSpec<F>,
    pub args_field: String,
    pub arg_fields: Vec<String>,
    pub args: FunctionArgsSpec<F>,
    pub alternate_type: Option<TypeSpec<F>>,
    pub converter: Option<String>,
    pub name_extractor: Option<String>,
    pub call_extractor: Option<String>,
    pub result_type_parameter: Option<String>,
    pub type_descriptor: Option<FunctionTypeDescriptorSpec<F>>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FunctionArgsSpec<F: TypeFamily = AuthoredFamily> {
    Varargs {
        prefix: Vec<FunctionArgSpec<F>>,
        typescript_drop_prefix: bool,
    },
    Fixed(Vec<FunctionArgSpec<F>>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct FunctionArgSpec<F: TypeFamily = AuthoredFamily> {
    pub name: String,
    pub field_type: TypeSpec<F>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FunctionResultSpec<F: TypeFamily = AuthoredFamily> {
    Authored(TypeSpec<F>),
    Annotation(F::Text),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionTypeDescriptorSpec<F: TypeFamily = AuthoredFamily> {
    pub value_type: F::Text,
    pub args_type: F::Text,
}

impl<F: TypeFamily> FunctionTypeDescriptorSpec<F> {
    fn map_names_with<G, M>(self, map: &mut M) -> FunctionTypeDescriptorSpec<G>
    where
        G: TypeFamily,
        M: ApiSpecTransform<F, G>,
    {
        FunctionTypeDescriptorSpec {
            value_type: map.map_text(self.value_type),
            args_type: map.map_text(self.args_type),
        }
    }
}

impl<F: TypeFamily> FunctionFieldSpec<F> {
    fn map_names_with<G, M>(self, map: &mut M) -> FunctionFieldSpec<G>
    where
        G: TypeFamily,
        M: ApiSpecTransform<F, G>,
    {
        FunctionFieldSpec {
            primary: self.primary,
            result: self.result.map_names_with(map),
            args_field: self.args_field,
            arg_fields: self.arg_fields,
            args: self.args.map_names_with(map),
            alternate_type: self
                .alternate_type
                .map(|alternate_type| alternate_type.map_names_with(map)),
            converter: self.converter,
            name_extractor: self.name_extractor,
            call_extractor: self.call_extractor,
            result_type_parameter: self.result_type_parameter,
            type_descriptor: self
                .type_descriptor
                .map(|descriptor| descriptor.map_names_with(map)),
        }
    }
}

impl<F: TypeFamily> FunctionArgsSpec<F> {
    fn map_names_with<G, M>(self, map: &mut M) -> FunctionArgsSpec<G>
    where
        G: TypeFamily,
        M: ApiSpecTransform<F, G>,
    {
        match self {
            FunctionArgsSpec::Varargs {
                prefix,
                typescript_drop_prefix,
            } => FunctionArgsSpec::Varargs {
                prefix: prefix
                    .into_iter()
                    .map(|arg| arg.map_names_with(map))
                    .collect(),
                typescript_drop_prefix,
            },
            FunctionArgsSpec::Fixed(args) => FunctionArgsSpec::Fixed(
                args.into_iter()
                    .map(|arg| arg.map_names_with(map))
                    .collect(),
            ),
        }
    }
}

impl<F: TypeFamily> FunctionArgSpec<F> {
    fn map_names_with<G, M>(self, map: &mut M) -> FunctionArgSpec<G>
    where
        G: TypeFamily,
        M: ApiSpecTransform<F, G>,
    {
        FunctionArgSpec {
            name: self.name,
            field_type: self.field_type.map_names_with(map),
        }
    }
}

impl<F: TypeFamily> FunctionResultSpec<F> {
    fn map_names_with<G, M>(self, map: &mut M) -> FunctionResultSpec<G>
    where
        G: TypeFamily,
        M: ApiSpecTransform<F, G>,
    {
        match self {
            FunctionResultSpec::Authored(authored_type) => {
                FunctionResultSpec::Authored(authored_type.map_names_with(map))
            }
            FunctionResultSpec::Annotation(annotation) => {
                FunctionResultSpec::Annotation(map.map_text(annotation))
            }
        }
    }
}

/// A semantic compiler pass over an API-spec tree.
///
/// The shared tree traversal retains module structure and source metadata;
/// implementations provide the leaf-level transformation for the pass.
pub trait CompilerPass<From: TypeFamily, To: TypeFamily> {
    type Error;

    fn transform_leaf(&mut self, leaf: ApiSpecLeaf<From>) -> Result<ApiSpecLeaf<To>, Self::Error>;

    /// Apply this pass to the full tree.
    fn apply(self, tree: ApiSpecTree<From>) -> Result<ApiSpecTree<To>, Self::Error>
    where
        Self: Sized,
    {
        let mut pass = self;
        transform_tree(tree, &mut pass)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ApiSpecTree<F: TypeFamily = AuthoredFamily> {
    pub root: ApiSpecNode<F>,
}

impl ApiSpecTree<AuthoredFamily> {
    pub fn single(spec: ApiSpec) -> Self {
        Self {
            root: ApiSpecNode::Leaf(ApiSpecLeaf {
                module_path: ModulePath::default(),
                source_root: PathBuf::new(),
                source_path: PathBuf::new(),
                spec,
            }),
        }
    }

    pub fn into_single_spec(self) -> Option<ApiSpec> {
        match self.root {
            ApiSpecNode::Leaf(leaf) => Some(leaf.spec),
            ApiSpecNode::Branch(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ApiSpecNode<F: TypeFamily = AuthoredFamily> {
    Leaf(ApiSpecLeaf<F>),
    Branch(ApiSpecBranch<F>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ApiSpecBranch<F: TypeFamily = AuthoredFamily> {
    pub module_path: ModulePath,
    pub children: BTreeMap<String, ApiSpecNode<F>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ApiSpecLeaf<F: TypeFamily = AuthoredFamily> {
    pub module_path: ModulePath,
    pub source_root: PathBuf,
    pub source_path: PathBuf,
    pub spec: ApiSpec<F>,
}

fn transform_tree<F, G, P>(tree: ApiSpecTree<F>, pass: &mut P) -> Result<ApiSpecTree<G>, P::Error>
where
    F: TypeFamily,
    G: TypeFamily,
    P: CompilerPass<F, G>,
{
    fn transform_node<F, G, P>(
        node: ApiSpecNode<F>,
        pass: &mut P,
    ) -> Result<ApiSpecNode<G>, P::Error>
    where
        F: TypeFamily,
        G: TypeFamily,
        P: CompilerPass<F, G>,
    {
        match node {
            ApiSpecNode::Leaf(leaf) => pass.transform_leaf(leaf).map(ApiSpecNode::Leaf),
            ApiSpecNode::Branch(branch) => Ok(ApiSpecNode::Branch(ApiSpecBranch {
                module_path: branch.module_path,
                children: branch
                    .children
                    .into_iter()
                    .map(|(name, child)| Ok((name, transform_node(child, pass)?)))
                    .collect::<Result<_, _>>()?,
            })),
        }
    }

    Ok(ApiSpecTree {
        root: transform_node(tree.root, pass)?,
    })
}
