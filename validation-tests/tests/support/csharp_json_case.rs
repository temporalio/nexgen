//! Generation half of the C# JSON Schema test harness, shared by the Rust-only
//! tests in `tests/generate_dotnet_json_schema.rs` and the toolchain tests in
//! `support/dotnet_json_schema.rs`, which add the `dotnet` build and run steps.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use nexgen::language::Language;
use nexgen::nexgen_config::GenerationTargets;
use nexgen::nexgen_config::NexgenConfig;
use nexgen::{GenerateRequest, generate_to_file};

static OUTPUT_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn unique_output_path(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let counter = OUTPUT_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("nexgen-{label}-{unique}-{counter}"))
}

/// The namespace every single-input case generates into, so a `Program.cs`
/// can name the models without knowing the derivation rules.
pub const NAMESPACE: &str = "Probe";

/// The generated files of one run, keyed by path relative to the output
/// directory.
pub struct Output(pub BTreeMap<PathBuf, String>);

impl Output {
    pub fn file(&self, relative: &str) -> &str {
        self.0.get(Path::new(relative)).unwrap_or_else(|| {
            panic!(
                "no generated `{relative}`; generated: {:?}",
                self.0.keys().collect::<Vec<_>>()
            )
        })
    }

    pub fn has_file(&self, relative: &str) -> bool {
        self.0.contains_key(Path::new(relative))
    }

    pub fn all(&self) -> String {
        self.0
            .iter()
            .map(|(path, contents)| format!("### {}\n{contents}", path.display()))
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

/// Asserts `haystack` contains every needle, showing the haystack on failure.
#[track_caller]
pub fn assert_has(haystack: &str, needles: &[&str]) {
    for needle in needles {
        assert!(
            haystack.contains(needle),
            "expected to find:\n{needle}\n\nin:\n{haystack}"
        );
    }
}

/// Asserts `haystack` contains none of the needles.
#[track_caller]
pub fn assert_lacks(haystack: &str, needles: &[&str]) {
    for needle in needles {
        assert!(
            !haystack.contains(needle),
            "expected not to find:\n{needle}\n\nin:\n{haystack}"
        );
    }
}

/// One scratch directory: authored schemas under `input/`, generated C# under
/// `output/`, and — for the toolchain tests' `JsonCase::run` — a console project at the root that
/// compiles both with a hand-written `Program.cs`. Removed on success; kept on
/// failure so the generated code can be inspected.
pub struct JsonCase {
    pub root: PathBuf,
}

impl JsonCase {
    pub fn new(label: &str) -> Self {
        let root = unique_output_path(&format!("dotnet-json-{label}"));
        fs::create_dir_all(root.join("input")).unwrap();
        Self { root }
    }

    /// Writes an input file at `relative` below `input/`.
    pub fn write(&self, relative: &str, contents: &str) -> &Self {
        let path = self.root.join("input").join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
        self
    }

    pub fn input(&self, relative: &str) -> PathBuf {
        self.root.join("input").join(relative)
    }

    pub fn output_dir(&self) -> PathBuf {
        self.root.join("output")
    }

    /// Generates definitions-only C# for `inputs` (paths below `input/`; `.`
    /// is the whole input directory) into the [`NAMESPACE`] root namespace.
    pub fn generate(&self, inputs: &[&str]) -> Output {
        self.try_generate(inputs, GenerationTargets::NONE, Some(NAMESPACE))
            .unwrap_or_else(|error| panic!("generation failed: {error}"))
    }

    pub fn generate_native(&self, inputs: &[&str]) -> Output {
        self.try_generate(inputs, GenerationTargets::WORKFLOW, Some(NAMESPACE))
            .unwrap_or_else(|error| panic!("generation failed: {error}"))
    }

    /// The load or generation diagnostic for `inputs`.
    pub fn generate_err(&self, inputs: &[&str]) -> String {
        match self.try_generate(inputs, GenerationTargets::NONE, Some(NAMESPACE)) {
            Ok(output) => panic!("generation succeeded:\n{}", output.all()),
            Err(error) => error,
        }
    }

    pub fn try_generate(
        &self,
        inputs: &[&str],
        targets: GenerationTargets,
        namespace: Option<&str>,
    ) -> Result<Output, String> {
        let output_path = self.output_dir();
        if output_path.exists() {
            fs::remove_dir_all(&output_path).unwrap();
        }
        generate_to_file(&GenerateRequest {
            config: NexgenConfig {
                targets,
                ..Default::default()
            },
            language: Language::Dotnet,
            input_paths: inputs.iter().map(|input| self.input(input)).collect(),
            support_package: None,
            descriptor_paths: Vec::new(),
            output_path: output_path.clone(),
            format: false,
            java_package_name: None,
            ts_date_time_types: Default::default(),
            dotnet_namespace: namespace.map(str::to_string),
        })
        .map_err(|error| error.to_string())?;
        Ok(Output(read_cs_files(&output_path)))
    }
}

impl Drop for JsonCase {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

pub fn read_cs_files(dir: &Path) -> BTreeMap<PathBuf, String> {
    pub fn visit(root: &Path, dir: &Path, files: &mut BTreeMap<PathBuf, String>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        let mut paths = entries
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        paths.sort();
        for path in paths {
            if path.is_dir() {
                visit(root, &path, files);
            } else if path.extension().is_some_and(|extension| extension == "cs") {
                files.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    fs::read_to_string(&path).unwrap(),
                );
            }
        }
    }
    let mut files = BTreeMap::new();
    visit(dir, dir, &mut files);
    files
}
