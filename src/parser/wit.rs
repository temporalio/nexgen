use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use heck::{ToSnakeCase, ToUpperCamelCase};
use tempfile::TempDir;
use wit_parser_crate::{
    Function, FunctionKind, Handle, Interface, PackageId, PackageSourceMap, Record, Resolve, Type,
    TypeDef, TypeDefKind, TypeId, TypeOwner, WorldId, WorldItem, WorldKey,
};

use crate::error::{Error, Result};
use crate::language::Language;
use crate::spec::*;

type PackageOrigins = BTreeMap<PackageId, PathBuf>;

pub(crate) struct ParsedWitPackage {
    pub resolve: Resolve,
    pub package_id: PackageId,
    pub package_origins: PackageOrigins,
    _workspace: TempDir,
}

fn split_input_paths(input_paths: &[PathBuf]) -> Result<(&PathBuf, &[PathBuf])> {
    input_paths.split_first().ok_or_else(|| Error::InvalidWit {
        path: PathBuf::from("<input>"),
        reason: "at least one WIT input path is required".to_string(),
    })
}

pub fn load_api_spec_from_wit_for_language_with_inputs(
    language: Language,
    input_paths: &[PathBuf],
) -> Result<ApiSpec> {
    let (path, linked_input_paths) = split_input_paths(input_paths)?;
    let input = fs::read_to_string(path).map_err(|source| Error::ReadFile {
        path: path.clone(),
        source,
    })?;
    parse_api_spec_from_wit_for_language_with_inputs(
        language,
        &input,
        path.clone(),
        linked_input_paths,
    )
}

#[cfg(test)]
pub(crate) fn parse_api_spec_from_wit_for_language(
    language: Language,
    input: &str,
    path: PathBuf,
) -> Result<ApiSpec> {
    parse_api_spec_from_wit_for_language_with_inputs(language, input, path, &[])
}

pub(crate) fn parse_api_spec_from_wit_for_language_with_inputs(
    language: Language,
    input: &str,
    path: PathBuf,
    linked_input_paths: &[PathBuf],
) -> Result<ApiSpec> {
    let parsed = parse_wit_with_inputs(input, &path, linked_input_paths)?;
    api_spec_from_wit(
        &parsed.resolve,
        parsed.package_id,
        &parsed.package_origins,
        path,
        language,
    )
}

fn api_spec_from_wit(
    resolve: &Resolve,
    package_id: PackageId,
    package_origins: &PackageOrigins,
    path: PathBuf,
    language: Language,
) -> Result<ApiSpec> {
    let package = &resolve.packages[package_id];
    let world_id = select_world(resolve, package_id, &path)?;
    let world = &resolve.worlds[world_id];
    let support = collect_support_spec(resolve, package_id, package_origins)?;

    let mut types = BTreeMap::new();
    for (_, dependency_package) in resolve.packages.iter() {
        for interface_id in dependency_package.interfaces.values() {
            let interface = &resolve.interfaces[*interface_id];
            collect_interface_types(resolve, interface, &path, language, &mut types)?;
        }
    }

    let mut services = Vec::new();
    for (key, item) in &world.exports {
        let WorldItem::Interface { id, .. } = item else {
            continue;
        };
        let interface = &resolve.interfaces[*id];
        let service = build_service(resolve, key, interface, &path, language)?;
        if service.operations.is_empty() && service.resources.is_empty() {
            for type_id in interface.types.values() {
                let full_name = wit_type_full_name(resolve, *type_id);
                if let Some(entry) = types.get_mut(&full_name) {
                    entry.module_export = crate::spec::ModuleExport::Owned;
                }
            }
        }
        services.push(service);
    }

    let spec = ApiSpec {
        module_path: crate::spec::ModulePath::default(),
        data: (),
        version: package
            .name
            .version
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| "0.0.0".to_string()),
        support,
        services,
        types,
    };
    Ok(spec)
}

pub fn write_prepared_wit_directory(input_paths: &[PathBuf], output_path: &Path) -> Result<()> {
    if output_path.exists() {
        return Err(Error::OutputPathExists {
            path: output_path.to_path_buf(),
        });
    }

    let (input_path, linked_input_paths) = split_input_paths(input_paths)?;
    let input = fs::read_to_string(input_path).map_err(|source| Error::ReadFile {
        path: input_path.clone(),
        source,
    })?;
    let workspace = prepare_wit_workspace(&input, input_path, linked_input_paths)?;
    copy_directory_tree(&workspace.package_root, output_path)?;
    Ok(())
}

pub(crate) fn parse_wit_with_inputs(
    input: &str,
    path: &Path,
    linked_input_paths: &[PathBuf],
) -> Result<ParsedWitPackage> {
    let workspace = prepare_wit_workspace(input, path, linked_input_paths)?;
    parse_prepared_wit_workspace(workspace, path)
}

fn parse_prepared_wit_workspace(
    workspace: PreparedWitWorkspace,
    path: &Path,
) -> Result<ParsedWitPackage> {
    let mut resolve = Resolve::default();
    let (package_id, source_map) =
        resolve
            .push_dir(&workspace.package_root)
            .map_err(|error| Error::WitParse {
                path: path.to_path_buf(),
                message: format_error_chain(&error),
            })?;
    let package_origins = collect_package_origins(&resolve, &source_map)?;
    Ok(ParsedWitPackage {
        resolve,
        package_id,
        package_origins,
        _workspace: workspace.temp_dir,
    })
}

fn format_error_chain(error: &impl std::fmt::Display) -> String {
    format!("{error:#}")
}

fn collect_support_spec(
    resolve: &Resolve,
    current_package_id: PackageId,
    package_origins: &PackageOrigins,
) -> Result<SupportSpec> {
    let mut fragments = BTreeMap::new();

    for language in all_languages() {
        let mut language_fragments = Vec::new();
        let mut seen_paths = BTreeSet::new();

        for (package_id, origin_path) in package_origins {
            if *package_id == current_package_id {
                continue;
            }
            collect_package_support_fragments(
                language,
                resolve,
                *package_id,
                origin_path,
                &mut seen_paths,
                &mut language_fragments,
            )?;
        }

        if let Some(origin_path) = package_origins.get(&current_package_id) {
            collect_package_support_fragments(
                language,
                resolve,
                current_package_id,
                origin_path,
                &mut seen_paths,
                &mut language_fragments,
            )?;
        }

        if !language_fragments.is_empty() {
            fragments.insert(language, language_fragments);
        }
    }

    Ok(SupportSpec { fragments })
}

fn collect_package_support_fragments(
    language: Language,
    resolve: &Resolve,
    package_id: PackageId,
    origin_path: &Path,
    seen_paths: &mut BTreeSet<String>,
    fragments: &mut Vec<SupportFragmentSpec>,
) -> Result<()> {
    let package = &resolve.packages[package_id];
    let package_name = if let Some(version) = &package.name.version {
        format!(
            "{}:{}@{}",
            package.name.namespace, package.name.name, version
        )
    } else {
        format!("{}:{}", package.name.namespace, package.name.name)
    };

    collect_support_fragment_from_docs(
        language,
        package.docs.contents.as_deref(),
        origin_path,
        &format!("package `{package_name}`"),
        seen_paths,
        fragments,
    )?;

    for (world_name, world_id) in &package.worlds {
        let world = &resolve.worlds[*world_id];
        collect_support_fragment_from_docs(
            language,
            world.docs.contents.as_deref(),
            origin_path,
            &format!("package `{package_name}` world `{world_name}`"),
            seen_paths,
            fragments,
        )?;
    }

    Ok(())
}

fn collect_support_fragment_from_docs(
    language: Language,
    docs: Option<&str>,
    origin_path: &Path,
    context: &str,
    seen_paths: &mut BTreeSet<String>,
    fragments: &mut Vec<SupportFragmentSpec>,
) -> Result<()> {
    let directives = parse_directives(docs, origin_path, context)?;
    let Some(relative_path) =
        directive_value_for_language(&directives, "support", origin_path, context, language)?
    else {
        return Ok(());
    };
    let namespace = directive_value_for_language(
        &directives,
        "support-namespace",
        origin_path,
        context,
        language,
    )?;

    let resolved_path = resolve_support_path(origin_path, &relative_path);
    let normalized_path = resolved_path.to_string_lossy().replace('\\', "/");
    if !seen_paths.insert(normalized_path.clone()) {
        return Ok(());
    }

    let contents = load_support_fragment_contents(&resolved_path)?;
    fragments.push(SupportFragmentSpec {
        path: normalized_path,
        contents,
        namespace,
    });
    Ok(())
}

fn load_support_fragment_contents(path: &Path) -> Result<String> {
    fs::read_to_string(path).map_err(|source| Error::ReadFile {
        path: path.to_path_buf(),
        source,
    })
}

fn resolve_support_path(base_dir: &Path, support_path: &str) -> PathBuf {
    let support_path = PathBuf::from(support_path);
    if support_path.is_absolute() {
        support_path
    } else {
        base_dir.join(support_path)
    }
}

struct PreparedWitWorkspace {
    temp_dir: TempDir,
    package_root: PathBuf,
}

