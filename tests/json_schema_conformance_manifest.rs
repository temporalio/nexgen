use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

const MANIFEST_PATH: &str = "samples/conformance/json-schema.json";
const TARGETS: [&str; 4] = ["go", "java", "python", "typescript"];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u64,
    cases: Vec<Case>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    id: String,
    schema: String,
    expected_load: ExpectedLoad,
    accepted_wire_values: Vec<AcceptedWireValue>,
    parse_failures: Vec<Failure>,
    serialize_failures: Vec<Failure>,
    permitted_presence_nullability_collapse: Vec<PresenceCollapse>,
    consumers: BTreeMap<String, Consumer>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpectedLoad {
    result: LoadResult,
    diagnostic: Option<String>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum LoadResult {
    Accepted,
    Rejected,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AcceptedWireValue {
    fixture: Option<String>,
    wire_json: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Failure {
    wire_json: Option<String>,
    native_value: Option<String>,
    expected_paths: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PresenceCollapse {
    path: String,
    targets: Vec<String>,
    wire_presence: Presence,
    serialized_presence: Presence,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Presence {
    Absent,
    ExplicitNull,
    Present,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Consumer {
    source: String,
    anchor: String,
}

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn repository_path(root: &Path, relative: &str) -> PathBuf {
    let path = Path::new(relative);
    assert!(
        !path.is_absolute(),
        "manifest path must be relative: {relative}"
    );
    assert!(
        path.components()
            .all(|component| !matches!(component, Component::ParentDir)),
        "manifest path must not traverse out of the repository: {relative}"
    );
    root.join(path)
}

fn assert_nonempty(value: &str, context: &str) {
    assert!(!value.trim().is_empty(), "{context} must not be empty");
}

fn assert_valid_json(value: &str, context: &str) {
    serde_json::from_str::<serde_json::Value>(value)
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

#[test]
fn json_schema_conformance_manifest_is_structural_and_consumed() {
    let root = repository_root();
    let manifest_bytes = fs::read(repository_path(&root, MANIFEST_PATH)).unwrap();
    let manifest: Manifest = serde_json::from_slice(&manifest_bytes).unwrap();

    assert_eq!(
        manifest.version, 1,
        "unsupported conformance manifest version"
    );
    assert!(!manifest.cases.is_empty(), "manifest must contain cases");

    let expected_targets = TARGETS.into_iter().collect::<BTreeSet<_>>();
    let mut case_ids = BTreeSet::new();

    for case in manifest.cases {
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

        let schema_path = repository_path(&root, &case.schema);
        assert!(
            schema_path.is_file(),
            "{} schema does not exist: {}",
            case.id,
            case.schema
        );

        match case.expected_load.result {
            LoadResult::Accepted => {
                assert!(
                    case.expected_load.diagnostic.is_none(),
                    "{} accepted load must not declare a diagnostic",
                    case.id
                );
                assert!(
                    !case.accepted_wire_values.is_empty(),
                    "{} accepted load must declare an accepted wire value",
                    case.id
                );
            }
            LoadResult::Rejected => {
                assert!(
                    case.accepted_wire_values.is_empty(),
                    "{} rejected load must not declare accepted wire values",
                    case.id
                );
                assert_nonempty(
                    case.expected_load.diagnostic.as_deref().unwrap_or_default(),
                    &format!("{} rejected-load diagnostic", case.id),
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
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                );
            }
            if let Some(wire_json) = &value.wire_json {
                assert_valid_json(wire_json, &context);
            }
        }

        for (index, failure) in case.parse_failures.iter().enumerate() {
            let context = format!("{} parse_failures[{index}]", case.id);
            assert!(
                failure.native_value.is_none(),
                "{context} must use wire_json"
            );
            let wire_json = failure
                .wire_json
                .as_deref()
                .unwrap_or_else(|| panic!("{context} must declare wire_json"));
            assert_valid_json(wire_json, &context);
            assert_unique_nonempty_paths(&failure.expected_paths, &context);
        }

        for (index, failure) in case.serialize_failures.iter().enumerate() {
            let context = format!("{} serialize_failures[{index}]", case.id);
            assert!(
                failure.wire_json.is_none(),
                "{context} must use native_value"
            );
            assert_nonempty(
                failure.native_value.as_deref().unwrap_or_default(),
                &format!("{context} native_value"),
            );
            assert_unique_nonempty_paths(&failure.expected_paths, &context);
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
                targets.is_subset(&expected_targets),
                "{context} names an unsupported target: {:?}",
                collapse.targets
            );
        }

        let actual_targets = case
            .consumers
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            actual_targets, expected_targets,
            "{} must declare one consumer for every target",
            case.id
        );

        for (target, consumer) in case.consumers {
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
}
