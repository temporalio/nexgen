//! The cross-language conformance manifest: its shape, and its execution.
//!
//! `samples/conformance/json-schema.json` is the repository's statement of P1 —
//! "a value one target accepts round-trips through any other unchanged". Until
//! this file executed it, the statement was asserted by four independently
//! hand-written suites over four *different* schemas, and forty-seven P0
//! divergences accumulated behind that gap.
//!
//! Two tests live here:
//!
//! * [`json_schema_conformance_manifest_is_structural_and_consumed`] keeps the
//!   manifest well-formed and its declared consumer anchors alive.
//! * [`json_schema_conformance_cases_agree_across_targets`] *runs* it: every
//!   case is generated into Go, Java, Python and TypeScript, every declared wire
//!   value is pushed through the generated code of all four, and the verdicts
//!   must agree with the manifest **and with each other**.
//!
//! A case that a target still gets wrong carries an `expected_divergence` naming
//! the gap-analysis findings that own it. The driver then tolerates that case's
//! failures — and fails if the case starts passing, so the marker cannot outlive
//! the bug.

mod toolchain;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

use toolchain::{
    PlanCase, Probe, ProbeKind, TARGETS, Target, TargetVerdicts, Verdict, Workspace,
    canonical_value, repository_root,
};

const MANIFEST_PATH: &str = "samples/conformance/json-schema.json";

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
    /// Prose stating what the case pins. Not asserted; read by whoever the
    /// driver's output lands on.
    intent: String,
    schemas: Vec<String>,
    /// The generated model every wire value is driven through. Conformance
    /// schemas keep this name identical in all four targets.
    model: Option<String>,
    expected_load: ExpectedLoad,
    #[serde(default)]
    accepted_wire_values: Vec<AcceptedWireValue>,
    #[serde(default)]
    parse_failures: Vec<ParseFailure>,
    #[serde(default)]
    serialize_failures: Vec<SerializeFailure>,
    #[serde(default)]
    permitted_presence_nullability_collapse: Vec<PresenceCollapse>,
    /// Findings this case still reproduces. Present = the case is allowed to
    /// fail; absent = it must pass.
    #[serde(default)]
    expected_divergence: Option<ExpectedDivergence>,
    #[serde(default)]
    consumers: BTreeMap<String, Consumer>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpectedDivergence {
    /// Gap-analysis finding ids, or `new:<slug>` for a divergence this driver
    /// measured first.
    findings: Vec<String>,
    /// Whether this is a defect awaiting a fix or a permanent property of the
    /// target.
    status: DivergenceStatus,
    note: String,
    /// Substrings that classify a driver finding as *this* known divergence.
    /// Every observed finding must match one, and every entry must match at
    /// least one finding, so a pinned case cannot quietly absorb a new bug or
    /// keep a stale expectation.
    matches: Vec<String>,
}

