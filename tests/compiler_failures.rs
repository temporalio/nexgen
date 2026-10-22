//! File-backed compiler rejection tests, requiring no target-language tools.
#![cfg(feature = "advanced")]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use nexgen::error::Error;
use nexgen::generator::GenerationMode;
use nexgen::language::Language;
use nexgen::{GenerateRequest, generate_to_file};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Expectation {
    #[serde(default = "default_languages")]
    languages: Vec<Language>,
    #[serde(default = "default_inputs")]
    inputs: Vec<PathBuf>,
    descriptors: Option<Descriptors>,
    error: ExpectedError,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Descriptors {
    Preset(DescriptorPreset),
    Paths(Vec<PathBuf>),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum DescriptorPreset {
    Temporal,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpectedError {
    kind: ErrorKind,
    contains: Vec<String>,
}

// This is deliberately a test-only vocabulary, not a public diagnostic API.
// Extend it as more existing rejection tests move into the fixture suite.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum ErrorKind {
    InvalidProtoEnumBinding,
    InvalidTypeOverrideField,
    DotnetTypeParameterNameConflict,
    UnsupportedJavaProtoModel,
}

impl ErrorKind {
    fn matches(&self, error: &Error) -> bool {
        matches!(
            (self, error),
            (
                Self::InvalidProtoEnumBinding,
                Error::InvalidProtoEnumBinding { .. }
            ) | (
                Self::InvalidTypeOverrideField,
                Error::InvalidTypeOverrideField { .. }
            ) | (
                Self::DotnetTypeParameterNameConflict,
                Error::DotnetTypeParameterNameConflict { .. }
            ) | (
                Self::UnsupportedJavaProtoModel,
                Error::UnsupportedJavaProtoModel { .. }
            )
        )
    }
}

fn support_package(language: Language) -> Option<String> {
    match language {
        Language::Python => Some("temporal_support".into()),
        Language::TypeScript => Some("./support".into()),
        Language::Go => Some("example.com/support".into()),
        Language::Dotnet => Some("Nexgen.Support".into()),
        Language::Java | Language::Ruby => None,
    }
}

fn default_languages() -> Vec<Language> {
    vec![
        Language::Python,
        Language::TypeScript,
        Language::Go,
        Language::Dotnet,
    ]
}

fn default_inputs() -> Vec<PathBuf> {
    vec![PathBuf::from("input.wit")]
}

fn parse_expectation(source: &str) -> Result<Expectation, String> {
    let expected: Expectation = toml::from_str(source).map_err(|error| error.to_string())?;
    if expected.languages.is_empty() {
        return Err("languages must not be empty".into());
    }
    if expected.languages.iter().collect::<BTreeSet<_>>().len() != expected.languages.len() {
        return Err("languages must not contain duplicates".into());
    }
    if expected.languages.contains(&Language::Ruby) {
        return Err("Ruby generation is not supported by the compiler fixture suite".into());
    }
    if expected.inputs.is_empty() {
        return Err("inputs must not be empty".into());
    }
    if expected.error.contains.is_empty()
        || expected
            .error
            .contains
            .iter()
            .any(|part| part.trim().is_empty())
    {
        return Err("error.contains must include at least one nonempty fragment".into());
    }
    Ok(expected)
}

fn check_result(expected: &ExpectedError, result: Result<(), Error>) -> Result<(), String> {
    let error = result
        .err()
        .ok_or_else(|| format!("expected {:?}, but compilation succeeded", expected.kind))?;
    let message = error.to_string();
    if !expected.kind.matches(&error) {
        return Err(format!(
            "expected error kind {:?}, got {error:?}\ndiagnostic: {message}",
            expected.kind
        ));
    }
    let missing = expected
        .contains
        .iter()
        .filter(|fragment| !message.contains(fragment.as_str()))
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(format!(
            "missing diagnostic fragments {missing:?}\ndiagnostic: {message}"
        ));
    }
    Ok(())
}

// Resolve configuration before compiling: a broken fixture must never count as
// the expected compiler failure, even if it happens to produce matching text.
fn resolve_paths(directory: &Path, paths: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    paths
        .iter()
        .map(|path| {
            let path = directory.join(path);
            path.canonicalize()
                .map_err(|error| format!("cannot resolve {}: {error}", path.display()))
        })
        .collect()
}

fn resolve_descriptors(
    descriptors: Option<&Descriptors>,
    directory: &Path,
    repo: &Path,
) -> Result<Vec<PathBuf>, String> {
    match descriptors {
        None => Ok(Vec::new()),
        Some(Descriptors::Paths(paths)) => resolve_paths(directory, paths),
        Some(Descriptors::Preset(DescriptorPreset::Temporal)) => resolve_paths(
            repo,
            &[PathBuf::from(
                "advanced/samples/descriptors/temporal_api.bin",
            )],
        ),
    }
}

fn run_case(directory: &Path, repo: &Path) -> Result<Vec<String>, String> {
    let config_path = directory.join("expected.toml");
    let source = fs::read_to_string(&config_path)
        .map_err(|error| format!("cannot read {}: {error}", config_path.display()))?;
    let expected = parse_expectation(&source)
        .map_err(|error| format!("{}: {error}", config_path.display()))?;
    let inputs = resolve_paths(directory, &expected.inputs)?;
    let descriptors = resolve_descriptors(expected.descriptors.as_ref(), directory, repo)?;
    let mut failures = Vec::new();
    for &language in &expected.languages {
        let output = tempfile::tempdir().map_err(|error| error.to_string())?;
        let result = generate_to_file(&GenerateRequest {
            config: nexgen::nexgen_config::NexgenConfig {
                mode: GenerationMode::NativeApi,
                ..Default::default()
            },
            language,
            input_paths: inputs.clone(),
            support_package: support_package(language),
            descriptor_paths: descriptors.clone(),
            output_path: output.path().join("generated"),
            format: false,
            java_package_name: (language == Language::Java).then(|| "generated".into()),
            ts_date_time_types: Default::default(),
        });
        if let Err(error) = check_result(&expected.error, result) {
            failures.push(format!("{} [{}]: {error}", directory.display(), language));
        }
    }
    Ok(failures)
}

#[test]
fn compiler_failures() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = repo.join("advanced/samples/invalid");
    let mut cases = fs::read_dir(&root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    cases.sort();
    assert!(
        !cases.is_empty(),
        "no compiler failure fixtures in {}",
        root.display()
    );

    let mut failures = Vec::new();
    for directory in cases {
        match run_case(&directory, repo) {
            Ok(errors) => failures.extend(errors),
            Err(error) => failures.push(format!("{}: {error}", directory.display())),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn expectations_are_strict_and_require_meaningful_assertions() {
    let valid = r#"
[error]
kind = "invalid-proto-enum-binding"
contains = ["severity-low"]
"#;
    let expected = parse_expectation(valid).unwrap();
    assert_eq!(expected.languages, default_languages());
    assert_eq!(expected.inputs, default_inputs());
    assert!(expected.descriptors.is_none());
    let explicit = parse_expectation(&format!(
        "languages = [\"java\"]\ninputs = [\"api.wit\", \"deps\"]\ndescriptors = []\n{valid}"
    ))
    .unwrap();
    assert_eq!(explicit.languages, vec![Language::Java]);
    assert_eq!(
        explicit.inputs,
        vec![PathBuf::from("api.wit"), PathBuf::from("deps")]
    );
    assert!(matches!(explicit.descriptors, Some(Descriptors::Paths(paths)) if paths.is_empty()));
    let preset = parse_expectation(&format!("descriptors = \"temporal\"\n{valid}")).unwrap();
    assert!(matches!(
        preset.descriptors,
        Some(Descriptors::Preset(DescriptorPreset::Temporal))
    ));
    for invalid in [
        valid.replace("[error]", "typo = true\n[error]"),
        valid.replace("contains =", "typo ="),
        valid.replace("invalid-proto-enum-binding", "unknown-error-kind"),
        valid.replace("[\"severity-low\"]", "[]"),
        valid.replace("[\"severity-low\"]", "[\" \"]"),
        format!("languages = []\n{valid}"),
        format!("languages = [\"python\", \"python\"]\n{valid}"),
        format!("languages = [\"unknown\"]\n{valid}"),
        format!("languages = [\"ruby\"]\n{valid}"),
        format!("inputs = []\n{valid}"),
        format!("descriptors = \"unknown\"\n{valid}"),
        format!("descriptors = true\n{valid}"),
    ] {
        assert!(
            parse_expectation(&invalid).is_err(),
            "accepted invalid expectation: {invalid}"
        );
    }
}

#[test]
fn descriptors_are_opt_in_and_paths_resolve_from_their_declared_roots() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let case = tempfile::tempdir().unwrap();
    // With no descriptors selected, even a nonexistent repository is unused.
    assert!(
        resolve_descriptors(None, case.path(), &case.path().join("missing-repo"))
            .unwrap()
            .is_empty()
    );
    assert!(
        resolve_descriptors(Some(&Descriptors::Paths(Vec::new())), case.path(), repo)
            .unwrap()
            .is_empty()
    );
    let temporal = repo
        .join("advanced/samples/descriptors/temporal_api.bin")
        .canonicalize()
        .unwrap();
    assert_eq!(
        resolve_descriptors(
            Some(&Descriptors::Preset(DescriptorPreset::Temporal)),
            case.path(),
            repo,
        )
        .unwrap(),
        vec![temporal]
    );
    fs::write(case.path().join("custom.bin"), []).unwrap();
    assert_eq!(
        resolve_descriptors(
            Some(&Descriptors::Paths(vec![PathBuf::from("custom.bin")])),
            case.path(),
            repo,
        )
        .unwrap(),
        vec![case.path().join("custom.bin").canonicalize().unwrap()]
    );
}

#[test]
fn fixture_setup_errors_cannot_satisfy_an_expected_failure() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let case = tempfile::tempdir().unwrap();
    assert!(
        run_case(case.path(), repo)
            .unwrap_err()
            .contains("expected.toml")
    );

    let expected = r#"
[error]
kind = "invalid-type-override-field"
contains = ["input.wit"]
"#;
    fs::write(case.path().join("expected.toml"), expected).unwrap();
    assert!(
        run_case(case.path(), repo)
            .unwrap_err()
            .contains("input.wit")
    );

    fs::write(case.path().join("input.wit"), "").unwrap();
    fs::write(
        case.path().join("expected.toml"),
        format!("descriptors = [\"missing.bin\"]\n{expected}"),
    )
    .unwrap();
    assert!(
        run_case(case.path(), repo)
            .unwrap_err()
            .contains("missing.bin")
    );
}

#[test]
fn matching_requires_the_error_kind_and_every_fragment() {
    let expected = ExpectedError {
        kind: ErrorKind::InvalidProtoEnumBinding,
        contains: vec!["Severity".into(), "severity-low".into()],
    };
    assert!(check_result(&expected, Ok(())).is_err());
    assert!(
        check_result(
            &expected,
            Err(Error::UnsupportedJavaProtoModel {
                message: "Severity severity-low".into()
            })
        )
        .is_err()
    );
    assert!(
        check_result(
            &expected,
            Err(Error::InvalidProtoEnumBinding {
                enumeration: "Severity".into(),
                reason: "unrelated case".into(),
            })
        )
        .is_err()
    );
    assert!(
        check_result(
            &expected,
            Err(Error::InvalidProtoEnumBinding {
                enumeration: "Severity".into(),
                reason: "severity-low".into(),
            })
        )
        .is_ok()
    );
}
