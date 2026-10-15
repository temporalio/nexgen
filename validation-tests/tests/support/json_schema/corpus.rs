//! The pinned corpora, run through the four generated runtimes.
//!
//! `specs/json-schema/corpora/` is where this project writes down what a
//! `pattern` matches and what a `format` accepts. Until now those files were
//! read only by the Rust gate (`src/json_schema/pattern.rs`,
//! `src/json_schema/format.rs`) — so `format.md`'s claim that every rule was
//! "verified value-for-value across all four runtime targets" was unproven, and
//! the `format_materialize_clock` rows (nanosecond precision, `-00:00`, trailing
//! fractional zeros) were never round-tripped in any language at all.
//!
//! This file turns each corpus into a generated model with one member per rule
//! and pushes every row through Go, Java, Python and TypeScript:
//!
//! * `pattern_conformance` — each pair's `expect_match` is the verdict all four
//!   must produce. Pairs flagged `expect_gate_reject` are the gate's business,
//!   not a runtime's, and are skipped.
//! * The format corpora — each row's expected validity is the verdict all four
//!   base targets and both additional TypeScript temporal profiles must produce.
//! * `format_materialize_clock` and the duration/URI-reference corpora — every
//!   accepted row is round-tripped against its declared common canonical wire,
//!   with explicit overrides only for legacy TypeScript `Date` capacity loss.

use super::toolchain;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

use nexgen::generator::TsDateTimeTypes;
use serde_json::{Value, json};

use toolchain::{
    PlanCase, Probe, ProbeKind, Target, TargetVerdicts, Verdict, Workspace, repository_root,
};

fn corpus(name: &str) -> Value {
    let path = repository_root()
        .join("specs/json-schema/corpora")
        .join(name)
        .join("corpus.json");
    serde_json::from_slice(&fs::read(&path).unwrap_or_else(|error| {
        panic!("read corpus {}: {error}", path.display());
    }))
    .unwrap_or_else(|error| panic!("parse corpus {}: {error}", path.display()))
}

fn rows<'a>(value: &'a Value, key: &str) -> &'a Vec<Value> {
    value
        .get(key)
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("corpus has no {key} array"))
}

fn text<'a>(row: &'a Value, key: &str) -> &'a str {
    row.get(key)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("corpus row {row} has no string {key}"))
}

/// One member of a corpus model, plus the rows it judges.
#[derive(Clone)]
struct Member {
    name: String,
    schema: Value,
}

/// A single row: which member it goes to, the wire string, and the verdict every
/// target must produce.
#[derive(Clone)]
struct Row {
    id: String,
    member: String,
    instance: String,
    expected: Expectation,
    /// Canonical field wire for a round-trip row.
    expected_wire: Option<String>,
    /// Intentional legacy `Date` loss, where it differs from the common wire.
    typescript_date_wire: Option<String>,
}

#[derive(Clone, Copy)]
enum Expectation {
    Accepted,
    Rejected,
    /// No declared verdict — the targets must simply agree, and must agree on
    /// the wire they re-emit.
    Agree,
}

fn write_schema(workspace: &Workspace, model: &str, members: &[Member]) -> PathBuf {
    let mut properties = serde_json::Map::new();
    for member in members {
        properties.insert(member.name.clone(), member.schema.clone());
    }
    let document = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": model,
        "description": "Generated from a pinned conformance corpus; one member per rule.",
        "type": "object",
        "additionalProperties": false,
        "properties": properties,
    });
    let schemas = workspace.root().join("schemas");
    fs::create_dir_all(&schemas).expect("create schema directory");
    let path = schemas.join(format!("{model}.json"));
    fs::write(
        &path,
        serde_json::to_vec_pretty(&document).expect("render schema"),
    )
    .expect("write corpus schema");
    path
}

/// Generate the corpus model, dropping any member the loader refuses.
///
/// The `pattern` gate tightens over time and the corpus flags only the rows its
/// author knew about, so a member the gate now rejects is removed and reported
/// rather than failing the whole corpus.
fn generate_corpus(
    workspace: &Workspace,
    targets: &[Target],
    model: &str,
    dir: &str,
    mut members: Vec<Member>,
) -> (Vec<Member>, Vec<String>) {
    let mut dropped = Vec::new();
    loop {
        let schema = write_schema(workspace, model, &members);
        let mut failure = None;
        for &target in targets {
            if let Err(error) = workspace.generate(target, &schema, dir) {
                failure = Some(error);
                break;
            }
        }
        let Some(error) = failure else {
            return (members, dropped);
        };
        let offending = members
            .iter()
            .map(|member| member.name.clone())
            .find(|name| error.contains(&format!("properties.{name}:")))
            .unwrap_or_else(|| {
                panic!("{model}: the loader rejected the corpus for no single member:\n{error}")
            });
        dropped.push(format!("{offending}: {}", toolchain::brief(&error)));
        members.retain(|member| member.name != offending);
        assert!(
            !members.is_empty(),
            "{model}: the loader rejected every member"
        );
    }
}

