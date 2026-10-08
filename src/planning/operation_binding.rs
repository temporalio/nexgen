//! `OperationBindingPass` attaches resolved operation/resource relationships
//! to their owning nodes.
//!
//! It consumes resource-bound IR and produces `OperationBoundFamily`. This is
//! deliberately semantic only: type materialization belongs to a later pass.

use super::*;

/// Semantic operation and resource relationships after resource resolution.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct OperationBoundOperation {
    pub(crate) output_resource_return: Option<ResolvedResourceReturnSpec>,
    pub(crate) direct_return: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct OperationBoundResource {
    pub(crate) resolved: Option<ResolvedResourceSpec>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OperationBoundFamily;

impl TypeFamily for OperationBoundFamily {
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
}

pub(crate) struct OperationBindingPass;

impl OperationBindingPass {
    pub(crate) fn new() -> Self {
        Self
    }
}

impl CompilerPass<ResourceBoundFamily, OperationBoundFamily> for OperationBindingPass {
    type Error = Error;

    fn transform_leaf(
        &mut self,
        leaf: ApiSpecLeaf<ResourceBoundFamily>,
    ) -> Result<ApiSpecLeaf<OperationBoundFamily>> {
        let bindings = leaf.spec.data.bindings.clone();
        let mut spec = leaf.spec.map_names(OperationBindingMapper);

        for service in &mut spec.services {
            let resolved = bindings.get(&service.name);
            for operation in &mut service.operations {
                let output_resource_return = resolved
                    .and_then(|resources| resources.operation_returns.get(&operation.name))
                    .cloned();
                operation.data = OperationBoundOperation {
                    direct_return: operation.output_transform().is_some()
                        || output_resource_return.is_some()
                        || matches!(
                            operation.output_type(),
                            Some(TypeSpec::Resource(resource)) if resource.wire_type.is_none()
                        ),
                    output_resource_return,
                };
            }
            for resource in &mut service.resources {
                resource.data = OperationBoundResource {
                    resolved: resolved.and_then(|resources| {
                        resources
                            .resources
                            .iter()
                            .find(|candidate| candidate.name == resource.name)
                            .cloned()
                    }),
                };
            }
        }

        Ok(ApiSpecLeaf {
            module_path: leaf.module_path,
            source_root: leaf.source_root,
            source_path: leaf.source_path,
            spec,
        })
    }
}

struct OperationBindingMapper;

impl ApiSpecTransform<ResourceBoundFamily, OperationBoundFamily> for OperationBindingMapper {
    fn map_spec_data(&mut self, _data: ResourceBoundData) {}
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
    fn map_resource_data(&mut self, _: &str, _: ()) -> OperationBoundResource {
        OperationBoundResource { resolved: None }
    }
    fn map_operation_data(&mut self, _: &str, _: ()) -> OperationBoundOperation {
        OperationBoundOperation {
            output_resource_return: None,
            direct_return: false,
        }
    }
    fn map_field_data(&mut self, _: &str, _: &str, _: ()) {}
    fn map_text(&mut self, value: SelectedTextSpec) -> SelectedTextSpec {
        value
    }
}
