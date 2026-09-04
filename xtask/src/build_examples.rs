use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

use heck::ToSnakeCase;
use nexgen::error::{Error, Result};
use nexgen::generator::{GenerationMode, TsDateTimeTypes};
use nexgen::language::Language;
use nexgen::nexgen_config::NexgenConfig;
use nexgen::{GenerateRequest, generate_to_file};

#[derive(Clone)]
pub struct BuildExamplesRequest {
    pub format: Option<ExampleFormat>,
    pub languages: Vec<Language>,
    pub example_ids: Vec<String>,
}

#[derive(Clone, Copy)]
pub enum ExampleFormat {
    Wit,
    JsonSchema,
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

pub fn build_examples(request: &BuildExamplesRequest) -> Result<()> {
    match request.format {
        Some(ExampleFormat::Wit) => build_wit_examples(request),
        Some(ExampleFormat::JsonSchema) => build_json_examples(request),
        None => {
            let repo_root = repo_root();
            let wit_request = request_for_languages(request, |language| {
                matches!(
                    language,
                    Language::Dotnet | Language::Python | Language::TypeScript | Language::Go
                )
            });
            let json_request = request_for_languages(request, |language| {
                matches!(
                    language,
                    Language::Python | Language::TypeScript | Language::Go | Language::Java
                )
            });
            build_selected_examples(&repo_root, wit_request, json_request)?;
            Ok(())
        }
    }
}

fn build_selected_examples(
    repo_root: &Path,
    wit_request: Option<BuildExamplesRequest>,
    json_request: Option<BuildExamplesRequest>,
) -> Result<()> {
    let wit_ids = wit_request
        .as_ref()
        .map(|request| available_wit_example_ids(repo_root, request))
        .transpose()?
        .unwrap_or_default();
    let json_ids: std::collections::BTreeSet<String> = json_request
        .as_ref()
        .map(|_| discover_json_example_ids(repo_root).map(|ids| ids.into_iter().collect()))
        .transpose()?
        .unwrap_or_default();

    for request in [wit_request.as_ref(), json_request.as_ref()]
        .into_iter()
        .flatten()
    {
        for example_id in &request.example_ids {
            if !wit_ids.contains(example_id) && !json_ids.contains(example_id) {
                return Err(Error::UnknownExampleId {
                    language: request
                        .languages
                        .first()
                        .copied()
                        .unwrap_or(Language::Python),
                    example_id: example_id.clone(),
                });
            }
        }
    }

    if let Some(request) =
        wit_request.and_then(|request| request_with_example_ids(request, &wit_ids))
    {
        build_wit_examples(&request)?;
    }
    if let Some(request) =
        json_request.and_then(|request| request_with_example_ids(request, &json_ids))
    {
        build_json_examples(&request)?;
    }
    Ok(())
}

fn available_wit_example_ids(
    repo_root: &Path,
    request: &BuildExamplesRequest,
) -> Result<std::collections::BTreeSet<String>> {
    let languages = if request.languages.is_empty() {
        vec![
            Language::Dotnet,
            Language::Python,
            Language::TypeScript,
            Language::Go,
        ]
    } else {
        request.languages.clone()
    };
    let mut ids = std::collections::BTreeSet::new();
    for language in languages {
        ids.extend(discover_example_ids(repo_root, language)?);
    }
    Ok(ids)
}

fn request_with_example_ids(
    mut request: BuildExamplesRequest,
    available_ids: &std::collections::BTreeSet<String>,
) -> Option<BuildExamplesRequest> {
    let requested_ids = !request.example_ids.is_empty();
    request.example_ids.retain(|id| available_ids.contains(id));
    (!requested_ids || !request.example_ids.is_empty()).then_some(request)
}

fn request_for_languages(
    request: &BuildExamplesRequest,
    supports: impl Fn(Language) -> bool,
) -> Option<BuildExamplesRequest> {
    if request.languages.is_empty() {
        return Some(request.clone());
    }
    let languages = request
        .languages
        .iter()
        .copied()
        .filter(|language| supports(*language))
        .collect::<Vec<_>>();
    (!languages.is_empty()).then_some(BuildExamplesRequest {
        format: request.format,
        languages,
        example_ids: request.example_ids.clone(),
    })
}

fn build_wit_examples(request: &BuildExamplesRequest) -> Result<()> {
    let repo_root = repo_root();
    let use_default_languages = request.languages.is_empty();
    let languages = if use_default_languages {
        vec![
            Language::Dotnet,
            Language::Python,
            Language::TypeScript,
            Language::Go,
        ]
    } else {
        request.languages.clone()
    };
    for language in languages {
        let example_ids = if request.example_ids.is_empty() {
            discover_example_ids(&repo_root, language)?
        } else if use_default_languages {
            filter_available_example_ids(&repo_root, language, &request.example_ids)?
        } else {
            validate_example_ids(&repo_root, language, &request.example_ids)?
        };
        if example_ids.is_empty() {
            continue;
        }
        if language == Language::TypeScript {
            ensure_typescript_dependencies(&advanced_language_root(&repo_root, language))?;
        }
        for example_id in example_ids {
            build_example(&repo_root, language, &example_id)?;
        }
    }
    Ok(())
}

pub fn build_json_examples(request: &BuildExamplesRequest) -> Result<()> {
    let repo_root = repo_root();
    let languages = if request.languages.is_empty() {
        vec![
            Language::Python,
            Language::TypeScript,
            Language::Go,
            Language::Java,
        ]
    } else {
        request.languages.clone()
    };
    for language in languages {
        if !matches!(
            language,
            Language::Python | Language::TypeScript | Language::Go | Language::Java
        ) {
            return Err(Error::UnsupportedLanguage { language });
        }
        if language == Language::TypeScript {
            ensure_typescript_dependencies(&samples_language_root(&repo_root, language))?;
            ensure_typescript_dependencies(&advanced_language_root(&repo_root, language))?;
        }
        let example_ids = if request.example_ids.is_empty() {
            discover_json_example_ids(&repo_root)?
        } else {
            validate_json_example_ids(&repo_root, language, &request.example_ids)?
        };
        for example_id in example_ids {
            build_json_example(&repo_root, language, &example_id)?;
        }
    }
    Ok(())
}

fn discover_example_ids(repo_root: &Path, language: Language) -> Result<Vec<String>> {
    let input_root = repo_root.join("advanced/samples/inputs");
    let mut ids = fs::read_dir(&input_root)
        .map_err(|source| Error::ReadFile {
            path: input_root,
            source,
        })?
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            let id = if path.is_file() {
                path.file_stem()?.to_string_lossy().into_owned()
            } else if path.join("main.wit").is_file() {
                path.file_name()?.to_string_lossy().into_owned()
            } else {
                return None;
            };
            example_output_path(repo_root, language, &id)
                .is_dir()
                .then_some(id)
        })
        .collect::<Vec<_>>();
    ids.sort();
    Ok(ids)
}

