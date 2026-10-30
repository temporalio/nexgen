use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

use heck::{ToSnakeCase, ToUpperCamelCase};
use nexgen::error::Error as NexgenError;
use nexgen::generator::TsDateTimeTypes;
use nexgen::language::Language;
use nexgen::nexgen_config::{GenerationTargets, NexgenConfig};
use nexgen::{GenerateRequest, format_generated_files, generate_to_file};
use serde::Deserialize;

use crate::error::{Error, Result};

#[derive(Default)]
struct ExamplesConfig {
    examples: std::collections::BTreeMap<String, ExampleConfig>,
}

#[derive(Clone, Default)]
struct ExampleConfig {
    options: ExampleOptions,
    children: std::collections::BTreeMap<String, ExampleConfig>,
}

#[derive(Clone, Default, Deserialize)]
struct ExampleOptions {
    languages: Option<Vec<String>>,
    system_nexus: Option<bool>,
    targets: Option<Vec<ExampleTarget>>,
    // Each example plans against only the protos it declares, so one sample's protos can't
    // change how another example resolves a proto name.
    descriptors: Option<ExampleDescriptors>,
}

/// Opt-in, as in the compiler rejection fixtures. An example with no `descriptors` gets none.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
enum ExampleDescriptors {
    Preset(DescriptorPreset),
    /// File names under `advanced/samples/descriptors/`.
    Paths(Vec<PathBuf>),
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum DescriptorPreset {
    Temporal,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum ExampleTarget {
    Workflow,
    Client,
}

/// The keys of a table that are settings. Any other key must be a child table.
const OPTION_KEYS: [&str; 4] = ["languages", "system_nexus", "targets", "descriptors"];

impl ExamplesConfig {
    fn from_table(table: toml::Table) -> std::result::Result<Self, String> {
        let mut examples = std::collections::BTreeMap::new();
        for (name, value) in table {
            let toml::Value::Table(section) = value else {
                return Err(format!(
                    "unknown setting `{name}` outside an example section"
                ));
            };
            let example = ExampleConfig::from_table(&name, section, true)?;
            examples.insert(name, example);
        }
        Ok(Self { examples })
    }
}

impl ExampleConfig {
    // Splitting the keys by hand keeps a misspelled setting from becoming a child
    // configuration, which would silently stop the parent from generating.
    fn from_table(
        path: &str,
        table: toml::Table,
        allow_children: bool,
    ) -> std::result::Result<Self, String> {
        let mut options = toml::Table::new();
        let mut children = std::collections::BTreeMap::new();
        for (key, value) in table {
            if OPTION_KEYS.contains(&key.as_str()) {
                options.insert(key, value);
                continue;
            }
            let toml::Value::Table(child) = value else {
                return Err(format!("unknown setting `{key}` in `[{path}]`"));
            };
            if !allow_children {
                return Err(format!(
                    "`{path}.{key}`: child configurations cannot contain further \
                     child configurations"
                ));
            }
            let child_path = format!("{path}.{key}");
            children.insert(key, Self::from_table(&child_path, child, false)?);
        }
        let options = toml::Value::Table(options)
            .try_into()
            .map_err(|error: toml::de::Error| format!("`[{path}]`: {}", error.message()))?;
        Ok(Self { options, children })
    }

    fn configurations(&self, example_id: &str) -> Vec<(PathBuf, ExampleOptions)> {
        if self.children.is_empty() {
            return vec![(PathBuf::from(example_id), self.options.clone())];
        }
        self.children
            .iter()
            .map(|(name, child)| {
                (
                    Path::new(example_id).join(name),
                    self.options.overridden_by(&child.options),
                )
            })
            .collect()
    }
}

impl ExampleOptions {
    fn overridden_by(&self, child: &Self) -> Self {
        Self {
            languages: child.languages.clone().or_else(|| self.languages.clone()),
            system_nexus: child.system_nexus.or(self.system_nexus),
            targets: child.targets.clone().or_else(|| self.targets.clone()),
            descriptors: child
                .descriptors
                .clone()
                .or_else(|| self.descriptors.clone()),
        }
    }

    fn targets(&self) -> GenerationTargets {
        let Some(targets) = &self.targets else {
            return GenerationTargets::NONE;
        };
        GenerationTargets {
            workflow: targets.contains(&ExampleTarget::Workflow),
            client: targets.contains(&ExampleTarget::Client),
        }
    }

    fn supports(&self, language: Language) -> bool {
        self.languages
            .as_ref()
            .is_none_or(|languages| languages.iter().any(|name| name == language.as_str()))
    }
}

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
                    Language::Dotnet
                        | Language::Python
                        | Language::TypeScript
                        | Language::Go
                        | Language::Java
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
            Language::Dotnet,
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
            Language::Dotnet
                | Language::Python
                | Language::TypeScript
                | Language::Go
                | Language::Java
        ) {
            return Err(NexgenError::UnsupportedLanguage { language }.into());
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
    let config = load_examples_config(&input_root)?;
    let mut input_paths = Vec::new();
    discover_example_input_paths(&input_root, &mut input_paths)?;
    // An example without a section generates only definitions. Without this check, a
    // misspelled section name would drop the generated API and give no error.
    let input_ids = input_paths
        .iter()
        .filter_map(|path| example_id_for_input(path))
        .collect::<Vec<_>>();
    if let Some(name) = config
        .examples
        .keys()
        .find(|name| !input_ids.contains(name))
    {
        return Err(Error::InvalidExampleConfig {
            path: input_root.join("examples.toml"),
            reason: format!("section `[{name}]` matches no `.wit` input file"),
        });
    }
    let mut ids = input_paths
        .into_iter()
        .filter_map(|path| {
            let id = example_id_for_input(&path)?;
            config
                .examples
                .get(&id)
                .is_none_or(|example| {
                    example
                        .configurations(&id)
                        .iter()
                        .any(|(_, options)| options.supports(language))
                })
                .then_some(id)
        })
        .collect::<Vec<_>>();
    ids.sort();
    Ok(ids)
}

fn load_examples_config(input_root: &Path) -> Result<ExamplesConfig> {
    let path = input_root.join("examples.toml");
    let contents = fs::read_to_string(&path).map_err(|source| NexgenError::ReadFile {
        path: path.clone(),
        source,
    })?;
    parse_examples_config(&contents).map_err(|reason| Error::InvalidExampleConfig { path, reason })
}

fn parse_examples_config(contents: &str) -> std::result::Result<ExamplesConfig, String> {
    let table: toml::Table = toml::from_str(contents).map_err(|error| error.to_string())?;
    let config = ExamplesConfig::from_table(table)?;
    for (name, example) in &config.examples {
        validate_child_names(name, example)?;
    }
    Ok(config)
}

/// Words that cannot name a Python package.
const PYTHON_KEYWORDS: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue",
    "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import",
    "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while",
    "with", "yield",
];

