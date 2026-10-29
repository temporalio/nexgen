//! Language-neutral reachability for outbound default initialization.
//!
//! Backends own presence tests, copying, expression scope, and rendering. This
//! analysis only identifies records whose outbound inputs need preparation.

use std::collections::BTreeSet;

use crate::planning::{PlannedFamily, PlannedSpec, PlannedType};
use crate::spec::{ExternalTypeSpec, RecordFieldSpec, RecordFieldVisibility};

pub(super) fn default_expression(field: &RecordFieldSpec<PlannedFamily>) -> Option<&str> {
    field.default_value.as_ref().and_then(|text| text.value())
}

pub(super) fn records_with_defaults(spec: &PlannedSpec) -> BTreeSet<String> {
    let mut records = BTreeSet::new();
    loop {
        let mut changed = false;
        for (name, record) in spec.records() {
            if !records.contains(name)
                && record.fields.values().any(|field| {
                    field.visibility != RecordFieldVisibility::Omitted
                        && (default_expression(field).is_some()
                            || type_has_defaults(&field.field_type, &records))
                })
            {
                records.insert(name.to_string());
                changed = true;
            }
        }
        if !changed {
            return records;
        }
    }
}

pub(super) fn type_has_defaults(value: &PlannedType, records: &BTreeSet<String>) -> bool {
    match value {
        PlannedType::Record(record) => records.contains(&record.full_name),
        PlannedType::Option(inner) => type_has_defaults(inner, records),
        PlannedType::External(ExternalTypeSpec::Alias(alias))
            if alias.type_name.value().is_none() =>
        {
            type_has_defaults(&alias.target, records)
        }
        _ => false,
    }
}

pub(super) fn input_record<'a>(
    value: &'a PlannedType,
    records: &BTreeSet<String>,
) -> Option<&'a crate::planning::PlannedRecordType> {
    match value {
        PlannedType::Record(record) if records.contains(&record.full_name) => Some(record),
        PlannedType::Option(inner) => input_record(inner, records),
        PlannedType::External(ExternalTypeSpec::Alias(alias))
            if alias.type_name.value().is_none() =>
        {
            input_record(&alias.target, records)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::planning::PlannedRecordType;

    #[test]
    fn initialization_stops_at_container_boundaries() {
        let record = PlannedType::Record(PlannedRecordType {
            full_name: "example.record".to_string(),
            model_name: "Record".to_string(),
        });
        let records = BTreeSet::from(["example.record".to_string()]);
        assert!(type_has_defaults(&record, &records));
        assert!(type_has_defaults(
            &PlannedType::Option(Box::new(record.clone())),
            &records
        ));
        assert!(!type_has_defaults(
            &PlannedType::List(Box::new(record.clone())),
            &records
        ));
        assert!(!type_has_defaults(
            &PlannedType::Map(Box::new(PlannedType::String), Box::new(record)),
            &records,
        ));
    }
}