fn validate_example_ids(
    repo_root: &Path,
    language: Language,
    ids: &[String],
) -> Result<Vec<String>> {
    let available = discover_example_ids(repo_root, language)?
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
    for id in ids {
        if !available.contains(id) {
            return Err(Error::UnknownExampleId {
                language,
                example_id: id.clone(),
            });
        }
    }
    Ok(ids.to_vec())
}

fn filter_available_example_ids(
    repo_root: &Path,
    language: Language,
    ids: &[String],
) -> Result<Vec<String>> {
    let available = discover_example_ids(repo_root, language)?
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
    Ok(ids
        .iter()
        .filter(|id| available.contains(id.as_str()))
        .cloned()
        .collect())
}

fn discover_json_example_ids(repo_root: &Path) -> Result<Vec<String>> {
    let root = repo_root.join("samples/schemas");
    let mut ids = fs::read_dir(&root)
        .map_err(|source| Error::ReadFile { path: root, source })?
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            if path.is_dir() {
                return directory_contains_input_file(&path)
                    .then(|| path.file_name()?.to_str().map(str::to_string))
                    .flatten();
            }
            let stem = path.file_stem()?.to_str()?;
            is_json_schema_input_path(&path)
                .then(|| stem.strip_suffix(".nexusrpc").unwrap_or(stem).to_string())
        })
        .collect::<Vec<_>>();
    ids.sort();
    Ok(ids)
}

