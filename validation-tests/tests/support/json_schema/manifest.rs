//! Manifest data and parsing shared by structural Rust tests and runtime validation.
//! Each consumer uses a different subset of the manifest fields.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

pub(super) const MANIFEST_PATH: &str = "samples/conformance/json-schema.json";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Manifest {
    pub version: u64,
    pub cases: Vec<Case>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Case {
    pub id: String,
    /// Prose stating what the case pins, for the driver's report.
    pub intent: String,
    pub schemas: Vec<String>,
    /// The generated model, named identically across the four targets.
    pub model: Option<String>,
    pub expected_load: ExpectedLoad,
    #[serde(default)]
    pub accepted_wire_values: Vec<AcceptedWireValue>,
    #[serde(default)]
    pub parse_failures: Vec<ParseFailure>,
    #[serde(default)]
    pub serialize_failures: Vec<SerializeFailure>,
    #[serde(default)]
    pub permitted_presence_nullability_collapse: Vec<PresenceCollapse>,
    #[serde(default)]
    pub expected_divergence: Option<ExpectedDivergence>,
    #[serde(default)]
    pub consumers: BTreeMap<String, Consumer>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ExpectedDivergence {
    pub findings: Vec<String>,
    pub status: DivergenceStatus,
    pub note: String,
    /// Every observed finding must match a pattern, and every pattern must
    /// match a finding in the complete run; expectations cannot hide new bugs
    /// or outlive their defects.
    pub matches: Vec<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum DivergenceStatus {
    Open,
    Structural,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ExpectedLoad {
    pub result: LoadResult,
    pub diagnostic: Option<String>,
    #[serde(default)]
    pub covers: Vec<String>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum LoadResult {
    Accepted,
    Rejected,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AcceptedWireValue {
    pub fixture: Option<String>,
    pub wire_json: Option<String>,
    /// Canonical wire where it differs from the input.
    pub expected_wire: Option<String>,
    #[serde(default)]
    pub mutations: Vec<Value>,
    pub covers: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ParseFailure {
    pub wire_json: String,
    pub expected_paths: Vec<String>,
    pub covers: Vec<String>,
}

/// Parse a wire, mutate the native value, then assert rejection on serialization.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SerializeFailure {
    pub from_wire: Option<String>,
    pub from_fixture: Option<String>,
    pub mutations: Vec<Value>,
    pub expected_paths: Vec<String>,
    pub covers: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PresenceCollapse {
    pub path: String,
    pub targets: Vec<String>,
    pub wire_presence: Presence,
    pub serialized_presence: Presence,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum Presence {
    Absent,
    ExplicitNull,
    Present,
}

impl Presence {
    pub fn of(value: Option<&Value>) -> Self {
        match value {
            None => Presence::Absent,
            Some(Value::Null) => Presence::ExplicitNull,
            Some(_) => Presence::Present,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Presence::Absent => "absent",
            Presence::ExplicitNull => "explicit_null",
            Presence::Present => "present",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Consumer {
    pub source: String,
    pub anchor: String,
}

pub(super) fn repository_path(root: &Path, relative: &str) -> PathBuf {
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

pub(super) fn load_manifest(root: &Path) -> Manifest {
    let bytes = fs::read(repository_path(root, MANIFEST_PATH)).expect("read conformance manifest");
    serde_json::from_slice(&bytes).expect("parse conformance manifest")
}
