//! `TypePlanningPass` turns operation-lowered selected types into planned types.
//!
//! Its analysis is local to the pass. The output is a self-contained
//! `PlannedFamily`, never a saved planner state for a later pass.
fn module_import_index(
    tree: &ApiSpecTree<OperationLoweredFamily>,
) -> BTreeMap<ModulePath, BTreeMap<ModulePath, BTreeSet<String>>> {
    let mut imports = BTreeMap::<ModulePath, BTreeMap<ModulePath, BTreeSet<String>>>::new();
    collect_tree_module_imports(&tree.root, &mut imports);
    imports
}

fn collect_tree_module_imports(
    node: &ApiSpecNode<OperationLoweredFamily>,
    imports: &mut BTreeMap<ModulePath, BTreeMap<ModulePath, BTreeSet<String>>>,
) {
    match node {
        ApiSpecNode::Leaf(leaf) => collect_spec_module_imports(&leaf.spec, imports),
        ApiSpecNode::Branch(branch) => {
            for child in branch.children.values() {
                collect_tree_module_imports(child, imports);
            }
        }
    }
}

fn collect_spec_module_imports(
    spec: &ApiSpec<OperationLoweredFamily>,
    imports: &mut BTreeMap<ModulePath, BTreeMap<ModulePath, BTreeSet<String>>>,
) {
    for service in &spec.services {
        for operation in &service.operations {
            collect_type_module_imports(&spec.module_path, operation.input_type(), imports);
            collect_type_module_imports(&spec.module_path, operation.output_type(), imports);
        }
        for resource in &service.resources {
            for field in &resource.fields {
                collect_type_module_imports(&spec.module_path, Some(&field.field_type), imports);
            }
            for method in &resource.methods {
                for field in &method.params {
                    collect_type_module_imports(
                        &spec.module_path,
                        Some(&field.field_type),
                        imports,
                    );
                }
                if let Some(result) = &method.result {
                    collect_type_module_imports(
                        &spec.module_path,
                        Some(&result.result_type),
                        imports,
                    );
                }
            }
        }
    }
    for entry in spec.types.values() {
        match &entry.declaration {
            TypeDeclSpec::External(binding) => {
                collect_external_type_module_imports(
                    &spec.module_path,
                    &binding.external_type,
                    imports,
                );
                if let Some(authored_type) = &binding.authored_type {
                    collect_type_module_imports(&spec.module_path, Some(authored_type), imports);
                }
            }
            TypeDeclSpec::Record(record) => {
                if let Some(source_type) = &record.source_type {
                    collect_external_type_module_imports(&spec.module_path, source_type, imports);
                }
                for field in record.fields.values() {
                    collect_type_module_imports(
                        &spec.module_path,
                        Some(&field.field_type),
                        imports,
                    );
                    if let Some(function) = &field.function {
                        if let Some(alternate) = &function.alternate_type {
                            collect_type_module_imports(
                                &spec.module_path,
                                Some(alternate),
                                imports,
                            );
                        }
                        collect_function_args_module_imports(
                            &spec.module_path,
                            &function.args,
                            imports,
                        );
                        if let Some(result) = &function.result_type_parameter {
                            let _ = result;
                        }
                    }
                }
            }
            TypeDeclSpec::Variant(variant) => {
                for case in &variant.cases {
                    if let Some(payload) = &case.payload {
                        collect_type_module_imports(&spec.module_path, Some(payload), imports);
                    }
                }
            }
            TypeDeclSpec::Enum(_) | TypeDeclSpec::Flags(_) => {}
        }
    }
}

fn collect_function_args_module_imports(
    source_module: &ModulePath,
    args: &FunctionArgsSpec<OperationLoweredFamily>,
    imports: &mut BTreeMap<ModulePath, BTreeMap<ModulePath, BTreeSet<String>>>,
) {
    let args = match args {
        FunctionArgsSpec::Varargs { prefix, .. } => prefix.as_slice(),
        FunctionArgsSpec::Fixed(args) => args.as_slice(),
    };
    for arg in args {
        collect_type_module_imports(source_module, Some(&arg.field_type), imports);
    }
}

fn collect_type_module_imports(
    source_module: &ModulePath,
    ty: Option<&TypeSpec<OperationLoweredFamily>>,
    imports: &mut BTreeMap<ModulePath, BTreeMap<ModulePath, BTreeSet<String>>>,
) {
    let Some(ty) = ty else {
        return;
    };
    match ty {
        TypeSpec::Record(symbol)
        | TypeSpec::Enum(symbol)
        | TypeSpec::Flags(symbol)
        | TypeSpec::Variant(symbol) => {
            collect_symbol_module_import(source_module, symbol, imports);
        }
        TypeSpec::Resource(resource) => {
            collect_resource_symbol_module_import(source_module, resource, imports)
        }
        TypeSpec::External(external) => {
            collect_external_type_module_imports(source_module, external, imports)
        }
        TypeSpec::Option(inner) | TypeSpec::List(inner) => {
            collect_type_module_imports(source_module, Some(inner), imports);
        }
        TypeSpec::Tuple(items) => {
            for item in items {
                collect_type_module_imports(source_module, Some(item), imports);
            }
        }
        TypeSpec::Map(key, value) => {
            collect_type_module_imports(source_module, Some(key), imports);
            collect_type_module_imports(source_module, Some(value), imports);
        }
        TypeSpec::Result { ok, err } => {
            collect_type_module_imports(source_module, ok.as_deref(), imports);
            collect_type_module_imports(source_module, err.as_deref(), imports);
        }
        _ => {}
    }
}

fn collect_external_type_module_imports(
    source_module: &ModulePath,
    external: &ExternalTypeSpec<OperationLoweredFamily>,
    imports: &mut BTreeMap<ModulePath, BTreeMap<ModulePath, BTreeSet<String>>>,
) {
    match external {
        ExternalTypeSpec::Proto(symbol) => {
            collect_symbol_module_import(source_module, symbol, imports)
        }
        ExternalTypeSpec::Json(json_type) => {
            collect_symbol_module_import(source_module, &json_type.name, imports)
        }
        ExternalTypeSpec::Alias { name, target, .. } => {
            collect_symbol_module_import(source_module, name, imports);
            collect_type_module_imports(source_module, Some(target), imports);
        }
    }
}