fn directory_contains_input_file(path: &Path) -> bool {
    fs::read_dir(path).ok().is_some_and(|entries| {
        entries.flatten().any(|entry| {
            let path = entry.path();
            path.is_file() || (path.is_dir() && directory_contains_input_file(&path))
        })
    })
}
fn is_json_schema_input_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e, "json" | "yaml" | "yml"))
}
fn validate_json_example_ids(
    repo_root: &Path,
    language: Language,
    ids: &[String],
) -> Result<Vec<String>> {
    let available = discover_json_example_ids(repo_root)?
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
    for id in ids {
        if !available.contains(id) {
            return Err(Error::UnknownExampleId {
                language,
                example_id: id.clone(),
            });
        }
    }
    Ok(ids.to_vec())
}

fn ensure_typescript_dependencies(cwd: &Path) -> Result<()> {
    if cwd.join("node_modules").exists() {
        return Ok(());
    }
    let command = "npm install --no-fund --no-audit".to_string();
    let status = ProcessCommand::new("npm")
        .current_dir(cwd)
        .args(["install", "--no-fund", "--no-audit"])
        .status()
        .map_err(|source| Error::RunCommand {
            cwd: cwd.to_path_buf(),
            command: command.clone(),
            source,
        })?;
    if !status.success() {
        return Err(Error::CommandFailed {
            cwd: cwd.to_path_buf(),
            command,
            status,
        });
    }
    Ok(())
}

fn reset_example_output_directory(language_root: &Path, output_path: &Path) -> Result<()> {
    let below_root = output_path
        .strip_prefix(language_root)
        .is_ok_and(|relative| {
            let mut components = relative.components().peekable();
            components.peek().is_some()
                && components.all(|component| matches!(component, std::path::Component::Normal(_)))
        });
    if !below_root {
        return Err(Error::ExampleOutputPathOutsideRoot {
            path: output_path.to_path_buf(),
            root: language_root.to_path_buf(),
        });
    }
    match fs::remove_dir_all(output_path) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(Error::WriteFile {
            path: output_path.to_path_buf(),
            source,
        }),
    }
}

fn build_example(repo_root: &Path, language: Language, example_id: &str) -> Result<()> {
    let input_path = example_input_path(repo_root, example_id);
    let mut input_paths = vec![input_path.clone()];
    input_paths.extend(example_linked_input_paths(repo_root, &input_path)?);
    let output_path = example_output_path(repo_root, language, example_id);
    let language_root = advanced_language_root(repo_root, language);
    reset_example_output_directory(&language_root, &output_path)?;
    let generate_request = GenerateRequest {
        config: NexgenConfig {
            mode: GenerationMode::NativeApi,
            system_nexus: matches!(
                language,
                Language::Dotnet | Language::Python | Language::TypeScript
            ) && example_id == "workflow-service",
        },
        language,
        input_paths,
        support_paths: Vec::new(),
        descriptor_paths: vec![repo_root.join("advanced/samples/descriptors/temporal_api.bin")],
        output_path: output_path.clone(),
        format: false,
        java_package_name: (language == Language::Java)
            .then(|| example_directory_name(language, example_id)),
        ts_date_time_types: Default::default(),
    };
    generate_to_file(&generate_request)?;
    format_example_output(&language_root, language, &output_path)?;
    normalize_example_headers(&output_path)?;
    println!("Built {} with nexgen", output_path.display());
    Ok(())
}

fn build_json_example(repo_root: &Path, language: Language, id: &str) -> Result<()> {
    build_json_example_variant(repo_root, language, id, id, TsDateTimeTypes::String)?;
    if language == Language::TypeScript && id == "temporal" {
        build_json_example_variant(
            repo_root,
            language,
            id,
            "temporal-date",
            TsDateTimeTypes::Date,
        )?;
        build_json_example_variant(
            repo_root,
            language,
            id,
            "temporal-temporal",
            TsDateTimeTypes::Temporal,
        )?;
    }
    Ok(())
}

fn build_json_example_variant(
    repo_root: &Path,
    language: Language,
    input_id: &str,
    output_id: &str,
    ts_date_time_types: TsDateTimeTypes,
) -> Result<()> {
    let input_path = json_example_input_path(repo_root, input_id);
    let dir_name = example_directory_name(language, output_id);
    for (mode, root) in [
        (
            GenerationMode::DefinitionsOnly,
            samples_language_root(repo_root, language),
        ),
        (
            GenerationMode::NativeApi,
            advanced_language_root(repo_root, language),
        ),
    ] {
        let output_path = json_example_output_path(repo_root, language, output_id, mode);
        reset_example_output_directory(&root, &output_path)?;
        generate_to_file(&GenerateRequest {
            config: NexgenConfig {
                mode,
                ..Default::default()
            },
            language,
            input_paths: vec![input_path.clone()],
            support_paths: Vec::new(),
            descriptor_paths: Vec::new(),
            output_path: output_path.clone(),
            format: false,
            java_package_name: (language == Language::Java)
                .then(|| json_example_java_package(&dir_name, mode)),
            ts_date_time_types,
        })?;
        format_example_output(&root, language, &output_path)?;
        normalize_example_headers(&output_path)?;
        println!("Built {} with nexgen", output_path.display());
    }
    Ok(())
}

