//! Execute manifest load, wire, round-trip and mutation checks with real toolchains.

use super::manifest;
use super::toolchain;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use manifest::*;
use serde_json::Value;
use toolchain::{
    PlanCase, Probe, ProbeKind, Target, TargetVerdicts, Verdict, Workspace, canonical_value,
    repository_root,
};

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

pub fn run(target: Option<Target>) {
    let targets = toolchain::selected_targets(target);
    let all_targets = target.is_none();
    let root = repository_root();
    let manifest = load_manifest(&root);
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
            for &target in &targets {
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
    for &target in &targets {
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
    for &target in &targets {
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
        for &target in &targets {
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
        check_collapse_declarations(case, item, &targets, &per_target, &mut findings);

        if findings.is_empty() {
            if all_targets && let Some(divergence) = &case.expected_divergence {
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
        // Patterns can describe another target or a cross-target disagreement.
        // Only the complete run can prove those declarations stale.
        if all_targets && !stale.is_empty() {
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
    targets: &[Target],
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
            if !targets.contains(&target) {
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
