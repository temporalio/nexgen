//! Structural checks for the conformance manifest, without external toolchains.
//! Runtime execution belongs to the `validation-tests` package.

#[path = "../validation-tests/tests/support/json_schema/manifest.rs"]
mod manifest;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use manifest::*;
use serde_json::Value;

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum CoveragePhase {
    Load,
    Parse,
    RoundTrip,
    Serialize,
}

impl CoveragePhase {
    fn name(self) -> &'static str {
        match self {
            Self::Load => "load",
            Self::Parse => "parse",
            Self::RoundTrip => "round_trip",
            Self::Serialize => "serialize",
        }
    }
}

/// Every supported wire-affecting feature and every phase at which its contract
/// can fail. Annotation-only keywords are intentionally absent: they have no
/// wire verdict to drive and remain compile/import probes.
const COVERAGE_REQUIREMENTS: &[(&str, &[CoveragePhase])] = &[
    (
        "type",
        &[
            CoveragePhase::Parse,
            CoveragePhase::RoundTrip,
            CoveragePhase::Serialize,
        ],
    ),
    (
        "integer",
        &[
            CoveragePhase::Parse,
            CoveragePhase::RoundTrip,
            CoveragePhase::Serialize,
        ],
    ),
    ("required", &[CoveragePhase::Parse]),
    (
        "nullability",
        &[
            CoveragePhase::Parse,
            CoveragePhase::RoundTrip,
            CoveragePhase::Serialize,
        ],
    ),
    ("minimum", &[CoveragePhase::Parse, CoveragePhase::Serialize]),
    ("maximum", &[CoveragePhase::Parse, CoveragePhase::Serialize]),
    (
        "exclusiveMinimum",
        &[CoveragePhase::Parse, CoveragePhase::Serialize],
    ),
    (
        "exclusiveMaximum",
        &[CoveragePhase::Parse, CoveragePhase::Serialize],
    ),
    (
        "multipleOf",
        &[CoveragePhase::Parse, CoveragePhase::Serialize],
    ),
    (
        "minLength",
        &[CoveragePhase::Parse, CoveragePhase::Serialize],
    ),
    (
        "maxLength",
        &[CoveragePhase::Parse, CoveragePhase::Serialize],
    ),
    ("pattern", &[CoveragePhase::Parse, CoveragePhase::Serialize]),
    ("items", &[CoveragePhase::Parse, CoveragePhase::Serialize]),
    (
        "minItems",
        &[CoveragePhase::Parse, CoveragePhase::Serialize],
    ),
    (
        "maxItems",
        &[CoveragePhase::Parse, CoveragePhase::Serialize],
    ),
    (
        "uniqueItems",
        &[CoveragePhase::Parse, CoveragePhase::Serialize],
    ),
    (
        "contains",
        &[CoveragePhase::Parse, CoveragePhase::Serialize],
    ),
    (
        "minContains",
        &[CoveragePhase::Parse, CoveragePhase::Serialize],
    ),
    (
        "maxContains",
        &[CoveragePhase::Parse, CoveragePhase::Serialize],
    ),
    (
        "additionalProperties",
        &[
            CoveragePhase::Parse,
            CoveragePhase::RoundTrip,
            CoveragePhase::Serialize,
        ],
    ),
    (
        "minProperties",
        &[CoveragePhase::Parse, CoveragePhase::Serialize],
    ),
    (
        "maxProperties",
        &[CoveragePhase::Parse, CoveragePhase::Serialize],
    ),
    (
        "propertyNames",
        &[CoveragePhase::Parse, CoveragePhase::Serialize],
    ),
    (
        "dependentRequired",
        &[CoveragePhase::Parse, CoveragePhase::Serialize],
    ),
    (
        "allOf",
        &[
            CoveragePhase::Parse,
            CoveragePhase::RoundTrip,
            CoveragePhase::Serialize,
        ],
    ),
    (
        "ref.local",
        &[
            CoveragePhase::Parse,
            CoveragePhase::RoundTrip,
            CoveragePhase::Serialize,
        ],
    ),
    (
        "ref.cross_file",
        &[
            CoveragePhase::Parse,
            CoveragePhase::RoundTrip,
            CoveragePhase::Serialize,
        ],
    ),
    (
        "ref.recursive",
        &[
            CoveragePhase::Parse,
            CoveragePhase::RoundTrip,
            CoveragePhase::Serialize,
        ],
    ),
    (
        "oneOf.token",
        &[
            CoveragePhase::Parse,
            CoveragePhase::RoundTrip,
            CoveragePhase::Serialize,
        ],
    ),
    (
        "oneOf.discriminator",
        &[
            CoveragePhase::Parse,
            CoveragePhase::RoundTrip,
            CoveragePhase::Serialize,
        ],
    ),
    (
        "oneOf.branch_constraints",
        &[CoveragePhase::Parse, CoveragePhase::Serialize],
    ),
    ("const", &[CoveragePhase::Parse, CoveragePhase::Serialize]),
    ("enum", &[CoveragePhase::Parse, CoveragePhase::Serialize]),
    (
        "default",
        &[CoveragePhase::RoundTrip, CoveragePhase::Serialize],
    ),
    (
        "contentEncoding",
        &[
            CoveragePhase::Parse,
            CoveragePhase::RoundTrip,
            CoveragePhase::Serialize,
        ],
    ),
    (
        "format",
        &[
            CoveragePhase::Parse,
            CoveragePhase::RoundTrip,
            CoveragePhase::Serialize,
        ],
    ),
    (
        "duration",
        &[
            CoveragePhase::Parse,
            CoveragePhase::RoundTrip,
            CoveragePhase::Serialize,
        ],
    ),
    (
        "uri-reference",
        &[
            CoveragePhase::Parse,
            CoveragePhase::RoundTrip,
            CoveragePhase::Serialize,
        ],
    ),
    ("strict.anyOf", &[CoveragePhase::Load]),
    ("strict.not", &[CoveragePhase::Load]),
    ("strict.conditional", &[CoveragePhase::Load]),
    ("strict.dependentSchemas", &[CoveragePhase::Load]),
    ("strict.patternProperties", &[CoveragePhase::Load]),
    ("strict.prefixItems", &[CoveragePhase::Load]),
    ("strict.unevaluatedItems", &[CoveragePhase::Load]),
    ("strict.unevaluatedProperties", &[CoveragePhase::Load]),
    ("strict.contentMediaType", &[CoveragePhase::Load]),
    ("strict.contentSchema", &[CoveragePhase::Load]),
    ("strict.readOnly", &[CoveragePhase::Load]),
    ("strict.writeOnly", &[CoveragePhase::Load]),
    ("strict.deferredFormat", &[CoveragePhase::Load]),
    ("strict.deferredEncoding", &[CoveragePhase::Load]),
    ("strict.fractional_multipleOf", &[CoveragePhase::Load]),
    ("strict.mixedEnum", &[CoveragePhase::Load]),
    ("strict.compositeClosedValue", &[CoveragePhase::Load]),
    ("strict.undecidableOneOf", &[CoveragePhase::Load]),
];

