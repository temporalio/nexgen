//! The cross-language toolchain harness shared by the conformance driver, the
//! probe matrix and the corpus runners.
//!
//! Everything here exists to make one claim checkable: *the four generated
//! backends accept and reject the same wire values, and re-emit the same wire.*
//! Reading generated source cannot establish that, so this module generates into
//! a scratch workspace and then runs the **real** toolchain of every target —
//! `go build`, `tsc`/vitest, CPython, `javac`/`java` — over **unformatted**
//! generator output.
//!
//! Layout of a workspace (`$TMPDIR/nexgen-conformance-*/`):
//!
//! ```text
//! go/          go.mod (copied from samples/go so module deps resolve), one package per case
//! python/      one package per case
//! typescript/  node_modules -> samples/typescript/node_modules, one directory per case
//! java/src/conformance/<case>/  one package per case
//! ```
//!
//! Nothing is generated into the committed samples: they are golden snapshots
//! owned by the regeneration pass, and several agents edit the emitters
//! concurrently.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;

use nexgen::generator::TsDateTimeTypes;
use nexgen::language::Language;
use nexgen::{GenerateRequest, generate_to_file};
use serde::Deserialize;
use serde_json::{Map, Value, json};

/// The four JSON-Schema targets, in the order reports list them.
pub const TARGETS: [Target; 4] = [Target::Go, Target::Java, Target::Python, Target::TypeScript];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Target {
    Go,
    Java,
    Python,
    TypeScript,
}

impl Target {
    pub fn name(self) -> &'static str {
        match self {
            Target::Go => "go",
            Target::Java => "java",
            Target::Python => "python",
            Target::TypeScript => "typescript",
        }
    }

    pub fn language(self) -> Language {
        match self {
            Target::Go => Language::Go,
            Target::Java => Language::Java,
            Target::Python => Language::Python,
            Target::TypeScript => Language::TypeScript,
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        TARGETS.into_iter().find(|target| target.name() == name)
    }
}

impl std::fmt::Display for Target {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.name())
    }
}

pub fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// A scratch tree the harness generates into and runs toolchains in.
///
/// Kept on disk when `NEXGEN_CONFORMANCE_KEEP=1`, which is the difference
/// between "a target disagreed" and "a target disagreed and here is the code".
pub struct Workspace {
    root: PathBuf,
    keep: bool,
}