fn collect_resource_symbol_module_import(
    source_module: &ModulePath,
    resource: &AuthoredResourceType,
    imports: &mut BTreeMap<ModulePath, BTreeMap<ModulePath, BTreeSet<String>>>,
) {
    collect_symbol_module_import(source_module, &resource.name, imports);
    if let Some(wire_type) = &resource.wire_type {
        collect_authored_external_type_module_imports(source_module, wire_type, imports);
    }
}

fn collect_authored_external_type_module_imports(
    source_module: &ModulePath,
    external: &ExternalTypeSpec,
    imports: &mut BTreeMap<ModulePath, BTreeMap<ModulePath, BTreeSet<String>>>,
) {
    match external {
        ExternalTypeSpec::Proto(symbol) => {
            collect_symbol_module_import(source_module, symbol, imports)
        }
        ExternalTypeSpec::Json(json_type) => {
            collect_symbol_module_import(source_module, &json_type.name, imports)
        }
        ExternalTypeSpec::Alias { name, target, .. } => {
            collect_symbol_module_import(source_module, name, imports);
            collect_authored_type_module_imports(source_module, target, imports);
        }
    }
}

fn collect_authored_type_module_imports(
    source_module: &ModulePath,
    ty: &TypeSpec,
    imports: &mut BTreeMap<ModulePath, BTreeMap<ModulePath, BTreeSet<String>>>,
) {
    match ty {
        TypeSpec::Record(symbol)
        | TypeSpec::Enum(symbol)
        | TypeSpec::Flags(symbol)
        | TypeSpec::Variant(symbol) => collect_symbol_module_import(source_module, symbol, imports),
        TypeSpec::Resource(resource) => {
            collect_resource_symbol_module_import(source_module, resource, imports)
        }
        TypeSpec::External(external) => {
            collect_authored_external_type_module_imports(source_module, external, imports)
        }
        TypeSpec::Option(inner) | TypeSpec::List(inner) => {
            collect_authored_type_module_imports(source_module, inner, imports)
        }
        TypeSpec::Tuple(items) => {
            for item in items {
                collect_authored_type_module_imports(source_module, item, imports);
            }
        }
        TypeSpec::Map(key, value) => {
            collect_authored_type_module_imports(source_module, key, imports);
            collect_authored_type_module_imports(source_module, value, imports);
        }
        TypeSpec::Result { ok, err } => {
            if let Some(ok) = ok {
                collect_authored_type_module_imports(source_module, ok, imports);
            }
            if let Some(err) = err {
                collect_authored_type_module_imports(source_module, err, imports);
            }
        }
        _ => {}
    }
}

fn collect_symbol_module_import(
    source_module: &ModulePath,
    symbol: &Symbol,
    imports: &mut BTreeMap<ModulePath, BTreeMap<ModulePath, BTreeSet<String>>>,
) {
    let Some(target_module) = symbol.module_path() else {
        return;
    };
    if target_module == source_module {
        return;
    }
    imports
        .entry(source_module.clone())
        .or_default()
        .entry(target_module.clone())
        .or_default()
        .insert(symbol.local_name().to_string());
}

use super::*;
use crate::spec::{FunctionTypeDescriptorSpec, OperationOutputTransformSpec, ResourceResultSpec};

pub(super) struct TypePlanningContext<'a> {
    pub(super) spec: ApiSpec<OperationLoweredFamily>,
    spec_data: PlannedSpecData,
    language: Language,
    pub(super) descriptors: &'a DescriptorIndex,
    resource_data: IndexMap<String, PlannedResource>,
    record_plans: IndexMap<String, PlannedRecordData>,
}

struct PlannedServiceBuild {
    operations: Vec<OperationSpec<PlannedFamily>>,
}

/// Planned operation metadata before it is written into the planned graph.
struct PlannedOperations {
    operations: IndexMap<String, Vec<OperationSpec<PlannedFamily>>>,
}

#[derive(Debug, Clone, Copy)]
struct OperationBindingInfo<'a> {
    name: &'a str,
    direct_return: bool,
}

struct TypePlanningMapper<'a, 'descriptors> {
    source_spec: &'a ApiSpec<OperationLoweredFamily>,
    planner: &'a mut TypePlanningContext<'descriptors>,
}

impl ApiSpecTransform<OperationLoweredFamily, PlannedFamily> for TypePlanningMapper<'_, '_> {
    fn map_spec_data(&mut self, _data: ()) -> PlannedSpecData {
        self.planner.spec_data.clone()
    }

    fn map_record(&mut self, name: Symbol) -> PlannedRecordType {
        let record = self
            .source_spec
            .record(name.as_str())
            .unwrap_or_else(|| panic!("record `{name}` should resolve during planning"));
        self.planner.plan_record_type(record)
    }

    fn map_enum(&mut self, name: Symbol) -> PlannedEnumType {
        let enumeration = self
            .source_spec
            .enum_decl(name.as_str())
            .unwrap_or_else(|| panic!("enum `{name}` should resolve during planning"));
        PlannedEnumType {
            full_name: enumeration.full_name.clone(),
            name: enumeration.name.clone(),
        }
    }

    fn map_flags(&mut self, name: Symbol) -> PlannedFlagsType {
        let flags = self
            .source_spec
            .flags_decl(name.as_str())
            .unwrap_or_else(|| panic!("flags `{name}` should resolve during planning"));
        PlannedFlagsType {
            full_name: flags.full_name.clone(),
            name: flags.name.clone(),
        }
    }

    fn map_variant(&mut self, name: Symbol) -> PlannedVariantType {
        let variant = self
            .source_spec
            .variant(name.as_str())
            .unwrap_or_else(|| panic!("variant `{name}` should resolve during planning"));
        PlannedVariantType {
            full_name: variant.full_name.clone(),
            name: variant.name.clone(),
        }
    }

    fn map_resource(&mut self, resource: AuthoredResourceType) -> PlannedResourceType {
        self.planner.planned_resource_type_from_authored(&resource)
    }