fn assert_nonempty(value: &str, context: &str) {
    assert!(!value.trim().is_empty(), "{context} must not be empty");
}

fn assert_valid_json(value: &str, context: &str) {
    serde_json::from_str::<Value>(value)
        .unwrap_or_else(|error| panic!("{context} is not valid JSON: {error}"));
}

fn assert_unique_nonempty_paths(paths: &[String], context: &str) {
    assert!(!paths.is_empty(), "{context} must declare expected paths");
    let mut unique = BTreeSet::new();
    for path in paths {
        assert_nonempty(path, context);
        assert!(unique.insert(path), "{context} repeats path {path:?}");
    }
}

fn assert_valid_mutation(mutation: &Value, context: &str) {
    let object = mutation
        .as_object()
        .unwrap_or_else(|| panic!("{context} mutation must be an object"));
    assert_nonempty(
        object
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        &format!("{context} mutation path"),
    );
    let operators = [
        "set_integer",
        "set_number",
        "set_string",
        "set_null",
        "duplicate_element",
        "remove_array_element",
        "put_map_entry",
        "remove_map_entry",
        "set_absent",
        "set_bytes",
        "set_duration",
    ];
    let present = operators
        .iter()
        .filter(|operator| object.contains_key(**operator))
        .count();
    assert_eq!(
        present, 1,
        "{context} mutation must declare exactly one known operator: {mutation}"
    );
    assert!(
        object
            .keys()
            .all(|key| key == "path" || operators.contains(&key.as_str())),
        "{context} mutation contains an unknown key: {mutation}"
    );
}

