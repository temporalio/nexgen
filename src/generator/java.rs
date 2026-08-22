use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use heck::ToLowerCamelCase;

use crate::error::{Error, Result};
use crate::generator::json_schema::java as java_json;
use crate::generator::json_schema::java::JavaContext;
use crate::generator::{GeneratedFiles, GeneratedOutputLayout, GenerationMode};
use crate::planning::{PlannedFamily, PlannedJsonType};
use crate::spec::{ApiSpecLeaf, ApiSpecNode};
use crate::spec::{ExternalTypeSpec, ModulePath, ServiceSpec, TypeSpec};

const DEFAULT_PACKAGE: &str = "generated";
const JAVA_FORMAT_LINE_LENGTH: usize = 88;

pub(crate) fn generate(
    tree: &crate::spec::ApiSpecTree<PlannedFamily>,
    _support: &crate::SupportFiles,
    _mode: GenerationMode,
    base_package: Option<&str>,
) -> Result<GeneratedFiles> {
    let base_package = base_package.unwrap_or(DEFAULT_PACKAGE);
    let root_package = base_package;

    let mut leaves = Vec::new();
    collect_leaves(&tree.root, &mut leaves);
    for leaf in &leaves {
        if let Some((_, record)) = leaf
            .spec
            .records()
            .find(|(_, record)| record.data.proto.is_some())
        {
            return Err(Error::UnsupportedJavaProtoModel {
                message: record
                    .data
                    .proto
                    .as_ref()
                    .expect("checked above")
                    .full_name
                    .clone(),
            });
        }
    }

    // Registry from each model's canonical `full_name` (the string that appears
    // in planned `$ref` values) to its Java (package, class).
    let mut registry: BTreeMap<String, (String, String)> = BTreeMap::new();
    for leaf in &leaves {
        for (_, binding) in leaf.spec.external_types() {
            if let ExternalTypeSpec::Json(json_type) = &binding.external_type {
                let module = json_type.module_path.as_ref().unwrap_or(&leaf.module_path);
                registry.insert(
                    json_type.full_name.clone(),
                    (
                        JavaContext::package_for_module(base_package, &module.0),
                        json_type.model_name.clone(),
                    ),
                );
            }
        }
    }

    // Every JSON model keyed by its canonical `full_name`, so a `oneOf` union
    // def can resolve its `$ref` object members' schemas + discriminants.
    let mut all_models: BTreeMap<String, PlannedJsonType> = BTreeMap::new();
    for leaf in &leaves {
        for (_, binding) in leaf.spec.external_types() {
            if let ExternalTypeSpec::Json(json_type) = &binding.external_type {
                all_models.insert(json_type.full_name.clone(), json_type.clone());
            }
        }
    }

    let mut files = BTreeMap::new();
    let mut packages: BTreeSet<Vec<String>> = BTreeSet::new();
    packages.insert(Vec::new());

    for leaf in &leaves {
        let module = &leaf.module_path;
        packages.insert(module.0.clone());
        let module_dir = module.to_path_buf();

        for (_, binding) in leaf.spec.external_types() {
            let ExternalTypeSpec::Json(json_type) = &binding.external_type else {
                continue;
            };
            let contents = java_json::render_model_file(
                json_type,
                base_package,
                module,
                root_package,
                &registry,
                &all_models,
            )?;
            let path = module_dir.join(format!("{}.java", json_type.model_name));
            insert_file(&mut files, path, contents)?;
        }

        for service in &leaf.spec.services {
            let contents = render_service_file(service, module, base_package)?;
            let service_ident = service
                .code_name
                .for_language(crate::language::Language::Java)
                .unwrap_or(&service.name);
            let path = module_dir.join(format!("{service_ident}.java"));
            insert_file(&mut files, path, contents)?;
        }
    }

    // Runtime classes live once at the package root.
    insert_file(
        &mut files,
        PathBuf::from("Violation.java"),
        java_json::render_violation_file(root_package),
    )?;
    insert_file(
        &mut files,
        PathBuf::from("ValidationException.java"),
        java_json::render_validation_exception_file(root_package),
    )?;
    insert_file(
        &mut files,
        PathBuf::from("SpecNumbers.java"),
        java_json::render_spec_numbers_file(root_package),
    )?;
    // The materialized-temporal runtime is emitted only when a model uses a
    // temporal `format`, so non-temporal packages stay lean.
    if all_models.values().any(java_json::model_uses_temporal) {
        insert_file(
            &mut files,
            PathBuf::from("TemporalSupport.java"),
            java_json::render_temporal_support_file(root_package),
        )?;
    }
    // The materialized-contentEncoding runtime is emitted only when a model uses
    // a `contentEncoding`, so non-bytes packages stay lean.
    if all_models
        .values()
        .any(java_json::model_uses_content_encoding)
    {
        insert_file(
            &mut files,
            PathBuf::from("Base64Support.java"),
            java_json::render_base64_support_file(root_package),
        )?;
    }

    // A `package-info.java` marking each emitted package `@NullMarked`.
    for module in &packages {
        let package = JavaContext::package_for_module(base_package, module);
        let mut path = PathBuf::new();
        for segment in module {
            path.push(segment);
        }
        path.push("package-info.java");
        insert_file(&mut files, path, java_json::render_package_info(&package))?;
    }

    Ok(GeneratedFiles {
        layout: GeneratedOutputLayout::Directory,
        files,
        warnings: Vec::new(),
    })
}

