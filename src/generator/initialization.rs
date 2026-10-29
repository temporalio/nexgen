//! Selected expressions for defaults on exposed operation arguments.

use crate::planning::PlannedFamily;
use crate::spec::RecordFieldSpec;

pub(super) fn default_expression(field: &RecordFieldSpec<PlannedFamily>) -> Option<&str> {
    field.default_value.as_ref().and_then(|text| text.value())
}