fn samples_language_root(root: &Path, language: Language) -> PathBuf {
    root.join("samples").join(language.as_str())
}
fn advanced_language_root(root: &Path, language: Language) -> PathBuf {
    root.join("advanced/samples").join(language.as_str())
}
fn example_input_path(root: &Path, id: &str) -> PathBuf {
    let flat = root
        .join("advanced/samples/inputs")
        .join(format!("{id}.wit"));
    if flat.is_file() {
        flat
    } else {
        root.join("advanced/samples/inputs")
            .join(id)
            .join("main.wit")
    }
}
fn json_example_input_path(root: &Path, id: &str) -> PathBuf {
    let input_root = root.join("samples/schemas");
    let directory = input_root.join(id);
    if directory.is_dir() {
        return directory;
    }
    for extension in ["yaml", "yml", "json"] {
        for stem in [id.to_string(), format!("{id}.nexusrpc")] {
            let path = input_root.join(format!("{stem}.{extension}"));
            if path.is_file() {
                return path;
            }
        }
    }
    input_root.join(format!("{id}.yaml"))
}
fn example_linked_input_paths(root: &Path, input_path: &Path) -> Result<Vec<PathBuf>> {
    let input = fs::read_to_string(input_path).map_err(|source| Error::ReadFile {
        path: input_path.to_path_buf(),
        source,
    })?;
    if !input.contains("use nexus:temporal-types/") {
        return Ok(Vec::new());
    }
    let path = root.join("advanced/samples/inputs/deps");
    Ok(path.is_dir().then_some(path).into_iter().collect())
}
fn example_output_path(root: &Path, language: Language, id: &str) -> PathBuf {
    let root = advanced_language_root(root, language);
    match language {
        Language::Go => root.join(example_directory_name(language, id)),
        _ => root.join("wit").join(example_directory_name(language, id)),
    }
}
fn json_example_output_path(
    root: &Path,
    language: Language,
    id: &str,
    mode: GenerationMode,
) -> PathBuf {
    let name = example_directory_name(language, id);
    match mode {
        GenerationMode::DefinitionsOnly if language == Language::Java => {
            samples_language_root(root, language)
                .join("src/main/java/json_schema/definitions")
                .join(name)
        }
        GenerationMode::DefinitionsOnly => samples_language_root(root, language).join(name),
        GenerationMode::NativeApi if language == Language::Java => {
            advanced_language_root(root, language)
                .join("src/main/java/json_schema/api")
                .join(name)
        }
        GenerationMode::NativeApi => advanced_language_root(root, language)
            .join("json_schema/api")
            .join(name),
    }
}
fn json_example_java_package(name: &str, mode: GenerationMode) -> String {
    format!(
        "json_schema.{}.{name}",
        match mode {
            GenerationMode::DefinitionsOnly => "definitions",
            GenerationMode::NativeApi => "api",
        }
    )
}
fn example_directory_name(language: Language, id: &str) -> String {
    match language {
        Language::Go => id
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect::<String>()
            .to_lowercase(),
        Language::Python => id.to_snake_case(),
        _ => id.to_string(),
    }
}