fn collect_leaves<'a>(
    node: &'a ApiSpecNode<PlannedFamily>,
    leaves: &mut Vec<&'a ApiSpecLeaf<PlannedFamily>>,
) {
    match node {
        ApiSpecNode::Leaf(leaf) => leaves.push(leaf),
        ApiSpecNode::Branch(branch) => {
            for child in branch.children.values() {
                collect_leaves(child, leaves);
            }
        }
    }
}

fn insert_file(
    files: &mut BTreeMap<PathBuf, String>,
    path: PathBuf,
    contents: String,
) -> Result<()> {
    if files.insert(path.clone(), contents).is_some() {
        return Err(Error::GeneratedFileConflict { path });
    }
    Ok(())
}

fn render_service_file(
    service: &ServiceSpec<PlannedFamily>,
    module: &ModulePath,
    base_package: &str,
) -> Result<String> {
    let package = JavaContext::package_for_module(base_package, &module.0);

    let mut imports: BTreeSet<String> = BTreeSet::new();
    imports.insert("io.nexusrpc.Operation".to_string());
    imports.insert("io.nexusrpc.Service".to_string());

    let mut body = String::new();
    render_service_javadoc(
        &mut body,
        service.doc.for_language(crate::language::Language::Java),
        service.deprecated,
        "service",
    );
    body.push_str(&format!(
        "@Service(name = {})\n{}public interface {} {{\n",
        java_string_literal(&service.wire_name),
        if service.deprecated {
            "@Deprecated\n"
        } else {
            ""
        },
        service
            .code_name
            .for_language(crate::language::Language::Java)
            .unwrap_or(&service.name)
    ));

    for (index, operation) in service.operations.iter().enumerate() {
        if index > 0 {
            body.push('\n');
        }
        render_service_javadoc_indented(
            &mut body,
            operation.doc.for_language(crate::language::Language::Java),
            operation.deprecated,
            "operation",
        );
        body.push_str(&format!(
            "    @Operation(name = {})\n",
            java_string_literal(&operation.wire_name)
        ));
        if operation.deprecated {
            body.push_str("    @Deprecated\n");
        }

        let output = io_type(operation.output.as_ref(), module, base_package);
        let input = io_type(operation.input.as_ref(), module, base_package);
        let return_type = match &output {
            Some((pkg, class)) => {
                if pkg != &package {
                    imports.insert(format!("{pkg}.{class}"));
                }
                class.clone()
            }
            None => "void".to_string(),
        };
        let method = operation
            .code_name
            .for_language(crate::language::Language::Java)
            .map(str::to_string)
            .unwrap_or_else(|| operation.name.to_lower_camel_case());
        match &input {
            Some((pkg, class)) => {
                if pkg != &package {
                    imports.insert(format!("{pkg}.{class}"));
                }
                body.push_str(&format!("    {return_type} {method}({class} input);\n"));
            }
            None => {
                body.push_str(&format!("    {return_type} {method}();\n"));
            }
        }
    }
    body.push_str("}\n");

    let mut output = String::new();
    output.push_str(java_json::GENERATED_HEADER);
    output.push_str(&format!("package {package};\n\n"));
    for import in &imports {
        output.push_str(&format!("import {import};\n"));
    }
    output.push('\n');
    output.push_str(&body);
    Ok(output)
}