fn required_coverage() -> BTreeSet<String> {
    COVERAGE_REQUIREMENTS
        .iter()
        .flat_map(|(feature, phases)| {
            phases
                .iter()
                .map(move |phase| format!("{feature}.{}", phase.name()))
        })
        .collect()
}

fn assert_coverage_claims(
    claims: &[String],
    phases: &[CoveragePhase],
    context: &str,
    required: &BTreeSet<String>,
    seen: &mut BTreeMap<String, String>,
) {
    let mut local = BTreeSet::new();
    for claim in claims {
        assert_nonempty(claim, context);
        assert!(
            phases
                .iter()
                .any(|phase| claim.ends_with(&format!(".{}", phase.name()))),
            "{context} claim {claim:?} is phase-incompatible; expected one of {:?}",
            phases.iter().map(|phase| phase.name()).collect::<Vec<_>>()
        );
        assert!(
            required.contains(claim),
            "{context} has unknown or stale coverage claim {claim:?}"
        );
        assert!(
            local.insert(claim),
            "{context} repeats coverage claim {claim:?}"
        );
        assert!(
            seen.insert(claim.clone(), context.to_string()).is_none(),
            "coverage claim {claim:?} is duplicated; each requirement needs one live owner"
        );
    }
}

fn corpus_document(root: &Path, name: &str) -> Value {
    let path = root
        .join("specs/json-schema/corpora")
        .join(name)
        .join("corpus.json");
    serde_json::from_slice(
        &fs::read(&path)
            .unwrap_or_else(|error| panic!("read coverage corpus {}: {error}", path.display())),
    )
    .unwrap_or_else(|error| panic!("parse coverage corpus {}: {error}", path.display()))
}

fn seed_corpus_coverage(
    root: &Path,
    required: &BTreeSet<String>,
    seen: &mut BTreeMap<String, String>,
) {
    let pattern = corpus_document(root, "pattern_conformance");
    let pairs = pattern["pairs"].as_array().expect("pattern corpus pairs");
    let gate = pairs
        .iter()
        .filter(|row| row["expect_gate_reject"].as_bool() == Some(true))
        .count();
    assert_eq!(
        pairs.len() - gate,
        97,
        "pattern runtime corpus anchor drifted"
    );
    assert_eq!(gate, 44, "pattern loader-gate corpus anchor drifted");

    for (name, array) in [
        ("format_conformance", "pairs"),
        ("format_duration", "cases"),
        ("format_uri_reference", "cases"),
        ("format_materialize_clock", "date-time"),
    ] {
        let document = corpus_document(root, name);
        assert!(
            document[array]
                .as_array()
                .is_some_and(|rows| !rows.is_empty()),
            "{name} corpus anchor is empty or absent"
        );
    }

    for (claim, phase) in [
        ("pattern.parse", CoveragePhase::Parse),
        ("format.parse", CoveragePhase::Parse),
        ("format.round_trip", CoveragePhase::RoundTrip),
        ("duration.parse", CoveragePhase::Parse),
        ("duration.round_trip", CoveragePhase::RoundTrip),
        ("uri-reference.parse", CoveragePhase::Parse),
        ("uri-reference.round_trip", CoveragePhase::RoundTrip),
    ] {
        assert_coverage_claims(
            &[claim.to_string()],
            &[phase],
            &format!("{claim} corpus anchor"),
            required,
            seen,
        );
    }
}

fn assert_complete_coverage(required: &BTreeSet<String>, coverage: &BTreeMap<String, String>) {
    let covered = coverage.keys().cloned().collect::<BTreeSet<_>>();
    let missing = required.difference(&covered).cloned().collect::<Vec<_>>();
    assert!(
        missing.is_empty(),
        "conformance coverage is incomplete; missing claims: {missing:?}"
    );
}

