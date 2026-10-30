//! `ReachabilityPass` removes planned declarations that no generated surface
//! references.
//!
//! It derives reachability from the planned graph itself, rather than consuming
//! a side table produced by a previous pass.

use super::*;
use crate::spec::ModuleExport;

pub(crate) struct ReachabilityPass;

impl ReachabilityPass {
    pub(crate) fn new() -> Self {
        Self
    }
}

impl CompilerPass<PlannedFamily, PlannedFamily> for ReachabilityPass {
    type Error = Error;

    fn transform_leaf(
        &mut self,
        mut leaf: ApiSpecLeaf<PlannedFamily>,
    ) -> Result<ApiSpecLeaf<PlannedFamily>> {
        prune(&mut leaf.spec);
        Ok(leaf)
    }
}

fn prune(spec: &mut PlannedSpec) {
    let mut pending = Vec::new();
    let mut reachable = spec
        .types
        .iter()
        .filter(|(_, entry)| entry.is_module_export())
        .map(|(name, _)| name.clone())
        .collect::<BTreeSet<_>>();
    // Whether this spec's front end assigns declarations to modules at all. A
    // module that owns nothing still counts as scoped when it carries foreign
    // declarations, which is how a service file whose every operation type is
    // `$ref`d from another file avoids re-emitting all of them.
    let module_scoped = spec
        .types
        .values()
        .any(|entry| entry.module_export != ModuleExport::Unscoped);
    for service in &spec.services {
        for operation in &service.operations {
            pending.extend(
                [operation.input_type(), operation.output_type()]
                    .into_iter()
                    .flatten()
                    .cloned(),
            );
            if let Some(holder) = operation
                .data
                .output_resource_return
                .as_ref()
                .and_then(|resource_return| resource_return.holder.as_ref())
            {
                add_name(spec, &holder.record_full_name, &mut reachable);
            }
        }
        for resource in &service.resources {
            enqueue_resource(&resource.data, &mut pending);
        }
    }

    let mut expanded = BTreeSet::new();
    loop {
        while let Some(kind) = pending.pop() {
            enqueue_type_references(spec, &kind, &mut reachable, &mut pending);
        }
        let to_expand = reachable
            .iter()
            .filter(|name| expanded.insert((*name).clone()))
            .cloned()
            .collect::<Vec<_>>();
        if to_expand.is_empty() {
            break;
        }
        for name in to_expand {
            if let Some(declaration) = spec.types.get(&name) {
                enqueue_declaration_references(&declaration.declaration, &mut pending);
            }
        }
    }

    spec.types.retain(|name, entry| {
        reachable.contains(name)
            && (!matches!(entry.declaration, TypeDeclSpec::External(_))
                || !module_scoped
                || entry.is_module_export())
    });
}

fn enqueue_resource(resource: &PlannedResource, pending: &mut Vec<PlannedType>) {
    for field in &resource.fields {
        pending.push(field.kind.clone());
    }
    for method in &resource.methods {
        for param in &method.params {
            pending.push(param.kind.clone());
        }
        if let Some(PlannedResourceMethodResult {
            kind: PlannedResourceMethodResultKind::Value(kind),
            ..
        }) = &method.result
        {
            pending.push(kind.clone());
        }
    }
}

fn enqueue_type_references(
    spec: &PlannedSpec,
    kind: &PlannedType,
    reachable: &mut BTreeSet<String>,
    pending: &mut Vec<PlannedType>,
) {
    match kind {
        TypeSpec::Option(inner) | TypeSpec::List(inner) => pending.push((**inner).clone()),
        TypeSpec::Map(key, value) => {
            pending.push((**key).clone());
            pending.push((**value).clone());
        }
        TypeSpec::Tuple(items) => pending.extend(items.clone()),
        TypeSpec::Result { ok, err } => {
            pending.extend(ok.iter().chain(err.iter()).map(|kind| (**kind).clone()));
        }
        TypeSpec::Record(record) => add_name(spec, &record.full_name, reachable),
        TypeSpec::Enum(enumeration) => add_name(spec, &enumeration.full_name, reachable),
        TypeSpec::Flags(flags) => add_name(spec, &flags.full_name, reachable),
        TypeSpec::Variant(variant) => add_name(spec, &variant.full_name, reachable),
        TypeSpec::Resource(resource) => {
            if let Some(wire_type) = &resource.wire_type {
                enqueue_external_type(spec, wire_type, reachable, pending);
            }
        }
        TypeSpec::External(external) => enqueue_external_type(spec, external, reachable, pending),
        TypeSpec::Bool
        | TypeSpec::Int(_)
        | TypeSpec::Float
        | TypeSpec::String
        | TypeSpec::Bytes
        | TypeSpec::TypeParameter(_) => {}
    }
}

fn enqueue_external_type(
    spec: &PlannedSpec,
    external: &ExternalTypeSpec<PlannedFamily>,
    reachable: &mut BTreeSet<String>,
    pending: &mut Vec<PlannedType>,
) {
    match external {
        ExternalTypeSpec::Proto(PlannedProtoType::Message(message)) => {
            add_name(spec, &message.proto.full_name, reachable)
        }
        ExternalTypeSpec::Proto(PlannedProtoType::Enum(enumeration)) => {
            add_name(spec, &enumeration.proto.full_name, reachable)
        }
        ExternalTypeSpec::Json(json) => add_name(spec, &json.full_name, reachable),
        ExternalTypeSpec::Alias(alias) => pending.push((*alias.target).clone()),
    }
}

fn add_name(spec: &PlannedSpec, name: &str, reachable: &mut BTreeSet<String>) {
    if spec.types.contains_key(name) {
        reachable.insert(name.to_string());
    }
}

fn enqueue_declaration_references(
    declaration: &TypeDeclSpec<PlannedFamily>,
    pending: &mut Vec<PlannedType>,
) {
    match declaration {
        TypeDeclSpec::Record(record) => {
            pending.extend(
                record
                    .fields
                    .values()
                    .filter(|field| {
                        !field
                            .origin
                            .as_ref()
                            .is_some_and(|origin| origin.is_omitted())
                    })
                    .map(|field| field.field_type.clone()),
            );
        }
        TypeDeclSpec::Variant(variant) => {
            pending.extend(variant.cases.iter().filter_map(|case| case.payload.clone()));
        }
        TypeDeclSpec::External(binding) => {
            if let Some(proto) = binding.proto_alias() {
                pending.push(TypeSpec::External(ExternalTypeSpec::Proto(
                    proto.proto.clone(),
                )));
            } else if let Some(json) = binding.json_model() {
                pending.push(TypeSpec::External(ExternalTypeSpec::Json(json.clone())));
            }
            if let Some(authored_type) = binding.authored_type() {
                pending.push(authored_type.clone());
            }
        }
        TypeDeclSpec::Enum(_) | TypeDeclSpec::Flags(_) => {}
    }
}