fn run_corpus(
    workspace: &Workspace,
    targets: &[Target],
    case_id: &str,
    model: &str,
    dir: &str,
    members: Vec<Member>,
    corpus_rows: Vec<Row>,
) -> Vec<String> {
    let (kept, dropped) = generate_corpus(workspace, targets, model, dir, members);
    let live: BTreeSet<String> = kept.iter().map(|member| member.name.clone()).collect();
    let corpus_rows: Vec<Row> = corpus_rows
        .into_iter()
        .filter(|row| live.contains(&row.member))
        .collect();

    let probes: Vec<Probe> = corpus_rows
        .iter()
        .enumerate()
        .map(|(index, row)| Probe {
            id: format!("row{index}"),
            kind: match row.expected {
                Expectation::Agree => ProbeKind::RoundTrip,
                _ => ProbeKind::Parse,
            },
            wire: serde_json::to_string(&json!({ &row.member: &row.instance }))
                .expect("render probe wire"),
            mutations: Vec::new(),
        })
        .collect();

    let plan = vec![PlanCase {
        id: case_id.to_string(),
        dir: dir.to_string(),
        model: model.to_string(),
        java_model: model.to_string(),
        probes,
    }];

    let mut verdicts: BTreeMap<Target, TargetVerdicts> = BTreeMap::new();
    let mut findings: Vec<String> = dropped
        .into_iter()
        .map(|note| format!("member dropped at load — {note}"))
        .collect();
    for &target in targets {
        match toolchain::run_target(workspace, target, &plan) {
            Ok(result) => {
                verdicts.insert(target, result);
            }
            Err(error) => findings.push(format!("{target}: {}", toolchain::brief(&error))),
        }
    }

    for (index, row) in corpus_rows.iter().enumerate() {
        let probe_id = format!("row{index}");
        let mut observed: BTreeMap<Target, &Verdict> = BTreeMap::new();
        for (target, result) in &verdicts {
            if let Some(verdict) = result.get(case_id).and_then(|probes| probes.get(&probe_id)) {
                observed.insert(*target, verdict);
            }
        }
        if observed.len() < verdicts.len() {
            findings.push(format!("{}: not every target reported a verdict", row.id));
            continue;
        }

        for (target, verdict) in &observed {
            let wrong = match row.expected {
                Expectation::Accepted | Expectation::Agree => verdict.outcome != "accepted",
                Expectation::Rejected => verdict.outcome != "parse_rejected",
            };
            if wrong {
                findings.push(format!(
                    "{} ({}={:?}): {target} {}, corpus says {}",
                    row.id,
                    row.member,
                    row.instance,
                    verdict.summary(),
                    match row.expected {
                        Expectation::Rejected => "reject",
                        _ => "accept",
                    }
                ));
            }
            if matches!(row.expected, Expectation::Agree)
                && verdict.outcome == "accepted"
                && let (Some(expected), Some(actual)) = (&row.expected_wire, &verdict.wire)
            {
                let expected = serde_json::to_string(&json!({ &row.member: expected }))
                    .expect("render expected corpus wire");
                if toolchain::canonical_json(actual) != toolchain::canonical_json(&expected) {
                    findings.push(format!(
                        "{} ({}={:?}): {target} re-emitted {}, expected {}",
                        row.id,
                        row.member,
                        row.instance,
                        toolchain::canonical_json(actual),
                        toolchain::canonical_json(&expected)
                    ));
                }
            }
        }

        let signatures: BTreeMap<Target, String> = observed
            .iter()
            .map(|(target, verdict)| {
                (
                    *target,
                    match row.expected {
                        // The wire matters only where the corpus declares no
                        // verdict: there, agreeing on the re-emitted string is
                        // the whole contract.
                        Expectation::Agree => verdict.summary(),
                        _ => verdict.outcome_summary(),
                    },
                )
            })
            .collect();
        if signatures.values().collect::<BTreeSet<_>>().len() > 1 {
            findings.push(format!(
                "{} ({}={:?}): targets disagree: {}",
                row.id,
                row.member,
                row.instance,
                signatures
                    .iter()
                    .map(|(target, signature)| format!("{target}={signature}"))
                    .collect::<Vec<_>>()
                    .join("  |  ")
            ));
        }
    }
    findings
}