#[test]
fn json_schema_conformance_manifest_is_structural_and_consumed() {
    let root = repository_root();
    let manifest = load_manifest(&root);

    assert_eq!(
        manifest.version, 3,
        "unsupported conformance manifest version"
    );
    assert!(!manifest.cases.is_empty(), "manifest must contain cases");

    let target_names = ["go", "java", "python", "typescript"]
        .into_iter()
        .collect::<BTreeSet<_>>();
    let mut case_ids = BTreeSet::new();
    let required = required_coverage();
    let mut coverage = BTreeMap::new();
    seed_corpus_coverage(&root, &required, &mut coverage);

    for case in &manifest.cases {
        assert_nonempty(&case.id, "case id");
        assert!(
            case.id
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'),
            "case id must be lowercase kebab-case: {:?}",
            case.id
        );
        assert!(
            case_ids.insert(case.id.clone()),
            "duplicate case id: {}",
            case.id
        );
        assert_nonempty(&case.intent, &format!("{} intent", case.id));

        assert!(!case.schemas.is_empty(), "{} must declare schemas", case.id);
        let mut schema_paths = BTreeSet::new();
        for schema in &case.schemas {
            let schema_path = repository_path(&root, schema);
            assert!(
                schema_path.is_file(),
                "{} schema does not exist: {schema}",
                case.id
            );
            assert!(
                schema_paths.insert(schema),
                "{} repeats schema input {schema}",
                case.id
            );
        }

        match case.expected_load.result {
            LoadResult::Accepted => {
                assert!(
                    case.expected_load.diagnostic.is_none(),
                    "{} accepted load must not declare a diagnostic",
                    case.id
                );
                assert!(
                    case.model.is_some(),
                    "{} must name the model its wire values drive",
                    case.id
                );
                assert!(
                    !case.accepted_wire_values.is_empty()
                        || !case.parse_failures.is_empty()
                        || !case.serialize_failures.is_empty(),
                    "{} accepted load must declare at least one wire value",
                    case.id
                );
                assert!(
                    case.expected_load.covers.is_empty(),
                    "{} accepted load must not declare load coverage",
                    case.id
                );
            }
            LoadResult::Rejected => {
                assert!(
                    case.accepted_wire_values.is_empty()
                        && case.parse_failures.is_empty()
                        && case.serialize_failures.is_empty(),
                    "{} rejected load must not declare wire values",
                    case.id
                );
                assert_nonempty(
                    case.expected_load.diagnostic.as_deref().unwrap_or_default(),
                    &format!("{} rejected-load diagnostic", case.id),
                );
                assert!(
                    !case.expected_load.covers.is_empty(),
                    "{} rejected load must declare coverage",
                    case.id
                );
                assert_coverage_claims(
                    &case.expected_load.covers,
                    &[CoveragePhase::Load],
                    &format!("{} expected_load", case.id),
                    &required,
                    &mut coverage,
                );
            }
        }

        let mut fixture_names = Vec::new();
        for (index, value) in case.accepted_wire_values.iter().enumerate() {
            let context = format!("{} accepted_wire_values[{index}]", case.id);
            assert_eq!(
                usize::from(value.fixture.is_some()) + usize::from(value.wire_json.is_some()),
                1,
                "{context} must declare exactly one of fixture or wire_json"
            );
            if let Some(fixture) = &value.fixture {
                let fixture_path = repository_path(&root, fixture);
                assert!(
                    fixture_path.is_file(),
                    "{context} fixture does not exist: {fixture}"
                );
                fixture_names.push(
                    fixture_path
                        .file_name()
                        .expect("fixture file name")
                        .to_string_lossy()
                        .into_owned(),
                );
            }
            if let Some(wire_json) = &value.wire_json {
                assert_valid_json(wire_json, &context);
            }
            if let Some(expected) = &value.expected_wire {
                assert_valid_json(expected, &format!("{context} expected_wire"));
            }
            for (mutation_index, mutation) in value.mutations.iter().enumerate() {
                assert_valid_mutation(mutation, &format!("{context} mutations[{mutation_index}]"));
            }
            assert_coverage_claims(
                &value.covers,
                &[CoveragePhase::RoundTrip, CoveragePhase::Serialize],
                &context,
                &required,
                &mut coverage,
            );
        }

        for (index, failure) in case.parse_failures.iter().enumerate() {
            let context = format!("{} parse_failures[{index}]", case.id);
            assert_valid_json(&failure.wire_json, &context);
            assert_unique_nonempty_paths(&failure.expected_paths, &context);
            assert_coverage_claims(
                &failure.covers,
                &[CoveragePhase::Parse],
                &context,
                &required,
                &mut coverage,
            );
        }

        for (index, failure) in case.serialize_failures.iter().enumerate() {
            let context = format!("{} serialize_failures[{index}]", case.id);
            assert_eq!(
                usize::from(failure.from_wire.is_some())
                    + usize::from(failure.from_fixture.is_some()),
                1,
                "{context} must declare exactly one of from_wire or from_fixture"
            );
            if let Some(wire) = &failure.from_wire {
                assert_valid_json(wire, &context);
            }
            if let Some(fixture) = &failure.from_fixture {
                assert!(
                    repository_path(&root, fixture).is_file(),
                    "{context} fixture does not exist: {fixture}"
                );
            }
            assert!(
                !failure.mutations.is_empty(),
                "{context} must declare a native mutation"
            );
            for (mutation_index, mutation) in failure.mutations.iter().enumerate() {
                assert_valid_mutation(mutation, &format!("{context} mutations[{mutation_index}]"));
            }
            assert_unique_nonempty_paths(&failure.expected_paths, &context);
            assert_coverage_claims(
                &failure.covers,
                &[CoveragePhase::Serialize],
                &context,
                &required,
                &mut coverage,
            );
        }

        for (index, collapse) in case
            .permitted_presence_nullability_collapse
            .iter()
            .enumerate()
        {
            let context = format!("{} permitted collapse[{index}]", case.id);
            assert_nonempty(&collapse.path, &context);
            assert_ne!(
                collapse.wire_presence, collapse.serialized_presence,
                "{context} must describe an actual presence change"
            );
            assert!(!collapse.targets.is_empty(), "{context} must name targets");
            let targets = collapse
                .targets
                .iter()
                .map(String::as_str)
                .collect::<BTreeSet<_>>();
            assert_eq!(
                targets.len(),
                collapse.targets.len(),
                "{context} repeats a target"
            );
            assert!(
                targets.is_subset(&target_names),
                "{context} names an unsupported target: {:?}",
                collapse.targets
            );
        }

        if let Some(divergence) = &case.expected_divergence {
            assert!(
                !divergence.findings.is_empty(),
                "{} expected_divergence must cite the findings it pins",
                case.id
            );
            assert!(
                !divergence.matches.is_empty(),
                "{} expected_divergence must classify the findings it tolerates",
                case.id
            );
            assert_nonempty(&divergence.note, &format!("{} divergence note", case.id));
        }

        if case.consumers.is_empty() {
            continue;
        }
        let actual_targets = case
            .consumers
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            actual_targets, target_names,
            "{} declares consumers, so it must declare one for every target",
            case.id
        );
        for (target, consumer) in &case.consumers {
            assert_nonempty(&consumer.anchor, &format!("{} {target} anchor", case.id));
            let source_path = repository_path(&root, &consumer.source);
            let source = fs::read_to_string(&source_path).unwrap_or_else(|error| {
                panic!(
                    "{} {target} consumer source cannot be read ({}): {error}",
                    case.id, consumer.source
                )
            });
            assert!(
                source.contains(&consumer.anchor),
                "{} {target} consumer anchor {:?} is absent from {}",
                case.id,
                consumer.anchor,
                consumer.source
            );
            for fixture_name in &fixture_names {
                assert!(
                    source.contains(fixture_name),
                    "{} {target} consumer {} does not mention fixture {fixture_name}",
                    case.id,
                    consumer.source
                );
            }
        }
    }

    assert_complete_coverage(&required, &coverage);
}