fn io_type(
    ty: Option<&TypeSpec<PlannedFamily>>,
    module: &ModulePath,
    base_package: &str,
) -> Option<(String, String)> {
    let ty = ty?;
    match ty.without_option() {
        TypeSpec::External(ExternalTypeSpec::Json(json_type)) => {
            let target = json_type.module_path.as_ref().unwrap_or(module);
            Some((
                JavaContext::package_for_module(base_package, &target.0),
                json_type.model_name.clone(),
            ))
        }
        _ => None,
    }
}

fn render_service_javadoc(output: &mut String, doc: Option<&str>, deprecated: bool, kind: &str) {
    let tags = if deprecated {
        vec![(
            "@deprecated".to_string(),
            format!("This {kind} is deprecated."),
        )]
    } else {
        Vec::new()
    };
    render_java_doc_comment(output, "", doc, &tags);
}

fn render_service_javadoc_indented(
    output: &mut String,
    doc: Option<&str>,
    deprecated: bool,
    kind: &str,
) {
    let tags = if deprecated {
        vec![(
            "@deprecated".to_string(),
            format!("This {kind} is deprecated."),
        )]
    } else {
        Vec::new()
    };
    render_java_doc_comment(output, "    ", doc, &tags);
}

pub(in crate::generator) fn render_java_doc_comment(
    output: &mut String,
    indent: &str,
    summary: Option<&str>,
    tags: &[(String, String)],
) {
    let has_summary = summary.is_some_and(|summary| !summary.trim().is_empty());
    let has_tags = tags
        .iter()
        .any(|(tag, doc)| !tag.trim().is_empty() || !doc.trim().is_empty());
    if !has_summary && !has_tags {
        return;
    }

    output.push_str(indent);
    output.push_str("/**\n");
    if let Some(summary) = summary.map(str::trim).filter(|summary| !summary.is_empty()) {
        for line in summary.lines() {
            push_wrapped_java_doc_line(output, indent, "", "", line.trim());
        }
    }
    if has_summary && has_tags {
        output.push_str(indent);
        output.push_str(" *\n");
    }
    for (tag, doc) in tags {
        let tag = tag.trim();
        let doc = doc.trim();
        if tag.is_empty() && doc.is_empty() {
            continue;
        }
        if doc.is_empty() {
            push_wrapped_java_doc_line(output, indent, "", "", tag);
        } else {
            push_wrapped_java_doc_line(output, indent, &format!("{tag} "), "    ", doc);
        }
    }
    output.push_str(indent);
    output.push_str(" */\n");
}

fn push_wrapped_java_doc_line(
    output: &mut String,
    indent: &str,
    first_prefix: &str,
    continuation_prefix: &str,
    text: &str,
) {
    let max_width = JAVA_FORMAT_LINE_LENGTH.saturating_sub(indent.chars().count() + 3);
    if text.trim().is_empty() {
        output.push_str(indent);
        output.push_str(" *\n");
        return;
    }

    let escaped = text
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace("*/", "* /");
    let mut prefix = first_prefix;
    let mut current = String::new();
    for word in escaped.split_whitespace() {
        let prefix_width = prefix.chars().count();
        let current_width = current.chars().count();
        let word_width = word.chars().count();
        let separator_width = usize::from(!current.is_empty());
        if current_width > 0
            && prefix_width + current_width + separator_width + word_width > max_width
        {
            output.push_str(indent);
            output.push_str(" * ");
            output.push_str(prefix);
            output.push_str(&current);
            output.push('\n');
            prefix = continuation_prefix;
            current.clear();
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }

    output.push_str(indent);
    output.push_str(" * ");
    output.push_str(prefix);
    output.push_str(&current);
    output.push('\n');
}

fn java_string_literal(value: &str) -> String {
    let mut output = String::with_capacity(value.len() + 2);
    output.push('"');
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            other => output.push(other),
        }
    }
    output.push('"');
    output
}