fn run_typescript_profile(
    profile: &str,
    repr: TsDateTimeTypes,
    model: &str,
    dir: &str,
    members: &[Member],
    corpus_rows: &[Row],
) -> Vec<String> {
    let workspace = Workspace::new(profile);
    let schema = write_schema(&workspace, model, members);
    if let Err(error) =
        workspace.generate_with_typescript_profile(Target::TypeScript, &[schema], dir, repr)
    {
        return vec![format!(
            "{profile}: generation failed: {}",
            toolchain::brief(&error)
        )];
    }
    let probes = corpus_rows
        .iter()
        .enumerate()
        .map(|(index, row)| Probe {
            id: format!("row{index}"),
            kind: match row.expected {
                Expectation::Agree => ProbeKind::RoundTrip,
                _ => ProbeKind::Parse,
            },
            wire: serde_json::to_string(&json!({ &row.member: &row.instance }))
                .expect("render profile wire"),
            mutations: Vec::new(),
        })
        .collect();
    let plan = vec![PlanCase {
        id: profile.to_string(),
        dir: dir.to_string(),
        model: model.to_string(),
        java_model: model.to_string(),
        probes,
    }];
    let verdicts = match toolchain::run_target(&workspace, Target::TypeScript, &plan) {
        Ok(verdicts) => verdicts,
        Err(error) => return vec![format!("{profile}: {}", toolchain::brief(&error))],
    };
    let mut findings = Vec::new();
    for (index, row) in corpus_rows.iter().enumerate() {
        let Some(verdict) = verdicts
            .get(profile)
            .and_then(|case| case.get(&format!("row{index}")))
        else {
            findings.push(format!("{}: {profile} reported no verdict", row.id));
            continue;
        };
        let accepted = matches!(row.expected, Expectation::Accepted | Expectation::Agree);
        let wanted = if accepted {
            "accepted"
        } else {
            "parse_rejected"
        };
        if verdict.outcome != wanted {
            findings.push(format!(
                "{} ({}={:?}): {profile} {}, corpus says {wanted}",
                row.id,
                row.member,
                row.instance,
                verdict.summary()
            ));
            continue;
        }
        if matches!(row.expected, Expectation::Agree) {
            let expected_field = if profile == "typescript-date" {
                row.typescript_date_wire
                    .as_ref()
                    .or(row.expected_wire.as_ref())
            } else {
                row.expected_wire.as_ref()
            };
            if let (Some(expected_field), Some(actual)) = (expected_field, &verdict.wire) {
                let expected = serde_json::to_string(&json!({ &row.member: expected_field }))
                    .expect("render profile expected wire");
                if toolchain::canonical_json(actual) != toolchain::canonical_json(&expected) {
                    findings.push(format!(
                        "{} ({}={:?}): {profile} re-emitted {}, expected {}",
                        row.id,
                        row.member,
                        row.instance,
                        toolchain::canonical_json(actual),
                        toolchain::canonical_json(&expected)
                    ));
                }
            }
        }
    }
    findings
}

pub fn run(target: Option<Target>) {
    pattern_corpus_matches_identically_in_every_runtime(target);
    format_corpora_hold_in_every_runtime(target);
}

pub fn pattern_corpus_matches_identically_in_every_runtime(target: Option<Target>) {
    let targets = toolchain::selected_targets(target);
    let document = corpus("pattern_conformance");
    let mut members = Vec::new();
    let mut index_of: BTreeMap<String, String> = BTreeMap::new();
    let mut corpus_rows = Vec::new();

    for row in rows(&document, "pairs") {
        if row
            .get("expect_gate_reject")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            continue;
        }
        let pattern = text(row, "pattern");
        let expected = row
            .get("expect_match")
            .and_then(Value::as_bool)
            .unwrap_or_else(|| {
                panic!(
                    "pattern pair {} declares neither expect_match nor expect_gate_reject",
                    text(row, "id")
                )
            });
        let name = index_of.entry(pattern.to_string()).or_insert_with(|| {
            let name = format!("p{}", members.len());
            members.push(Member {
                name: name.clone(),
                schema: json!({ "type": "string", "pattern": pattern }),
            });
            name
        });
        corpus_rows.push(Row {
            id: text(row, "id").to_string(),
            member: name.clone(),
            instance: text(row, "instance").to_string(),
            expected: if expected {
                Expectation::Accepted
            } else {
                Expectation::Rejected
            },
            expected_wire: None,
            typescript_date_wire: None,
        });
    }

    let workspace = Workspace::new("pattern-corpus");
    let findings = run_corpus(
        &workspace,
        &targets,
        "pattern-conformance",
        "PatternCorpus",
        "pattern_corpus",
        members,
        corpus_rows,
    );
    let (unexpected, open, structural) = triage(findings, &[], &targets);
    report_open(&open, &structural);
    assert!(
        unexpected.is_empty(),
        "the pattern corpus does not hold across the four runtimes ({} rows):\n  {}",
        unexpected.len(),
        unexpected.join("\n  ")
    );
}