fn validate_child_names(name: &str, example: &ExampleConfig) -> std::result::Result<(), String> {
    for child_name in example.children.keys() {
        if !child_name.starts_with(|c: char| c.is_ascii_alphabetic())
            || !child_name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(format!(
                "`{name}.{child_name}`: a child configuration name must start with a letter \
                 and may use letters, numbers, '-' or '_'"
            ));
        }
        let package = example_directory_name(Language::Python, child_name);
        if PYTHON_KEYWORDS.contains(&package.as_str()) {
            return Err(format!(
                "`{name}.{child_name}`: `{package}` is a Python keyword and cannot name a package"
            ));
        }
    }
    // Each child writes into a directory named for the language, and the example output
    // directory is cleared only once. Two children in one directory would merge their files.
    for language in [
        Language::Dotnet,
        Language::Python,
        Language::TypeScript,
        Language::Go,
        Language::Java,
    ] {
        let mut directories = std::collections::BTreeMap::new();
        for child_name in example.children.keys() {
            let directory = example_directory_name(language, child_name);
            if let Some(other) = directories.insert(directory.clone(), child_name) {
                return Err(format!(
                    "`{name}`: child configurations `{other}` and `{child_name}` both write to \
                     the {language} directory `{directory}`"
                ));
            }
        }
    }
    Ok(())
}

fn example_descriptor_paths(repo_root: &Path, options: &ExampleOptions) -> Result<Vec<PathBuf>> {
    let root = repo_root.join("advanced/samples/descriptors");
    let names = match options.descriptors.as_ref() {
        None => Vec::new(),
        Some(ExampleDescriptors::Preset(DescriptorPreset::Temporal)) => {
            vec![PathBuf::from("temporal_api.bin")]
        }
        Some(ExampleDescriptors::Paths(paths)) => paths.clone(),
    };
    names
        .into_iter()
        .map(|name| {
            let path = root.join(name);
            if path.is_file() {
                Ok(path)
            } else {
                Err(Error::InvalidExampleConfig {
                    path: repo_root.join("advanced/samples/inputs/examples.toml"),
                    reason: format!("descriptor set `{}` does not exist", path.display()),
                })
            }
        })
        .collect()
}

