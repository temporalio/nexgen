mod planning;

pub mod add_rpc;
pub mod descriptors;
pub mod error;
pub mod generator;
pub mod json_schema;
pub mod language;
pub mod nexgen_config;
pub mod parser;
pub mod spec;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use descriptors::DescriptorIndex;
use error::Result;
use generator::{GenerateFilesOptions, GeneratedFiles, GeneratedOutputLayout, GenerationMode};
use language::Language;
use spec::SupportFragmentSpec;
use spec::{ApiSpecNode, ApiSpecTree, CompilerPass};

pub use add_rpc::{
    AddMessageRequest, AddRpcRequest, add_message_to_file, add_message_to_string, add_rpc_to_file,
    add_rpc_to_string,
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SupportFiles {
    pub fragments: Vec<SupportFragmentSpec>,
}

pub struct GenerateRequest {
    pub config: nexgen_config::NexgenConfig,
    pub language: Language,
    pub input_paths: Vec<PathBuf>,
    pub support_paths: Vec<PathBuf>,
    pub descriptor_paths: Vec<PathBuf>,
    pub output_path: PathBuf,
    pub format: bool,
    /// Java-only: the base package for generated types. Its last
    /// dot-separated segment must match the output directory's name. Ignored
    /// for other languages.
    pub java_package_name: Option<String>,
    /// TypeScript-only: the in-memory representation for materialized temporal
    /// `format` fields. `Default` is `TsDateTimeTypes::String`.
    pub ts_date_time_types: generator::TsDateTimeTypes,
}

pub fn generate_to_file(request: &GenerateRequest) -> Result<()> {
    let _config_scope = nexgen_config::scope(request.config);
    // A resolved output path with no name at all (the filesystem root, or
    // `..` past it) is never a real output directory: Go and Java derive
    // package names from its basename, and for every language it means the
    // caller pointed `--output` somewhere unintended.
    if absolute_output_path(&request.output_path)?
        .file_name()
        .is_none()
    {
        return Err(error::Error::OutputPathIsRoot {
            path: request.output_path.clone(),
        });
    }

    let tree = parser::load_api_spec_tree_for_language_with_inputs(
        request.language,
        &request.input_paths,
    )?;
    let descriptors = DescriptorIndex::load_many(&request.descriptor_paths)?;
    let support = load_support_files_for_tree(request.language, &tree, &request.support_paths)?;
    let options = GenerateFilesOptions {
        go_output_dir_name: if request.language == Language::Go {
            output_dir_name(&request.output_path)?
        } else {
            String::new()
        },
        java_package_root: if request.language == Language::Java {
            Some(resolve_java_package(request)?)
        } else {
            None
        },
        ts_date_time_types: request.ts_date_time_types,
    };
    let generated = compile_tree_to_files(request.language, tree, &descriptors, &support, options)?;
    print_warnings(&generated);

    write_generated_files(&request.output_path, &generated)?;

    if request.format {
        format_generated_file(request.language, &request.output_path)?;
    }

    Ok(())
}

/// The compiler's explicit high-level pipeline. Parsing is deliberately kept
/// at the call site: different input frontends produce the authored tree, then
/// every common compiler pass is visible here in order.
pub(crate) fn compile_tree_to_files(
    language: Language,
    authored_tree: ApiSpecTree,
    descriptors: &DescriptorIndex,
    support: &SupportFiles,
    options: GenerateFilesOptions,
) -> Result<GeneratedFiles> {
    let mode = nexgen_config::current().mode;
    // parse (frontend) -> validate authored intent -> select target metadata
    let authored_tree =
        planning::AuthoredValidationPass::new(descriptors, language).apply(authored_tree)?;
    let selected_tree = planning::LanguageSelectionPass::new(language)
        .apply(authored_tree)
        .expect("language selection is infallible");

    // selected IR -> resource bindings -> operation relationships -> lowered
    // operation results -> planned types -> reachability-pruned planned IR
    let resource_bound_tree = planning::ResourceResolutionPass::new(
        descriptors,
        match mode {
            GenerationMode::NativeApi => planning::PlanningMode::NativeApi,
            GenerationMode::DefinitionsOnly => planning::PlanningMode::DefinitionsOnly,
        },
    )
    .apply(selected_tree)?;
    let operation_bound_tree = planning::OperationBindingPass::new().apply(resource_bound_tree)?;
    let operation_lowered_tree =
        planning::OperationLoweringPass::new().apply(operation_bound_tree)?;
    let type_planning =
        planning::TypePlanningPass::new(&operation_lowered_tree, descriptors, language);
    let planned_tree = type_planning.apply(operation_lowered_tree)?;
    let planned_tree = planning::ReachabilityPass::new().apply(planned_tree)?;

    // planned IR -> emitted JSON names -> render target-language files
    let name_resolution = planning::EmittedNameResolutionPass::new(language, &planned_tree)?;
    let generator_ready_tree = name_resolution.apply(planned_tree)?;
    generator::generate_files_from_planned_tree(language, &generator_ready_tree, support, options)
}

/// The output directory's basename, used as the Go package name — matching
/// the Go convention that a package's name is its directory's name. Resolved
/// against the working directory so relative paths, including `.` and `..`,
/// name the real directory rather than being read as a literal path segment
/// (`Path::file_name` returns `None` for a path that lexically terminates in
/// `.` or `..`).
///
/// `generate_to_file` already rejects an output path that resolves to the
/// filesystem root before this runs, so `OutputPathIsRoot` here is
/// unreachable through that caller; the check exists so this function is
/// correct standing alone. A non-UTF-8 path component is a real, if rare,
/// case: reported as an error rather than guessed at.
fn output_dir_name(output_path: &Path) -> Result<String> {
    let resolved = absolute_output_path(output_path)?;
    let name = resolved
        .file_name()
        .ok_or_else(|| error::Error::OutputPathIsRoot {
            path: output_path.to_path_buf(),
        })?;
    name.to_str()
        .map(str::to_string)
        .ok_or_else(|| error::Error::OutputPathNotUtf8 {
            path: output_path.to_path_buf(),
        })
}

/// Resolves `output_path` to an absolute, lexically normalized path: relative
/// paths are joined onto the working directory, then `.`/`..` components are
/// collapsed without touching the filesystem (the output directory may not
/// exist yet, so this can't just `fs::canonicalize`).
fn absolute_output_path(output_path: &Path) -> Result<PathBuf> {
    let absolute = if output_path.is_absolute() {
        output_path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|source| error::Error::ReadFile {
                path: PathBuf::from("."),
                source,
            })?
            .join(output_path)
    };
    Ok(normalize_path(&absolute))
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir
                if matches!(normalized.last(), Some(std::path::Component::Normal(_))) =>
            {
                normalized.pop();
            }
            other => normalized.push(other),
        }
    }
    normalized.into_iter().collect()
}