pub fn format_corpora_hold_in_every_runtime(target: Option<Target>) {
    let targets = toolchain::selected_targets(target);
    // One member per asserted format, plus the three materializing clock
    // formats, so every corpus row lands on the generated check for its rule.
    let members = vec![
        ("uuid", "uuid"),
        ("ipv4", "ipv4"),
        ("ipv6", "ipv6"),
        ("date", "date"),
        ("time", "time"),
        ("dateTime", "date-time"),
        ("email", "email"),
        ("hostname", "hostname"),
        ("uri", "uri"),
        ("uriReference", "uri-reference"),
        ("duration", "duration"),
    ]
    .into_iter()
    .map(|(name, format)| Member {
        name: name.to_string(),
        schema: json!({ "type": "string", "format": format }),
    })
    .collect::<Vec<_>>();

    let member_for = |format: &str| match format {
        "date-time" => "dateTime".to_string(),
        other => other.to_string(),
    };

    let mut corpus_rows = Vec::new();
    for row in rows(&corpus("format_conformance"), "pairs") {
        corpus_rows.push(Row {
            id: format!("format_conformance/{}", text(row, "id")),
            member: member_for(text(row, "format")),
            instance: text(row, "value").to_string(),
            expected: expectation(row, "expect_valid"),
            expected_wire: None,
            typescript_date_wire: None,
        });
    }
    for row in rows(&corpus("format_email"), "pairs") {
        corpus_rows.push(Row {
            id: format!("format_email/{}", text(row, "id")),
            member: "email".to_string(),
            instance: text(row, "instance").to_string(),
            expected: expectation(row, "expect_valid"),
            expected_wire: None,
            typescript_date_wire: None,
        });
    }
    for row in rows(&corpus("format_hostname"), "cases") {
        corpus_rows.push(Row {
            id: format!("format_hostname/{}", text(row, "id")),
            member: "hostname".to_string(),
            instance: text(row, "instance").to_string(),
            expected: expectation(row, "valid"),
            expected_wire: None,
            typescript_date_wire: None,
        });
    }
    for row in rows(&corpus("format_uri"), "pairs") {
        corpus_rows.push(Row {
            id: format!("format_uri/{}", text(row, "id")),
            member: "uri".to_string(),
            instance: text(row, "value").to_string(),
            expected: expectation(row, "expect"),
            expected_wire: None,
            typescript_date_wire: None,
        });
    }

    for row in rows(&corpus("format_uri_reference"), "cases") {
        let valid = expectation(row, "expect_valid");
        corpus_rows.push(Row {
            id: format!("format_uri_reference/{}", text(row, "id")),
            member: "uriReference".to_string(),
            instance: text(row, "wire").to_string(),
            expected_wire: matches!(valid, Expectation::Accepted)
                .then(|| text(row, "wire").to_string()),
            expected: match valid {
                Expectation::Accepted => Expectation::Agree,
                other => other,
            },
            typescript_date_wire: None,
        });
    }

    let clock = corpus("format_materialize_clock");
    for (key, member) in [
        ("date-time", "dateTime"),
        ("date", "date"),
        ("time", "time"),
    ] {
        for row in rows(&clock, key) {
            corpus_rows.push(Row {
                id: format!("format_materialize_clock/{}", text(row, "id")),
                member: member.to_string(),
                instance: text(row, "wire").to_string(),
                // The corpus is a list of wires that are VALID, so an absent
                // `expect_valid` means "accepted, and the four re-emitted
                // strings must agree". An explicit `false` marks a wire the
                // shipped grammar rejects, where agreement on the rejection is
                // the contract and there is no wire to compare.
                expected: match row.get("expect_valid").and_then(Value::as_bool) {
                    Some(false) => Expectation::Rejected,
                    _ => Expectation::Agree,
                },
                expected_wire: (row.get("expect_valid").and_then(Value::as_bool) != Some(false))
                    .then(|| text(row, "expected_wire").to_string()),
                typescript_date_wire: row
                    .get("typescript_date_wire")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            });
        }
    }

    for row in rows(&corpus("format_duration"), "cases") {
        let valid = expectation(row, "expect_valid");
        corpus_rows.push(Row {
            id: format!("format_duration/{}", text(row, "id")),
            member: "duration".to_string(),
            instance: text(row, "wire").to_string(),
            expected_wire: matches!(valid, Expectation::Accepted)
                .then(|| text(row, "expected_wire").to_string()),
            expected: match valid {
                Expectation::Accepted => Expectation::Agree,
                other => other,
            },
            typescript_date_wire: None,
        });
    }

    let mut profile_findings = Vec::new();
    if targets.contains(&Target::TypeScript) {
        for (profile, repr) in [
            ("typescript-date", TsDateTimeTypes::Date),
            ("typescript-temporal", TsDateTimeTypes::Temporal),
        ] {
            profile_findings.extend(run_typescript_profile(
                profile,
                repr,
                "FormatCorpus",
                "format_corpus",
                &members,
                &corpus_rows,
            ));
        }
    }
    let workspace = Workspace::new("format-corpus");
    let mut findings = run_corpus(
        &workspace,
        &targets,
        "format-conformance",
        "FormatCorpus",
        "format_corpus",
        members,
        corpus_rows,
    );
    findings.extend(profile_findings);
    let (unexpected, open, structural) = triage(findings, OPEN_FORMAT_ROWS, &targets);
    report_open(&open, &structural);
    assert!(
        unexpected.is_empty(),
        "the format corpora do not hold across the four runtimes ({} rows):\n  {}",
        unexpected.len(),
        unexpected.join("\n  ")
    );
}