#[test]
#[should_panic(expected = "conformance coverage is incomplete")]
fn structural_coverage_rejects_a_removed_requirement_claim() {
    let required = required_coverage();
    let mut coverage = required
        .iter()
        .map(|claim| (claim.clone(), "test".to_string()))
        .collect::<BTreeMap<_, _>>();
    coverage.remove(required.first().expect("coverage requirement"));
    assert_complete_coverage(&required, &coverage);
}

#[test]
#[should_panic(expected = "pattern runtime corpus anchor drifted")]
fn structural_coverage_rejects_a_removed_corpus_anchor() {
    let root = repository_root();
    let mut pattern = corpus_document(&root, "pattern_conformance");
    let pairs = pattern["pairs"]
        .as_array_mut()
        .expect("pattern corpus pairs");
    let at = pairs
        .iter()
        .position(|row| row["expect_gate_reject"].as_bool() != Some(true))
        .expect("runtime pattern row");
    pairs.remove(at);
    let pairs = pattern["pairs"].as_array().expect("pattern corpus pairs");
    let gate = pairs
        .iter()
        .filter(|row| row["expect_gate_reject"].as_bool() == Some(true))
        .count();
    assert_eq!(
        pairs.len() - gate,
        97,
        "pattern runtime corpus anchor drifted"
    );
}