/// Resolves the Java base package from the request's `--package-name`,
/// enforcing that its last dot-separated segment matches the output
/// directory's name — the Java convention that a package's leaf segment is
/// its directory. `--package-name` is required for Java.
fn resolve_java_package(request: &GenerateRequest) -> Result<String> {
    let package_name = request
        .java_package_name
        .as_deref()
        .ok_or(error::Error::JavaPackageNameMissing)?;
    let output_dir_name = output_dir_name(&request.output_path)?;
    let last_segment = package_name.rsplit('.').next().unwrap_or(package_name);
    if last_segment != output_dir_name {
        return Err(error::Error::JavaPackageNameMismatch {
            package_name: package_name.to_string(),
            last_segment: last_segment.to_string(),
            output_dir_name,
        });
    }
    Ok(package_name.to_string())
}

fn format_generated_file(language: Language, output_path: &Path) -> Result<()> {
    if language == Language::Java {
        // No formatter dependency is required for Java; skip formatting.
        return Ok(());
    }
    let (program, args) = formatter_command(language, output_path)?;
    let command = format_formatter_command(program, &args);
    let status = Command::new(program)
        .args(&args)
        .status()
        .map_err(|source| error::Error::RunFormatter {
            path: output_path.to_path_buf(),
            command: command.clone(),
            source,
        })?;

    if !status.success() {
        return Err(error::Error::FormatterFailed {
            path: output_path.to_path_buf(),
            command,
            status,
        });
    }

    Ok(())
}

fn print_warnings(generated: &GeneratedFiles) {
    for warning in &generated.warnings {
        eprintln!("warning: {warning}");
    }
}

