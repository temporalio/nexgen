use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub(crate) mod dotnet;
pub(crate) mod go;
pub(crate) mod java;
pub(crate) mod json_schema;
pub(crate) mod proto;
pub(crate) mod python;
mod resource_plan;
pub(crate) mod typescript;

use crate::SupportFiles;
use crate::descriptors::DescriptorIndex;
use crate::error::{Error, Result};
use crate::language::Language;
use crate::planning::{PlannedFamily, PlannedSpec, PlannedType};
use crate::spec::{ApiSpec, RecordSpec};
use crate::spec::{ApiSpecNode, ApiSpecTree};

pub(crate) use resource_plan::render_request_plan;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeneratedOutputLayout {
    SingleFile,
    Directory,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedFiles {
    pub layout: GeneratedOutputLayout,
    pub files: BTreeMap<PathBuf, String>,
    pub warnings: Vec<String>,
}

impl GeneratedFiles {
    pub fn single_file(contents: String) -> Self {
        let mut files = BTreeMap::new();
        files.insert(PathBuf::from("output"), contents);
        Self {
            layout: GeneratedOutputLayout::SingleFile,
            files,
            warnings: Vec::new(),
        }
    }

    pub fn directory(files: BTreeMap<PathBuf, String>) -> Self {
        Self {
            layout: GeneratedOutputLayout::Directory,
            files,
            warnings: Vec::new(),
        }
    }

    pub fn single_file_contents(&self) -> Option<&str> {
        (self.layout == GeneratedOutputLayout::SingleFile)
            .then(|| self.files.values().next().map(String::as_str))
            .flatten()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GeneratedFile {
    contents: String,
    origin: GeneratedFileOrigin,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GeneratedFileOrigin {
    InputModule {
        language: Language,
        source: PathBuf,
    },
    OutputDirectoryNamedModule {
        language: Language,
        output_dir_name: String,
    },
    SupportFragment {
        language: Language,
        source: PathBuf,
    },
    Operation {
        language: Language,
        service: String,
        operation: String,
    },
    Resource {
        language: Language,
        service: String,
        resource: String,
    },
    TypeDeclaration {
        language: Language,
        source: PathBuf,
        declaration: String,
    },
    ServiceDeclaration {
        language: Language,
        source: PathBuf,
        service: String,
    },
    FixedArtifact {
        description: String,
    },
}

impl GeneratedFileOrigin {
    pub(crate) fn input_module(language: Language, source: impl Into<PathBuf>) -> Self {
        Self::InputModule {
            language,
            source: source.into(),
        }
    }

    pub(crate) fn output_directory_named_module(
        language: Language,
        output_dir_name: impl Into<String>,
    ) -> Self {
        Self::OutputDirectoryNamedModule {
            language,
            output_dir_name: output_dir_name.into(),
        }
    }

    pub(crate) fn support_fragment(language: Language, source: impl Into<PathBuf>) -> Self {
        Self::SupportFragment {
            language,
            source: source.into(),
        }
    }

    pub(crate) fn operation(
        language: Language,
        service: impl Into<String>,
        operation: impl Into<String>,
    ) -> Self {
        Self::Operation {
            language,
            service: service.into(),
            operation: operation.into(),
        }
    }

    pub(crate) fn resource(
        language: Language,
        service: impl Into<String>,
        resource: impl Into<String>,
    ) -> Self {
        Self::Resource {
            language,
            service: service.into(),
            resource: resource.into(),
        }
    }

    pub(crate) fn type_declaration(
        language: Language,
        source: impl Into<PathBuf>,
        declaration: impl Into<String>,
    ) -> Self {
        Self::TypeDeclaration {
            language,
            source: source.into(),
            declaration: declaration.into(),
        }
    }

    pub(crate) fn service_declaration(
        language: Language,
        source: impl Into<PathBuf>,
        service: impl Into<String>,
    ) -> Self {
        Self::ServiceDeclaration {
            language,
            source: source.into(),
            service: service.into(),
        }
    }

    pub(crate) fn fixed(description: impl Into<String>) -> Self {
        Self::FixedArtifact {
            description: description.into(),
        }
    }

    fn description(&self) -> String {
        match self {
            Self::InputModule { language, source } => format!(
                "{} input module `{}`",
                language_display_name(*language),
                source.display()
            ),
            Self::OutputDirectoryNamedModule {
                language,
                output_dir_name,
            } => format!(
                "{} module derived from output directory `{output_dir_name}`",
                language_display_name(*language)
            ),
            Self::SupportFragment { language, source } => format!(
                "{} support file `{}`",
                language_display_name(*language),
                source.display()
            ),
            Self::Operation {
                language,
                service,
                operation,
            } => format!(
                "{} operation `{service}.{operation}`",
                language_display_name(*language)
            ),
            Self::Resource {
                language,
                service,
                resource,
            } => format!(
                "{} resource `{service}.{resource}`",
                language_display_name(*language)
            ),
            Self::TypeDeclaration {
                language,
                source,
                declaration,
            } => format!(
                "{} type declaration `{declaration}` in `{}`",
                language_display_name(*language),
                source.display()
            ),
            Self::ServiceDeclaration {
                language,
                source,
                service,
            } => format!(
                "{} service declaration `{service}` in `{}`",
                language_display_name(*language),
                source.display()
            ),
            Self::FixedArtifact { description } => description.clone(),
        }
    }

    fn path_changing_action(&self) -> Option<String> {
        match self {
            Self::InputModule { language, source } => Some(format!(
                "rename input file or directory `{}` so it generates a different {} path",
                source.display(),
                language_display_name(*language)
            )),
            Self::OutputDirectoryNamedModule { language, .. } => Some(format!(
                "point `--output` at a directory whose name generates a different {} file path",
                language_display_name(*language)
            )),
            Self::SupportFragment { language, source } => Some(format!(
                "rename support file `{}` so it generates a different {} path",
                source.display(),
                language_display_name(*language)
            )),
            Self::Operation {
                language,
                service,
                operation,
            } => Some(format!(
                "rename operation `{operation}` in service `{service}` so it generates a different {} path",
                language_display_name(*language)
            )),
            Self::Resource {
                language,
                service,
                resource,
            } => Some(format!(
                "rename resource `{resource}` in service `{service}` so it generates a different {} path",
                language_display_name(*language)
            )),
            Self::TypeDeclaration {
                language,
                source,
                declaration,
            } => Some(format!(
                "rename type declaration `{declaration}` in `{}` so it generates a different {} path",
                source.display(),
                language_display_name(*language)
            )),
            Self::ServiceDeclaration {
                language,
                source,
                service,
            } => Some(format!(
                "rename service `{service}` in `{}` so it generates a different {} path",
                source.display(),
                language_display_name(*language)
            )),
            Self::FixedArtifact { .. } => None,
        }
    }
}

fn language_display_name(language: Language) -> &'static str {
    match language {
        Language::Dotnet => ".NET",
        Language::Go => "Go",
        Language::Java => "Java",
        Language::Python => "Python",
        Language::Ruby => "Ruby",
        Language::TypeScript => "TypeScript",
    }
}

/// Accumulates generated files with the origins that control their paths, so a
/// collision can offer every applicable path-changing action.
#[derive(Debug, Default)]
pub(crate) struct GeneratedFileMap {
    files: BTreeMap<PathBuf, GeneratedFile>,
}

impl GeneratedFileMap {
    pub(crate) fn insert(
        &mut self,
        path: impl Into<PathBuf>,
        contents: String,
        origin: GeneratedFileOrigin,
    ) -> Result<()> {
        let path = path.into();
        if path.is_absolute()
            || path
                .components()
                .any(|component| component == std::path::Component::ParentDir)
        {
            return Err(Error::InvalidGeneratedPath {
                path,
                reason:
                    "generated file paths must be relative and stay within the output directory"
                        .to_string(),
            });
        }
        if let Some(first) = self.files.get(&path) {
            let first_source = first.origin.description();
            let second_source = origin.description();
            let remedy = match (
                first.origin.path_changing_action(),
                origin.path_changing_action(),
            ) {
                (Some(first), Some(second)) => format!("{first}; or {second}"),
                (Some(action), None) | (None, Some(action)) => action,
                (None, None) => format!(
                    "this is an internal generator defect: fixed artifacts {first_source} and {second_source} produce the same path"
                ),
            };
            return Err(Error::GeneratedFileSourceConflict {
                path,
                first_source,
                second_source,
                remedy,
            });
        }
        self.files.insert(path, GeneratedFile { contents, origin });
        Ok(())
    }

    pub(crate) fn insert_multi(
        &mut self,
        files: BTreeMap<PathBuf, String>,
        origin: GeneratedFileOrigin,
    ) -> Result<()> {
        for (path, contents) in files {
            self.insert(path, contents, origin.clone())?;
        }
        Ok(())
    }

    pub(crate) fn prefix(self, prefix: impl AsRef<Path>) -> Result<Self> {
        let mut prefixed = Self::default();
        for (path, file) in self.files {
            prefixed.insert(prefix.as_ref().join(path), file.contents, file.origin)?;
        }
        Ok(prefixed)
    }

    pub(crate) fn rekey_all(self, path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let mut rekeyed = Self::default();
        for file in self.files.into_values() {
            rekeyed.insert(path.clone(), file.contents, file.origin)?;
        }
        Ok(rekeyed)
    }

    pub(crate) fn extend(&mut self, other: Self) -> Result<()> {
        for (path, file) in other.files {
            self.insert(path, file.contents, file.origin)?;
        }
        Ok(())
    }

    pub(crate) fn contents_mut(&mut self, path: impl AsRef<Path>) -> Option<&mut String> {
        self.files
            .get_mut(path.as_ref())
            .map(|file| &mut file.contents)
    }

    pub(crate) fn into_files(self) -> BTreeMap<PathBuf, String> {
        self.files
            .into_iter()
            .map(|(path, file)| (path, file.contents))
            .collect()
    }
}

pub(crate) trait ExternalModelBackend<ModelType = PlannedType> {
    type ModelFragments;
    type WireConversion;

    /// Give the backend a chance to precompute model metadata from the planned spec.
    fn prepare(&mut self, api_plan: &PlannedSpec) -> Result<()>;

    /// Render model definitions owned by this backend.
    fn render_models(&self) -> Result<Self::ModelFragments>;

    /// Render support files owned by this backend.
    fn render_support_files(&self) -> Result<BTreeMap<PathBuf, String>> {
        Ok(BTreeMap::new())
    }

    /// Return the target-language type annotation/name for a model reference.
    fn model_type_annotation(&self, model_type: &ModelType) -> Option<String>;

    /// Return the stable wire/runtime type identifier for a model reference.
    fn wire_type_identifier(&self, model_type: &ModelType) -> Option<String>;

    /// Return conversion templates between public model values and wire values.
    ///
    /// `planned_record` is present when the type has already resolved to a planned
    /// record model, which lets backends handle generated/local model conversions.
    fn wire_conversion(
        &self,
        model_type: &ModelType,
        planned_record: Option<&RecordSpec<PlannedFamily>>,
    ) -> Option<Self::WireConversion>;
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum GenerationMode {
    #[default]
    NativeApi,
    DefinitionsOnly,
}

/// The TypeScript in-memory representation of a materialized temporal `format`
/// field, selected by the `--date-time-types` generator flag (P16 API parity).
/// Affects **only** the TypeScript output; Go / Java / Python are unchanged. See
/// `specs/json-schema/features/format.md` (JS temporal representation).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TsDateTimeTypes {
    /// Every temporal is a `string` holding the generator-serialized form
    /// (lossless, still materialized — the narrowed grammar rejects `:60`).
    #[default]
    String,
    /// `date-time` → `Date` (UTC instant, ms, offset folded); the others stay
    /// `string`.
    Date,
    /// `date-time` → `Temporal.ZonedDateTime`, `date` → `Temporal.PlainDate`,
    /// `duration` → `Temporal.Duration`; `time` stays `string`.
    Temporal,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct GenerateFilesOptions {
    pub(crate) go_output_dir_name: String,
    pub(crate) java_package_root: Option<String>,
    /// The TypeScript temporal representation (`--date-time-types`); ignored by
    /// the non-TypeScript backends.
    pub(crate) ts_date_time_types: TsDateTimeTypes,
}

pub(crate) fn generate_files_for_tree_with_mode_and_options(
    language: Language,
    tree: ApiSpecTree,
    descriptors: &DescriptorIndex,
    support: &SupportFiles,
    mode: GenerationMode,
    options: GenerateFilesOptions,
) -> Result<GeneratedFiles> {
    let config = crate::nexgen_config::NexgenConfig {
        mode,
        ..crate::nexgen_config::current()
    };
    let _scope = crate::nexgen_config::scope(config);
    crate::compile_tree_to_files(language, tree, descriptors, support, options)
}

pub(crate) fn generate_files_from_planned_tree(
    language: Language,
    tree: &ApiSpecTree<PlannedFamily>,
    support: &SupportFiles,
    options: GenerateFilesOptions,
) -> Result<GeneratedFiles> {
    let mode = crate::nexgen_config::current().mode;
    let mut generated = match language {
        Language::Dotnet => dotnet::generate(tree, support),
        Language::Go => generate_go_tree(tree, support, options),
        Language::Java => java::generate(tree, support, options.java_package_root.as_deref()),
        Language::Python => python::generate(tree, support),
        Language::TypeScript => typescript::generate(tree, support, options.ts_date_time_types),
        language => Err(Error::UnsupportedLanguage { language }),
    }?;
    generated.warnings = if mode == GenerationMode::NativeApi {
        generation_warnings_for_tree(tree)
    } else {
        Vec::new()
    };
    Ok(generated)
}

fn generate_go_tree(
    tree: &ApiSpecTree<PlannedFamily>,
    support: &SupportFiles,
    options: GenerateFilesOptions,
) -> Result<GeneratedFiles> {
    go::generate_tree(
        tree,
        support,
        &go::GoOptions {
            output_dir_name: options.go_output_dir_name,
            ..go::GoOptions::default()
        },
    )
}

pub fn generate_source(
    language: Language,
    spec: ApiSpec,
    descriptors: &DescriptorIndex,
    support: &SupportFiles,
) -> Result<String> {
    generate_source_with_mode(
        language,
        spec,
        descriptors,
        support,
        GenerationMode::NativeApi,
    )
}

pub fn generate_source_with_mode(
    language: Language,
    spec: ApiSpec,
    descriptors: &DescriptorIndex,
    support: &SupportFiles,
    mode: GenerationMode,
) -> Result<String> {
    let generated = generate_files_for_tree_with_mode_and_options(
        language,
        ApiSpecTree::single(spec),
        descriptors,
        support,
        mode,
        GenerateFilesOptions::default(),
    )?;
    Ok(match generated.layout {
        GeneratedOutputLayout::SingleFile => generated
            .single_file_contents()
            .expect("single-file output should contain one file")
            .to_string(),
        GeneratedOutputLayout::Directory => generated
            .files
            .iter()
            .map(|(path, contents)| format!("### {}\n{contents}", path.display()))
            .collect::<Vec<_>>()
            .join("\n\n"),
    })
}

fn generation_warnings(plan: &PlannedSpec) -> Vec<String> {
    plan.services
        .iter()
        .flat_map(|service| {
            service.resources.iter().flat_map(|resource| {
                resource.data.methods.iter().filter_map(|method| {
                    matches!(
                        method.binding,
                        crate::planning::PlannedResourceMethodBindingSpec::Stub
                    )
                    .then(|| {
                        format!(
                            "resource method `{}.{}` generated as a stub because no operation could be bound",
                            resource.data.type_name, method.name
                        )
                    })
                })
            })
        })
        .collect()
}

fn generation_warnings_for_tree(tree: &ApiSpecTree<PlannedFamily>) -> Vec<String> {
    fn collect(node: &ApiSpecNode<PlannedFamily>, warnings: &mut Vec<String>) {
        match node {
            ApiSpecNode::Leaf(leaf) => warnings.extend(generation_warnings(&leaf.spec)),
            ApiSpecNode::Branch(branch) => {
                for child in branch.children.values() {
                    collect(child, warnings);
                }
            }
        }
    }

    let mut warnings = Vec::new();
    collect(&tree.root, &mut warnings);
    warnings
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use prost_types::FileDescriptorSet;

    use crate::SupportFiles;
    use crate::descriptors::DescriptorIndex;
    use crate::language::Language;

    use super::{
        GenerateFilesOptions, GeneratedFileMap, GeneratedFileOrigin, GenerationMode,
        generate_files_for_tree_with_mode_and_options,
    };
    use crate::spec::ApiSpecTree;

    #[test]
    fn output_directory_named_origin_uses_its_target_language() {
        let origin =
            GeneratedFileOrigin::output_directory_named_module(Language::TypeScript, "client");

        assert_eq!(
            origin.description(),
            "TypeScript module derived from output directory `client`"
        );
        assert_eq!(
            origin.path_changing_action().as_deref(),
            Some(
                "point `--output` at a directory whose name generates a different TypeScript file path"
            )
        );
    }

    #[test]
    fn generated_file_insert_reports_both_mutable_origins_and_actions() {
        let mut files = GeneratedFileMap::default();
        let first = GeneratedFileOrigin::operation(Language::Python, "First", "collide");
        let second = GeneratedFileOrigin::operation(Language::Python, "Second", "collide");
        files
            .insert(
                PathBuf::from("models.py"),
                "first".to_string(),
                first.clone(),
            )
            .unwrap();
        let error = files
            .insert(
                PathBuf::from("models.py"),
                "second".to_string(),
                second.clone(),
            )
            .unwrap_err();
        let crate::error::Error::GeneratedFileSourceConflict {
            path,
            first_source,
            second_source,
            remedy,
        } = error
        else {
            panic!("expected generated-file source conflict, got {error}");
        };
        assert_eq!(path, PathBuf::from("models.py"));
        assert_eq!(first_source, "Python operation `First.collide`");
        assert_eq!(second_source, "Python operation `Second.collide`");
        assert!(remedy.contains("rename operation `collide` in service `First`"));
        assert!(remedy.contains("rename operation `collide` in service `Second`"));

        let mut reversed = GeneratedFileMap::default();
        reversed
            .insert("models.py", "second".to_string(), second)
            .unwrap();
        let reversed = reversed
            .insert("models.py", "first".to_string(), first)
            .unwrap_err();
        let crate::error::Error::GeneratedFileSourceConflict {
            first_source,
            second_source,
            remedy,
            ..
        } = reversed
        else {
            panic!("expected generated-file source conflict, got {reversed}");
        };
        assert_eq!(first_source, "Python operation `Second.collide`");
        assert_eq!(second_source, "Python operation `First.collide`");
        assert!(remedy.contains("service `First`"));
        assert!(remedy.contains("service `Second`"));
    }

    #[test]
    fn generated_file_insert_reports_both_resource_origins_and_actions() {
        let mut files = GeneratedFileMap::default();
        files
            .insert(
                "_resources/account.py",
                "first".to_string(),
                GeneratedFileOrigin::resource(Language::Python, "First", "Account"),
            )
            .unwrap();
        let error = files
            .insert(
                "_resources/account.py",
                "second".to_string(),
                GeneratedFileOrigin::resource(Language::Python, "Second", "Account"),
            )
            .unwrap_err();
        let crate::error::Error::GeneratedFileSourceConflict {
            path,
            first_source,
            second_source,
            remedy,
        } = error
        else {
            panic!("expected generated-file source conflict, got {error}");
        };
        assert_eq!(path, PathBuf::from("_resources/account.py"));
        assert_eq!(first_source, "Python resource `First.Account`");
        assert_eq!(second_source, "Python resource `Second.Account`");
        assert!(remedy.contains("rename resource `Account` in service `First`"));
        assert!(remedy.contains("rename resource `Account` in service `Second`"));
    }

    #[test]
    fn generated_file_insert_reports_only_mutable_action_against_fixed_artifact() {
        let mutable = GeneratedFileOrigin::input_module(Language::Go, "support.json");
        let fixed = GeneratedFileOrigin::fixed("generated Go support file");
        for (first, second, expected_first, expected_second) in [
            (
                mutable.clone(),
                fixed.clone(),
                "Go input module `support.json`",
                "generated Go support file",
            ),
            (
                fixed.clone(),
                mutable.clone(),
                "generated Go support file",
                "Go input module `support.json`",
            ),
        ] {
            let mut files = GeneratedFileMap::default();
            files
                .insert("support.go", "first".to_string(), first)
                .unwrap();
            let error = files
                .insert("support.go", "second".to_string(), second)
                .unwrap_err();
            let crate::error::Error::GeneratedFileSourceConflict {
                path,
                first_source,
                second_source,
                remedy,
            } = error
            else {
                panic!("expected generated-file source conflict, got {error}");
            };
            assert_eq!(path, PathBuf::from("support.go"));
            assert_eq!(first_source, expected_first);
            assert_eq!(second_source, expected_second);
            assert_eq!(
                remedy,
                "rename input file or directory `support.json` so it generates a different Go path"
            );
        }
    }

    #[test]
    fn generated_file_insert_reports_fixed_collision_as_internal_defect() {
        let mut files = GeneratedFileMap::default();
        files
            .insert(
                "runtime.py",
                "first".to_string(),
                GeneratedFileOrigin::fixed("first fixed runtime"),
            )
            .unwrap();
        let error = files
            .insert(
                "runtime.py",
                "second".to_string(),
                GeneratedFileOrigin::fixed("second fixed runtime"),
            )
            .unwrap_err();
        let crate::error::Error::GeneratedFileSourceConflict {
            first_source,
            second_source,
            remedy,
            ..
        } = error
        else {
            panic!("expected generated-file source conflict, got {error}");
        };
        assert_eq!(first_source, "first fixed runtime");
        assert_eq!(second_source, "second fixed runtime");
        assert!(remedy.contains("internal generator defect"));
        assert!(remedy.contains("first fixed runtime"));
        assert!(remedy.contains("second fixed runtime"));
    }

    #[test]
    fn generated_file_insert_multi_inserts_every_file() {
        let mut files = GeneratedFileMap::default();
        files
            .insert_multi(
                BTreeMap::from([
                    (PathBuf::from("a.py"), "a".to_string()),
                    (PathBuf::from("b.py"), "b".to_string()),
                ]),
                GeneratedFileOrigin::input_module(Language::Python, "one.yaml"),
            )
            .unwrap();
        assert_eq!(
            files.into_files(),
            BTreeMap::from([
                (PathBuf::from("a.py"), "a".to_string()),
                (PathBuf::from("b.py"), "b".to_string()),
            ])
        );
    }

    #[test]
    fn generated_file_insert_multi_reports_first_ordered_conflict() {
        let mut files = GeneratedFileMap::default();
        files
            .insert(
                PathBuf::from("b.py"),
                "first".to_string(),
                GeneratedFileOrigin::input_module(Language::Python, "one.yaml"),
            )
            .unwrap();
        let error = files
            .insert_multi(
                BTreeMap::from([
                    (PathBuf::from("a.py"), "a".to_string()),
                    (PathBuf::from("b.py"), "second".to_string()),
                    (PathBuf::from("c.py"), "c".to_string()),
                ]),
                GeneratedFileOrigin::input_module(Language::Python, "two.yaml"),
            )
            .unwrap_err();
        let crate::error::Error::GeneratedFileSourceConflict {
            path,
            first_source,
            second_source,
            remedy,
        } = error
        else {
            panic!("expected generated-file source conflict, got {error}");
        };
        assert_eq!(path, PathBuf::from("b.py"));
        assert_eq!(first_source, "Python input module `one.yaml`");
        assert_eq!(second_source, "Python input module `two.yaml`");
        assert!(remedy.contains("`one.yaml`"));
        assert!(remedy.contains("`two.yaml`"));
        assert!(files.files.contains_key(&PathBuf::from("a.py")));
        assert!(!files.files.contains_key(&PathBuf::from("c.py")));
    }

    #[test]
    fn generated_file_insert_rejects_paths_outside_the_output_directory() {
        for path in [PathBuf::from("/absolute.py"), PathBuf::from("../parent.py")] {
            let mut files = GeneratedFileMap::default();
            let error = files
                .insert(
                    path.clone(),
                    String::new(),
                    GeneratedFileOrigin::fixed("test artifact"),
                )
                .unwrap_err();
            assert!(matches!(
                error,
                crate::error::Error::InvalidGeneratedPath { path: actual, .. }
                    if actual == path
            ));
        }
    }

    #[test]
    fn warns_when_resource_method_generates_as_stub() {
        let wit = r#"
package temporal:users@1.0.0;

world system {
  export user-service;
}

/// @nexus.endpoint "__user_service"
interface user-service {
  resource user {
    constructor(user-id: string, email: string);

    update-email: func(email: string) -> user-result;
  }

  type user-result = own<user>;

  record update-email-request {
    users-id: string,
    email: string,
  }

  update-email: func(request: update-email-request) -> user-result;
}
"#;
        let spec = crate::parser::parse_api_spec_from_wit_for_language(
            Language::Python,
            wit,
            PathBuf::from("inline.wit"),
        )
        .unwrap();
        let descriptors =
            DescriptorIndex::from_descriptor_set(FileDescriptorSet { file: Vec::new() }).unwrap();
        let generated = generate_files_for_tree_with_mode_and_options(
            Language::Python,
            ApiSpecTree::single(spec),
            &descriptors,
            &SupportFiles::default(),
            GenerationMode::NativeApi,
            GenerateFilesOptions::default(),
        )
        .unwrap();

        assert_eq!(
            generated.warnings,
            vec![
                "resource method `User.update-email` generated as a stub because no operation could be bound"
                    .to_string()
            ]
        );
    }

    #[test]
    fn definitions_only_generation_does_not_require_endpoint() {
        let wit = r#"
package temporal:example@1.0.0;

world system {
  export example-service;
}

interface example-service {
  record request {
    name: string,
  }

  record response {
    message: string,
  }

  example-operation: func(request: request) -> response;
}
"#;
        let spec = crate::parser::parse_api_spec_from_wit_for_language(
            Language::Python,
            wit,
            PathBuf::from("inline.wit"),
        )
        .unwrap();
        let descriptors =
            DescriptorIndex::from_descriptor_set(FileDescriptorSet { file: Vec::new() }).unwrap();

        let generated = generate_files_for_tree_with_mode_and_options(
            Language::Python,
            ApiSpecTree::single(spec),
            &descriptors,
            &SupportFiles::default(),
            GenerationMode::DefinitionsOnly,
            GenerateFilesOptions::default(),
        )
        .unwrap();

        assert!(generated.files.contains_key(&PathBuf::from("models.py")));
        assert!(generated.files.contains_key(&PathBuf::from("services.py")));
        assert!(
            !generated
                .files
                .contains_key(&PathBuf::from("operations/example_operation.py"))
        );
        assert!(generated.files[&PathBuf::from("services.py")].contains("class ExampleService"));
    }
}