impl Workspace {
    pub fn new(tag: &str) -> Self {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock before the epoch")
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("nexgen-{tag}-{unique}-{}", std::process::id()));
        fs::create_dir_all(&root).expect("create workspace");
        Self {
            root,
            keep: std::env::var_os("NEXGEN_CONFORMANCE_KEEP").is_some(),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn target_root(&self, target: Target) -> PathBuf {
        self.root.join(target.name())
    }

    /// Generate `schema` into this workspace as `dir`, unformatted.
    ///
    /// Unformatted on purpose: `ruff format`/`gofmt`/`prettier` reparse and
    /// rewrite the emitter's output, which is exactly how a nested same-quote
    /// f-string that is a `SyntaxError` below Python 3.12 reached the samples
    /// without any test noticing.
    pub fn generate(&self, target: Target, schema: &Path, dir: &str) -> Result<PathBuf, String> {
        self.generate_schemas(target, &[schema.to_path_buf()], dir)
    }

    /// Generate an ordered schema set into one target package. The input order
    /// is intentionally retained because it is part of the public generation
    /// request and cross-file `$ref` conformance needs to exercise the real
    /// multi-input path.
    pub fn generate_schemas(
        &self,
        target: Target,
        schemas: &[PathBuf],
        dir: &str,
    ) -> Result<PathBuf, String> {
        self.generate_with_typescript_profile(target, schemas, dir, TsDateTimeTypes::default())
    }

    /// Generate with an explicit TypeScript temporal representation. Other
    /// targets ignore this option, matching the production request contract.
    pub fn generate_with_typescript_profile(
        &self,
        target: Target,
        schemas: &[PathBuf],
        dir: &str,
        ts_date_time_types: TsDateTimeTypes,
    ) -> Result<PathBuf, String> {
        let output_path = self.package_path(target, dir);
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        generate_to_file(&GenerateRequest {
            config: Default::default(),
            language: target.language(),
            input_paths: schemas.to_vec(),
            support_package: match target {
                Target::Go => Some("samples/go/support".into()),
                Target::Python => Some("conformance_support".into()),
                Target::TypeScript => Some("./support".into()),
                Target::Java => None,
            },
            descriptor_paths: Vec::new(),
            output_path: output_path.clone(),
            format: false,
            java_package_name: (target == Target::Java).then(|| format!("conformance.{dir}")),
            ts_date_time_types,
        })
        .map_err(|error| error.to_string())?;
        Ok(output_path)
    }

    pub fn package_path(&self, target: Target, dir: &str) -> PathBuf {
        match target {
            Target::Java => self.target_root(target).join("src/conformance").join(dir),
            _ => self.target_root(target).join(dir),
        }
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        if self.keep {
            eprintln!("kept conformance workspace at {}", self.root.display());
            return;
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}

// ---------------------------------------------------------------------------
// Probe plan / verdict protocol shared with the four runners.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Probe {
    pub id: String,
    pub kind: ProbeKind,
    pub wire: String,
    pub mutations: Vec<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeKind {
    /// Deserialize only.
    Parse,
    /// Deserialize, then serialize, reporting the re-emitted wire.
    RoundTrip,
    /// Deserialize, mutate the native value, then serialize.
    Serialize,
}

impl ProbeKind {
    fn name(self) -> &'static str {
        match self {
            ProbeKind::Parse => "parse",
            ProbeKind::RoundTrip => "round_trip",
            ProbeKind::Serialize => "serialize",
        }
    }
}

#[derive(Debug, Clone)]
pub struct PlanCase {
    pub id: String,
    pub dir: String,
    pub model: String,
    /// Fully qualified below the case package for Java's per-input modules.
    /// Other runners use `model` from the generated root barrel/package.
    pub java_model: String,
    pub probes: Vec<Probe>,
}

impl PlanCase {
    fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "dir": self.dir,
            "model": self.model,
            "java_model": self.java_model,
            "probes": self.probes.iter().map(|probe| json!({
                "id": probe.id,
                "kind": probe.kind.name(),
                "wire": probe.wire,
                "mutations": probe.mutations,
            })).collect::<Vec<_>>(),
        })
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct Violation {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Verdict {
    pub outcome: String,
    #[serde(default)]
    pub violations: Vec<Violation>,
    #[serde(default)]
    pub wire: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
}

impl Verdict {
    pub fn paths(&self) -> Vec<String> {
        let mut paths: Vec<String> = self
            .violations
            .iter()
            .map(|violation| violation.path.clone())
            .collect();
        paths.sort();
        paths.dedup();
        paths
    }

    /// A one-line rendering used in assertion messages and, for probes with no
    /// permitted per-target deviation, as the cross-target signature.
    pub fn summary(&self) -> String {
        self.render(true)
    }

    /// The verdict without the re-emitted wire. Round-trip probes compare their
    /// wire against the *input* instead, member by member, because the manifest
    /// may permit one target to deviate where another may not.
    pub fn outcome_summary(&self) -> String {
        self.render(false)
    }

    fn render(&self, with_wire: bool) -> String {
        match self.outcome.as_str() {
            "accepted" => match (&self.wire, &self.note) {
                (Some(wire), _) if with_wire => format!("accepted {}", canonical_json(wire)),
                (Some(_), _) => "accepted".to_string(),
                (None, Some(note)) => format!("accepted (unencodable: {note})"),
                (None, None) => "accepted".to_string(),
            },
            "error" => format!(
                "error {}",
                brief(self.message.as_deref().unwrap_or_default())
            ),
            other => format!("{other} {:?}", self.paths()),
        }
    }
}

/// The first meaningful line of a toolchain diagnostic.
///
/// A `javac` or `go build` failure is a page of context; repeated once per probe
/// it buries the divergence report it is meant to explain.
pub fn brief(message: &str) -> String {
    let line = message
        .lines()
        .map(str::trim)
        .find(|line| line.contains("error") || line.contains("Error"))
        .or_else(|| message.lines().map(str::trim).find(|line| !line.is_empty()))
        .unwrap_or("");
    if line.chars().count() > 200 {
        format!("{}...", line.chars().take(200).collect::<String>())
    } else {
        line.to_string()
    }
}

/// The message prefix marking a case whose generated code never compiled.
pub const BUILD_FAILURE: &str = "generated code does not build";

/// Every target's verdicts, keyed by case id then probe id.
pub type TargetVerdicts = BTreeMap<String, BTreeMap<String, Verdict>>;

// ---------------------------------------------------------------------------
// JSON canonicalization.
// ---------------------------------------------------------------------------

/// A stable rendering of a JSON document for cross-language comparison.
///
/// Object members are key-sorted (no target promises member order) and numbers
/// are reduced to their mathematical value, which is what P1 makes
/// identity-bearing — but `-0.0` stays distinct from `0.0`, because
/// `uniqueItems` and `const` turn on exactly that distinction.
pub fn canonical_json(text: &str) -> String {
    match serde_json::from_str::<Value>(text) {
        Ok(value) => canonical_value(&value),
        Err(error) => format!("<invalid JSON: {error}: {text}>"),
    }
}

pub fn canonical_value(value: &Value) -> String {
    let mut out = String::new();
    write_canonical(value, &mut out);
    out
}

fn write_canonical(value: &Value, out: &mut String) {
    match value {
        Value::Object(members) => {
            let sorted: BTreeMap<&String, &Value> = members.iter().collect();
            out.push('{');
            for (index, (key, member)) in sorted.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                let _ = write!(out, "{}:", Value::String((*key).clone()));
                write_canonical(member, out);
            }
            out.push('}');
        }
        Value::Array(elements) => {
            out.push('[');
            for (index, element) in elements.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_canonical(element, out);
            }
            out.push(']');
        }
        Value::Number(number) => out.push_str(&canonical_number(number)),
        other => {
            let _ = write!(out, "{other}");
        }
    }
}