/// Writes the generated files into `output_path`, creating directories as
/// needed. Nothing already on disk is deleted: an existing output directory is
/// written into rather than replaced, so hand-written files living alongside
/// generated ones survive. Generated files themselves are overwritten in
/// place, which also means a file left over from a previous run whose source
/// definition has since been renamed or removed stays until it is deleted.
fn write_generated_files(output_path: &Path, generated: &GeneratedFiles) -> Result<()> {
    match generated.layout {
        GeneratedOutputLayout::SingleFile => {
            if let Some(parent) = output_path.parent() {
                fs::create_dir_all(parent).map_err(|source| error::Error::WriteFile {
                    path: parent.to_path_buf(),
                    source,
                })?;
            }
            fs::write(
                output_path,
                generated
                    .single_file_contents()
                    .expect("single-file output should contain one file"),
            )
            .map_err(|source| error::Error::WriteFile {
                path: output_path.to_path_buf(),
                source,
            })?;
        }
        GeneratedOutputLayout::Directory => {
            if output_path.is_file() {
                return Err(error::Error::OutputPathExists {
                    path: output_path.to_path_buf(),
                });
            }
            fs::create_dir_all(output_path).map_err(|source| error::Error::WriteFile {
                path: output_path.to_path_buf(),
                source,
            })?;

            for (relative_path, contents) in &generated.files {
                let path = output_path.join(relative_path);
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent).map_err(|source| error::Error::WriteFile {
                        path: parent.to_path_buf(),
                        source,
                    })?;
                }
                fs::write(&path, contents).map_err(|source| error::Error::WriteFile {
                    path: path.clone(),
                    source,
                })?;
            }
        }
    }

    Ok(())
}

fn formatter_command(
    language: Language,
    output_path: &Path,
) -> Result<(&'static str, Vec<String>)> {
    let output_path = output_path.to_string_lossy().into_owned();
    match language {
        Language::Go => Ok(("gofmt", vec!["-w".to_string(), output_path])),
        Language::Python => Ok((
            "ruff",
            vec![
                "format".to_string(),
                "--line-length".to_string(),
                "88".to_string(),
                output_path,
            ],
        )),
        // Let the generated package choose its own Prettier configuration. In
        // particular, the TypeScript SDK uses Prettier's default width while
        // Nexgen's standalone examples configure a wider width themselves.
        Language::TypeScript => Ok(("prettier", vec!["--write".to_string(), output_path])),
        _ => Err(error::Error::UnsupportedLanguage { language }),
    }
}