    fn map_proto(&mut self, name: Symbol) -> PlannedProtoType {
        if let Some(message) = self.planner.descriptors.message(name.as_str()) {
            PlannedProtoType::Message(proto::planned_message_reference(message, self.planner))
        } else if let Some(enumeration) = self.planner.descriptors.enumeration(name.as_str()) {
            PlannedProtoType::Enum(proto::planned_enum_reference(
                enumeration,
                &self.planner.spec,
            ))
        } else {
            panic!("proto `{name}` should resolve during planning");
        }
    }

    fn map_json(&mut self, name: JsonModelSpec<Symbol>) -> PlannedJsonType {
        PlannedJsonType {
            full_name: name.name.as_str().to_string(),
            module_path: name.name.module_path().cloned(),
            model_name: name.model_name,
            schema: name.schema,
        }
    }

    fn map_alias(&mut self, name: Symbol) -> PlannedAliasType {
        PlannedAliasType {
            name: name.as_str().to_string(),
        }
    }

    fn map_service_data(&mut self, _name: &str, _data: ()) {}

    fn map_record_data(&mut self, full_name: &str, _data: ()) -> PlannedRecordData {
        self.planner
            .record_plans
            .get(full_name)
            .cloned()
            .unwrap_or_default()
    }

    fn map_resource_data(&mut self, name: &str, _data: OperationBoundResource) -> PlannedResource {
        self.planner
            .resource_data
            .get(name)
            .cloned()
            .unwrap_or_else(|| PlannedResource {
                name: name.to_string(),
                type_name: name.to_upper_camel_case(),
                fields: Vec::new(),
                methods: Vec::new(),
            })
    }

    fn map_operation_data(
        &mut self,
        _name: &str,
        _data: OperationBoundOperation,
    ) -> PlannedOperationData {
        PlannedOperationData::default()
    }

    fn map_field_data(
        &mut self,
        record_full_name: &str,
        field_name: &str,
        _data: (),
    ) -> PlannedFieldData {
        self.source_spec
            .record(record_full_name)
            .cloned()
            .and_then(|record| proto::planned_record_field_data(&record, field_name, self.planner))
            .unwrap_or_default()
    }

    fn map_text(&mut self, text: SelectedTextSpec) -> LanguageStringSpec {
        materialize_selected_text(&text)
    }

    fn map_support(&mut self, support: SelectedSupportSpec) -> SupportSpec {
        SupportSpec {
            fragments: BTreeMap::from([(self.planner.language, support.fragments)]),
        }
    }
}

impl<'a> TypePlanningContext<'a> {
    fn new(
        spec: ApiSpec<OperationLoweredFamily>,
        spec_data: PlannedSpecData,
        descriptors: &'a DescriptorIndex,
        language: Language,
    ) -> Result<Self> {
        Ok(Self {
            spec,
            spec_data,
            language,
            descriptors,
            resource_data: IndexMap::new(),
            record_plans: IndexMap::new(),
        })
    }

    fn plan_spec(&mut self, spec: ApiSpec<OperationLoweredFamily>) -> PlannedSpec {
        for record in spec.records().map(|(_, record)| record.clone()) {
            self.ensure_record_model_plan(&record);
        }
        let source_spec = spec.clone();
        let mut planned_spec = spec.map_names(TypePlanningMapper {
            source_spec: &source_spec,
            planner: self,
        });
        self.resolve_planned_record_field_types(&source_spec, &mut planned_spec);
        planned_spec
    }

    fn resolve_planned_record_field_types(
        &mut self,
        source_spec: &ApiSpec<OperationLoweredFamily>,
        planned_spec: &mut PlannedSpec,
    ) {
        for (record_name, planned_entry) in &mut planned_spec.types {
            let TypeDeclSpec::Record(planned_record) = &mut planned_entry.declaration else {
                continue;
            };
            let Some(source_record) = source_spec.record(record_name) else {
                continue;
            };
            for (field_name, field) in &mut planned_record.fields {
                let source_field = source_record
                    .fields
                    .get(field_name)
                    .expect("planned record field should exist in source record");
                field.field_type =
                    proto::planned_record_field_type(source_record, field_name, self)
                        .unwrap_or_else(|| {
                            self.planned_type_from_authored(
                                source_field.field_type.without_option(),
                            )
                        });
            }
        }
    }

    pub(super) fn insert_record_plan(&mut self, full_name: String, data: PlannedRecordData) {
        self.record_plans.insert(full_name, data);
    }

    fn plan_service(
        &mut self,
        service: &ServiceSpec<OperationLoweredFamily>,
    ) -> Result<PlannedServiceBuild> {
        let operation_builds = service
            .operations
            .iter()
            .map(|operation| self.plan_operation(service, operation))
            .collect::<Result<Vec<_>>>()?;

        let resources = {
            let operation_bindings = operation_builds
                .iter()
                .map(|operation| OperationBindingInfo {
                    name: &operation.name,
                    direct_return: service
                        .operation(&operation.name)
                        .expect("planned operation should exist in source service")
                        .data
                        .direct_return,
                })
                .collect::<Vec<_>>();
            service
                .resources
                .iter()
                .filter_map(|resource| resource.data.resolved.as_ref())
                .map(|resource| {
                    let resource_plan =
                        self.plan_resource(service, resource, &operation_bindings)?;
                    Ok((resource.name.clone(), resource_plan))
                })
                .collect::<Result<IndexMap<_, _>>>()?
        };

        self.resource_data.extend(resources);

        Ok(PlannedServiceBuild {
            operations: operation_builds,
        })
    }

    fn plan_operation(
        &mut self,
        service: &ServiceSpec<OperationLoweredFamily>,
        operation: &OperationSpec<OperationLoweredFamily>,
    ) -> Result<OperationSpec<PlannedFamily>> {
        let input = self.plan_operation_input(service, operation)?;
        let output = self.plan_operation_output(service, operation)?;

        Ok(OperationSpec {
            name: operation.name.clone(),
            code_name: materialize_selected_text(&operation.code_name),
            wire_name: operation.wire_name.clone(),
            experimental: operation.experimental,
            deprecated: operation.deprecated,
            doc: materialize_selected_text(&operation.doc),
            return_doc: materialize_selected_text(&operation.return_doc),
            input,
            output,
            output_transform: operation.output_transform.as_ref().map(|transform| {
                OperationOutputTransformSpec {
                    type_name: materialize_selected_text(&transform.type_name),
                    transform: materialize_selected_text(&transform.transform),
                }
            }),
            serialization_context: materialize_selected_text(&operation.serialization_context),
            data: PlannedOperationData {
                output_resource_return: plan_operation_resource_return(
                    operation.data.output_resource_return.as_ref(),
                ),
            },
        })
    }

