//! File-backed compiler rejection tests, requiring no target-language tools.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use nexgen::error::{Error, ErrorKind};
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
    #[serde(default)]
    system_nexus: bool,
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
    if expected.kind != ErrorKind::from(&error) {
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
                system_nexus: expected.system_nexus,
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

fn discover_cases(directory: &Path) -> Result<Vec<PathBuf>, String> {
    // An expectation marks a case root. Do not descend into linked inputs or
    // dependencies within that case, even if they contain subdirectories.
    if directory.join("expected.toml").is_file() {
        return Ok(vec![directory.to_path_buf()]);
    }
    let mut children = fs::read_dir(directory)
        .map_err(|error| format!("cannot read {}: {error}", directory.display()))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    children.retain(|path| path.is_dir());
    children.sort();
    if directory.join("input.wit").exists() || children.is_empty() {
        return Err(format!(
            "{}: expected a category containing cases or a case with expected.toml",
            directory.display()
        ));
    }
    let mut cases = Vec::new();
    for child in children {
        cases.extend(discover_cases(&child)?);
    }
    Ok(cases)
}

#[test]
fn compiler_failures() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = repo.join("advanced/samples/invalid");
    let cases = discover_cases(&root).unwrap();

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
fn system_nexus_requires_explicit_configuration_not_a_category_name() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let fixture =
        repo.join("advanced/samples/invalid/system-nexus/serialization-context/helper-keyword");
    let temp = tempfile::tempdir().unwrap();
    let case = temp.path().join("system-nexus");
    fs::create_dir(&case).unwrap();
    fs::copy(fixture.join("input.wit"), case.join("input.wit")).unwrap();
    let config = fs::read_to_string(fixture.join("expected.toml")).unwrap();

    fs::write(
        case.join("expected.toml"),
        config.replace("system_nexus = true\n", ""),
    )
    .unwrap();
    let failures = run_case(&case, repo).unwrap();
    assert_eq!(failures.len(), 1);
    assert!(
        failures[0].contains("compilation succeeded"),
        "{}",
        failures[0]
    );

    fs::write(case.join("expected.toml"), config).unwrap();
    assert!(run_case(&case, repo).unwrap().is_empty());
}

#[test]
fn discovery_recurses_through_categories_but_not_case_dependencies() {
    let root = tempfile::tempdir().unwrap();
    assert!(discover_cases(root.path()).is_err());
    for case in ["wit-parser/directive", "proto/enums/sparse"] {
        let path = root.path().join(case);
        fs::create_dir_all(path.join("deps/nested-package")).unwrap();
        fs::write(path.join("expected.toml"), "").unwrap();
    }
    assert_eq!(
        discover_cases(root.path()).unwrap(),
        vec![
            root.path().join("proto/enums/sparse"),
            root.path().join("wit-parser/directive"),
        ]
    );
    let incomplete = root.path().join("wit-parser/missing-expectation");
    fs::create_dir_all(incomplete.join("deps")).unwrap();
    fs::write(incomplete.join("input.wit"), "").unwrap();
    let error = discover_cases(root.path()).unwrap_err();
    assert!(error.contains("missing-expectation"), "{error}");
    assert!(error.contains("expected.toml"), "{error}");
}

#[test]
fn expectations_reject_vacuous_diagnostic_assertions() {
    for contains in ["[]", "[\" \"]"] {
        let source =
            format!("[error]\nkind = \"invalid-proto-enum-binding\"\ncontains = {contains}");
        assert_eq!(
            parse_expectation(&source).unwrap_err(),
            "error.contains must include at least one nonempty fragment"
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
fn matches_io_error_by_kind_and_message() {
    let expected = parse_expectation(
        r#"
[error]
kind = "read-file"
contains = ["missing.wit", "fixture file missing"]
"#,
    )
    .unwrap();
    let actual = Error::ReadFile {
        path: PathBuf::from("missing.wit"),
        source: std::io::Error::new(std::io::ErrorKind::NotFound, "fixture file missing"),
    };
    check_result(&expected.error, Err(actual)).unwrap();
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