fn discover_example_input_paths(path: &Path, inputs: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(path).map_err(|source| NexgenError::ReadFile {
        path: path.to_path_buf(),
        source,
    })? {
        let path = entry
            .map_err(|source| NexgenError::ReadFile {
                path: path.to_path_buf(),
                source,
            })?
            .path();
        if path.is_dir() {
            if path.file_name().is_some_and(|name| name == "deps") {
                continue;
            }
            discover_example_input_paths(&path, inputs)?;
        } else if path.extension().is_some_and(|extension| extension == "wit") {
            inputs.push(path);
        }
    }
    Ok(())
}

fn example_id_for_input(input_path: &Path) -> Option<String> {
    Some(input_path.file_stem()?.to_string_lossy().into_owned())
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
        .map_err(|source| NexgenError::ReadFile { path: root, source })?
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
    let command = "npm ci --no-fund --no-audit".to_string();
    let status = ProcessCommand::new("npm")
        .current_dir(cwd)
        .args(["ci", "--no-fund", "--no-audit"])
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
        Err(source) => Err(NexgenError::WriteFile {
            path: output_path.to_path_buf(),
            source,
        }
        .into()),
    }
}

fn build_example(repo_root: &Path, language: Language, example_id: &str) -> Result<()> {
    let config = load_examples_config(&repo_root.join("advanced/samples/inputs"))?;
    let example = config.examples.get(example_id).cloned().unwrap_or_default();
    let configurations = example
        .configurations(example_id)
        .into_iter()
        .filter(|(_, options)| options.supports(language))
        .collect::<Vec<_>>();
    if configurations.is_empty() {
        return Ok(());
    }
    let language_root = advanced_language_root(repo_root, language);
    let output_root = example_output_path(repo_root, language, Path::new(example_id));
    reset_example_output_directory(&language_root, &output_root)?;
    for (output_id, options) in configurations {
        build_example_configuration(repo_root, language, example_id, &output_id, &options)?;
    }
    Ok(())
}

