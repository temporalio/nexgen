use std::collections::{BTreeMap, BTreeSet};

use heck::ToUpperCamelCase;
use indexmap::IndexMap;
use prost_types::FileOptions;

use crate::descriptors::DescriptorIndex;
use crate::error::{Error, Result};
use crate::language::Language;
use crate::spec::{
    AliasTypeSpec, ApiSpec, ApiSpecTransform, AuthoredResourceType, ExternalTypeSourceSpec,
    ExternalTypeSpec, ExternalVariantSourceSpec, FunctionArgSpec, FunctionArgsSpec,
    FunctionFieldSpec, FunctionResultSpec, JsonModelSpec, LanguageStringSpec, ModulePath,
    OperationSpec, RecordFieldVisibility, RecordSpec, ResourceFieldSpec, SelectedFamily,
    SelectedTextSpec, ServiceSpec, Symbol, TypeDeclSpec, TypeFamily, TypeReplacementSpec, TypeSpec,
};
use crate::spec::{ApiSpecLeaf, ApiSpecNode, ApiSpecTree, CompilerPass};

mod authored_validation;
mod emitted_names;
mod operation_binding;
mod operation_lowering;
mod proto;
mod reachability;
mod resource_resolution;
mod selection;
mod type_planning;

pub(crate) use authored_validation::AuthoredValidationPass;
pub(crate) use emitted_names::EmittedNameResolutionPass;
pub(crate) use emitted_names::build_json_name_manifest;
pub(crate) use operation_binding::{
    OperationBindingPass, OperationBoundFamily, OperationBoundOperation, OperationBoundResource,
};
pub(crate) use operation_lowering::{OperationLoweredFamily, OperationLoweringPass};
pub(crate) use proto::{message_model_name, relative_descriptor_name};
pub(crate) use reachability::ReachabilityPass;
pub(crate) use resource_resolution::{
    RequestPlan, RequestPlanSource, ResolvedResourceBindingSource, ResolvedResourceMethodBinding,
    ResolvedResourceReturnSpec, ResolvedResourceSpec, ResourceBoundData, ResourceBoundFamily,
    ResourceResolutionPass,
};
pub(crate) use selection::LanguageSelectionPass;
#[cfg(test)]
pub(crate) use selection::select_spec;
pub(crate) use type_planning::TypePlanningPass;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PlannedFamily;

impl TypeFamily for PlannedFamily {
    type SpecData = PlannedSpecData;
    type Record = PlannedRecordType;
    type Enum = PlannedEnumType;
    type Flags = PlannedFlagsType;
    type Variant = PlannedVariantType;
    type Resource = PlannedResourceType;
    type Proto = PlannedProtoType;
    type Json = PlannedJsonType;
    type Alias = PlannedAliasType;
    type ServiceData = ();
    type RecordData = PlannedRecordData;
    type ResourceData = PlannedResource;
    type OperationData = PlannedOperationData;
    type FieldData = PlannedFieldData;
    // Planned code still exposes the legacy text container to the generators.
    // `TypePlanningMapper` materializes it from already-selected values with no
    // language override maps, so no later stage performs selection.
    type Text = LanguageStringSpec;
}

pub(crate) type PlannedSpec = ApiSpec<PlannedFamily>;
pub(crate) type PlannedType = TypeSpec<PlannedFamily>;

pub(super) fn materialize_selected_text(text: &SelectedTextSpec) -> LanguageStringSpec {
    LanguageStringSpec {
        default: text.value.clone(),
        default_import: text.import.clone(),
        ..Default::default()
    }
}

pub(super) fn materialize_selected_replacement(
    replacement: &TypeReplacementSpec<OperationLoweredFamily>,
) -> TypeReplacementSpec {
    TypeReplacementSpec {
        type_name: materialize_selected_text(&replacement.type_name),
        from_proto: materialize_selected_text(&replacement.from_proto),
        to_proto: materialize_selected_text(&replacement.to_proto),
    }
}

pub(crate) fn operation_input_model(
    operation: &OperationSpec<PlannedFamily>,
) -> Option<&PlannedType> {
    match operation.input_type() {
        Some(input) if planned_type_is_model_shaped(input) => Some(input),
        Some(_) => panic!("planned operation inputs are model-shaped when present"),
        None => None,
    }
}

pub(crate) fn planned_type_is_model_shaped(planned_type: &PlannedType) -> bool {
    matches!(
        planned_type,
        TypeSpec::External(ExternalTypeSpec::Proto(PlannedProtoType::Message(_)))
            | TypeSpec::External(ExternalTypeSpec::Json(_))
            | TypeSpec::Record(_)
    )
}