/// Why a case is allowed to diverge.
///
/// The distinction matters for triage, not for the assertion: both keep their
/// `matches` live, so either kind fails the driver the day it stops happening.
/// `structural` says nobody should go looking for a fix — the target cannot
/// express the value, so the honest move is to document it (decision D8's
/// shape) rather than leave it on an open list forever.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum DivergenceStatus {
    /// A defect. Someone owns it and it is expected to close.
    Open,
    /// A permanent limitation of the target, not a bug to be fixed.
    Structural,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpectedLoad {
    result: LoadResult,
    diagnostic: Option<String>,
    #[serde(default)]
    covers: Vec<String>,
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
    /// The wire every target must re-emit, when it is not the input itself
    /// (canonicalizing inputs: lowercase `t`/`z`, `PT90M`, `+00:00`).
    expected_wire: Option<String>,
    #[serde(default)]
    mutations: Vec<Value>,
    covers: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ParseFailure {
    wire_json: String,
    expected_paths: Vec<String>,
    covers: Vec<String>,
}

/// A serialize-side rejection: parse `from_wire`, mutate the **native** value,
/// then serialize. Mutating a parsed model is the only language-neutral way to
/// reach a native state the parser would never produce (an over-cap integer, a
/// non-finite `number`, a duplicate array element).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SerializeFailure {
    from_wire: Option<String>,
    from_fixture: Option<String>,
    mutations: Vec<Value>,
    expected_paths: Vec<String>,
    covers: Vec<String>,
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PresenceCollapse {
    path: String,
    targets: Vec<String>,
    wire_presence: Presence,
    serialized_presence: Presence,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Presence {
    Absent,
    ExplicitNull,
    Present,
}

impl Presence {
    fn of(value: Option<&Value>) -> Self {
        match value {
            None => Presence::Absent,
            Some(Value::Null) => Presence::ExplicitNull,
            Some(_) => Presence::Present,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Presence::Absent => "absent",
            Presence::ExplicitNull => "explicit_null",
            Presence::Present => "present",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Consumer {
    source: String,
    anchor: String,
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

fn load_manifest() -> Manifest {
    let root = repository_root();
    let bytes = fs::read(repository_path(&root, MANIFEST_PATH)).expect("read conformance manifest");
    serde_json::from_slice(&bytes).expect("parse conformance manifest")
}

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

// ---------------------------------------------------------------------------
// Structural test
// ---------------------------------------------------------------------------

#[test]
fn json_schema_conformance_manifest_is_structural_and_consumed() {
    let root = repository_root();
    let manifest = load_manifest();

    assert_eq!(
        manifest.version, 3,
        "unsupported conformance manifest version"
    );
    assert!(!manifest.cases.is_empty(), "manifest must contain cases");

    let target_names = TARGETS
        .into_iter()
        .map(Target::name)
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

// ---------------------------------------------------------------------------
// Executable driver
// ---------------------------------------------------------------------------

/// A probe id, plus everything needed to judge its verdicts.
struct Expectation {
    probe_id: String,
    description: String,
    outcome: &'static str,
    expected_paths: Vec<String>,
    /// For a round trip: the wire every target must re-emit.
    expected_wire: Option<String>,
    /// For a round trip: the input, for the presence-collapse comparison.
    input_wire: Option<String>,
}

struct Prepared {
    plan: PlanCase,
    expectations: Vec<Expectation>,
}

fn case_dir(id: &str) -> String {
    id.replace('-', "_")
}

fn wire_of(root: &Path, value: &AcceptedWireValue) -> String {
    match (&value.fixture, &value.wire_json) {
        (Some(fixture), _) => {
            fs::read_to_string(repository_path(root, fixture)).expect("read wire fixture")
        }
        (_, Some(inline)) => inline.clone(),
        _ => unreachable!("structural test rejects a value with neither"),
    }
}

fn sorted(paths: &[String]) -> Vec<String> {
    let mut out = paths.to_vec();
    out.sort();
    out.dedup();
    out
}

fn prepare(root: &Path, case: &Case) -> Prepared {
    let dir = case_dir(&case.id);
    let model = case.model.clone().expect("accepted case names a model");
    let java_model = if case.schemas.len() > 1 {
        let module = Path::new(&case.schemas[0])
            .file_stem()
            .expect("schema file stem")
            .to_string_lossy();
        format!("{module}.{model}")
    } else {
        model.clone()
    };
    let mut probes = Vec::new();
    let mut expectations = Vec::new();

    for (index, value) in case.accepted_wire_values.iter().enumerate() {
        let probe_id = format!("accept{index}");
        let wire = wire_of(root, value);
        probes.push(Probe {
            id: probe_id.clone(),
            kind: ProbeKind::RoundTrip,
            wire: wire.clone(),
            mutations: value.mutations.clone(),
        });
        expectations.push(Expectation {
            probe_id,
            description: format!(
                "accepted_wire_values[{index}] {}",
                value
                    .fixture
                    .clone()
                    .unwrap_or_else(|| "inline".to_string())
            ),
            outcome: "accepted",
            expected_paths: Vec::new(),
            expected_wire: value.expected_wire.clone(),
            input_wire: Some(wire),
        });
    }

    for (index, failure) in case.parse_failures.iter().enumerate() {
        let probe_id = format!("parse{index}");
        probes.push(Probe {
            id: probe_id.clone(),
            kind: ProbeKind::Parse,
            wire: failure.wire_json.clone(),
            mutations: Vec::new(),
        });
        expectations.push(Expectation {
            probe_id,
            description: format!("parse_failures[{index}]"),
            outcome: "parse_rejected",
            expected_paths: sorted(&failure.expected_paths),
            expected_wire: None,
            input_wire: None,
        });
    }

    for (index, failure) in case.serialize_failures.iter().enumerate() {
        let probe_id = format!("serialize{index}");
        probes.push(Probe {
            id: probe_id.clone(),
            kind: ProbeKind::Serialize,
            wire: match (&failure.from_wire, &failure.from_fixture) {
                (Some(wire), _) => wire.clone(),
                (_, Some(fixture)) => fs::read_to_string(repository_path(root, fixture))
                    .expect("read serialize-failure fixture"),
                _ => unreachable!("structural test rejects a failure with neither"),
            },
            mutations: failure.mutations.clone(),
        });
        expectations.push(Expectation {
            probe_id,
            description: format!("serialize_failures[{index}]"),
            outcome: "serialize_rejected",
            expected_paths: sorted(&failure.expected_paths),
            expected_wire: None,
            input_wire: None,
        });
    }

    Prepared {
        plan: PlanCase {
            id: case.id.clone(),
            dir,
            model,
            java_model,
            probes,
        },
        expectations,
    }
}

/// A presence or value difference between an input wire and a re-emitted one.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Deviation {
    Presence {
        path: String,
        from: Presence,
        to: Presence,
    },
    Value {
        path: String,
        from: String,
        to: String,
    },
}

impl Deviation {
    fn render(&self) -> String {
        match self {
            Deviation::Presence { path, from, to } => {
                format!("{path}: {} -> {}", from.name(), to.name())
            }
            Deviation::Value { path, from, to } => format!("{path}: {from} -> {to}"),
        }
    }
}

fn join(prefix: &str, segment: &str) -> String {
    if prefix.is_empty() {
        segment.to_string()
    } else {
        format!("{prefix}.{segment}")
    }
}

fn diff(path: &str, input: &Value, output: &Value, out: &mut Vec<Deviation>) {
    match (input, output) {
        (Value::Object(left), Value::Object(right)) => {
            let keys: BTreeSet<&String> = left.keys().chain(right.keys()).collect();
            for key in keys {
                let here = join(path, key);
                let (before, after) = (left.get(key), right.get(key));
                let (from, to) = (Presence::of(before), Presence::of(after));
                if from != to {
                    out.push(Deviation::Presence {
                        path: here,
                        from,
                        to,
                    });
                    continue;
                }
                if let (Some(before), Some(after)) = (before, after) {
                    diff(&here, before, after, out);
                }
            }
        }
        (Value::Array(left), Value::Array(right)) if left.len() == right.len() => {
            for (index, (before, after)) in left.iter().zip(right).enumerate() {
                diff(&format!("{path}[{index}]"), before, after, out);
            }
        }
        _ => {
            let (from, to) = (canonical_value(input), canonical_value(output));
            if from != to {
                out.push(Deviation::Value {
                    path: if path.is_empty() {
                        "<root>".to_string()
                    } else {
                        path.to_string()
                    },
                    from,
                    to,
                });
            }
        }
    }
}

/// `blocks[0].page` -> `blocks[].page`, the notation a declaration uses to cover
/// every element of an array.
fn wildcard(path: &str) -> String {
    let mut out = String::new();
    let mut in_index = false;
    for character in path.chars() {
        match character {
            '[' => {
                in_index = true;
                out.push('[');
            }
            ']' => {
                in_index = false;
                out.push(']');
            }
            _ if in_index => {}
            _ => out.push(character),
        }
    }
    out
}

fn collapse_matches(collapse: &PresenceCollapse, target: Target, deviation: &Deviation) -> bool {
    let Deviation::Presence { path, from, to } = deviation else {
        return false;
    };
    collapse.targets.iter().any(|name| name == target.name())
        && (collapse.path == *path || collapse.path == wildcard(path))
        && collapse.wire_presence == *from
        && collapse.serialized_presence == *to
}

fn error_verdicts(plan: &PlanCase, message: &str) -> BTreeMap<String, Verdict> {
    plan.probes
        .iter()
        .map(|probe| {
            (
                probe.id.clone(),
                Verdict {
                    outcome: "error".to_string(),
                    violations: Vec::new(),
                    wire: None,
                    note: None,
                    message: Some(message.to_string()),
                },
            )
        })
        .collect()
}

#[test]
fn json_schema_conformance_cases_agree_across_targets() {
    let root = repository_root();
    let manifest = load_manifest();
    let workspace = Workspace::new("conformance");

    let mut prepared = Vec::new();
    let mut report = String::new();
    let mut hard_failures = Vec::new();
    let mut open_pins: Vec<String> = Vec::new();
    let mut structural_pins: Vec<String> = Vec::new();

    for case in &manifest.cases {
        let schemas = case
            .schemas
            .iter()
            .map(|schema| repository_path(&root, schema))
            .collect::<Vec<_>>();
        if case.expected_load.result == LoadResult::Rejected {
            let diagnostic = case
                .expected_load
                .diagnostic
                .clone()
                .expect("structural test requires a diagnostic");
            for target in toolchain::active_targets() {
                let outcome = workspace.generate_schemas(target, &schemas, &case_dir(&case.id));
                match outcome {
                    Ok(_) => hard_failures.push(format!(
                        "{}: {target} loaded the schema but the manifest declares it rejected",
                        case.id
                    )),
                    Err(error) if !error.contains(&diagnostic) => hard_failures.push(format!(
                        "{}: {target} load diagnostic does not mention {diagnostic:?}:\n{error}",
                        case.id
                    )),
                    Err(_) => {}
                }
            }
            continue;
        }
        prepared.push((case, prepare(&root, case)));
    }

    // Generate every accepted case into every target before running any of
    // them: one build per target amortizes Gradle, `go build` and vitest across
    // the whole manifest.
    let mut generation_failures: BTreeMap<Target, BTreeMap<String, String>> = BTreeMap::new();
    for target in toolchain::active_targets() {
        for (case, item) in &prepared {
            let schemas = case
                .schemas
                .iter()
                .map(|schema| repository_path(&root, schema))
                .collect::<Vec<_>>();
            if let Err(error) = workspace.generate_schemas(target, &schemas, &item.plan.dir) {
                generation_failures
                    .entry(target)
                    .or_default()
                    .insert(case.id.clone(), error);
            }
        }
    }

    let plans: Vec<PlanCase> = prepared
        .iter()
        .filter(|(case, _)| {
            !generation_failures
                .values()
                .any(|failures| failures.contains_key(&case.id))
        })
        .map(|(_, item)| item.plan.clone())
        .collect();

    let mut verdicts: BTreeMap<Target, TargetVerdicts> = BTreeMap::new();
    let mut target_failures: BTreeMap<Target, String> = BTreeMap::new();
    for target in toolchain::active_targets() {
        match toolchain::run_target(&workspace, target, &plans) {
            Ok(result) => {
                verdicts.insert(target, result);
            }
            Err(error) => {
                target_failures.insert(target, error);
            }
        }
    }

    for (case, item) in &prepared {
        let mut findings: Vec<String> = Vec::new();
        for (target, failures) in &generation_failures {
            if let Some(error) = failures.get(&case.id) {
                findings.push(format!("{target}: generation failed: {error}"));
            }
        }

        let mut per_target: BTreeMap<Target, BTreeMap<String, Verdict>> = BTreeMap::new();
        for target in toolchain::active_targets() {
            let entry = match (verdicts.get(&target), target_failures.get(&target)) {
                (Some(result), _) => result.get(&case.id).cloned().unwrap_or_else(|| {
                    error_verdicts(&item.plan, "the runner reported no verdict for this case")
                }),
                (None, Some(error)) => error_verdicts(&item.plan, error),
                (None, None) => error_verdicts(&item.plan, "target did not run"),
            };
            per_target.insert(target, entry);
        }

        // A target whose generated code never compiled has one finding, not one
        // per probe: the build break *is* the divergence.
        per_target.retain(|target, probes| {
            let broken = !probes.is_empty()
                && probes.values().all(|verdict| {
                    verdict
                        .message
                        .as_deref()
                        .is_some_and(|message| message.starts_with(toolchain::BUILD_FAILURE))
                });
            if broken {
                let detail = probes
                    .values()
                    .next()
                    .and_then(|verdict| verdict.message.clone())
                    .unwrap_or_default();
                findings.push(format!("{target}: {detail}"));
            }
            !broken
        });

        for expectation in &item.expectations {
            judge(case, expectation, &per_target, &mut findings);
        }
        check_collapse_declarations(case, item, &per_target, &mut findings);

        // A narrowed run (`NEXGEN_CONFORMANCE_TARGETS`) cannot observe a
        // divergence pinned for a target it skipped, so staleness is only judged
        // over a full run.
        let full_run = toolchain::active_targets().len() == TARGETS.len();
        if findings.is_empty() {
            if !full_run {
                continue;
            }
            if let Some(divergence) = &case.expected_divergence {
                hard_failures.push(format!(
                    "{}: every target now agrees, so its expected_divergence ({}) is stale — \
                     delete it from the manifest",
                    case.id,
                    divergence.findings.join(", ")
                ));
            }
            continue;
        }

        let rendered = format!("\n=== {} ===\n  {}\n", case.id, findings.join("\n  "));
        let Some(divergence) = &case.expected_divergence else {
            hard_failures.push(rendered);
            continue;
        };
        let unexpected: Vec<&String> = findings
            .iter()
            .filter(|finding| {
                !divergence
                    .matches
                    .iter()
                    .any(|pattern| finding.contains(pattern))
            })
            .collect();
        if !unexpected.is_empty() {
            hard_failures.push(format!(
                "\n=== {} ===\n  its expected_divergence ({}) does not cover:\n  {}\n",
                case.id,
                divergence.findings.join(", "),
                unexpected
                    .iter()
                    .map(|finding| finding.as_str())
                    .collect::<Vec<_>>()
                    .join("\n  ")
            ));
        }
        let stale: Vec<&String> = divergence
            .matches
            .iter()
            .filter(|pattern| !findings.iter().any(|finding| finding.contains(*pattern)))
            .collect();
        if !stale.is_empty() && full_run {
            hard_failures.push(format!(
                "{}: expected_divergence matches nothing any more: {stale:?} — \
                 narrow or delete it",
                case.id
            ));
        }
        let _ = write!(
            report,
            "{rendered}  (expected: {} — {})\n",
            divergence.findings.join(", "),
            divergence.note
        );
        let pin = format!("{} pins {}", case.id, divergence.findings.join(", "));
        match divergence.status {
            DivergenceStatus::Open => open_pins.push(pin),
            DivergenceStatus::Structural => structural_pins.push(pin),
        }
    }

    if !report.is_empty() {
        eprintln!("open cross-language divergences (each pinned to a finding):\n{report}");
    }
    if !structural_pins.is_empty() {
        eprintln!(
            "documented target limitations (not defects; kept live so a fix is noticed):\n  {}",
            structural_pins.join("\n  ")
        );
    }
    if !open_pins.is_empty() {
        eprintln!("still open:\n  {}", open_pins.join("\n  "));
    }
    assert!(
        hard_failures.is_empty(),
        "cross-language conformance failed:\n{}",
        hard_failures.join("\n")
    );
}

fn judge(
    case: &Case,
    expectation: &Expectation,
    per_target: &BTreeMap<Target, BTreeMap<String, Verdict>>,
    findings: &mut Vec<String>,
) {
    let mut observed: BTreeMap<Target, &Verdict> = BTreeMap::new();
    for (target, probes) in per_target {
        match probes.get(&expectation.probe_id) {
            Some(verdict) => {
                observed.insert(*target, verdict);
            }
            None => findings.push(format!(
                "{}: {target} reported no verdict",
                expectation.description
            )),
        }
    }

    // Against the manifest.
    for (target, verdict) in &observed {
        if verdict.outcome != expectation.outcome {
            findings.push(format!(
                "{}: {target} {} but the manifest declares {}",
                expectation.description,
                verdict.summary(),
                expectation.outcome
            ));
            continue;
        }
        if !expectation.expected_paths.is_empty() && verdict.paths() != expectation.expected_paths {
            findings.push(format!(
                "{}: {target} rejected at {:?}, manifest declares {:?}",
                expectation.description,
                verdict.paths(),
                expectation.expected_paths
            ));
        }
    }

    // Against each other. P11 frees the reason text across targets, so the
    // contract is the accepted/rejected verdict plus the violation paths.
    let with_wire = expectation.input_wire.is_none();
    let signatures: BTreeMap<Target, String> = observed
        .iter()
        .map(|(target, verdict)| {
            (
                *target,
                if with_wire {
                    verdict.summary()
                } else {
                    verdict.outcome_summary()
                },
            )
        })
        .collect();
    let distinct: BTreeSet<&String> = signatures.values().collect();
    if distinct.len() > 1 {
        let rendered = signatures
            .iter()
            .map(|(target, signature)| format!("{target}={signature}"))
            .collect::<Vec<_>>()
            .join("  |  ");
        findings.push(format!(
            "{}: targets disagree: {rendered}",
            expectation.description
        ));
    }

    // Round-trip fidelity, member by member.
    let Some(input) = &expectation.input_wire else {
        return;
    };
    let base = expectation.expected_wire.as_ref().unwrap_or(input);
    let Ok(expected) = serde_json::from_str::<Value>(base) else {
        findings.push(format!(
            "{}: expected wire is not JSON",
            expectation.description
        ));
        return;
    };
    for (target, verdict) in &observed {
        if verdict.outcome != "accepted" {
            continue;
        }
        let Some(wire) = &verdict.wire else {
            findings.push(format!(
                "{}: {target} accepted the value but could not encode it ({})",
                expectation.description,
                verdict.note.clone().unwrap_or_default()
            ));
            continue;
        };
        let Ok(actual) = serde_json::from_str::<Value>(wire) else {
            findings.push(format!(
                "{}: {target} re-emitted invalid JSON: {wire}",
                expectation.description
            ));
            continue;
        };
        let mut deviations = Vec::new();
        diff("", &expected, &actual, &mut deviations);
        for deviation in deviations {
            if case
                .permitted_presence_nullability_collapse
                .iter()
                .any(|collapse| collapse_matches(collapse, *target, &deviation))
            {
                continue;
            }
            findings.push(format!(
                "{}: {target} did not round-trip {}",
                expectation.description,
                deviation.render()
            ));
        }
    }
}

/// `permitted_presence_nullability_collapse` is a **closed** declaration: every
/// entry must actually happen in every target it names, and nothing else may
/// deviate. A stale entry is as much a defect as a missing one — it hides the
/// day a target stops collapsing.
fn check_collapse_declarations(
    case: &Case,
    item: &Prepared,
    per_target: &BTreeMap<Target, BTreeMap<String, Verdict>>,
    findings: &mut Vec<String>,
) {
    for (index, collapse) in case
        .permitted_presence_nullability_collapse
        .iter()
        .enumerate()
    {
        for name in &collapse.targets {
            let Some(target) = Target::from_name(name) else {
                continue;
            };
            if !toolchain::active_targets().contains(&target) {
                continue;
            }
            let mut seen = false;
            for expectation in &item.expectations {
                let (Some(input), Some(verdict)) = (
                    &expectation.input_wire,
                    per_target
                        .get(&target)
                        .and_then(|probes| probes.get(&expectation.probe_id)),
                ) else {
                    continue;
                };
                let Some(wire) = &verdict.wire else { continue };
                let base = expectation.expected_wire.as_ref().unwrap_or(input);
                let (Ok(expected), Ok(actual)) = (
                    serde_json::from_str::<Value>(base),
                    serde_json::from_str::<Value>(wire),
                ) else {
                    continue;
                };
                let mut deviations = Vec::new();
                diff("", &expected, &actual, &mut deviations);
                seen |= deviations
                    .iter()
                    .any(|deviation| collapse_matches(collapse, target, deviation));
            }
            if !seen {
                findings.push(format!(
                    "permitted collapse[{index}] {} never happens in {target}: \
                     delete the target from the declaration",
                    collapse.path
                ));
            }
        }
    }
}