fn build_example_configuration(
    repo_root: &Path,
    language: Language,
    example_id: &str,
    output_id: &Path,
    options: &ExampleOptions,
) -> Result<()> {
    let input_path = example_input_path(repo_root, example_id);
    let mut input_paths = vec![input_path.clone()];
    input_paths.extend(example_linked_input_paths(repo_root, &input_path)?);
    let output_path = example_output_path(repo_root, language, output_id);
    let language_root = advanced_language_root(repo_root, language);
    let system_nexus = options.system_nexus.unwrap_or_default();
    let generate_request = GenerateRequest {
        config: NexgenConfig {
            system_nexus,
            targets: options.targets(),
        },
        language,
        input_paths,
        support_package: if language == Language::Go && system_nexus {
            Some("go.temporal.io/sdk/advanced/samples/go/systemsupport".into())
        } else {
            sample_support_package(repo_root, language, &output_path)
        },
        descriptor_paths: example_descriptor_paths(repo_root, options)?,
        output_path: output_path.clone(),
        format: false,
        java_package_name: (language == Language::Java).then(|| example_java_package(output_id)),
        ts_date_time_types: Default::default(),
        dotnet_namespace: None,
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
    for (targets, root) in [
        (
            GenerationTargets::NONE,
            samples_language_root(repo_root, language),
        ),
        (
            GenerationTargets::WORKFLOW,
            advanced_language_root(repo_root, language),
        ),
    ] {
        let output_path = json_example_output_path(repo_root, language, output_id, targets);
        reset_example_output_directory(&root, &output_path)?;
        generate_to_file(&GenerateRequest {
            config: NexgenConfig {
                targets,
                ..Default::default()
            },
            language,
            input_paths: vec![input_path.clone()],
            support_package: None,
            descriptor_paths: Vec::new(),
            output_path: output_path.clone(),
            format: false,
            java_package_name: (language == Language::Java)
                .then(|| json_example_java_package(&dir_name, targets)),
            ts_date_time_types,
            dotnet_namespace: (language == Language::Dotnet)
                .then(|| json_example_csharp_namespace(output_id)),
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

fn sample_support_package(
    repo_root: &Path,
    language: Language,
    output_path: &Path,
) -> Option<String> {
    match language {
        Language::Go => Some("go.temporal.io/sdk/advanced/samples/go/support".into()),
        Language::Python => Some("temporal_support".into()),
        Language::Dotnet => Some("Nexgen.Support".into()),
        Language::TypeScript => {
            let support_module =
                repo_root.join("advanced/samples/typescript/support/temporal_model_converters");
            let shared = output_path
                .ancestors()
                .find(|ancestor| support_module.starts_with(ancestor))
                .expect("sample paths share the repository root");
            let levels = output_path
                .strip_prefix(shared)
                .expect("shared ancestor is a prefix")
                .components()
                .count();
            let relative = std::iter::repeat_n("..", levels).collect::<PathBuf>().join(
                support_module
                    .strip_prefix(shared)
                    .expect("shared ancestor is a prefix"),
            );
            Some(relative.to_string_lossy().replace('\\', "/"))
        }
        Language::Java => None,
        _ => None,
    }
}

fn example_input_path(root: &Path, id: &str) -> PathBuf {
    let input_root = root.join("advanced/samples/inputs");
    let mut inputs = Vec::new();
    if discover_example_input_paths(&input_root, &mut inputs).is_ok()
        && let Some(path) = inputs
            .into_iter()
            .find(|path| example_id_for_input(path).is_some_and(|input_id| input_id == id))
    {
        return path;
    }
    input_root.join(format!("{id}.wit"))
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
    let input = fs::read_to_string(input_path).map_err(|source| NexgenError::ReadFile {
        path: input_path.to_path_buf(),
        source,
    })?;
    if !input.contains("use nexus:temporal-types/") {
        return Ok(Vec::new());
    }
    let path = root.join("advanced/samples/inputs/deps");
    Ok(path.is_dir().then_some(path).into_iter().collect())
}
fn example_output_path(root: &Path, language: Language, id: &Path) -> PathBuf {
    let root = advanced_language_root(root, language);
    let directory: PathBuf = id
        .iter()
        .map(|component| {
            example_directory_name(
                language,
                component.to_str().expect("example names are UTF-8"),
            )
        })
        .collect();
    match language {
        Language::Go => root.join(directory),
        _ => root.join("wit").join(directory),
    }
}

fn example_java_package(id: &Path) -> String {
    id.iter()
        .map(|component| {
            example_directory_name(
                Language::Java,
                component.to_str().expect("example names are UTF-8"),
            )
        })
        .collect::<Vec<_>>()
        .join(".")
}
fn json_example_output_path(
    root: &Path,
    language: Language,
    id: &str,
    targets: GenerationTargets,
) -> PathBuf {
    let name = example_directory_name(language, id);
    match (targets.workflow, language) {
        (false, Language::Java) => samples_language_root(root, language)
            .join("src/main/java/json_schema/definitions")
            .join(name),
        (false, _) => samples_language_root(root, language).join(name),
        (true, Language::Java) => advanced_language_root(root, language)
            .join("src/main/java/json_schema/api")
            .join(name),
        (true, _) => advanced_language_root(root, language)
            .join("json_schema/api")
            .join(name),
    }
}
fn json_example_java_package(name: &str, targets: GenerationTargets) -> String {
    format!(
        "json_schema.{}.{name}",
        if targets.workflow {
            "api"
        } else {
            "definitions"
        }
    )
}
/// The root namespace of a C# JSON Schema example. Every example compiles into
/// one sample project, so each needs its own namespace for the shared runtime.
fn json_example_csharp_namespace(id: &str) -> String {
    format!("Nexgen.JsonSchema.{}", id.to_upper_camel_case())
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
        Language::Dotnet => return Ok(()),
        Language::Java => return format_generated_files(language, output_path).map_err(Into::into),
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
        language => return Err(NexgenError::UnsupportedLanguage { language }.into()),
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
    for entry in fs::read_dir(output_path).map_err(|source| NexgenError::ReadFile {
        path: output_path.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| NexgenError::ReadFile {
            path: output_path.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|source| NexgenError::ReadFile {
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
    let contents = fs::read_to_string(path).map_err(|source| NexgenError::ReadFile {
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
    fs::write(path, normalized).map_err(|source| NexgenError::WriteFile {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_configurations_inherit_and_override_all_example_options() {
        let config = parse_examples_config(
            r#"
[sample]
languages = ["python"]
system_nexus = true
targets = ["workflow", "client"]
descriptors = "temporal"

[sample.inherited]

[sample.cleared]
system_nexus = false
targets = []
descriptors = []

[sample.replaced]
languages = ["go"]
targets = ["workflow"]
descriptors = ["streams.bin"]
"#,
        )
        .unwrap();
        let configurations = config.examples["sample"].configurations("sample");
        assert_eq!(
            configurations
                .iter()
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>(),
            [
                PathBuf::from("sample/cleared"),
                PathBuf::from("sample/inherited"),
                PathBuf::from("sample/replaced")
            ]
        );
        let cleared = &configurations[0].1;
        assert_eq!(cleared.system_nexus, Some(false));
        assert_eq!(cleared.targets(), GenerationTargets::NONE);
        assert_eq!(
            cleared.descriptors,
            Some(ExampleDescriptors::Paths(Vec::new()))
        );
        assert!(cleared.supports(Language::Python));
        assert!(!cleared.supports(Language::Go));

        let inherited = &configurations[1].1;
        assert_eq!(inherited.system_nexus, Some(true));
        assert_eq!(inherited.targets(), GenerationTargets::ALL);
        assert_eq!(
            inherited.descriptors,
            Some(ExampleDescriptors::Preset(DescriptorPreset::Temporal))
        );

        let replaced = &configurations[2].1;
        assert_eq!(replaced.system_nexus, Some(true));
        assert_eq!(replaced.targets(), GenerationTargets::WORKFLOW);
        assert_eq!(
            replaced.descriptors,
            Some(ExampleDescriptors::Paths(vec![PathBuf::from(
                "streams.bin"
            )]))
        );
        assert!(replaced.supports(Language::Go));
        assert!(!replaced.supports(Language::Python));
    }

    #[test]
    fn an_example_without_children_keeps_its_output_name() {
        let config = parse_examples_config("[sample]\n").unwrap();
        let configurations = config.examples["sample"].configurations("sample");
        assert_eq!(configurations.len(), 1);
        assert_eq!(configurations[0].0, Path::new("sample"));
        assert_eq!(configurations[0].1.targets(), GenerationTargets::NONE);
        assert!(!configurations[0].1.system_nexus.unwrap_or_default());
        assert!(configurations[0].1.descriptors.is_none());
        assert!(configurations[0].1.supports(Language::Python));
    }

    #[test]
    fn child_output_paths_preserve_hierarchy_and_language_casing() {
        let root = Path::new("repo");
        let configuration = Path::new("some-service/client-only");
        for (language, expected) in [
            (
                Language::Python,
                "advanced/samples/python/wit/some_service/client_only",
            ),
            (
                Language::TypeScript,
                "advanced/samples/typescript/wit/some-service/client-only",
            ),
            (Language::Go, "advanced/samples/go/someservice/clientonly"),
            (
                Language::Dotnet,
                "advanced/samples/dotnet/wit/some-service/client-only",
            ),
            (
                Language::Java,
                "advanced/samples/java/wit/some-service/client-only",
            ),
        ] {
            assert_eq!(
                example_output_path(root, language, configuration),
                root.join(expected)
            );
        }
        let java_configuration = Path::new("someService/clientOnly");
        let package = example_java_package(java_configuration);
        assert_eq!(package, "someService.clientOnly");
        let output = example_output_path(root, Language::Java, java_configuration);
        assert_eq!(
            output.file_name().unwrap().to_str().unwrap(),
            package.rsplit('.').next().unwrap()
        );
    }

    #[test]
    fn discovery_filters_languages_after_resolving_child_options() {
        let root = temp_language_root("child-config-discovery");
        let inputs = root.join("advanced/samples/inputs");
        fs::create_dir_all(&inputs).unwrap();
        fs::write(inputs.join("sample.wit"), "").unwrap();
        fs::write(
            inputs.join("examples.toml"),
            r#"
[sample]
languages = ["python"]

[sample.go]
languages = ["go"]
targets = ["workflow"]
"#,
        )
        .unwrap();
        assert!(
            discover_example_ids(&root, Language::Python)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            discover_example_ids(&root, Language::Go).unwrap(),
            ["sample"]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn configuration_rejects_unknown_settings_and_nested_children() {
        for (source, message) in [
            (
                "[sample]\nsystem_nexuz = true\n",
                "unknown setting `system_nexuz` in `[sample]`",
            ),
            (
                "[sample]\ntargets = [\"workflow\"]\n\n[sample.child]\nsystem_nexuz = true\n",
                "unknown setting `system_nexuz` in `[sample.child]`",
            ),
            (
                "[sample.child]\ntarget = [\"client\"]\n",
                "unknown setting `target` in `[sample.child]`",
            ),
            (
                "system_nexus = true\n",
                "unknown setting `system_nexus` outside an example section",
            ),
            (
                "[sample.child.grandchild]\ntargets = []\n",
                "`sample.child.grandchild`: child configurations cannot contain further",
            ),
            (
                "[sample.\"../outside\"]\ntargets = []\n",
                "`sample.../outside`: a child configuration name must",
            ),
            (
                "[sample.\"__\"]\ntargets = []\n",
                "`sample.__`: a child configuration name must",
            ),
        ] {
            let error = parse_examples_config(source).err().unwrap_or_default();
            assert!(error.contains(message), "{source}\n{error}");
        }
    }

    #[test]
    fn discovery_rejects_a_section_that_matches_no_input() {
        let root = temp_language_root("section-without-input");
        let inputs = root.join("advanced/samples/inputs");
        fs::create_dir_all(&inputs).unwrap();
        fs::write(inputs.join("user-service.wit"), "").unwrap();
        fs::write(
            inputs.join("examples.toml"),
            "[user_service]\ntargets = [\"workflow\"]\n",
        )
        .unwrap();
        let error = discover_example_ids(&root, Language::Python)
            .unwrap_err()
            .to_string();
        assert!(error.contains("examples.toml"), "{error}");
        assert!(
            error.contains("section `[user_service]` matches no `.wit` input file"),
            "{error}"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn configuration_rejects_children_that_share_a_directory_in_any_language() {
        for (first, second, language, directory) in [
            ("client-only", "client_only", "python", "client_only"),
            ("client-only", "ClientOnly", "python", "client_only"),
            ("client_only", "ClientOnly", "python", "client_only"),
            ("a-b", "ab", "go", "ab"),
        ] {
            let source = format!("[sample.\"{first}\"]\n[sample.\"{second}\"]\n");
            let error = parse_examples_config(&source).err().unwrap_or_default();
            let message = format!(
                "`{}` and `{}` both write to the {language} directory `{directory}`",
                first.min(second),
                first.max(second)
            );
            assert!(error.contains(&message), "{source}\n{error}");
        }
        assert!(parse_examples_config("[sample.client]\n[sample.both]\n").is_ok());
    }

    #[test]
    fn configuration_rejects_children_that_cannot_name_a_python_package() {
        for (child, message) in [
            (
                "\"2\"",
                "`sample.2`: a child configuration name must start with a letter",
            ),
            (
                "_hidden",
                "`sample._hidden`: a child configuration name must start with a letter",
            ),
            ("async", "`sample.async`: `async` is a Python keyword"),
            ("Class", "`sample.Class`: `class` is a Python keyword"),
        ] {
            let source = format!("[sample.{child}]\n");
            let error = parse_examples_config(&source).err().unwrap_or_default();
            assert!(error.contains(message), "{source}\n{error}");
        }
    }

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
    fn java_example_formatting_skips_directories_without_java_files() {
        let root = temp_language_root("java-format-empty");
        fs::write(root.join("notes.txt"), "leave alone").unwrap();
        format_example_output(&root, Language::Java, &root).unwrap();
        assert_eq!(
            fs::read_to_string(root.join("notes.txt")).unwrap(),
            "leave alone"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn java_example_formatting_propagates_file_discovery_errors() {
        let root = temp_language_root("java-format-missing");
        let output = root.join("missing");
        assert!(matches!(
            format_example_output(&root, Language::Java, &output),
            Err(Error::Nexgen(NexgenError::ReadFile { path, .. })) if path == output
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
    fn typescript_support_import_is_relative_to_each_generated_output() {
        let root = repo_root();
        for (output, expected) in [
            (
                "advanced/samples/typescript/wit/workflow-service",
                "../../support/temporal_model_converters",
            ),
            (
                "advanced/samples/typescript/wit/example/workflow",
                "../../../support/temporal_model_converters",
            ),
            (
                "advanced/samples/typescript/json_schema/api/chat",
                "../../../support/temporal_model_converters",
            ),
            (
                "samples/typescript/chat",
                "../../../advanced/samples/typescript/support/temporal_model_converters",
            ),
        ] {
            assert_eq!(
                sample_support_package(&root, Language::TypeScript, &root.join(output)),
                Some(expected.to_string()),
            );
        }
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