/// A corpus row that does not hold yet, and the finding that owns it.
///
/// Same contract as the conformance manifest's `expected_divergence`: the entry
/// must match at least one observed finding, so it cannot outlive the defect.
struct OpenRow {
    matches: &'static str,
    /// Targets whose execution can prove this entry stale.
    targets: &'static [Target],
    finding: &'static str,
    status: RowStatus,
    note: &'static str,
}

/// Why a row is allowed to diverge — the same distinction the conformance
/// manifest draws. Either way the entry stays live and fails the day the row
/// starts holding.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RowStatus {
    /// A defect, or a gap in unimplemented spec surface. Expected to close.
    Open,
    /// A permanent limitation of a target. Documented, not fixed.
    #[allow(dead_code)]
    Structural,
}

static OPEN_FORMAT_ROWS: &[OpenRow] = &[
    OpenRow {
        matches: "format_materialize_clock/dt-frac-9 (",
        targets: &[Target::Python],
        finding: "09#8",
        status: RowStatus::Open,
        note: "Python truncates nanoseconds to microseconds, so it re-emits a different wire \
               than the other three. P1 exception (b) conditions this on a recoverable \
               opt-out that does not exist yet.",
    },
    OpenRow {
        matches: "format_materialize_clock/dt-frac-9-offset (",
        targets: &[Target::Python],
        finding: "09#8",
        status: RowStatus::Open,
        note: "As above, with an offset.",
    },
];

/// Split findings into "must fail the build", "known open" and "documented
/// target limitation".
fn triage(
    findings: Vec<String>,
    open_rows: &[OpenRow],
    targets: &[Target],
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut unexpected = Vec::new();
    let mut open = Vec::new();
    let mut structural = Vec::new();
    for finding in findings {
        match open_rows.iter().find(|row| finding.contains(row.matches)) {
            Some(row) => {
                let rendered = format!("{finding}\n    (expected: {} — {})", row.finding, row.note);
                match row.status {
                    RowStatus::Open => open.push(rendered),
                    RowStatus::Structural => structural.push(rendered),
                }
            }
            None => unexpected.push(finding),
        }
    }
    for row in open_rows {
        if !row.targets.iter().all(|target| targets.contains(target)) {
            continue;
        }
        let seen = open
            .iter()
            .chain(structural.iter())
            .any(|finding| finding.contains(row.matches));
        if !seen {
            unexpected.push(format!(
                "the open-row entry {:?} ({}) matches nothing any more — delete it",
                row.matches, row.finding
            ));
        }
    }
    (unexpected, open, structural)
}

fn report_open(open: &[String], structural: &[String]) {
    if !structural.is_empty() {
        eprintln!(
            "documented target limitations (not defects; kept live so a fix is noticed):\n  {}",
            structural.join("\n  ")
        );
    }
    if !open.is_empty() {
        eprintln!("still open:\n  {}", open.join("\n  "));
    }
}

fn expectation(row: &Value, key: &str) -> Expectation {
    match row.get(key).and_then(Value::as_bool) {
        Some(true) => Expectation::Accepted,
        Some(false) => Expectation::Rejected,
        None => panic!("corpus row {row} has no boolean {key}"),
    }
}