pub(crate) fn operation_output_direct_result(operation: &OperationSpec<PlannedFamily>) -> bool {
    matches!(
        operation.output_type(),
        Some(TypeSpec::Resource(PlannedResourceType {
            wire_type: None,
            ..
        }))
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PlanningMode {
    NativeApi,
    DefinitionsOnly,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PlannedOperationResourceReturn {
    pub(crate) resource_type_name: String,
    pub(crate) bindings: Vec<PlannedOperationResourceFieldBinding>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PlannedOperationResourceFieldBinding {
    pub(crate) field_name: String,
    pub(crate) optional: bool,
    pub(crate) source: ResolvedResourceBindingSource,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PlannedResource {
    pub(crate) name: String,
    pub(crate) type_name: String,
    pub(crate) fields: Vec<PlannedResourceField>,
    pub(crate) methods: Vec<PlannedResourceMethod>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PlannedResourceField {
    pub(crate) name: String,
    pub(crate) optional: bool,
    pub(crate) kind: PlannedType,
    pub(crate) function: Option<FunctionFieldSpec<PlannedFamily>>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PlannedResourceMethod {
    pub(crate) name: String,
    pub(crate) params: Vec<PlannedResourceField>,
    pub(crate) result: Option<PlannedResourceMethodResult>,
    pub(crate) binding: PlannedResourceMethodBindingSpec,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PlannedResourceMethodResult {
    pub(crate) kind: PlannedResourceMethodResultKind,
    pub(crate) optional: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PlannedResourceMethodResultKind {
    Resource { type_name: String },
    Value(PlannedType),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PlannedResourceMethodBindingSpec {
    Operation {
        operation_name: String,
        request_plan: RequestPlan,
        direct_return: bool,
    },
    Stub,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PlannedProtoTypeInfo {
    pub(crate) full_name: String,
    pub(crate) package: String,
    pub(crate) file_name: Option<String>,
    pub(crate) file_options: Option<FileOptions>,
    pub(crate) reference: LanguageStringSpec,
    pub(crate) type_name: LanguageStringSpec,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct PlannedRecordData {
    pub(crate) proto: Option<PlannedProtoTypeInfo>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct PlannedFieldData {
    pub(crate) has_presence: Option<bool>,
    pub(crate) wire_binding: Option<PlannedWireFieldBinding>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PlannedWireFieldBinding {
    Value {
        wire_name: String,
        wire_type: PlannedType,
    },
    VariantMembers {
        wire_name: String,
        members: Vec<PlannedWireVariantMember>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PlannedWireVariantMember {
    pub(crate) wire_name: String,
    pub(crate) wire_type: PlannedType,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct PlannedOperationData {
    pub(crate) output_resource_return: Option<PlannedOperationResourceReturn>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PlannedEnumType {
    pub(crate) full_name: String,
    pub(crate) name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PlannedFlagsType {
    pub(crate) full_name: String,
    pub(crate) name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PlannedVariantType {
    pub(crate) full_name: String,
    pub(crate) name: String,
}

impl AsRef<str> for PlannedVariantType {
    fn as_ref(&self) -> &str {
        &self.full_name
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PlannedAliasType {
    pub(crate) name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PlannedJsonType {
    pub(crate) full_name: String,
    pub(crate) module_path: Option<ModulePath>,
    pub(crate) model_name: String,
    pub(crate) schema: serde_json::Value,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct PlannedSpecData {
    pub(crate) module_imports: BTreeMap<ModulePath, BTreeSet<String>>,
    /// The emitted identifier of every JSON model declared in *another* module,
    /// keyed by model full name. A leaf cannot derive these: the `x-<lang>-name`
    /// override that moves an identifier is declared in the other input file, so
    /// `EmittedNameResolutionPass` resolves them from the tree-wide manifest and
    /// records them here for the generators' `$ref` registries.
    pub(crate) cross_module_model_names: BTreeMap<String, String>,
}

impl AsRef<str> for PlannedJsonType {
    fn as_ref(&self) -> &str {
        &self.full_name
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PlannedProtoType {
    Message(PlannedProtoMessageType),
    Enum(PlannedProtoEnumType),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PlannedProtoMessageType {
    pub(crate) proto: PlannedProtoTypeInfo,
    pub(crate) model_name: String,
    pub(crate) replacement: Option<TypeReplacementSpec>,
    pub(crate) authored_type: Option<Box<PlannedType>>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PlannedProtoEnumType {
    pub(crate) proto: PlannedProtoTypeInfo,
    pub(crate) name: String,
    pub(crate) replacement: Option<TypeReplacementSpec>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PlannedRecordType {
    pub(crate) full_name: String,
    pub(crate) model_name: String,
}

impl AsRef<str> for PlannedRecordType {
    fn as_ref(&self) -> &str {
        &self.full_name
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PlannedResourceType {
    pub(crate) type_name: String,
    pub(crate) wire_type: Option<ExternalTypeSpec<PlannedFamily>>,
}

impl AsRef<str> for PlannedResourceType {
    fn as_ref(&self) -> &str {
        &self.type_name
    }
}

impl PlannedProtoType {
    pub(crate) fn full_name(&self) -> &str {
        match self {
            PlannedProtoType::Message(message) => &message.proto.full_name,
            PlannedProtoType::Enum(enumeration) => &enumeration.proto.full_name,
        }
    }
}