    fn plan_operation_input(
        &mut self,
        service: &ServiceSpec<OperationLoweredFamily>,
        operation: &OperationSpec<OperationLoweredFamily>,
    ) -> Result<Option<PlannedType>> {
        match operation.input_type() {
            None => Ok(None),
            Some(TypeSpec::External(ExternalTypeSpec::Proto(type_ref))) => {
                let input_message =
                    self.descriptors.message(type_ref.as_str()).ok_or_else(|| {
                        Error::UnknownOperationInputProto {
                            service: service.name.clone(),
                            operation: operation.name.clone(),
                            type_name: type_ref.as_str().to_string(),
                        }
                    })?;
                Ok(Some(proto::planned_type_for_message(input_message, self)))
            }
            Some(TypeSpec::External(ExternalTypeSpec::Json(json_type))) => Ok(Some(
                TypeSpec::External(ExternalTypeSpec::Json(self.map_json_type(json_type))),
            )),
            Some(TypeSpec::Record(record_name)) => {
                let record = self
                    .spec
                    .record(record_name.as_str())
                    .cloned()
                    .ok_or_else(|| Error::UnknownOperationInputProto {
                        service: service.name.clone(),
                        operation: operation.name.clone(),
                        type_name: record_name.as_str().to_string(),
                    })?;
                Ok(Some(TypeSpec::Record(self.plan_record_type(&record))))
            }
            Some(TypeSpec::Resource(resource_name)) => Err(Error::InvalidWit {
                path: std::path::PathBuf::from("<api-plan>"),
                reason: format!(
                    "operation `{}` uses resource `{resource_name}` as an input type, which is not supported for generated operations yet",
                    operation.name
                ),
            }),
            Some(_) => Err(Error::InvalidWit {
                path: std::path::PathBuf::from("<api-plan>"),
                reason: format!("operation `{}` input must be a named type", operation.name),
            }),
        }
    }

    fn plan_operation_output(
        &mut self,
        service: &ServiceSpec<OperationLoweredFamily>,
        operation: &OperationSpec<OperationLoweredFamily>,
    ) -> Result<Option<PlannedType>> {
        match operation.output_type() {
            Some(TypeSpec::External(ExternalTypeSpec::Proto(output_proto))) => {
                let output_message =
                    self.descriptors
                        .message(output_proto.as_str())
                        .ok_or_else(|| Error::UnknownOperationOutputProto {
                            service: service.name.clone(),
                            operation: operation.name.clone(),
                            type_name: output_proto.as_str().to_string(),
                        })?;
                Ok(Some(proto::planned_type_for_message(output_message, self)))
            }
            Some(TypeSpec::External(ExternalTypeSpec::Json(json_type))) => Ok(Some(
                TypeSpec::External(ExternalTypeSpec::Json(self.map_json_type(json_type))),
            )),
            Some(TypeSpec::Record(record_name)) => {
                let record = self
                    .spec
                    .record(record_name.as_str())
                    .cloned()
                    .ok_or_else(|| Error::UnknownOperationOutputProto {
                        service: service.name.clone(),
                        operation: operation.name.clone(),
                        type_name: record_name.as_str().to_string(),
                    })?;
                Ok(Some(TypeSpec::Record(self.plan_record_type(&record))))
            }
            Some(TypeSpec::Resource(resource_name)) => Ok(Some(TypeSpec::Resource(
                self.plan_operation_resource_type(service, operation, resource_name)?,
            ))),
            Some(_) => Err(Error::InvalidWit {
                path: std::path::PathBuf::from("<api-plan>"),
                reason: format!("operation `{}` output must be a named type", operation.name),
            }),
            None => Ok(None),
        }
    }

    fn plan_operation_resource_type(
        &mut self,
        service: &ServiceSpec<OperationLoweredFamily>,
        operation: &OperationSpec<OperationLoweredFamily>,
        resource: &AuthoredResourceType,
    ) -> Result<PlannedResourceType> {
        let Some(output_type) = resource.wire_type.as_ref() else {
            return Ok(PlannedResourceType {
                type_name: resource.as_str().to_upper_camel_case(),
                wire_type: None,
            });
        };

        let wire_type = match output_type {
            ExternalTypeSpec::Proto(output_proto) => {
                let output_message =
                    self.descriptors
                        .message(output_proto.as_str())
                        .ok_or_else(|| Error::UnknownOperationOutputProto {
                            service: service.name.clone(),
                            operation: operation.name.clone(),
                            type_name: output_proto.as_str().to_string(),
                        })?;
                ExternalTypeSpec::Proto(PlannedProtoType::Message(
                    proto::planned_message_reference(output_message, self),
                ))
            }
            ExternalTypeSpec::Json(json_type) => {
                ExternalTypeSpec::Json(self.map_json_type(json_type))
            }
            ExternalTypeSpec::Alias { name, .. } => {
                return Err(Error::UnknownOperationOutputProto {
                    service: service.name.clone(),
                    operation: operation.name.clone(),
                    type_name: name.as_str().to_string(),
                });
            }
        };

        Ok(PlannedResourceType {
            type_name: resource.as_str().to_upper_camel_case(),
            wire_type: Some(wire_type),
        })
    }

