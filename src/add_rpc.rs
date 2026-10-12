use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use heck::{ToKebabCase, ToSnakeCase, ToUpperCamelCase};
use prost_types::FieldDescriptorProto;
use prost_types::field_descriptor_proto::{Label, Type};
use wit_parser_crate::{
    Function, FunctionKind, Interface, Record, Resolve, Type as WitType, TypeDefKind, TypeId,
    WorldItem, WorldKey,
};

use crate::descriptors::{
    DescriptorIndex, EnumMetadata, MessageMetadata, ProtoOneofGroup, RpcMetadata, real_oneof_groups,
};
use crate::error::{Error, Result};
use crate::parser::{
    directive, directive_value, find_proto_name_for_type, find_proto_name_for_type_def,
    parse_directives, parse_wit_with_inputs, resolve_function_signature_args, select_world,
    wire_operation_name_from_docs,
};

const DEFAULT_PACKAGE_NAME: &str = "temporal:nexus@1.0.0";
const DEFAULT_WORLD_NAME: &str = "system";
const DEFAULT_ENDPOINT_PLACEHOLDER: &str = "__REPLACE_ME__";
const LINKED_WIT_ROOT: &str = "package temporary:root@0.0.0;\n\nworld system {}\n";
const LINKED_WIT_ROOT_PATH: &str = "add-rpc-linked-root.wit";