fn canonical_number(number: &serde_json::Number) -> String {
    let Some(float) = number.as_f64() else {
        return number.to_string();
    };
    if float == 0.0 {
        return if float.is_sign_negative() { "-0" } else { "0" }.to_string();
    }
    if float.fract() == 0.0 && float.abs() < 1e17 {
        return format!("{}", float as i128);
    }
    format!("{float:?}")
}

/// Split a JSON object into `(key, canonical member)` pairs, for the
/// member-by-member presence comparison the collapse declaration needs.
pub fn object_members(text: &str) -> Option<Map<String, Value>> {
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(members)) => Some(members),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Toolchain invocation.
// ---------------------------------------------------------------------------

fn describe(output: &Output) -> String {
    format!(
        "exit {}\n--- stdout ---\n{}\n--- stderr ---\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

pub struct ToolRun {
    pub ok: bool,
    pub detail: String,
}

impl ToolRun {
    pub fn expect_ok(self, context: &str) -> String {
        assert!(self.ok, "{context} failed:\n{}", self.detail);
        self.detail
    }
}

pub fn run(command: &mut Command) -> ToolRun {
    match command.output() {
        Ok(output) => ToolRun {
            ok: output.status.success(),
            detail: describe(&output),
        },
        Err(error) => ToolRun {
            ok: false,
            detail: format!("could not spawn {command:?}: {error}"),
        },
    }
}

/// A `PATH` that reaches nvm's Node when the system one predates `Temporal`.
///
/// `samples/typescript` needs a Node whose global `Temporal` exists; on macOS
/// the Homebrew Node 26 that shadows it does not have one. CI installs a Node 26
/// that does, so the override is a no-op there.
pub fn tool_path() -> String {
    static PATH: OnceLock<String> = OnceLock::new();
    PATH.get_or_init(|| {
        let existing = std::env::var("PATH").unwrap_or_default();
        let Some(home) = std::env::var_os("HOME") else {
            return existing;
        };
        let versions = PathBuf::from(home).join(".nvm/versions/node");
        let Ok(entries) = fs::read_dir(&versions) else {
            return existing;
        };
        let mut candidates: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path().join("bin"))
            .filter(|bin| bin.join("node").is_file())
            .collect();
        candidates.sort();
        match candidates.pop() {
            Some(bin) => format!("{}:{existing}", bin.display()),
            None => existing,
        }
    })
    .clone()
}

pub fn command(program: &str) -> Command {
    let mut command = Command::new(program);
    command.env("PATH", tool_path());
    command
}

// ---------------------------------------------------------------------------
// Go
// ---------------------------------------------------------------------------

/// Prepare `<workspace>/go` as a module: the sample module's dependency set
/// (nexus-rpc, for schemas that declare a service) resolved from the local
/// module cache, under a neutral module path.
pub fn prepare_go_module(workspace: &Workspace) -> Result<PathBuf, String> {
    let root = workspace.target_root(Target::Go);
    fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    let samples = repository_root().join("samples/go");
    let go_mod = fs::read_to_string(samples.join("go.mod")).map_err(|error| error.to_string())?;
    let rewritten = go_mod.replacen("module samples/go", "module conformance", 1);
    fs::write(root.join("go.mod"), rewritten).map_err(|error| error.to_string())?;
    fs::copy(samples.join("go.sum"), root.join("go.sum")).map_err(|error| error.to_string())?;
    Ok(root)
}

fn go_registry(cases: &[PlanCase]) -> String {
    let mut imports = String::new();
    let mut entries = String::new();
    for case in cases {
        let _ = writeln!(imports, "\t{} \"conformance/{}\"", case.dir, case.dir);
        let _ = writeln!(
            entries,
            "\t{:?}: reflect.TypeOf({}.{}{{}}),",
            case.id, case.dir, case.model
        );
    }
    format!(
        "// Code generated by the nexgen conformance harness. DO NOT EDIT.\n\
         package main\n\n\
         import (\n\t\"reflect\"\n\n{imports})\n\n\
         var registry = map[string]reflect.Type{{\n{entries}}}\n"
    )
}

/// Build the runner over `cases`, and on failure work out which cases are
/// individually buildable.
///
/// Go links a package set, so one uncompilable case would otherwise blind the
/// whole target — and "the generated package does not compile" is itself one of
/// the verdicts this harness exists to report, per case.
fn go_build(root: &Path, cases: &[PlanCase]) -> Result<(), String> {
    fs::write(root.join("registry.go"), go_registry(cases)).map_err(|error| error.to_string())?;
    let build = run(command("go")
        .current_dir(root)
        .args(["build", "-o", "runner", "."]));
    if build.ok { Ok(()) } else { Err(build.detail) }
}

pub fn run_go(workspace: &Workspace, cases: &[PlanCase]) -> Result<TargetVerdicts, String> {
    let root = prepare_go_module(workspace)?;
    fs::write(
        root.join("runner.go"),
        include_str!("../../samples/conformance/runners/runner.go"),
    )
    .map_err(|error| error.to_string())?;

    let mut broken: BTreeMap<String, String> = BTreeMap::new();
    let mut buildable: Vec<PlanCase> = cases.to_vec();
    if let Err(whole) = go_build(&root, &buildable) {
        buildable.clear();
        for case in cases {
            match go_build(&root, std::slice::from_ref(case)) {
                Ok(()) => buildable.push(case.clone()),
                Err(error) => {
                    broken.insert(case.id.clone(), error);
                }
            }
        }
        if broken.is_empty() {
            return Err(format!(
                "go build failed over the whole case set but every case builds alone:\n{whole}"
            ));
        }
        go_build(&root, &buildable).map_err(|error| format!("go build failed:\n{error}"))?;
    }

    let plan_path = root.join("plan.json");
    let result_path = root.join("result.json");
    write_plan(&plan_path, &buildable)?;
    let mut verdicts = if buildable.is_empty() {
        TargetVerdicts::new()
    } else {
        let execution = run(command("./runner")
            .current_dir(&root)
            .arg(&plan_path)
            .arg(&result_path));
        if !execution.ok {
            return Err(format!("go runner failed:\n{}", execution.detail));
        }
        read_verdicts(&result_path)?
    };
    add_build_failures(&mut verdicts, cases, &broken);
    Ok(verdicts)
}

/// Record an unbuildable case as an `error` verdict on each of its probes.
fn add_build_failures(
    verdicts: &mut TargetVerdicts,
    cases: &[PlanCase],
    broken: &BTreeMap<String, String>,
) {
    for case in cases {
        let Some(error) = broken.get(&case.id) else {
            continue;
        };
        verdicts.insert(
            case.id.clone(),
            case.probes
                .iter()
                .map(|probe| {
                    (
                        probe.id.clone(),
                        Verdict {
                            outcome: "error".to_string(),
                            violations: Vec::new(),
                            wire: None,
                            note: None,
                            message: Some(format!("{BUILD_FAILURE}: {}", brief(error))),
                        },
                    )
                })
                .collect(),
        );
    }
}

// ---------------------------------------------------------------------------
// Python
// ---------------------------------------------------------------------------

pub fn python_interpreter() -> PathBuf {
    if let Some(explicit) = std::env::var_os("NEXGEN_CONFORMANCE_PYTHON") {
        return PathBuf::from(explicit);
    }
    let venv = repository_root().join("samples/python/.venv/bin/python");
    if venv.is_file() {
        return venv;
    }
    PathBuf::from("python3")
}

/// Resolve an interpreter with the generated models' runtime dependencies.
///
/// Rust validation runs before the Python sample validation in CI, so the
/// sample virtual environment may be missing or stale. Synchronize it from the
/// lockfile once per test process instead of trusting whichever dependencies an
/// existing environment happens to contain.
pub fn python_runtime_command() -> Result<Command, String> {
    if let Some(explicit) = std::env::var_os("NEXGEN_CONFORMANCE_PYTHON") {
        return Ok(command(&PathBuf::from(explicit).to_string_lossy()));
    }

    let sample_root = repository_root().join("samples/python");
    let venv = sample_root.join(".venv/bin/python");
    static SYNC: OnceLock<Result<(), String>> = OnceLock::new();
    SYNC.get_or_init(|| {
        let sync = run(command("uv")
            .current_dir(&sample_root)
            .args(["sync", "--locked"]));
        if !sync.ok {
            return Err(format!(
                "failed to provision Python conformance dependencies:\n{}",
                sync.detail
            ));
        }
        Ok(())
    })
    .clone()?;
    if !venv.is_file() {
        return Err(format!(
            "uv sync did not create the Python interpreter at {}",
            venv.display()
        ));
    }
    Ok(command(&venv.to_string_lossy()))
}

pub fn run_python(workspace: &Workspace, cases: &[PlanCase]) -> Result<TargetVerdicts, String> {
    let root = workspace.target_root(Target::Python);
    fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    let runner = root.join("runner.py");
    fs::write(
        &runner,
        include_str!("../../samples/conformance/runners/runner.py"),
    )
    .map_err(|error| error.to_string())?;
    let plan_path = root.join("plan.json");
    let result_path = root.join("result.json");
    write_plan(&plan_path, cases)?;
    let execution = run(python_runtime_command()?
        .current_dir(&root)
        .arg(&runner)
        .arg(&plan_path)
        .arg(&result_path)
        .arg(&root));
    if !execution.ok {
        return Err(format!("python runner failed:\n{}", execution.detail));
    }
    read_verdicts(&result_path)
}

// ---------------------------------------------------------------------------
// TypeScript
// ---------------------------------------------------------------------------

/// Link the samples' installed dependencies into a scratch directory so Node's
/// resolver finds `nexus-rpc`/`vitest` without generating into the samples tree.
pub fn link_node_modules(root: &Path) -> Result<(), String> {
    let target = repository_root().join("samples/typescript/node_modules");
    if !target.is_dir() {
        return Err(format!(
            "{} is missing; run `npm ci` in samples/typescript",
            target.display()
        ));
    }
    let link = root.join("node_modules");
    if link.exists() {
        return Ok(());
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(&target, &link).map_err(|error| error.to_string())?;
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(&target, &link).map_err(|error| error.to_string())?;
    Ok(())
}

/// The lazy import table the TypeScript runners consume.
pub fn typescript_registry_of(entries: &[(String, String, String)]) -> String {
    let mut rows = String::new();
    for (id, dir, model) in entries {
        let _ = writeln!(
            rows,
            "  {id:?}: {{ load: () => import(\"./{dir}/index\"), model: {model:?} }},"
        );
    }
    format!(
        "// Code generated by the nexgen conformance harness. DO NOT EDIT.\n\
         export const REGISTRY: Record<string, {{ load: () => Promise<unknown>; model: string }}> = {{\n\
         {rows}}};\n"
    )
}

fn typescript_registry(cases: &[PlanCase]) -> String {
    let mut entries = String::new();
    for case in cases {
        let _ = writeln!(
            entries,
            "  {:?}: {{ load: () => import(\"./{}/index\"), model: {:?} }},",
            case.id, case.dir, case.model
        );
    }
    format!(
        "// Code generated by the nexgen conformance harness. DO NOT EDIT.\n\
         export const REGISTRY: Record<string, {{ load: () => Promise<unknown>; model: string }}> = {{\n\
         {entries}}};\n"
    )
}

pub fn run_typescript(workspace: &Workspace, cases: &[PlanCase]) -> Result<TargetVerdicts, String> {
    let root = workspace.target_root(Target::TypeScript);
    fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    link_node_modules(&root)?;
    fs::write(
        root.join("runner.test.ts"),
        include_str!("../../samples/conformance/runners/runner.test.ts"),
    )
    .map_err(|error| error.to_string())?;
    fs::write(root.join("registry.ts"), typescript_registry(cases))
        .map_err(|error| error.to_string())?;
    fs::write(
        root.join("package.json"),
        "{\n  \"name\": \"nexgen-conformance\",\n  \"private\": true,\n  \"type\": \"module\"\n}\n",
    )
    .map_err(|error| error.to_string())?;
    let plan_path = root.join("plan.json");
    let result_path = root.join("result.json");
    write_plan(&plan_path, cases)?;
    let vitest = repository_root().join("samples/typescript/node_modules/.bin/vitest");
    let execution = run(command(&vitest.to_string_lossy())
        .current_dir(&root)
        .env("NEXGEN_CONFORMANCE_PLAN", &plan_path)
        .env("NEXGEN_CONFORMANCE_RESULT", &result_path)
        .args(["run", "--root", ".", "runner.test.ts"]));
    if !execution.ok {
        return Err(format!("typescript runner failed:\n{}", execution.detail));
    }
    read_verdicts(&result_path)
}

// ---------------------------------------------------------------------------
// Java
// ---------------------------------------------------------------------------

/// The samples' compile+runtime classpath, resolved once per process.
///
/// Read out of Gradle through an init script rather than by editing
/// `samples/java/build.gradle`, which no agent owns in this rollout.
pub fn java_classpath() -> Result<String, String> {
    static CLASSPATH: OnceLock<Result<String, String>> = OnceLock::new();
    CLASSPATH.get_or_init(resolve_java_classpath).clone()
}

fn resolve_java_classpath() -> Result<String, String> {
    let cache = repository_root().join("target/nexgen-conformance/java-classpath.txt");
    let build_file = repository_root().join("samples/java/build.gradle");
    if let (Ok(cached), Ok(cached_meta), Ok(build_meta)) = (
        fs::read_to_string(&cache),
        fs::metadata(&cache),
        fs::metadata(&build_file),
    ) && cached_meta
        .modified()
        .ok()
        .zip(build_meta.modified().ok())
        .is_some_and(|(cached_at, built_at)| cached_at >= built_at)
        && !cached.trim().is_empty()
    {
        return Ok(cached.trim().to_string());
    }

    let init_script = repository_root().join("target/nexgen-conformance/classpath-init.gradle");
    fs::create_dir_all(init_script.parent().expect("parent")).map_err(|error| error.to_string())?;
    fs::write(
        &init_script,
        "// Generated by the nexgen conformance harness.\n\
         allprojects { project ->\n\
         \x20 project.plugins.withId('java') {\n\
         \x20   project.tasks.register('nexgenPrintConformanceClasspath') {\n\
         \x20     doLast {\n\
         \x20       println 'NEXGEN_CLASSPATH=' + (project.sourceSets.main.compileClasspath \
         + project.sourceSets.main.runtimeClasspath).files.join(File.pathSeparator)\n\
         \x20     }\n\
         \x20   }\n\
         \x20 }\n\
         }\n",
    )
    .map_err(|error| error.to_string())?;

    let samples = repository_root().join("samples/java");
    let execution = run(command("./gradlew")
        .current_dir(&samples)
        .args(["-q", "--console=plain", "--init-script"])
        .arg(&init_script)
        .arg("nexgenPrintConformanceClasspath"));
    if !execution.ok {
        return Err(format!(
            "gradle classpath probe failed:\n{}",
            execution.detail
        ));
    }
    let classpath = execution
        .detail
        .lines()
        .find_map(|line| line.strip_prefix("NEXGEN_CLASSPATH="))
        .ok_or_else(|| format!("gradle printed no classpath:\n{}", execution.detail))?
        .to_string();
    let _ = fs::write(&cache, &classpath);
    Ok(classpath)
}

fn java_sources(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            java_sources(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "java") {
            out.push(path);
        }
    }
}

/// Compile a set of Java sources into `classes`.
///
/// `--release 8` because the generated code declares that baseline; compiling at
/// the host JDK's default level would not assert it.
pub fn javac(root: &Path, classes: &Path, sources: &[PathBuf]) -> Result<(), String> {
    if sources.is_empty() {
        return Ok(());
    }
    let classpath = java_classpath()?;
    let mut javac = command("javac");
    javac
        .current_dir(root)
        .args(["-nowarn", "-encoding", "UTF-8", "--release", "8"])
        .arg("-cp")
        .arg(format!("{}:{classpath}", classes.display()))
        .arg("-d")
        .arg(classes)
        .args(sources);
    let compile = run(&mut javac);
    if compile.ok {
        Ok(())
    } else {
        Err(compile.detail)
    }
}

pub fn java_sources_of(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    java_sources(root, &mut out);
    out.sort();
    out
}

/// Compile every generated Java package in the workspace.
///
/// One uncompilable package would fail the whole `javac` invocation, so a
/// failure is retried package by package: "the generated code does not compile"
/// is a per-case verdict, not a reason to lose the other twelve.
pub fn compile_java(
    workspace: &Workspace,
    dirs: &[String],
) -> Result<(PathBuf, BTreeMap<String, String>), String> {
    let root = workspace.target_root(Target::Java);
    let classes = root.join("classes");
    fs::create_dir_all(&classes).map_err(|error| error.to_string())?;
    let all = java_sources_of(&root.join("src"));
    let mut broken = BTreeMap::new();
    if javac(&root, &classes, &all).is_ok() {
        return Ok((classes, broken));
    }
    for dir in dirs {
        let package = root.join("src/conformance").join(dir);
        if let Err(error) = javac(&root, &classes, &java_sources_of(&package)) {
            broken.insert(dir.clone(), error);
        }
    }
    Ok((classes, broken))
}

pub fn run_java(workspace: &Workspace, cases: &[PlanCase]) -> Result<TargetVerdicts, String> {
    let root = workspace.target_root(Target::Java);
    fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    let dirs: Vec<String> = cases.iter().map(|case| case.dir.clone()).collect();
    let (classes, by_dir) = compile_java(workspace, &dirs)?;
    let broken: BTreeMap<String, String> = cases
        .iter()
        .filter_map(|case| {
            by_dir
                .get(&case.dir)
                .map(|error| (case.id.clone(), error.clone()))
        })
        .collect();
    let runner_source = root.join("Runner.java");
    fs::write(
        &runner_source,
        include_str!("../../samples/conformance/runners/Runner.java"),
    )
    .map_err(|error| error.to_string())?;
    javac(&root, &classes, std::slice::from_ref(&runner_source))
        .map_err(|error| format!("the conformance runner itself does not compile:\n{error}"))?;

    let runnable: Vec<PlanCase> = cases
        .iter()
        .filter(|case| !broken.contains_key(&case.id))
        .cloned()
        .collect();
    let plan_path = root.join("plan.json");
    let result_path = root.join("result.json");
    write_plan(&plan_path, &runnable)?;
    let classpath = format!("{}:{}", classes.display(), java_classpath()?);
    let mut verdicts = if runnable.is_empty() {
        TargetVerdicts::new()
    } else {
        let execution = run(command("java")
            .current_dir(&root)
            .arg("-cp")
            .arg(&classpath)
            .arg("Runner")
            .arg(&plan_path)
            .arg(&result_path));
        if !execution.ok {
            return Err(format!("java runner failed:\n{}", execution.detail));
        }
        read_verdicts(&result_path)?
    };
    add_build_failures(&mut verdicts, cases, &broken);
    Ok(verdicts)
}

// ---------------------------------------------------------------------------
// Driver entry point
// ---------------------------------------------------------------------------

fn write_plan(path: &Path, cases: &[PlanCase]) -> Result<(), String> {
    let plan = json!({ "cases": cases.iter().map(PlanCase::to_json).collect::<Vec<_>>() });
    fs::write(
        path,
        serde_json::to_vec_pretty(&plan).expect("serialize plan"),
    )
    .map_err(|error| error.to_string())
}

fn read_verdicts(path: &Path) -> Result<TargetVerdicts, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("runner wrote no result at {}: {error}", path.display()))?;
    serde_json::from_str(&text).map_err(|error| format!("unreadable runner result: {error}"))
}

/// Run one plan through a target, returning either its verdicts or the reason
/// the whole target could not report (a build break, usually).
pub fn run_target(
    workspace: &Workspace,
    target: Target,
    cases: &[PlanCase],
) -> Result<TargetVerdicts, String> {
    match target {
        Target::Go => run_go(workspace, cases),
        Target::Java => run_java(workspace, cases),
        Target::Python => run_python(workspace, cases),
        Target::TypeScript => run_typescript(workspace, cases),
    }
}