    fn plan_resource(
        &mut self,
        service: &ServiceSpec<OperationLoweredFamily>,
        resource: &ResolvedResourceSpec,
        operations: &[OperationBindingInfo<'_>],
    ) -> Result<PlannedResource> {
        let methods = resource
            .methods
            .iter()
            .map(|method| {
                let binding = match &method.binding {
                    ResolvedResourceMethodBinding::Operation {
                        operation_name,
                        request_plan,
                    } => {
                        let operation = operations
                            .iter()
                            .find(|operation| operation.name == operation_name)
                            .ok_or_else(|| Error::InvalidResourceMethod {
                                service: service.name.clone(),
                                resource: resource.name.to_upper_camel_case(),
                                method: method.name.to_string(),
                                reason: format!(
                                    "bound operation `{operation_name}` was not rendered"
                                ),
                            })?;
                        PlannedResourceMethodBindingSpec::Operation {
                            operation_name: operation.name.to_string(),
                            request_plan: request_plan.clone(),
                            direct_return: operation.direct_return,
                        }
                    }
                    ResolvedResourceMethodBinding::Stub => PlannedResourceMethodBindingSpec::Stub,
                };

                Ok(PlannedResourceMethod {
                    name: method.name.clone(),
                    params: method
                        .params
                        .iter()
                        .map(|field| self.planned_resource_field(field))
                        .collect(),
                    result: method
                        .result
                        .as_ref()
                        .map(|result| self.planned_resource_method_result(result)),
                    binding,
                })
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(PlannedResource {
            name: resource.name.clone(),
            type_name: resource.name.to_upper_camel_case(),
            fields: resource
                .fields
                .iter()
                .map(|field| self.planned_resource_field(field))
                .collect(),
            methods,
        })
    }

    fn planned_resource_method_result(
        &mut self,
        result: &ResourceResultSpec<SelectedFamily>,
    ) -> PlannedResourceMethodResult {
        let optional = matches!(result.result_type, TypeSpec::Option(_));
        let kind = match result.result_type.without_option() {
            TypeSpec::Resource(resource) => PlannedResourceMethodResultKind::Resource {
                type_name: resource.as_str().to_upper_camel_case(),
            },
            _ => PlannedResourceMethodResultKind::Value(
                self.planned_selected_type_from_authored(&result.result_type),
            ),
        };
        PlannedResourceMethodResult { kind, optional }
    }

    fn planned_resource_field(
        &mut self,
        field: &ResourceFieldSpec<SelectedFamily>,
    ) -> PlannedResourceField {
        let kind = self.planned_selected_type_from_authored(&field.field_type);
        PlannedResourceField {
            name: field.name.clone(),
            optional: field.optional,
            kind,
            function: field
                .function
                .as_ref()
                .map(|function| self.planned_selected_function_from_authored(function)),
        }
    }

    pub(super) fn plan_record_type(
        &mut self,
        record: &RecordSpec<OperationLoweredFamily>,
    ) -> PlannedRecordType {
        let planned_record = PlannedRecordType {
            full_name: record.full_name.clone(),
            model_name: record.name.clone(),
        };
        self.ensure_record_model_plan(record);
        planned_record
    }

    fn ensure_record_model_plan(&mut self, record: &RecordSpec<OperationLoweredFamily>) {
        if self.record_plans.contains_key(&record.full_name) {
            return;
        }

        self.insert_record_plan(
            record.full_name.clone(),
            PlannedRecordData {
                proto: proto::record_proto_info(record, &self.spec, self.descriptors),
            },
        );
    }

    fn planned_external_type_from_resource(
        &mut self,
        authored_type: &ExternalTypeSpec,
    ) -> ExternalTypeSpec<PlannedFamily> {
        match authored_type {
            ExternalTypeSpec::Proto(proto_name) => {
                let planned_proto = if let Some(message) =
                    self.descriptors.message(proto_name.as_str())
                {
                    PlannedProtoType::Message(proto::planned_message_reference(message, self))
                } else if let Some(enumeration) = self.descriptors.enumeration(proto_name.as_str())
                {
                    PlannedProtoType::Enum(proto::planned_enum_reference(enumeration, &self.spec))
                } else {
                    panic!("proto `{proto_name}` should resolve during planning");
                };
                ExternalTypeSpec::Proto(planned_proto)
            }
            ExternalTypeSpec::Json(json_type) => {
                ExternalTypeSpec::Json(self.map_json_type(json_type))
            }
            ExternalTypeSpec::Alias {
                name,
                target,
                type_name,
            } => ExternalTypeSpec::Alias {
                name: PlannedAliasType {
                    name: name.as_str().to_string(),
                },
                target: Box::new(self.planned_type_from_resource(target)),
                type_name: LanguageStringSpec {
                    default: type_name.for_language(self.language).map(ToOwned::to_owned),
                    default_import: type_name
                        .import_for_language(self.language)
                        .map(ToOwned::to_owned),
                    ..Default::default()
                },
            },
        }
    }

    fn planned_type_from_resource(&mut self, authored_type: &TypeSpec) -> PlannedType {
        match authored_type {
            TypeSpec::External(external) => {
                TypeSpec::External(self.planned_external_type_from_resource(external))
            }
            TypeSpec::Option(inner) => {
                TypeSpec::Option(Box::new(self.planned_type_from_resource(inner)))
            }
            TypeSpec::List(inner) => {
                TypeSpec::List(Box::new(self.planned_type_from_resource(inner)))
            }
            TypeSpec::Tuple(items) => TypeSpec::Tuple(
                items
                    .iter()
                    .map(|item| self.planned_type_from_resource(item))
                    .collect(),
            ),
            TypeSpec::Map(key, value) => TypeSpec::Map(
                Box::new(self.planned_type_from_resource(key)),
                Box::new(self.planned_type_from_resource(value)),
            ),
            TypeSpec::Result { ok, err } => TypeSpec::Result {
                ok: ok
                    .as_deref()
                    .map(|item| Box::new(self.planned_type_from_resource(item))),
                err: err
                    .as_deref()
                    .map(|item| Box::new(self.planned_type_from_resource(item))),
            },
            TypeSpec::Record(name) => {
                self.planned_type_from_authored(&TypeSpec::Record(name.clone()))
            }
            TypeSpec::Enum(name) => self.planned_type_from_authored(&TypeSpec::Enum(name.clone())),
            TypeSpec::Flags(name) => {
                self.planned_type_from_authored(&TypeSpec::Flags(name.clone()))
            }
            TypeSpec::Variant(name) => {
                self.planned_type_from_authored(&TypeSpec::Variant(name.clone()))
            }
            TypeSpec::Resource(resource) => {
                TypeSpec::Resource(self.planned_resource_type_from_authored(resource))
            }
            TypeSpec::TypeParameter(parameter) => TypeSpec::TypeParameter(parameter.clone()),
            TypeSpec::Bool => TypeSpec::Bool,
            TypeSpec::Int(int) => TypeSpec::Int(*int),
            TypeSpec::Float => TypeSpec::Float,
            TypeSpec::String => TypeSpec::String,
            TypeSpec::Bytes => TypeSpec::Bytes,
        }
    }

    fn planned_resource_type_from_authored(
        &mut self,
        resource: &AuthoredResourceType,
    ) -> PlannedResourceType {
        PlannedResourceType {
            type_name: resource.as_str().to_upper_camel_case(),
            wire_type: resource
                .wire_type
                .as_ref()
                .map(|wire_type| self.planned_external_type_from_resource(wire_type)),
        }
    }

    pub(super) fn planned_type_from_authored(
        &mut self,
        authored_type: &TypeSpec<OperationLoweredFamily>,
    ) -> PlannedType {
        if let Some(kind) = proto::planned_type_from_authored_proto(authored_type, self) {
            return kind;
        }

        match authored_type {
            TypeSpec::Option(inner) => self.planned_type_from_authored(inner),
            TypeSpec::List(inner) => TypeSpec::List(Box::new(
                self.planned_type_from_authored(inner.without_option()),
            )),
            TypeSpec::Map(key, value) => TypeSpec::Map(
                Box::new(self.planned_type_from_authored(key.without_option())),
                Box::new(self.planned_type_from_authored(value.without_option())),
            ),
            _ => self.planned_value_type_from_authored(authored_type.without_option()),
        }
    }

    fn planned_selected_type_from_authored(
        &mut self,
        authored_type: &TypeSpec<SelectedFamily>,
    ) -> PlannedType {
        self.planned_type_from_authored(
            &authored_type
                .clone()
                .map_names(SelectedToOperationLoweredMapper),
        )
    }

    fn planned_selected_function_from_authored(
        &mut self,
        function: &FunctionFieldSpec<SelectedFamily>,
    ) -> FunctionFieldSpec<PlannedFamily> {
        FunctionFieldSpec {
            primary: function.primary,
            result: match &function.result {
                FunctionResultSpec::Annotation(annotation) => {
                    FunctionResultSpec::Annotation(materialize_selected_text(annotation))
                }
                FunctionResultSpec::Authored(authored_type) => FunctionResultSpec::Authored(
                    self.planned_selected_type_from_authored(authored_type),
                ),
            },
            args_field: function.args_field.clone(),
            arg_fields: function.arg_fields.clone(),
            args: match &function.args {
                FunctionArgsSpec::Varargs {
                    prefix,
                    typescript_drop_prefix,
                } => FunctionArgsSpec::Varargs {
                    prefix: prefix
                        .iter()
                        .map(|arg| FunctionArgSpec {
                            name: arg.name.clone(),
                            field_type: self.planned_selected_type_from_authored(&arg.field_type),
                        })
                        .collect(),
                    typescript_drop_prefix: *typescript_drop_prefix,
                },
                FunctionArgsSpec::Fixed(args) => FunctionArgsSpec::Fixed(
                    args.iter()
                        .map(|arg| FunctionArgSpec {
                            name: arg.name.clone(),
                            field_type: self.planned_selected_type_from_authored(&arg.field_type),
                        })
                        .collect(),
                ),
            },
            alternate_type: function
                .alternate_type
                .as_ref()
                .map(|ty| self.planned_selected_type_from_authored(ty)),
            converter: function.converter.clone(),
            name_extractor: function.name_extractor.clone(),
            call_extractor: function.call_extractor.clone(),
            result_type_parameter: function.result_type_parameter.clone(),
            type_descriptor: function.type_descriptor.as_ref().map(|descriptor| {
                FunctionTypeDescriptorSpec {
                    value_type: materialize_selected_text(&descriptor.value_type),
                    args_type: materialize_selected_text(&descriptor.args_type),
                }
            }),
        }
    }

    pub(super) fn planned_authored_type_override_from_authored(
        &mut self,
        authored_type: &TypeSpec<OperationLoweredFamily>,
    ) -> PlannedType {
        match authored_type {
            TypeSpec::Option(inner) => TypeSpec::Option(Box::new(
                self.planned_authored_type_override_from_authored(inner),
            )),
            TypeSpec::List(inner) => TypeSpec::List(Box::new(
                self.planned_authored_type_override_from_authored(inner),
            )),
            TypeSpec::Tuple(items) => TypeSpec::Tuple(
                items
                    .iter()
                    .map(|item| self.planned_authored_type_override_from_authored(item))
                    .collect(),
            ),
            TypeSpec::Map(key, value) => TypeSpec::Map(
                Box::new(self.planned_authored_type_override_from_authored(key)),
                Box::new(self.planned_authored_type_override_from_authored(value)),
            ),
            TypeSpec::Result { ok, err } => TypeSpec::Result {
                ok: ok
                    .as_ref()
                    .map(|ok| Box::new(self.planned_authored_type_override_from_authored(ok))),
                err: err
                    .as_ref()
                    .map(|err| Box::new(self.planned_authored_type_override_from_authored(err))),
            },
            TypeSpec::External(ExternalTypeSpec::Alias {
                name,
                target,
                type_name,
            }) => TypeSpec::External(ExternalTypeSpec::Alias {
                name: PlannedAliasType {
                    name: name.as_str().to_string(),
                },
                target: Box::new(self.planned_authored_type_override_from_authored(target)),
                type_name: materialize_selected_text(type_name),
            }),
            TypeSpec::External(ExternalTypeSpec::Json(json_type)) => {
                TypeSpec::External(ExternalTypeSpec::Json(self.map_json_type(json_type)))
            }
            _ => self.planned_value_type_from_authored(authored_type),
        }
    }

    fn planned_value_type_from_authored(
        &mut self,
        authored_type: &TypeSpec<OperationLoweredFamily>,
    ) -> PlannedType {
        match authored_type {
            TypeSpec::Bool => TypeSpec::Bool,
            TypeSpec::Int(int) => TypeSpec::Int(*int),
            TypeSpec::Float => TypeSpec::Float,
            TypeSpec::String => TypeSpec::String,
            TypeSpec::Bytes => TypeSpec::Bytes,
            TypeSpec::TypeParameter(parameter) => TypeSpec::TypeParameter(parameter.clone()),
            TypeSpec::External(ExternalTypeSpec::Proto(proto_name)) => {
                proto::planned_value_type_from_authored_proto(proto_name.as_str(), self)
            }
            TypeSpec::External(ExternalTypeSpec::Json(json_type)) => {
                TypeSpec::External(ExternalTypeSpec::Json(self.map_json_type(json_type)))
            }
            TypeSpec::Record(record_name) => self
                .spec
                .record(record_name.as_str())
                .cloned()
                .map(|record| TypeSpec::Record(self.plan_record_type(&record)))
                .unwrap_or(TypeSpec::String),
            TypeSpec::Enum(enum_name) => self
                .spec
                .enum_decl(enum_name.as_str())
                .cloned()
                .map(|enumeration| {
                    TypeSpec::Enum(PlannedEnumType {
                        full_name: enumeration.full_name.clone(),
                        name: enumeration.name.clone(),
                    })
                })
                .unwrap_or(TypeSpec::String),
            TypeSpec::Flags(flags_name) => self
                .spec
                .flags_decl(flags_name.as_str())
                .cloned()
                .map(|flags| {
                    TypeSpec::Flags(PlannedFlagsType {
                        full_name: flags.full_name.clone(),
                        name: flags.name.clone(),
                    })
                })
                .unwrap_or(TypeSpec::String),
            TypeSpec::Variant(variant_name) => self
                .spec
                .variant(variant_name.as_str())
                .cloned()
                .map(|variant| {
                    TypeSpec::Variant(PlannedVariantType {
                        full_name: variant.full_name.clone(),
                        name: variant.name.clone(),
                    })
                })
                .unwrap_or(TypeSpec::String),
            TypeSpec::Resource(resource_name) => TypeSpec::Resource(PlannedResourceType {
                type_name: resource_name.as_str().to_upper_camel_case(),
                wire_type: resource_name
                    .wire_type
                    .as_ref()
                    .map(|wire_type| self.planned_external_type_from_resource(wire_type)),
            }),
            TypeSpec::Option(inner) => {
                self.planned_value_type_from_authored(inner.without_option())
            }
            TypeSpec::Tuple(items) => TypeSpec::Tuple(
                items
                    .iter()
                    .map(|item| self.planned_value_type_from_authored(item))
                    .collect(),
            ),
            TypeSpec::Result { ok, err } => TypeSpec::Result {
                ok: ok
                    .as_ref()
                    .map(|ok| Box::new(self.planned_value_type_from_authored(ok))),
                err: err
                    .as_ref()
                    .map(|err| Box::new(self.planned_value_type_from_authored(err))),
            },
            TypeSpec::External(ExternalTypeSpec::Alias {
                name,
                target,
                type_name,
            }) => {
                let fallback = self.planned_value_type_from_authored(target.without_option());
                TypeSpec::External(ExternalTypeSpec::Alias {
                    name: PlannedAliasType {
                        name: name.as_str().to_string(),
                    },
                    target: Box::new(fallback),
                    type_name: materialize_selected_text(type_name),
                })
            }
            TypeSpec::List(inner) => TypeSpec::List(Box::new(
                self.planned_type_from_authored(inner.without_option()),
            )),
            TypeSpec::Map(key, value) => TypeSpec::Map(
                Box::new(self.planned_type_from_authored(key.without_option())),
                Box::new(self.planned_type_from_authored(value.without_option())),
            ),
        }
    }

    fn map_json_type(&mut self, json_type: &JsonModelSpec<Symbol>) -> PlannedJsonType {
        PlannedJsonType {
            full_name: json_type.name.as_str().to_string(),
            module_path: json_type.name.module_path().cloned(),
            model_name: json_type.model_name.clone(),
            schema: json_type.schema.clone(),
        }
    }
}

fn plan_operation_resource_return(
    output_resource_return: Option<&ResolvedResourceReturnSpec>,
) -> Option<PlannedOperationResourceReturn> {
    output_resource_return.map(|resource_return| PlannedOperationResourceReturn {
        resource_type_name: resource_return.resource_name.to_upper_camel_case(),
        bindings: resource_return
            .bindings
            .iter()
            .map(|binding| PlannedOperationResourceFieldBinding {
                field_name: binding.field_name.clone(),
                optional: binding.optional,
                source: binding.source.clone(),
            })
            .collect(),
    })
}

pub(crate) struct TypePlanningPass<'a> {
    imports: BTreeMap<ModulePath, BTreeMap<ModulePath, BTreeSet<String>>>,
    descriptors: &'a DescriptorIndex,
    language: Language,
}

impl<'a> TypePlanningPass<'a> {
    pub(crate) fn new(
        tree: &ApiSpecTree<OperationLoweredFamily>,
        descriptors: &'a DescriptorIndex,
        language: Language,
    ) -> Self {
        Self {
            imports: module_import_index(tree),
            descriptors,
            language,
        }
    }
}

impl CompilerPass<OperationLoweredFamily, PlannedFamily> for TypePlanningPass<'_> {
    type Error = Error;

    fn transform_leaf(
        &mut self,
        leaf: ApiSpecLeaf<OperationLoweredFamily>,
    ) -> Result<ApiSpecLeaf<PlannedFamily>> {
        let spec_data = PlannedSpecData {
            module_imports: self
                .imports
                .get(&leaf.module_path)
                .cloned()
                .unwrap_or_default(),
            // Resolved later, by `EmittedNameResolutionPass`: naming a model in
            // another module needs the tree-wide name manifest.
            cross_module_model_names: BTreeMap::new(),
        };
        let mut planner = TypePlanningContext::new(
            leaf.spec.clone(),
            spec_data.clone(),
            self.descriptors,
            self.language,
        )?;
        let lowered = planner.plan_operations()?;
        let source_spec = planner.spec.clone();
        let mut planned = planner.plan_spec(source_spec);
        for service in &mut planned.services {
            if let Some(operations) = lowered.operations.get(&service.name) {
                service.operations = operations.clone();
            }
        }
        Ok(ApiSpecLeaf {
            module_path: leaf.module_path,
            source_root: leaf.source_root,
            source_path: leaf.source_path,
            spec: planned,
        })
    }
}

struct SelectedToOperationLoweredMapper;

impl ApiSpecTransform<SelectedFamily, OperationLoweredFamily> for SelectedToOperationLoweredMapper {
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
    fn map_support(&mut self, value: SelectedSupportSpec) -> SelectedSupportSpec {
        value
    }
}

impl TypePlanningContext<'_> {
    fn plan_operations(&mut self) -> Result<PlannedOperations> {
        let services = self.spec.services.clone();
        let mut operations = IndexMap::new();
        for service in &services {
            let service_plan = self.plan_service(service)?;
            operations.insert(service.name.clone(), service_plan.operations);
        }
        Ok(PlannedOperations { operations })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use super::*;
    use crate::language::Language;
    use crate::spec::{ApiSpecLeaf, ApiSpecNode, ApiSpecTree, CompilerPass};
    use crate::spec::{ExternalTypeSpec, LanguageStringSpec, ModulePath, SupportSpec};

    #[test]
    fn descriptor_proto_refs_remain_external_models() {
        let proto_name = "temporal.api.common.v1.RetryPolicy";
        let plan = plan_single_leaf(proto_operation_spec(proto_name));

        assert!(plan.records().next().is_none());
        let operation = &plan.services[0].operations[0];
        for model_type in [operation.input_type(), operation.output_type()] {
            let Some(TypeSpec::External(ExternalTypeSpec::Proto(PlannedProtoType::Message(
                message,
            )))) = model_type
            else {
                panic!("descriptor-only proto reference should stay external");
            };
            assert_eq!(message.proto.full_name, proto_name);
        }
    }

    #[test]
    fn grouped_oneof_plans_as_an_ordinary_variant_field() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let spec = crate::parser::load_api_spec_from_wit_for_language_with_inputs(
            Language::Python,
            &[
                root.join("advanced/samples/inputs/proto-oneof.wit"),
                root.join("advanced/samples/inputs/deps"),
            ],
        )
        .unwrap();
        let plan = plan_single_leaf(spec);
        let record = plan
            .records()
            .map(|(_, record)| record)
            .find(|record| {
                record
                    .data
                    .proto
                    .as_ref()
                    .is_some_and(|proto| proto.full_name == "temporal.api.update.v1.Outcome")
            })
            .expect("proto-backed outcome record should be planned");
        assert_eq!(
            record
                .data
                .proto
                .as_ref()
                .map(|proto| proto.full_name.as_str()),
            Some("temporal.api.update.v1.Outcome")
        );
        let field = record.fields.get("value").expect("grouped field");
        assert!(matches!(field.field_type, PlannedType::Variant(_)));
        assert_eq!(field.data.has_presence, Some(true));
        let Some(super::PlannedWireFieldBinding::VariantMembers { wire_name, members }) =
            &field.data.wire_binding
        else {
            panic!("grouped field should retain its wire variant members");
        };
        assert_eq!(wire_name, "value");
        assert_eq!(
            members
                .iter()
                .map(|member| member.wire_name.as_str())
                .collect::<Vec<_>>(),
            ["success", "failure"]
        );
        for (member, expected_type) in members.iter().zip([
            "temporal.api.common.v1.Payloads",
            "temporal.api.failure.v1.Failure",
        ]) {
            let PlannedType::External(ExternalTypeSpec::Proto(PlannedProtoType::Message(message))) =
                &member.wire_type
            else {
                panic!("oneof member should retain its external wire type");
            };
            assert_eq!(message.proto.full_name, expected_type);
        }
    }

