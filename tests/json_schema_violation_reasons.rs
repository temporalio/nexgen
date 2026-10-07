//! `pattern` violation reasons, compared byte-for-byte across the four runtimes.
//!
//! The conformance manifest compares violation *paths* only (P11 leaves reason
//! wording target-idiomatic in general), but `pattern.md` and
//! `propertyNames.md` define one exact `pattern` reason form. This drives the
//! same wire values through generated Go, Java, Python and TypeScript and
//! requires every target to report exactly the specified `(path, reason)` set —
//! including a pattern and values that contain `"`, `\` and control characters,
//! where ad-hoc quoting (Go `%q`, plain wrapping, a bare value) would diverge.

mod toolchain;

use std::collections::BTreeMap;
use std::fs;

use serde_json::json;

use toolchain::{PlanCase, Probe, ProbeKind, TARGETS, Violation, Workspace};

const MODEL: &str = "PatternReasons";
const DIR: &str = "pattern_reasons";

fn violation(path: &str, reason: &str) -> Violation {
    Violation {
        path: path.to_string(),
        reason: reason.to_string(),
    }
}

#[test]
fn pattern_violation_reasons_are_identical_in_every_runtime() {
    let workspace = Workspace::new("pattern-reasons");
    let schemas = workspace.root().join("schemas");
    fs::create_dir_all(&schemas).expect("create schema directory");
    let schema = schemas.join(format!("{MODEL}.json"));
    let document = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "code": { "type": "string", "pattern": "^[a-z]+$" },
            // A pattern containing `"` and `\`: the reason quotes it as a JSON
            // string, so both are backslash-escaped identically everywhere.
            "quoted": { "type": "string", "pattern": "^\"[a-z]\\.\"$" },
            "names": {
                "type": "object",
                "additionalProperties": { "type": "string" },
                "propertyNames": { "type": "string", "pattern": "^[a-z]+$" }
            }
        }
    });
    fs::write(
        &schema,
        serde_json::to_vec_pretty(&document).expect("render schema"),
    )
    .expect("write schema");
    for target in TARGETS {
        workspace
            .generate(target, &schema, DIR)
            .unwrap_or_else(|error| panic!("{target}: generate: {error}"));
    }

    let parse_wire = serde_json::to_string(&json!({
        "code": "AB1",
        "quoted": "x\"\\\t\u{1}",
        "names": { "ok": "x", "Bad \"Key\"": "y" }
    }))
    .expect("render wire");
    let plan = vec![PlanCase {
        id: "pattern-reasons".to_string(),
        dir: DIR.to_string(),
        model: MODEL.to_string(),
        java_model: MODEL.to_string(),
        probes: vec![
            Probe {
                id: "parse".to_string(),
                kind: ProbeKind::Parse,
                wire: parse_wire,
                mutations: Vec::new(),
            },
            Probe {
                id: "serialize".to_string(),
                kind: ProbeKind::Serialize,
                wire: r#"{"code":"ab","quoted":"\"a.\"","names":{"ok":"x"}}"#.to_string(),
                mutations: vec![json!({
                    "path": "names",
                    "put_map_entry": { "key": "BAD", "value": "x" }
                })],
            },
        ],
    }];

    let expected: BTreeMap<&str, Vec<Violation>> = BTreeMap::from([
        (
            "parse",
            vec![
                violation("code", r#"must match pattern "^[a-z]+$", got "AB1""#),
                violation(
                    r#"names["Bad \"Key\""]"#,
                    r#"invalid property name "Bad \"Key\"": must match pattern "^[a-z]+$""#,
                ),
                violation(
                    "quoted",
                    r#"must match pattern "^\"[a-z]\\.\"$", got "x\"\\\t\u0001""#,
                ),
            ],
        ),
        (
            "serialize",
            vec![violation(
                "names.BAD",
                r#"invalid property name "BAD": must match pattern "^[a-z]+$""#,
            )],
        ),
    ]);

    let mut findings = Vec::new();
    for target in TARGETS {
        let verdicts = match toolchain::run_target(&workspace, target, &plan) {
            Ok(verdicts) => verdicts,
            Err(error) => {
                findings.push(format!("{target}: {}", toolchain::brief(&error)));
                continue;
            }
        };
        for (probe, want) in &expected {
            let Some(verdict) = verdicts
                .get("pattern-reasons")
                .and_then(|probes| probes.get(*probe))
            else {
                findings.push(format!("{target} {probe}: no verdict"));
                continue;
            };
            let mut got = verdict.violations.clone();
            got.sort();
            if &got != want {
                findings.push(format!(
                    "{target} {probe} ({}):\n    got  {got:?}\n    want {want:?}",
                    verdict.outcome
                ));
            }
        }
    }
    assert!(
        findings.is_empty(),
        "pattern violation reasons diverge from the specified form:\n  {}",
        findings.join("\n  ")
    );
}