fn format_formatter_command(program: &str, args: &[String]) -> String {
    std::iter::once(program)
        .chain(args.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ")
}

fn load_support_files_for_tree(
    language: Language,
    tree: &ApiSpecTree,
    support_paths: &[PathBuf],
) -> Result<SupportFiles> {
    if !support_paths.is_empty() {
        return Ok(SupportFiles {
            fragments: load_support_fragments_from_paths(language, support_paths)?,
        });
    }
    let mut fragments = Vec::new();
    collect_tree_support_fragments(language, &tree.root, &mut fragments);
    Ok(SupportFiles { fragments })
}

fn collect_tree_support_fragments(
    language: Language,
    node: &ApiSpecNode,
    fragments: &mut Vec<SupportFragmentSpec>,
) {
    match node {
        ApiSpecNode::Leaf(leaf) => {
            fragments.extend(leaf.spec.support.fragments_for_language(language).to_vec());
        }
        ApiSpecNode::Branch(branch) => {
            for child in branch.children.values() {
                collect_tree_support_fragments(language, child, fragments);
            }
        }
    }
}

fn load_support_fragments_from_paths(
    language: Language,
    support_paths: &[PathBuf],
) -> Result<Vec<SupportFragmentSpec>> {
    support_paths
        .iter()
        .map(|path| {
            let contents = fs::read_to_string(path).map_err(|source| error::Error::ReadFile {
                path: path.clone(),
                source,
            })?;
            Ok(SupportFragmentSpec {
                path: path.to_string_lossy().replace('\\', "/"),
                namespace: infer_support_namespace(language, &contents),
                contents,
            })
        })
        .collect()
}

fn infer_support_namespace(language: Language, contents: &str) -> Option<String> {
    match language {
        Language::Dotnet => infer_dotnet_namespace(contents),
        _ => None,
    }
}

fn infer_dotnet_namespace(contents: &str) -> Option<String> {
    for line in contents.lines() {
        let line = line.trim_start();
        if line.starts_with("//") {
            continue;
        }
        let mut parts = line.split_whitespace();
        if parts.next() != Some("namespace") {
            continue;
        }
        let namespace = parts
            .next()
            .unwrap_or("")
            .trim_end_matches(|character| character == '{' || character == ';');
        if !namespace.is_empty() {
            return Some(namespace.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::path::PathBuf;

    use super::{
        GenerateRequest, format_formatter_command, formatter_command, infer_dotnet_namespace,
        output_dir_name, resolve_java_package,
    };
    use crate::language::Language;

    fn java_request(output_path: &str, java_package_name: Option<&str>) -> GenerateRequest {
        GenerateRequest {
            config: Default::default(),
            language: Language::Java,
            input_paths: Vec::new(),
            support_paths: Vec::new(),
            descriptor_paths: Vec::new(),
            output_path: PathBuf::from(output_path),
            format: false,
            java_package_name: java_package_name.map(str::to_string),
            ts_date_time_types: Default::default(),
        }
    }

    #[test]
    fn chooses_python_formatter_command() {
        let (program, args) = formatter_command(Language::Python, Path::new("output")).unwrap();
        assert_eq!(program, "ruff");
        assert_eq!(args, vec!["format", "--line-length", "88", "output"]);
        assert_eq!(
            format_formatter_command(program, &args),
            "ruff format --line-length 88 output"
        );
    }

    #[test]
    fn chooses_typescript_formatter_command() {
        let (program, args) = formatter_command(Language::TypeScript, Path::new("output")).unwrap();
        assert_eq!(program, "prettier");
        assert_eq!(args, vec!["--write", "output"]);
        assert_eq!(format_formatter_command(program, &args), "prettier --write output");
    }

    #[test]
    fn output_dir_name_resolves_plain_absolute_path() {
        assert_eq!(output_dir_name(Path::new("/tmp/foo/bar")).unwrap(), "bar");
    }

    #[test]
    fn output_dir_name_collapses_trailing_current_dir() {
        // `Path::file_name` alone returns `None` for a path that lexically
        // terminates in `.`; normalization must resolve it to the real
        // directory name instead.
        assert_eq!(output_dir_name(Path::new("/tmp/foo/bar/.")).unwrap(), "bar");
    }

    #[test]
    fn output_dir_name_collapses_trailing_parent_dir() {
        // Same for a path that lexically terminates in `..`: it must resolve
        // to the parent directory's real name, not `None`.
        assert_eq!(
            output_dir_name(Path::new("/tmp/foo/bar/..")).unwrap(),
            "foo"
        );
    }

    #[test]
    fn output_dir_name_collapses_internal_parent_dir() {
        assert_eq!(
            output_dir_name(Path::new("/tmp/foo/../bar")).unwrap(),
            "bar"
        );
    }

    #[test]
    fn output_dir_name_rejects_filesystem_root() {
        // `generate_to_file` also rejects a root output path up front, but
        // this function must be correct standing alone rather than relying
        // on that caller.
        assert!(matches!(
            output_dir_name(Path::new("/")).unwrap_err(),
            crate::error::Error::OutputPathIsRoot { path } if path == Path::new("/")
        ));
    }

    #[test]
    fn infers_dotnet_support_namespace_from_file() {
        assert_eq!(
            infer_dotnet_namespace(
                r#"
using System;

namespace Nexgen.Support
{
    internal static class TemporalSupport { }
}
"#
            )
            .as_deref(),
            Some("Nexgen.Support")
        );
        assert_eq!(
            infer_dotnet_namespace("namespace Nexgen.Support;\ninternal static class Support { }")
                .as_deref(),
            Some("Nexgen.Support")
        );
    }

    #[test]
    fn resolve_java_package_accepts_matching_last_segment() {
        let request = java_request("/tmp/foo/chat", Some("json_schema.definitions.chat"));
        assert_eq!(
            resolve_java_package(&request).unwrap(),
            "json_schema.definitions.chat"
        );
    }

    #[test]
    fn resolve_java_package_accepts_single_segment_matching_directory() {
        let request = java_request("/tmp/foo/chat", Some("chat"));
        assert_eq!(resolve_java_package(&request).unwrap(), "chat");
    }

    #[test]
    fn resolve_java_package_rejects_mismatched_last_segment() {
        let request = java_request("/tmp/foo/chat", Some("com.example.wrong"));
        assert!(matches!(
            resolve_java_package(&request).unwrap_err(),
            crate::error::Error::JavaPackageNameMismatch {
                package_name,
                last_segment,
                output_dir_name,
            } if package_name == "com.example.wrong"
                && last_segment == "wrong"
                && output_dir_name == "chat"
        ));
    }

    #[test]
    fn resolve_java_package_requires_package_name() {
        let request = java_request("/tmp/foo/chat", None);
        assert!(matches!(
            resolve_java_package(&request).unwrap_err(),
            crate::error::Error::JavaPackageNameMissing
        ));
    }
}