    fn plan_single_leaf(spec: ApiSpec) -> PlannedSpec {
        let descriptors = DescriptorIndex::load(
            &PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("advanced/samples/descriptors/temporal_api.bin"),
        )
        .unwrap();
        let authored_tree = ApiSpecTree {
            root: ApiSpecNode::Leaf(ApiSpecLeaf {
                module_path: ModulePath::default(),
                source_root: PathBuf::new(),
                source_path: PathBuf::new(),
                spec: super::super::select_spec(spec, Language::Python),
            }),
        };
        let resource_resolved = super::super::ResourceResolutionPass::new(
            &descriptors,
            super::super::PlanningMode::DefinitionsOnly,
        )
        .apply(authored_tree)
        .unwrap();
        let operation_bound = super::super::OperationBindingPass::new()
            .apply(resource_resolved)
            .unwrap();
        let operation_lowered = super::super::OperationLoweringPass::new()
            .apply(operation_bound)
            .unwrap();
        let planned = TypePlanningPass::new(&operation_lowered, &descriptors, Language::Python)
            .apply(operation_lowered)
            .unwrap();
        let ApiSpecNode::Leaf(leaf) = planned.root else {
            panic!("test helper expects one leaf");
        };
        leaf.spec
    }

    fn proto_operation_spec(proto_name: &str) -> ApiSpec {
        ApiSpec {
            module_path: ModulePath::default(),
            data: (),
            version: "1.0.0".to_string(),
            support: SupportSpec::default(),
            services: vec![ServiceSpec {
                name: "example-service".to_string(),
                code_name: LanguageStringSpec::default(),
                wire_name: "ExampleService".to_string(),
                doc: LanguageStringSpec::default(),
                namespace: LanguageStringSpec::default(),
                operations_class: LanguageStringSpec::default(),
                endpoint: Some("example".to_string()),
                experimental: false,
                deprecated: false,
                delay_load_temporalio_workflow: false,
                operations: vec![OperationSpec {
                    name: "example-operation".to_string(),
                    code_name: LanguageStringSpec::default(),
                    wire_name: "ExampleOperation".to_string(),
                    experimental: false,
                    deprecated: false,
                    doc: LanguageStringSpec::default(),
                    return_doc: LanguageStringSpec::default(),
                    input: Some(TypeSpec::External(ExternalTypeSpec::Proto(Symbol::new(
                        proto_name,
                    )))),
                    output: Some(TypeSpec::External(ExternalTypeSpec::Proto(Symbol::new(
                        proto_name,
                    )))),
                    output_transform: None,
                    serialization_context: LanguageStringSpec::default(),
                    data: (),
                }],
                resources: Vec::new(),
                data: (),
            }],
            types: BTreeMap::new(),
        }
    }
}
