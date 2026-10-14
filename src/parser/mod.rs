mod json_schema;
mod wit;

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::language::Language;
use crate::spec::{ApiSpec, ApiSpecLeaf, ApiSpecNode, ApiSpecTree, ModulePath};

pub(crate) use json_schema::strip_json_schema_extension;
pub use json_schema::{
    load_api_spec_from_json_schema_for_language_with_inputs,
    load_api_spec_tree_from_json_schema_for_language_with_inputs,
};

pub(crate) use json_schema::{
    ManifestModel, ManifestService, NameManifest, build_name_manifest,
    ts_transfer_type_converter_name,
};
pub use wit::{load_api_spec_from_wit_for_language_with_inputs, write_prepared_wit_directory};

pub(crate) use wit::{
    directive, directive_value, find_proto_name_for_type, find_proto_name_for_type_def,
    parse_directives, parse_wit_with_inputs, resolve_function_signature_args, select_world,
    wire_operation_name_from_docs,
};

#[cfg(test)]
pub(crate) use wit::{
    parse_api_spec_from_wit_for_language, parse_api_spec_from_wit_for_language_with_inputs,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InputFormat {
    Wit,
    JsonSchema,
}

pub fn load_api_spec_for_language_with_inputs(
    language: Language,
    input_paths: &[PathBuf],
) -> Result<ApiSpec> {
    let tree = load_api_spec_tree_for_language_with_inputs(language, input_paths)?;
    tree.into_single_spec()
        .ok_or_else(|| Error::InvalidJsonSchema {
            path: PathBuf::from("<input>"),
            reason: "multiple input modules require tree generation".to_string(),
        })
}

pub fn load_api_spec_tree_for_language_with_inputs(
    language: Language,
    input_paths: &[PathBuf],
) -> Result<ApiSpecTree> {
    let format = detect_input_format(input_paths)?;
    match format {
        InputFormat::Wit => {
            let spec = load_api_spec_from_wit_for_language_with_inputs(language, input_paths)?;
            let source_path = input_paths
                .first()
                .cloned()
                .expect("WIT input paths were validated during format detection");
            Ok(ApiSpecTree {
                root: ApiSpecNode::Leaf(ApiSpecLeaf {
                    module_path: ModulePath::default(),
                    source_root: source_path
                        .parent()
                        .map(Path::to_path_buf)
                        .unwrap_or_default(),
                    source_path,
                    spec,
                }),
            })
        }
        InputFormat::JsonSchema => {
            load_api_spec_tree_from_json_schema_for_language_with_inputs(language, input_paths)
        }
    }
}

/// CLI validation uses the same format detection as parsing, before entering
/// the input-format-neutral compiler pipeline.
pub(crate) fn is_wit_input(input_paths: &[PathBuf]) -> Result<bool> {
    Ok(detect_input_format(input_paths)? == InputFormat::Wit)
}

fn detect_input_format(input_paths: &[PathBuf]) -> Result<InputFormat> {
    let Some((first, rest)) = input_paths.split_first() else {
        return Err(Error::InvalidWit {
            path: PathBuf::from("<input>"),
            reason: "at least one input path is required".to_string(),
        });
    };
    let format = input_format(first)?;
    if format == InputFormat::Wit {
        return Ok(format);
    }
    for path in rest {
        let found = input_format(path)?;
        if found != format {
            return Err(Error::MixedInputFormats {
                first: input_format_name(format),
                path: path.clone(),
                found: input_format_name(found),
            });
        }
    }
    Ok(format)
}

fn input_format(path: &Path) -> Result<InputFormat> {
    if path.is_dir() {
        return Ok(InputFormat::JsonSchema);
    }
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("wit") => Ok(InputFormat::Wit),
        Some("json" | "yaml" | "yml") => Ok(InputFormat::JsonSchema),
        _ => Err(Error::UnsupportedInputFormat {
            path: path.to_path_buf(),
        }),
    }
}

fn input_format_name(format: InputFormat) -> &'static str {
    match format {
        InputFormat::Wit => "wit",
        InputFormat::JsonSchema => "json-schema",
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    #[test]
    fn generic_loader_detects_json_schema_inputs() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("api.yaml");
        fs::write(
            &path,
            r##"
nexusrpc: "1.0.0"
services:
  ChatService:
    operations:
      getRoom:
        input:
          type: object
          properties: {}
"##,
        )
        .unwrap();

        let spec =
            load_api_spec_for_language_with_inputs(Language::Python, std::slice::from_ref(&path))
                .unwrap();

        assert_eq!(spec.services[0].name, "ChatService");
        assert!(spec.records().next().is_none());
    }

    #[test]
    fn generic_loader_rejects_mixed_input_formats() {
        let temp_dir = TempDir::new().unwrap();
        let json_path = temp_dir.path().join("api.json");
        let wit_path = temp_dir.path().join("api.wit");

        let error =
            load_api_spec_for_language_with_inputs(Language::Python, &[json_path, wit_path])
                .unwrap_err();

        assert!(error.to_string().contains("mixed input formats"));
    }

    #[test]
    fn generic_loader_rejects_unsupported_input_format() {
        let error =
            load_api_spec_for_language_with_inputs(Language::Python, &[PathBuf::from("api.txt")])
                .unwrap_err();

        assert!(error.to_string().contains("unsupported input format"));
    }
}