fn prepare_wit_workspace(
    input: &str,
    path: &Path,
    linked_input_paths: &[PathBuf],
) -> Result<PreparedWitWorkspace> {
    let temp_dir = tempfile::tempdir().map_err(|source| Error::WriteFile {
        path: PathBuf::from("<tempdir>"),
        source,
    })?;
    let package_root = temp_dir.path().join("main");
    fs::create_dir_all(&package_root).map_err(|source| Error::WriteFile {
        path: package_root.clone(),
        source,
    })?;

    if let Some(source_dir) = input_package_source_dir(path) {
        copy_package_source_dir(&source_dir, &package_root, path)?;
    } else if let Some(source_dir) = input_support_source_dir(path) {
        copy_standalone_input_support_dir(&source_dir, &package_root, path)?;
    }

    let target_name = input_target_name(path);
    let target_path = package_root.join(&target_name);
    if let Some(parent) = target_path.parent() {
        fs::create_dir_all(parent).map_err(|source| Error::WriteFile {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    fs::write(&target_path, input).map_err(|source| Error::WriteFile {
        path: target_path,
        source,
    })?;

    copy_linked_inputs(&package_root, linked_input_paths)?;

    Ok(PreparedWitWorkspace {
        temp_dir,
        package_root,
    })
}

fn input_package_source_dir(path: &Path) -> Option<PathBuf> {
    if path.is_dir() {
        return Some(path.to_path_buf());
    }

    if path.file_name()? != "main.wit" {
        return None;
    }

    let parent = path.parent()?;
    if parent.as_os_str().is_empty() || !parent.exists() {
        return None;
    }
    Some(parent.to_path_buf())
}

fn input_support_source_dir(path: &Path) -> Option<PathBuf> {
    if path.is_dir() {
        return None;
    }

    let parent = path.parent()?;
    if parent.as_os_str().is_empty() || !parent.exists() {
        return None;
    }
    Some(parent.to_path_buf())
}

fn input_target_name(path: &Path) -> OsString {
    path.file_name()
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| OsString::from("input.wit"))
}

fn copy_package_source_dir(
    source_dir: &Path,
    destination_dir: &Path,
    input_path: &Path,
) -> Result<()> {
    for entry in fs::read_dir(source_dir).map_err(|source| Error::ReadFile {
        path: source_dir.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| Error::ReadFile {
            path: source_dir.to_path_buf(),
            source,
        })?;
        let source_path = entry.path();
        let destination_path = destination_dir.join(entry.file_name());

        if source_path == input_path {
            continue;
        }

        let file_type = entry.file_type().map_err(|source| Error::ReadFile {
            path: source_path.clone(),
            source,
        })?;
        if file_type.is_dir() {
            copy_package_source_dir(&source_path, &destination_path, input_path)?;
            continue;
        }

        if let Some(parent) = destination_path.parent() {
            fs::create_dir_all(parent).map_err(|source| Error::WriteFile {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        fs::copy(&source_path, &destination_path).map_err(|source| Error::WriteFile {
            path: destination_path,
            source,
        })?;
    }

    Ok(())
}

fn copy_standalone_input_support_dir(
    source_dir: &Path,
    destination_dir: &Path,
    input_path: &Path,
) -> Result<()> {
    for entry in fs::read_dir(source_dir).map_err(|source| Error::ReadFile {
        path: source_dir.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| Error::ReadFile {
            path: source_dir.to_path_buf(),
            source,
        })?;
        let source_path = entry.path();
        let destination_path = destination_dir.join(entry.file_name());

        if source_path == input_path {
            continue;
        }

        let file_type = entry.file_type().map_err(|source| Error::ReadFile {
            path: source_path.clone(),
            source,
        })?;
        if file_type.is_dir() {
            if entry.file_name() == "deps" {
                continue;
            }
            copy_standalone_input_support_dir(&source_path, &destination_path, input_path)?;
            continue;
        }

        if source_path
            .extension()
            .is_some_and(|extension| extension == "wit")
        {
            continue;
        }

        if let Some(parent) = destination_path.parent() {
            fs::create_dir_all(parent).map_err(|source| Error::WriteFile {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        fs::copy(&source_path, &destination_path).map_err(|source| Error::WriteFile {
            path: destination_path,
            source,
        })?;
    }

    Ok(())
}

fn copy_linked_inputs(package_root: &Path, linked_input_paths: &[PathBuf]) -> Result<()> {
    for linked_input_path in linked_input_paths {
        copy_linked_input(package_root, linked_input_path)?;
    }
    Ok(())
}

fn copy_linked_input(package_root: &Path, linked_input_path: &Path) -> Result<()> {
    let metadata = fs::metadata(linked_input_path).map_err(|source| Error::ReadFile {
        path: linked_input_path.to_path_buf(),
        source,
    })?;
    if metadata.is_file() {
        return copy_linked_input_file(package_root, linked_input_path);
    }
    if linked_input_path_is_package_dir(linked_input_path)? {
        return copy_linked_input_package_dir(package_root, linked_input_path);
    }
    copy_linked_input_collection_dir(package_root, linked_input_path)
}

fn copy_linked_input_file(package_root: &Path, linked_input_path: &Path) -> Result<()> {
    let package_name = linked_input_package_dir_name(linked_input_path)?;
    let destination_path = package_root
        .join("deps")
        .join(package_name)
        .join(input_target_name(linked_input_path));
    if let Some(parent) = destination_path.parent() {
        fs::create_dir_all(parent).map_err(|source| Error::WriteFile {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    fs::copy(linked_input_path, &destination_path).map_err(|source| Error::WriteFile {
        path: destination_path,
        source,
    })?;
    Ok(())
}

fn copy_linked_input_package_dir(package_root: &Path, linked_input_path: &Path) -> Result<()> {
    let package_name = linked_input_package_dir_name(linked_input_path)?;
    let destination_path = package_root.join("deps").join(package_name);
    copy_directory_tree(linked_input_path, &destination_path)
}

fn copy_linked_input_collection_dir(package_root: &Path, linked_input_path: &Path) -> Result<()> {
    for entry in fs::read_dir(linked_input_path).map_err(|source| Error::ReadFile {
        path: linked_input_path.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| Error::ReadFile {
            path: linked_input_path.to_path_buf(),
            source,
        })?;
        let source_path = entry.path();
        let file_type = entry.file_type().map_err(|source| Error::ReadFile {
            path: source_path.clone(),
            source,
        })?;
        if file_type.is_dir() {
            if linked_input_path_is_package_dir(&source_path)? {
                copy_linked_input_package_dir(package_root, &source_path)?;
            } else {
                copy_linked_input_collection_dir(package_root, &source_path)?;
            }
        } else if source_path
            .extension()
            .is_some_and(|extension| extension == "wit")
        {
            copy_linked_input_file(package_root, &source_path)?;
        }
    }
    Ok(())
}

fn linked_input_path_is_package_dir(linked_input_path: &Path) -> Result<bool> {
    for entry in fs::read_dir(linked_input_path).map_err(|source| Error::ReadFile {
        path: linked_input_path.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| Error::ReadFile {
            path: linked_input_path.to_path_buf(),
            source,
        })?;
        let source_path = entry.path();
        if source_path
            .extension()
            .is_some_and(|extension| extension == "wit")
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn linked_input_package_dir_name(linked_input_path: &Path) -> Result<OsString> {
    let file_name = linked_input_path
        .file_name()
        .ok_or_else(|| Error::InvalidWit {
            path: linked_input_path.to_path_buf(),
            reason: "linked WIT input path must name a package directory".to_string(),
        })?;
    if linked_input_path
        .extension()
        .is_some_and(|extension| extension == "wit")
    {
        return linked_input_path
            .file_stem()
            .map(|stem| stem.to_os_string())
            .ok_or_else(|| Error::InvalidWit {
                path: linked_input_path.to_path_buf(),
                reason: "linked WIT input file must have a stem".to_string(),
            });
    }
    Ok(file_name.to_os_string())
}

fn copy_directory_tree(source_dir: &Path, destination_dir: &Path) -> Result<()> {
    for entry in fs::read_dir(source_dir).map_err(|source| Error::ReadFile {
        path: source_dir.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| Error::ReadFile {
            path: source_dir.to_path_buf(),
            source,
        })?;
        let source_path = entry.path();
        let destination_path = destination_dir.join(entry.file_name());
        let file_type = entry.file_type().map_err(|source| Error::ReadFile {
            path: source_path.clone(),
            source,
        })?;

        if file_type.is_dir() {
            fs::create_dir_all(&destination_path).map_err(|source| Error::WriteFile {
                path: destination_path.clone(),
                source,
            })?;
            copy_directory_tree(&source_path, &destination_path)?;
            continue;
        }

        if let Some(parent) = destination_path.parent() {
            fs::create_dir_all(parent).map_err(|source| Error::WriteFile {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        fs::copy(&source_path, &destination_path).map_err(|source| Error::WriteFile {
            path: destination_path,
            source,
        })?;
    }
    Ok(())
}

fn collect_package_origins(
    resolve: &Resolve,
    source_map: &PackageSourceMap,
) -> Result<PackageOrigins> {
    let mut package_origins = BTreeMap::new();

    for (package_id, _) in resolve.packages.iter() {
        let Some(paths) = source_map.package_paths(package_id) else {
            continue;
        };
        let mut package_paths = paths.collect::<Vec<_>>();
        if package_paths.is_empty() {
            continue;
        }
        package_paths.sort();
        let origin = package_paths[0]
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        package_origins.insert(package_id, origin);
    }

    if package_origins.is_empty() {
        return Err(Error::InvalidWit {
            path: PathBuf::from("<workspace>"),
            reason: "resolved WIT package graph had no source origins".to_string(),
        });
    }

    Ok(package_origins)
}

fn authored_field_type_for_language(field_type: TypeSpec, language: Language) -> TypeSpec {
    match field_type {
        TypeSpec::Option(inner) => {
            TypeSpec::Option(Box::new(authored_field_type_for_language(*inner, language)))
        }
        TypeSpec::List(inner) => {
            TypeSpec::List(Box::new(authored_field_type_for_language(*inner, language)))
        }
        TypeSpec::Tuple(items) => TypeSpec::Tuple(
            items
                .into_iter()
                .map(|item| authored_field_type_for_language(item, language))
                .collect(),
        ),
        TypeSpec::Map(key, value) => TypeSpec::Map(
            Box::new(authored_field_type_for_language(*key, language)),
            Box::new(authored_field_type_for_language(*value, language)),
        ),
        TypeSpec::Result { ok, err } => TypeSpec::Result {
            ok: ok.map(|ok| Box::new(authored_field_type_for_language(*ok, language))),
            err: err.map(|err| Box::new(authored_field_type_for_language(*err, language))),
        },
        TypeSpec::External(ExternalTypeSpec::Alias {
            name,
            target,
            type_name,
        }) => {
            let target = authored_field_type_for_language(*target, language);
            if type_name.for_language(language).is_some() {
                TypeSpec::External(ExternalTypeSpec::Alias {
                    name,
                    target: Box::new(target),
                    type_name,
                })
            } else {
                target
            }
        }
        other => other,
    }
}

#[derive(Debug, Clone, PartialEq)]
struct FlattenedFunctionTypeSpec {
    arg_fields: Vec<FlattenedFunctionArgSpec>,
    function: Option<FunctionFieldSpec>,
}

#[derive(Debug, Clone, PartialEq)]
struct FlattenedFunctionArgSpec {
    args_name: String,
    field_name: String,
    field_type: TypeSpec,
    required: bool,
}

#[derive(Debug, Clone, PartialEq)]
struct ResolvedFunctionSignature {
    args: ResolvedFunctionSignatureArgs,
    result: TypeSpec,
}

#[derive(Debug, Clone, PartialEq)]
struct ResolvedFunctionSignatureArgs {
    name: String,
    field_type: TypeSpec,
    function_args: FunctionArgsSpec,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FunctionVarargsSpec {
    param: String,
    typescript_drop_prefix: bool,
}

fn collect_interface_types(
    resolve: &Resolve,
    interface: &Interface,
    path: &Path,
    language: Language,
    types: &mut BTreeMap<String, TypeDeclEntry>,
) -> Result<()> {
    let interface_name = interface
        .name
        .as_deref()
        .unwrap_or("unnamed-interface")
        .to_string();
    for type_id in interface.types.values() {
        let type_def = &resolve.types[*type_id];
        validate_type_parameter_directive(resolve, *type_id, type_def, path)?;
        if let Some(record) =
            build_wit_record_spec(resolve, *type_id, type_def, path, &interface_name, language)?
        {
            if types
                .insert(
                    record.full_name.clone(),
                    TypeDeclEntry::new(TypeDeclSpec::Record(record)),
                )
                .is_some()
            {
                return Err(Error::InvalidWit {
                    path: path.to_path_buf(),
                    reason: format!(
                        "duplicate WIT record mapping for `{}`",
                        wit_type_full_name(resolve, *type_id)
                    ),
                });
            }
        }
        if let Some(enumeration) = build_wit_enum_spec(resolve, *type_id, type_def) {
            if types
                .insert(
                    enumeration.full_name.clone(),
                    TypeDeclEntry::new(TypeDeclSpec::Enum(enumeration)),
                )
                .is_some()
            {
                return Err(Error::InvalidWit {
                    path: path.to_path_buf(),
                    reason: format!(
                        "duplicate WIT enum mapping for `{}`",
                        wit_type_full_name(resolve, *type_id)
                    ),
                });
            }
        }
        if let Some(flag_set) = build_wit_flags_spec(resolve, *type_id, type_def) {
            if types
                .insert(
                    flag_set.full_name.clone(),
                    TypeDeclEntry::new(TypeDeclSpec::Flags(flag_set)),
                )
                .is_some()
            {
                return Err(Error::InvalidWit {
                    path: path.to_path_buf(),
                    reason: format!(
                        "duplicate WIT flags mapping for `{}`",
                        wit_type_full_name(resolve, *type_id)
                    ),
                });
            }
        }
        if let Some(variant) = build_wit_variant_spec(resolve, *type_id, type_def, path, language)?
        {
            if types
                .insert(
                    variant.full_name.clone(),
                    TypeDeclEntry::new(TypeDeclSpec::Variant(variant)),
                )
                .is_some()
            {
                return Err(Error::InvalidWit {
                    path: path.to_path_buf(),
                    reason: format!(
                        "duplicate WIT variant mapping for `{}`",
                        wit_type_full_name(resolve, *type_id)
                    ),
                });
            }
        }
        let Some((proto_name, external_type)) =
            build_external_type_binding(resolve, type_def, path, &interface_name, language)?
        else {
            continue;
        };
        if types
            .insert(
                proto_name.clone(),
                TypeDeclEntry::new(TypeDeclSpec::External(external_type)),
            )
            .is_some()
        {
            return Err(Error::InvalidWit {
                path: path.to_path_buf(),
                reason: format!("duplicate `@nexus.proto` mapping for `{proto_name}`"),
            });
        }
    }

    Ok(())
}

fn validate_type_parameter_directive(
    resolve: &Resolve,
    type_id: TypeId,
    type_def: &TypeDef,
    path: &Path,
) -> Result<()> {
    let context = format!("type `{}`", wit_type_full_name(resolve, type_id));
    let directives = parse_directives(type_def.docs.contents.as_deref(), path, &context)?;
    let Some(parameter) = directive(&directives, "type-parameter", path, &context)? else {
        return Ok(());
    };
    if !parameter.args.is_empty() {
        return Err(Error::InvalidWitDirective {
            path: path.to_path_buf(),
            context,
            directive: "@nexus.type-parameter".to_string(),
            reason: "does not take arguments".to_string(),
        });
    }
    if !matches!(type_def.kind, TypeDefKind::Type(_)) {
        return Err(Error::InvalidWitDirective {
            path: path.to_path_buf(),
            context,
            directive: "@nexus.type-parameter".to_string(),
            reason: "is only supported on WIT type aliases".to_string(),
        });
    }
    for conflict in ["proto", "type", "function"] {
        if directive(&directives, conflict, path, &context)?.is_some() {
            return Err(Error::InvalidWitDirective {
                path: path.to_path_buf(),
                context,
                directive: "@nexus.type-parameter".to_string(),
                reason: format!("cannot be combined with `@nexus.{conflict}`"),
            });
        }
    }
    Ok(())
}

fn build_wit_enum_spec(resolve: &Resolve, type_id: TypeId, type_def: &TypeDef) -> Option<EnumSpec> {
    let TypeDefKind::Enum(enumeration) = &type_def.kind else {
        return None;
    };

    let type_name = type_def.name.as_deref().unwrap_or("unnamed-type");
    Some(EnumSpec {
        name: type_name.to_upper_camel_case(),
        full_name: wit_type_full_name(resolve, type_id),
        values: enumeration
            .cases
            .iter()
            .enumerate()
            .map(|(index, value)| EnumValueSpec {
                wire_name: value.name.clone(),
                name: value.name.to_upper_camel_case(),
                number: i32::try_from(index).expect("WIT enum case index should fit in i32"),
            })
            .collect(),
    })
}

fn build_wit_flags_spec(
    resolve: &Resolve,
    type_id: TypeId,
    type_def: &TypeDef,
) -> Option<FlagsSpec> {
    let TypeDefKind::Flags(flags) = &type_def.kind else {
        return None;
    };

    let type_name = type_def.name.as_deref().unwrap_or("unnamed-type");
    Some(FlagsSpec {
        name: type_name.to_upper_camel_case(),
        full_name: wit_type_full_name(resolve, type_id),
        flags: flags
            .flags
            .iter()
            .enumerate()
            .map(|(index, flag)| FlagSpec {
                name: flag.name.to_upper_camel_case(),
                bit: index,
            })
            .collect(),
    })
}

fn build_wit_variant_spec(
    resolve: &Resolve,
    type_id: TypeId,
    type_def: &TypeDef,
    path: &Path,
    language: Language,
) -> Result<Option<VariantSpec>> {
    let TypeDefKind::Variant(variant) = &type_def.kind else {
        return Ok(None);
    };

    let type_name = type_def.name.as_deref().unwrap_or("unnamed-type");
    let context = format!("type `{}`", wit_type_full_name(resolve, type_id));
    let cases = variant
        .cases
        .iter()
        .map(|case| {
            let case_context = format!("{context} case `{}`", case.name);
            let payload = case
                .ty
                .as_ref()
                .map(|ty| {
                    resolve_authored_field_type_spec(resolve, ty, path, &case_context)
                        .map(|field_type| authored_field_type_for_language(field_type, language))
                })
                .transpose()?;
            Ok(VariantCaseSpec {
                name: case.name.clone(),
                wire_name: case.name.to_snake_case(),
                payload,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Some(VariantSpec {
        name: type_name.to_upper_camel_case(),
        full_name: wit_type_full_name(resolve, type_id),
        cases,
    }))
}

fn build_wit_record_spec(
    resolve: &Resolve,
    type_id: TypeId,
    type_def: &TypeDef,
    path: &Path,
    interface_name: &str,
    language: Language,
) -> Result<Option<RecordSpec>> {
    let TypeDefKind::Record(record) = &type_def.kind else {
        return Ok(None);
    };

    let type_name = type_def.name.as_deref().unwrap_or("unnamed-type");
    let context = format!("type `{interface_name}.{type_name}`");
    let directives = parse_directives(type_def.docs.contents.as_deref(), path, &context)?;
    let experimental = experimental_directive(&directives, path, &context)?;
    let source_type = directive(&directives, "proto", path, &context)?
        .map(|proto_directive| {
            proto_directive
                .value("value")
                .map(|proto_name| ExternalTypeSpec::Proto(Symbol::new(proto_name.to_string())))
                .ok_or_else(|| Error::InvalidWitDirective {
                    path: path.to_path_buf(),
                    context: context.clone(),
                    directive: "@nexus.proto".to_string(),
                    reason: "missing required proto type name".to_string(),
                })
        })
        .transpose()?;
    let flatten_in_api = directive(&directives, "flatten-in-api", path, &context)?.is_some();
    if directive(&directives, "omit", path, &context)?.is_some() {
        return Err(Error::InvalidWitDirective {
            path: path.to_path_buf(),
            context: context.clone(),
            directive: "@nexus.omit".to_string(),
            reason: "mark record fields with `@nexus.omit`; type-level omit is no longer supported"
                .to_string(),
        });
    }
    let model_doc = directive(&directives, "doc", path, &context)?
        .map(directive_language_string)
        .unwrap_or_default();
    let (doc, fields) = build_fields_from_record(
        resolve,
        type_def.owner,
        record,
        model_doc,
        path,
        &context,
        language,
    )?;

    Ok(Some(RecordSpec {
        name: type_name.to_upper_camel_case(),
        full_name: wit_type_full_name(resolve, type_id),
        doc,
        source_type,
        experimental,
        flatten_in_api,
        fields,
        data: (),
    }))
}

fn wit_type_full_name(resolve: &Resolve, type_id: TypeId) -> String {
    let type_def = &resolve.types[type_id];
    let type_name = type_def.name.as_deref().unwrap_or("unnamed-type");
    match type_def.owner {
        TypeOwner::Interface(interface_id) => {
            let interface = &resolve.interfaces[interface_id];
            let interface_name = interface.name.as_deref().unwrap_or("unnamed-interface");
            format!("{interface_name}.{type_name}")
        }
        TypeOwner::World(world_id) => {
            let world = &resolve.worlds[world_id];
            let world_name = world.name.as_str();
            format!("{world_name}.{type_name}")
        }
        TypeOwner::None => type_name.to_string(),
    }
}

fn build_external_type_binding(
    resolve: &Resolve,
    type_def: &TypeDef,
    path: &Path,
    interface_name: &str,
    language: Language,
) -> Result<Option<(String, ExternalTypeBindingSpec)>> {
    let type_name = type_def.name.as_deref().unwrap_or("unnamed-type");
    let context = format!("type `{interface_name}.{type_name}`");
    let directives = parse_directives(type_def.docs.contents.as_deref(), path, &context)?;
    let Some(proto_directive) = directive(&directives, "proto", path, &context)? else {
        return Ok(None);
    };
    let Some(proto_name) = proto_directive.value("value").map(ToOwned::to_owned) else {
        return Err(Error::InvalidWitDirective {
            path: path.to_path_buf(),
            context,
            directive: "@nexus.proto".to_string(),
            reason: "missing required proto type name".to_string(),
        });
    };
    let mut reference = LanguageStringSpec {
        default: Some(proto_name.clone()),
        ..Default::default()
    };
    apply_directive_imports(&mut reference, proto_directive);
    let type_name = directive_prefixed_language_string(proto_directive, "type");

    let replacement = build_type_replacement(&directives, path, &context, &proto_name, language)?;

    let flatten_in_api = directive(&directives, "flatten-in-api", path, &context)?.is_some();
    let authored_record = matches!(type_def.kind, TypeDefKind::Record(_));
    if flatten_in_api && !authored_record {
        return Err(Error::InvalidWitDirective {
            path: path.to_path_buf(),
            context: context.clone(),
            directive: "@nexus.flatten-in-api".to_string(),
            reason: "only supported on record types".to_string(),
        });
    }

    if directive(&directives, "omit", path, &context)?.is_some() {
        return Err(Error::InvalidWitDirective {
            path: path.to_path_buf(),
            context: context.clone(),
            directive: "@nexus.omit".to_string(),
            reason: "mark record fields with `@nexus.omit`; type-level omit is no longer supported"
                .to_string(),
        });
    }

    let external_type = ExternalTypeBindingSpec {
        external_type: ExternalTypeSpec::Proto(Symbol::new(proto_name.clone())),
        reference,
        type_name,
        replacement,
        authored_type: resolve_authored_type_def_kind(resolve, type_def, path, &context)?
            .map(|field_type| authored_field_type_for_language(field_type, language)),
    };

    Ok(Some((proto_name, external_type)))
}

fn build_fields_from_record(
    resolve: &Resolve,
    owner: TypeOwner,
    record: &Record,
    doc: LanguageStringSpec,
    path: &Path,
    context: &str,
    language: Language,
) -> Result<(
    LanguageStringSpec,
    indexmap::IndexMap<String, RecordFieldSpec>,
)> {
    let mut authored_proto_fields = BTreeSet::new();
    let mut fields = indexmap::IndexMap::new();
    for field in &record.fields {
        let field_context = format!("{context} field `{}`", field.name);
        let directives = parse_directives(field.docs.contents.as_deref(), path, &field_context)?;
        reject_misplaced_type_parameter(&directives, path, &field_context)?;
        let generated_field_name = directive(&directives, "name", path, &field_context)?
            .and_then(|directive| directive_language_value(directive, language))
            .unwrap_or(&field.name)
            .to_string();
        let omit_directive = directive(&directives, "omit", path, &field_context)?;
        let api_omit_directive = directive(&directives, "api-omit", path, &field_context)?;
        let proto_field_name =
            directive_value(&directives, "proto-field", path, &field_context, "value")?
                .unwrap_or_else(|| field.name.to_snake_case());
        let function_directive = directive(&directives, "function", path, &field_context)?;
        let default_directive = directive(&directives, "default", path, &field_context)?;
        let flattened_function_type = if omit_directive.is_none() && function_directive.is_none() {
            find_flattened_function_type_spec(resolve, &field.ty, path, language)?
        } else {
            None
        };

        if !authored_proto_fields.insert(proto_field_name.clone()) {
            return Err(Error::InvalidWit {
                path: path.to_path_buf(),
                reason: format!(
                    "{field_context} maps to duplicate proto field `{proto_field_name}`"
                ),
            });
        }

        if omit_directive.is_some() && api_omit_directive.is_some() {
            return Err(Error::InvalidWitDirective {
                path: path.to_path_buf(),
                context: field_context,
                directive: "@nexus.api-omit".to_string(),
                reason: "cannot be combined with `@nexus.omit`".to_string(),
            });
        }

        if let Some(omit_directive) = omit_directive {
            if !omit_directive.args.is_empty() {
                return Err(Error::InvalidWitDirective {
                    path: path.to_path_buf(),
                    context: field_context,
                    directive: "@nexus.omit".to_string(),
                    reason: "field-level omit does not take arguments".to_string(),
                });
            }

            for conflicting_directive in ["source", "type", "flattened-type", "function", "default"]
            {
                if directive(&directives, conflicting_directive, path, &field_context)?.is_some() {
                    return Err(Error::InvalidWitDirective {
                        path: path.to_path_buf(),
                        context: field_context,
                        directive: "@nexus.omit".to_string(),
                        reason: format!("cannot be combined with `@nexus.{conflicting_directive}`"),
                    });
                }
            }

            if flattened_function_type.is_some() {
                return Err(Error::InvalidWitDirective {
                    path: path.to_path_buf(),
                    context: field_context,
                    directive: "@nexus.omit".to_string(),
                    reason: "cannot be combined with a flattened function field type".to_string(),
                });
            }

            let field_type = authored_field_type_for_language(
                resolve_authored_field_type_spec(resolve, &field.ty, path, &field_context)?,
                language,
            );
            fields.insert(
                proto_field_name,
                RecordFieldSpec {
                    name: generated_field_name.clone(),
                    doc: None,
                    annotation: None,
                    flattened_annotation: None,
                    field_type,
                    default_value: None,
                    required: false,
                    visibility: RecordFieldVisibility::Omitted,
                    function: None,
                    data: (),
                },
            );
            continue;
        }

        let field_doc = directive(&directives, "doc", path, &field_context)?
            .map(directive_language_string)
            .filter(|doc| !doc.is_empty());
        let field_type = authored_field_type_for_language(
            resolve_authored_field_type_spec(resolve, &field.ty, path, &field_context)?,
            language,
        );
        let field_default =
            build_field_default(resolve, &field.ty, default_directive, path, &field_context)?;
        let required = !is_optional_type(resolve, &field.ty) && field_default.is_none();
        let source = build_source_call(&directives, path, &field_context, language)?;
        if let Some(api_omit_directive) = api_omit_directive {
            if !api_omit_directive.args.is_empty() {
                return Err(Error::InvalidWitDirective {
                    path: path.to_path_buf(),
                    context: field_context,
                    directive: "@nexus.api-omit".to_string(),
                    reason: "field-level api-omit does not take arguments".to_string(),
                });
            }
            if source.is_some() {
                return Err(Error::InvalidWitDirective {
                    path: path.to_path_buf(),
                    context: field_context,
                    directive: "@nexus.api-omit".to_string(),
                    reason: "cannot be combined with `@nexus.source`".to_string(),
                });
            }
        }
        let annotation =
            directive(&directives, "type", path, &field_context)?.map(directive_language_string);
        let flattened_annotation = directive(&directives, "flattened-type", path, &field_context)?
            .map(directive_language_string);
        let function =
            build_function_field(resolve, owner, &directives, path, &field_context, language)?;

        if source.is_some() && function.is_some() {
            return Err(Error::ConflictingTypeOverrideFieldProperties {
                message: context.to_string(),
                field: proto_field_name,
                property: "source",
                conflicting_property: "function",
            });
        }

        fields.insert(
            proto_field_name.clone(),
            RecordFieldSpec {
                name: generated_field_name,
                doc: field_doc,
                annotation,
                flattened_annotation,
                field_type,
                default_value: field_default,
                required,
                visibility: source
                    .map(|source_expr| RecordFieldVisibility::Sourced { source_expr })
                    .unwrap_or_else(|| {
                        if api_omit_directive.is_some() {
                            RecordFieldVisibility::ApiOmitted
                        } else {
                            RecordFieldVisibility::Public
                        }
                    }),
                function,
                data: (),
            },
        );

        if let Some(flattened_function_type) = flattened_function_type {
            let arg_field_count = flattened_function_type.arg_fields.len();
            for arg_field in flattened_function_type.arg_fields {
                if !authored_proto_fields.insert(arg_field.field_name.clone()) {
                    return Err(Error::InvalidWit {
                        path: path.to_path_buf(),
                        reason: format!(
                            "{field_context} maps to duplicate proto field `{}`",
                            arg_field.field_name
                        ),
                    });
                }
                let arg_doc = generated_function_arg_doc(&field.name, &arg_field, arg_field_count);
                fields.insert(
                    arg_field.field_name,
                    RecordFieldSpec {
                        name: arg_field.args_name,
                        doc: Some(arg_doc),
                        annotation: None,
                        flattened_annotation: None,
                        field_type: authored_field_type_for_language(
                            arg_field.field_type,
                            language,
                        ),
                        default_value: None,
                        required: arg_field.required,
                        visibility: RecordFieldVisibility::Public,
                        function: None,
                        data: (),
                    },
                );
            }
            if let Some(function) = flattened_function_type.function {
                fields
                    .get_mut(&proto_field_name)
                    .expect("flattened function owner field should be inserted")
                    .function = Some(function);
            }
        }
    }

    Ok((doc, fields))
}

fn generated_function_arg_doc(
    function_field_name: &str,
    arg_field: &FlattenedFunctionArgSpec,
    arg_field_count: usize,
) -> LanguageStringSpec {
    let function_name = human_field_name(function_field_name);
    let arg_name = human_field_name(&arg_field.args_name);
    let doc = if arg_field_count == 1
        && (matches!(arg_field.field_type.without_option(), TypeSpec::List(_))
            || arg_name.ends_with("args")
            || arg_name.ends_with("arguments"))
    {
        format!("Arguments for the {function_name}.")
    } else {
        format!("The {arg_name} argument for the {function_name}.")
    };
    LanguageStringSpec {
        default: Some(doc),
        by_language: BTreeMap::new(),
        default_import: None,
        imports: BTreeMap::new(),
    }
}

fn human_field_name(name: &str) -> String {
    name.replace(['-', '_'], " ")
}

fn build_type_replacement(
    directives: &[Directive],
    path: &Path,
    context: &str,
    type_name: &str,
    language: Language,
) -> Result<Option<TypeReplacementSpec>> {
    let directive = directive(directives, "type", path, context)?;
    let Some(directive) = directive else {
        return Ok(None);
    };

    let type_name_spec = directive_language_string(directive);
    let from_proto = directive_prefixed_language_string(directive, "from");
    let to_proto = directive_prefixed_language_string(directive, "to");
    if type_name_spec.is_empty() {
        if !from_proto.is_empty() || !to_proto.is_empty() {
            return Err(Error::IncompleteTypeOverride {
                type_name: type_name.to_string(),
            });
        }
        return Ok(None);
    }
    if type_name_spec.for_language(language).is_none() {
        return Ok(None);
    }
    Ok(Some(TypeReplacementSpec {
        type_name: type_name_spec,
        from_proto,
        to_proto,
    }))
}

fn resolve_authored_field_type_spec(
    resolve: &Resolve,
    ty: &Type,
    path: &Path,
    context: &str,
) -> Result<TypeSpec> {
    match ty {
        Type::Bool => Ok(TypeSpec::Bool),
        Type::U8
        | Type::U16
        | Type::U32
        | Type::U64
        | Type::S8
        | Type::S16
        | Type::S32
        | Type::S64 => Ok(TypeSpec::Int(IntSpec::I32)),
        Type::F32 | Type::F64 => Ok(TypeSpec::Float),
        Type::Char | Type::String => Ok(TypeSpec::String),
        Type::Id(id) => {
            let type_def = &resolve.types[*id];
            let type_name = type_def.name.as_deref().unwrap_or("unnamed-type");
            let type_context = format!("{context} type `{type_name}`");
            let directives =
                parse_directives(type_def.docs.contents.as_deref(), path, &type_context)?;
            if directive(&directives, "type-parameter", path, &type_context)?.is_some() {
                return Ok(TypeSpec::TypeParameter(TypeParameterSpec {
                    name: type_name.to_upper_camel_case(),
                    full_name: wit_type_full_name(resolve, *id),
                }));
            }
            if let Some(proto_name) = find_proto_name_for_type_def(type_def, path, &type_context)? {
                if let Some(resource_name) =
                    find_owned_resource_name_for_type_def(resolve, type_def)
                    && directive(&directives, "type", path, &type_context)?.is_none()
                {
                    return Ok(TypeSpec::Resource(AuthoredResourceType::new(resource_name)));
                }
                if let Some(type_directive) = directive(&directives, "type", path, &type_context)? {
                    return Ok(TypeSpec::External(ExternalTypeSpec::Alias {
                        name: Symbol::new(wit_type_full_name(resolve, *id)),
                        target: Box::new(TypeSpec::External(ExternalTypeSpec::Proto(Symbol::new(
                            proto_name,
                        )))),
                        type_name: directive_language_string(type_directive),
                    }));
                }
                return Ok(TypeSpec::External(ExternalTypeSpec::Proto(Symbol::new(
                    proto_name,
                ))));
            }
            match &type_def.kind {
                TypeDefKind::Option(inner) => Ok(TypeSpec::Option(Box::new(
                    resolve_authored_field_type_spec(resolve, inner, path, &type_context)?,
                ))),
                TypeDefKind::List(inner) => Ok(TypeSpec::List(Box::new(
                    resolve_authored_field_type_spec(resolve, inner, path, &type_context)?,
                ))),
                TypeDefKind::Tuple(tuple) => Ok(TypeSpec::Tuple(
                    tuple
                        .types
                        .iter()
                        .map(|item| {
                            resolve_authored_field_type_spec(resolve, item, path, &type_context)
                        })
                        .collect::<Result<Vec<_>>>()?,
                )),
                TypeDefKind::Map(key, value) => Ok(TypeSpec::Map(
                    Box::new(resolve_authored_field_type_spec(
                        resolve,
                        key,
                        path,
                        &type_context,
                    )?),
                    Box::new(resolve_authored_field_type_spec(
                        resolve,
                        value,
                        path,
                        &type_context,
                    )?),
                )),
                TypeDefKind::Result(result) => Ok(TypeSpec::Result {
                    ok: result
                        .ok
                        .as_ref()
                        .map(|ok| {
                            resolve_authored_field_type_spec(resolve, ok, path, &type_context)
                                .map(Box::new)
                        })
                        .transpose()?,
                    err: result
                        .err
                        .as_ref()
                        .map(|err| {
                            resolve_authored_field_type_spec(resolve, err, path, &type_context)
                                .map(Box::new)
                        })
                        .transpose()?,
                }),
                TypeDefKind::Type(next) => {
                    let target =
                        resolve_authored_field_type_spec(resolve, next, path, &type_context)?;
                    if let Some(type_directive) =
                        directive(&directives, "type", path, &type_context)?
                    {
                        Ok(TypeSpec::External(ExternalTypeSpec::Alias {
                            name: Symbol::new(wit_type_full_name(resolve, *id)),
                            target: Box::new(target),
                            type_name: directive_language_string(type_directive),
                        }))
                    } else if let Some(function_directive) =
                        directive(&directives, "function", path, &type_context)?
                    {
                        if let Some(type_name) = function_alias_type_name(
                            resolve,
                            type_def,
                            function_directive,
                            path,
                            &type_context,
                        )? {
                            Ok(TypeSpec::External(ExternalTypeSpec::Alias {
                                name: Symbol::new(wit_type_full_name(resolve, *id)),
                                target: Box::new(target),
                                type_name,
                            }))
                        } else {
                            Ok(target)
                        }
                    } else {
                        Ok(target)
                    }
                }
                TypeDefKind::Record(_) => Ok(TypeSpec::Record(Symbol::new(wit_type_full_name(
                    resolve, *id,
                )))),
                TypeDefKind::Enum(_) => Ok(TypeSpec::Enum(Symbol::new(wit_type_full_name(
                    resolve, *id,
                )))),
                TypeDefKind::Flags(_) => Ok(TypeSpec::Flags(Symbol::new(wit_type_full_name(
                    resolve, *id,
                )))),
                TypeDefKind::Variant(_) => Ok(TypeSpec::Variant(Symbol::new(wit_type_full_name(
                    resolve, *id,
                )))),
                TypeDefKind::Handle(Handle::Own(resource_id))
                | TypeDefKind::Handle(Handle::Borrow(resource_id)) => {
                    let resource_def = &resolve.types[*resource_id];
                    let resource_name = resource_def.name.as_deref().unwrap_or("unnamed-resource");
                    Ok(TypeSpec::Resource(AuthoredResourceType::new(resource_name)))
                }
                TypeDefKind::Resource => {
                    Ok(TypeSpec::Resource(AuthoredResourceType::new(type_name)))
                }
                _ => Err(Error::InvalidWit {
                    path: path.to_path_buf(),
                    reason: format!(
                        "{type_context} uses unsupported WIT type `{}` for generated model fields",
                        type_def.kind.as_str()
                    ),
                }),
            }
        }
        _ => Err(Error::InvalidWit {
            path: path.to_path_buf(),
            reason: format!("{context} uses unsupported WIT type for generated model fields"),
        }),
    }
}

pub(crate) fn find_proto_name_for_type(
    resolve: &Resolve,
    ty: &Type,
    path: &Path,
    context: &str,
) -> Result<Option<String>> {
    let mut current = ty;
    loop {
        match current {
            Type::Id(id) => {
                let type_def = &resolve.types[*id];
                let type_name = type_def.name.as_deref().unwrap_or("unnamed-type");
                let type_context = format!("{context} type `{type_name}`");
                let directives =
                    parse_directives(type_def.docs.contents.as_deref(), path, &type_context)?;
                if let Some(proto_name) =
                    directive_value(&directives, "proto", path, &type_context, "value")?
                {
                    return Ok(Some(proto_name));
                }
                match &type_def.kind {
                    TypeDefKind::Type(next) => current = next,
                    _ => return Ok(None),
                }
            }
            _ => return Ok(None),
        }
    }
}

fn find_owned_resource_name_for_type(resolve: &Resolve, ty: &Type) -> Option<String> {
    match ty {
        Type::Id(id) => find_owned_resource_name_for_type_def(resolve, &resolve.types[*id]),
        _ => None,
    }
}

fn find_owned_resource_name_for_type_def(resolve: &Resolve, type_def: &TypeDef) -> Option<String> {
    match &type_def.kind {
        TypeDefKind::Handle(Handle::Own(resource_id)) => resolve.types[*resource_id]
            .name
            .as_deref()
            .map(str::to_string),
        TypeDefKind::Type(next) => find_owned_resource_name_for_type(resolve, next),
        _ => None,
    }
}

fn resolve_authored_type_def_kind(
    resolve: &Resolve,
    type_def: &TypeDef,
    path: &Path,
    context: &str,
) -> Result<Option<TypeSpec>> {
    match &type_def.kind {
        TypeDefKind::Type(next) => Ok(Some(resolve_authored_field_type_spec(
            resolve, next, path, context,
        )?)),
        TypeDefKind::Option(inner) => Ok(Some(TypeSpec::Option(Box::new(
            resolve_authored_field_type_spec(resolve, inner, path, context)?,
        )))),
        TypeDefKind::List(inner) => Ok(Some(TypeSpec::List(Box::new(
            resolve_authored_field_type_spec(resolve, inner, path, context)?,
        )))),
        TypeDefKind::Tuple(tuple) => Ok(Some(TypeSpec::Tuple(
            tuple
                .types
                .iter()
                .map(|item| resolve_authored_field_type_spec(resolve, item, path, context))
                .collect::<Result<Vec<_>>>()?,
        ))),
        TypeDefKind::Map(key, value) => Ok(Some(TypeSpec::Map(
            Box::new(resolve_authored_field_type_spec(
                resolve, key, path, context,
            )?),
            Box::new(resolve_authored_field_type_spec(
                resolve, value, path, context,
            )?),
        ))),
        TypeDefKind::Result(result) => Ok(Some(TypeSpec::Result {
            ok: result
                .ok
                .as_ref()
                .map(|ok| {
                    resolve_authored_field_type_spec(resolve, ok, path, context).map(Box::new)
                })
                .transpose()?,
            err: result
                .err
                .as_ref()
                .map(|err| {
                    resolve_authored_field_type_spec(resolve, err, path, context).map(Box::new)
                })
                .transpose()?,
        })),
        _ => Ok(None),
    }
}

fn find_wit_record_name_for_type(resolve: &Resolve, ty: &Type) -> Option<String> {
    match ty {
        Type::Id(id) => find_wit_record_name_for_type_def(resolve, *id, &resolve.types[*id]),
        _ => None,
    }
}

fn find_wit_record_name_for_type_def(
    resolve: &Resolve,
    type_id: TypeId,
    type_def: &TypeDef,
) -> Option<String> {
    match &type_def.kind {
        TypeDefKind::Record(_) => Some(wit_type_full_name(resolve, type_id)),
        TypeDefKind::Type(next) => find_wit_record_name_for_type(resolve, next),
        _ => None,
    }
}

pub(crate) fn find_proto_name_for_type_def(
    type_def: &TypeDef,
    path: &Path,
    context: &str,
) -> Result<Option<String>> {
    let directives = parse_directives(type_def.docs.contents.as_deref(), path, context)?;
    directive_value(&directives, "proto", path, context, "value")
}

fn build_field_default(
    resolve: &Resolve,
    ty: &Type,
    directive: Option<&Directive>,
    path: &Path,
    context: &str,
) -> Result<Option<FieldDefaultSpec>> {
    let Some(directive) = directive else {
        return Ok(None);
    };
    let Some(case_name) = directive.value("value") else {
        return Err(Error::InvalidWitDirective {
            path: path.to_path_buf(),
            context: context.to_string(),
            directive: "@nexus.default".to_string(),
            reason: "missing required default enum case".to_string(),
        });
    };
    if case_name.is_empty() {
        return Err(Error::InvalidWitDirective {
            path: path.to_path_buf(),
            context: context.to_string(),
            directive: "@nexus.default".to_string(),
            reason: "default enum case cannot be empty".to_string(),
        });
    }
    let enum_value = resolve_default_enum_value(resolve, ty, case_name, path, context)?;
    Ok(Some(FieldDefaultSpec {
        enum_case: case_name.to_string(),
        enum_value,
    }))
}

fn resolve_default_enum_value(
    resolve: &Resolve,
    ty: &Type,
    case_name: &str,
    path: &Path,
    context: &str,
) -> Result<i32> {
    match ty {
        Type::Id(id) => resolve_default_enum_value_for_type_def(
            resolve,
            *id,
            &resolve.types[*id],
            case_name,
            path,
            context,
        ),
        _ => Err(default_on_non_enum_error(path, context)),
    }
}

fn resolve_default_enum_value_for_type_def(
    resolve: &Resolve,
    type_id: TypeId,
    type_def: &TypeDef,
    case_name: &str,
    path: &Path,
    context: &str,
) -> Result<i32> {
    match &type_def.kind {
        TypeDefKind::Enum(enumeration) => {
            for (index, case) in enumeration.cases.iter().enumerate() {
                if case.name == case_name {
                    return i32::try_from(index).map_err(|_| Error::InvalidWitDirective {
                        path: path.to_path_buf(),
                        context: context.to_string(),
                        directive: "@nexus.default".to_string(),
                        reason: "enum case index does not fit in i32".to_string(),
                    });
                }
            }
            Err(Error::InvalidWitDirective {
                path: path.to_path_buf(),
                context: context.to_string(),
                directive: "@nexus.default".to_string(),
                reason: format!(
                    "unknown enum case `{case_name}` for `{}`",
                    wit_type_full_name(resolve, type_id)
                ),
            })
        }
        TypeDefKind::Option(inner) | TypeDefKind::Type(inner) => {
            resolve_default_enum_value(resolve, inner, case_name, path, context)
        }
        _ => Err(default_on_non_enum_error(path, context)),
    }
}

fn default_on_non_enum_error(path: &Path, context: &str) -> Error {
    Error::InvalidWitDirective {
        path: path.to_path_buf(),
        context: context.to_string(),
        directive: "@nexus.default".to_string(),
        reason: "only enum field defaults are supported".to_string(),
    }
}

fn build_source_call(
    directives: &[Directive],
    path: &Path,
    context: &str,
    language: Language,
) -> Result<Option<String>> {
    let Some(directive) = directive(directives, "source", path, context)? else {
        return Ok(None);
    };

    let Some(helper_name) =
        directive_language_value(directive, language).or_else(|| directive.value("value"))
    else {
        return Ok(None);
    };

    Ok(Some(helper_name.to_string()))
}

fn is_valid_support_helper_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

fn is_valid_support_helper_path(name: &str) -> bool {
    name.split('.').all(is_valid_support_helper_name)
}

fn build_function_field(
    resolve: &Resolve,
    owner: TypeOwner,
    directives: &[Directive],
    path: &Path,
    context: &str,
    language: Language,
) -> Result<Option<FunctionFieldSpec>> {
    let Some(directive) = directive(directives, "function", path, context)? else {
        return Ok(None);
    };

    let result = directive_result_language_string(directive);
    if result.is_empty() {
        return Ok(None);
    }

    let Some(args_field) = directive.value("args-field") else {
        return Err(Error::InvalidWitDirective {
            path: path.to_path_buf(),
            context: context.to_string(),
            directive: "@nexus.function".to_string(),
            reason: "missing required `args-field`".to_string(),
        });
    };

    let primary = directive
        .value("primary")
        .map(parse_bool)
        .transpose()
        .map_err(|reason| Error::InvalidWitDirective {
            path: path.to_path_buf(),
            context: context.to_string(),
            directive: "@nexus.function".to_string(),
            reason,
        })?
        .unwrap_or(false);

    Ok(Some(FunctionFieldSpec {
        primary,
        result: FunctionResultSpec::Annotation(result),
        args_field: args_field.to_snake_case(),
        arg_fields: vec![args_field.to_snake_case()],
        args: FunctionArgsSpec::Varargs {
            prefix: Vec::new(),
            typescript_drop_prefix: false,
        },
        alternate_type: function_alternate_type(resolve, owner, directive, path, context)?
            .map(|field_type| authored_field_type_for_language(field_type, language)),
        converter: directive_converter(directive, language),
        name_extractor: directive_function_name_extractor(directive, language, path, context)?,
        call_extractor: directive_function_call_extractor(directive, language, path, context)?,
        result_type_parameter: directive_result_type_parameter(directive),
        type_descriptor: function_type_descriptor(directive, path, context)?,
    }))
}

fn build_function_field_for_type_alias(
    resolve: &Resolve,
    type_def: &TypeDef,
    directives: &[Directive],
    path: &Path,
    context: &str,
    language: Language,
) -> Result<Option<(String, TypeSpec, FunctionFieldSpec)>> {
    let Some(function_directive) = directive(directives, "function", path, context)? else {
        return Ok(None);
    };

    let primary = function_directive
        .value("primary")
        .map(parse_bool)
        .transpose()
        .map_err(|reason| Error::InvalidWitDirective {
            path: path.to_path_buf(),
            context: context.to_string(),
            directive: "@nexus.function".to_string(),
            reason,
        })?
        .unwrap_or(false);

    let converter = directive_converter(function_directive, language);

    if let Some(signature_name) = function_directive.value("signature") {
        if function_directive.value("args-name").is_some()
            || !directive_result_language_string(function_directive).is_empty()
        {
            return Err(Error::InvalidWitDirective {
                path: path.to_path_buf(),
                context: context.to_string(),
                directive: "@nexus.function".to_string(),
                reason: "signature cannot be combined with args-name or result overrides"
                    .to_string(),
            });
        }

        let mut signature =
            resolve_function_signature(resolve, type_def, signature_name, path, context)?;
        signature.args.field_type =
            authored_field_type_for_language(signature.args.field_type, language);
        signature.args.function_args =
            function_args_for_language(signature.args.function_args, language);
        signature.result = authored_field_type_for_language(signature.result, language);
        let args_name = signature.args.name;
        let args_type = signature.args.field_type;
        let function_args = signature.args.function_args;
        let args_field = function_directive
            .value("args-field")
            .unwrap_or(&args_name)
            .to_snake_case();
        let arg_fields = function_arg_fields(&function_args, &args_field);
        return Ok(Some((
            args_name.clone(),
            args_type,
            FunctionFieldSpec {
                primary,
                result: FunctionResultSpec::Authored(signature.result),
                args_field,
                arg_fields,
                args: function_args,
                alternate_type: function_alternate_type(
                    resolve,
                    type_def.owner,
                    function_directive,
                    path,
                    context,
                )?
                .map(|field_type| authored_field_type_for_language(field_type, language)),
                converter,
                name_extractor: directive_function_name_extractor(
                    function_directive,
                    language,
                    path,
                    context,
                )?,
                call_extractor: directive_function_call_extractor(
                    function_directive,
                    language,
                    path,
                    context,
                )?,
                result_type_parameter: directive_result_type_parameter(function_directive),
                type_descriptor: function_type_descriptor(function_directive, path, context)?,
            },
        )));
    }

    let result = directive_result_language_string(function_directive);
    if result.is_empty() {
        return Ok(None);
    }

    Err(Error::InvalidWitDirective {
        path: path.to_path_buf(),
        context: context.to_string(),
        directive: "@nexus.function".to_string(),
        reason: "type-level function annotations must use `signature`".to_string(),
    })
}

fn function_alternate_type(
    resolve: &Resolve,
    owner: TypeOwner,
    directive: &Directive,
    path: &Path,
    context: &str,
) -> Result<Option<TypeSpec>> {
    directive
        .value("alternate-type")
        .map(|type_name| {
            resolve_named_wit_type(resolve, owner, type_name, path, context, "@nexus.function")
        })
        .transpose()
}

fn function_type_descriptor(
    directive: &Directive,
    path: &Path,
    context: &str,
) -> Result<Option<FunctionTypeDescriptorSpec>> {
    let value_type = directive_language_string_for_key(directive, "value-type");
    let args_type = directive_language_string_for_key(directive, "args-type");
    if value_type.is_empty() != args_type.is_empty() {
        return Err(Error::InvalidWitDirective {
            path: path.to_path_buf(),
            context: context.to_string(),
            directive: "@nexus.function".to_string(),
            reason: "`value-type` and `args-type` must be specified together".to_string(),
        });
    }
    Ok(
        (!value_type.is_empty()).then_some(FunctionTypeDescriptorSpec {
            value_type,
            args_type,
        }),
    )
}

fn function_args_for_language(args: FunctionArgsSpec, language: Language) -> FunctionArgsSpec {
    match args {
        FunctionArgsSpec::Varargs {
            prefix,
            typescript_drop_prefix,
        } => FunctionArgsSpec::Varargs {
            prefix: prefix
                .into_iter()
                .map(|arg| function_arg_for_language(arg, language))
                .collect(),
            typescript_drop_prefix,
        },
        FunctionArgsSpec::Fixed(args) => FunctionArgsSpec::Fixed(
            args.into_iter()
                .map(|arg| function_arg_for_language(arg, language))
                .collect(),
        ),
    }
}

fn function_arg_for_language(arg: FunctionArgSpec, language: Language) -> FunctionArgSpec {
    FunctionArgSpec {
        name: arg.name,
        field_type: authored_field_type_for_language(arg.field_type, language),
    }
}

fn function_arg_fields(function_args: &FunctionArgsSpec, args_field: &str) -> Vec<String> {
    match function_args {
        FunctionArgsSpec::Varargs { .. } => vec![args_field.to_string()],
        FunctionArgsSpec::Fixed(args) => args.iter().map(|arg| arg.name.to_snake_case()).collect(),
    }
}

fn flattened_function_arg_fields(
    function: &FunctionFieldSpec,
    args_name: &str,
    args_type: &TypeSpec,
) -> Vec<FlattenedFunctionArgSpec> {
    match &function.args {
        FunctionArgsSpec::Varargs { .. } => vec![FlattenedFunctionArgSpec {
            args_name: args_name.to_string(),
            field_name: function.args_field.clone(),
            field_type: args_type.clone(),
            required: false,
        }],
        FunctionArgsSpec::Fixed(args) => args
            .iter()
            .map(|arg| FlattenedFunctionArgSpec {
                args_name: arg.name.clone(),
                field_name: arg.name.to_snake_case(),
                field_type: arg.field_type.clone(),
                required: true,
            })
            .collect(),
    }
}

fn find_flattened_function_type_spec(
    resolve: &Resolve,
    ty: &Type,
    path: &Path,
    language: Language,
) -> Result<Option<FlattenedFunctionTypeSpec>> {
    let mut current = ty;
    loop {
        match current {
            Type::Id(id) => {
                let type_def = &resolve.types[*id];
                let type_name = type_def.name.as_deref().unwrap_or("unnamed-type");
                let context = format!("type `{type_name}`");
                let directives =
                    parse_directives(type_def.docs.contents.as_deref(), path, &context)?;
                let function = build_function_field_for_type_alias(
                    resolve,
                    type_def,
                    &directives,
                    path,
                    &context,
                    language,
                )?;
                if let Some((args_name, args_type, function)) = function {
                    let arg_fields =
                        flattened_function_arg_fields(&function, &args_name, &args_type);
                    return Ok(Some(FlattenedFunctionTypeSpec {
                        arg_fields,
                        function: Some(function),
                    }));
                }
                match &type_def.kind {
                    TypeDefKind::Type(next) => current = next,
                    _ => return Ok(None),
                }
            }
            _ => return Ok(None),
        }
    }
}

fn function_alias_type_name(
    resolve: &Resolve,
    type_def: &TypeDef,
    function_directive: &Directive,
    path: &Path,
    context: &str,
) -> Result<Option<LanguageStringSpec>> {
    let result = if let Some(signature_name) = function_directive.value("signature") {
        resolve_function_signature(resolve, type_def, signature_name, path, context)?.result
    } else {
        let result = directive_result_language_string(function_directive);
        if result.is_empty() {
            return Ok(None);
        }
        TypeSpec::External(ExternalTypeSpec::Alias {
            name: Symbol::new(""),
            target: Box::new(TypeSpec::String),
            type_name: result,
        })
    };
    let mut result_type = authored_type_language_string(&result);
    if result_type.is_empty() {
        return Ok(None);
    }
    if let Some(type_parameter) = directive_result_type_parameter(function_directive) {
        replace_type_parameter_for_language(
            &mut result_type,
            Language::Python,
            &type_parameter,
            "typing.Any",
        );
        replace_type_parameter_for_language(
            &mut result_type,
            Language::TypeScript,
            &type_parameter,
            "any",
        );
    }

    let mut type_name = LanguageStringSpec::default();
    if let Some(result_type) = result_type.for_language(Language::Python) {
        type_name.by_language.insert(
            Language::Python,
            format!("str | collections.abc.Callable[..., {result_type}]"),
        );
    }
    if let Some(result_type) = result_type.for_language(Language::TypeScript) {
        type_name.by_language.insert(
            Language::TypeScript,
            format!("string | ((...args: any[]) => {result_type})"),
        );
    }
    Ok((!type_name.is_empty()).then_some(type_name))
}

fn authored_type_language_string(authored_type: &TypeSpec) -> LanguageStringSpec {
    match authored_type {
        TypeSpec::External(ExternalTypeSpec::Alias {
            type_name, target, ..
        }) => {
            if type_name.is_empty() {
                authored_type_language_string(target)
            } else {
                type_name.clone()
            }
        }
        _ => LanguageStringSpec::default(),
    }
}

fn replace_type_parameter_for_language(
    spec: &mut LanguageStringSpec,
    language: Language,
    type_parameter: &str,
    replacement: &str,
) {
    if let Some(value) = spec.by_language.get_mut(&language) {
        *value = value.replace(type_parameter, replacement);
    }
}

fn resolve_function_signature(
    resolve: &Resolve,
    type_def: &TypeDef,
    signature_name: &str,
    path: &Path,
    context: &str,
) -> Result<ResolvedFunctionSignature> {
    let TypeOwner::Interface(interface_id) = type_def.owner else {
        return Err(Error::InvalidWitDirective {
            path: path.to_path_buf(),
            context: context.to_string(),
            directive: "@nexus.function".to_string(),
            reason: "signature is only supported on interface-owned types".to_string(),
        });
    };
    let interface = &resolve.interfaces[interface_id];
    let Some(function) = interface.functions.get(signature_name) else {
        return Err(Error::InvalidWitDirective {
            path: path.to_path_buf(),
            context: context.to_string(),
            directive: "@nexus.function".to_string(),
            reason: format!("unknown signature `{signature_name}`"),
        });
    };
    let args = resolve_function_signature_args_for_function(
        resolve,
        function,
        path,
        context,
        signature_name,
    )?;
    let function_context = format!("{context} signature `{signature_name}`");
    let Some(result_type) = &function.result else {
        return Err(Error::InvalidWitDirective {
            path: path.to_path_buf(),
            context: context.to_string(),
            directive: "@nexus.function".to_string(),
            reason: format!("signature `{signature_name}` must declare a result type"),
        });
    };

    Ok(ResolvedFunctionSignature {
        args,
        result: resolve_authored_field_type_spec(resolve, result_type, path, &function_context)?,
    })
}

fn resolve_named_wit_type(
    resolve: &Resolve,
    owner: TypeOwner,
    type_name: &str,
    path: &Path,
    context: &str,
    directive_name: &str,
) -> Result<TypeSpec> {
    if let Some(primitive) = primitive_wit_type(type_name) {
        return Ok(primitive);
    }
    let TypeOwner::Interface(interface_id) = owner else {
        return Err(Error::InvalidWitDirective {
            path: path.to_path_buf(),
            context: context.to_string(),
            directive: directive_name.to_string(),
            reason: "`alternate-type` is only supported for interface-owned types".to_string(),
        });
    };
    let interface = &resolve.interfaces[interface_id];
    let Some(type_id) = interface.types.get(type_name) else {
        return Err(Error::InvalidWitDirective {
            path: path.to_path_buf(),
            context: context.to_string(),
            directive: directive_name.to_string(),
            reason: format!("unknown alternate type `{type_name}`"),
        });
    };
    resolve_authored_field_type_spec(resolve, &Type::Id(*type_id), path, context)
}

fn primitive_wit_type(type_name: &str) -> Option<TypeSpec> {
    match type_name {
        "bool" => Some(TypeSpec::Bool),
        "u8" | "u16" | "u32" | "u64" | "s8" | "s16" | "s32" | "s64" => {
            Some(TypeSpec::Int(IntSpec::I32))
        }
        "f32" | "f64" => Some(TypeSpec::Float),
        "char" | "string" => Some(TypeSpec::String),
        "bytes" => Some(TypeSpec::Bytes),
        _ => None,
    }
}

pub(crate) fn resolve_function_signature_args(
    resolve: &Resolve,
    type_def: &TypeDef,
    signature_name: &str,
    path: &Path,
    context: &str,
) -> Result<(String, TypeSpec)> {
    let TypeOwner::Interface(interface_id) = type_def.owner else {
        return Err(Error::InvalidWitDirective {
            path: path.to_path_buf(),
            context: context.to_string(),
            directive: "@nexus.function".to_string(),
            reason: "signature is only supported on interface-owned types".to_string(),
        });
    };
    let interface = &resolve.interfaces[interface_id];
    let Some(function) = interface.functions.get(signature_name) else {
        return Err(Error::InvalidWitDirective {
            path: path.to_path_buf(),
            context: context.to_string(),
            directive: "@nexus.function".to_string(),
            reason: format!("unknown signature `{signature_name}`"),
        });
    };

    let args = resolve_function_signature_args_for_function(
        resolve,
        function,
        path,
        context,
        signature_name,
    )?;
    Ok((args.name, args.field_type))
}

fn resolve_function_signature_args_for_function(
    resolve: &Resolve,
    function: &Function,
    path: &Path,
    context: &str,
    signature_name: &str,
) -> Result<ResolvedFunctionSignatureArgs> {
    if !matches!(
        function.kind,
        FunctionKind::Freestanding | FunctionKind::AsyncFreestanding
    ) {
        return Err(Error::InvalidWitDirective {
            path: path.to_path_buf(),
            context: context.to_string(),
            directive: "@nexus.function".to_string(),
            reason: format!(
                "signature `{signature_name}` must be a freestanding interface function"
            ),
        });
    }
    if function.params.is_empty() {
        return Err(Error::InvalidWitDirective {
            path: path.to_path_buf(),
            context: context.to_string(),
            directive: "@nexus.function".to_string(),
            reason: format!("signature `{signature_name}` must have at least one parameter"),
        });
    }

    let function_args = function
        .params
        .iter()
        .map(|param| {
            let function_context = format!(
                "{context} signature `{signature_name}` parameter `{}`",
                param.name
            );
            Ok(FunctionArgSpec {
                name: param.name.clone(),
                field_type: resolve_authored_field_type_spec(
                    resolve,
                    &param.ty,
                    path,
                    &function_context,
                )?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let varargs = function_varargs(function, path, context, signature_name)?;
    let field_type = if let Some(varargs) = &varargs {
        function_args
            .iter()
            .find(|arg| arg.name == varargs.param)
            .map(|arg| arg.field_type.clone())
            .expect("validated varargs param should exist")
    } else {
        function_args_field_type(&function_args)
    };
    Ok(ResolvedFunctionSignatureArgs {
        name: if let Some(varargs) = varargs.clone() {
            varargs.param
        } else if function.params.len() == 1 {
            function_args[0].name.clone()
        } else {
            "args".to_string()
        },
        function_args: if let Some(varargs) = varargs {
            let varargs_index = function_args
                .iter()
                .position(|arg| arg.name == varargs.param)
                .expect("validated varargs param should exist");
            FunctionArgsSpec::Varargs {
                prefix: function_args[..varargs_index].to_vec(),
                typescript_drop_prefix: varargs.typescript_drop_prefix,
            }
        } else {
            FunctionArgsSpec::Fixed(function_args)
        },
        field_type,
    })
}

fn function_varargs(
    function: &Function,
    path: &Path,
    context: &str,
    signature_name: &str,
) -> Result<Option<FunctionVarargsSpec>> {
    let function_context = format!("{context} signature `{signature_name}`");
    let directives = parse_directives(function.docs.contents.as_deref(), path, &function_context)?;
    let Some(directive) = directive(&directives, "function-args", path, &function_context)? else {
        return Ok(None);
    };
    let varargs = directive
        .value("varargs")
        .map(parse_bool)
        .transpose()
        .map_err(|reason| Error::InvalidWitDirective {
            path: path.to_path_buf(),
            context: function_context.clone(),
            directive: "@nexus.function-args".to_string(),
            reason,
        })?
        .unwrap_or(false);
    if !varargs {
        return Ok(None);
    }
    let typescript_drop_prefix = directive
        .value("typescript-drop-prefix")
        .map(parse_bool)
        .transpose()
        .map_err(|reason| Error::InvalidWitDirective {
            path: path.to_path_buf(),
            context: function_context.clone(),
            directive: "@nexus.function-args".to_string(),
            reason,
        })?
        .unwrap_or(false);
    let param_name = if let Some(param_name) = directive.value("param") {
        param_name.to_string()
    } else if function.params.len() == 1 {
        function.params[0].name.clone()
    } else {
        return Err(Error::InvalidWitDirective {
            path: path.to_path_buf(),
            context: function_context,
            directive: "@nexus.function-args".to_string(),
            reason:
                "`param` is required when varargs is used on a signature with multiple parameters"
                    .to_string(),
        });
    };
    if !function.params.iter().any(|param| param.name == param_name) {
        return Err(Error::InvalidWitDirective {
            path: path.to_path_buf(),
            context: function_context,
            directive: "@nexus.function-args".to_string(),
            reason: format!("unknown varargs parameter `{param_name}`"),
        });
    }
    if function
        .params
        .last()
        .is_none_or(|param| param.name != param_name)
    {
        return Err(Error::InvalidWitDirective {
            path: path.to_path_buf(),
            context: function_context,
            directive: "@nexus.function-args".to_string(),
            reason: format!("varargs parameter `{param_name}` must be the final parameter"),
        });
    }
    Ok(Some(FunctionVarargsSpec {
        param: param_name,
        typescript_drop_prefix,
    }))
}

fn function_args_field_type(args: &[FunctionArgSpec]) -> TypeSpec {
    if let Some(first) = args.first()
        && args.iter().all(|arg| arg.field_type == first.field_type)
    {
        return TypeSpec::List(Box::new(first.field_type.clone()));
    }
    TypeSpec::Tuple(
        args.iter()
            .map(|arg| arg.field_type.clone())
            .collect::<Vec<_>>(),
    )
}

fn build_service(
    resolve: &Resolve,
    key: &WorldKey,
    interface: &Interface,
    path: &Path,
    language: Language,
) -> Result<ServiceSpec> {
    let interface_name = interface_export_name(key, interface);
    let context = format!("interface `{interface_name}`");
    let directives = parse_directives(interface.docs.contents.as_deref(), path, &context)?;
    reject_misplaced_type_parameter(&directives, path, &context)?;
    let endpoint = directive_value_for_language(&directives, "endpoint", path, &context, language)?;
    let service_name = interface_name.to_upper_camel_case();
    let wire_service_name = build_wire_service_name(&directives, path, &context, &service_name)?;
    let namespace = directive(&directives, "namespace", path, &context)?
        .map(directive_language_string)
        .unwrap_or_default();
    let operations_class = directive(&directives, "operations-class", path, &context)?
        .map(directive_language_string)
        .unwrap_or_default();
    let experimental = experimental_directive(&directives, path, &context)?;
    let delay_load_temporalio_workflow =
        delay_load_temporalio_workflow_directive(&directives, path, &context)?;

    let operations = interface
        .functions
        .iter()
        .filter(|(_, function)| {
            matches!(
                function.kind,
                FunctionKind::Freestanding | FunctionKind::AsyncFreestanding
            )
        })
        .map(|(_, function)| build_operation(resolve, function, path, &context, &service_name))
        .collect::<Result<Vec<_>>>()?;
    ensure_unique_wire_operation_names(path, &context, &operations)?;

    let mut resources = Vec::new();
    for type_id in interface.types.values() {
        let type_def = &resolve.types[*type_id];
        if !matches!(type_def.kind, TypeDefKind::Resource) {
            continue;
        }
        resources.push(build_resource(
            resolve, interface, *type_id, type_def, path, &context, language,
        )?);
    }

    Ok(ServiceSpec {
        name: service_name,
        code_name: LanguageStringSpec::default(),
        wire_name: wire_service_name,
        doc: LanguageStringSpec::default(),
        namespace,
        operations_class,
        endpoint,
        experimental,
        deprecated: false,
        delay_load_temporalio_workflow,
        operations,
        resources,
        data: (),
    })
}

fn build_wire_service_name(
    directives: &[Directive],
    path: &Path,
    context: &str,
    default_wire_service_name: &str,
) -> Result<String> {
    let Some(directive) = directive(directives, "service-name", path, context)? else {
        return Ok(default_wire_service_name.to_string());
    };
    let Some(name) = directive.value("name").or_else(|| directive.value("value")) else {
        return Err(Error::InvalidWitDirective {
            path: path.to_path_buf(),
            context: context.to_string(),
            directive: "@nexus.service-name".to_string(),
            reason: "missing required `name`".to_string(),
        });
    };
    if name.is_empty() {
        return Err(Error::InvalidWitDirective {
            path: path.to_path_buf(),
            context: context.to_string(),
            directive: "@nexus.service-name".to_string(),
            reason: "`name` cannot be empty".to_string(),
        });
    }
    Ok(name.to_string())
}

fn ensure_unique_wire_operation_names(
    path: &Path,
    context: &str,
    operations: &[OperationSpec],
) -> Result<()> {
    let mut seen = BTreeMap::<String, String>::new();
    for operation in operations {
        if let Some(existing) = seen.insert(operation.wire_name.clone(), operation.name.clone()) {
            return Err(Error::InvalidWit {
                path: path.to_path_buf(),
                reason: format!(
                    "{context} operations `{existing}` and `{}` both use Nexus operation name `{}`",
                    operation.name, operation.wire_name
                ),
            });
        }
    }
    Ok(())
}

fn build_resource(
    resolve: &Resolve,
    interface: &Interface,
    resource_id: TypeId,
    type_def: &TypeDef,
    path: &Path,
    service_context: &str,
    language: Language,
) -> Result<ResourceSpec> {
    let resource_name = type_def
        .name
        .as_deref()
        .ok_or_else(|| Error::InvalidWit {
            path: path.to_path_buf(),
            reason: format!("{service_context} declares an unnamed resource"),
        })?
        .to_string();
    let context = format!(
        "{service_context} resource `{}`",
        resource_name.to_upper_camel_case()
    );

    let constructor = interface.functions.values().find(
        |function| matches!(function.kind, FunctionKind::Constructor(id) if id == resource_id),
    );
    if let Some(constructor) = constructor {
        let constructor_context = format!("{context} constructor");
        let directives = parse_directives(
            constructor.docs.contents.as_deref(),
            path,
            &constructor_context,
        )?;
        reject_misplaced_type_parameter(&directives, path, &constructor_context)?;
    }
    let fields = match constructor {
        Some(constructor) => constructor
            .params
            .iter()
            .map(|param| {
                build_resource_field(
                    resolve,
                    &param.name,
                    &param.ty,
                    path,
                    &context,
                    "constructor",
                    language,
                )
            })
            .collect::<Result<Vec<_>>>()?,
        None => Vec::new(),
    };

    let methods = interface
        .functions
        .values()
        .filter(|function| {
            matches!(
                function.kind,
                FunctionKind::Method(id) | FunctionKind::AsyncMethod(id) if id == resource_id
            )
        })
        .map(|function| build_resource_method(resolve, function, path, &context, language))
        .collect::<Result<Vec<_>>>()?;

    for function in interface.functions.values() {
        match function.kind {
            FunctionKind::Static(id) | FunctionKind::AsyncStatic(id) if id == resource_id => {
                return Err(Error::InvalidWit {
                    path: path.to_path_buf(),
                    reason: format!(
                        "{context} static methods are not supported yet (`{}`)",
                        function.name
                    ),
                });
            }
            _ => {}
        }
    }

    Ok(ResourceSpec {
        name: resource_name,
        fields,
        methods,
        data: (),
    })
}

fn build_resource_method(
    resolve: &Resolve,
    function: &Function,
    path: &Path,
    resource_context: &str,
    language: Language,
) -> Result<ResourceMethodSpec> {
    let method_name = function
        .name
        .rsplit('.')
        .next()
        .unwrap_or(function.name.as_str())
        .to_string();
    let context = format!(
        "{resource_context} method `{}`",
        method_name.to_upper_camel_case()
    );
    let directives = parse_directives(function.docs.contents.as_deref(), path, &context)?;
    reject_misplaced_type_parameter(&directives, path, &context)?;
    let params = function
        .params
        .iter()
        .skip_while(|param| param.name == "self")
        .map(|param| {
            build_resource_field(
                resolve,
                &param.name,
                &param.ty,
                path,
                &context,
                "parameter",
                language,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    let result = function
        .result
        .as_ref()
        .map(|ty| build_resource_result(resolve, ty, path, &context, language))
        .transpose()?;

    Ok(ResourceMethodSpec {
        name: method_name,
        params,
        result,
        operation_name: build_resource_method_operation_name(&directives, path, &context)?,
    })
}

fn build_resource_method_operation_name(
    directives: &[Directive],
    path: &Path,
    context: &str,
) -> Result<Option<String>> {
    let Some(directive) = directive(directives, "operation", path, context)? else {
        return Ok(None);
    };
    let Some(name) = directive.value("name").or_else(|| directive.value("value")) else {
        return Err(Error::InvalidWitDirective {
            path: path.to_path_buf(),
            context: context.to_string(),
            directive: "@nexus.operation".to_string(),
            reason: "missing required `name`".to_string(),
        });
    };
    if name.is_empty() {
        return Err(Error::InvalidWitDirective {
            path: path.to_path_buf(),
            context: context.to_string(),
            directive: "@nexus.operation".to_string(),
            reason: "`name` cannot be empty".to_string(),
        });
    }
    Ok(Some(name.to_upper_camel_case()))
}

fn build_resource_result(
    resolve: &Resolve,
    ty: &Type,
    path: &Path,
    context: &str,
    language: Language,
) -> Result<ResourceResultSpec> {
    Ok(ResourceResultSpec {
        result_type: authored_field_type_for_language(
            resolve_authored_field_type_spec(resolve, ty, path, context)?,
            language,
        ),
    })
}

fn build_resource_field(
    resolve: &Resolve,
    name: &str,
    ty: &Type,
    path: &Path,
    context: &str,
    _role: &str,
    language: Language,
) -> Result<ResourceFieldSpec> {
    let field_type = authored_field_type_for_language(
        resolve_authored_field_type_spec(resolve, ty, path, context)?,
        language,
    );
    let function = find_flattened_function_type_spec(resolve, ty, path, language)?
        .and_then(|function_type| function_type.function);
    Ok(ResourceFieldSpec {
        name: name.to_string(),
        optional: is_optional_type(resolve, ty),
        field_type,
        function,
    })
}

fn build_operation(
    resolve: &Resolve,
    function: &Function,
    path: &Path,
    service_context: &str,
    service_name: &str,
) -> Result<OperationSpec> {
    let operation_name = function.name.to_upper_camel_case();
    let context = format!("{service_context} operation `{operation_name}`");
    let directives = parse_directives(function.docs.contents.as_deref(), path, &context)?;
    reject_misplaced_type_parameter(&directives, path, &context)?;
    let wire_operation_name =
        build_wire_operation_name(&directives, path, &context, &operation_name)?;
    let experimental = experimental_directive(&directives, path, &context)?;

    let [parameter] = function.params.as_slice() else {
        return Err(Error::InvalidWit {
            path: path.to_path_buf(),
            reason: format!("{context} must declare exactly one input parameter"),
        });
    };
    let parameter_name = &parameter.name;
    let input_type = &parameter.ty;
    let input = find_operation_type_spec(resolve, input_type, path, &context)?.ok_or_else(|| {
        Error::InvalidWit {
            path: path.to_path_buf(),
            reason: format!(
                "{context} parameter `{parameter_name}` type must resolve to a record, resource, or type annotated with `@nexus.proto`"
            ),
        }
    })?;
    let output_transform = build_operation_output_transform(
        &directives,
        path,
        &context,
        service_name,
        &operation_name,
    )?;
    let serialization_context = build_operation_serialization_context(&directives, path, &context)?;
    let output = if let Some(output_type) = function.result.as_ref() {
        Some(
            find_operation_type_spec(resolve, output_type, path, &context)?.ok_or_else(|| {
                Error::InvalidWit {
                    path: path.to_path_buf(),
                    reason: format!(
                        "{context} result type must resolve to a record, resource, or type annotated with `@nexus.proto`"
                    ),
                }
            })?,
        )
    } else {
        if output_transform.is_some() {
            return Err(Error::InvalidWitDirective {
                path: path.to_path_buf(),
                context,
                directive: "@nexus.output-transform".to_string(),
                reason: "operation does not declare a result type".to_string(),
            });
        }
        None
    };

    Ok(OperationSpec {
        name: operation_name,
        code_name: LanguageStringSpec::default(),
        wire_name: wire_operation_name,
        experimental,
        deprecated: false,
        doc: directive(&directives, "doc", path, &context)?
            .map(directive_language_string)
            .unwrap_or_default(),
        return_doc: directive(&directives, "doc", path, &context)?
            .map(directive_returns_language_string)
            .unwrap_or_default(),
        input: Some(input),
        output,
        output_transform,
        serialization_context,
        data: (),
    })
}

fn find_operation_type_spec(
    resolve: &Resolve,
    ty: &Type,
    path: &Path,
    context: &str,
) -> Result<Option<TypeSpec>> {
    if let Some(resource_name) = find_owned_resource_name_for_type(resolve, ty) {
        let wire_type = find_proto_name_for_type(resolve, ty, path, context)?
            .map(|proto_name| ExternalTypeSpec::Proto(Symbol::new(proto_name)));
        let alias = match ty {
            Type::Id(type_id)
                if resolve.types[*type_id].name.is_some()
                    && !matches!(resolve.types[*type_id].kind, TypeDefKind::Resource) =>
            {
                let type_def = &resolve.types[*type_id];
                Some(DeclaredTypeName {
                    name: type_def
                        .name
                        .as_deref()
                        .expect("type name checked above")
                        .to_upper_camel_case(),
                    full_name: wit_type_full_name(resolve, *type_id),
                })
            }
            _ => None,
        };
        return Ok(Some(TypeSpec::Resource(AuthoredResourceType {
            name: Symbol::new(resource_name),
            wire_type,
            alias,
        })));
    }
    if let Some(proto_name) = find_proto_name_for_type(resolve, ty, path, context)? {
        return Ok(Some(TypeSpec::External(ExternalTypeSpec::Proto(
            Symbol::new(proto_name),
        ))));
    }
    if let Some(record_name) = find_wit_record_name_for_type(resolve, ty) {
        return Ok(Some(TypeSpec::Record(Symbol::new(record_name))));
    }
    Ok(None)
}

fn build_wire_operation_name(
    directives: &[Directive],
    path: &Path,
    context: &str,
    default_wire_operation_name: &str,
) -> Result<String> {
    let Some(directive) = directive(directives, "operation", path, context)? else {
        return Ok(default_wire_operation_name.to_string());
    };
    let Some(name) = directive.value("name").or_else(|| directive.value("value")) else {
        return Err(Error::InvalidWitDirective {
            path: path.to_path_buf(),
            context: context.to_string(),
            directive: "@nexus.operation".to_string(),
            reason: "missing required `name`".to_string(),
        });
    };
    if name.is_empty() {
        return Err(Error::InvalidWitDirective {
            path: path.to_path_buf(),
            context: context.to_string(),
            directive: "@nexus.operation".to_string(),
            reason: "`name` cannot be empty".to_string(),
        });
    }
    Ok(name.to_string())
}

pub(crate) fn wire_operation_name_from_docs(
    docs: Option<&str>,
    path: &Path,
    context: &str,
    default_wire_operation_name: &str,
) -> Result<String> {
    let directives = parse_directives(docs, path, context)?;
    build_wire_operation_name(&directives, path, context, default_wire_operation_name)
}

fn build_operation_output_transform(
    directives: &[Directive],
    path: &Path,
    context: &str,
    service_name: &str,
    operation_name: &str,
) -> Result<Option<OperationOutputTransformSpec>> {
    let Some(directive) = directive(directives, "output-transform", path, context)? else {
        return Ok(None);
    };

    let type_name = directive_prefixed_language_string(directive, "type");
    let transform = directive_language_string(directive);

    if type_name.is_empty() && transform.is_empty() {
        Ok(None)
    } else if !type_name.is_empty() && !transform.is_empty() {
        Ok(Some(OperationOutputTransformSpec {
            type_name,
            transform,
        }))
    } else {
        Err(Error::IncompleteOperationOutputTransform {
            service: service_name.to_string(),
            operation: operation_name.to_string(),
        })
    }
}

fn build_operation_serialization_context(
    directives: &[Directive],
    path: &Path,
    context: &str,
) -> Result<LanguageStringSpec> {
    let Some(directive) = directive(directives, "serialization-context", path, context)? else {
        return Ok(LanguageStringSpec::default());
    };

    let spec = directive_language_string(directive);
    for (language, helper) in spec
        .default
        .iter()
        .map(|helper| ("default", helper.as_str()))
        .chain(
            spec.by_language
                .iter()
                .map(|(language, helper)| (language_key(*language), helper.as_str())),
        )
    {
        if !is_valid_support_helper_path(helper) {
            return Err(Error::InvalidWitDirective {
                path: path.to_path_buf(),
                context: context.to_string(),
                directive: "@nexus.serialization-context".to_string(),
                reason: format!("invalid `{language}` support helper `{helper}`"),
            });
        }
    }
    Ok(spec)
}

pub(crate) fn select_world(
    resolve: &Resolve,
    package_id: PackageId,
    path: &Path,
) -> Result<WorldId> {
    let package = &resolve.packages[package_id];
    match package.worlds.len() {
        1 => Ok(*package
            .worlds
            .values()
            .next()
            .expect("world map length checked")),
        0 => Err(Error::InvalidWit {
            path: path.to_path_buf(),
            reason: "package must declare exactly one world".to_string(),
        }),
        _ => Err(Error::InvalidWit {
            path: path.to_path_buf(),
            reason: "package declares multiple worlds; choose one world per input".to_string(),
        }),
    }
}

fn interface_export_name(key: &WorldKey, interface: &Interface) -> String {
    match key {
        WorldKey::Name(name) => name.clone(),
        WorldKey::Interface(_) => interface
            .name
            .clone()
            .unwrap_or_else(|| "unnamed-interface".to_string()),
    }
}

fn is_optional_type(resolve: &Resolve, ty: &Type) -> bool {
    let mut current = ty;
    loop {
        match current {
            Type::Id(id) => match &resolve.types[*id].kind {
                TypeDefKind::Option(_) => return true,
                TypeDefKind::Type(next) => current = next,
                _ => return false,
            },
            _ => return false,
        }
    }
}

fn directive_value_for_language(
    directives: &[Directive],
    name: &str,
    path: &Path,
    context: &str,
    language: Language,
) -> Result<Option<String>> {
    let Some(directive) = directive(directives, name, path, context)? else {
        return Ok(None);
    };
    Ok(directive_language_value(directive, language)
        .or_else(|| directive.value("value"))
        .map(ToOwned::to_owned))
}

fn directive_language_string(directive: &Directive) -> LanguageStringSpec {
    let mut spec = LanguageStringSpec {
        default: directive.value("value").map(ToOwned::to_owned),
        by_language: BTreeMap::new(),
        default_import: None,
        imports: BTreeMap::new(),
    };
    for language in all_languages() {
        if let Some(value) = directive.value(language_key(language)) {
            spec.by_language.insert(language, value.to_string());
        }
    }
    apply_directive_imports(&mut spec, directive);
    spec
}

fn directive_prefixed_language_string(directive: &Directive, suffix: &str) -> LanguageStringSpec {
    let mut spec = LanguageStringSpec::default();
    for language in all_languages() {
        if let Some(value) = directive.value(&format!("{}-{suffix}", language_key(language))) {
            spec.by_language.insert(language, value.to_string());
        }
    }
    apply_directive_imports(&mut spec, directive);
    spec
}

fn directive_language_string_for_key(directive: &Directive, key: &str) -> LanguageStringSpec {
    let mut spec = directive_prefixed_language_string(directive, key);
    spec.default = directive.value(key).map(ToOwned::to_owned);
    spec
}

fn directive_returns_language_string(directive: &Directive) -> LanguageStringSpec {
    let mut spec = directive_prefixed_language_string(directive, "returns");
    spec.default = directive.value("returns").map(ToOwned::to_owned);
    spec
}

fn directive_result_language_string(directive: &Directive) -> LanguageStringSpec {
    let mut spec = directive_prefixed_language_string(directive, "result");
    spec.default = directive.value("result").map(ToOwned::to_owned);
    spec
}

fn apply_directive_imports(spec: &mut LanguageStringSpec, directive: &Directive) {
    spec.default_import = directive.value("import").map(ToOwned::to_owned);
    for language in all_languages() {
        if let Some(value) = directive.value(&format!("{}-import", language_key(language))) {
            spec.imports.insert(language, value.to_string());
        }
    }
}

fn directive_converter(directive: &Directive, language: Language) -> Option<String> {
    let mut spec = directive_prefixed_language_string(directive, "converter");
    spec.default = directive.value("converter").map(ToOwned::to_owned);
    spec.for_language(language).map(ToOwned::to_owned)
}

fn directive_function_name_extractor(
    directive: &Directive,
    language: Language,
    path: &Path,
    context: &str,
) -> Result<Option<String>> {
    let mut spec = directive_prefixed_language_string(directive, "name-extractor");
    spec.default = directive.value("name-extractor").map(ToOwned::to_owned);
    let Some(extractor) = spec.for_language(language) else {
        return Ok(None);
    };
    if !is_valid_support_helper_path(extractor) {
        return Err(Error::InvalidWitDirective {
            path: path.to_path_buf(),
            context: context.to_string(),
            directive: "@nexus.function".to_string(),
            reason: format!(
                "invalid `{}` name-extractor `{extractor}`",
                language_key(language)
            ),
        });
    }
    Ok(Some(extractor.to_string()))
}

fn directive_function_call_extractor(
    directive: &Directive,
    language: Language,
    path: &Path,
    context: &str,
) -> Result<Option<String>> {
    let mut spec = directive_prefixed_language_string(directive, "call-extractor");
    spec.default = directive.value("call-extractor").map(ToOwned::to_owned);
    let Some(extractor) = spec.for_language(language) else {
        return Ok(None);
    };
    if !is_valid_support_helper_path(extractor) {
        return Err(Error::InvalidWitDirective {
            path: path.to_path_buf(),
            context: context.to_string(),
            directive: "@nexus.function".to_string(),
            reason: format!(
                "invalid `{}` call-extractor `{extractor}`",
                language_key(language)
            ),
        });
    }
    Ok(Some(extractor.to_string()))
}

fn directive_result_type_parameter(directive: &Directive) -> Option<String> {
    directive
        .value("result-type-parameter")
        .map(ToOwned::to_owned)
}

pub(crate) fn directive_value(
    directives: &[Directive],
    name: &str,
    path: &Path,
    context: &str,
    key: &str,
) -> Result<Option<String>> {
    Ok(directive(directives, name, path, context)?
        .and_then(|directive| directive.value(key))
        .map(ToOwned::to_owned))
}

fn experimental_directive(directives: &[Directive], path: &Path, context: &str) -> Result<bool> {
    let Some(directive) = directive(directives, "experimental", path, context)? else {
        return Ok(false);
    };
    if !directive.args.is_empty() {
        return Err(Error::InvalidWitDirective {
            path: path.to_path_buf(),
            context: context.to_string(),
            directive: "@nexus.experimental".to_string(),
            reason: "does not take arguments".to_string(),
        });
    }
    Ok(true)
}

fn delay_load_temporalio_workflow_directive(
    directives: &[Directive],
    path: &Path,
    context: &str,
) -> Result<bool> {
    let Some(directive) = directive(directives, "delay-load-temporalio-workflow", path, context)?
    else {
        return Ok(false);
    };
    if !directive.args.is_empty() {
        return Err(Error::InvalidWitDirective {
            path: path.to_path_buf(),
            context: context.to_string(),
            directive: "@nexus.delay-load-temporalio-workflow".to_string(),
            reason: "does not take arguments".to_string(),
        });
    }
    Ok(true)
}

pub(crate) fn directive<'a>(
    directives: &'a [Directive],
    name: &str,
    path: &Path,
    context: &str,
) -> Result<Option<&'a Directive>> {
    let mut matches = directives.iter().filter(|directive| directive.name == name);
    let first = matches.next();
    if matches.next().is_some() {
        return Err(Error::InvalidWitDirective {
            path: path.to_path_buf(),
            context: context.to_string(),
            directive: format!("@nexus.{name}"),
            reason: "duplicate directive".to_string(),
        });
    }
    Ok(first)
}

fn reject_misplaced_type_parameter(
    directives: &[Directive],
    path: &Path,
    context: &str,
) -> Result<()> {
    if directive(directives, "type-parameter", path, context)?.is_some() {
        return Err(Error::InvalidWitDirective {
            path: path.to_path_buf(),
            context: context.to_string(),
            directive: "@nexus.type-parameter".to_string(),
            reason: "is only supported on WIT type aliases".to_string(),
        });
    }
    Ok(())
}

fn directive_language_value<'a>(directive: &'a Directive, language: Language) -> Option<&'a str> {
    directive.value(language_key(language))
}

fn all_languages() -> [Language; 6] {
    [
        Language::Dotnet,
        Language::Go,
        Language::Java,
        Language::Python,
        Language::Ruby,
        Language::TypeScript,
    ]
}

fn language_key(language: Language) -> &'static str {
    match language {
        Language::Dotnet => "dotnet",
        Language::Go => "go",
        Language::Java => "java",
        Language::Python => "python",
        Language::Ruby => "ruby",
        Language::TypeScript => "typescript",
    }
}

pub(crate) fn parse_directives(
    docs: Option<&str>,
    path: &Path,
    context: &str,
) -> Result<Vec<Directive>> {
    let Some(docs) = docs else {
        return Ok(Vec::new());
    };

    let mut directives = Vec::new();
    let mut current = None::<String>;

    for line in docs.lines() {
        let trimmed_start = line.trim_start();
        if trimmed_start.starts_with("@nexus.") {
            if let Some(previous) = current.take() {
                directives.push(parse_directive_line(&previous, path, context)?);
            }
            current = Some(trimmed_start.to_string());
            continue;
        }

        let is_continuation = current.is_some()
            && !trimmed_start.is_empty()
            && trimmed_start.len() != line.len()
            && (trimmed_start.starts_with('"') || trimmed_start.contains('='));

        if is_continuation {
            let directive = current
                .as_mut()
                .expect("continuation checked to have an active directive");
            directive.push(' ');
            directive.push_str(trimmed_start);
            continue;
        }

        if let Some(previous) = current.take() {
            directives.push(parse_directive_line(&previous, path, context)?);
        }
    }

    if let Some(previous) = current.take() {
        directives.push(parse_directive_line(&previous, path, context)?);
    }

    Ok(directives)
}

#[derive(Debug, Clone)]
pub(crate) struct Directive {
    name: String,
    args: BTreeMap<String, String>,
}

impl Directive {
    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn value(&self, key: &str) -> Option<&str> {
        self.args.get(key).map(String::as_str)
    }
}

fn parse_directive_line(line: &str, path: &Path, context: &str) -> Result<Directive> {
    let Some(rest) = line.strip_prefix("@nexus.") else {
        return Err(Error::InvalidWitDirective {
            path: path.to_path_buf(),
            context: context.to_string(),
            directive: line.to_string(),
            reason: "directive must start with `@nexus.`".to_string(),
        });
    };

    let name_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    let name = &rest[..name_end];
    let mut tail = rest[name_end..].trim_start();
    let mut args = BTreeMap::new();

    if tail.starts_with('"') {
        let (value, remaining) = parse_directive_value(tail, path, context, name)?;
        args.insert("value".to_string(), value);
        tail = remaining.trim_start();
    }

    while !tail.is_empty() {
        let key_end = tail
            .find(|character: char| character == '=' || character.is_whitespace())
            .unwrap_or(tail.len());
        let key = &tail[..key_end];
        let after_key = tail[key_end..].trim_start();
        let Some(after_equals) = after_key.strip_prefix('=') else {
            return Err(Error::InvalidWitDirective {
                path: path.to_path_buf(),
                context: context.to_string(),
                directive: format!("@nexus.{name}"),
                reason: format!("expected `=` after `{key}`"),
            });
        };
        let (value, remaining) =
            parse_directive_value(after_equals.trim_start(), path, context, name)?;
        args.insert(key.to_string(), value);
        tail = remaining.trim_start();
    }

    Ok(Directive {
        name: name.to_string(),
        args,
    })
}

fn parse_directive_value<'a>(
    input: &'a str,
    path: &Path,
    context: &str,
    name: &str,
) -> Result<(String, &'a str)> {
    if let Some(stripped) = input.strip_prefix('"') {
        let mut escaped = false;
        let mut value = String::new();
        for (index, character) in stripped.char_indices() {
            if escaped {
                value.push(character);
                escaped = false;
                continue;
            }
            match character {
                '\\' => escaped = true,
                '"' => return Ok((value, &stripped[index + 1..])),
                _ => value.push(character),
            }
        }

        return Err(Error::InvalidWitDirective {
            path: path.to_path_buf(),
            context: context.to_string(),
            directive: format!("@nexus.{name}"),
            reason: "unterminated quoted string".to_string(),
        });
    }

    let end = input.find(char::is_whitespace).unwrap_or(input.len());
    Ok((input[..end].to_string(), &input[end..]))
}

fn parse_bool(value: &str) -> std::result::Result<bool, String> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!("expected `true` or `false`, found `{value}`")),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::descriptors::DescriptorIndex;
    use crate::error::Error;
    use crate::language::Language;
    use crate::spec::CompilerPass;

    use super::{
        ApiSpec, AuthoredResourceType, DeclaredTypeName, ExternalTypeSpec, FunctionArgSpec,
        FunctionArgsSpec, Symbol, TypeSpec, directive, parse_directives,
    };

    fn root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    fn descriptors() -> DescriptorIndex {
        DescriptorIndex::load(&root().join("advanced/samples/descriptors/temporal_api.bin"))
            .unwrap()
    }

    fn linked_inputs_path() -> PathBuf {
        root().join("advanced/samples/inputs/deps")
    }

    fn parse(language: Language, wit: &str) -> ApiSpec {
        crate::parser::parse_api_spec_from_wit_for_language_with_inputs(
            language,
            wit,
            PathBuf::from("inline.wit"),
            &[linked_inputs_path()],
        )
        .unwrap()
    }

    fn parse_result(language: Language, wit: &str) -> Result<ApiSpec, Error> {
        crate::parser::parse_api_spec_from_wit_for_language_with_inputs(
            language,
            wit,
            PathBuf::from("inline.wit"),
            &[linked_inputs_path()],
        )
    }

    fn validate(language: Language, wit: &str) -> Result<(), Error> {
        let spec = parse_result(language, wit)?;
        let descriptors = descriptors();
        crate::planning::AuthoredValidationPass::new(&descriptors, language)
            .apply(crate::spec::ApiSpecTree::single(spec))
            .map(|_| ())
    }

    fn unique_temp_dir(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("nexgen-{label}-{unique}"))
    }

    #[test]
    fn records_public_variant_tags_wire_names_and_operation_free_exports() {
        let spec = parse(
            Language::Python,
            r#"
package test:exports@1.0.0;
world system { export types; }
interface types {
  variant choice {
    some-value(string),
    %type(string),
  }
}
"#,
        );
        let variant = spec.variant("types.choice").expect("variant should parse");
        assert_eq!(variant.cases[0].name, "some-value");
        assert_eq!(variant.cases[0].wire_name, "some_value");
        assert_eq!(variant.cases[1].name, "type");
        assert_eq!(variant.cases[1].wire_name, "type");
        assert!(spec.types["types.choice"].is_module_export());

        let service = parse(
            Language::Python,
            r#"
package test:exports@1.0.0;
world system { export api; }
interface api {
  record request { value: string, }
  call: func(request: request);
}
"#,
        );
        assert!(
            service
                .types
                .values()
                .all(|entry| !entry.is_module_export())
        );
    }

    const GENERIC_WIT: &str = r#"
package temporal:nexus@1.0.0;

world system { export generic-service; }

interface generic-service {
  use nexus:temporal-types/model@1.0.0.{placeholder};

  /// @nexus.type-parameter
  type context-t = placeholder;

  /// @nexus.type-parameter
  type output-t = placeholder;

  record inner { value: context-t, }

  record request {
    nested: inner,
    values: list<context-t>,
  }

  record response {
    context: context-t,
    output: output-t,
  }

  complete: func(request: request) -> response;
}
"#;

    #[test]
    fn parses_and_inferrs_generic_record_parameters_by_alias_identity() {
        let spec = parse(Language::TypeScript, GENERIC_WIT);
        let request = spec.record("generic-service.request").unwrap();
        let request_parameters =
            spec.record_type_parameters(&request.full_name, Language::TypeScript);
        assert_eq!(request_parameters.len(), 1);
        assert_eq!(request_parameters[0].parameter.name, "ContextT");
        assert_eq!(
            request_parameters[0].parameter.full_name,
            "generic-service.context-t"
        );

        let response = spec.record("generic-service.response").unwrap();
        let response_parameters =
            spec.record_type_parameters(&response.full_name, Language::TypeScript);
        assert_eq!(
            response_parameters
                .iter()
                .map(|usage| usage.parameter.name.as_str())
                .collect::<Vec<_>>(),
            ["ContextT", "OutputT"]
        );
    }

    #[test]
    fn rejects_type_parameter_directive_on_records_and_fields() {
        let record = GENERIC_WIT.replace(
            "record inner { value: context-t, }",
            "/// @nexus.type-parameter\n  record inner { value: context-t, }",
        );
        assert!(parse_result(Language::Python, &record).is_err());

        let field = GENERIC_WIT.replace(
            "nested: inner,",
            "/// @nexus.type-parameter\n    nested: inner,",
        );
        assert!(parse_result(Language::Python, &field).is_err());
    }

    #[test]
    fn rejects_generic_map_key_parameters() {
        let wit = GENERIC_WIT.replace(
            "/// @nexus.type-parameter\n  type output-t = placeholder;",
            "/// @nexus.type-parameter\n  type output-t = placeholder;\n\n  /// @nexus.type-parameter\n  type key-t = string;\n\n  type keyed-values = map<key-t, string>;",
        ).replace(
            "values: list<context-t>,",
            "values: list<context-t>,\n    by-key: keyed-values,",
        );
        assert!(parse_result(Language::Go, &wit).is_err());
    }

    #[test]
    fn language_type_overrides_and_omitted_fields_do_not_infer_parameters() {
        let wit = GENERIC_WIT.replace(
            "record inner { value: context-t, }",
            r#"record inner { value: context-t, }

  record overridden {
    /// @nexus.type typescript="string"
    value: context-t,
    /// @nexus.omit
    hidden: output-t,
  }"#,
        );
        let typescript = parse(Language::TypeScript, &wit);
        assert!(
            typescript
                .record_type_parameters("generic-service.overridden", Language::TypeScript)
                .is_empty()
        );

        let python = parse(Language::Python, &wit);
        let parameters =
            python.record_type_parameters("generic-service.overridden", Language::Python);
        assert_eq!(parameters.len(), 1);
        assert_eq!(parameters[0].parameter.name, "ContextT");
    }

    #[test]
    fn infers_type_parameters_through_nested_variants() {
        let variant = GENERIC_WIT
            .replace(
                "/// @nexus.type-parameter\n  type output-t = placeholder;",
                "/// @nexus.type-parameter\n  type output-t = placeholder;\n\n  /// @nexus.type-parameter\n  type key-t = string;",
            )
            .replace(
                "record inner { value: context-t, }",
                "variant inner-result { value(context-t), }\n\n  variant outer-result {\n    nested(inner-result),\n    keyed(map<string, key-t>),\n    output(output-t),\n  }\n\n  record inner { value: context-t, }",
            )
            .replace(
                "context: context-t,\n    output: output-t,",
                "result-value: outer-result,\n    repeated: context-t,",
            );
        let spec = parse(Language::Go, &variant);
        let inner = spec.variant_type_parameters("generic-service.inner-result", Language::Go);
        assert_eq!(inner[0].parameter.name, "ContextT");
        let outer = spec.variant_type_parameters("generic-service.outer-result", Language::Go);
        assert_eq!(
            outer
                .iter()
                .map(|usage| usage.parameter.name.as_str())
                .collect::<Vec<_>>(),
            ["ContextT", "KeyT", "OutputT"]
        );
        let response = spec.record_type_parameters("generic-service.response", Language::Go);
        assert_eq!(
            response
                .iter()
                .map(|usage| usage.parameter.name.as_str())
                .collect::<Vec<_>>(),
            ["ContextT", "KeyT", "OutputT"]
        );
    }

    #[test]
    fn rejects_type_parameter_arguments_and_conflicting_directives() {
        let arguments = GENERIC_WIT.replace(
            "@nexus.type-parameter\n  type context-t",
            "@nexus.type-parameter name=\"T\"\n  type context-t",
        );
        assert!(parse_result(Language::Go, &arguments).is_err());

        let conflict = GENERIC_WIT.replace(
            "@nexus.type-parameter\n  type context-t",
            "@nexus.type-parameter\n  /// @nexus.type typescript=\"string\"\n  type context-t",
        );
        assert!(parse_result(Language::TypeScript, &conflict).is_err());
    }

    #[test]
    fn parses_enum_field_defaults() {
        let wit = r#"
package temporal:nexus@1.0.0;

world system {
  export workflow-service;
}

interface workflow-service {
  use nexus:temporal-types/model@1.0.0.{workflow-id-reuse-policy};

  /// @nexus.proto "temporal.api.workflowservice.v1.SignalWithStartWorkflowExecutionRequest"
  record request {
    /// @nexus.proto-field "workflow_id_reuse_policy"
    /// @nexus.default "allow-duplicate"
    id-reuse-policy: workflow-id-reuse-policy,
  }
}
"#;

        let spec = parse(Language::Python, wit);
        let request = spec
            .record_for_proto(
                "temporal.api.workflowservice.v1.SignalWithStartWorkflowExecutionRequest",
            )
            .unwrap();
        assert!(!request.field_required("workflow_id_reuse_policy"));
        assert_eq!(
            request
                .field_default("workflow_id_reuse_policy")
                .map(|default| default.enum_value),
            Some(0)
        );
    }

    #[test]
    fn rejects_unknown_enum_field_default() {
        let wit = r#"
package temporal:nexus@1.0.0;

world system {
  export workflow-service;
}

interface workflow-service {
  use nexus:temporal-types/model@1.0.0.{workflow-id-reuse-policy};

  /// @nexus.proto "temporal.api.workflowservice.v1.SignalWithStartWorkflowExecutionRequest"
  record request {
    /// @nexus.proto-field "workflow_id_reuse_policy"
    /// @nexus.default "missing-case"
    id-reuse-policy: workflow-id-reuse-policy,
  }
}
"#;

        let error = crate::parser::parse_api_spec_from_wit_for_language_with_inputs(
            Language::Python,
            wit,
            PathBuf::from("inline.wit"),
            &[linked_inputs_path()],
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("unknown enum case `missing-case`")
        );
    }

    #[test]
    fn rejects_non_enum_field_default() {
        let wit = r#"
package temporal:nexus@1.0.0;

world system {
  export workflow-service;
}

interface workflow-service {
  /// @nexus.proto "temporal.api.workflowservice.v1.SignalWithStartWorkflowExecutionRequest"
  record request {
    /// @nexus.proto-field "request_id"
    /// @nexus.default "request-id"
    request-id: string,
  }
}
"#;

        let error = crate::parser::parse_api_spec_from_wit_for_language_with_inputs(
            Language::Python,
            wit,
            PathBuf::from("inline.wit"),
            &[linked_inputs_path()],
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("only enum field defaults are supported")
        );
    }

    #[test]
    fn ignores_language_specific_source_for_other_languages() {
        let wit = r#"
package temporal:nexus@1.0.0;

world system {
  export workflow-service;
}

interface workflow-service {
  use nexus:temporal-types/model@1.0.0.{workflow-id-reuse-policy};

  /// @nexus.proto "temporal.api.workflowservice.v1.SignalWithStartWorkflowExecutionRequest"
  record request {
    /// @nexus.proto-field "workflow_id_reuse_policy"
    /// @nexus.default "allow-duplicate"
    /// @nexus.source go="workflowIDReusePolicy(ctx)"
    id-reuse-policy: workflow-id-reuse-policy,
  }
}
"#;

        let go = crate::parser::parse_api_spec_from_wit_for_language_with_inputs(
            Language::Go,
            wit,
            PathBuf::from("inline.wit"),
            &[linked_inputs_path()],
        )
        .unwrap();
        let go_record = go
            .records()
            .find(|(_, record)| record.name == "Request")
            .unwrap()
            .1;
        assert!(matches!(
            go_record.fields["workflow_id_reuse_policy"].visibility,
            crate::spec::RecordFieldVisibility::Sourced { ref source_expr } if source_expr == "workflowIDReusePolicy(ctx)"
        ));

        let python = crate::parser::parse_api_spec_from_wit_for_language_with_inputs(
            Language::Python,
            wit,
            PathBuf::from("inline.wit"),
            &[linked_inputs_path()],
        )
        .unwrap();
        let python_record = python
            .records()
            .find(|(_, record)| record.name == "Request")
            .unwrap()
            .1;
        let field = &python_record.fields["workflow_id_reuse_policy"];
        assert!(matches!(
            field.visibility,
            crate::spec::RecordFieldVisibility::Public
        ));
        assert_eq!(
            field.default_value.as_ref().unwrap().enum_case,
            "allow-duplicate"
        );
    }

    #[test]
    fn parses_wit_into_selected_language_spec() {
        let wit = r#"
package temporal:nexus@1.0.0;

world system {
  export workflow-service;
}

/// @nexus.endpoint "__temporal_system"
/// @nexus.service-name "temporal.api.workflowservice.v1.WorkflowService"
interface workflow-service {
  use nexus:temporal-types/model@1.0.0.{placeholder, retry-policy, signal-function, workflow-function};

  /// @nexus.proto "temporal.api.workflowservice.v1.SignalWithStartWorkflowExecutionRequest"
  record signal-with-start-workflow-request {
    /// @nexus.proto-field "workflow_type"
    workflow: workflow-function,
    workflow-id: string,
    task-queue: string,
    /// @nexus.proto-field "signal_name"
    signal: signal-function,
    /// @nexus.name go="WorkflowExecutionTimeout"
    workflow-execution-timeout: placeholder,
    /// @nexus.source "workflow_namespace"
    namespace: option<string>,
    /// @nexus.omit
    header: placeholder,
    /// @nexus.omit
    time-skipping-config: placeholder,
  }

  /// @nexus.proto "temporal.api.workflowservice.v1.SignalWithStartWorkflowExecutionResponse"
  record signal-with-start-workflow-response {
    run-id: option<string>,
    started: option<bool>,
    /// @nexus.omit
    signal-link: placeholder,
  }

  /// @nexus.output-transform
  ///   python-type="temporalio.workflow.ExternalWorkflowHandle[WorkflowResult]"
  ///   python="temporalio.workflow.get_external_workflow_handle(request.workflow_id, run_id=result.run_id)"
  ///   typescript-type="workflow.ExternalWorkflowHandle"
  ///   typescript="workflow.getExternalWorkflowHandle(request.workflowId, result.runId ?? undefined)"
  ///   typescript-import="@temporalio/workflow"
  signal-with-start-workflow-execution: func(
    request: signal-with-start-workflow-request
  ) -> signal-with-start-workflow-response;
}
"#;

        let python = parse(Language::Python, wit);
        let typescript = parse(Language::TypeScript, wit);
        let dotnet = parse(Language::Dotnet, wit);
        let go = parse(Language::Go, wit);
        assert_eq!(python.services[0].name, "WorkflowService");
        assert_eq!(
            python.services[0].wire_name,
            "temporal.api.workflowservice.v1.WorkflowService"
        );
        assert_eq!(
            typescript.services[0].wire_name,
            "temporal.api.workflowservice.v1.WorkflowService"
        );
        let python_support = python.support.fragments_for_language(Language::Python);
        let typescript_support = typescript
            .support
            .fragments_for_language(Language::TypeScript);
        let dotnet_support = dotnet.support.fragments_for_language(Language::Dotnet);
        assert_eq!(python_support.len(), 1);
        assert_eq!(typescript_support.len(), 1);
        assert_eq!(dotnet_support.len(), 1);
        assert!(
            python_support[0]
                .path
                .ends_with("deps/nexus-temporal-types/python/temporal_model_converters.py")
        );
        assert!(
            typescript_support[0]
                .path
                .ends_with("deps/nexus-temporal-types/typescript/temporal_model_converters.ts")
        );
        assert!(
            python_support[0]
                .contents
                .contains("def retry_policy_from_proto(")
        );
        assert!(
            typescript_support[0]
                .contents
                .contains("export function retryPolicyFromProto(")
        );
        assert_eq!(
            dotnet_support[0].namespace.as_deref(),
            Some("Nexgen.Support")
        );
        assert!(
            python
                .external_type_binding("temporal.api.common.v1.Payloads")
                .unwrap()
                .replacement
                .is_none()
        );
        assert_eq!(
            dotnet
                .external_type_binding("temporal.api.common.v1.Payloads")
                .unwrap()
                .replacement
                .as_ref()
                .and_then(|replacement| replacement.to_proto.for_language(Language::Dotnet)),
            Some("ProtoExtensions.ToPayloads")
        );

        let request = python
            .record_for_proto(
                "temporal.api.workflowservice.v1.SignalWithStartWorkflowExecutionRequest",
            )
            .unwrap();
        assert_eq!(
            python.services[0].operations[0]
                .input_type()
                .and_then(TypeSpec::reference),
            Some("temporal.api.workflowservice.v1.SignalWithStartWorkflowExecutionRequest")
        );
        assert_eq!(
            python.services[0].operations[0]
                .output_type()
                .and_then(TypeSpec::reference),
            Some("temporal.api.workflowservice.v1.SignalWithStartWorkflowExecutionResponse")
        );
        assert!(request.field_required("workflow_type"));
        assert!(request.field_omitted("header"));
        let model = request;
        assert!(model.field_source("header").is_none());
        assert_eq!(model.field_name_override("workflow_type"), Some("workflow"));
        assert_eq!(
            model.field_name_override("workflow_execution_timeout"),
            Some("workflow-execution-timeout")
        );
        assert_eq!(
            go.record_for_proto(
                "temporal.api.workflowservice.v1.SignalWithStartWorkflowExecutionRequest",
            )
            .unwrap()
            .field_name_override("workflow_execution_timeout"),
            Some("WorkflowExecutionTimeout")
        );
        assert_eq!(model.field_name_override("input"), Some("args"));
        assert_eq!(
            model.field_name_override("workflow_id"),
            Some("workflow-id")
        );
        assert!(model.function("workflow_type").unwrap().primary);
        assert_eq!(model.function("workflow_type").unwrap().converter, None);
        assert_eq!(model.function("workflow_type").unwrap().args_field, "input");
        assert_eq!(
            model
                .function("workflow_type")
                .unwrap()
                .result_type_parameter
                .as_deref(),
            Some("WorkflowResult")
        );
        assert_eq!(
            model
                .function("workflow_type")
                .unwrap()
                .alternate_type
                .as_ref()
                .unwrap()
                .to_type_string(),
            "model.workflow-type"
        );
        assert_eq!(
            model.function("signal_name").unwrap().converter.as_deref(),
            Some("signal_function_to_proto")
        );
        assert_eq!(
            model
                .function("signal_name")
                .unwrap()
                .alternate_type
                .as_ref()
                .unwrap()
                .to_type_string(),
            "string"
        );
        assert_eq!(model.field_source("namespace"), Some("workflow_namespace"));

        let dotnet_model = dotnet
            .record_for_proto(
                "temporal.api.workflowservice.v1.SignalWithStartWorkflowExecutionRequest",
            )
            .unwrap()
            .clone();
        assert_eq!(
            dotnet_model
                .function("workflow_type")
                .unwrap()
                .call_extractor
                .as_deref(),
            Some("TemporalFunctionNames.ExtractCall")
        );
        assert_eq!(
            dotnet_model
                .function("signal_name")
                .unwrap()
                .call_extractor
                .as_deref(),
            Some("TemporalFunctionNames.ExtractCall")
        );

        let typescript_model = typescript
            .record_for_proto(
                "temporal.api.workflowservice.v1.SignalWithStartWorkflowExecutionRequest",
            )
            .unwrap()
            .clone();
        assert!(
            python
                .record_for_proto("temporal.api.sdk.v1.UserMetadata")
                .unwrap()
                .flatten_in_api
        );
        assert!(typescript_model.function("workflow_type").is_some());
        assert!(typescript_model.function("signal_name").is_some());
        assert_eq!(
            typescript_model
                .function("workflow_type")
                .unwrap()
                .converter,
            None
        );
        assert_eq!(
            typescript_model
                .function("signal_name")
                .unwrap()
                .converter
                .as_deref(),
            None
        );
        assert_eq!(
            typescript_model
                .function("signal_name")
                .unwrap()
                .name_extractor
                .as_deref(),
            Some("signalFunctionName")
        );
        assert_eq!(
            typescript_model
                .function("signal_name")
                .unwrap()
                .type_descriptor
                .as_ref()
                .and_then(|descriptor| descriptor.value_type.for_language(Language::TypeScript)),
            Some("workflow.SignalDefinition<any[]>")
        );
    }

    #[test]
    fn accepts_language_specific_source_helpers() {
        let wit = r#"
package temporal:nexus@1.0.0;

world system {
  export workflow-service;
}

interface workflow-service {
  /// @nexus.proto "temporal.api.workflowservice.v1.SignalWithStartWorkflowExecutionRequest"
  record request {
    /// @nexus.source python="workflow_namespace" typescript="workflowNamespace" dotnet="TemporalWorkflowContext.WorkflowNamespace"
    namespace: option<string>,
  }

  request-op: func(request: request) -> request;
}
"#;

        let python = crate::parser::parse_api_spec_from_wit_for_language(
            Language::Python,
            wit,
            PathBuf::from("inline.wit"),
        )
        .unwrap();
        let typescript = crate::parser::parse_api_spec_from_wit_for_language(
            Language::TypeScript,
            wit,
            PathBuf::from("inline.wit"),
        )
        .unwrap();
        let dotnet = crate::parser::parse_api_spec_from_wit_for_language(
            Language::Dotnet,
            wit,
            PathBuf::from("inline.wit"),
        )
        .unwrap();
        assert_eq!(
            python
                .record_for_proto(
                    "temporal.api.workflowservice.v1.SignalWithStartWorkflowExecutionRequest"
                )
                .unwrap()
                .field_source("namespace"),
            Some("workflow_namespace")
        );
        assert_eq!(
            typescript
                .record_for_proto(
                    "temporal.api.workflowservice.v1.SignalWithStartWorkflowExecutionRequest"
                )
                .unwrap()
                .field_source("namespace"),
            Some("workflowNamespace")
        );
        assert_eq!(
            dotnet
                .record_for_proto(
                    "temporal.api.workflowservice.v1.SignalWithStartWorkflowExecutionRequest"
                )
                .unwrap()
                .field_source("namespace"),
            Some("TemporalWorkflowContext.WorkflowNamespace")
        );
    }

    #[test]
    fn parses_language_specific_api_docs() {
        let wit = r#"
package temporal:nexus@1.0.0;

world system {
  export workflow-service;
}

interface workflow-service {
  /// @nexus.proto "temporal.api.workflowservice.v1.SignalWithStartWorkflowExecutionRequest"
  record request {
    /// @nexus.doc "Default field doc" python="Python field doc" typescript="TypeScript field doc"
    id: string,
  }

  /// @nexus.doc
  ///   "Default operation doc"
  ///   python="Python operation doc"
  ///   typescript="TypeScript operation doc"
  ///   returns="Default return doc"
  ///   python-returns="Python return doc"
  ///   typescript-returns="TypeScript return doc"
  request-op: func(request: request) -> request;
}
"#;

        let python = parse(Language::Python, wit);
        let typescript = parse(Language::TypeScript, wit);
        assert_eq!(
            python.services[0]
                .operation("RequestOp")
                .unwrap()
                .doc
                .for_language(Language::Python),
            Some("Python operation doc")
        );
        assert_eq!(
            python.services[0]
                .operation("RequestOp")
                .unwrap()
                .return_doc
                .for_language(Language::Python),
            Some("Python return doc")
        );
        assert_eq!(
            typescript.services[0]
                .operation("RequestOp")
                .unwrap()
                .doc
                .for_language(Language::TypeScript),
            Some("TypeScript operation doc")
        );
        assert_eq!(
            typescript.services[0]
                .operation("RequestOp")
                .unwrap()
                .return_doc
                .for_language(Language::TypeScript),
            Some("TypeScript return doc")
        );
        assert_eq!(
            python
                .record_for_proto(
                    "temporal.api.workflowservice.v1.SignalWithStartWorkflowExecutionRequest"
                )
                .unwrap()
                .field_doc("id")
                .and_then(|doc| doc.for_language(Language::Python)),
            Some("Python field doc")
        );
    }

    #[test]
    fn parses_experimental_annotations() {
        let wit = r#"
package temporal:nexus@1.0.0;

world system {
  export workflow-service;
}

/// @nexus.endpoint "__temporal_system"
/// @nexus.experimental
interface workflow-service {
  /// @nexus.experimental
  /// @nexus.proto "temporal.api.workflowservice.v1.SignalWithStartWorkflowExecutionRequest"
  record request {
    id: string,
  }

  /// @nexus.experimental
  request-op: func(request: request) -> request;
}
"#;

        let spec = parse(Language::Python, wit);
        assert!(spec.services[0].experimental);
        assert!(
            spec.services[0]
                .operation("RequestOp")
                .unwrap()
                .experimental
        );
        assert!(
            spec.record_for_proto(
                "temporal.api.workflowservice.v1.SignalWithStartWorkflowExecutionRequest"
            )
            .unwrap()
            .experimental
        );
        assert!(
            spec.record("workflow-service.request")
                .unwrap()
                .experimental
        );
    }

    #[test]
    fn operation_specs_use_single_input_and_output_types() {
        let wit = r#"
package temporal:users@1.0.0;

world system {
  export user-service;
}

/// @nexus.endpoint "__user_service"
interface user-service {
  resource user {
    get: func() -> user-result;
  }

  /// @nexus.proto "acme.users.v1.ProtoRequest"
  record proto-request {
  }

  record json-request {
  }

  /// @nexus.proto "acme.users.v1.UserResult"
  type user-result = own<user>;
  type user-input = own<user>;

  proto-to-resource: func(request: proto-request) -> user-result;
  json-echo: func(request: json-request) -> json-request;
  resource-input: func(request: user-input) -> json-request;
}
"#;

        let spec = crate::parser::parse_api_spec_from_wit_for_language(
            Language::Python,
            wit,
            PathBuf::from("inline.wit"),
        )
        .unwrap();
        let service = &spec.services[0];
        assert_eq!(
            spec.record("user-service.proto-request")
                .unwrap()
                .source_type
                .as_ref(),
            Some(&ExternalTypeSpec::Proto(Symbol::new(
                "acme.users.v1.ProtoRequest"
            )))
        );
        assert_eq!(
            spec.record("user-service.json-request")
                .unwrap()
                .source_type
                .as_ref(),
            None
        );

        let proto_to_resource = service.operation("ProtoToResource").unwrap();
        assert_eq!(
            proto_to_resource.input,
            Some(TypeSpec::External(ExternalTypeSpec::Proto(Symbol::new(
                "acme.users.v1.ProtoRequest"
            ))))
        );
        assert_eq!(
            proto_to_resource.output,
            Some(TypeSpec::Resource(AuthoredResourceType {
                name: Symbol::new("user"),
                wire_type: Some(ExternalTypeSpec::Proto(Symbol::new(
                    "acme.users.v1.UserResult"
                ))),
                alias: Some(DeclaredTypeName {
                    name: "UserResult".to_string(),
                    full_name: "user-service.user-result".to_string(),
                }),
            }))
        );
        assert_eq!(
            service.resource("user").unwrap().methods[0]
                .result
                .as_ref()
                .unwrap()
                .result_type,
            TypeSpec::Resource(AuthoredResourceType::new("user"))
        );

        let json_echo = service.operation("JsonEcho").unwrap();
        assert_eq!(
            json_echo.input,
            Some(TypeSpec::Record(Symbol::new("user-service.json-request")))
        );
        assert_eq!(
            json_echo.output,
            Some(TypeSpec::Record(Symbol::new("user-service.json-request")))
        );

        let resource_input = service.operation("ResourceInput").unwrap();
        assert_eq!(
            resource_input.input,
            Some(TypeSpec::Resource(AuthoredResourceType {
                name: Symbol::new("user"),
                wire_type: None,
                alias: Some(DeclaredTypeName {
                    name: "UserInput".to_string(),
                    full_name: "user-service.user-input".to_string(),
                }),
            }))
        );
    }

    #[test]
    fn parses_delay_load_temporalio_workflow_annotation() {
        let wit = r#"
package temporal:nexus@1.0.0;

world system {
  export workflow-service;
}

/// @nexus.endpoint "__temporal_system"
/// @nexus.delay-load-temporalio-workflow
interface workflow-service {
  record request {
    id: string,
  }

  request-op: func(request: request) -> request;
}
"#;

        let spec = parse(Language::Python, wit);
        assert!(spec.services[0].delay_load_temporalio_workflow);
    }

    #[test]
    fn rejects_experimental_annotation_arguments() {
        let wit = r#"
package temporal:nexus@1.0.0;

world system {
  export workflow-service;
}

/// @nexus.endpoint "__temporal_system"
/// @nexus.experimental reason="preview"
interface workflow-service {
  record request {
    id: string,
  }

  request-op: func(request: request) -> request;
}
"#;

        let error = crate::parser::parse_api_spec_from_wit_for_language(
            Language::Python,
            wit,
            PathBuf::from("inline.wit"),
        )
        .unwrap_err();
        assert!(error.to_string().contains("@nexus.experimental"));
        assert!(error.to_string().contains("does not take arguments"));
    }

    #[test]
    fn infers_python_sequence_annotation_for_wit_lists() {
        let wit = r#"
package temporal:nexus@1.0.0;

world system {
  export workflow-service;
}

/// @nexus.endpoint "__temporal_system"
interface workflow-service {
  resource started-workflow {
    get-result: func() -> list<string>;
  }
}
"#;

        let python = parse(Language::Python, wit);
        let resource = python.services[0].resource("started-workflow").unwrap();
        let method = resource
            .methods
            .iter()
            .find(|method| method.name == "get-result")
            .unwrap();
        assert_eq!(
            method.result.as_ref().unwrap().result_type.to_type_string(),
            "list<string>"
        );
    }

    #[test]
    fn records_fixed_function_signature_args_as_fields() {
        let wit = r#"
package temporal:function-execution@1.0.0;

world system {
  export function-execution;
}

interface functions {
  function-call: func(name: string, enabled: bool) -> string;

  /// @nexus.function
  ///   primary=true
  ///   signature="function-call"
  type executable-function = string;
}

/// @nexus.endpoint "function-execution"
interface function-execution {
  use functions.{executable-function};

  record execute-function-request {
    function: executable-function,
  }

  record execute-function-result {
    value: string,
  }

  execute-function: func(request: execute-function-request) -> execute-function-result;
}
"#;

        let spec = parse(Language::Python, wit);
        let request = spec
            .record("function-execution.execute-function-request")
            .unwrap();
        let model = request;
        assert_eq!(model.field_name_override("name"), Some("name"));
        assert_eq!(model.field_name_override("enabled"), Some("enabled"));
        assert_eq!(model.field_type("name").unwrap().to_type_string(), "string");
        assert_eq!(
            model
                .field_doc("name")
                .unwrap()
                .for_language(Language::Python),
            Some("The name argument for the function.")
        );
        assert_eq!(
            model.field_type("enabled").unwrap().to_type_string(),
            "bool"
        );
        assert_eq!(
            model.function("function").unwrap().arg_fields,
            vec!["name".to_string(), "enabled".to_string()]
        );
        assert_ne!(
            model.field_type("name").unwrap().to_type_string(),
            "list<string>"
        );
        assert!(model.function("function").unwrap().primary);
        assert_eq!(model.function("function").unwrap().args_field, "args");
        assert_eq!(
            model.function("function").unwrap().args,
            FunctionArgsSpec::Fixed(vec![
                FunctionArgSpec {
                    name: "name".to_string(),
                    field_type: TypeSpec::String,
                },
                FunctionArgSpec {
                    name: "enabled".to_string(),
                    field_type: TypeSpec::Bool,
                },
            ])
        );
    }

    #[test]
    fn records_varargs_function_args_from_signature_annotation() {
        let wit = r#"
package temporal:function-execution@1.0.0;

world system {
  export function-execution;
}

interface functions {
  type function-args = list<string>;

  /// @nexus.function-args varargs=true
  function-call: func(args: function-args) -> string;

  /// @nexus.function
  ///   primary=true
  ///   signature="function-call"
  ///   args-field="args"
  type executable-function = string;
}

/// @nexus.endpoint "function-execution"
interface function-execution {
  use functions.{executable-function};

  record execute-function-request {
    function: executable-function,
  }

  record execute-function-result {
    value: string,
  }

  execute-function: func(request: execute-function-request) -> execute-function-result;
}
"#;

        let spec = parse(Language::Python, wit);
        let request = spec
            .record("function-execution.execute-function-request")
            .unwrap();
        let model = request;
        assert_eq!(
            model.field_type("args").unwrap().to_type_string(),
            "list<string>"
        );
        assert_eq!(
            model
                .field_doc("args")
                .unwrap()
                .for_language(Language::Python),
            Some("Arguments for the function.")
        );
        assert_eq!(
            model.function("function").unwrap().args,
            FunctionArgsSpec::Varargs {
                prefix: Vec::new(),
                typescript_drop_prefix: false,
            }
        );
    }

    #[test]
    fn validates_proto_backed_wit_field_types_and_keeps_flattened_types_separate() {
        let wit = r#"
package temporal:nexus@1.0.0;

world system {
  export workflow-service;
}

/// @nexus.endpoint "__temporal_system"
interface workflow-service {
  use nexus:temporal-types/model@1.0.0.{payload};

  /// @nexus.proto "temporal.api.workflowservice.v1.SignalWithStartWorkflowExecutionRequest"
  record request {
    /// @nexus.proto-field "memo"
    /// @nexus.flattened-type python="str" typescript="string"
    metadata: option<payload>,
  }

  /// @nexus.proto "temporal.api.workflowservice.v1.SignalWithStartWorkflowExecutionResponse"
  record response {
    run-id: option<string>,
  }

  run: func(request: request) -> response;
}
"#;

        let python = parse(Language::Python, wit);
        let python_model = python
            .record_for_proto(
                "temporal.api.workflowservice.v1.SignalWithStartWorkflowExecutionRequest",
            )
            .unwrap()
            .clone();
        assert_eq!(
            python_model.field_type("memo").unwrap().to_type_string(),
            "option<model.payload>"
        );
        assert_eq!(python_model.field_annotation("memo"), None);
        assert_eq!(
            python_model
                .field_flattened_annotation("memo")
                .and_then(|annotation| annotation.for_language(Language::Python)),
            Some("str")
        );

        let mismatch = r#"
package temporal:nexus@1.0.0;

world system {
  export workflow-service;
}

/// @nexus.endpoint "__temporal_system"
interface workflow-service {
  use nexus:temporal-types/model@1.0.0.{placeholder, retry-policy, task-queue};

  /// @nexus.proto "temporal.api.activity.v1.ActivityOptions"
  record request {
    task-queue: option<task-queue>,
    /// @nexus.proto-field "retry_policy"
    retry-policy: option<string>,
    /// @nexus.omit
    schedule-to-close-timeout: placeholder,
    /// @nexus.omit
    schedule-to-start-timeout: placeholder,
    /// @nexus.omit
    start-to-close-timeout: placeholder,
    /// @nexus.omit
    heartbeat-timeout: placeholder,
    /// @nexus.omit
    priority: placeholder,
  }

  run: func(request: request) -> request;
}
"#;

        let error = validate(Language::Python, mismatch).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("authored field type `option<string>` does not match proto field type `option<temporal.api.common.v1.RetryPolicy>`")
        );
    }

    #[test]
    fn accumulates_linked_and_root_input_support_fragments() {
        let temp_dir = unique_temp_dir("support-fragments");
        fs::create_dir_all(&temp_dir).unwrap();
        let input_path = temp_dir.join("input.wit");
        let extra_support_path = temp_dir.join("extra_support.py");
        fs::write(
            &extra_support_path,
            "def extra_support_hook() -> str:\n    return 'extra'\n",
        )
        .unwrap();

        let wit = r#"
/// @nexus.support python="extra_support.py"
package temporal:nexus@1.0.0;

world system {
  export workflow-service;
}

interface workflow-service {
  use nexus:temporal-types/model@1.0.0.{retry-policy};

  retry-policy-operation: func(request: retry-policy) -> retry-policy;
}
"#;

        let spec = crate::parser::parse_api_spec_from_wit_for_language_with_inputs(
            Language::Python,
            wit,
            input_path,
            &[linked_inputs_path()],
        )
        .unwrap();
        let python_support = spec.support.fragments_for_language(Language::Python);
        assert_eq!(python_support.len(), 2);
        assert!(
            python_support[0]
                .path
                .ends_with("deps/nexus-temporal-types/python/temporal_model_converters.py")
        );
        assert!(python_support[1].path.ends_with("extra_support.py"));
        assert!(
            python_support[0]
                .contents
                .contains("def retry_policy_from_proto(")
        );
        assert!(
            python_support[1]
                .contents
                .contains("def extra_support_hook() -> str:")
        );

        fs::remove_dir_all(temp_dir).unwrap();
    }

    #[test]
    fn parses_sibling_wit_files_from_main_wit_package_directory() {
        let temp_dir = unique_temp_dir("sibling-wit");
        fs::create_dir_all(&temp_dir).unwrap();
        let shared_path = temp_dir.join("shared.wit");
        let input_path = temp_dir.join("main.wit");
        fs::write(
            &shared_path,
            r#"
package temporal:nexus@1.0.0;

interface shared {
  /// @nexus.proto "acme.foo.v1.LocalRetryPolicy"
  record local-retry-policy {
  }
}
"#,
        )
        .unwrap();

        let wit = r#"
package temporal:nexus@1.0.0;

world system {
  export workflow-service;
}

interface workflow-service {
  use shared.{local-retry-policy};

  retry-policy-operation: func(request: local-retry-policy) -> local-retry-policy;
}
"#;

        let spec = crate::parser::parse_api_spec_from_wit_for_language_with_inputs(
            Language::Python,
            wit,
            input_path,
            &[linked_inputs_path()],
        )
        .unwrap();
        assert_eq!(
            spec.services[0].operations[0]
                .input_type()
                .and_then(TypeSpec::reference),
            Some("acme.foo.v1.LocalRetryPolicy")
        );

        fs::remove_dir_all(temp_dir).unwrap();
    }

    #[test]
    fn ignores_sibling_wit_files_for_standalone_input_wit() {
        let temp_dir = unique_temp_dir("standalone-wit");
        fs::create_dir_all(&temp_dir).unwrap();
        let shared_path = temp_dir.join("shared.wit");
        let input_path = temp_dir.join("input.wit");
        fs::write(
            &shared_path,
            r#"
package temporal:nexus@1.0.0;

interface shared {
  /// @nexus.proto "acme.foo.v1.LocalRetryPolicy"
  record local-retry-policy {
  }
}
"#,
        )
        .unwrap();

        let wit = r#"
package temporal:nexus@1.0.0;

world system {
  export workflow-service;
}

interface workflow-service {
  use nexus:temporal-types/model@1.0.0.{retry-policy};

  retry-policy-operation: func(request: retry-policy) -> retry-policy;
}
"#;

        let spec = crate::parser::parse_api_spec_from_wit_for_language_with_inputs(
            Language::Python,
            wit,
            input_path,
            &[linked_inputs_path()],
        )
        .unwrap();
        assert_eq!(
            spec.services[0].operations[0]
                .input_type()
                .and_then(TypeSpec::reference),
            Some("temporal.api.common.v1.RetryPolicy")
        );

        fs::remove_dir_all(temp_dir).unwrap();
    }

    #[test]
    fn validates_wit_function_fields() {
        let wit = r#"
package temporal:nexus@1.0.0;

world system {
  export workflow-service;
}

/// @nexus.endpoint "__temporal_system"
interface workflow-service {
  use nexus:temporal-types/model@1.0.0.{duration, placeholder, signal-function, task-queue, workflow-function};

  /// @nexus.proto "temporal.api.workflowservice.v1.SignalWithStartWorkflowExecutionRequest"
  record signal-with-start-workflow-request {
    /// @nexus.proto-field "workflow_type"
    workflow: workflow-function,
    workflow-id: string,
    task-queue: task-queue,
    /// @nexus.proto-field "signal_name"
    signal: signal-function,
    /// @nexus.source "workflow_namespace"
    namespace: option<string>,
    /// @nexus.omit
    workflow-execution-timeout: placeholder,
    /// @nexus.omit
    workflow-run-timeout: placeholder,
    /// @nexus.omit
    workflow-task-timeout: placeholder,
    /// @nexus.omit
    identity: placeholder,
    /// @nexus.omit
    request-id: placeholder,
    /// @nexus.omit
    workflow-id-reuse-policy: placeholder,
    /// @nexus.omit
    workflow-id-conflict-policy: placeholder,
    /// @nexus.omit
    control: placeholder,
    /// @nexus.omit
    retry-policy: placeholder,
    /// @nexus.omit
    cron-schedule: placeholder,
    /// @nexus.omit
    memo: placeholder,
    /// @nexus.omit
    search-attributes: placeholder,
    /// @nexus.omit
    header: placeholder,
    workflow-start-delay: option<duration>,
    /// @nexus.omit
    user-metadata: placeholder,
    /// @nexus.omit
    links: placeholder,
    /// @nexus.omit
    versioning-override: placeholder,
    /// @nexus.omit
    priority: placeholder,
    /// @nexus.omit
    time-skipping-config: placeholder,
  }

  /// @nexus.proto "temporal.api.workflowservice.v1.SignalWithStartWorkflowExecutionResponse"
  record signal-with-start-workflow-response {
    run-id: option<string>,
    started: option<bool>,
    /// @nexus.omit
    signal-link: placeholder,
  }

  signal-with-start-workflow-execution: func(
    request: signal-with-start-workflow-request
  ) -> signal-with-start-workflow-response;
}
"#;

        validate(Language::Python, wit).unwrap();
    }

    #[test]
    fn requires_explicit_omit_for_proto_fields_left_out_of_records() {
        let wit = r#"
package temporal:nexus@1.0.0;

world system {
  export workflow-service;
}

/// @nexus.endpoint "__temporal_system"
interface workflow-service {
  use nexus:temporal-types/model@1.0.0.{placeholder, retry-policy, task-queue};

  /// @nexus.proto "temporal.api.activity.v1.ActivityOptions"
  record activity-options {
    task-queue: option<task-queue>,
    retry-policy: retry-policy,
  }

  activity-options-operation: func(request: activity-options) -> activity-options;
}
"#;

        let error = validate(Language::Python, wit).unwrap_err();
        assert!(error.to_string().contains(
            "must declare field `schedule_to_close_timeout` in WIT or add that field and mark it with `@nexus.omit`"
        ));
    }

    #[test]
    fn allows_explicitly_omitted_proto_fields() {
        let wit = r#"
package temporal:nexus@1.0.0;

world system {
  export workflow-service;
}

/// @nexus.endpoint "__temporal_system"
interface workflow-service {
  use nexus:temporal-types/model@1.0.0.{placeholder, retry-policy, task-queue};

  /// @nexus.proto "temporal.api.activity.v1.ActivityOptions"
  record activity-options {
    task-queue: option<task-queue>,
    retry-policy: retry-policy,
    /// @nexus.omit
    schedule-to-close-timeout: placeholder,
    /// @nexus.omit
    schedule-to-start-timeout: placeholder,
    /// @nexus.omit
    start-to-close-timeout: placeholder,
    /// @nexus.omit
    heartbeat-timeout: placeholder,
    /// @nexus.omit
    priority: placeholder,
  }

  activity-options-operation: func(request: activity-options) -> activity-options;
}
"#;

        validate(Language::Python, wit).unwrap();
    }

    #[test]
    fn rejects_type_level_omit_directive() {
        let wit = r#"
package temporal:nexus@1.0.0;

world system {
  export workflow-service;
}

/// @nexus.endpoint "__temporal_system"
interface workflow-service {
  use nexus:temporal-types/model@1.0.0.{retry-policy, task-queue};

  /// @nexus.proto "temporal.api.activity.v1.ActivityOptions"
  /// @nexus.omit
  record activity-options {
    task-queue: option<task-queue>,
    retry-policy: retry-policy,
  }

  activity-options-operation: func(request: activity-options) -> activity-options;
}
"#;

        let error = crate::parser::parse_api_spec_from_wit_for_language_with_inputs(
            Language::Python,
            wit,
            PathBuf::from("inline.wit"),
            &[linked_inputs_path()],
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("type-level omit is no longer supported")
        );
    }

    #[test]
    fn wit_parse_errors_include_parser_diagnostics() {
        let wit = r#"
package temporal:nexus@1.0.0;

world system {
  export example;
}

interface example {
  record request {
    include: string,
  }
}
"#;

        let error = crate::parser::parse_api_spec_from_wit_for_language(
            Language::Python,
            wit,
            PathBuf::from("inline.wit"),
        )
        .unwrap_err();
        let message = error.to_string();
        assert!(message.contains("expected an identifier or string"));
        assert!(message.contains("found keyword `include`"));
        assert!(message.contains("include: string"));
    }

    #[test]
    fn parses_multiline_directive_arguments() {
        let directives = parse_directives(
            Some(
                r#"@nexus.type
  python="temporalio.common.RetryPolicy"
  typescript="common.RetryPolicy""#,
            ),
            &PathBuf::from("inline.wit"),
            "type `example`",
        )
        .unwrap();

        let directive = directive(
            &directives,
            "type",
            &PathBuf::from("inline.wit"),
            "type `example`",
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            directive.value("python"),
            Some("temporalio.common.RetryPolicy")
        );
        assert_eq!(directive.value("typescript"), Some("common.RetryPolicy"));
    }

    #[test]
    fn rejects_duplicate_proto_field_mappings() {
        let wit = r#"
package temporal:nexus@1.0.0;

world system {
  export workflow-service;
}

interface workflow-service {
  /// @nexus.proto "temporal.api.workflowservice.v1.SignalWithStartWorkflowExecutionRequest"
  record signal-with-start-workflow-request {
    /// @nexus.proto-field "workflow_id"
    workflow-id: string,
    /// @nexus.proto-field "workflow_id"
    workflow-id-alias: string,
  }
}
"#;

        let err = crate::parser::parse_api_spec_from_wit_for_language(
            Language::Python,
            wit,
            PathBuf::from("inline.wit"),
        )
        .unwrap_err();
        assert!(matches!(err, Error::InvalidWit { .. }));
    }
}
