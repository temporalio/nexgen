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
    ApiSpecTransform, ExternalTypeSourceSpec, ExternalTypeSpec, JsonModelSpec, ProtoTypeSpec,
    RecordFieldSpec, RecordSpec, SelectedResourceType, Symbol, TypeDeclEntry, TypeDeclSpec,
    TypeSpec,
};

use super::{
    OperationBoundFamily, OperationBoundOperation, OperationBoundResource,
    ResolvedResourceBindingSource, SelectedTextSpec,
};

/// Operation-bound IR after resource-return structures become explicit records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OperationLoweredFamily;

impl crate::spec::TypeFamily for OperationLoweredFamily {
    type Origin = Option<crate::spec::FieldOrigin>;
    type SpecData = ();
    type Record = Symbol;
    type Enum = Symbol;
    type Flags = Symbol;
    type Variant = Symbol;
    type Resource = SelectedResourceType;
    type Proto = Symbol;
    type Json = JsonModelSpec<Symbol>;
    type Alias = Symbol;
    type ServiceData = ();
    type RecordData = ();
    type ResourceData = OperationBoundResource;
    type OperationData = OperationBoundOperation;
    type FieldData = ();
    type Text = SelectedTextSpec;
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
        let mut holders = Vec::new();

        for service in &mut spec.services {
            for operation in &mut service.operations {
                let Some(resource_return) = operation.data.output_resource_return.as_ref() else {
                    continue;
                };
                if let Some(holder) = &resource_return.holder {
                    holders.push((
                        service.name.clone(),
                        operation.name.clone(),
                        holder.clone(),
                        resource_return.clone(),
                    ));
                    continue;
                }
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
                        source: Some(ExternalTypeSourceSpec::Proto(ProtoTypeSpec {
                            proto: Symbol::new(resource_return.output_message_name.clone()),
                            reference: Default::default(),
                            type_name: Default::default(),
                        })),
                        experimental: operation.experimental,
                        flatten_in_api: false,
                        fields: resource_result_fields(resource_return),
                        data: (),
                    },
                ));
            }
        }

        for (service_name, operation_name, holder, resource_return) in holders {
            let (wire_full_name, wire_record) = lower_resource_holder(
                &spec,
                &service_name,
                &operation_name,
                &holder,
                &resource_return,
            )?;
            if let Some(TypeDeclEntry {
                declaration: TypeDeclSpec::Record(record),
                ..
            }) = spec.types.get_mut(&holder.record_name)
            {
                // The holder is the API model only. Its message travels as the wire record.
                record.source = None;
            }
            let operation = spec
                .services
                .iter_mut()
                .filter(|service| service.name == service_name)
                .flat_map(|service| service.operations.iter_mut())
                .find(|operation| operation.name == operation_name)
                .expect("the holder operation is in its service");
            operation.output = Some(TypeSpec::Record(Symbol::new(wire_full_name.clone())));
            generated_records.push((wire_full_name, wire_record));
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

/// The wire record of a result record that holds a resource. It takes the name of the proto
/// message, and has the holder's other fields plus the fields that the resource binds from.
fn lower_resource_holder(
    spec: &crate::spec::ApiSpec<OperationLoweredFamily>,
    service_name: &str,
    operation_name: &str,
    holder: &super::ResolvedResourceHolder,
    resource_return: &super::ResolvedResourceReturnSpec,
) -> Result<(String, RecordSpec<OperationLoweredFamily>)> {
    let record = spec
        .record(&holder.record_name)
        .expect("the holder record is declared");
    let wire_name = resource_return
        .output_message_name
        .rsplit('.')
        .next()
        .unwrap_or(&resource_return.output_message_name)
        .to_string();
    if spec
        .types
        .values()
        .any(|entry| {
            matches!(&entry.declaration, TypeDeclSpec::Record(other) if other.name == wire_name)
        })
    {
        return Err(Error::InvalidResource {
            service: service_name.to_string(),
            resource: resource_return.resource_name.to_upper_camel_case(),
            reason: format!(
                "operation `{operation_name}` needs the name `{wire_name}` for the wire record \
                 of `{}`, and a record already has it",
                record.name
            ),
        });
    }
    let mut fields = IndexMap::new();
    for (key, field) in &record.fields {
        if key == &holder.field_key {
            fields.extend(resource_result_fields(resource_return));
        } else {
            fields.insert(key.clone(), field.clone());
        }
    }
    let full_name = format!("__generated.{service_name}.{wire_name}");
    Ok((
        full_name.clone(),
        RecordSpec {
            name: wire_name,
            full_name,
            doc: record.doc.clone(),
            source: Some(ExternalTypeSourceSpec::Proto(ProtoTypeSpec {
                proto: Symbol::new(resource_return.output_message_name.clone()),
                reference: Default::default(),
                type_name: Default::default(),
            })),
            experimental: record.experimental,
            flatten_in_api: false,
            fields,
            data: (),
        },
    ))
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
                    required: false,
                    origin: None,
                    function: None,
                    data: (),
                },
            ))
        })
        .collect()
}

struct OperationLoweringMapper;

impl ApiSpecTransform<OperationBoundFamily, OperationLoweredFamily> for OperationLoweringMapper {
    fn map_origin(
        &mut self,
        origin: Option<crate::spec::FieldOrigin>,
    ) -> Option<crate::spec::FieldOrigin> {
        origin
    }
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
    fn map_resource(&mut self, value: SelectedResourceType) -> SelectedResourceType {
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
}
