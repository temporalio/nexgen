//! Refuse a client poll on an operation that can't get an iterator, or whose `items` field isn't
//! a list in the model. The check runs before any generator, so it doesn't depend on the
//! language. It runs on the planned spec, since an operation that returns a resource is known
//! only after resource resolution.

use std::path::Path;

use super::{PlannedFamily, PlannedSpec, operation_input_model, operation_output_direct_result};
use crate::error::{Error, Result};
use crate::nexgen_config::{self, GenerationTarget};
use crate::spec::{ApiSpecLeaf, CompilerPass, FieldPolicy, OperationSpec, TypeSpec};

pub(crate) struct PollValidationPass;

impl CompilerPass<PlannedFamily, PlannedFamily> for PollValidationPass {
    type Error = Error;

    fn transform_leaf(
        &mut self,
        leaf: ApiSpecLeaf<PlannedFamily>,
    ) -> Result<ApiSpecLeaf<PlannedFamily>> {
        // Only the client target gets an iterator.
        if nexgen_config::current()
            .targets
            .contains(&GenerationTarget::Client)
        {
            validate_client_polls(&leaf.spec, &leaf.source_path)?;
        }
        Ok(leaf)
    }
}

fn validate_client_polls(spec: &PlannedSpec, path: &Path) -> Result<()> {
    for service in &spec.services {
        for operation in &service.operations {
            let Some(poll) = operation
                .poll
                .as_ref()
                .filter(|poll| poll.targets.contains(&GenerationTarget::Client))
            else {
                continue;
            };
            let has_function_inputs = match operation_input_model(operation) {
                Some(TypeSpec::Record(record)) => spec
                    .record(&record.full_name)
                    .is_some_and(|record| record.functions().next().is_some()),
                _ => false,
            };
            let reason = if has_function_inputs {
                Some("has function inputs".to_string())
            } else if operation.output_transform.is_some() {
                Some("has an output transform".to_string())
            } else if operation.data.output_resource_return.is_some()
                || operation_output_direct_result(operation)
            {
                Some("returns a resource".to_string())
            } else if operation.output_type().is_none() {
                Some("returns nothing".to_string())
            } else {
                poll.items
                    .as_deref()
                    .and_then(|key| items_reason(spec, operation, key))
            };
            if let Some(reason) = reason {
                return Err(Error::UnsupportedPoll {
                    path: path.to_path_buf(),
                    operation: format!("{}.{}", service.name, operation.name),
                    reason,
                });
            }
        }
    }
    Ok(())
}

/// Why the `items` field of the response has no items to yield, when it has none.
fn items_reason(
    spec: &PlannedSpec,
    operation: &OperationSpec<PlannedFamily>,
    key: &str,
) -> Option<String> {
    let field = match operation.output_type() {
        Some(TypeSpec::Record(record)) => spec
            .record(&record.full_name)
            .and_then(|record| record.fields.get(key)),
        _ => None,
    };
    let Some(field) = field else {
        return Some(format!(
            "has an `items` field `{key}` that its response doesn't have"
        ));
    };
    if field.origin.as_ref().is_some_and(FieldPolicy::is_omitted) {
        return Some(format!(
            "has an `items` field `{key}` that the response model omits"
        ));
    }
    let field_type = match &field.field_type {
        TypeSpec::Option(inner) => inner.as_ref(),
        field_type => field_type,
    };
    // A proto `bytes` field is a WIT `list<u8>`, but the model has no list to yield.
    if !matches!(field_type, TypeSpec::List(_)) {
        return Some(format!(
            "has an `items` field `{key}` that isn't a list in its response"
        ));
    }
    None
}
