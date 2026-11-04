//! Refuse a client poll on an operation that can't get an iterator. The check runs before any
//! generator, so it doesn't depend on the language.

use std::path::Path;

use super::{PlannedFamily, PlannedSpec, operation_input_model, operation_output_direct_result};
use crate::error::{Error, Result};
use crate::nexgen_config::GenerationTarget;
use crate::spec::{ApiSpecLeaf, CompilerPass, TypeSpec};

pub(crate) struct PollValidationPass {
    targets: Vec<GenerationTarget>,
}

impl PollValidationPass {
    pub(crate) fn new(targets: Vec<GenerationTarget>) -> Self {
        Self { targets }
    }
}

impl CompilerPass<PlannedFamily, PlannedFamily> for PollValidationPass {
    type Error = Error;

    fn transform_leaf(
        &mut self,
        leaf: ApiSpecLeaf<PlannedFamily>,
    ) -> Result<ApiSpecLeaf<PlannedFamily>> {
        // Only the client target gets an iterator.
        if self.targets.contains(&GenerationTarget::Client) {
            validate_client_polls(&leaf.spec, &leaf.source_path)?;
        }
        Ok(leaf)
    }
}

fn validate_client_polls(spec: &PlannedSpec, path: &Path) -> Result<()> {
    for service in &spec.services {
        for operation in &service.operations {
            let polled = operation
                .poll
                .as_ref()
                .is_some_and(|poll| poll.targets.contains(&GenerationTarget::Client));
            if !polled {
                continue;
            }
            let has_function_inputs = match operation_input_model(operation) {
                Some(TypeSpec::Record(record)) => spec
                    .record(&record.full_name)
                    .is_some_and(|record| record.functions().next().is_some()),
                _ => false,
            };
            let reason = if has_function_inputs {
                "has function inputs"
            } else if operation.output_transform.is_some() {
                "has an output transform"
            } else if operation.data.output_resource_return.is_some()
                || operation_output_direct_result(operation)
            {
                "returns a resource"
            } else if operation.output_type().is_none() {
                "returns nothing"
            } else {
                continue;
            };
            return Err(Error::UnsupportedPoll {
                path: path.to_path_buf(),
                operation: format!("{}.{}", service.name, operation.name),
                reason: reason.to_string(),
            });
        }
    }
    Ok(())
}