fn format_example_output(
    language_root: &Path,
    language: Language,
    output_path: &Path,
) -> Result<()> {
    let (program, args): (&str, Vec<String>) = match language {
        Language::Dotnet | Language::Java => return Ok(()),
        Language::Go => (
            "gofmt",
            vec!["-w".into(), output_path.to_string_lossy().into_owned()],
        ),
        Language::Python => (
            "uv",
            vec![
                "run".into(),
                "ruff".into(),
                "format".into(),
                "--line-length".into(),
                "88".into(),
                "--config".into(),
                "pyproject.toml".into(),
                output_path.to_string_lossy().into_owned(),
            ],
        ),
        Language::TypeScript => (
            "npm",
            vec![
                "exec".into(),
                "--".into(),
                "prettier".into(),
                "--write".into(),
                "--print-width".into(),
                "88".into(),
                output_path.to_string_lossy().into_owned(),
            ],
        ),
        language => return Err(Error::UnsupportedLanguage { language }),
    };
    let command = format_command(program, &args);
    let status = ProcessCommand::new(program)
        .current_dir(language_root)
        .args(&args)
        .status()
        .map_err(|source| Error::RunCommand {
            cwd: language_root.to_path_buf(),
            command: command.clone(),
            source,
        })?;
    if !status.success() {
        return Err(Error::CommandFailed {
            cwd: language_root.to_path_buf(),
            command,
            status,
        });
    }
    Ok(())
}
fn format_command(program: &str, args: &[String]) -> String {
    std::iter::once(program)
        .chain(args.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Replaces the release-specific version in generated example headers so a
/// release bump does not create unrelated diffs across checked-in examples.
fn normalize_example_headers(output_path: &Path) -> Result<()> {
    for entry in fs::read_dir(output_path).map_err(|source| Error::ReadFile {
        path: output_path.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| Error::ReadFile {
            path: output_path.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|source| Error::ReadFile {
            path: path.clone(),
            source,
        })?;
        if file_type.is_dir() {
            normalize_example_headers(&path)?;
        } else if file_type.is_file() {
            normalize_example_header(&path)?;
        }
    }
    Ok(())
}

fn normalize_example_header(path: &Path) -> Result<()> {
    let contents = fs::read_to_string(path).map_err(|source| Error::ReadFile {
        path: path.to_path_buf(),
        source,
    })?;
    let Some(version_start) = contents.find("nexgen v") else {
        return Ok(());
    };
    let version_number_start = version_start + "nexgen v".len();
    let Some(version_end) = contents[version_number_start..].find(". DO NOT EDIT") else {
        return Ok(());
    };
    let version_end = version_number_start + version_end;
    let mut normalized = contents;
    normalized.replace_range(version_start..version_end, "nexgen latest");
    fs::write(path, normalized).map_err(|source| Error::WriteFile {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn temp_language_root(label: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "nexgen-{label}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }
    #[test]
    fn reset_example_output_directory_clears_the_example_directory() {
        let root = temp_language_root("reset-example-output");
        let output = root.join("chat");
        fs::create_dir_all(output.join("nested")).unwrap();
        fs::write(output.join("stale.go"), "stale\n").unwrap();
        fs::write(root.join("go.mod"), "module sample\n").unwrap();
        reset_example_output_directory(&root, &output).unwrap();
        assert!(!output.exists());
        assert!(root.join("go.mod").is_file());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn reset_example_output_directory_refuses_the_language_root_itself() {
        let root = temp_language_root("reset-example-output-root");
        assert!(matches!(
            reset_example_output_directory(&root, &root),
            Err(Error::ExampleOutputPathOutsideRoot { .. })
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn discovers_json_schema_example_ids_from_sample_file_names() {
        assert_eq!(
            discover_json_example_ids(&repo_root()).unwrap(),
            ["chat", "kb", "showcase", "temporal"]
        );
    }

    #[test]
    fn normalizes_generated_headers_recursively() {
        let root = temp_language_root("normalize-example-headers");
        let nested = root.join("nested");
        fs::create_dir_all(&nested).unwrap();
        let go_file = root.join("example.go");
        let python_file = nested.join("example.py");
        let untouched_file = root.join("README.md");
        fs::write(
            &go_file,
            "// Code generated by nexgen v0.2.3. DO NOT EDIT.\npackage example\n",
        )
        .unwrap();
        fs::write(
            &python_file,
            "# Generated by nexgen v99.1.0. DO NOT EDIT!\nvalue = 1\n",
        )
        .unwrap();
        fs::write(
            &untouched_file,
            "nexgen v0.2.3 without a generated header\n",
        )
        .unwrap();

        normalize_example_headers(&root).unwrap();

        assert_eq!(
            fs::read_to_string(go_file).unwrap(),
            "// Code generated by nexgen latest. DO NOT EDIT.\npackage example\n"
        );
        assert_eq!(
            fs::read_to_string(python_file).unwrap(),
            "# Generated by nexgen latest. DO NOT EDIT!\nvalue = 1\n"
        );
        assert_eq!(
            fs::read_to_string(untouched_file).unwrap(),
            "nexgen v0.2.3 without a generated header\n"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
