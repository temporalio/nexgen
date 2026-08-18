//! `OperationLoweringPass` makes operation-result structure explicit.
//!
//! Resource binding has already established the result-field bindings. This
//! pass turns a wire-backed resource return into a concrete generated record,
//! leaving type materialization for `TypePlanningPass`.

use heck::ToUpperCamelCase;
use indexmap::IndexMap;

use crate::error::{Error, Result};
use crate::spec::{ApiSpecLeaf, CompilerPass};
use crate::spec::{
    ApiSpecTransform, AuthoredResourceType, ExternalTypeSpec, JsonModelSpec, RecordFieldSpec,
    RecordFieldVisibility, RecordSpec, Symbol, TypeDeclEntry, TypeDeclSpec, TypeSpec,
};

use super::{
    OperationBoundFamily, OperationBoundOperation, OperationBoundResource,
    ResolvedResourceBindingSource, SelectedSupportSpec, SelectedTextSpec,
};

/// Operation-bound IR after resource-return structures become explicit records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OperationLoweredFamily;

impl crate::spec::TypeFamily for OperationLoweredFamily {
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
    type ResourceData = OperationBoundResource;
    type OperationData = OperationBoundOperation;
    type FieldData = ();
    type Text = SelectedTextSpec;
    type Support = SelectedSupportSpec;
}

pub(crate) struct OperationLoweringPass;

impl OperationLoweringPass {
    pub(crate) fn new() -> Self {
        Self
    }
}

impl CompilerPass<OperationBoundFamily, OperationLoweredFamily> for OperationLoweringPass {
    type Error = Error;

    fn transform_leaf(
        &mut self,
        leaf: ApiSpecLeaf<OperationBoundFamily>,
    ) -> Result<ApiSpecLeaf<OperationLoweredFamily>> {
        let mut spec = leaf.spec.map_names(OperationLoweringMapper);
        let mut generated_records = Vec::new();

        for service in &mut spec.services {
            for operation in &mut service.operations {
                let Some(resource_return) = operation.data.output_resource_return.as_ref() else {
                    continue;
                };
                let Some(TypeSpec::Resource(resource)) = operation.output_type() else {
                    continue;
                };
                if !matches!(resource.wire_type, Some(ExternalTypeSpec::Proto(_))) {
                    continue;
                }

                let (record_name, record_full_name) = resource
                    .alias
                    .as_ref()
                    .map(|alias| (alias.name.clone(), alias.full_name.clone()))
                    .unwrap_or_else(|| {
                        let record_name = format!("{}Result", operation.name.to_upper_camel_case());
                        (
                            record_name.clone(),
                            format!("__generated.{}.{}", service.name, record_name),
                        )
                    });
                operation.output = Some(TypeSpec::Record(Symbol::new(record_full_name.clone())));
                generated_records.push((
                    record_full_name.clone(),
                    RecordSpec {
                        name: record_name.clone(),
                        full_name: record_full_name.clone(),
                        doc: SelectedTextSpec::default(),
                        source_type: Some(ExternalTypeSpec::Proto(Symbol::new(
                            resource_return.output_message_name.clone(),
                        ))),
                        experimental: operation.experimental,
                        flatten_in_api: false,
                        fields: resource_result_fields(resource_return),
                        data: (),
                    },
                ));
            }
        }

        for (full_name, record) in generated_records {
            spec.types
                .entry(full_name)
                .or_insert_with(|| TypeDeclEntry::new(TypeDeclSpec::Record(record)));
        }

        Ok(ApiSpecLeaf {
            module_path: leaf.module_path,
            source_root: leaf.source_root,
            source_path: leaf.source_path,
            spec,
        })
    }
}

fn resource_result_fields(
    resource_return: &super::ResolvedResourceReturnSpec,
) -> IndexMap<String, RecordFieldSpec<OperationLoweredFamily>> {
    resource_return
        .bindings
        .iter()
        .filter_map(|binding| {
            let ResolvedResourceBindingSource::ResultField {
                field_name,
                proto_field_name,
            } = &binding.source
            else {
                return None;
            };
            Some((
                proto_field_name.clone(),
                RecordFieldSpec {
                    name: field_name.clone(),
                    doc: None,
                    annotation: None,
                    flattened_annotation: None,
                    field_type: TypeSpec::String,
                    default_value: None,
                    required: false,
                    visibility: RecordFieldVisibility::Public,
                    function: None,
                    data: (),
                },
            ))
        })
        .collect()
}

struct OperationLoweringMapper;

impl ApiSpecTransform<OperationBoundFamily, OperationLoweredFamily> for OperationLoweringMapper {
    fn map_spec_data(&mut self, _data: ()) {}
    fn map_record(&mut self, value: Symbol) -> Symbol {
        value
    }
    fn map_enum(&mut self, value: Symbol) -> Symbol {
        value
    }
    fn map_flags(&mut self, value: Symbol) -> Symbol {
        value
    }
    fn map_variant(&mut self, value: Symbol) -> Symbol {
        value
    }
    fn map_resource(&mut self, value: AuthoredResourceType) -> AuthoredResourceType {
        value
    }
    fn map_proto(&mut self, value: Symbol) -> Symbol {
        value
    }
    fn map_json(&mut self, value: JsonModelSpec<Symbol>) -> JsonModelSpec<Symbol> {
        value
    }
    fn map_alias(&mut self, value: Symbol) -> Symbol {
        value
    }
    fn map_service_data(&mut self, _: &str, _: ()) {}
    fn map_record_data(&mut self, _: &str, _: ()) {}
    fn map_resource_data(
        &mut self,
        _: &str,
        value: super::OperationBoundResource,
    ) -> super::OperationBoundResource {
        value
    }
    fn map_operation_data(
        &mut self,
        _: &str,
        value: super::OperationBoundOperation,
    ) -> super::OperationBoundOperation {
        value
    }
    fn map_field_data(&mut self, _: &str, _: &str, _: ()) {}
    fn map_text(&mut self, value: SelectedTextSpec) -> SelectedTextSpec {
        value
    }
    fn map_support(&mut self, value: super::SelectedSupportSpec) -> super::SelectedSupportSpec {
        value
    }
}
