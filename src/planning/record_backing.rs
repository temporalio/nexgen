//! Validate record containment and JSON resource-result constructor fields.

use std::collections::BTreeSet;

use crate::descriptors::DescriptorIndex;
use crate::error::{Error, Result};
use crate::language::Language;
use crate::spec::{
    ApiSpec, ExternalTypeSourceSpec, ExternalTypeSpec, FieldOrigin, RecordSpec, TypeSpec,
};

#[derive(Clone, Copy)]
enum BackingOwner<'a> {
    Record(&'a RecordSpec),
    JsonResource { service: &'a str, resource: &'a str },
}

impl BackingOwner<'_> {
    fn is_proto(self) -> bool {
        match self {
            Self::Record(record) => is_proto(record),
            Self::JsonResource { .. } => false,
        }
    }
}

pub(super) fn validate(
    spec: &ApiSpec,
    descriptors: &DescriptorIndex,
    language: Language,
) -> Result<()> {
    for (_, record) in spec.records() {
        for field in record.fields.values().filter(|field| {
            matches!(
                field.origin.for_language(language),
                None | Some(FieldOrigin::Default { .. } | FieldOrigin::ApiOmitted)
            )
        }) {
            validate_nested(
                spec,
                descriptors,
                BackingOwner::Record(record),
                &field.field_type,
                &field.name,
                &mut BTreeSet::new(),
            )?;
        }
    }
    for service in &spec.services {
        for operation in &service.operations {
            if let Some(TypeSpec::Resource(output)) = operation.output_type()
                && output.wire_type.is_none()
                && let Some(resource) = service.resource(output.as_str())
            {
                for field in &resource.fields {
                    if field.function.is_none() {
                        validate_nested(
                            spec,
                            descriptors,
                            BackingOwner::JsonResource {
                                service: &service.name,
                                resource: &resource.name,
                            },
                            &field.field_type,
                            &field.name,
                            &mut BTreeSet::new(),
                        )?;
                    }
                }
            }
        }
    }
    Ok(())
}

fn validate_nested(
    spec: &ApiSpec,
    descriptors: &DescriptorIndex,
    owner: BackingOwner<'_>,
    kind: &TypeSpec,
    path: &str,
    visited_variants: &mut BTreeSet<String>,
) -> Result<()> {
    let nested = match kind {
        TypeSpec::Record(name) => spec.record(name.as_ref()),
        TypeSpec::External(ExternalTypeSpec::Proto(name)) => spec.record_for_proto(name.as_ref()),
        _ => None,
    };
    if let Some(nested) = nested {
        // Each record is checked separately, so record cycles do not require
        // recursively visiting its fields from every parent.
        return check_backing(owner, path, &nested.full_name, is_proto(nested));
    }
    match kind {
        TypeSpec::External(ExternalTypeSpec::Proto(name)) => {
            // Replaced external types own their conversions (e.g. a duration
            // represented by a string). An unreplaced message is still a
            // protobuf object even when it has no authored record declaration.
            let replaced = spec
                .external_type_binding(name.as_ref())
                .is_some_and(|binding| binding.replacement().is_some());
            if !replaced && descriptors.message(name.as_ref()).is_some() {
                check_backing(owner, path, name.as_ref(), true)?;
            }
        }
        TypeSpec::External(ExternalTypeSpec::Alias(alias)) => {
            validate_nested(
                spec,
                descriptors,
                owner,
                &alias.target,
                path,
                visited_variants,
            )?;
        }
        TypeSpec::Option(inner) => {
            validate_nested(spec, descriptors, owner, inner, path, visited_variants)?;
        }
        TypeSpec::List(inner) => {
            validate_nested(
                spec,
                descriptors,
                owner,
                inner,
                &format!("{path}[]"),
                visited_variants,
            )?;
        }
        TypeSpec::Map(key, value) => {
            validate_nested(
                spec,
                descriptors,
                owner,
                key,
                &format!("{path}.key"),
                visited_variants,
            )?;
            validate_nested(
                spec,
                descriptors,
                owner,
                value,
                &format!("{path}.value"),
                visited_variants,
            )?;
        }
        TypeSpec::Tuple(items) => {
            for (index, item) in items.iter().enumerate() {
                validate_nested(
                    spec,
                    descriptors,
                    owner,
                    item,
                    &format!("{path}[{index}]"),
                    visited_variants,
                )?;
            }
        }
        TypeSpec::Result { ok, err } => {
            if let Some(ok) = ok {
                validate_nested(
                    spec,
                    descriptors,
                    owner,
                    ok,
                    &format!("{path}.ok"),
                    visited_variants,
                )?;
            }
            if let Some(err) = err {
                validate_nested(
                    spec,
                    descriptors,
                    owner,
                    err,
                    &format!("{path}.err"),
                    visited_variants,
                )?;
            }
        }
        TypeSpec::Variant(name) if visited_variants.insert(name.as_ref().to_string()) => {
            if let Some(variant) = spec.variant(name.as_ref()) {
                for case in &variant.cases {
                    if let Some(payload) = &case.payload {
                        validate_nested(
                            spec,
                            descriptors,
                            owner,
                            payload,
                            &format!("{path}.{}", case.name),
                            visited_variants,
                        )?;
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn is_proto(record: &RecordSpec) -> bool {
    matches!(record.source, Some(ExternalTypeSourceSpec::Proto(_)))
}

fn check_backing(
    owner: BackingOwner<'_>,
    path: &str,
    nested_name: &str,
    nested_proto: bool,
) -> Result<()> {
    if owner.is_proto() != nested_proto {
        return Err(match owner {
            BackingOwner::Record(record) => Error::MixedRecordBacking {
                record: record.full_name.clone(),
                field: path.to_string(),
                backing: if is_proto(record) { "protobuf" } else { "JSON" },
                nested_record: nested_name.to_string(),
                nested_backing: if nested_proto { "protobuf" } else { "JSON" },
            },
            BackingOwner::JsonResource { service, resource } => Error::InvalidResource {
                service: service.to_string(),
                resource: resource.to_string(),
                reason: format!(
                    "JSON resource result constructor field `{path}` contains protobuf-backed \
                     record `{nested_name}`; remove `@nexus.proto` from the nested record, or \
                     return the resource through a protobuf-backed result"
                ),
            },
        });
    }
    Ok(())
}