#[derive(Debug, Clone, PartialEq, Eq)]
struct LinkedTypeMetadata {
    wit_name: String,
    record_fields: Option<BTreeMap<String, LinkedRecordFieldMetadata>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProtoOneofSource {
    message: String,
    name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OneofMetadata {
    source: ProtoOneofSource,
    cases: BTreeMap<String, Option<String>>,
}

impl OneofMetadata {
    fn from_resolve(
        resolve: &Resolve,
        type_id: TypeId,
        proto_name: &str,
        path: &Path,
        context: &str,
    ) -> Result<Self> {
        let TypeDefKind::Variant(variant) = &resolve.types[type_id].kind else {
            unreachable!("oneof metadata is collected only for variants");
        };
        Ok(Self {
            source: parse_proto_oneof_source(proto_name, path, context)?,
            cases: variant
                .cases
                .iter()
                .map(|case| {
                    (
                        case.name.to_snake_case(),
                        case.ty.as_ref().map(|ty| render_wit_type(resolve, ty)),
                    )
                })
                .collect(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LinkedRecordFieldMetadata {
    wit_name: String,
    type_expr: String,
    explicit_proto_field: bool,
    omitted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LinkedWitMetadata {
    proto_types: BTreeMap<String, LinkedTypeMetadata>,
    oneofs_by_variant: BTreeMap<String, OneofMetadata>,
    type_parameters: BTreeSet<String>,
    type_compatibility: BTreeMap<String, BTreeSet<String>>,
    type_covered_fields: BTreeMap<String, BTreeSet<String>>,
    type_use_paths: BTreeMap<String, String>,
}

pub struct AddRpcRequest {
    pub descriptor_paths: Vec<PathBuf>,
    pub rpc_name: String,
    pub input_paths: Vec<PathBuf>,
    pub output_path: Option<PathBuf>,
}

pub struct AddMessageRequest {
    pub descriptor_paths: Vec<PathBuf>,
    pub message_names: Vec<String>,
    pub file_names: Vec<String>,
    pub input_paths: Vec<PathBuf>,
    pub output_path: Option<PathBuf>,
}

pub fn add_rpc_to_string(
    descriptor_paths: &[PathBuf],
    rpc_name: &str,
    input_paths: &[PathBuf],
) -> Result<String> {
    let descriptors = DescriptorIndex::load_many(descriptor_paths)?;
    let (input_path, linked_input_paths) = add_rpc_input_parts(input_paths);
    let output = if let Some(input_path) = input_path {
        let input = fs::read_to_string(input_path).map_err(|source| Error::ReadFile {
            path: input_path.to_path_buf(),
            source,
        })?;
        generate_add_rpc_wit_with_input(
            &descriptors,
            rpc_name,
            input_path,
            &input,
            linked_input_paths,
        )?
    } else {
        generate_add_rpc_wit(&descriptors, rpc_name, input_paths)?
    };
    validate_scaffold_output(output, input_path, linked_input_paths)
}

pub fn add_rpc_to_file(request: &AddRpcRequest) -> Result<()> {
    let output = add_rpc_to_string(
        &request.descriptor_paths,
        &request.rpc_name,
        &request.input_paths,
    )?;
    if let Some(path) = &request.output_path {
        fs::write(path, output).map_err(|source| Error::WriteFile {
            path: path.clone(),
            source,
        })?;
    } else {
        print!("{output}");
    }
    Ok(())
}

pub fn add_message_to_string(
    descriptor_paths: &[PathBuf],
    message_name: &str,
    input_paths: &[PathBuf],
) -> Result<String> {
    add_messages_to_string(
        descriptor_paths,
        &[message_name.to_string()],
        &[],
        input_paths,
    )
}

pub fn add_messages_to_string(
    descriptor_paths: &[PathBuf],
    message_names: &[String],
    file_names: &[String],
    input_paths: &[PathBuf],
) -> Result<String> {
    let descriptors = DescriptorIndex::load_many(descriptor_paths)?;
    let mut selected = BTreeMap::new();
    for name in message_names {
        let message = descriptors.resolve_message(name)?;
        selected.insert(message.full_name.clone(), message);
    }
    for file_name in file_names {
        for message in descriptors.messages_in_file(file_name)? {
            selected.insert(message.full_name.clone(), message);
        }
    }
    if selected.is_empty() {
        return Err(Error::UnsupportedAddMessage {
            context: "selection".to_string(),
            reason: "select at least one message with --message or a proto file containing messages with --file".to_string(),
        });
    }
    let messages = selected.into_values().collect::<Vec<_>>();
    let (input_path, linked_input_paths) = add_rpc_input_parts(input_paths);
    let output = if let Some(input_path) = input_path {
        let input = fs::read_to_string(input_path).map_err(|source| Error::ReadFile {
            path: input_path.to_path_buf(),
            source,
        })?;
        generate_add_message_wit_with_input(
            &descriptors,
            &messages,
            input_path,
            &input,
            linked_input_paths,
        )?
    } else {
        generate_add_message_wit(&descriptors, &messages, input_paths)?
    };
    validate_scaffold_output(output, input_path, linked_input_paths)
}

fn validate_scaffold_output(
    output: String,
    input_path: Option<&Path>,
    linked_input_paths: &[PathBuf],
) -> Result<String> {
    parse_wit_with_inputs(
        &output,
        input_path.unwrap_or_else(|| Path::new("proto-scaffold.wit")),
        linked_input_paths,
    )?;
    Ok(output)
}

pub fn add_message_to_file(request: &AddMessageRequest) -> Result<()> {
    let output = add_messages_to_string(
        &request.descriptor_paths,
        &request.message_names,
        &request.file_names,
        &request.input_paths,
    )?;
    if let Some(path) = &request.output_path {
        fs::write(path, output).map_err(|source| Error::WriteFile {
            path: path.clone(),
            source,
        })?;
    } else {
        print!("{output}");
    }
    Ok(())
}

fn add_rpc_input_parts(input_paths: &[PathBuf]) -> (Option<&Path>, &[PathBuf]) {
    if let Some((first, rest)) = input_paths.split_first()
        && (first.is_file()
            || first
                .extension()
                .is_some_and(|extension| extension == "wit"))
    {
        return (Some(first.as_path()), rest);
    }
    (None, input_paths)
}

pub fn generate_add_rpc_wit(
    descriptors: &DescriptorIndex,
    rpc_name: &str,
    input_paths: &[PathBuf],
) -> Result<String> {
    let rpc = descriptors.resolve_rpc(rpc_name)?;
    let linked_wit = load_linked_wit_metadata_from_inputs(input_paths)?;
    ProtoWitBuilder::for_rpc(descriptors, rpc, linked_wit)
        .build()
        .map(|rendered| rendered.render_standalone())
}

fn load_linked_wit_metadata_from_inputs(input_paths: &[PathBuf]) -> Result<LinkedWitMetadata> {
    let parsed = parse_wit_with_inputs(
        LINKED_WIT_ROOT,
        Path::new(LINKED_WIT_ROOT_PATH),
        input_paths,
    )?;
    let mut proto_types = BTreeMap::new();
    let mut oneofs_by_variant = BTreeMap::new();
    let mut type_parameters = BTreeSet::new();
    let mut type_compatibility = BTreeMap::<String, BTreeSet<String>>::new();
    let mut type_covered_fields = BTreeMap::<String, BTreeSet<String>>::new();
    let mut type_use_paths = BTreeMap::new();

    for (package_id, package) in parsed.resolve.packages.iter() {
        if package_id == parsed.package_id {
            continue;
        }

        let package_name = if let Some(version) = &package.name.version {
            format!(
                "{}:{}@{}",
                package.name.namespace, package.name.name, version
            )
        } else {
            format!("{}:{}", package.name.namespace, package.name.name)
        };
        let origin_path = parsed
            .package_origins
            .get(&package_id)
            .cloned()
            .or_else(|| input_paths.first().cloned())
            .unwrap_or_else(|| PathBuf::from("<input>"));

        for interface_id in package.interfaces.values() {
            let interface = &parsed.resolve.interfaces[*interface_id];
            let Some(interface_name) = interface.name.as_deref() else {
                continue;
            };
            let use_path = if let Some(version) = &package.name.version {
                format!(
                    "{}:{}/{}@{}",
                    package.name.namespace, package.name.name, interface_name, version
                )
            } else {
                format!(
                    "{}:{}/{}",
                    package.name.namespace, package.name.name, interface_name
                )
            };

            for type_id in interface.types.values() {
                let type_def = &parsed.resolve.types[*type_id];
                let Some(type_name) = type_def.name.as_deref() else {
                    continue;
                };
                let context =
                    format!("linked WIT type `{package_name}.{interface_name}.{type_name}`");
                let directives =
                    parse_directives(type_def.docs.contents.as_deref(), &origin_path, &context)?;
                if directive(&directives, "type-parameter", &origin_path, &context)?.is_some() {
                    type_parameters.insert(type_name.to_string());
                }

                for directive in &directives {
                    if directive.name() != "add-rpc-compatible-with" {
                        continue;
                    }
                    let Some(target) = directive.value("value") else {
                        return Err(Error::InvalidWitDirective {
                            path: origin_path.join("model.wit"),
                            context: context.clone(),
                            directive: "@nexus.add-rpc-compatible-with".to_string(),
                            reason: "missing compatibility target".to_string(),
                        });
                    };
                    type_compatibility
                        .entry(type_name.to_string())
                        .or_default()
                        .insert(target.to_string());
                }

                for directive in &directives {
                    if directive.name() != "function" {
                        continue;
                    }
                    let Some(signature_name) = directive.value("signature") else {
                        continue;
                    };
                    let covered_field = if let Some(args_field) = directive.value("args-field") {
                        args_field.to_string()
                    } else {
                        resolve_function_signature_args(
                            &parsed.resolve,
                            type_def,
                            signature_name,
                            &origin_path,
                            &context,
                        )?
                        .0
                    }
                    .replace('-', "_");
                    type_covered_fields
                        .entry(type_name.to_string())
                        .or_default()
                        .insert(covered_field);
                }

                if let Some(existing) =
                    type_use_paths.insert(type_name.to_string(), use_path.clone())
                    && existing != use_path
                {
                    return Err(Error::InvalidWit {
                        path: origin_path.join("model.wit"),
                        reason: format!(
                            "linked WIT type `{type_name}` is declared under multiple use paths"
                        ),
                    });
                }

                let Some(proto_name) =
                    directive_value(&directives, "proto", &origin_path, &context, "value")?
                else {
                    continue;
                };

                if matches!(type_def.kind, TypeDefKind::Variant(_)) {
                    oneofs_by_variant.insert(
                        type_name.to_string(),
                        OneofMetadata::from_resolve(
                            &parsed.resolve,
                            *type_id,
                            &proto_name,
                            &origin_path,
                            &context,
                        )?,
                    );
                    continue;
                }

                let record_fields = if let TypeDefKind::Record(record) = &type_def.kind {
                    let mut fields = BTreeMap::new();
                    for field in &record.fields {
                        let field_context = format!("{context} field `{}`", field.name);
                        let field_directives = parse_directives(
                            field.docs.contents.as_deref(),
                            &origin_path,
                            &field_context,
                        )?;
                        let explicit_proto_name = directive_value(
                            &field_directives,
                            "proto-field",
                            &origin_path,
                            &field_context,
                            "value",
                        )?;
                        let field_proto_name = explicit_proto_name
                            .clone()
                            .unwrap_or_else(|| field.name.replace('-', "_"));
                        fields.insert(
                            field_proto_name,
                            LinkedRecordFieldMetadata {
                                wit_name: field.name.clone(),
                                type_expr: render_wit_type(&parsed.resolve, &field.ty),
                                explicit_proto_field: explicit_proto_name.is_some(),
                                omitted: directive(
                                    &field_directives,
                                    "omit",
                                    &origin_path,
                                    &field_context,
                                )?
                                .is_some(),
                            },
                        );
                    }
                    Some(fields)
                } else {
                    None
                };

                if let Some(existing) = proto_types.insert(
                    proto_name.clone(),
                    LinkedTypeMetadata {
                        wit_name: type_name.to_string(),
                        record_fields,
                    },
                ) {
                    return Err(Error::InvalidWit {
                        path: origin_path.join("model.wit"),
                        reason: format!(
                            "duplicate linked `@nexus.proto` mapping for `{proto_name}` (`{}` and `{type_name}`)",
                            existing.wit_name
                        ),
                    });
                }
            }
        }
    }

    Ok(LinkedWitMetadata {
        proto_types,
        oneofs_by_variant,
        type_parameters,
        type_compatibility,
        type_covered_fields,
        type_use_paths,
    })
}

fn generate_add_message_wit(
    descriptors: &DescriptorIndex,
    messages: &[&MessageMetadata],
    input_paths: &[PathBuf],
) -> Result<String> {
    let linked_wit = load_linked_wit_metadata_from_inputs(input_paths)?;
    MessageTreeBuilder::new(descriptors, messages, linked_wit)
        .build()
        .map(|rendered| rendered.render_standalone())
}

fn generate_add_message_wit_with_input(
    descriptors: &DescriptorIndex,
    messages: &[&MessageMetadata],
    input_path: &Path,
    input: &str,
    linked_input_paths: &[PathBuf],
) -> Result<String> {
    let linked_wit = load_linked_wit_metadata_from_inputs(linked_input_paths)?;
    let existing = ExistingWitDocument::load(input_path, input, linked_input_paths)?;
    if existing.interfaces.len() != 1 {
        return Err(Error::UnsupportedAddMessage {
            context: messages[0].full_name.clone(),
            reason: format!(
                "existing WIT world `{}` must export exactly one interface, but exports {}",
                existing.world_name,
                existing.interfaces.len()
            ),
        });
    }
    let interface = existing
        .interfaces
        .values()
        .next()
        .expect("checked for exactly one interface");
    let rendered = MessageTreeBuilder::new(descriptors, messages, linked_wit)
        .with_existing_interface(interface)
        .build()?;
    rendered.changes.apply(input, &interface.name)
}

pub fn generate_add_rpc_wit_with_input(
    descriptors: &DescriptorIndex,
    rpc_name: &str,
    input_path: &Path,
    input: &str,
    linked_input_paths: &[PathBuf],
) -> Result<String> {
    let rpc = descriptors.resolve_rpc(rpc_name)?;
    let linked_wit = load_linked_wit_metadata_from_inputs(linked_input_paths)?;
    let existing = ExistingWitDocument::load(input_path, input, linked_input_paths)?;
    let interface_name = rpc.service_name.to_kebab_case();
    let operation_name = rpc.name.to_kebab_case();

    if let Some(interface) = existing.interfaces.get(&interface_name) {
        if interface.function_names.contains(&operation_name) {
            let update = ProtoWitBuilder::for_rpc(descriptors, rpc, linked_wit)
                .with_existing_interface(interface)
                .build_existing_operation_update(interface, &operation_name)?;
            return update.apply(input, &interface_name);
        }
        if let Some(existing_operation_name) = interface.function_names_by_wire_name.get(&rpc.name)
        {
            let update = ProtoWitBuilder::for_rpc(descriptors, rpc, linked_wit)
                .with_existing_interface(interface)
                .build_existing_operation_update(interface, existing_operation_name)?;
            return update.apply(input, &interface_name);
        }

        let rendered = ProtoWitBuilder::for_rpc(descriptors, rpc, linked_wit)
            .with_existing_interface(interface)
            .build()?;
        let source = rendered.changes.apply(input, &interface_name)?;
        return insert_into_named_block(&source, "interface", &interface_name, &rendered.operation);
    }

    let rendered = ProtoWitBuilder::for_rpc(descriptors, rpc, linked_wit).build()?;
    let source = insert_world_export(input, &existing.world_name, &interface_name)?;
    let interface_block = rendered.render_new_interface_block();
    insert_after_named_block(&source, "world", &existing.world_name, &interface_block)
}

#[derive(Debug, Clone)]
struct ExistingWitDocument {
    world_name: String,
    interfaces: BTreeMap<String, ExistingInterface>,
}

impl ExistingWitDocument {
    fn load(path: &Path, input: &str, linked_input_paths: &[PathBuf]) -> Result<Self> {
        let parsed = parse_wit_with_inputs(input, path, linked_input_paths)?;
        let world_id = select_world(&parsed.resolve, parsed.package_id, path)?;
        let world = &parsed.resolve.worlds[world_id];

        let mut interfaces = BTreeMap::new();
        for (key, item) in &world.exports {
            let WorldItem::Interface { id, .. } = item else {
                continue;
            };
            let interface = &parsed.resolve.interfaces[*id];
            let export_name = exported_interface_name(key, interface);
            let interface_source = find_named_block(input, "interface", &export_name)
                .map(|block| &input[block.brace_start + 1..block.end_start]);
            interfaces.insert(
                export_name.clone(),
                ExistingInterface::from_resolve(
                    &parsed.resolve,
                    interface,
                    export_name,
                    path,
                    interface_source,
                )?,
            );
        }

        Ok(Self {
            world_name: world.name.clone(),
            interfaces,
        })
    }
}

#[derive(Debug, Clone, Default)]
struct ExistingInterface {
    name: String,
    function_names: BTreeSet<String>,
    function_names_by_wire_name: BTreeMap<String, String>,
    functions: BTreeMap<String, ExistingFunction>,
    records_by_proto: BTreeMap<String, ExistingRecord>,
    oneofs_by_variant: BTreeMap<String, OneofMetadata>,
    declared_type_names: BTreeSet<String>,
    type_names_by_proto: BTreeMap<String, String>,
    type_names_in_scope: BTreeSet<String>,
    type_parameters: BTreeSet<String>,
}

impl ExistingInterface {
    fn from_resolve(
        resolve: &Resolve,
        interface: &Interface,
        export_name: String,
        path: &Path,
        interface_source: Option<&str>,
    ) -> Result<Self> {
        let function_names = interface.functions.keys().cloned().collect();
        let mut function_names_by_wire_name = BTreeMap::new();
        let mut functions = BTreeMap::new();
        let mut type_names_by_proto = BTreeMap::new();
        let mut type_names_in_scope = BTreeSet::new();
        let mut type_parameters = BTreeSet::new();
        let mut records_by_proto = BTreeMap::new();
        let mut oneofs_by_variant = BTreeMap::new();
        let mut declared_type_names = BTreeSet::new();

        for (function_name, function) in &interface.functions {
            let context = format!("interface `{export_name}` function `{function_name}`");
            let function = ExistingFunction::from_resolve(resolve, function, path, &context)?;
            function_names_by_wire_name.insert(function.wire_name.clone(), function_name.clone());
            functions.insert(function_name.clone(), function);
        }

        for (type_name, type_id) in &interface.types {
            type_names_in_scope.insert(type_name.clone());
            declared_type_names.insert(type_name.clone());
            let type_def = &resolve.types[*type_id];
            let context = format!("interface `{export_name}` type `{type_name}`");
            let directives = parse_directives(type_def.docs.contents.as_deref(), path, &context)?;
            if directive(&directives, "type-parameter", path, &context)?.is_some() {
                type_parameters.insert(type_name.clone());
            }
            // Aliases use the metadata of their resolved type, never a linked
            // variant that happens to have the same name.
            let mut resolved_id = *type_id;
            while let TypeDefKind::Type(WitType::Id(target)) = &resolve.types[resolved_id].kind {
                resolved_id = *target;
                let resolved_directives = parse_directives(
                    resolve.types[resolved_id].docs.contents.as_deref(),
                    path,
                    &context,
                )?;
                if directive(&resolved_directives, "type-parameter", path, &context)?.is_some() {
                    type_parameters.insert(type_name.clone());
                }
            }
            if matches!(resolve.types[resolved_id].kind, TypeDefKind::Variant(_)) {
                if let Some(proto_name) =
                    find_proto_name_for_type(resolve, &WitType::Id(*type_id), path, &context)?
                {
                    oneofs_by_variant.insert(
                        type_name.clone(),
                        OneofMetadata::from_resolve(
                            resolve,
                            resolved_id,
                            &proto_name,
                            path,
                            &context,
                        )?,
                    );
                }
                continue;
            }
            let Some(proto_name) = find_proto_name_for_type_def(type_def, path, &context)? else {
                continue;
            };
            type_names_by_proto.insert(proto_name.clone(), type_name.clone());
            if let TypeDefKind::Record(record) = &type_def.kind {
                records_by_proto.insert(
                    proto_name,
                    ExistingRecord::from_resolve(resolve, type_name, record),
                );
            }
        }
        if let Some(interface_source) = interface_source {
            type_names_in_scope.extend(collect_used_type_names(interface_source));
        }

        Ok(Self {
            name: export_name,
            function_names,
            function_names_by_wire_name,
            functions,
            records_by_proto,
            oneofs_by_variant,
            declared_type_names,
            type_names_by_proto,
            type_names_in_scope,
            type_parameters,
        })
    }
}

#[derive(Debug, Clone)]
struct ExistingFunction {
    wire_name: String,
    parameter_name: Option<String>,
    input_type_name: Option<String>,
    input_proto: Option<String>,
    output_type_name: Option<String>,
    output_proto: Option<String>,
    params_len: usize,
    has_result: bool,
    freestanding: bool,
}

impl ExistingFunction {
    fn from_resolve(
        resolve: &Resolve,
        function: &Function,
        path: &Path,
        context: &str,
    ) -> Result<Self> {
        let wire_name = wire_operation_name_from_docs(
            function.docs.contents.as_deref(),
            path,
            context,
            &function.name.to_upper_camel_case(),
        )?;
        let (parameter_name, input_type_name, input_proto) =
            if let Some(parameter) = function.params.first() {
                (
                    Some(parameter.name.clone()),
                    wit_type_name(resolve, &parameter.ty),
                    find_proto_name_for_type(resolve, &parameter.ty, path, context)?,
                )
            } else {
                (None, None, None)
            };
        let (output_type_name, output_proto) = if let Some(output_type) = function.result.as_ref() {
            (
                wit_type_name(resolve, output_type),
                find_proto_name_for_type(resolve, output_type, path, context)?,
            )
        } else {
            (None, None)
        };

        Ok(Self {
            wire_name,
            parameter_name,
            input_type_name,
            input_proto,
            output_type_name,
            output_proto,
            params_len: function.params.len(),
            has_result: function.result.is_some(),
            freestanding: matches!(
                function.kind,
                FunctionKind::Freestanding | FunctionKind::AsyncFreestanding
            ),
        })
    }
}

#[derive(Debug, Clone)]
struct ExistingRecord {
    wit_name: String,
    fields: BTreeMap<String, ExistingField>,
}

impl ExistingRecord {
    fn from_resolve(resolve: &Resolve, wit_name: &str, record: &Record) -> Self {
        let mut fields = BTreeMap::new();
        for field in &record.fields {
            let docs = field.docs.contents.as_deref();
            let explicit_proto_name = proto_field_name_from_docs(docs);
            let proto_name = explicit_proto_name
                .clone()
                .unwrap_or_else(|| field.name.replace('-', "_"));
            fields.insert(
                proto_name,
                ExistingField {
                    wit_name: field.name.clone(),
                    type_expr: render_wit_type(resolve, &field.ty),
                    explicit_proto_field: explicit_proto_name.is_some(),
                    omitted: field_has_omit_directive(docs),
                },
            );
        }

        Self {
            wit_name: wit_name.to_string(),
            fields,
        }
    }
}

#[derive(Debug, Clone)]
struct ExistingField {
    wit_name: String,
    type_expr: String,
    explicit_proto_field: bool,
    omitted: bool,
}

struct ProtoWitBuilder<'a> {
    descriptors: &'a DescriptorIndex,
    rpc: Option<&'a RpcMetadata>,
    context: String,
    linked_wit: LinkedWitMetadata,
    linked_uses: BTreeMap<String, BTreeSet<String>>,
    available_type_names: BTreeMap<String, String>,
    existing_records_by_proto: BTreeMap<String, ExistingRecord>,
    existing_oneofs_by_variant: BTreeMap<String, OneofMetadata>,
    existing_declared_type_names: BTreeSet<String>,
    reserved_type_names: BTreeSet<String>,
    type_parameters: BTreeSet<String>,
    rendered_types: BTreeSet<String>,
    rendered_definitions: Vec<String>,
    reconciled_types: BTreeSet<String>,
    record_updates: Vec<RecordFieldUpdate>,
}

impl<'a> ProtoWitBuilder<'a> {
    fn unsupported(&self, context: impl Into<String>, reason: impl Into<String>) -> Error {
        if self.rpc.is_some() {
            Error::UnsupportedAddRpc {
                context: context.into(),
                reason: reason.into(),
            }
        } else {
            Error::UnsupportedAddMessage {
                context: context.into(),
                reason: reason.into(),
            }
        }
    }

    fn for_rpc(
        descriptors: &'a DescriptorIndex,
        rpc: &'a RpcMetadata,
        linked_wit: LinkedWitMetadata,
    ) -> Self {
        Self {
            descriptors,
            rpc: Some(rpc),
            context: rpc.full_name.clone(),
            type_parameters: linked_wit.type_parameters.clone(),
            linked_wit,
            linked_uses: BTreeMap::new(),
            available_type_names: BTreeMap::new(),
            existing_records_by_proto: BTreeMap::new(),
            existing_oneofs_by_variant: BTreeMap::new(),
            existing_declared_type_names: BTreeSet::new(),
            reserved_type_names: BTreeSet::new(),
            rendered_types: BTreeSet::new(),
            rendered_definitions: Vec::new(),
            reconciled_types: BTreeSet::new(),
            record_updates: Vec::new(),
        }
    }

    fn for_message(
        descriptors: &'a DescriptorIndex,
        message: &MessageMetadata,
        linked_wit: LinkedWitMetadata,
    ) -> Self {
        Self {
            descriptors,
            rpc: None,
            context: message.full_name.clone(),
            type_parameters: linked_wit.type_parameters.clone(),
            linked_wit,
            linked_uses: BTreeMap::new(),
            available_type_names: BTreeMap::new(),
            existing_records_by_proto: BTreeMap::new(),
            existing_oneofs_by_variant: BTreeMap::new(),
            existing_declared_type_names: BTreeSet::new(),
            reserved_type_names: BTreeSet::new(),
            rendered_types: BTreeSet::new(),
            rendered_definitions: Vec::new(),
            reconciled_types: BTreeSet::new(),
            record_updates: Vec::new(),
        }
    }

    fn with_existing_interface(mut self, interface: &ExistingInterface) -> Self {
        self.available_type_names = interface.type_names_by_proto.clone();
        self.existing_records_by_proto = interface.records_by_proto.clone();
        self.existing_oneofs_by_variant = interface.oneofs_by_variant.clone();
        self.existing_declared_type_names = interface.declared_type_names.clone();
        self.reserved_type_names = interface.type_names_in_scope.clone();
        self.type_parameters
            .retain(|name| !interface.declared_type_names.contains(name));
        self.type_parameters
            .extend(interface.type_parameters.clone());
        self
    }

    fn build(mut self) -> Result<RenderedAddRpcWit> {
        let rpc = self.rpc.expect("add-rpc builder must have an RPC").clone();
        let input_type = self.render_type_reference(&rpc.input_type, &rpc.full_name)?;
        let output_type = self.render_type_reference(&rpc.output_type, &rpc.full_name)?;

        Ok(RenderedAddRpcWit {
            rpc_full_name: rpc.full_name.clone(),
            interface_name: rpc.service_name.to_kebab_case(),
            changes: self.into_changes(),
            operation: format!(
                "  {}: func(\n    request: {},\n  ) -> {};\n",
                rpc.name.to_kebab_case(),
                input_type,
                output_type
            ),
        })
    }

    fn build_existing_operation_update(
        mut self,
        interface: &ExistingInterface,
        operation_name: &str,
    ) -> Result<ModelChanges> {
        let rpc = self.rpc.expect("add-rpc builder must have an RPC").clone();
        let input_type = self.render_type_reference(&rpc.input_type, &rpc.full_name)?;
        let output_type = self.render_type_reference(&rpc.output_type, &rpc.full_name)?;
        let Some(function) = interface.functions.get(operation_name) else {
            return Err(Error::UnsupportedAddRpc {
                context: self.context.clone(),
                reason: format!("existing operation `{operation_name}` was not found"),
            });
        };
        self.validate_existing_function(function, operation_name, &input_type, &output_type)?;

        Ok(self.into_changes())
    }

    fn into_changes(self) -> ModelChanges {
        ModelChanges {
            linked_uses: self.linked_uses,
            rendered_definitions: self.rendered_definitions,
            record_updates: self.record_updates,
        }
    }

    fn validate_existing_function(
        &self,
        function: &ExistingFunction,
        operation_name: &str,
        input_type: &str,
        output_type: &str,
    ) -> Result<()> {
        let mut conflicts = Vec::new();
        if !function.freestanding {
            conflicts.push("operation is not a freestanding function".to_string());
        }
        if function.params_len != 1 {
            conflicts.push(format!(
                "operation has {} parameters instead of 1",
                function.params_len
            ));
        }
        if function.parameter_name.as_deref() != Some("request") {
            conflicts.push(format!(
                "operation parameter is `{}` instead of `request`",
                function.parameter_name.as_deref().unwrap_or("<missing>")
            ));
        }
        if function.input_type_name.as_deref() != Some(input_type) {
            conflicts.push(format!(
                "operation request type is `{}` instead of `{input_type}`",
                function.input_type_name.as_deref().unwrap_or("<missing>")
            ));
        }
        let rpc = self.rpc.expect("add-rpc builder must have an RPC");
        if function.input_proto.as_deref() != Some(rpc.input_type.trim_start_matches('.')) {
            conflicts.push(format!(
                "operation request proto is `{}` instead of `{}`",
                function.input_proto.as_deref().unwrap_or("<missing>"),
                rpc.input_type.trim_start_matches('.')
            ));
        }
        if !function.has_result {
            conflicts.push("operation does not declare a result type".to_string());
        }
        if function.output_type_name.as_deref() != Some(output_type) {
            conflicts.push(format!(
                "operation result type is `{}` instead of `{output_type}`",
                function.output_type_name.as_deref().unwrap_or("<missing>")
            ));
        }
        if function.output_proto.as_deref() != Some(rpc.output_type.trim_start_matches('.')) {
            conflicts.push(format!(
                "operation result proto is `{}` instead of `{}`",
                function.output_proto.as_deref().unwrap_or("<missing>"),
                rpc.output_type.trim_start_matches('.')
            ));
        }

        if conflicts.is_empty() {
            return Ok(());
        }

        Err(Error::UnsupportedAddRpc {
            context: self.context.clone(),
            reason: format!(
                "existing operation `{operation_name}` conflicts with descriptor: {}",
                conflicts.join("; ")
            ),
        })
    }

    fn missing_record_fields(
        &mut self,
        record: &ExistingRecord,
        message: &MessageMetadata,
        proto_name: &str,
    ) -> Result<Vec<String>> {
        let oneofs = self.real_oneof_groups(message)?;
        let mut expected_by_proto = message
            .descriptor
            .field
            .iter()
            .map(|field| field_name(field, proto_name))
            .collect::<Result<BTreeSet<_>>>()?
            .into_iter()
            .map(str::to_string)
            .collect::<BTreeSet<_>>();
        expected_by_proto.extend(oneofs.iter().map(|oneof| oneof.name.to_string()));

        for existing_proto_name in record.fields.keys() {
            if !expected_by_proto.contains(existing_proto_name) {
                return Err(self.unsupported(
                    proto_name,
                    format!(
                        "existing WIT record `{}` contains non-descriptor field `{existing_proto_name}`",
                        record.wit_name
                    ),
                ));
            }
        }

        let mut oneof_by_field = BTreeMap::new();
        for (group_index, oneof) in oneofs.iter().enumerate() {
            for (field_index, _) in &oneof.fields {
                oneof_by_field.insert(*field_index, group_index);
            }
        }

        let mut missing = Vec::new();
        for (field_index, field) in message.descriptor.field.iter().enumerate() {
            if let Some(group_index) = oneof_by_field.get(&field_index) {
                let oneof = &oneofs[*group_index];
                if field_index == oneof.fields[0].0 && !record.fields.contains_key(oneof.name) {
                    let variant_name =
                        self.render_or_reuse_oneof_variant(message, &record.wit_name, oneof)?;
                    missing.push(render_proto_field_line(
                        oneof.name,
                        &oneof.name.to_kebab_case(),
                        &format!("option<{variant_name}>"),
                    ));
                }
                continue;
            }

            self.reconcile_existing_proto_field(record, message, field, &mut missing)?;
        }

        Ok(missing)
    }

    fn reconcile_existing_proto_field(
        &mut self,
        record: &ExistingRecord,
        message: &MessageMetadata,
        field: &FieldDescriptorProto,
        missing: &mut Vec<String>,
    ) -> Result<()> {
        let proto_name = &message.full_name;
        let field_name = field_name(field, proto_name)?;
        let Some(existing) = record.fields.get(field_name) else {
            if self.record_has_field_covering_proto(record, field_name) {
                return Ok(());
            }
            let expected = self.render_message_field(message, field)?;
            missing.push(expected.line.clone());
            return Ok(());
        };
        if existing.omitted {
            return Ok(());
        }
        if self.field_uses_type_parameter(&existing.type_expr, field) {
            if existing.wit_name != field_name.to_kebab_case() && !existing.explicit_proto_field {
                return Err(self.unsupported(
                    format!("{proto_name}.{field_name}"),
                    "type-parameter field must retain its proto field name or declare @nexus.proto-field",
                ));
            }
            return Ok(());
        }
        let expected = self.render_message_field_for_comparison(message, field)?;
        let name_matches = existing.wit_name == expected.wit_name || existing.explicit_proto_field;
        let type_matches = self.type_is_compatible(&existing.type_expr, &expected.type_expr);
        if !name_matches || !type_matches {
            return Err(self.unsupported(
                format!("{proto_name}.{}", field_name),
                format!(
                    "existing WIT field is `{}: {}` but descriptor requires `{}: {}`",
                    existing.wit_name, existing.type_expr, expected.wit_name, expected.type_expr
                ),
            ));
        }
        if base_type_expr(&existing.type_expr) == base_type_expr(&expected.type_expr) {
            if let Some(type_name) = field.type_name.as_deref() {
                self.reconcile_type(type_name, &format!("{proto_name}.{field_name}"))?;
            }
        }
        Ok(())
    }

    fn field_uses_type_parameter(&self, type_expr: &str, field: &FieldDescriptorProto) -> bool {
        if !matches!(
            field
                .type_name
                .as_deref()
                .map(|name| name.trim_start_matches('.')),
            Some("temporal.api.common.v1.Payload" | "temporal.api.common.v1.Payloads")
        ) {
            return false;
        }
        let mut type_expr = base_type_expr(type_expr);
        if field.label == Some(Label::Repeated as i32) {
            let Some(inner) = type_expr
                .strip_prefix("list<")
                .and_then(|ty| ty.strip_suffix('>'))
            else {
                return false;
            };
            type_expr = inner;
        }
        self.type_parameters.contains(type_expr)
    }

    fn type_is_compatible(&self, existing: &str, expected: &str) -> bool {
        if existing == expected {
            return true;
        }

        match (option_inner_type(existing), option_inner_type(expected)) {
            (Some(existing_inner), Some(expected_inner)) => {
                self.type_is_compatible(existing_inner, expected_inner)
            }
            (None, Some(expected_inner)) => self.type_is_compatible(existing, expected_inner),
            (Some(existing_inner), None) => self.type_is_compatible(existing_inner, expected),
            (None, None) => self.base_type_is_compatible(existing, expected),
        }
    }

    fn base_type_is_compatible(&self, existing: &str, expected: &str) -> bool {
        if let (Some(existing), Some(expected)) = (
            existing
                .strip_prefix("list<")
                .and_then(|ty| ty.strip_suffix('>')),
            expected
                .strip_prefix("list<")
                .and_then(|ty| ty.strip_suffix('>')),
        ) {
            return self.type_is_compatible(existing, expected);
        }
        let mut visited = BTreeSet::new();
        let mut pending = vec![existing];
        while let Some(current) = pending.pop() {
            if current == expected {
                return true;
            }
            if !visited.insert(current.to_string()) {
                continue;
            }
            if let Some(targets) = self.linked_wit.type_compatibility.get(current) {
                pending.extend(targets.iter().map(String::as_str));
            }
        }
        false
    }

    fn record_has_field_covering_proto(&self, record: &ExistingRecord, proto_name: &str) -> bool {
        record.fields.values().any(|field| {
            self.linked_wit
                .type_covered_fields
                .get(base_type_expr(&field.type_expr))
                .is_some_and(|covered_fields| covered_fields.contains(proto_name))
        })
    }

    /// Reconcile mapped records without importing their internal field types.
    /// Opaque mappings are substitution boundaries; they own their conversions.
    fn reconcile_type(&mut self, proto_name: &str, context: &str) -> Result<()> {
        let proto_name = proto_name.trim_start_matches('.');
        if !self.reconciled_types.insert(proto_name.to_string()) {
            return Ok(());
        }
        let Some(message) = self.descriptors.message(proto_name) else {
            return Ok(());
        };
        let (record, owned) = if let Some(record) = self.existing_records_by_proto.get(proto_name) {
            (record.clone(), true)
        } else if self.available_type_names.contains_key(proto_name) {
            return Ok(());
        } else if let Some(linked_type) = self.linked_wit.proto_types.get(proto_name) {
            let Some(fields) = &linked_type.record_fields else {
                return Ok(());
            };
            (
                ExistingRecord {
                    wit_name: linked_type.wit_name.clone(),
                    fields: fields
                        .iter()
                        .map(|(name, field)| {
                            (
                                name.clone(),
                                ExistingField {
                                    wit_name: field.wit_name.clone(),
                                    type_expr: field.type_expr.clone(),
                                    explicit_proto_field: field.explicit_proto_field,
                                    omitted: field.omitted,
                                },
                            )
                        })
                        .collect(),
                },
                false,
            )
        } else {
            return Ok(());
        };
        self.validate_reused_oneofs(proto_name, &record.wit_name, Some(&record), context)?;
        let fields = self.missing_record_fields(&record, message, proto_name)?;
        if !fields.is_empty() {
            if !owned {
                return Err(self.unsupported(proto_name, format!(
                    "linked WIT record `{}` is missing descriptor fields; update its owning WIT package: {}",
                    record.wit_name, fields.join(" ")
                )));
            }
            self.record_updates.push(RecordFieldUpdate {
                record_name: record.wit_name,
                fields,
            });
        }
        Ok(())
    }

    fn render_type_reference(&mut self, proto_name: &str, context: &str) -> Result<String> {
        let proto_name = proto_name.trim_start_matches('.');
        self.reconcile_type(proto_name, context)?;
        if let Some(existing_name) = self.available_type_names.get(proto_name).cloned() {
            return Ok(existing_name);
        }

        if let Some(linked_type) = self.linked_wit.proto_types.get(proto_name).cloned() {
            self.use_linked_type(proto_name, &linked_type.wit_name)?;
            return Ok(linked_type.wit_name);
        }

        if let Some(message) = self.descriptors.message(proto_name) {
            let wit_name = self.reserve_local_type_name(proto_name, context)?;
            if self.rendered_types.insert(proto_name.to_string()) {
                let definition = self.render_message(message, &wit_name)?;
                self.rendered_definitions.push(definition);
            }
            return Ok(wit_name);
        }

        if let Some(enumeration) = self.descriptors.enumeration(proto_name) {
            let wit_name = self.reserve_local_type_name(proto_name, context)?;
            if self.rendered_types.insert(proto_name.to_string()) {
                let definition = self.render_enum(enumeration, &wit_name)?;
                self.rendered_definitions.push(definition);
            }
            return Ok(wit_name);
        }

        Err(self.unsupported(context, format!("unknown proto type `{proto_name}`")))
    }

    fn validate_reused_oneofs(
        &mut self,
        proto_name: &str,
        wit_name: &str,
        record: Option<&ExistingRecord>,
        context: &str,
    ) -> Result<()> {
        let Some(message) = self.descriptors.message(proto_name) else {
            return Ok(());
        };
        let oneofs = self.real_oneof_groups(message)?;
        if oneofs.is_empty() {
            return Ok(());
        }
        let Some(record) = record else {
            return Err(self.unsupported(
                context,
                format!(
                    "existing WIT type `{wit_name}` is not a record, but protobuf message `{proto_name}` contains a oneof and must use grouped variant fields"
                ),
            ));
        };

        for oneof in oneofs {
            let grouped = record.fields.get(oneof.name);
            let has_individual_members = oneof.fields.iter().any(|(_, field)| {
                field
                    .name
                    .as_deref()
                    .is_some_and(|name| record.fields.contains_key(name))
                    || field
                        .name
                        .as_deref()
                        .is_some_and(|name| self.record_has_field_covering_proto(record, name))
            });
            if has_individual_members {
                return Err(self.unsupported(
                    format!("{}.{}", message.full_name, oneof.name),
                    format!(
                        "existing WIT record `{}` represents this oneof as individual fields; expected `{}: option<{}-{}>`",
                        record.wit_name,
                        oneof.name.to_kebab_case(),
                        record.wit_name,
                        oneof.name.to_kebab_case()
                    ),
                ));
            }
            let Some(grouped) = grouped else {
                // Owned missing groups are planned by missing_record_fields;
                // linked records cannot apply those additions and will fail.
                continue;
            };
            if grouped.omitted {
                continue;
            }
            let wit_name = oneof.name.to_kebab_case();
            let expected_type = format!("option<{}-{wit_name}>", record.wit_name);
            let name_matches = grouped.wit_name == wit_name || grouped.explicit_proto_field;
            if !name_matches {
                return Err(self.unsupported(
                    format!("{}.{}", message.full_name, oneof.name),
                    format!(
                        "existing WIT oneof field is `{}: {}` but descriptor requires `{}: {}`",
                        grouped.wit_name, grouped.type_expr, wit_name, expected_type
                    ),
                ));
            }
            let variant_name = base_type_expr(&grouped.type_expr);
            let Some(metadata) = self.oneof_metadata_for_variant(variant_name).cloned() else {
                return Err(self.unsupported(
                    format!("{}.{}", message.full_name, oneof.name),
                    format!(
                        "existing WIT oneof field `{}: {}` must refer to a mapped variant; it must declare `@nexus.proto \"{}.{}\"`",
                        grouped.wit_name, grouped.type_expr, message.full_name, oneof.name
                    ),
                ));
            };
            let source = &metadata.source;
            if source.message.trim_start_matches('.') != message.full_name
                || source.name != oneof.name
            {
                return Err(self.unsupported(
                    format!("{}.{}", message.full_name, oneof.name),
                    format!(
                        "existing WIT variant `{variant_name}` binds to protobuf oneof `{}.{}`",
                        source.message, source.name
                    ),
                ));
            }
            let oneof_context = format!("{}.{}", message.full_name, oneof.name);
            let expected_cases = oneof
                .fields
                .iter()
                .map(|(_, field)| field_name(field, &message.full_name))
                .collect::<Result<BTreeSet<_>>>()?;
            for name in metadata.cases.keys() {
                if !expected_cases.contains(name.as_str()) {
                    return Err(self.unsupported(
                        &oneof_context,
                        format!("existing WIT variant `{variant_name}` has extra case `{name}`"),
                    ));
                }
            }
            for (_, field) in &oneof.fields {
                let name = field_name(field, &message.full_name)?;
                let Some(payload) = metadata.cases.get(name) else {
                    return Err(self.unsupported(
                        &oneof_context,
                        format!("existing WIT variant `{variant_name}` is missing case `{name}`"),
                    ));
                };
                let Some(payload) = payload else {
                    return Err(self.unsupported(&oneof_context, format!(
                        "existing WIT variant `{variant_name}` case `{name}` must carry a payload"
                    )));
                };
                if self.type_parameters.contains(payload)
                    && self.field_uses_type_parameter(payload, field)
                {
                    continue;
                }
                let expected =
                    self.field_base_type_for_comparison(field, &message.full_name, name)?;
                // Case payloads must be singular; only the grouped record field
                // may refine protobuf presence with option<>.
                if !self.base_type_is_compatible(payload, &expected) {
                    return Err(self.unsupported(&oneof_context, format!(
                        "existing WIT variant `{variant_name}` case `{name}` payload `{payload}` does not match descriptor type `{expected}`"
                    )));
                }
                if payload == &expected {
                    if let Some(type_name) = field.type_name.as_deref() {
                        self.reconcile_type(type_name, &oneof_context)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn oneof_metadata_for_variant(&self, variant_name: &str) -> Option<&OneofMetadata> {
        if self.existing_declared_type_names.contains(variant_name) {
            self.existing_oneofs_by_variant.get(variant_name)
        } else {
            self.linked_wit.oneofs_by_variant.get(variant_name)
        }
    }

    fn render_message(&mut self, message: &MessageMetadata, wit_name: &str) -> Result<String> {
        let rendered_fields = self.render_message_fields(message)?;

        let mut rendered = String::new();
        rendered.push_str(&format!("  /// @nexus.proto \"{}\"\n", message.full_name));
        rendered.push_str(&format!("  record {wit_name} {{\n"));
        for field in rendered_fields {
            rendered.push_str(&field.line);
            rendered.push('\n');
        }
        rendered.push_str("  }\n");
        Ok(rendered)
    }

    fn render_message_fields(
        &mut self,
        message: &MessageMetadata,
    ) -> Result<Vec<RenderedFieldSpec>> {
        let oneofs = self.real_oneof_groups(message)?;
        let mut oneof_by_field = BTreeMap::new();
        for (group_index, oneof) in oneofs.iter().enumerate() {
            for (field_index, _) in &oneof.fields {
                oneof_by_field.insert(*field_index, group_index);
            }
        }

        let mut rendered_fields = Vec::new();
        let mut rendered_names = BTreeSet::new();
        for (field_index, field) in message.descriptor.field.iter().enumerate() {
            if let Some(group_index) = oneof_by_field.get(&field_index) {
                let oneof = &oneofs[*group_index];
                if field_index != oneof.fields[0].0 {
                    continue;
                }
                let variant_name = self.render_or_reuse_oneof_variant(
                    message,
                    &self.local_type_name(&message.full_name),
                    oneof,
                )?;
                let wit_name = oneof.name.to_kebab_case();
                if !rendered_names.insert(wit_name.clone()) {
                    return Err(self.unsupported(
                        &message.full_name,
                        format!("generated record field `{wit_name}` would collide"),
                    ));
                }
                rendered_fields.push(RenderedFieldSpec {
                    wit_name: wit_name.clone(),
                    type_expr: format!("option<{variant_name}>"),
                    line: render_proto_field_line(
                        oneof.name,
                        &wit_name,
                        &format!("option<{variant_name}>"),
                    ),
                });
                continue;
            }

            let rendered = self.render_message_field(message, field)?;
            if !rendered_names.insert(rendered.wit_name.clone()) {
                return Err(self.unsupported(
                    &message.full_name,
                    format!(
                        "generated record field `{}` would collide",
                        rendered.wit_name
                    ),
                ));
            }
            rendered_fields.push(rendered);
        }
        Ok(rendered_fields)
    }

    fn real_oneof_groups<'m>(
        &self,
        message: &'m MessageMetadata,
    ) -> Result<Vec<ProtoOneofGroup<'m>>> {
        real_oneof_groups(message).map_err(|reason| self.unsupported(&message.full_name, reason))
    }

    fn render_or_reuse_oneof_variant(
        &mut self,
        message: &MessageMetadata,
        record_name: &str,
        oneof: &ProtoOneofGroup<'_>,
    ) -> Result<String> {
        let existing_name = self
            .existing_oneofs_by_variant
            .iter()
            .chain(
                self.linked_wit
                    .oneofs_by_variant
                    .iter()
                    .filter(|(name, _)| !self.existing_declared_type_names.contains(*name)),
            )
            .find(|(_, metadata)| {
                metadata.source.message == message.full_name && metadata.source.name == oneof.name
            })
            .map(|(name, _)| name.clone());
        if let Some(name) = existing_name {
            let record = ExistingRecord {
                wit_name: record_name.to_string(),
                fields: BTreeMap::from([(
                    oneof.name.to_string(),
                    ExistingField {
                        wit_name: oneof.name.to_kebab_case(),
                        type_expr: format!("option<{name}>"),
                        explicit_proto_field: false,
                        omitted: false,
                    },
                )]),
            };
            self.validate_reused_oneofs(
                &message.full_name,
                record_name,
                Some(&record),
                &message.full_name,
            )?;
            if !self.existing_declared_type_names.contains(&name) {
                self.use_linked_type(&format!("{}.{}", message.full_name, oneof.name), &name)?;
            }
            return Ok(name);
        }
        let name = self.reserve_oneof_variant_name(record_name, oneof.name, &message.full_name)?;
        let definition = self.render_oneof_variant(message, &name, oneof)?;
        self.rendered_definitions.push(definition);
        Ok(name)
    }

    fn reserve_oneof_variant_name(
        &mut self,
        message_wit_name: &str,
        oneof_name: &str,
        context: &str,
    ) -> Result<String> {
        let name = format!("{message_wit_name}-{}", oneof_name.to_kebab_case());
        if !self.reserved_type_names.insert(name.clone()) {
            return Err(self.unsupported(
                context,
                format!("generated oneof variant name `{name}` would collide with a WIT type"),
            ));
        }
        Ok(name)
    }

    fn render_oneof_variant(
        &mut self,
        message: &MessageMetadata,
        wit_name: &str,
        oneof: &ProtoOneofGroup<'_>,
    ) -> Result<String> {
        let mut cases = Vec::new();
        let mut case_names = BTreeSet::new();
        for (_, field) in &oneof.fields {
            let proto_name = field_name(field, &message.full_name)?;
            let case_name = proto_name.to_kebab_case();
            if !case_names.insert(case_name.clone()) {
                return Err(self.unsupported(
                    &message.full_name,
                    format!("generated variant case `{case_name}` would collide"),
                ));
            }
        }
        for (_, field) in &oneof.fields {
            let proto_name = field_name(field, &message.full_name)?;
            let case_name = proto_name.to_kebab_case();
            if case_name.to_snake_case() != proto_name {
                return Err(self.unsupported(
                    format!("{}.{}", message.full_name, oneof.name),
                    format!("oneof member `{proto_name}` cannot be represented by WIT case `{case_name}` without changing its wire name"),
                ));
            }
            let payload = self.render_oneof_case_type(field, &message.full_name, proto_name)?;
            cases.push((case_name, payload));
        }

        let mut rendered = String::new();
        rendered.push_str(&format!(
            "  /// Protobuf oneof `{}.{}`.\n",
            message.full_name, oneof.name
        ));
        rendered.push_str(&format!(
            "  /// @nexus.proto \"{}.{}\"\n",
            message.full_name, oneof.name
        ));
        rendered.push_str(&format!("  variant {wit_name} {{\n"));
        for (case_name, payload) in cases {
            rendered.push_str(&format!(
                "    {}({payload}),\n",
                render_wit_identifier(&case_name)
            ));
        }
        rendered.push_str("  }\n");
        Ok(rendered)
    }

    fn render_oneof_case_type(
        &mut self,
        field: &FieldDescriptorProto,
        parent_type: &str,
        field_name: &str,
    ) -> Result<String> {
        let context = format!("{parent_type}.{field_name}");
        let label = Label::try_from(field.label.unwrap_or(Label::Optional as i32))
            .map_err(|_| self.unsupported(&context, "unknown field label"))?;
        if label == Label::Repeated {
            return Err(self.unsupported(&context, "oneof fields cannot be repeated"));
        }
        self.render_field_base_type(field, parent_type, field_name)
    }

    fn render_message_field(
        &mut self,
        message: &MessageMetadata,
        field: &FieldDescriptorProto,
    ) -> Result<RenderedFieldSpec> {
        let field_name = field_name(field, &message.full_name)?;
        let wit_field_type = self.render_field_type(field, &message.full_name, field_name)?;
        let wit_field_name = field_name.to_kebab_case();
        Ok(RenderedFieldSpec {
            wit_name: wit_field_name.clone(),
            type_expr: wit_field_type.clone(),
            line: render_proto_field_line(field_name, &wit_field_name, &wit_field_type),
        })
    }

    fn render_message_field_for_comparison(
        &self,
        message: &MessageMetadata,
        field: &FieldDescriptorProto,
    ) -> Result<RenderedFieldSpec> {
        let name = field_name(field, &message.full_name)?;
        let base_type = self.field_base_type_for_comparison(field, &message.full_name, name)?;
        let type_expr = self.field_type_with_base(field, &message.full_name, name, base_type)?;
        let wit_name = name.to_kebab_case();
        Ok(RenderedFieldSpec {
            line: render_proto_field_line(name, &wit_name, &type_expr),
            wit_name,
            type_expr,
        })
    }

    /// Resolve the expected spelling without planning declarations or imports.
    fn field_base_type_for_comparison(
        &self,
        field: &FieldDescriptorProto,
        parent_type: &str,
        field_name: &str,
    ) -> Result<String> {
        let context = format!("{parent_type}.{field_name}");
        let field_type = Type::try_from(field.r#type.unwrap_or_default())
            .map_err(|_| self.unsupported(&context, "unknown field type"))?;
        if let Some(primitive) = primitive_wit_type(field_type) {
            return Ok(primitive.to_string());
        }
        let name = field
            .type_name
            .as_deref()
            .ok_or_else(|| self.unsupported(&context, "field is missing a referenced type name"))?
            .trim_start_matches('.');
        if let Some(wit_name) = self.available_type_names.get(name) {
            return Ok(wit_name.clone());
        }
        if let Some(linked) = self.linked_wit.proto_types.get(name) {
            return Ok(linked.wit_name.clone());
        }
        if self.descriptors.message(name).is_none() && self.descriptors.enumeration(name).is_none()
        {
            return Err(self.unsupported(&context, format!("unknown proto type `{name}`")));
        }
        let wit_name = self.local_type_name(name);
        if self.reserved_type_names.contains(&wit_name) {
            return Err(self.unsupported(&context, format!(
                "generated type name `{wit_name}` would collide with an existing WIT type without a matching proto mapping"
            )));
        }
        Ok(wit_name)
    }

    fn render_enum(&mut self, enumeration: &EnumMetadata, wit_name: &str) -> Result<String> {
        let mut rendered = String::new();
        rendered.push_str(&format!(
            "  /// @nexus.proto \"{}\"\n",
            enumeration.full_name
        ));
        rendered.push_str(&format!("  enum {wit_name} {{\n"));
        for value in &enumeration.descriptor.value {
            let Some(name) = value.name.as_deref() else {
                return Err(
                    self.unsupported(&enumeration.full_name, "enum value is missing a name")
                );
            };
            rendered.push_str(&format!("    {},\n", name.to_kebab_case()));
        }
        rendered.push_str("  }\n");
        Ok(rendered)
    }

    fn render_field_type(
        &mut self,
        field: &FieldDescriptorProto,
        parent_type: &str,
        field_name: &str,
    ) -> Result<String> {
        let base_type = self.render_field_base_type(field, parent_type, field_name)?;
        self.field_type_with_base(field, parent_type, field_name, base_type)
    }

    fn field_type_with_base(
        &self,
        field: &FieldDescriptorProto,
        parent_type: &str,
        field_name: &str,
        base_type: String,
    ) -> Result<String> {
        let context = format!("{parent_type}.{field_name}");
        let label = Label::try_from(field.label.unwrap_or(Label::Optional as i32))
            .map_err(|_| self.unsupported(&context, "unknown field label"))?;
        let field_type = Type::try_from(field.r#type.unwrap_or_default())
            .map_err(|_| self.unsupported(&context, "unknown field type"))?;

        if label == Label::Repeated {
            return Ok(format!("list<{base_type}>"));
        }

        if field_has_presence(field, field_type)
            || !field_supports_required_without_presence(field_type)
        {
            return Ok(format!("option<{base_type}>"));
        }

        Ok(base_type)
    }

    fn render_field_base_type(
        &mut self,
        field: &FieldDescriptorProto,
        parent_type: &str,
        field_name: &str,
    ) -> Result<String> {
        let context = format!("{parent_type}.{field_name}");
        let field_type = Type::try_from(field.r#type.unwrap_or_default())
            .map_err(|_| self.unsupported(&context, "unknown field type"))?;
        if let Some(primitive) = primitive_wit_type(field_type) {
            return Ok(primitive.to_string());
        }
        let Some(type_name) = field.type_name.as_deref() else {
            return Err(self.unsupported(context, "field is missing a referenced type name"));
        };
        self.render_type_reference(type_name, parent_type)
    }

    fn reserve_local_type_name(&mut self, proto_name: &str, context: &str) -> Result<String> {
        if let Some(existing_name) = self.available_type_names.get(proto_name) {
            return Ok(existing_name.clone());
        }

        let wit_name = self.local_type_name(proto_name);
        if self.reserved_type_names.contains(&wit_name) {
            return Err(self.unsupported(
                context,
                format!("generated type name `{wit_name}` would collide with an existing WIT type"),
            ));
        }

        self.available_type_names
            .insert(proto_name.to_string(), wit_name.clone());
        self.reserved_type_names.insert(wit_name.clone());
        Ok(wit_name)
    }

    fn use_linked_type(&mut self, proto_name: &str, wit_name: &str) -> Result<()> {
        let Some(use_path) = self.linked_wit.type_use_paths.get(wit_name) else {
            return Err(self.unsupported(
                &self.context,
                format!("linked WIT type `{wit_name}` was not found in linked metadata"),
            ));
        };

        let already_in_scope = self.reserved_type_names.contains(wit_name);
        self.available_type_names
            .insert(proto_name.to_string(), wit_name.to_string());
        self.reserved_type_names.insert(wit_name.to_string());
        if !already_in_scope {
            self.linked_uses
                .entry(use_path.clone())
                .or_default()
                .insert(wit_name.to_string());
        }
        Ok(())
    }

    fn local_type_name(&self, proto_name: &str) -> String {
        if let Some(message) = self.descriptors.message(proto_name) {
            return descriptor_relative_name(&message.full_name, &message.package);
        }
        if let Some(enumeration) = self.descriptors.enumeration(proto_name) {
            return descriptor_relative_name(&enumeration.full_name, &enumeration.package);
        }
        proto_name
            .trim_start_matches('.')
            .replace('.', "-")
            .to_kebab_case()
    }
}

fn primitive_wit_type(field_type: Type) -> Option<&'static str> {
    match field_type {
        Type::Double => Some("f64"),
        Type::Float => Some("f32"),
        Type::Int64 | Type::Sint64 | Type::Sfixed64 => Some("s64"),
        Type::Uint64 | Type::Fixed64 => Some("u64"),
        Type::Int32 | Type::Sint32 | Type::Sfixed32 => Some("s32"),
        Type::Uint32 | Type::Fixed32 => Some("u32"),
        Type::Bool => Some("bool"),
        Type::String => Some("string"),
        Type::Bytes => Some("list<u8>"),
        Type::Message | Type::Group | Type::Enum => None,
    }
}

fn render_proto_field_line(proto_name: &str, wit_name: &str, type_expr: &str) -> String {
    let mapping = if wit_name.replace('-', "_") != proto_name {
        format!("    /// @nexus.proto-field \"{proto_name}\"\n")
    } else {
        String::new()
    };
    format!(
        "{mapping}    {}: {type_expr},",
        render_wit_identifier(wit_name)
    )
}

struct MessageTreeBuilder<'a> {
    builder: ProtoWitBuilder<'a>,
    messages: &'a [&'a MessageMetadata],
}

impl<'a> MessageTreeBuilder<'a> {
    fn new(
        descriptors: &'a DescriptorIndex,
        messages: &'a [&'a MessageMetadata],
        linked_wit: LinkedWitMetadata,
    ) -> Self {
        Self {
            builder: ProtoWitBuilder::for_message(descriptors, messages[0], linked_wit),
            messages,
        }
    }

    fn with_existing_interface(mut self, interface: &ExistingInterface) -> Self {
        self.builder = self.builder.with_existing_interface(interface);
        self
    }

    fn build(mut self) -> Result<RenderedAddMessageWit> {
        for message in self.messages {
            self.builder
                .render_type_reference(&message.full_name, &message.full_name)?;
        }
        let message = self.messages[0];
        Ok(RenderedAddMessageWit {
            source_description: if self.messages.len() == 1 {
                message.full_name.clone()
            } else {
                format!("{} proto messages", self.messages.len())
            },
            interface_name: if self.messages.len() == 1 {
                message
                    .full_name
                    .rsplit('.')
                    .next()
                    .unwrap_or(&message.full_name)
                    .to_kebab_case()
            } else {
                "messages".to_string()
            },
            changes: self.builder.into_changes(),
        })
    }
}

#[derive(Debug, Clone)]
struct RenderedFieldSpec {
    wit_name: String,
    type_expr: String,
    line: String,
}

struct ModelChanges {
    linked_uses: BTreeMap<String, BTreeSet<String>>,
    rendered_definitions: Vec<String>,
    record_updates: Vec<RecordFieldUpdate>,
}

impl ModelChanges {
    fn apply(&self, input: &str, interface_name: &str) -> Result<String> {
        let mut source = input.to_string();
        let interface_additions =
            render_interface_additions(&self.linked_uses, &self.rendered_definitions);
        if !interface_additions.is_empty() {
            source = insert_into_named_block(
                &source,
                "interface",
                interface_name,
                &interface_additions,
            )?;
        }
        if !self.record_updates.is_empty() {
            let block = find_named_block(&source, "interface", interface_name)
                .expect("existing exported interface was parsed before planning updates");
            let contents = block.brace_start + 1..block.end_start;
            let mut updated = source[contents.clone()].to_string();
            for update in &self.record_updates {
                updated = insert_into_named_block(
                    &updated,
                    "record",
                    &update.record_name,
                    &update.fields.join("\n"),
                )?;
            }
            source.replace_range(contents, &updated);
        }
        Ok(source)
    }
}

struct RecordFieldUpdate {
    record_name: String,
    fields: Vec<String>,
}

struct RenderedAddRpcWit {
    rpc_full_name: String,
    interface_name: String,
    changes: ModelChanges,
    operation: String,
}

struct RenderedAddMessageWit {
    source_description: String,
    interface_name: String,
    changes: ModelChanges,
}

impl RenderedAddMessageWit {
    fn render_standalone(&self) -> String {
        let mut rendered = String::new();
        rendered.push_str(&format!(
            "/// WIT scaffold generated from `{}`.\n",
            self.source_description
        ));
        rendered.push_str(&format!("package {DEFAULT_PACKAGE_NAME};\n\n"));
        rendered.push_str(&format!("world {DEFAULT_WORLD_NAME} {{\n"));
        rendered.push_str(&format!("  export {};\n", self.interface_name));
        rendered.push_str("}\n\n");
        rendered.push_str(&format!("interface {} {{\n", self.interface_name));
        rendered.push_str(&render_interface_additions(
            &self.changes.linked_uses,
            &self.changes.rendered_definitions,
        ));
        rendered.push_str("}\n");
        rendered
    }
}

impl RenderedAddRpcWit {
    fn render_standalone(&self) -> String {
        let mut rendered = String::new();
        rendered.push_str(&format!(
            "/// WIT scaffold generated from `{}`.\n",
            self.rpc_full_name
        ));
        rendered.push_str(
            "/// Replace the endpoint and refine any inferred field mappings as needed.\n",
        );
        rendered.push_str(&format!("package {DEFAULT_PACKAGE_NAME};\n\n"));
        rendered.push_str(&format!("world {DEFAULT_WORLD_NAME} {{\n"));
        rendered.push_str(&format!("  export {};\n", self.interface_name));
        rendered.push_str("}\n\n");
        rendered.push_str(&self.render_new_interface_block());
        rendered
    }

    fn render_new_interface_block(&self) -> String {
        let mut rendered = String::new();
        rendered.push_str(&format!(
            "/// @nexus.endpoint \"{DEFAULT_ENDPOINT_PLACEHOLDER}\"\n"
        ));
        rendered.push_str(&format!("interface {} {{\n", self.interface_name));
        rendered.push_str(&self.render_interface_items());
        rendered.push_str("}\n");
        rendered
    }

    fn render_interface_items(&self) -> String {
        let mut rendered = render_interface_additions(
            &self.changes.linked_uses,
            &self.changes.rendered_definitions,
        );
        if !rendered.is_empty() {
            rendered.push('\n');
        }
        rendered.push_str(&self.operation);
        rendered
    }
}

fn render_interface_additions(
    linked_uses: &BTreeMap<String, BTreeSet<String>>,
    rendered_definitions: &[String],
) -> String {
    let mut rendered = String::new();

    if !linked_uses.is_empty() {
        rendered.push_str(&render_use_block(linked_uses));
    }

    if !rendered_definitions.is_empty() {
        if !rendered.is_empty() {
            rendered.push('\n');
        }
        for (index, definition) in rendered_definitions.iter().enumerate() {
            rendered.push_str(definition);
            if index + 1 != rendered_definitions.len() {
                rendered.push('\n');
            }
        }
    }

    rendered
}

fn render_use_block(linked_uses: &BTreeMap<String, BTreeSet<String>>) -> String {
    let mut rendered = String::new();
    for (use_path, linked_types) in linked_uses {
        rendered.push_str(&format!("  use {use_path}.{{\n"));
        for linked_type in linked_types {
            rendered.push_str(&format!("    {linked_type},\n"));
        }
        rendered.push_str("  };\n");
    }
    rendered
}

#[derive(Debug, Clone, Copy)]
struct NamedBlock {
    brace_start: usize,
    end_start: usize,
    end_exclusive: usize,
}

fn insert_into_named_block(
    source: &str,
    keyword: &str,
    name: &str,
    snippet: &str,
) -> Result<String> {
    let Some(block) = find_named_block(source, keyword, name) else {
        return Err(Error::UnsupportedAddRpc {
            context: format!("{keyword} `{name}`"),
            reason: "existing WIT file does not contain the target block".to_string(),
        });
    };

    let insertion_start = source[..block.end_start]
        .char_indices()
        .rev()
        .find_map(|(index, character)| match character {
            ' ' | '\t' => None,
            '\n' => Some(index + 1),
            _ => Some(block.end_start),
        })
        .unwrap_or(block.end_start);
    let mut rendered = String::with_capacity(source.len() + snippet.len() + 1);
    rendered.push_str(&source[..insertion_start]);
    if !rendered.ends_with('\n') {
        rendered.push('\n');
    }
    rendered.push_str(snippet);
    if !snippet.ends_with('\n') {
        rendered.push('\n');
    }
    rendered.push_str(&source[insertion_start..]);
    Ok(rendered)
}

fn insert_world_export(source: &str, world_name: &str, export_name: &str) -> Result<String> {
    let Some(block) = find_named_block(source, "world", world_name) else {
        return Err(Error::UnsupportedAddRpc {
            context: format!("world `{world_name}`"),
            reason: "existing WIT file does not contain the target world".to_string(),
        });
    };

    let mut rendered = String::with_capacity(source.len() + export_name.len() + 12);
    rendered.push_str(&source[..block.end_start]);
    rendered.push_str(&format!("  export {export_name};\n"));
    rendered.push_str(&source[block.end_start..]);
    Ok(rendered)
}

fn insert_after_named_block(
    source: &str,
    keyword: &str,
    name: &str,
    snippet: &str,
) -> Result<String> {
    let Some(block) = find_named_block(source, keyword, name) else {
        return Err(Error::UnsupportedAddRpc {
            context: format!("{keyword} `{name}`"),
            reason: "existing WIT file does not contain the target block".to_string(),
        });
    };

    let mut rendered = String::with_capacity(source.len() + snippet.len() + 2);
    rendered.push_str(&source[..block.end_exclusive]);
    rendered.push_str("\n\n");
    rendered.push_str(snippet);
    rendered.push_str(&source[block.end_exclusive..]);
    Ok(rendered)
}

fn find_named_block(source: &str, keyword: &str, name: &str) -> Option<NamedBlock> {
    let needle = format!("{keyword} {name}");
    for (index, _) in source.match_indices(&needle) {
        if index > 0 && !source[..index].chars().next_back().unwrap().is_whitespace() {
            continue;
        }

        let after_name = index + needle.len();
        let brace_offset = source[after_name..].find('{')?;
        let brace_start = after_name + brace_offset;
        if !source[after_name..brace_start]
            .chars()
            .all(char::is_whitespace)
        {
            continue;
        }

        let end_start = find_matching_brace(source, brace_start)?;
        return Some(NamedBlock {
            brace_start,
            end_start,
            end_exclusive: end_start + 1,
        });
    }
    None
}

fn find_matching_brace(source: &str, open_brace: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (offset, character) in source[open_brace..].char_indices() {
        match character {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open_brace + offset);
                }
            }
            _ => {}
        }
    }
    None
}

fn exported_interface_name(key: &WorldKey, interface: &Interface) -> String {
    match key {
        WorldKey::Name(name) => name.clone(),
        WorldKey::Interface(_) => interface
            .name
            .clone()
            .unwrap_or_else(|| "unnamed-interface".to_string()),
    }
}

fn collect_used_type_names(interface_source: &str) -> BTreeSet<String> {
    let mut used_names = BTreeSet::new();
    let mut offset = 0usize;

    while let Some(use_index) = interface_source[offset..].find("use ") {
        let use_start = offset + use_index;
        let Some(brace_index) = interface_source[use_start..].find('{') else {
            break;
        };
        let brace_start = use_start + brace_index;
        let Some(brace_end_rel) = interface_source[brace_start + 1..].find('}') else {
            break;
        };
        let brace_end = brace_start + 1 + brace_end_rel;

        for name in interface_source[brace_start + 1..brace_end].split(',') {
            let name = name.trim();
            if name.is_empty() {
                continue;
            }
            let imported_name = name.split_whitespace().next().unwrap_or(name);
            used_names.insert(imported_name.to_string());
        }

        offset = brace_end + 1;
    }

    used_names
}

fn proto_field_name_from_docs(docs: Option<&str>) -> Option<String> {
    let docs = docs?;
    for line in docs.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("@nexus.proto-field") else {
            continue;
        };
        let rest = rest.trim();
        if rest.starts_with('"') && rest.ends_with('"') && rest.len() >= 2 {
            return Some(rest[1..rest.len() - 1].to_string());
        }
    }
    None
}

fn field_has_omit_directive(docs: Option<&str>) -> bool {
    docs.into_iter()
        .flat_map(str::lines)
        .map(str::trim)
        .any(|line| line == "@nexus.omit")
}

fn wit_type_name(resolve: &Resolve, ty: &WitType) -> Option<String> {
    let WitType::Id(type_id) = ty else {
        return None;
    };
    resolve.types[*type_id].name.clone()
}

fn render_wit_type(resolve: &Resolve, ty: &WitType) -> String {
    match ty {
        WitType::Bool => "bool".to_string(),
        WitType::U8 => "u8".to_string(),
        WitType::U16 => "u16".to_string(),
        WitType::U32 => "u32".to_string(),
        WitType::U64 => "u64".to_string(),
        WitType::S8 => "s8".to_string(),
        WitType::S16 => "s16".to_string(),
        WitType::S32 => "s32".to_string(),
        WitType::S64 => "s64".to_string(),
        WitType::F32 => "f32".to_string(),
        WitType::F64 => "f64".to_string(),
        WitType::Char => "char".to_string(),
        WitType::String => "string".to_string(),
        WitType::ErrorContext => "error-context".to_string(),
        WitType::Id(type_id) => render_wit_type_id(resolve, *type_id),
    }
}

fn render_wit_type_id(resolve: &Resolve, type_id: TypeId) -> String {
    let type_def = &resolve.types[type_id];
    if let Some(name) = &type_def.name {
        return name.clone();
    }
    match &type_def.kind {
        TypeDefKind::Option(inner) => format!("option<{}>", render_wit_type(resolve, inner)),
        TypeDefKind::List(inner) => format!("list<{}>", render_wit_type(resolve, inner)),
        TypeDefKind::Type(inner) => render_wit_type(resolve, inner),
        TypeDefKind::Tuple(tuple) => format!(
            "tuple<{}>",
            tuple
                .types
                .iter()
                .map(|ty| render_wit_type(resolve, ty))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        TypeDefKind::Result(result) => {
            let ok = result
                .ok
                .as_ref()
                .map(|ty| render_wit_type(resolve, ty))
                .unwrap_or_else(|| "_".to_string());
            let err = result
                .err
                .as_ref()
                .map(|ty| render_wit_type(resolve, ty))
                .unwrap_or_else(|| "_".to_string());
            format!("result<{ok}, {err}>")
        }
        _ => type_def
            .name
            .clone()
            .unwrap_or_else(|| type_def.kind.as_str().to_string()),
    }
}

fn option_inner_type(type_expr: &str) -> Option<&str> {
    type_expr
        .strip_prefix("option<")
        .and_then(|inner| inner.strip_suffix('>'))
}

fn render_wit_identifier(name: &str) -> String {
    if matches!(
        name,
        "use"
            | "type"
            | "func"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "s8"
            | "s16"
            | "s32"
            | "s64"
            | "f32"
            | "f64"
            | "char"
            | "record"
            | "resource"
            | "own"
            | "borrow"
            | "flags"
            | "variant"
            | "enum"
            | "bool"
            | "string"
            | "option"
            | "result"
            | "future"
            | "stream"
            | "error-context"
            | "list"
            | "map"
            | "_"
            | "as"
            | "from"
            | "static"
            | "interface"
            | "tuple"
            | "import"
            | "export"
            | "world"
            | "package"
            | "constructor"
            | "async"
            | "include"
            | "with"
    ) {
        format!("%{name}")
    } else {
        name.to_string()
    }
}

fn base_type_expr(type_expr: &str) -> &str {
    option_inner_type(type_expr).unwrap_or(type_expr)
}

fn parse_proto_oneof_source(value: &str, path: &Path, context: &str) -> Result<ProtoOneofSource> {
    let Some((message, name)) = value.rsplit_once('.') else {
        return Err(Error::InvalidWitDirective {
            path: path.to_path_buf(),
            context: context.to_string(),
            directive: "@nexus.proto".to_string(),
            reason: "protobuf variants must identify a oneof as `<message>.<oneof>`".to_string(),
        });
    };
    if message.is_empty() || name.is_empty() {
        return Err(Error::InvalidWitDirective {
            path: path.to_path_buf(),
            context: context.to_string(),
            directive: "@nexus.proto".to_string(),
            reason: "protobuf variants must identify a oneof as `<message>.<oneof>`".to_string(),
        });
    }
    Ok(ProtoOneofSource {
        message: message.trim_start_matches('.').to_string(),
        name: name.to_string(),
    })
}

fn field_name<'a>(field: &'a FieldDescriptorProto, parent_type: &str) -> Result<&'a str> {
    field
        .name
        .as_deref()
        .ok_or_else(|| Error::UnsupportedAddRpc {
            context: parent_type.to_string(),
            reason: "field is missing a name".to_string(),
        })
}

fn field_has_presence(field: &FieldDescriptorProto, field_type: Type) -> bool {
    field.proto3_optional.unwrap_or(false)
        || field.oneof_index.is_some()
        || matches!(field_type, Type::Message | Type::Group)
}

fn field_supports_required_without_presence(field_type: Type) -> bool {
    matches!(field_type, Type::String | Type::Bytes)
}

fn descriptor_relative_name(full_name: &str, package: &str) -> String {
    let relative = full_name
        .trim_start_matches('.')
        .strip_prefix(&format!("{package}."))
        .unwrap_or(full_name.trim_start_matches('.'));
    relative.replace('.', "-").to_kebab_case()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::load_linked_wit_metadata_from_inputs;

    #[test]
    fn loads_linked_wit_metadata_from_temporal_type_input() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let input_path = root.join("advanced/samples/inputs/deps");
        let linked_types = load_linked_wit_metadata_from_inputs(&[input_path]).unwrap();

        assert_eq!(
            linked_types
                .proto_types
                .get("temporal.api.common.v1.Payload")
                .map(|type_metadata| type_metadata.wit_name.as_str()),
            Some("payload")
        );
        assert_eq!(
            linked_types
                .proto_types
                .get("temporal.api.taskqueue.v1.TaskQueue")
                .map(|type_metadata| type_metadata.wit_name.as_str()),
            Some("task-queue")
        );
        assert_eq!(
            linked_types
                .type_use_paths
                .get("workflow-function")
                .map(String::as_str),
            Some("nexus:temporal-types/model@1.0.0")
        );
        assert_eq!(
            linked_types
                .type_use_paths
                .get("signal-function")
                .map(String::as_str),
            Some("nexus:temporal-types/model@1.0.0")
        );
    }
}
