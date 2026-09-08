use std::collections::{BTreeMap, BTreeSet, VecDeque, btree_map};
use std::fs;
use std::path::{Path, PathBuf};

use heck::{ToLowerCamelCase, ToShoutySnakeCase, ToSnakeCase, ToUpperCamelCase};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Error, Result};
// The P15 collision pass names every synthesized identifier through the emitter's
// own naming helpers, so the load-time check cannot drift from what is emitted.
use crate::generator::json_schema::{java, python, typescript};
use crate::language::Language;
use crate::spec::{
    ApiSpec, ExternalTypeBindingSpec, ExternalTypeSpec, JsonModelBindingSpec, JsonModelSpec,
    LanguageStringSpec, ModulePath, OperationSpec, ServiceSpec, SupportSpec, Symbol, TypeDeclEntry,
    TypeDeclSpec, TypeSpec,
};
use crate::spec::{ApiSpecBranch, ApiSpecLeaf, ApiSpecNode, ApiSpecTree};

#[derive(Debug, Clone, Deserialize, Default)]
struct Document {
    #[serde(default, deserialize_with = "deserialize_present_value")]
    nexusrpc: Option<Value>,
    #[serde(rename = "$schema")]
    #[serde(default, deserialize_with = "deserialize_present_value")]
    schema: Option<Value>,
    #[serde(default, deserialize_with = "deserialize_services")]
    services: Option<IndexMap<String, Service>>,
    #[serde(rename = "$defs")]
    defs: Option<IndexMap<String, Schema>>,
    #[serde(flatten)]
    root: Schema,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct Service {
    fqn: Option<String>,
    #[serde(default, deserialize_with = "deserialize_annotation")]
    description: Option<String>,
    endpoint: Option<String>,
    #[serde(default)]
    operations: IndexMap<String, Operation>,
    #[serde(flatten)]
    extra: IndexMap<String, Value>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct Operation {
    fqn: Option<String>,
    #[serde(default, deserialize_with = "deserialize_annotation")]
    description: Option<String>,
    input: Option<Schema>,
    output: Option<Schema>,
    #[serde(flatten)]
    extra: IndexMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
struct Schema {
    #[serde(rename = "$ref")]
    reference: Option<String>,
    #[serde(rename = "$id")]
    id: Option<Value>,
    // `skip_serializing_if` keeps the internal round-trip (merge, hoist,
    // planning) faithful: `deserialize_present_value` reads an explicit
    // `type: null` as *present*, so writing the absent case as `"type": null`
    // would turn it into a malformed one on the way back in.
    #[serde(
        rename = "type",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_present_value"
    )]
    ty: Option<Value>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_annotation"
    )]
    title: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_annotation"
    )]
    description: Option<String>,
    properties: Option<IndexMap<String, Schema>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_present_value"
    )]
    required: Option<Value>,
    #[serde(
        rename = "additionalProperties",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_present_value"
    )]
    additional_properties: Option<Value>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_items"
    )]
    items: Option<Box<Schema>>,
    #[serde(rename = "oneOf")]
    one_of: Option<Vec<Schema>>,
    #[serde(flatten)]
    extra: IndexMap<String, Value>,
}

/// Deserializes an `Option<Value>` field that distinguishes **absent** from
/// **present and `null`**.
///
/// `Option<Value>`'s stock deserializer folds `type: null` into `None`, so a
/// `type` explicitly written as `null` was reported as a *missing* `type` — a
/// diagnostic pointing at the wrong defect and offering a fix-it ("add a
/// `type`") the author had already followed.
fn deserialize_present_value<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<Value>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Value::deserialize(deserializer).map(Some)
}

/// Deserializes the document-level `services` map without folding an explicit
/// YAML `null` into absence. `services: null` is malformed, not an omitted
/// envelope member.
fn deserialize_services<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<IndexMap<String, Service>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    if value.is_null() {
        return Err(serde::de::Error::custom(
            "`services` must be an object mapping service names to service definitions, not null",
        ));
    }
    serde_json::from_value(value)
        .map(Some)
        .map_err(serde::de::Error::custom)
}

/// Gives boolean- and tuple-valued `items` the subset's actionable diagnostic
/// instead of leaking serde's representation error before the schema validator
/// can name the supported uniform-element form.
fn deserialize_items<'de, D>(deserializer: D) -> std::result::Result<Option<Box<Schema>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    match value {
        Value::Bool(_) => Err(serde::de::Error::custom(
            "boolean `items` schemas are not supported; use one schema object describing the uniform element type",
        )),
        Value::Array(_) => Err(serde::de::Error::custom(
            "tuple-valued `items` is not supported; use one schema object describing the uniform element type (or `prefixItems` in a tool that supports tuples)",
        )),
        Value::Object(_) => serde_json::from_value(value)
            .map(|schema| Some(Box::new(schema)))
            .map_err(serde::de::Error::custom),
        other => Err(serde::de::Error::custom(format!(
            "`items` must be a schema object describing the uniform element type, got {other}"
        ))),
    }
}

/// Deserializes a `title`/`description` annotation, rejecting a non-string
/// value.
///
/// Typing the field `Option<String>` is not enough. A YAML plain scalar is
/// handed to `deserialize_string` as its raw text, so `title: 42` and
/// `description: true` were silently coerced to `"42"` and `"true"` and emitted
/// as doc comments. Only the document root rejected them, because its
/// `#[serde(flatten)]` routes through serde's strict buffered `Content`
/// representation — which is why the two existing tests missed every nested
/// schema and every service/operation description. Resolving the scalar with
/// `deserialize_any` first restores the type.
fn deserialize_annotation<'de, D>(deserializer: D) -> std::result::Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    match Value::deserialize(deserializer)? {
        Value::String(text) => Ok(Some(text)),
        other => Err(serde::de::Error::custom(format!(
            "must be a string, got {other}"
        ))),
    }
}

/// Parses one authored document through a raw-value preflight before lowering
/// schema positions into [`Schema`]. JSON Schema permits boolean schemas, but
/// this generator's typed subset does not: without the preflight serde rejects
/// `properties: {value: true}` or a boolean `oneOf` branch before we can name
/// the member/branch and give the explicit-`type` remedy.
fn parse_json_schema_document(path: &Path, input: &str) -> Result<Document> {
    let raw = serde_yaml::from_str::<Value>(input).map_err(|error| Error::JsonSchemaParse {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    validate_raw_document_schema_positions(path, &raw)?;
    serde_yaml::from_str::<Document>(input).map_err(|error| Error::JsonSchemaParse {
        path: path.to_path_buf(),
        message: error.to_string(),
    })
}

fn validate_raw_document_schema_positions(path: &Path, raw: &Value) -> Result<()> {
    let Some(document) = raw.as_object() else {
        return Err(Error::InvalidJsonSchema {
            path: path.to_path_buf(),
            reason: "a JSON Schema document must be an object".to_string(),
        });
    };
    validate_raw_schema_position(path, raw, "root schema", false)?;
    if let Some(services) = document.get("services").and_then(Value::as_object) {
        for (service_name, service) in services {
            let Some(operations) = service
                .as_object()
                .and_then(|service| service.get("operations"))
                .and_then(Value::as_object)
            else {
                continue;
            };
            for (operation_name, operation) in operations {
                let Some(operation) = operation.as_object() else {
                    continue;
                };
                for label in ["input", "output"] {
                    if let Some(schema) = operation.get(label) {
                        validate_raw_schema_position(
                            path,
                            schema,
                            &format!("services.{service_name}.operations.{operation_name}.{label}"),
                            false,
                        )?;
                    }
                }
            }
        }
    }
    Ok(())
}

/// Recurses only through schema-valued positions. Annotation/example/default
/// payloads are deliberately opaque, so schema-looking data cannot affect
/// parsing or `$ref` discovery.
fn validate_raw_schema_position(
    path: &Path,
    value: &Value,
    context: &str,
    one_of_branch: bool,
) -> Result<()> {
    let reject = |reason: String| {
        Err(Error::InvalidJsonSchema {
            path: path.to_path_buf(),
            reason,
        })
    };
    let object = match value {
        Value::Object(object) => object,
        Value::Bool(boolean) if one_of_branch => {
            return reject(format!(
                "{context}: boolean schema `{boolean}` has no classifiable `oneOf` kind; replace it with a schema object declaring one recognized `type`"
            ));
        }
        Value::Bool(boolean) => {
            return reject(format!(
                "{context}: boolean schema `{boolean}` has no supported typed shape; replace it with a schema object declaring an explicit `type`"
            ));
        }
        other if one_of_branch => {
            return reject(format!(
                "{context}: a `oneOf` branch must be a schema object declaring one recognized `type`, got {other}"
            ));
        }
        other => {
            return reject(format!(
                "{context}: expected a schema object declaring an explicit `type`, got {other}"
            ));
        }
    };

    if let Some(properties) = object.get("properties").and_then(Value::as_object) {
        for (name, property) in properties {
            validate_raw_schema_position(
                path,
                property,
                &format!("{context}.properties.{name}"),
                false,
            )?;
        }
    }
    if let Some(items) = object.get("items") {
        let items_context = format!("{context}.items");
        match items {
            Value::Object(_) => {
                validate_raw_schema_position(path, items, &items_context, false)?;
            }
            Value::Bool(boolean) => {
                return reject(format!(
                    "{items_context}: boolean schema `{boolean}` is not supported for `items`; use one schema object describing the uniform element type"
                ));
            }
            Value::Array(_) => {
                return reject(format!(
                    "{items_context}: tuple-valued `items` is not supported; use one schema object describing the uniform element type (or `prefixItems` in a tool that supports tuples)"
                ));
            }
            other => {
                return reject(format!(
                    "{items_context}: `items` must be a schema object describing the uniform element type, got {other}"
                ));
            }
        }
    }
    if let Some(branches) = object.get("oneOf").and_then(Value::as_array) {
        for (index, branch) in branches.iter().enumerate() {
            validate_raw_schema_position(path, branch, &format!("{context}.oneOf[{index}]"), true)?;
        }
    }
    if let Some(defs) = object.get("$defs").and_then(Value::as_object) {
        for (name, schema) in defs {
            let def_context = if context == "root schema" {
                format!("$defs.{name}")
            } else {
                format!("{context}.$defs.{name}")
            };
            validate_raw_schema_position(path, schema, &def_context, false)?;
        }
    }
    if let Some(Value::Object(_)) = object.get("additionalProperties") {
        validate_raw_schema_position(
            path,
            &object["additionalProperties"],
            &format!("{context}.additionalProperties"),
            false,
        )?;
    }
    // `allOf: true` is a supported identity branch and `allOf: false` gets the
    // conjunction-specific unsatisfiable diagnostic later. Object branches can
    // still contain typed positions which need this preflight.
    if let Some(branches) = object.get("allOf").and_then(Value::as_array) {
        for (index, branch) in branches.iter().enumerate() {
            if branch.is_object() {
                validate_raw_schema_position(
                    path,
                    branch,
                    &format!("{context}.allOf[{index}]"),
                    false,
                )?;
            }
        }
    }
    Ok(())
}

impl Schema {
    fn is_bare_ref(&self) -> bool {
        self.reference.is_some()
            && Schema {
                reference: None,
                ..self.clone()
            } == Schema::default()
    }

    /// True when the schema is a `$ref` carrying only non-conjunct siblings:
    /// member-name overrides or annotations that mark/document the member but
    /// assert nothing about the referenced value. These stay a reference rather
    /// than cloning the target through the implicit-`allOf` rewrite.
    fn is_ref_with_non_conjunct_siblings_only(&self) -> bool {
        self.reference.is_some()
            && Schema {
                reference: None,
                extra: IndexMap::new(),
                ..self.clone()
            } == Schema::default()
            && self.extra.keys().all(|keyword| {
                LANG_NAME_KEYWORDS.contains(&keyword.as_str())
                    || matches!(keyword.as_str(), "$comment" | "examples" | "deprecated")
            })
    }
}

/// Every target's `x-<lang>-name` keyword. On a member it renames the member; on
/// a type declaration it renames the type (see [`lang_name_keyword`] for the
/// per-target lookup).
const LANG_NAME_KEYWORDS: [&str; 4] = ["x-go-name", "x-ts-name", "x-py-name", "x-java-name"];

/// Keywords admitted by the strict schema-node grammar. This is deliberately
/// an exact allowlist: supported keywords, specifically rejected keywords, and
/// generator extensions all have an owner below. Anything else is a typo or a
/// dialect feature the loader cannot preserve coherently.
fn schema_extra_keyword_is_known(keyword: &str) -> bool {
    matches!(
        keyword,
        "allOf"
            | "anyOf"
            | "not"
            | "if"
            | "then"
            | "else"
            | "prefixItems"
            | "unevaluatedProperties"
            | "unevaluatedItems"
            | "dependentSchemas"
            | "patternProperties"
            | "nullable"
            | "$anchor"
            | "$dynamicRef"
            | "$dynamicAnchor"
            | "$vocabulary"
            | "$defs"
            | "readOnly"
            | "writeOnly"
            | "contentMediaType"
            | "contentSchema"
            | "minimum"
            | "maximum"
            | "exclusiveMinimum"
            | "exclusiveMaximum"
            | "multipleOf"
            | "minLength"
            | "maxLength"
            | "pattern"
            | "format"
            | "contentEncoding"
            | "minItems"
            | "maxItems"
            | "uniqueItems"
            | "contains"
            | "minContains"
            | "maxContains"
            | "minProperties"
            | "maxProperties"
            | "propertyNames"
            | "dependentRequired"
            | "dependencies"
            | "const"
            | "enum"
            | "default"
            | "deprecated"
            | "$comment"
            | "examples"
            | "x-go-const-name"
            | "x-java-const-name"
            | "x-go-enum-names"
            | "x-java-enum-names"
    ) || LANG_NAME_KEYWORDS.contains(&keyword)
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum TypeKey {
    Root(PathBuf),
    /// Names of each definition in an RFC 6901 `$defs` chain, outermost
    /// first. Keeping tokens separate avoids confusing an escaped `/` inside a
    /// definition name with a pointer path separator.
    Def(PathBuf, Vec<String>),
}

#[derive(Clone, Debug)]
struct JsonModel {
    full_name: String,
    canonical_path: PathBuf,
    model_name: String,
    schema: Schema,
}

#[derive(Clone, Debug)]
struct JsonSource {
    path: PathBuf,
    source_root: PathBuf,
    relative_path: PathBuf,
    input: String,
}

struct ParsedJsonDocuments {
    docs: IndexMap<PathBuf, (PathBuf, Document)>,
    models: BTreeMap<TypeKey, JsonModel>,
}

pub fn load_api_spec_from_json_schema_for_language_with_inputs(
    language: Language,
    input_paths: &[PathBuf],
) -> Result<ApiSpec> {
    let sources = expand_json_schema_sources(input_paths)?;
    api_spec_from_json_schema_sources(
        language,
        sources
            .into_iter()
            .map(|source| (source.path, source.input))
            .collect(),
    )
}

pub fn load_api_spec_tree_from_json_schema_for_language_with_inputs(
    language: Language,
    input_paths: &[PathBuf],
) -> Result<ApiSpecTree> {
    let sources = expand_json_schema_sources(input_paths)?;
    api_spec_tree_from_json_schema_sources(language, sources)
}

fn expand_json_schema_sources(input_paths: &[PathBuf]) -> Result<Vec<JsonSource>> {
    if input_paths.is_empty() {
        return Err(Error::InvalidJsonSchema {
            path: PathBuf::from("<input>"),
            reason: "at least one JSON schema input path is required".to_string(),
        });
    }
    let invocation_root = json_schema_invocation_root(input_paths)?;
    let mut source_inputs = BTreeMap::<PathBuf, String>::new();
    for input_path in input_paths {
        if input_path.is_dir() {
            let mut files = Vec::new();
            collect_json_schema_files(input_path, &mut files)?;
            files.sort();
            for path in files {
                insert_json_schema_source(&path, &mut source_inputs)?;
            }
        } else {
            insert_json_schema_source(input_path, &mut source_inputs)?;
        }
    }

    // The input set is the transitive closure of local file refs, not merely
    // the paths named on the command line. Scan raw values so refs inside dead
    // `$defs` are included too: those definitions are generated API surface and
    // therefore their dependencies must be available.
    let mut pending = source_inputs.keys().cloned().collect::<VecDeque<_>>();
    while let Some(path) = pending.pop_front() {
        let input = source_inputs
            .get(&path)
            .expect("queued JSON schema source should be present");
        let document = parse_json_schema_document(&path, input)?;
        // Diagnose malformed/unknown authored grammar before following refs it
        // may have placed in a position that is not a schema at all.
        validate_raw_document_grammar(&path, &document)?;
        let mut references = Vec::new();
        collect_local_ref_file_parts(&document, &mut references)?;
        for (file_part, reference, context) in references {
            if Path::new(&file_part).is_absolute() {
                let target = canonical(Path::new(&file_part));
                let invocation_remedy = (!target.starts_with(&invocation_root)).then(|| {
                    format!(
                        "; the target `{}` is also outside the invocation root `{}`, so widen the invocation to include that ancestor or pass the target as an additional input",
                        target.display(),
                        invocation_root.display(),
                    )
                });
                return Err(Error::InvalidJsonSchema {
                    path: path.clone(),
                    reason: format!(
                        "{context}: absolute-path `$ref` `{reference}` is not supported; use a path relative to the referring schema{}",
                        invocation_remedy.unwrap_or_default(),
                    ),
                });
            }
            let target = normalize(
                &path
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join(&file_part),
            );
            let target = canonical(&target);
            if !target.starts_with(&invocation_root) {
                return Err(Error::InvalidJsonSchema {
                    path: path.clone(),
                    reason: format!(
                        "{context}: `$ref` `{reference}` escapes the invocation root `{}` to `{}`; widen the invocation to include that ancestor or pass the target as an additional input",
                        invocation_root.display(),
                        target.display(),
                    ),
                });
            }
            if source_inputs.contains_key(&target) {
                continue;
            }
            insert_json_schema_source(&target, &mut source_inputs).map_err(|error| match error {
                Error::ReadFile { source, .. } => Error::InvalidJsonSchema {
                    path: path.clone(),
                    reason: format!(
                        "{context}: `$ref` `{reference}` resolves to `{}`, but that target file could not be read: {source}; add the file at that path or correct the relative `$ref`",
                        target.display()
                    ),
                },
                other => other,
            })?;
            pending.push_back(target);
        }
    }

    let source_root =
        common_source_root(source_inputs.keys()).ok_or_else(|| Error::InvalidJsonSchema {
            path: PathBuf::from("<input>"),
            reason: "could not determine a common root for the JSON schema input set".to_string(),
        })?;
    let mut sources = source_inputs
        .into_iter()
        .map(|(path, input)| JsonSource {
            relative_path: normalize(
                path.strip_prefix(&source_root)
                    .expect("common source root must prefix every input path"),
            ),
            path,
            source_root: source_root.clone(),
            input,
        })
        .collect::<Vec<_>>();
    sources.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    let mut seen = BTreeMap::<PathBuf, PathBuf>::new();
    for source in &sources {
        if let Some(existing) = seen.insert(source.relative_path.clone(), source.path.clone()) {
            return Err(Error::InvalidJsonSchema {
                path: source.path.clone(),
                reason: format!(
                    "duplicate JSON schema module path `{}` also provided by `{}`",
                    source.relative_path.display(),
                    existing.display()
                ),
            });
        }
    }
    Ok(sources)
}

/// The user-chosen boundary refs may not escape. For a directory input this is
/// the directory itself; for file inputs it is their common containing
/// directory. Computing it before closure expansion prevents a ref from making
/// generated module paths depend on the checkout's absolute location.
fn json_schema_invocation_root(input_paths: &[PathBuf]) -> Result<PathBuf> {
    let mut roots = input_paths.iter().map(|path| {
        let canonical_path = canonical(path);
        if path.is_dir() {
            canonical_path
        } else {
            canonical_path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .to_path_buf()
        }
    });
    let Some(mut root) = roots.next() else {
        return Err(Error::InvalidJsonSchema {
            path: PathBuf::from("<input>"),
            reason: "at least one JSON schema input path is required".to_string(),
        });
    };
    for path in roots {
        while !path.starts_with(&root) {
            if !root.pop() {
                return Err(Error::InvalidJsonSchema {
                    path: PathBuf::from("<input>"),
                    reason: "could not determine the JSON schema invocation root".to_string(),
                });
            }
        }
    }
    Ok(root)
}

fn insert_json_schema_source(path: &Path, sources: &mut BTreeMap<PathBuf, String>) -> Result<()> {
    let input = fs::read_to_string(path).map_err(|source| Error::ReadFile {
        path: path.to_path_buf(),
        source,
    })?;
    sources.entry(canonical(path)).or_insert(input);
    Ok(())
}

fn collect_local_ref_file_parts(
    document: &Document,
    out: &mut Vec<(String, String, String)>,
) -> Result<()> {
    if let Some(defs) = &document.defs {
        for (name, schema) in defs {
            collect_schema_local_ref_file_parts(schema, &format!("$defs.{name}"), out)?;
        }
    }
    if root_is_schema_shaped(&document.root) {
        collect_schema_local_ref_file_parts(&document.root, "root schema", out)?;
    }
    if let Some(services) = &document.services {
        for (service_name, service) in services {
            for (operation_name, operation) in &service.operations {
                if let Some(input) = &operation.input {
                    collect_schema_local_ref_file_parts(
                        input,
                        &format!("services.{service_name}.operations.{operation_name}.input"),
                        out,
                    )?;
                }
                if let Some(output) = &operation.output {
                    collect_schema_local_ref_file_parts(
                        output,
                        &format!("services.{service_name}.operations.{operation_name}.output"),
                        out,
                    )?;
                }
            }
        }
    }
    Ok(())
}

/// Walks schema-valued positions only. Values under `examples`, `default`,
/// `const`, and `enum` are data even when they contain a `$ref`-shaped key.
fn collect_schema_local_ref_file_parts(
    schema: &Schema,
    context: &str,
    out: &mut Vec<(String, String, String)>,
) -> Result<()> {
    if let Some(reference) = &schema.reference {
        let file_part = reference
            .split_once('#')
            .map_or(reference.as_str(), |(file, _)| file);
        if !file_part.is_empty() && !ref_file_part_has_uri_scheme(file_part) {
            out.push((
                file_part.to_string(),
                reference.clone(),
                context.to_string(),
            ));
        }
    }
    if let Some(properties) = &schema.properties {
        for (name, property) in properties {
            collect_schema_local_ref_file_parts(
                property,
                &format!("{context}.properties.{name}"),
                out,
            )?;
        }
    }
    if let Some(items) = &schema.items {
        collect_schema_local_ref_file_parts(items, &format!("{context}.items"), out)?;
    }
    if let Some(branches) = &schema.one_of {
        for (index, branch) in branches.iter().enumerate() {
            collect_schema_local_ref_file_parts(branch, &format!("{context}.oneOf[{index}]"), out)?;
        }
    }
    if let Some(Value::Object(_)) = &schema.additional_properties {
        let child: Schema = serde_json::from_value(
            schema
                .additional_properties
                .clone()
                .expect("additionalProperties is present"),
        )
        .map_err(|error| Error::InvalidJsonSchema {
            path: PathBuf::from("<json-schema>"),
            reason: format!("{context}.additionalProperties is invalid: {error}"),
        })?;
        collect_schema_local_ref_file_parts(
            &child,
            &format!("{context}.additionalProperties"),
            out,
        )?;
    }
    if let Some(Value::Array(branches)) = schema.extra.get("allOf") {
        for (index, branch) in branches.iter().enumerate() {
            if branch.is_object() {
                let child: Schema = serde_json::from_value(branch.clone()).map_err(|error| {
                    Error::InvalidJsonSchema {
                        path: PathBuf::from("<json-schema>"),
                        reason: format!("{context}.allOf[{index}] is invalid: {error}"),
                    }
                })?;
                collect_schema_local_ref_file_parts(
                    &child,
                    &format!("{context}.allOf[{index}]"),
                    out,
                )?;
            }
        }
    }
    for keyword in ["contains", "propertyNames"] {
        if let Some(value @ Value::Object(_)) = schema.extra.get(keyword) {
            let child: Schema = serde_json::from_value(value.clone()).map_err(|error| {
                Error::InvalidJsonSchema {
                    path: PathBuf::from("<json-schema>"),
                    reason: format!("{context}.{keyword} is invalid: {error}"),
                }
            })?;
            collect_schema_local_ref_file_parts(&child, &format!("{context}.{keyword}"), out)?;
        }
    }
    if let Some(Value::Object(defs)) = schema.extra.get("$defs") {
        for (name, definition) in defs {
            if definition.is_object() {
                let child: Schema =
                    serde_json::from_value(definition.clone()).map_err(|error| {
                        Error::InvalidJsonSchema {
                            path: PathBuf::from("<json-schema>"),
                            reason: format!("{context}.$defs.{name} is invalid: {error}"),
                        }
                    })?;
                collect_schema_local_ref_file_parts(
                    &child,
                    &format!("{context}.$defs.{name}"),
                    out,
                )?;
            }
        }
    }
    Ok(())
}

fn ref_file_part_has_uri_scheme(file_part: &str) -> bool {
    let Some((scheme, _)) = file_part.split_once(':') else {
        return false;
    };
    let mut characters = scheme.chars();
    characters
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic())
        && characters.all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.')
        })
}

fn common_source_root<'a>(paths: impl Iterator<Item = &'a PathBuf>) -> Option<PathBuf> {
    let mut paths = paths;
    let first = paths.next()?;
    let mut root = first.parent()?.to_path_buf();
    for path in paths {
        while !path.starts_with(&root) {
            if !root.pop() {
                return None;
            }
        }
    }
    Some(root)
}

fn collect_json_schema_files(dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    let entries = fs::read_dir(dir).map_err(|source| Error::ReadFile {
        path: dir.to_path_buf(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| Error::ReadFile {
            path: dir.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if path.is_dir() {
            collect_json_schema_files(&path, files)?;
        } else if supported_json_schema_extension(&path) {
            files.push(path);
        }
    }
    Ok(())
}

fn supported_json_schema_extension(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("json" | "yaml" | "yml")
    )
}

fn api_spec_tree_from_json_schema_sources(
    language: Language,
    sources: Vec<JsonSource>,
) -> Result<ApiSpecTree> {
    let module_paths = sources
        .iter()
        .map(|source| {
            (
                canonical(&source.path),
                module_path_from_relative_source(&source.relative_path),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let parsed = parse_json_documents(
        language,
        sources
            .iter()
            .map(|source| (source.path.clone(), source.input.clone()))
            .collect(),
    )?;
    if sources.len() == 1 {
        let source = sources
            .into_iter()
            .next()
            .expect("single JSON source should be present");
        let mut spec = api_spec_from_parsed_json_documents(
            language,
            &parsed,
            &[canonical(&source.path)],
            None,
        )?;
        spec.module_path = ModulePath::default();
        return Ok(ApiSpecTree {
            root: ApiSpecNode::Leaf(ApiSpecLeaf {
                module_path: spec.module_path.clone(),
                source_root: source.source_root,
                source_path: source.relative_path,
                spec,
            }),
        });
    }

    for source in &sources {
        let module_path = module_path_from_relative_source(&source.relative_path);
        if let Some((_, segment)) = module_path
            .0
            .iter()
            .enumerate()
            .find(|(depth, segment)| is_reserved_module_name(segment, *depth))
        {
            return Err(Error::InvalidJsonSchema {
                path: source.path.clone(),
                reason: format!(
                    "input `{}` maps to the reserved module name `{segment}`, which collides with a generated runtime or aggregator file; rename the input file or directory",
                    source.relative_path.display()
                ),
            });
        }
        for segment in &module_path.0 {
            if let Some(reason) = module_segment_defect(segment) {
                return Err(Error::InvalidJsonSchema {
                    path: source.path.clone(),
                    reason: format!(
                        "input `{}` maps to the module segment `{segment}`, which {reason}; rename the input file or directory",
                        source.relative_path.display()
                    ),
                });
            }
        }
    }

    let mut root = ApiSpecBranch {
        module_path: ModulePath::default(),
        children: BTreeMap::new(),
    };
    for source in sources {
        let module_path = module_path_from_relative_source(&source.relative_path);
        let mut spec = api_spec_from_parsed_json_documents(
            language,
            &parsed,
            &[canonical(&source.path)],
            Some(&module_paths),
        )?;
        spec.module_path = module_path.clone();
        insert_leaf(
            &mut root,
            ApiSpecLeaf {
                module_path,
                source_root: source.source_root,
                source_path: source.relative_path,
                spec,
            },
        )?;
    }
    Ok(ApiSpecTree {
        root: ApiSpecNode::Branch(root),
    })
}

fn insert_leaf(branch: &mut ApiSpecBranch, leaf: ApiSpecLeaf) -> Result<()> {
    let segments = leaf.module_path.0.clone();
    let Some((segment, rest)) = segments.split_first() else {
        return Err(Error::InvalidJsonSchema {
            path: leaf.source_path,
            reason: "JSON schema module path must not be empty".to_string(),
        });
    };
    insert_leaf_at(branch, segment, rest, leaf)
}

fn insert_leaf_at(
    branch: &mut ApiSpecBranch,
    segment: &str,
    rest: &[String],
    leaf: ApiSpecLeaf,
) -> Result<()> {
    if rest.is_empty() {
        if let Some(existing) = branch.children.get(segment) {
            let first = first_leaf_source(existing)
                .unwrap_or_else(|| PathBuf::from("<existing JSON schema module>"));
            return Err(Error::InvalidJsonSchema {
                path: leaf.source_path.clone(),
                reason: format!(
                    "JSON schema inputs `{}` and `{}` map to the same module path `{}`; rename one input file or directory so their module paths differ",
                    first.display(),
                    leaf.source_path.display(),
                    leaf.module_path.as_module_key(),
                ),
            });
        }
        branch
            .children
            .insert(segment.to_string(), ApiSpecNode::Leaf(leaf));
        return Ok(());
    }

    let child_path = branch.module_path.child(segment);
    let child = branch
        .children
        .entry(segment.to_string())
        .or_insert_with(|| {
            ApiSpecNode::Branch(ApiSpecBranch {
                module_path: child_path,
                children: BTreeMap::new(),
            })
        });
    let ApiSpecNode::Branch(child_branch) = child else {
        let first = first_leaf_source(child)
            .unwrap_or_else(|| PathBuf::from("<existing JSON schema module>"));
        return Err(Error::InvalidJsonSchema {
            path: leaf.source_path.clone(),
            reason: format!(
                "JSON schema inputs `{}` and `{}` conflict because one module path is a prefix of the other; rename one input file or directory so a module is not also a module directory",
                first.display(),
                leaf.source_path.display(),
            ),
        });
    };
    insert_leaf_at(child_branch, &rest[0], &rest[1..], leaf)
}

fn first_leaf_source(node: &ApiSpecNode) -> Option<PathBuf> {
    match node {
        ApiSpecNode::Leaf(leaf) => Some(leaf.source_path.clone()),
        ApiSpecNode::Branch(branch) => branch.children.values().find_map(first_leaf_source),
    }
}

/// Whether a module-path segment collides with a name the generators reserve
/// for their own emitted files (the union across languages — see
/// `specs/json-schema/generated-file-layout.md`). Reserving the union means a name
/// reserved in *any* target is rejected for *all*, keeping the flat package
/// coherent everywhere.
///
/// Both spellings of the shared runtime module are reserved at the package root,
/// because the targets spell it differently: Go and TypeScript emit
/// `definitions.go` / `definitions.ts`, while Python emits `_definitions.py`
/// (module-private, like `_recursive.py`). The aggregator names are reserved at
/// every depth because a parent's `./index` / `.__init__` import self-resolves.
/// `models` and `services` are generated *inside* a leaf directory; a leaf can
/// never also have children, so a module segment of either name is harmless.
fn is_reserved_module_name(segment: &str, depth: usize) -> bool {
    matches!(segment, "index" | "__init__")
        || (depth == 0 && matches!(segment, "definitions" | "_definitions" | "_recursive"))
}

/// Whether a module-path segment cannot be emitted as a module/package name,
/// and why.
///
/// A segment is used **verbatim** as a directory and file name and then as an
/// import path component: Python writes `from .<segment> import ...` and Java
/// writes `package <base>.<segment>;`. Only generated-file *names* were checked,
/// so an input `class.json` produced `from .class import Class` (a `SyntaxError`
/// at import) and `package outj.class;` (uncompilable) — while `2fa.json` gave a
/// leading-digit module in both.
///
/// Reserved words are checked against the **union** of the four targets, as the
/// generated-file names above already are: there is no per-segment escape hatch
/// (`x-<lang>-name` names types and members, not modules), so a per-language
/// rule would mean the same input loads for Go and rejects for Java — the
/// cross-language load disagreement P1 forbids.
fn module_segment_defect(segment: &str) -> Option<&'static str> {
    if !ident_is_syntactically_valid(segment) {
        return Some(
            "is not a valid module name (a module name must start with an ASCII letter or `_` and contain only ASCII letters, digits and `_`)",
        );
    }
    let reserved_in = [
        Language::Go,
        Language::TypeScript,
        Language::Python,
        Language::Java,
    ]
    .into_iter()
    .any(|language| ident_is_reserved(language, segment));
    reserved_in.then_some(
        "is a reserved word in one of the target languages (a module name is emitted verbatim as a package/import path component, which no target can escape)",
    )
}

fn module_path_from_relative_source(path: &Path) -> ModulePath {
    let mut segments = path
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>();
    if let Some(last) = segments.last_mut() {
        *last = strip_json_schema_extension(last).to_string();
    }
    ModulePath(segments)
}

/// Strips a JSON-Schema input file's extension and, if present, the
/// `.nexusrpc` naming-convention infix that marks the file as carrying a
/// Nexus service/operation envelope (e.g. `chat.nexusrpc.yaml` -> `chat`).
pub(crate) fn strip_json_schema_extension(name: &str) -> &str {
    let without_extension = name
        .strip_suffix(".json")
        .or_else(|| name.strip_suffix(".yaml"))
        .or_else(|| name.strip_suffix(".yml"))
        .unwrap_or(name);
    without_extension
        .strip_suffix(".nexusrpc")
        .unwrap_or(without_extension)
}

#[cfg(test)]
pub(crate) fn parse_api_spec_from_json_schema_for_language(
    language: Language,
    input: &str,
    path: PathBuf,
) -> Result<ApiSpec> {
    api_spec_from_json_schema_sources(language, vec![(path, input.to_string())])
}

fn api_spec_from_json_schema_sources(
    language: Language,
    sources: Vec<(PathBuf, String)>,
) -> Result<ApiSpec> {
    let parsed = parse_json_documents(language, sources)?;
    let paths = parsed.docs.keys().cloned().collect::<Vec<_>>();
    api_spec_from_parsed_json_documents(language, &parsed, &paths, None)
}

/// Parses, normalizes, and validates every input document, then collects the
/// models it declares. Language-aware because two stages resolve per-target
/// names: the inline-object-branch hoist (below) reads the branch's
/// `x-<lang>-name`, and the caller's identifier pass runs per target.
fn parse_json_documents(
    language: Language,
    sources: Vec<(PathBuf, String)>,
) -> Result<ParsedJsonDocuments> {
    if sources.is_empty() {
        return Err(Error::InvalidJsonSchema {
            path: PathBuf::from("<input>"),
            reason: "at least one JSON schema input path is required".to_string(),
        });
    }

    let mut docs = IndexMap::new();
    for (path, input) in sources {
        let doc = parse_json_schema_document(&path, &input)?;
        docs.insert(canonical(&path), (path, doc));
    }

    // Validate the authored grammar before `allOf` normalization can discard or
    // override a malformed branch. The semantic validators still run over the
    // merged schema below; this first walk owns keyword allowlists and raw value
    // shapes only.
    for (path, doc) in docs.values() {
        validate_raw_document_grammar(path, doc)?;
    }

    // Snapshot the raw (pre-merge) schemas so that `allOf` / `$ref`-with-siblings
    // folds can resolve and inline a `$ref` branch's target, then normalize every
    // schema in place (merging/flattening `allOf` into a single materialized
    // schema) so the rest of the pipeline — validation, ref collection, and every
    // backend — sees a plain merged schema with no combinator residue.
    let raw_models = collect_raw_models(&docs)?;
    let doc_paths: BTreeSet<PathBuf> = docs.keys().cloned().collect();
    let merge_ctx = MergeCtx {
        doc_paths: &doc_paths,
        raw_models: &raw_models,
    };
    let mut ref_fold_annotations = RefFoldAnnotations::new();
    let canonical_paths: Vec<PathBuf> = docs.keys().cloned().collect();
    for canonical_path in &canonical_paths {
        let (path, doc) = docs
            .get_mut(canonical_path)
            .expect("document present for canonical path");
        let path = path.clone();
        let annotations = ref_fold_annotations.entry(path.clone()).or_default();
        normalize_document(&path, canonical_path, doc, &merge_ctx, annotations)?;
    }

    for (path, doc) in docs.values() {
        validate_document(path, doc)?;
        if let Some(defs) = &doc.defs {
            validate_def_model_tree(path, defs, &[])?;
        }
        if root_is_schema_shaped(&doc.root) {
            validate_model_schema(path, &doc.root, "root schema")?;
        }
    }

    // Names every inline object shape — a property's, an element's, a map
    // member's, a `oneOf` branch's — by moving it into `$defs`. Runs after the
    // per-model validation above (so a defect inside a shape is reported at the
    // position the user wrote it) and before models are collected, so a hoisted
    // definition is an ordinary model from here on.
    hoist_inline_object_shapes(language, &mut docs, &ref_fold_annotations)?;

    let mut models = BTreeMap::<TypeKey, JsonModel>::new();
    for (canonical_path, (path, doc)) in &docs {
        if let Some(defs) = &doc.defs {
            collect_json_models_from_defs(path, canonical_path, defs, &[], &mut models)?;
        }
        if root_is_schema_shaped(&doc.root) {
            let model_name = root_model_name(path);
            // The root type and the file's `$defs` share one namespace (P15), and
            // the root's derived name *is* its model identity — the key every
            // `$ref` resolves through and every target emits one type for. A
            // `$defs` entry of that name is therefore a second schema under one
            // identity, which no `x-<lang>-name` override can separate (an
            // override moves the emitted identifier, not the identity), so the
            // only fixes are renames. Reject rather than let one shape win.
            if doc
                .defs
                .as_ref()
                .is_some_and(|defs| defs.contains_key(&model_name))
            {
                return Err(Error::InvalidJsonSchema {
                    path: path.to_path_buf(),
                    reason: format!(
                        "the root schema derives the type name `{model_name}` from the file name `{}`, and the same file declares `$defs.{model_name}`; the two are different schemas that would emit one type. Rename the `$defs` entry (and the `$ref`s that point at it), or rename the file so the root schema derives a different name — an `x-<lang>-name` override cannot separate them, because the derived name is the model's identity and not just its emitted identifier (P15 — the generator never auto-mangles)",
                        root_file_name(path),
                    ),
                });
            }
            models.insert(
                TypeKey::Root(canonical_path.clone()),
                JsonModel {
                    full_name: model_name.clone(),
                    canonical_path: canonical_path.clone(),
                    model_name,
                    schema: doc.root.clone(),
                },
            );
        }
    }

    validate_model_refs(&docs, &models)?;
    validate_all_unions(&docs, &models)?;
    validate_reference_satisfiability(&docs, &models)?;

    Ok(ParsedJsonDocuments { docs, models })
}

fn validate_def_model_tree(
    path: &Path,
    defs: &IndexMap<String, Schema>,
    parent_names: &[String],
) -> Result<()> {
    for (name, schema) in defs {
        let mut names = parent_names.to_vec();
        names.push(name.clone());
        let context = def_context(&names);
        validate_model_schema(path, schema, &context)?;
        if let Some(nested) = nested_defs(path, schema, &context)? {
            validate_def_model_tree(path, &nested, &names)?;
        }
    }
    Ok(())
}

fn api_spec_from_parsed_json_documents(
    language: Language,
    parsed: &ParsedJsonDocuments,
    canonical_paths: &[PathBuf],
    module_paths: Option<&BTreeMap<PathBuf, ModulePath>>,
) -> Result<ApiSpec> {
    let mut external_types = BTreeMap::new();
    for (key, model) in &parsed.models {
        if model_key_path(key).is_some_and(|path| canonical_paths.contains(path)) {
            insert_json_external_type(
                &mut external_types,
                model,
                &parsed.docs,
                &parsed.models,
                module_paths,
            )?;
            collect_schema_model_refs(
                &model.canonical_path,
                &model.canonical_path,
                &model.schema,
                &parsed.docs,
                &parsed.models,
                module_paths,
                &mut external_types,
            )?;
        }
    }

    let mut services = Vec::new();
    for canonical_path in canonical_paths {
        let Some((path, doc)) = parsed.docs.get(canonical_path) else {
            continue;
        };
        let Some(service_specs) = &doc.services else {
            continue;
        };
        validate_service_key_scope(path, language, service_specs)?;
        for (service_key, service) in service_specs {
            services.push(build_service(
                path,
                canonical_path,
                service_key,
                service,
                &parsed.docs,
                &parsed.models,
                module_paths,
                &mut external_types,
                language,
            )?);
        }
    }

    let owned_module_paths = canonical_paths
        .iter()
        .filter_map(|path| module_paths.and_then(|paths| paths.get(path)))
        .collect::<BTreeSet<_>>();
    let types = external_types
        .into_iter()
        .map(|(name, binding)| {
            let Some(json_type) = binding.json_model() else {
                unreachable!("JSON Schema parser only produces JSON model bindings");
            };
            let module_exported = module_paths.is_none()
                || json_type
                    .name
                    .module_path()
                    .is_some_and(|path| owned_module_paths.contains(path));
            let declaration = TypeDeclSpec::External(binding);
            (
                name,
                if module_exported {
                    TypeDeclEntry::module_export(declaration)
                } else {
                    // Declared by another input file. Marking it foreign rather
                    // than merely "not exported" is what lets a service file that
                    // declares no types of its own still import these instead of
                    // re-emitting them into its own module.
                    TypeDeclEntry::foreign(declaration)
                },
            )
        })
        .collect();
    let spec = ApiSpec {
        module_path: ModulePath::default(),
        data: (),
        version: "0.0.0".to_string(),
        support: SupportSpec::default(),
        services,
        types,
    };
    validate_identifier_namespace(language, &spec)?;
    Ok(spec)
}

/// Service keys are still available here in their authored spelling. Preserve
/// that distinction through the case-mapping check: folding `HTTPService` and
/// `HttpService` first makes both declarations look like a duplicate insertion
/// of one origin and silently loses a binding in Python.
fn validate_service_key_scope(
    path: &Path,
    language: Language,
    services: &IndexMap<String, Service>,
) -> Result<()> {
    let mut names = BTreeMap::<String, String>::new();
    for (service_key, service) in services {
        let override_name = lang_name_keyword(language)
            .and_then(|keyword| service.extra.get(keyword))
            .and_then(Value::as_str);
        let ident = override_name
            .map(str::to_string)
            .unwrap_or_else(|| match language {
                Language::TypeScript => recase_member(language, service_key),
                _ => service_key.to_upper_camel_case(),
            });
        if let Some(previous) = names.insert(ident.clone(), service_key.clone())
            && previous != *service_key
        {
            return Err(Error::InvalidJsonSchema {
                path: path.to_path_buf(),
                reason: format!(
                    "identifier collision in {} output: services `{previous}` and `{service_key}` both map to `{ident}`; disambiguate with an `{}` override (P15 — the generator never auto-mangles)",
                    language.as_str(),
                    lang_name_keyword(language).unwrap_or("x-<lang>-name"),
                ),
            });
        }
    }
    Ok(())
}

fn model_key_path(key: &TypeKey) -> Option<&PathBuf> {
    match key {
        TypeKey::Root(path) | TypeKey::Def(path, _) => Some(path),
    }
}

fn collect_schema_model_refs(
    path: &Path,
    canonical_path: &Path,
    schema: &Schema,
    docs: &IndexMap<PathBuf, (PathBuf, Document)>,
    models: &BTreeMap<TypeKey, JsonModel>,
    module_paths: Option<&BTreeMap<PathBuf, ModulePath>>,
    external_types: &mut BTreeMap<String, ExternalTypeBindingSpec>,
) -> Result<()> {
    if let Some(reference) = &schema.reference {
        let model = resolve_ref(path, canonical_path, reference, docs, models)?;
        let model_key = json_model_key(model, module_paths);
        if !external_types.contains_key(&model_key) {
            insert_json_external_type(external_types, model, docs, models, module_paths)?;
            collect_schema_model_refs(
                &model.canonical_path,
                &model.canonical_path,
                &model.schema,
                docs,
                models,
                module_paths,
                external_types,
            )?;
        }
        return Ok(());
    }
    if let Some(properties) = &schema.properties {
        for property in properties.values() {
            collect_schema_model_refs(
                path,
                canonical_path,
                property,
                docs,
                models,
                module_paths,
                external_types,
            )?;
        }
    }
    if let Some(items) = &schema.items {
        collect_schema_model_refs(
            path,
            canonical_path,
            items,
            docs,
            models,
            module_paths,
            external_types,
        )?;
    }
    if let Some(one_of) = &schema.one_of {
        for branch in one_of {
            collect_schema_model_refs(
                path,
                canonical_path,
                branch,
                docs,
                models,
                module_paths,
                external_types,
            )?;
        }
    }
    if let Some(additional) = &schema.additional_properties
        && additional.is_object()
    {
        let additional_schema =
            serde_json::from_value::<Schema>(additional.clone()).map_err(|error| {
                Error::InvalidJsonSchema {
                    path: path.to_path_buf(),
                    reason: format!("additionalProperties is invalid: {error}"),
                }
            })?;
        collect_schema_model_refs(
            path,
            canonical_path,
            &additional_schema,
            docs,
            models,
            module_paths,
            external_types,
        )?;
    }
    Ok(())
}

/// Validates document/service/operation allowlists and recursively validates the
/// raw schema grammar before normalization. In particular, an invalid `allOf`
/// branch must not become valid merely because a later branch overrides it.
fn validate_raw_document_grammar(path: &Path, doc: &Document) -> Result<()> {
    let reject = |reason: String| {
        Err(Error::InvalidJsonSchema {
            path: path.to_path_buf(),
            reason,
        })
    };

    validate_document_markers(path, doc)?;
    let has_nexus_envelope = doc.nexusrpc.is_some();
    if has_nexus_envelope {
        // `description` is the only Schema field belonging to the envelope.
        // Distinguish an unknown envelope member from a recognized schema
        // keyword accidentally authored at the document root.
        if let Some(keyword) = doc
            .root
            .extra
            .keys()
            .find(|keyword| !schema_extra_keyword_is_known(keyword))
        {
            return reject(format!("unknown Nexus envelope keyword `{keyword}`"));
        }
        if root_is_schema_shaped(&doc.root) {
            // Give rejected reference/dialect keywords their owning diagnostic
            // rather than the generic envelope-root remedy, which would tell
            // the author to move an equally-illegal keyword into `$defs`.
            validate_schema_common(path, &doc.root, "Nexus document")?;
            return reject(
                "a Nexus JSON schema document root is an envelope, not a model; move the model into `$defs`"
                    .to_string(),
            );
        }
        validate_annotations(path, &doc.root, "Nexus document")?;
    } else if root_is_schema_shaped(&doc.root) {
        validate_raw_schema_grammar(path, &doc.root, "root schema")?;
    } else {
        // Definitions-only documents may carry a document description.
        validate_annotations(path, &doc.root, "document")?;
    }

    if let Some(defs) = &doc.defs {
        for (name, schema) in defs {
            validate_raw_model_schema_grammar(path, schema, &format!("$defs.{name}"))?;
        }
    }

    if let Some(services) = &doc.services {
        for (service_name, service) in services {
            if service.endpoint.is_some() {
                return reject(format!(
                    "service `{service_name}`: `endpoint` is not supported in a Nexus JSON Schema document; configure the endpoint when registering the generated service"
                ));
            }
            for (keyword, value) in &service.extra {
                if LANG_NAME_KEYWORDS.contains(&keyword.as_str()) {
                    continue;
                }
                if keyword == "deprecated" {
                    if !value.is_boolean() {
                        return reject(format!(
                            "service `{service_name}`: `deprecated` must be a boolean, got {value}"
                        ));
                    }
                    continue;
                }
                return reject(format!(
                    "service `{service_name}` has unknown keyword `{keyword}`"
                ));
            }
            if service
                .description
                .as_ref()
                .is_some_and(|description| description.trim().is_empty())
            {
                return reject(format!(
                    "service `{service_name}`: `description` must not be empty or whitespace-only"
                ));
            }
            if let Some(control) = service
                .description
                .as_deref()
                .and_then(first_forbidden_doc_control)
            {
                return reject(format!(
                    "service `{service_name}`: `description` must not contain control character U+{:04X}",
                    control as u32
                ));
            }

            for (operation_name, operation) in &service.operations {
                for (keyword, value) in &operation.extra {
                    if LANG_NAME_KEYWORDS.contains(&keyword.as_str()) {
                        continue;
                    }
                    if keyword == "deprecated" {
                        if !value.is_boolean() {
                            return reject(format!(
                                "operation `{operation_name}`: `deprecated` must be a boolean, got {value}"
                            ));
                        }
                        continue;
                    }
                    return reject(format!(
                        "operation `{operation_name}` has unknown keyword `{keyword}`"
                    ));
                }
                if operation
                    .description
                    .as_ref()
                    .is_some_and(|description| description.trim().is_empty())
                {
                    return reject(format!(
                        "operation `{operation_name}`: `description` must not be empty or whitespace-only"
                    ));
                }
                if let Some(control) = operation
                    .description
                    .as_deref()
                    .and_then(first_forbidden_doc_control)
                {
                    return reject(format!(
                        "operation `{operation_name}`: `description` must not contain control character U+{:04X}",
                        control as u32
                    ));
                }
                for (label, schema) in [
                    ("input", operation.input.as_ref()),
                    ("output", operation.output.as_ref()),
                ] {
                    if let Some(schema) = schema {
                        validate_raw_schema_grammar(
                            path,
                            schema,
                            &format!("services.{service_name}.operations.{operation_name}.{label}"),
                        )?;
                    }
                }
            }
        }
    }
    Ok(())
}

fn validate_document_markers(path: &Path, doc: &Document) -> Result<()> {
    let has_nexus_envelope = doc.nexusrpc.is_some();
    if let Some(marker) = &doc.nexusrpc
        && marker.as_str() != Some("1.0.0")
    {
        return Err(Error::InvalidJsonSchema {
            path: path.to_path_buf(),
            reason: format!(
                "`nexusrpc` must be exactly the string \"1.0.0\"; document declares {marker}"
            ),
        });
    }
    if let Some(schema) = &doc.schema
        && schema.as_str() != Some("https://json-schema.org/draft/2020-12/schema")
    {
        return Err(Error::InvalidJsonSchema {
            path: path.to_path_buf(),
            reason: format!(
                "`$schema` must be the string `https://json-schema.org/draft/2020-12/schema`; document declares {schema}"
            ),
        });
    }
    if !has_nexus_envelope && doc.services.is_some() {
        return Err(Error::InvalidJsonSchema {
            path: path.to_path_buf(),
            reason: "`services` require a Nexus JSON schema document; add `nexusrpc: \"1.0.0\"` to enable Nexus service generation"
                .to_string(),
        });
    }
    if !has_nexus_envelope && !root_is_schema_shaped(&doc.root) && doc.defs.is_none() {
        return Err(Error::InvalidJsonSchema {
            path: path.to_path_buf(),
            reason: "plain JSON schema files must define a root schema or `$defs`".to_string(),
        });
    }
    Ok(())
}

fn validate_schema_keyword_allowlist(path: &Path, schema: &Schema, context: &str) -> Result<()> {
    for keyword in schema.extra.keys() {
        if keyword == "discriminator" {
            return Err(Error::InvalidJsonSchema {
                path: path.to_path_buf(),
                reason: format!(
                    "{context}: OpenAPI `discriminator` is not yet supported; express a closed object union with `oneOf` branches carrying one shared required `const` property"
                ),
            });
        }
        if !schema_extra_keyword_is_known(keyword) {
            return Err(Error::InvalidJsonSchema {
                path: path.to_path_buf(),
                reason: format!("{context}: unknown schema keyword `{keyword}`"),
            });
        }
    }
    Ok(())
}

fn validate_legacy_dependencies(path: &Path, schema: &Schema, context: &str) -> Result<()> {
    let Some(value) = schema.extra.get("dependencies") else {
        return Ok(());
    };
    let Some(entries) = value.as_object() else {
        return Err(Error::InvalidJsonSchema {
            path: path.to_path_buf(),
            reason: format!(
                "{context}: legacy draft-4..7 `dependencies` must be an object; use `dependentRequired` for property-name arrays or replace schema dependencies with explicit types"
            ),
        });
    };
    if entries.values().all(Value::is_array) {
        return Err(Error::InvalidJsonSchema {
            path: path.to_path_buf(),
            reason: format!(
                "{context}: legacy draft-4..7 `dependencies` array form is not a 2020-12 keyword; rename `dependencies` to `dependentRequired` (for example `dependencies: {{a: [b]}}` becomes `dependentRequired: {{a: [b]}}`)"
            ),
        });
    }
    Err(Error::InvalidJsonSchema {
        path: path.to_path_buf(),
        reason: format!(
            "{context}: legacy draft-4..7 schema-form `dependencies` is not supported; it corresponds to `dependentSchemas`, whose conditional subschema has no static lowering — split the variants into explicit types"
        ),
    })
}

/// Parses and recursively validates a raw schema-valued keyword. Boolean
/// schemas are valid JSON Schema grammar even where this subset later rejects
/// them for having no useful typed lowering.
fn validate_raw_subschema_value(
    path: &Path,
    value: &Value,
    context: &str,
    keyword: &str,
) -> Result<()> {
    match value {
        Value::Bool(_) => Ok(()),
        Value::Object(_) => {
            let schema: Schema = serde_json::from_value(value.clone()).map_err(|error| {
                Error::InvalidJsonSchema {
                    path: path.to_path_buf(),
                    reason: format!("{context}: `{keyword}` is not a valid schema: {error}"),
                }
            })?;
            validate_raw_schema_grammar(path, &schema, &format!("{context}.{keyword}"))
        }
        _ => Err(Error::InvalidJsonSchema {
            path: path.to_path_buf(),
            reason: format!("{context}: `{keyword}` must be a boolean or schema object"),
        }),
    }
}

fn validate_dependent_required_grammar(path: &Path, schema: &Schema, context: &str) -> Result<()> {
    let Some(value) = schema.extra.get("dependentRequired") else {
        return Ok(());
    };
    let reject = |reason: String| {
        Err(Error::InvalidJsonSchema {
            path: path.to_path_buf(),
            reason,
        })
    };
    let Some(entries) = value.as_object() else {
        return reject(format!(
            "{context}: `dependentRequired` must be an object mapping a property to the properties required alongside it"
        ));
    };
    for (trigger, dependents) in entries {
        let Some(dependents) = dependents.as_array() else {
            return reject(format!(
                "{context}: `dependentRequired.{trigger}` must be an array of property-name strings"
            ));
        };
        let mut seen = BTreeSet::new();
        for dependent in dependents {
            let Some(dependent) = dependent.as_str() else {
                return reject(format!(
                    "{context}: `dependentRequired.{trigger}` must contain only property-name strings"
                ));
            };
            if !seen.insert(dependent) {
                return reject(format!(
                    "{context}: `dependentRequired.{trigger}` lists `{dependent}` more than once; entries must be unique"
                ));
            }
        }
    }
    Ok(())
}

/// Checks the portable property-count representation before normalization can
/// merge an `allOf` branch (or `$ref` sibling) away. The complete object-shape
/// validation still runs later on the normalized schema.
fn validate_raw_property_count_grammar(path: &Path, schema: &Schema, context: &str) -> Result<()> {
    const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;
    for key in ["minProperties", "maxProperties"] {
        let Some(value) = schema.extra.get(key) else {
            continue;
        };
        let valid = value.as_f64().is_some_and(|value| {
            value.is_finite() && value >= 0.0 && value <= MAX_SAFE_INTEGER && value.fract() == 0.0
        });
        if !valid {
            return Err(Error::InvalidJsonSchema {
                path: path.to_path_buf(),
                reason: format!(
                    "{context}: `{key}` must be a non-negative integer no greater than 9007199254740991"
                ),
            });
        }
    }
    Ok(())
}

/// P7.1: a node declaring both bounds on one axis is a typo — one always
/// dominates. Checked here, on the schema **as authored**, as well as on the
/// merged result, because `finalize_merged` deliberately collapses the pair: an
/// inclusive bound in one `allOf` branch and an exclusive one in another *is*
/// the accepted "tighten across inclusive/exclusive" row, but the collapse also
/// swallowed the identical typo written inside a single branch — silently there,
/// loudly on a plain node.
fn validate_redundant_same_axis_bounds(path: &Path, schema: &Schema, context: &str) -> Result<()> {
    for (inclusive, exclusive) in [
        ("maximum", "exclusiveMaximum"),
        ("minimum", "exclusiveMinimum"),
    ] {
        // Only when both are numbers: the draft-4 boolean form
        // (`exclusiveMaximum: true` beside `maximum`) has its own, more specific
        // diagnostic in `validate_numeric_constraints`.
        if schema.extra.get(inclusive).is_some_and(Value::is_number)
            && schema.extra.get(exclusive).is_some_and(Value::is_number)
        {
            return Err(Error::InvalidJsonSchema {
                path: path.to_path_buf(),
                reason: format!(
                    "{context}: specify exactly one of `{inclusive}` or `{exclusive}`, not both"
                ),
            });
        }
    }
    Ok(())
}

/// The keywords a `propertyNames` key subschema may carry: the string
/// assertions the emitters can express on a key, `type` itself, and the inert
/// annotations accepted everywhere.
fn property_names_keyword_is_allowed(keyword: &str) -> bool {
    matches!(
        keyword,
        "type"
            | "minLength"
            | "maxLength"
            | "pattern"
            | "enum"
            | "format"
            | "title"
            | "description"
            | "$comment"
            | "examples"
    )
}

fn property_names_unsupported_keyword_reason(context: &str, keyword: &str) -> String {
    format!(
        "{context}: `propertyNames` with `{keyword}` is not supported; use only `minLength`, `maxLength`, `pattern`, `enum`, or an asserted `format`"
    )
}

fn validate_raw_schema_grammar(path: &Path, schema: &Schema, context: &str) -> Result<()> {
    validate_raw_schema_grammar_at(path, schema, context, false)
}

fn validate_raw_model_schema_grammar(path: &Path, schema: &Schema, context: &str) -> Result<()> {
    validate_raw_schema_grammar_at(path, schema, context, true)
}

fn validate_raw_schema_grammar_at(
    path: &Path,
    schema: &Schema,
    context: &str,
    defs_allowed: bool,
) -> Result<()> {
    // Validate a `not` subschema's own authored grammar before reporting that
    // this subset rejects `not` itself. Otherwise a typo inside it is hidden by
    // a broader fix-it and resurfaces if the author follows that advice.
    if let Some(negated) = schema.extra.get("not") {
        validate_raw_subschema_value(path, negated, context, "not")?;
    }
    // Run the owning rejected-keyword diagnostics before normalization can
    // replace them with a merge error or discard a later conjunct entirely.
    validate_schema_common(path, schema, context)?;
    validate_annotations(path, schema, context)?;
    validate_required_grammar(path, schema, context)?;
    validate_dependent_required_grammar(path, schema, context)?;
    validate_raw_property_count_grammar(path, schema, context)?;
    validate_redundant_same_axis_bounds(path, schema, context)?;
    validate_default(path, schema, context)?;

    if let Some(value) = schema.extra.get("uniqueItems")
        && !value.is_boolean()
    {
        return Err(Error::InvalidJsonSchema {
            path: path.to_path_buf(),
            reason: format!("{context}: `uniqueItems` must be a boolean"),
        });
    }

    if let Some(properties) = &schema.properties {
        for (name, property) in properties {
            validate_raw_schema_grammar(path, property, &format!("{context}.properties.{name}"))?;
        }
    }
    if let Some(items) = &schema.items {
        validate_raw_schema_grammar(path, items, &format!("{context}.items"))?;
    }
    if let Some(branches) = &schema.one_of {
        for (index, branch) in branches.iter().enumerate() {
            // Validate the nullability branch grammar before its individual
            // keywords. Otherwise a sibling such as `default: null` can emit
            // a value-keyword diagnostic and obscure the more fundamental
            // requirement that the null branch have no siblings at all.
            if branch.ty.as_ref().and_then(Value::as_str) == Some("null")
                && branch
                    != &(Schema {
                        ty: Some(Value::String("null".to_string())),
                        ..Schema::default()
                    })
            {
                return Err(Error::InvalidJsonSchema {
                    path: path.to_path_buf(),
                    reason: format!(
                        "{context}.oneOf[{index}]: a null branch must be exactly `{{type: \"null\"}}` with no sibling keywords"
                    ),
                });
            }
            validate_raw_schema_grammar(path, branch, &format!("{context}.oneOf[{index}]"))?;
        }
    }
    if let Some(additional) = &schema.additional_properties {
        match additional {
            Value::Bool(_) => {}
            Value::Object(_) => {
                validate_raw_subschema_value(path, additional, context, "additionalProperties")?
            }
            _ => {
                return Err(Error::InvalidJsonSchema {
                    path: path.to_path_buf(),
                    reason: format!(
                        "{context}: `additionalProperties` must be `true`, `false`, or a schema object"
                    ),
                });
            }
        }
    }

    if let Some(all_of) = schema.extra.get("allOf") {
        let Some(branches) = all_of.as_array() else {
            return Err(Error::InvalidJsonSchema {
                path: path.to_path_buf(),
                reason: format!("{context}: `allOf` must be an array of schemas"),
            });
        };
        for (index, branch) in branches.iter().enumerate() {
            match branch {
                Value::Bool(_) => {}
                Value::Object(_) => validate_raw_subschema_value(
                    path,
                    branch,
                    &format!("{context}.allOf[{index}]"),
                    "branch",
                )?,
                _ => {
                    return Err(Error::InvalidJsonSchema {
                        path: path.to_path_buf(),
                        reason: format!(
                            "{context}.allOf[{index}]: `branch` must be a schema object"
                        ),
                    });
                }
            }
        }
    }
    if let Some(negated) = schema.extra.get("not") {
        validate_raw_subschema_value(path, negated, context, "not")?;
    }
    // Preserve the more specific subset diagnostics for matcher keywords, but
    // still recurse into well-formed schema objects so unsupported keywords
    // cannot hide inside them.
    for keyword in ["contains", "propertyNames"] {
        if let Some(value @ Value::Object(_)) = schema.extra.get(keyword) {
            validate_raw_subschema_value(path, value, context, keyword)?;
        }
    }
    // The `propertyNames` allowlist has to run *before* normalization: a `$ref`
    // (or `allOf`) inside the key subschema is folded into its `type: string`
    // sibling by the merge pass, and the reject then arrives as
    // "`allOf` branches declare disjoint types" — a diagnostic about a keyword
    // the user never wrote.
    if let Some(Value::Object(map)) = schema.extra.get("propertyNames") {
        for keyword in map.keys() {
            if !property_names_keyword_is_allowed(keyword) {
                return Err(Error::InvalidJsonSchema {
                    path: path.to_path_buf(),
                    reason: property_names_unsupported_keyword_reason(context, keyword),
                });
            }
        }
    }
    for keyword in [
        "if",
        "then",
        "else",
        "contentSchema",
        "unevaluatedProperties",
        "unevaluatedItems",
    ] {
        if let Some(value) = schema.extra.get(keyword) {
            validate_raw_subschema_value(path, value, context, keyword)?;
        }
    }
    for keyword in ["anyOf", "prefixItems"] {
        if let Some(value) = schema.extra.get(keyword) {
            let Some(values) = value.as_array() else {
                return Err(Error::InvalidJsonSchema {
                    path: path.to_path_buf(),
                    reason: format!("{context}: `{keyword}` must be an array of schemas"),
                });
            };
            for (index, value) in values.iter().enumerate() {
                validate_raw_subschema_value(
                    path,
                    value,
                    &format!("{context}.{keyword}[{index}]"),
                    "branch",
                )?;
            }
        }
    }
    for keyword in ["$defs", "patternProperties", "dependentSchemas"] {
        if let Some(value) = schema.extra.get(keyword) {
            if keyword == "$defs" && !defs_allowed {
                return Err(Error::InvalidJsonSchema {
                    path: path.to_path_buf(),
                    reason: format!(
                        "{context}: `$defs` is only allowed at a document root or inside a named `$defs` entry; move this definition to the document's `$defs`"
                    ),
                });
            }
            let Some(entries) = value.as_object() else {
                return Err(Error::InvalidJsonSchema {
                    path: path.to_path_buf(),
                    reason: format!("{context}: `{keyword}` must be an object of schemas"),
                });
            };
            for (name, value) in entries {
                if keyword == "$defs" && value.is_object() {
                    let definition: Schema = serde_json::from_value(value.clone()).map_err(
                        |error| Error::InvalidJsonSchema {
                            path: path.to_path_buf(),
                            reason: format!(
                                "{context}.$defs.{name}: `schema` is not a valid schema: {error}"
                            ),
                        },
                    )?;
                    validate_raw_schema_grammar_at(
                        path,
                        &definition,
                        &format!("{context}.$defs.{name}"),
                        true,
                    )?;
                } else {
                    validate_raw_subschema_value(
                        path,
                        value,
                        &format!("{context}.{keyword}.{name}"),
                        "schema",
                    )?;
                }
            }
        }
    }
    Ok(())
}

fn validate_document(path: &Path, doc: &Document) -> Result<()> {
    validate_document_markers(path, doc)?;
    let has_nexus_envelope = doc.nexusrpc.is_some();
    if has_nexus_envelope && root_is_schema_shaped(&doc.root) {
        return Err(Error::InvalidJsonSchema {
            path: path.to_path_buf(),
            reason: "a Nexus JSON schema document root is an envelope, not a model; move the model into `$defs`"
                .to_string(),
        });
    }
    // A definitions-only pure file (only `$defs`, plus optional `description` /
    // `$schema`, and no `nexusrpc`) is a definitions bucket, not a type: it has
    // no file-root type and contributes its `$defs` alone. See
    // `specs/json-schema/input-files.md` (Definitions-only exception). We reject only
    // a plain file that carries neither a root schema nor any `$defs`.
    Ok(())
}

fn validate_model_schema(path: &Path, schema: &Schema, context: &str) -> Result<()> {
    validate_schema_common(path, schema, context)?;
    if schema.reference.is_some() {
        return Ok(());
    }
    // A named `oneOf` union (a `$def` whose body is a supported sum type) is a
    // model in its own right; the structural sum-type checks (decidable
    // selector, disjoint kinds, discriminator) run in the ref-resolving union
    // pass. See `specs/json-schema/features/oneOf.md`.
    if schema.one_of.is_some() && schema.ty.is_none() {
        return validate_schema_tree(path, schema, context);
    }
    if schema.ty.as_ref().and_then(Value::as_str) != Some("object") {
        // The root/model shape is unsupported, but a closed value set can
        // carry a more specific authored defect. Raw validation intentionally
        // leaves `const`/`enum` semantics until after `allOf` normalization;
        // preserve that owning diagnostic before reporting the broader model
        // shape restriction.
        validate_const_enum(path, schema, context)?;
        return Err(Error::InvalidJsonSchema {
            path: path.to_path_buf(),
            reason: format!("{context} must be `type: object`, a `oneOf` union, or a bare `$ref`"),
        });
    }
    validate_schema_tree(path, schema, context)
}

fn validate_schema_tree(path: &Path, schema: &Schema, context: &str) -> Result<()> {
    validate_schema_node(path, schema, context, false)
}

fn validate_schema_node(
    path: &Path,
    schema: &Schema,
    context: &str,
    is_union_branch: bool,
) -> Result<()> {
    validate_schema_common(path, schema, context)?;
    if schema.one_of.is_some() {
        let sibling = schema
            .ty
            .as_ref()
            .map(|_| "type")
            .or_else(|| schema.properties.as_ref().map(|_| "properties"))
            .or_else(|| schema.required.as_ref().map(|_| "required"))
            .or_else(|| {
                schema
                    .additional_properties
                    .as_ref()
                    .map(|_| "additionalProperties")
            })
            .or_else(|| schema.items.as_ref().map(|_| "items"))
            .or_else(|| {
                schema.extra.keys().find_map(|keyword| {
                    (!matches!(
                        keyword.as_str(),
                        "$comment" | "examples" | "deprecated" | "default"
                    ) && !LANG_NAME_KEYWORDS.contains(&keyword.as_str()))
                    .then_some(keyword.as_str())
                })
            });
        if let Some(sibling) = sibling {
            return Err(Error::InvalidJsonSchema {
                path: path.to_path_buf(),
                reason: format!(
                    "{context}: `{sibling}` cannot be a sibling of `oneOf`; move it into the branch whose values it constrains"
                ),
            });
        }
    }
    // A `type: "null"` is legal only as a nullability `oneOf` branch; the array
    // `type` form is never legal. Skip the standalone-null reject for branches.
    if !is_union_branch {
        validate_type_form(path, schema, context)?;
    } else if matches!(&schema.ty, Some(Value::Array(_))) {
        validate_type_form(path, schema, context)?;
    }
    validate_numeric_constraints(path, schema, context)?;
    validate_string_constraints(path, schema, context)?;
    validate_format(path, schema, context)?;
    validate_content_encoding(path, schema, context)?;
    validate_array_constraints(path, schema, context)?;
    validate_object_constraints(path, schema, context)?;
    validate_const_enum(path, schema, context)?;
    validate_default(path, schema, context)?;
    // Runs after the value keywords so a composite `const`/`enum`/`default` on a
    // shapeless `type: object` reports the more specific value diagnostic first.
    if is_union_branch {
        // A branch's *kind* is checked by the sum-type pass, which needs the
        // model set to resolve a `$ref` branch and so cannot run here. Its
        // *shape* is not: an itemless `{type: array}` branch loaded, and Java
        // then inferred `List<String>` where Go, TypeScript and Python inferred
        // `any`/`unknown`/`Any` — `[1, 2]` accepted by three targets and
        // rejected by the fourth.
        validate_type_shape(path, schema, context)?;
    } else {
        validate_type_presence(path, schema, context)?;
    }
    validate_annotations(path, schema, context)?;
    validate_required(path, schema, context)?;
    if let Some(properties) = &schema.properties {
        let required: Vec<&str> = schema
            .required
            .as_ref()
            .and_then(Value::as_array)
            .map(|values| values.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        for (name, property) in properties {
            // `default` on a `required` member is dead metadata (a required
            // member is never absent, so its default never applies) → reject
            // (P7.1). See `specs/json-schema/features/default.md`.
            if property.extra.contains_key("default") && required.contains(&name.as_str()) {
                return Err(Error::InvalidJsonSchema {
                    path: path.to_path_buf(),
                    reason: format!(
                        "{context}.properties.{name}: `default` on a required member never applies (a required member is always present); make the member optional, or drop the `default`"
                    ),
                });
            }
            validate_schema_node(
                path,
                property,
                &format!("{context}.properties.{name}"),
                false,
            )?;
        }
    }
    if let Some(items) = &schema.items {
        validate_schema_node(path, items, &format!("{context}.items"), false)?;
    }
    if let Some(one_of) = &schema.one_of {
        // Structural sum-type acceptance/rejection (decidable selector,
        // disjoint kinds, discriminator, `integer`+`number` overlap, …) needs
        // `$ref` resolution and runs in `validate_all_unions`. Here we only
        // recurse into each branch's own subtree.
        for branch in one_of {
            validate_schema_node(path, branch, &format!("{context}.oneOf"), true)?;
        }
    }
    if let Some(additional) = &schema.additional_properties {
        match additional {
            // `true` (open map) / `false` (closed object) are the accepted flags.
            Value::Bool(_) => {}
            Value::Object(_) => {
                let additional_schema = serde_json::from_value::<Schema>(additional.clone())
                    .map_err(|error| Error::InvalidJsonSchema {
                        path: path.to_path_buf(),
                        reason: format!("{context}.additionalProperties is invalid: {error}"),
                    })?;
                // `additionalProperties: {}` — the empty schema — means "any value",
                // exactly what `true` means; require the unambiguous spelling. The
                // pre-validation normalize pass re-serializes an empty schema into a
                // null-filled object, so compare against the default rather than an
                // empty map.
                if additional_schema == Schema::default() {
                    return Err(Error::InvalidJsonSchema {
                        path: path.to_path_buf(),
                        reason: format!(
                            "{context}.additionalProperties: an empty schema `{{}}` means any value; write `additionalProperties: true` instead"
                        ),
                    });
                }
                validate_schema_tree(
                    path,
                    &additional_schema,
                    &format!("{context}.additionalProperties"),
                )?;
            }
            _ => {
                return Err(Error::InvalidJsonSchema {
                    path: path.to_path_buf(),
                    reason: format!(
                        "{context}.additionalProperties must be `true`, `false`, or a schema object"
                    ),
                });
            }
        }
    }
    Ok(())
}

/// The fix-it reason for an unsupported keyword: points at the coherent
/// in-subset alternative rather than a bare "not supported" (see each keyword's
/// feature spec).
fn unsupported_keyword_reason(keyword: &str) -> &'static str {
    match keyword {
        "anyOf" => {
            "`anyOf` is not supported; a value-level union is expressed with a `oneOf` of pairwise-disjoint kinds"
        }
        "if" | "then" | "else" => {
            "conditional schemas (`if`/`then`/`else`) are not supported; model the alternatives as a `oneOf`"
        }
        "prefixItems" => {
            "tuple arrays (`prefixItems`) are not supported; use a single uniform `items` element type"
        }
        "unevaluatedProperties" => {
            "`unevaluatedProperties` is not supported; bound extra members with `additionalProperties` (`true`, `false`, or a value schema)"
        }
        "unevaluatedItems" => {
            "`unevaluatedItems` is not supported; bound the element type with `items`"
        }
        "dependentSchemas" => {
            "`dependentSchemas` is not supported; a conditional subschema has no static shape — split the variants into explicit types"
        }
        "patternProperties" => {
            "`patternProperties` is not supported; use a typed map (`additionalProperties: {type: ...}`) or enumerate the keys under `properties`"
        }
        "nullable" => {
            "OAS 3.0 `nullable` is not supported; model a nullable field with `oneOf: [{type: T}, {type: \"null\"}]`"
        }
        "$anchor" => {
            "`$anchor` is not supported; put the target in `$defs` and reference it with a static `#/$defs/<Name>` `$ref`"
        }
        "$dynamicRef" | "$dynamicAnchor" => {
            "`$dynamicRef`/`$dynamicAnchor` are not supported because their target varies with dynamic validation scope; use a static `#/$defs/<Name>` `$ref` resolved once at generation time"
        }
        "$vocabulary" => {
            "`$vocabulary` is not supported; it is a meta-schema keyword with no place in a type schema (the dialect is pinned to 2020-12)"
        }
        other => panic!("unsupported-keyword reason requested for unhandled keyword `{other}`"),
    }
}

fn validate_schema_common(path: &Path, schema: &Schema, context: &str) -> Result<()> {
    validate_schema_keyword_allowlist(path, schema, context)?;
    validate_legacy_dependencies(path, schema, context)?;
    // A malformed `type` is its own defect, not an absent one. It used to be
    // reported as "a leaf schema requires an explicit `type`" — and on a node
    // that also carries `$ref` or `oneOf` it was never reported at all, because
    // the presence check returns early for those.
    if let Some(ty) = &schema.ty
        && !ty.is_string()
        // The array form has its own, more specific diagnostic in
        // `validate_type_form`.
        && !ty.is_array()
    {
        return Err(Error::InvalidJsonSchema {
            path: path.to_path_buf(),
            reason: format!(
                "{context}: `type` must be a string naming one of `null`, `boolean`, `object`, `array`, `number`, `string`, `integer`; got {ty}"
            ),
        });
    }
    if schema.id.is_some() {
        return Err(Error::InvalidJsonSchema {
            path: path.to_path_buf(),
            reason: format!(
                "{context}: `$id` is not supported; remove `$id` because refs resolve by local file path plus a `$defs` JSON pointer"
            ),
        });
    }
    if let Some(reference) = &schema.reference {
        let file_part = reference
            .split_once('#')
            .map_or(reference.as_str(), |(file, _)| file);
        if ref_file_part_has_uri_scheme(file_part) {
            return Err(Error::InvalidJsonSchema {
                path: path.to_path_buf(),
                reason: format!(
                    "{context}: remote `$ref` `{reference}` is not supported; references must name local files without a URI scheme"
                ),
            });
        }
    }
    // `not` has degenerate forms the spec calls out with distinct diagnostics.
    if let Some(negated) = schema.extra.get("not") {
        let reason = match negated {
            Value::Object(map) if map.is_empty() => {
                "`not: {}` is unsatisfiable — it accepts no instance (a dead type)"
            }
            Value::Bool(true) => {
                "`not: true` is unsatisfiable — it accepts no instance (a dead type)"
            }
            Value::Bool(false) => {
                "`not: false` is a no-op — it constrains nothing (a dead keyword); remove it"
            }
            _ => {
                "`not` is not supported; state the positive `type`/constraints, or enumerate the admissible values with `enum`, rather than what is disallowed"
            }
        };
        return Err(Error::InvalidJsonSchema {
            path: path.to_path_buf(),
            reason: format!("{context}: {reason}"),
        });
    }
    for keyword in [
        "anyOf",
        "if",
        "then",
        "else",
        "prefixItems",
        "unevaluatedProperties",
        "unevaluatedItems",
        "dependentSchemas",
        "patternProperties",
        "$anchor",
        "$dynamicRef",
        "$dynamicAnchor",
        "$vocabulary",
        "nullable",
    ] {
        if schema.extra.contains_key(keyword) {
            return Err(Error::InvalidJsonSchema {
                path: path.to_path_buf(),
                reason: format!("{context}: {}", unsupported_keyword_reason(keyword)),
            });
        }
    }
    // The directional / content metadata keywords reject with a fix-it (they
    // have no single-type lowering — see the reject specs). `deprecated` is the
    // supported sibling; `examples`/`$comment` are accepted-and-ignored.
    if schema.extra.contains_key("readOnly") || schema.extra.contains_key("writeOnly") {
        return Err(Error::InvalidJsonSchema {
            path: path.to_path_buf(),
            reason: format!(
                "{context}: `readOnly`/`writeOnly` is not supported; a directional annotation has no single-type lowering (drop it, or split the type into request/response shapes)"
            ),
        });
    }
    if schema.extra.contains_key("contentMediaType") {
        return Err(Error::InvalidJsonSchema {
            path: path.to_path_buf(),
            reason: format!(
                "{context}: `contentMediaType` is not supported; the string is carried verbatim (drop it, or validate the media type in application code)"
            ),
        });
    }
    if schema.extra.contains_key("contentSchema") {
        return Err(Error::InvalidJsonSchema {
            path: path.to_path_buf(),
            reason: format!(
                "{context}: `contentSchema` is not supported; a schema over encoded string content has no native lowering (drop it, or model the decoded value as its own typed member)"
            ),
        });
    }
    Ok(())
}

/// Requires every leaf schema to name an explicit, known `type`, and requires
/// `type: object` / `type: array` to carry a concrete shape (see
/// `specs/json-schema/features/type.md`). A `oneOf` / `$ref` schema is exempt — its
/// shape comes from the branches or the referenced target; `allOf` is merged
/// away before validation runs. A union branch runs [`validate_type_shape`]
/// instead: its *kind* is the sum-type pass's to check, because only that pass
/// can resolve a `$ref` branch's target.
fn validate_type_presence(path: &Path, schema: &Schema, context: &str) -> Result<()> {
    if schema.reference.is_some() || schema.one_of.is_some() {
        return Ok(());
    }
    let reject = |reason: String| {
        Err(Error::InvalidJsonSchema {
            path: path.to_path_buf(),
            reason,
        })
    };
    // The array `type` form and standalone `type: "null"` are rejected earlier
    // by `validate_type_form`, so here an unreadable `type` means it is absent.
    let Some(name) = schema.ty.as_ref().and_then(Value::as_str) else {
        return reject(format!(
            "{context}: a leaf schema requires an explicit `type`; add one (e.g. `type: object`), or supply the shape via `oneOf`, `allOf`, or `$ref`"
        ));
    };
    const KNOWN: [&str; 7] = [
        "null", "boolean", "object", "array", "number", "string", "integer",
    ];
    if !KNOWN.contains(&name) {
        return reject(format!(
            "{context}: unknown `type` `{name}`; use one of `null`, `boolean`, `object`, `array`, `number`, `string`, `integer`"
        ));
    }
    validate_type_shape(path, schema, context)
}

/// The shape half of [`validate_type_presence`]: a declared `type` must carry the
/// keywords that give it a concrete form (`properties`/`additionalProperties` for
/// an object, `items` for an array) and none that belong to another form.
///
/// Split out because a `oneOf` branch reaches only this half: the branch's
/// *kind* is the sum-type pass's to check (it alone can resolve a `$ref`
/// branch), but nothing checked the branch's shape, so a shapeless one loaded
/// and each target inferred a different element type for it.
fn validate_type_shape(path: &Path, schema: &Schema, context: &str) -> Result<()> {
    let reject = |reason: String| {
        Err(Error::InvalidJsonSchema {
            path: path.to_path_buf(),
            reason,
        })
    };
    let Some(name) = schema.ty.as_ref().and_then(Value::as_str) else {
        return Ok(());
    };
    match name {
        "object" => {
            if schema.items.is_some() {
                return reject(format!("{context}: `items` requires `type: array`"));
            }
            if schema.properties.is_none() && schema.additional_properties.is_none() {
                return reject(format!(
                    "{context}: `type: object` needs an explicit shape; add `properties: {{...}}` (typed struct), `additionalProperties: true` (open map), or `additionalProperties: false` (closed empty object)"
                ));
            }
        }
        "array" => {
            if schema.properties.is_some() || schema.additional_properties.is_some() {
                return reject(format!(
                    "{context}: `properties`/`additionalProperties` require `type: object`"
                ));
            }
            if schema.items.is_none() {
                return reject(format!(
                    "{context}: `type: array` needs an explicit element type; add `items: {{...}}`"
                ));
            }
        }
        _ => {
            if schema.properties.is_some() || schema.additional_properties.is_some() {
                return reject(format!(
                    "{context}: `properties`/`additionalProperties` require `type: object`"
                ));
            }
            if schema.items.is_some() {
                return reject(format!("{context}: `items` requires `type: array`"));
            }
        }
    }
    Ok(())
}

/// Validates the context-independent grammar of `required`, returning its
/// unique names. Kept separate so raw `allOf` branches can be checked before
/// their property maps are unioned.
fn validate_required_grammar(
    path: &Path,
    schema: &Schema,
    context: &str,
) -> Result<BTreeSet<String>> {
    let Some(value) = &schema.required else {
        return Ok(BTreeSet::new());
    };
    let reject = |reason: String| {
        Err(Error::InvalidJsonSchema {
            path: path.to_path_buf(),
            reason,
        })
    };
    let Some(entries) = value.as_array() else {
        return reject(format!(
            "{context}: `required` must be an array of property-name strings"
        ));
    };
    let mut names = BTreeSet::new();
    for entry in entries {
        let Some(name) = entry.as_str() else {
            return reject(format!(
                "{context}: `required` may contain only property-name strings; `{entry}` is not a string"
            ));
        };
        if !names.insert(name.to_string()) {
            return reject(format!(
                "{context}: `required` lists `{name}` more than once; entries must be unique"
            ));
        }
    }
    Ok(names)
}

/// Load-time validation of `required` (see `specs/json-schema/features/required.md`):
/// the value must be an array of unique property-name strings, and every name
/// must be declared in `properties` (P7.1 — a mandatory member with no declared
/// shape is undecidable).
fn validate_required(path: &Path, schema: &Schema, context: &str) -> Result<()> {
    let names = validate_required_grammar(path, schema, context)?;
    let reject = |reason: String| {
        Err(Error::InvalidJsonSchema {
            path: path.to_path_buf(),
            reason,
        })
    };
    let declared: BTreeSet<&str> = schema
        .properties
        .as_ref()
        .map(|properties| properties.keys().map(String::as_str).collect())
        .unwrap_or_default();
    for name in &names {
        if !declared.contains(name.as_str()) {
            return reject(format!(
                "{context}: `required` names `{name}`, which is not declared in `properties`; add it to `properties` or remove it from `required`"
            ));
        }
    }
    Ok(())
}

/// Rejects the two unsupported spellings of `type`: the array form
/// (`["string","null"]`) and a standalone `type: "null"`. Both are degenerate
/// or ambiguous here — nullability is modeled with the dedicated
/// `oneOf:[{type:T},{type:"null"}]` convention (see `specs/json-schema/features/type.md`
/// and `nullability.md`). A `type: "null"` is legal *only* as one branch of that
/// `oneOf`, so this check is skipped for union branches by the caller.
fn validate_type_form(path: &Path, schema: &Schema, context: &str) -> Result<()> {
    match &schema.ty {
        Some(Value::Array(_)) => Err(Error::InvalidJsonSchema {
            path: path.to_path_buf(),
            reason: format!(
                "{context}: an array `type` (e.g. `[\"string\",\"null\"]`) is not supported; it is structurally ambiguous — model a nullable field with `oneOf: [{{type: T}}, {{type: \"null\"}}]` instead"
            ),
        }),
        Some(Value::String(name)) if name == "null" => Err(Error::InvalidJsonSchema {
            path: path.to_path_buf(),
            reason: format!(
                "{context}: a standalone `type: \"null\"` is not supported; a field that is always null carries no information — model a nullable field with `oneOf: [{{type: T}}, {{type: \"null\"}}]` instead"
            ),
        }),
        _ => Ok(()),
    }
}

/// Load-time validation of the numeric-constraint keywords (`minimum`,
/// `maximum`, `exclusiveMinimum`, `exclusiveMaximum`, `multipleOf`). See
/// `specs/json-schema/features/maximum.md` and `multipleOf.md` for the authoritative
/// rules. The keywords remain in the schema `extra` map for the backends; this
/// only rejects statically unsatisfiable / unsupported forms with fix-its.
fn validate_numeric_constraints(path: &Path, schema: &Schema, context: &str) -> Result<()> {
    const KEYWORDS: [&str; 5] = [
        "minimum",
        "maximum",
        "exclusiveMinimum",
        "exclusiveMaximum",
        "multipleOf",
    ];
    if KEYWORDS.iter().all(|key| !schema.extra.contains_key(*key)) {
        return Ok(());
    }

    let reject = |reason: String| {
        Err(Error::InvalidJsonSchema {
            path: path.to_path_buf(),
            reason,
        })
    };

    let ty = schema.ty.as_ref().and_then(Value::as_str);
    let is_integer = ty == Some("integer");
    let is_number = ty == Some("number");
    if !is_integer && !is_number {
        // P7.1: a numeric bound is statically meaningless on any non-numeric
        // type (including a bare/typeless or nullable `oneOf` node).
        return reject(format!(
            "{context}: numeric constraint keywords (`minimum`/`maximum`/`exclusiveMinimum`/`exclusiveMaximum`/`multipleOf`) require `type: integer` or `type: number`"
        ));
    }

    // Extract each keyword as a finite f64, rejecting non-numbers and the
    // draft-4/OAS-3.0 boolean form of the exclusive keywords.
    let bound = |key: &str| -> Result<Option<f64>> {
        match schema.extra.get(key) {
            None => Ok(None),
            Some(Value::Number(number)) => match number.as_f64() {
                Some(value) if value.is_finite() => Ok(Some(value)),
                _ => Err(Error::InvalidJsonSchema {
                    path: path.to_path_buf(),
                    reason: format!("{context}: `{key}` must be a finite number"),
                }),
            },
            Some(Value::Bool(_)) if key == "exclusiveMaximum" || key == "exclusiveMinimum" => {
                let inclusive = if key == "exclusiveMaximum" {
                    "maximum"
                } else {
                    "minimum"
                };
                Err(Error::InvalidJsonSchema {
                    path: path.to_path_buf(),
                    reason: format!(
                        "{context}: the draft-4/OpenAPI-3.0 boolean form `{key}: true` is not supported; write `{key}: <number>` for a strict bound (or `{inclusive}: <number>` for an inclusive one)"
                    ),
                })
            }
            Some(_) => Err(Error::InvalidJsonSchema {
                path: path.to_path_buf(),
                reason: format!("{context}: `{key}` must be a number"),
            }),
        }
    };

    let minimum = bound("minimum")?;
    let maximum = bound("maximum")?;
    let exclusive_minimum = bound("exclusiveMinimum")?;
    let exclusive_maximum = bound("exclusiveMaximum")?;
    let multiple_of = bound("multipleOf")?;

    // Integer fields require integer-valued bounds (Pydantic cannot build a
    // fractional `le`/`ge` on an int field; keeps cross-language comparison
    // exact).
    if is_integer {
        for (key, value) in [
            ("minimum", minimum),
            ("maximum", maximum),
            ("exclusiveMinimum", exclusive_minimum),
            ("exclusiveMaximum", exclusive_maximum),
        ] {
            if let Some(value) = value
                && value.fract() != 0.0
            {
                return reject(format!(
                    "{context}: `{key}` must be an integer bound on an `integer` field (got {value}); use an integer bound, or make the field `type: number`"
                ));
            }
        }
    }

    // `multipleOf` must be a positive integer (fractional divisors deferred).
    if let Some(divisor) = multiple_of {
        let authored = &schema.extra["multipleOf"];
        if divisor <= 0.0 {
            return reject(format!(
                "{context}: `multipleOf` must be greater than 0 (got {authored})"
            ));
        }
        if divisor.fract() != 0.0 {
            return reject(format!(
                "{context}: `multipleOf: {authored}` is not yet supported; fractional divisors are deferred, use a positive integer divisor"
            ));
        }
        if is_integer && divisor > 9_007_199_254_740_991.0 {
            return reject(format!(
                "{context}: `multipleOf: {authored}` exceeds the portable integer-divisor ceiling 9007199254740991; use a smaller positive integer divisor"
            ));
        }
    }

    // Same-axis redundant pairs (P7.1): one bound always dominates.
    if maximum.is_some() && exclusive_maximum.is_some() {
        return reject(format!(
            "{context}: specify exactly one of `maximum` or `exclusiveMaximum`, not both"
        ));
    }
    if minimum.is_some() && exclusive_minimum.is_some() {
        return reject(format!(
            "{context}: specify exactly one of `minimum` or `exclusiveMinimum`, not both"
        ));
    }

    // Satisfiability of the combined bounds (empty accepted set → reject).
    let lower = minimum
        .map(|value| (value, false))
        .or(exclusive_minimum.map(|value| (value, true)));
    let upper = maximum
        .map(|value| (value, false))
        .or(exclusive_maximum.map(|value| (value, true)));
    if is_integer {
        const INTEGER_CAP: f64 = 9_007_199_254_740_991.0;
        let below_floor = upper.is_some_and(|(value, exclusive)| {
            value < -INTEGER_CAP || (value == -INTEGER_CAP && exclusive)
        });
        let above_ceiling = lower.is_some_and(|(value, exclusive)| {
            value > INTEGER_CAP || (value == INTEGER_CAP && exclusive)
        });
        if below_floor || above_ceiling {
            return reject(format!(
                "{context}: the numeric bounds describe an empty range outside the portable ±9007199254740991 integer cap"
            ));
        }
    }
    if let (Some((lo, lo_exclusive)), Some((hi, hi_exclusive))) = (lower, upper) {
        let empty = if is_integer {
            let smallest = if lo_exclusive { lo + 1.0 } else { lo };
            let largest = if hi_exclusive { hi - 1.0 } else { hi };
            smallest > largest
        } else {
            lo > hi || (lo == hi && (lo_exclusive || hi_exclusive))
        };
        if empty {
            return reject(format!(
                "{context}: the numeric bounds describe an empty range (no value can satisfy them)"
            ));
        }
    }

    // Range + `multipleOf`: reject when no runtime-representable binary64
    // multiple lies in the range. Not gated on `is_integer` — `multipleOf`
    // restricts a `number` to the same discrete lattice, so `{type: number,
    // minimum: 1, maximum: 2, multipleOf: 5}` is exactly as empty as its
    // integer twin.
    if let Some(divisor) = multiple_of
        && divisor.is_finite()
        && divisor > 0.0
        && let (Some((lo, lo_exclusive)), Some((hi, hi_exclusive))) = (lower, upper)
    {
        let authored_divisor = &schema.extra["multipleOf"];
        if !binary64_range_contains_multiple(lo, lo_exclusive, hi, hi_exclusive, divisor) {
            return reject(format!(
                "{context}: no multiple of {authored_divisor} lies within the accepted range"
            ));
        }
    }

    // A pinned literal (`const`/`default`) or any closed-set `enum` member on the
    // same node must satisfy the bounds — a value the field can never legally
    // hold is a schema bug (P13.1).
    let bound_violation = |value: f64| -> Option<String> {
        if let Some(max) = maximum
            && value > max
        {
            Some(format!("must be <= {max}"))
        } else if let Some(min) = minimum
            && value < min
        {
            Some(format!("must be >= {min}"))
        } else if let Some(excl) = exclusive_maximum
            && value >= excl
        {
            Some(format!("must be < {excl}"))
        } else if let Some(excl) = exclusive_minimum
            && value <= excl
        {
            Some(format!("must be > {excl}"))
        } else if let Some(divisor) = multiple_of
            // IEEE `fmod`, not `(value / divisor).fract()`: the quotient of two
            // doubles has no fractional part at all above 2^52, so the old form
            // silently returned "divisible" for every large literal — a third
            // divisibility semantics, disagreeing with all four runtimes.
            && value % divisor != 0.0
        {
            Some(format!("must be a multiple of {divisor}"))
        } else {
            None
        }
    };
    for literal_key in ["const", "default"] {
        let Some(Value::Number(number)) = schema.extra.get(literal_key) else {
            continue;
        };
        let Some(value) = number.as_f64() else {
            continue;
        };
        if let Some(reason) = bound_violation(value) {
            return reject(format!(
                "{context}: `{literal_key}` value {value} violates the numeric bounds ({reason})"
            ));
        }
    }
    if let Some(Value::Array(members)) = schema.extra.get("enum") {
        for member in members {
            let Some(value) = member.as_f64() else {
                continue;
            };
            if let Some(reason) = bound_violation(value) {
                return reject(format!(
                    "{context}: `enum` value {value} violates the numeric bounds ({reason})"
                ));
            }
        }
    }

    Ok(())
}

/// Whether the bounded binary64 interval contains a value for which every
/// generated runtime's `%`/`fmod` check reports an exact zero remainder.
///
/// Computing `(lo / divisor).ceil() * divisor` is insufficient: above 2^52 the
/// multiply can round a mathematical multiple onto an adjacent double whose
/// runtime remainder is nonzero. Instead, inspect the binary64 lattice one
/// exponent bin at a time. A positive normal double is `significand * 2^step`;
/// after factoring the integral divisor into `odd * 2^power`, divisibility is a
/// simple significand modulus. The final `%` is deliberate: it pins this
/// load-time proof to the check emitted by all four targets.
fn binary64_range_contains_multiple(
    lo: f64,
    lo_exclusive: bool,
    hi: f64,
    hi_exclusive: bool,
    divisor: f64,
) -> bool {
    let lower = if lo_exclusive { next_binary64(lo) } else { lo };
    let upper = if hi_exclusive {
        previous_binary64(hi)
    } else {
        hi
    };
    if lower > upper {
        return false;
    }
    if lower <= 0.0 && upper >= 0.0 {
        return true;
    }
    if upper < 0.0 {
        return positive_binary64_range_contains_multiple(-upper, -lower, divisor);
    }
    positive_binary64_range_contains_multiple(lower, upper, divisor)
}

fn next_binary64(value: f64) -> f64 {
    if value == f64::INFINITY {
        return value;
    }
    if value == -0.0 {
        return f64::from_bits(1);
    }
    let bits = value.to_bits();
    f64::from_bits(if value > 0.0 { bits + 1 } else { bits - 1 })
}

fn previous_binary64(value: f64) -> f64 {
    if value == f64::NEG_INFINITY {
        return value;
    }
    if value == 0.0 {
        return -f64::from_bits(1);
    }
    let bits = value.to_bits();
    f64::from_bits(if value > 0.0 { bits - 1 } else { bits + 1 })
}

fn positive_binary64_range_contains_multiple(lower: f64, upper: f64, divisor: f64) -> bool {
    // `multipleOf` is already gated to a positive integral binary64 value.
    // Factor its exact representation into odd * 2^power.
    let divisor_bits = divisor.to_bits();
    let divisor_exponent = ((divisor_bits >> 52) & 0x7ff) as i32 - 1023 - 52;
    let divisor_significand = (1_u64 << 52) | (divisor_bits & ((1_u64 << 52) - 1));
    let trailing = divisor_significand.trailing_zeros();
    let divisor_odd = divisor_significand >> trailing;
    let divisor_power = divisor_exponent + trailing as i32;
    debug_assert!(divisor_power >= 0, "validated divisor is integral");

    let lower = lower.max(1.0);
    if lower > upper {
        return false;
    }
    let lower_bits = lower.to_bits();
    let upper_bits = upper.to_bits();
    let first_bin = (lower_bits >> 52) & 0x7ff;
    let last_bin = (upper_bits >> 52) & 0x7ff;
    let fraction_mask = (1_u64 << 52) - 1;

    for exponent_bits in first_bin..=last_bin {
        // Intersect the accepted interval with this exponent bin and translate
        // its endpoints to the exact 53-bit significand interval.
        let bin_start = exponent_bits << 52;
        let bin_end = bin_start | fraction_mask;
        let start_bits = lower_bits.max(bin_start);
        let end_bits = upper_bits.min(bin_end);
        if start_bits > end_bits {
            continue;
        }
        let start = (1_u64 << 52) | (start_bits & fraction_mask);
        let end = (1_u64 << 52) | (end_bits & fraction_mask);
        let value_power = exponent_bits as i32 - 1023 - 52;

        let modulus = if value_power >= divisor_power {
            divisor_odd as u128
        } else {
            let shift = (divisor_power - value_power) as u32;
            let Some(modulus) = (divisor_odd as u128).checked_shl(shift) else {
                continue;
            };
            modulus
        };
        let start = start as u128;
        let end = end as u128;
        let first = start.div_ceil(modulus) * modulus;
        if first > end {
            continue;
        }

        let candidate_significand = first as u64;
        let candidate = f64::from_bits(
            bin_start | (candidate_significand.saturating_sub(1_u64 << 52) & fraction_mask),
        );
        if candidate >= lower && candidate <= upper && candidate % divisor == 0.0 {
            return true;
        }
    }
    false
}

/// Load-time validation of the string-length keywords (`minLength`,
/// `maxLength`). See `specs/json-schema/features/maxLength.md` for the authoritative
/// rules. Length is counted in Unicode code points. The keywords remain in the
/// schema `extra` map for the backends; this only rejects statically
/// unsatisfiable / unsupported forms with fix-its.
fn validate_string_constraints(path: &Path, schema: &Schema, context: &str) -> Result<()> {
    const KEYWORDS: [&str; 2] = ["minLength", "maxLength"];
    if KEYWORDS.iter().all(|key| !schema.extra.contains_key(*key)) {
        return Ok(());
    }

    let reject = |reason: String| {
        Err(Error::InvalidJsonSchema {
            path: path.to_path_buf(),
            reason,
        })
    };

    // P7.1: a string-length bound is statically meaningless on a non-string
    // type (the array-length analog is `maxItems`, the member-count analog is
    // `maxProperties`).
    if schema.ty.as_ref().and_then(Value::as_str) != Some("string") {
        return reject(format!(
            "{context}: string-length keywords (`minLength`/`maxLength`) require `type: string`"
        ));
    }

    // Each bound must be a non-negative integer. A `.0`-valued float is accepted
    // as its integer value (honoring the `1.0`-as-integer rule from `type`).
    const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;
    let bound = |key: &str| -> Result<Option<u64>> {
        match schema.extra.get(key) {
            None => Ok(None),
            Some(Value::Number(number)) => match number.as_f64() {
                Some(value)
                    if value.is_finite()
                        && value >= 0.0
                        && value <= MAX_SAFE_INTEGER
                        && value.fract() == 0.0 =>
                {
                    Ok(Some(value as u64))
                }
                _ => Err(Error::InvalidJsonSchema {
                    path: path.to_path_buf(),
                    reason: format!(
                        "{context}: `{key}` must be a non-negative integer no greater than 9007199254740991"
                    ),
                }),
            },
            Some(_) => Err(Error::InvalidJsonSchema {
                path: path.to_path_buf(),
                reason: format!(
                    "{context}: `{key}` must be a non-negative integer no greater than 9007199254740991"
                ),
            }),
        }
    };

    let min_length = bound("minLength")?;
    let max_length = bound("maxLength")?;

    // `minLength > maxLength` is unsatisfiable; `minLength == maxLength` pins an
    // exact length (accepted — a fixed-width string).
    if let (Some(min), Some(max)) = (min_length, max_length)
        && min > max
    {
        return reject(format!(
            "{context}: `minLength` ({min}) exceeds `maxLength` ({max}); the bounds describe an empty range (no string can satisfy them)"
        ));
    }

    // A `const`/`default`/`enum` string literal on the same node must satisfy
    // the bounds (code-point length) at load.
    let check_literal = |literal: &str, source: &str| -> Result<()> {
        let length = literal.chars().count() as u64;
        if let Some(max) = max_length
            && length > max
        {
            return reject(format!(
                "{context}: `{source}` value {literal:?} has length {length}, exceeding `maxLength` {max}"
            ));
        }
        if let Some(min) = min_length
            && length < min
        {
            return reject(format!(
                "{context}: `{source}` value {literal:?} has length {length}, below `minLength` {min}"
            ));
        }
        Ok(())
    };
    for literal_key in ["const", "default"] {
        if let Some(Value::String(literal)) = schema.extra.get(literal_key) {
            check_literal(literal, literal_key)?;
        }
    }
    if let Some(Value::Array(values)) = schema.extra.get("enum") {
        for value in values {
            if let Some(literal) = value.as_str() {
                check_literal(literal, "enum")?;
            }
        }
    }

    Ok(())
}

/// Load-time gate for the `format` keyword (JSON Schema 2020-12 §7). We opt into
/// `format-assertion` semantics for a curated portable subset and reject
/// everything else at load, so no `format` silently no-ops (P10). See
/// `specs/json-schema/features/format.md` and `crate::json_schema::format`.
///
/// Rejects (P7 / P7.1): a non-string `format` value, a `format` on a
/// non-`string` node, an unknown/non-standard name (with a fix-it), a
/// deferred standard format, the temporal formats (materialization pending), and
/// a `const`/`default`/`enum` string literal on the same node that fails its
/// format. The `format` value stays in the schema `extra` map for the backends.
fn validate_format(path: &Path, schema: &Schema, context: &str) -> Result<()> {
    let Some(value) = schema.extra.get("format") else {
        return Ok(());
    };

    let reject = |reason: String| -> Result<()> {
        Err(Error::InvalidJsonSchema {
            path: path.to_path_buf(),
            reason,
        })
    };

    let Some(format) = value.as_str() else {
        return reject(format!("{context}: `format` must be a string"));
    };

    // P7.1: `format` names a semantic shape of a string; it is statically
    // meaningless on any other type (a vacuous no-op the spec would allow, a load
    // reject here — as [[pattern]] treats a type mismatch).
    if schema.ty.as_ref().and_then(Value::as_str) != Some("string") {
        return reject(format!("{context}: `format` requires `type: string`"));
    }

    let (format_name, materializes) = match crate::json_schema::format::classify(format) {
        crate::json_schema::format::FormatClass::Supported(check) => (check.name, false),
        // The temporal formats are materialized into native typed fields with a
        // narrowed grammar (leap `:60` rejected; `duration` time-only). Any
        // supplied literal is validated against that materialized grammar below.
        crate::json_schema::format::FormatClass::Temporal(kind) => (kind.name(), true),
        crate::json_schema::format::FormatClass::Deferred => {
            return reject(format!(
                "{context}: `format: {format}` is not yet supported (deferred); \
                 it needs IDNA/Unicode or templating handling that is not yet portable"
            ));
        }
        crate::json_schema::format::FormatClass::Unknown => {
            return reject(format!(
                "{context}: unknown `format: {format}`; supported formats are {}",
                crate::json_schema::format::SUPPORTED_FORMATS.join(", ")
            ));
        }
    };

    // Both temporal formats and contentEncoding replace the schema's string
    // slot with a native value, but with incompatible types. String-shaped
    // formats remain valid beside contentEncoding: they constrain the encoded
    // wire string before the supported bytes materialization.
    if materializes && schema.extra.contains_key("contentEncoding") {
        return reject(format!(
            "{context}: materializing `format: {format}` cannot be combined with \
             `contentEncoding`; both replace the same string slot with incompatible \
             native types (remove one of the two keywords)"
        ));
    }

    // A supplied `const`/`default`/`enum` string literal on the same node must
    // satisfy the format at load (the literal-vs-constraint obligation).
    let check_literal = |literal: &str, source: &str| -> Result<()> {
        if !crate::json_schema::format::is_valid(format, literal) {
            return reject(format!(
                "{context}: `{source}` value {literal:?} is not a valid {format_name}"
            ));
        }
        Ok(())
    };
    for literal_key in ["const", "default"] {
        if let Some(Value::String(literal)) = schema.extra.get(literal_key) {
            check_literal(literal, literal_key)?;
        }
    }
    if let Some(Value::Array(values)) = schema.extra.get("enum") {
        for value in values {
            if let Some(literal) = value.as_str() {
                check_literal(literal, "enum")?;
            }
        }
    }

    Ok(())
}

/// Load-time gate for the `contentEncoding` keyword (JSON Schema 2020-12 §8.3).
/// We opt into assertion + materialization for the two byte-transform encodings
/// (`base64` / `base64url`, materialized to a native bytes type) and reject every
/// other encoding at load, so no `contentEncoding` silently no-ops (P10). See
/// `specs/json-schema/features/contentEncoding.md` and `crate::json_schema::content_encoding`.
///
/// Rejects (P7 / P7.1): a non-string `contentEncoding` value, a
/// `contentEncoding` on a non-`string` node, an unsupported encoding (with a
/// fix-it), a co-occurring materializing temporal `format` or
/// `contentMediaType` / `contentSchema` (owned by those
/// features, which have nowhere to emit the label in the model), and a
/// `const`/`default`/`enum` string literal that is not well-formed for the
/// declared encoding. The value stays in the schema `extra` map for the backends.
fn validate_content_encoding(path: &Path, schema: &Schema, context: &str) -> Result<()> {
    let Some(value) = schema.extra.get("contentEncoding") else {
        return Ok(());
    };

    let reject = |reason: String| -> Result<()> {
        Err(Error::InvalidJsonSchema {
            path: path.to_path_buf(),
            reason,
        })
    };

    // The spec's own MUST: the value MUST be a string.
    let Some(encoding_name) = value.as_str() else {
        return reject(format!("{context}: `contentEncoding` must be a string"));
    };

    // P7.1: `contentEncoding` describes a string that is really encoded binary;
    // it is statically meaningless on any other type (a vacuous no-op the spec
    // would allow, a load reject here — as [[format]] / [[pattern]] treat a type
    // mismatch).
    if schema.ty.as_ref().and_then(Value::as_str) != Some("string") {
        return reject(format!(
            "{context}: `contentEncoding` requires `type: string`"
        ));
    }

    let encoding = match crate::json_schema::content_encoding::classify(encoding_name) {
        crate::json_schema::content_encoding::EncodingClass::Supported(encoding) => encoding,
        crate::json_schema::content_encoding::EncodingClass::Unsupported => {
            return reject(format!(
                "{context}: `contentEncoding: {encoding_name}` is not supported; \
                 supported encodings are {}",
                crate::json_schema::content_encoding::SUPPORTED_ENCODINGS.join(", ")
            ));
        }
    };

    // `contentMediaType` / `contentSchema` on the same node are owned by their
    // own features (a base64 blob labeled with a media type has nowhere to emit
    // the label in the model); the reject there wins over materialization here.
    for labeled in ["contentMediaType", "contentSchema"] {
        if schema.extra.contains_key(labeled) {
            return reject(format!(
                "{context}: `{labeled}` alongside `contentEncoding` is not supported \
                 (materialized bytes are unlabeled binary; drop `{labeled}`)"
            ));
        }
    }

    // A supplied `const`/`default`/`enum` string literal on the same node must be
    // well-formed for the declared encoding at load (the literal-vs-constraint
    // obligation), and is thereby stored / echoed in its canonical form.
    let check_literal = |literal: &str, source: &str| -> Result<()> {
        if !crate::json_schema::content_encoding::is_valid(encoding, literal) {
            return reject(format!(
                "{context}: `{source}` value {literal:?} is not valid {}-encoded data",
                encoding.name()
            ));
        }
        Ok(())
    };
    for literal_key in ["const", "default"] {
        if let Some(Value::String(literal)) = schema.extra.get(literal_key) {
            check_literal(literal, literal_key)?;
        }
    }
    if let Some(Value::Array(values)) = schema.extra.get("enum") {
        for value in values {
            if let Some(literal) = value.as_str() {
                check_literal(literal, "enum")?;
            }
        }
    }

    Ok(())
}

/// The scalar JSON kind of a schema `type` string, if it names a scalar. Object
/// and array (and unknown) yield `None` — the composite line the array
/// assertions draw their support envelope at.
fn scalar_type(ty: Option<&str>) -> Option<&'static str> {
    match ty {
        Some("string") => Some("string"),
        Some("boolean") => Some("boolean"),
        Some("integer") => Some("integer"),
        Some("number") => Some("number"),
        _ => None,
    }
}

/// The scalar JSON kind of a literal value (for `const`/`enum` matcher
/// compatibility). Composite values (arrays/objects) yield `None`.
fn scalar_value_kind(value: &Value) -> Option<&'static str> {
    match value {
        Value::String(_) => Some("string"),
        Value::Bool(_) => Some("boolean"),
        Value::Number(number) => {
            if number.as_f64().is_some_and(|value| value.fract() == 0.0) {
                Some("integer")
            } else {
                Some("number")
            }
        }
        _ => None,
    }
}

/// True when two scalar schemas have any overlap. This relation is symmetric
/// and is used for matcher-vs-element compatibility.
fn scalar_kinds_overlap(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    matches!((a, b), ("integer", "number") | ("number", "integer"))
}

/// Whether a scalar literal kind can inhabit the declared schema kind. Unlike
/// overlap this is directional: an integer literal inhabits `number`, while a
/// fractional number never inhabits `integer`.
fn scalar_value_assignable(declared: &str, value: &str) -> bool {
    declared == value || (declared == "number" && value == "integer")
}

/// Load-time validation of the `default` annotation's own shape. `default` is a
/// pure annotation (no validator); the only load obligations are shape checks
/// (see `specs/json-schema/features/default.md`). The `const`+`default` /
/// `enum`+`default` interactions live in `validate_const_enum`, and the
/// default-against-constraint checks live in the numeric/string/content
/// validators; the `default` on a `required` member is caught at the parent
/// object level. This function enforces: no `null` default (degenerate), no
/// object/array default (deferred to composite-value materialization), and a
/// scalar default that is type-compatible with the declared `type`.
fn validate_default(path: &Path, schema: &Schema, context: &str) -> Result<()> {
    let Some(default) = schema.extra.get("default") else {
        return Ok(());
    };
    let reject = |reason: String| {
        Err(Error::InvalidJsonSchema {
            path: path.to_path_buf(),
            reason,
        })
    };
    // `default: null` → reject as degenerate (mirrors `const: null`).
    if default.is_null() {
        return reject(format!(
            "{context}: `default: null` is not supported; on a non-nullable member it is invalid, and on a nullable one it is a no-op (absence already surfaces as null) — drop it"
        ));
    }
    // Object/array default → reject (deferred; composite-value materialization is
    // not yet specified — the member's *type* may still be object/array).
    let Some(value_kind) = scalar_value_kind(default) else {
        return reject(format!(
            "{context}: an object/array `default` value is not yet supported; only scalar (string/number/integer/boolean) defaults are materialized on read (scalar values only)"
        ));
    };
    // Scalar default: it must be assignable to the declared `type`. A typeless
    // node (a bare/nullable `oneOf`) carries no scalar type to clash with, so a
    // scalar default is accepted there.
    let ty = schema.ty.as_ref().and_then(Value::as_str);
    match scalar_type(ty) {
        Some(declared) if !scalar_value_assignable(declared, value_kind) => reject(format!(
            "{context}: `default` value {default} (of kind `{value_kind}`) is incompatible with `type: {}`",
            ty.unwrap_or("")
        )),
        None if ty.is_some() => reject(format!(
            "{context}: `default` value {default} (of kind `{value_kind}`) is incompatible with `type: {}`",
            ty.unwrap_or("")
        )),
        _ => Ok(()),
    }
}

/// Load-time validation of the metadata annotations: `title` (§9.1),
/// `deprecated` (§9.3), `$comment` (core §8.3), and `examples` (§9.5). None
/// contribute a validator; these are pure shape checks (see the feature specs).
/// `title` becomes the doc-comment summary line, `deprecated` a native marker;
/// `examples` and `$comment` are accepted and dropped (never leak into output).
fn first_forbidden_doc_control(text: &str) -> Option<char> {
    text.chars()
        .find(|character| *character < ' ' && !matches!(character, '\n' | '\t'))
}

fn validate_annotations(path: &Path, schema: &Schema, context: &str) -> Result<()> {
    let reject = |reason: String| {
        Err(Error::InvalidJsonSchema {
            path: path.to_path_buf(),
            reason,
        })
    };
    // `title` — a short label; empty/whitespace-only or multi-line is degenerate.
    if let Some(title) = &schema.title {
        if title.trim().is_empty() {
            return reject(format!(
                "{context}: `title` must not be empty or whitespace-only (it would render a dead doc summary); drop it, or give it text"
            ));
        }
        if title.contains('\n') {
            return reject(format!(
                "{context}: `title` must be a single line (it is the doc-comment summary); move the prose to `description`"
            ));
        }
        if let Some(control) = first_forbidden_doc_control(title) {
            return reject(format!(
                "{context}: `title` must not contain control character U+{:04X}; remove it or replace it with printable prose",
                control as u32
            ));
        }
    }
    // `description` — the doc body; may span paragraphs, but an empty or
    // whitespace-only string renders a dead doc body (see
    // `specs/json-schema/features/description.md`).
    if let Some(description) = &schema.description {
        if description.trim().is_empty() {
            return reject(format!(
                "{context}: `description` must not be empty or whitespace-only; drop it, or give it text"
            ));
        }
        if let Some(control) = first_forbidden_doc_control(description) {
            return reject(format!(
                "{context}: `description` must not contain control character U+{:04X}; remove it or replace it with printable prose",
                control as u32
            ));
        }
    }
    // `deprecated` — the spec's own MUST: boolean. `false` is accepted and inert.
    if let Some(value) = schema.extra.get("deprecated")
        && !value.is_boolean()
    {
        return reject(format!(
            "{context}: `deprecated` must be a boolean, got {value}"
        ));
    }
    // `$comment` — the spec's own MUST: string (any content, incl. empty).
    if let Some(value) = schema.extra.get("$comment")
        && !value.is_string()
    {
        return reject(format!(
            "{context}: `$comment` must be a string, got {value}"
        ));
    }
    // `examples` — accepted and ignored (inert); its array-MUST is not enforced
    // while dropped (see specs/json-schema/features/examples.md). No check.
    Ok(())
}

/// Load-time validation of the array-constraint keywords (`minItems`,
/// `maxItems`, `uniqueItems`, `contains`, `minContains`, `maxContains`). See
/// `specs/json-schema/features/{minItems,maxItems,uniqueItems,contains,minContains,maxContains}.md`
/// for the authoritative rules. The keywords remain in the schema `extra` map
/// for the backends; this only rejects statically unsatisfiable / unsupported
/// (deferred) forms with fix-its.
fn validate_array_constraints(path: &Path, schema: &Schema, context: &str) -> Result<()> {
    const KEYWORDS: [&str; 6] = [
        "minItems",
        "maxItems",
        "uniqueItems",
        "contains",
        "minContains",
        "maxContains",
    ];
    if KEYWORDS.iter().all(|key| !schema.extra.contains_key(*key)) {
        return Ok(());
    }

    let reject = |reason: String| {
        Err(Error::InvalidJsonSchema {
            path: path.to_path_buf(),
            reason,
        })
    };

    // P7.1: array-constraint keywords are statically meaningless on a non-array
    // type (the string-length analog is `maxLength`, the object member-count
    // analog is `maxProperties`).
    if schema.ty.as_ref().and_then(Value::as_str) != Some("array") {
        return reject(format!(
            "{context}: array-constraint keywords (`minItems`/`maxItems`/`uniqueItems`/`contains`/`minContains`/`maxContains`) require `type: array`"
        ));
    }

    // Each count bound must be a non-negative safe integer. The shared cap is
    // what lets every target represent and compare the count exactly.
    const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;
    let bound = |key: &str| -> Result<Option<u64>> {
        match schema.extra.get(key) {
            None => Ok(None),
            Some(Value::Number(number)) => match number.as_f64() {
                Some(value)
                    if value.is_finite()
                        && value >= 0.0
                        && value <= MAX_SAFE_INTEGER
                        && value.fract() == 0.0 =>
                {
                    Ok(Some(value as u64))
                }
                _ => Err(Error::InvalidJsonSchema {
                    path: path.to_path_buf(),
                    reason: format!(
                        "{context}: `{key}` must be a non-negative integer no greater than 9007199254740991"
                    ),
                }),
            },
            Some(_) => Err(Error::InvalidJsonSchema {
                path: path.to_path_buf(),
                reason: format!(
                    "{context}: `{key}` must be a non-negative integer no greater than 9007199254740991"
                ),
            }),
        }
    };

    let min_items = bound("minItems")?;
    let max_items = bound("maxItems")?;
    // `minItems > maxItems` is unsatisfiable; `minItems == maxItems` pins an
    // exact size (accepted — a fixed-size array).
    if let (Some(min), Some(max)) = (min_items, max_items)
        && min > max
    {
        return reject(format!(
            "{context}: `minItems` ({min}) exceeds `maxItems` ({max}); the bounds describe an empty range (no array can satisfy them)"
        ));
    }

    // The element's *effective* kind: a nullable element is authored as the
    // `oneOf: [T, {"type": "null"}]` wrapper, which declares no `type` of its
    // own, so reading `items.type` classified every nullable element as
    // composite. Per decision D2 a nullable scalar element is accepted — a
    // `null` element simply never matches a scalar `contains` matcher, and two
    // `null`s are a duplicate for `uniqueItems`.
    let element = schema
        .items
        .as_deref()
        .map(|item| nullable_non_null_schema(item).unwrap_or(item));
    let items_kind = scalar_type(element.and_then(|item| item.ty.as_ref().and_then(Value::as_str)));
    let items_is_scalar = items_kind.is_some();

    // `uniqueItems` must be a boolean; `true` over a composite element type is
    // deferred (composite deep-equality is correct in principle, just costly).
    match schema.extra.get("uniqueItems") {
        None => {}
        Some(Value::Bool(unique)) => {
            if *unique && !items_is_scalar {
                return reject(format!(
                    "{context}: `uniqueItems: true` over a composite element type is not yet supported; deep structural equality is deferred (scalar `items` only)"
                ));
            }
        }
        Some(_) => {
            return reject(format!("{context}: `uniqueItems` must be a boolean"));
        }
    }

    // `contains` (with `minContains`/`maxContains`).
    let has_contains = schema.extra.contains_key("contains");
    let min_contains = bound("minContains")?;
    let max_contains = bound("maxContains")?;

    if !has_contains && (min_contains.is_some() || max_contains.is_some()) {
        return reject(format!(
            "{context}: `minContains`/`maxContains` require a sibling `contains` matcher (add a `contains` schema or remove them)"
        ));
    }

    if has_contains {
        let contains_value = &schema.extra["contains"];
        // Shapeless matcher (`{}` / `true` / `false`) — no element shape, so no
        // matcher. `{}`/`true` degenerate to "non-empty" (use `minItems: 1`);
        // `false` matches nothing.
        let matcher: Schema = match contains_value {
            Value::Object(_) => {
                serde_json::from_value(contains_value.clone()).map_err(|error| {
                    Error::InvalidJsonSchema {
                        path: path.to_path_buf(),
                        reason: format!("{context}: `contains` is not a valid schema: {error}"),
                    }
                })?
            }
            _ => {
                return reject(format!(
                    "{context}: `contains` must be a schema object with a scalar matcher (a bare `{{}}`/`true`/`false` is not a matcher — use `minItems`)"
                ));
            }
        };
        let matcher_context = format!("{context}.contains");
        validate_schema_common(path, &matcher, &matcher_context)?;
        validate_type_form(path, &matcher, &matcher_context)?;

        let matcher_ty = matcher.ty.as_ref().and_then(Value::as_str);
        let matcher_const_kind = matcher.extra.get("const").and_then(scalar_value_kind);
        let matcher_const_is_composite = matcher
            .extra
            .get("const")
            .is_some_and(|value| scalar_value_kind(value).is_none());
        let matcher_enum_kind = matcher
            .extra
            .get("enum")
            .and_then(Value::as_array)
            .and_then(|values| values.first())
            .and_then(scalar_value_kind);
        let matcher_enum_is_composite = matcher
            .extra
            .get("enum")
            .and_then(Value::as_array)
            .is_some_and(|values| {
                values
                    .iter()
                    .any(|value| scalar_value_kind(value).is_none())
            });

        // Composite matcher — an object/array-typed matcher, a `$ref`, or a
        // composite `const`/`enum` value — is deferred.
        if matches!(matcher_ty, Some("object" | "array"))
            || matcher.reference.is_some()
            || matcher.properties.is_some()
            || matcher.required.is_some()
            || matcher.additional_properties.is_some()
            || matcher.items.is_some()
            || matcher.one_of.is_some()
            || matcher_const_is_composite
            || matcher_enum_is_composite
        {
            return reject(format!(
                "{matcher_context}: a composite `contains` matcher is not yet supported; deep matching is deferred (scalar matcher only)"
            ));
        }
        if matcher.extra.contains_key("contentEncoding") {
            return reject(format!(
                "{matcher_context}: `contentEncoding` is not supported in a scalar matcher; match the encoded wire string with the supported string predicates instead"
            ));
        }
        // A materializing `format` turns the value into a native date/time. A
        // matcher is a *predicate*, not a slot, so there is nothing to
        // materialize into: no target emits the comparison, and the Go loop
        // scaffold it does emit has no body at all. Reject rather than no-op.
        if let Some(Value::String(format)) = matcher.extra.get("format")
            && crate::json_schema::format::TEMPORAL_FORMATS.contains(&format.as_str())
        {
            return reject(format!(
                "{matcher_context}: `format: {format}` is not supported in a `contains` matcher — it materializes a native date/time value, which a matcher has no slot for; match the wire string with `pattern`, `const` or `enum` instead, or move the assertion onto `items`"
            ));
        }
        if matcher.ty.is_some() {
            validate_type_presence(path, &matcher, &matcher_context)?;
        }

        // Composite element type — `contains` over a composite `items` is
        // deferred, exactly as `uniqueItems` defers composite elements.
        if !items_is_scalar {
            return reject(format!(
                "{context}: `contains` over a composite element type is not yet supported; deep matching is deferred (scalar `items` only)"
            ));
        }

        // Validate the matcher as a scalar schema, including constraints whose
        // invalid form would otherwise be hidden because `contains` is stored
        // in `extra` rather than the ordinary child fields. A matcher may omit
        // `type` when its assertion implies one (`minimum`, `minLength`, const,
        // or enum), so give the validation copy the effective scalar kind while
        // leaving the authored matcher unchanged for generator lowering.
        let mut validated_matcher = matcher.clone();
        if validated_matcher.ty.is_none() {
            let inferred_kind = if matcher.extra.contains_key("minimum")
                || matcher.extra.contains_key("maximum")
                || matcher.extra.contains_key("exclusiveMinimum")
                || matcher.extra.contains_key("exclusiveMaximum")
                || matcher.extra.contains_key("multipleOf")
            {
                match items_kind {
                    Some("integer") => Some("integer"),
                    _ => Some("number"),
                }
            } else if matcher.extra.contains_key("minLength")
                || matcher.extra.contains_key("maxLength")
                || matcher.extra.contains_key("pattern")
                || matcher.extra.contains_key("format")
            {
                Some("string")
            } else if matcher.extra.contains_key("const") || matcher.extra.contains_key("enum") {
                // A typeless literal matcher is evaluated in the element
                // domain. Inferring from the first enum member made
                // `items: number` + `enum: [2, 1.5]` spuriously become an
                // integer schema and reject its own fractional member.
                items_kind
            } else {
                None
            };
            if let Some(kind) = inferred_kind {
                validated_matcher.ty = Some(Value::String(kind.to_string()));
            }
        }
        validate_numeric_constraints(path, &validated_matcher, &matcher_context)?;
        validate_string_constraints(path, &validated_matcher, &matcher_context)?;
        validate_format(path, &validated_matcher, &matcher_context)?;
        validate_array_constraints(path, &validated_matcher, &matcher_context)?;
        validate_object_constraints(path, &validated_matcher, &matcher_context)?;
        validate_const_enum(path, &validated_matcher, &matcher_context)?;

        // The matcher must carry at least one recognized scalar assertion — a
        // scalar `type`, a `const`/`enum`, or a scalar constraint.
        let matcher_has_assertion = scalar_type(matcher_ty).is_some()
            || matcher_const_kind.is_some()
            || matcher_enum_kind.is_some()
            || matcher.extra.contains_key("minimum")
            || matcher.extra.contains_key("maximum")
            || matcher.extra.contains_key("exclusiveMinimum")
            || matcher.extra.contains_key("exclusiveMaximum")
            || matcher.extra.contains_key("multipleOf")
            || matcher.extra.contains_key("minLength")
            || matcher.extra.contains_key("maxLength");
        if !matcher_has_assertion {
            return reject(format!(
                "{context}: `contains` must be a schema object with a scalar matcher (a bare `{{}}`/`true`/`false` is not a matcher — use `minItems`)"
            ));
        }

        // The matcher kind must be compatible with the (scalar) element kind, or
        // no element could ever match (statically unsatisfiable).
        let matcher_kind = scalar_type(matcher_ty)
            .or(matcher_const_kind)
            .or(matcher_enum_kind)
            .or_else(|| {
                if matcher.extra.contains_key("minimum")
                    || matcher.extra.contains_key("maximum")
                    || matcher.extra.contains_key("exclusiveMinimum")
                    || matcher.extra.contains_key("exclusiveMaximum")
                    || matcher.extra.contains_key("multipleOf")
                {
                    Some("number")
                } else if matcher.extra.contains_key("minLength")
                    || matcher.extra.contains_key("maxLength")
                {
                    Some("string")
                } else {
                    None
                }
            });
        if let (Some(element), Some(matcher_kind)) = (items_kind, matcher_kind)
            && !scalar_kinds_overlap(element, matcher_kind)
        {
            return reject(format!(
                "{context}: the `contains` matcher type (`{matcher_kind}`) is incompatible with the element type (`{element}`); no element can ever match"
            ));
        }
    }

    // Match-count satisfiability. The `contains` default is `minContains: 1`.
    if let (Some(min), Some(max)) = (min_contains, max_contains)
        && min > max
    {
        return reject(format!(
            "{context}: `minContains` ({min}) exceeds `maxContains` ({max}); the bounds describe an empty range (no match count can satisfy them)"
        ));
    }
    if let Some(max) = max_contains {
        let effective_min = min_contains.unwrap_or(1);
        if effective_min > max {
            return reject(format!(
                "{context}: `maxContains` ({max}) is below the effective `minContains` ({effective_min}); the bounds describe an empty range (set `minContains: 0` to allow zero matches)"
            ));
        }
    }
    // `minContains: 0` alone (no `maxContains`) makes `contains` always pass with
    // no ceiling, so the whole block asserts nothing — reject as vacuous.
    if min_contains == Some(0) && max_contains.is_none() {
        return reject(format!(
            "{context}: `minContains: 0` without a `maxContains` makes `contains` assert nothing; add a `maxContains` or remove the `contains` block"
        ));
    }

    Ok(())
}

/// Load-time validation of the object-constraint keywords (`minProperties`,
/// `maxProperties`, `propertyNames`, `dependentRequired`). See
/// `specs/json-schema/features/{minProperties,maxProperties,propertyNames,dependentRequired}.md`
/// for the authoritative rules. The keywords remain in the schema `extra` map
/// (or the typed fields) for the backends; this only rejects statically
/// unsatisfiable / unsupported (deferred) forms with fix-its.
fn validate_object_constraints(path: &Path, schema: &Schema, context: &str) -> Result<()> {
    const KEYWORDS: [&str; 4] = [
        "minProperties",
        "maxProperties",
        "propertyNames",
        "dependentRequired",
    ];
    if KEYWORDS.iter().all(|key| !schema.extra.contains_key(*key)) {
        return Ok(());
    }

    let reject = |reason: String| {
        Err(Error::InvalidJsonSchema {
            path: path.to_path_buf(),
            reason,
        })
    };

    // P7.1: object-constraint keywords are statically meaningless on a
    // non-object type (the string-length analog is `maxLength`, the array-length
    // analog is `maxItems`).
    if schema.ty.as_ref().and_then(Value::as_str) != Some("object") {
        return reject(format!(
            "{context}: object-constraint keywords (`minProperties`/`maxProperties`/`propertyNames`/`dependentRequired`) require `type: object`"
        ));
    }

    let declared: Vec<&String> = schema
        .properties
        .as_ref()
        .map(|properties| properties.keys().collect())
        .unwrap_or_default();
    let has_properties = !declared.is_empty();
    let closed = schema.additional_properties.as_ref() == Some(&Value::Bool(false));
    let is_map = matches!(&schema.additional_properties, Some(value) if value.is_object())
        || schema.additional_properties.as_ref() == Some(&Value::Bool(true));
    let required: Vec<String> = schema
        .required
        .as_ref()
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    // Each count bound must be a non-negative safe integer. The shared cap is
    // what lets every target represent and compare the count exactly.
    const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;
    let bound = |key: &str| -> Result<Option<u64>> {
        match schema.extra.get(key) {
            None => Ok(None),
            Some(Value::Number(number)) => match number.as_f64() {
                Some(value)
                    if value.is_finite()
                        && value >= 0.0
                        && value <= MAX_SAFE_INTEGER
                        && value.fract() == 0.0 =>
                {
                    Ok(Some(value as u64))
                }
                _ => Err(Error::InvalidJsonSchema {
                    path: path.to_path_buf(),
                    reason: format!(
                        "{context}: `{key}` must be a non-negative integer no greater than 9007199254740991"
                    ),
                }),
            },
            Some(_) => Err(Error::InvalidJsonSchema {
                path: path.to_path_buf(),
                reason: format!(
                    "{context}: `{key}` must be a non-negative integer no greater than 9007199254740991"
                ),
            }),
        }
    };

    let min_properties = bound("minProperties")?;
    let max_properties = bound("maxProperties")?;

    // `minProperties > maxProperties` is unsatisfiable; equal pins an exact size.
    if let (Some(min), Some(max)) = (min_properties, max_properties)
        && min > max
    {
        return reject(format!(
            "{context}: `minProperties` ({min}) exceeds `maxProperties` ({max}); the bounds describe an empty range (no object can satisfy them)"
        ));
    }
    // A closed object caps the member count at the declared count; a
    // `minProperties` above that is unsatisfiable.
    if let Some(min) = min_properties
        && closed
        && !is_map
        && (declared.len() as u64) < min
    {
        return reject(format!(
            "{context}: `minProperties` ({min}) exceeds the {} declared propert{} of this closed object (no extras are allowed, so it can never be satisfied)",
            declared.len(),
            if declared.len() == 1 { "y" } else { "ies" }
        ));
    }
    // `maxProperties` below the count of required members is unsatisfiable.
    if let Some(max) = max_properties
        && (required.len() as u64) > max
    {
        return reject(format!(
            "{context}: `maxProperties` ({max}) is below the {} required member(s); the object can never satisfy the cap",
            required.len()
        ));
    }

    let mut property_names_capacity = None;

    // `propertyNames` — partial: map-shaped objects only (an object with
    // `additionalProperties` and NO `properties`).
    if let Some(property_names) = schema.extra.get("propertyNames") {
        if has_properties {
            return reject(format!(
                "{context}: `propertyNames` is only supported on a map-shaped object (`additionalProperties` with no `properties`); alongside `properties` it is ambiguous and deferred — encode the key shape on the map form instead"
            ));
        }
        if !is_map {
            return reject(format!(
                "{context}: `propertyNames` requires a map host (`additionalProperties` with a value schema or `true`)"
            ));
        }
        let subschema: Schema = match property_names {
            Value::Object(_) => {
                serde_json::from_value(property_names.clone()).map_err(|error| {
                    Error::InvalidJsonSchema {
                        path: path.to_path_buf(),
                        reason: format!(
                            "{context}: `propertyNames` is not a valid schema: {error}"
                        ),
                    }
                })?
            }
            _ => {
                return reject(format!(
                    "{context}: `propertyNames` must be a string schema constraining the keys (a bare `{{}}`/`true` asserts nothing — property names are always strings)"
                ));
            }
        };
        // Keys are always strings; a non-string subschema can never match.
        if subschema.ty.as_ref().and_then(Value::as_str) != Some("string") {
            return reject(format!(
                "{context}: `propertyNames` must be `type: string` (property names are always strings, so any other type can never match)"
            ));
        }
        const ASSERTIONS: [&str; 5] = ["minLength", "maxLength", "pattern", "enum", "format"];
        // The allowlist has to cover the *typed* fields too. Walking only
        // `extra` let `properties`, `required`, `items`, `oneOf`,
        // `additionalProperties` and `$ref` through silently — and the `$ref`
        // case then surfaced as "`allOf` branches declare disjoint types",
        // because the reference was folded into the `type: string` sibling
        // before anything looked at it.
        let structural = [
            subschema.reference.as_ref().map(|_| "$ref"),
            subschema.id.as_ref().map(|_| "$id"),
            subschema.properties.as_ref().map(|_| "properties"),
            subschema.required.as_ref().map(|_| "required"),
            subschema
                .additional_properties
                .as_ref()
                .map(|_| "additionalProperties"),
            subschema.items.as_ref().map(|_| "items"),
            subschema.one_of.as_ref().map(|_| "oneOf"),
        ];
        // `title`/`description` are pure annotations and assert nothing about a
        // key, so they are accepted-and-ignored here as everywhere else.
        for keyword in structural
            .into_iter()
            .flatten()
            .chain(subschema.extra.keys().map(String::as_str))
        {
            if !ASSERTIONS.contains(&keyword) {
                return reject(property_names_unsupported_keyword_reason(context, keyword));
            }
        }
        if !ASSERTIONS
            .iter()
            .any(|keyword| subschema.extra.contains_key(*keyword))
        {
            return reject(format!(
                "{context}: `propertyNames` asserts nothing (property names are already strings); add `minLength`, `maxLength`, `pattern`, `enum`, or an asserted `format`, or drop the keyword"
            ));
        }
        if let Some(value) = subschema.extra.get("enum") {
            let Some(values) = value.as_array() else {
                return reject(format!(
                    "{context}.propertyNames: `enum` must be an array of strings"
                ));
            };
            if values.is_empty() {
                return reject(format!("{context}.propertyNames: `enum` must not be empty"));
            }
            let mut seen = BTreeSet::new();
            for value in values {
                let Some(value) = value.as_str() else {
                    return reject(format!(
                        "{context}.propertyNames: `enum` must contain only strings"
                    ));
                };
                if !seen.insert(value) {
                    return reject(format!(
                        "{context}.propertyNames: `enum` lists {value:?} more than once"
                    ));
                }
            }
            property_names_capacity = Some(values.len() as u64);
        }
        // A materializing `format` cannot assert a *key*: the key is a map key in
        // every target, so there is nothing to materialize into. Go emitted the
        // per-key loop scaffold with no body (`declared and not used`), Python
        // emitted an empty `for` body (`IndentationError` at import), and
        // TypeScript and Java emitted no key check at all.
        if let Some(Value::String(format)) = subschema.extra.get("format")
            && crate::json_schema::format::TEMPORAL_FORMATS.contains(&format.as_str())
        {
            return reject(format!(
                "{context}.propertyNames: `format: {format}` is not supported — it materializes a native date/time value, and a property name is always a plain string key; assert the key shape with `pattern`, `minLength`/`maxLength` or `enum` instead"
            ));
        }
        // Reuse the ordinary string predicates over the key subschema. Pattern
        // was normalized during the recursive normalize pass above.
        validate_string_constraints(path, &subschema, &format!("{context}.propertyNames"))?;
        validate_format(path, &subschema, &format!("{context}.propertyNames"))?;

        // `maxLength: 0` leaves exactly one *candidate* key: the empty string.
        // Every sibling assertion still filters that candidate. In particular,
        // `pattern: ^a$` and every asserted format exclude it, so the finite
        // language has capacity zero rather than one. Together with an enum,
        // use the tighter enumerable cap.
        if subschema.extra.get("maxLength").and_then(Value::as_f64) == Some(0.0) {
            let empty_admitted = subschema
                .extra
                .get("minLength")
                .and_then(Value::as_u64)
                .is_none_or(|min| min == 0)
                && subschema
                    .extra
                    .get("enum")
                    .and_then(Value::as_array)
                    .is_none_or(|values| values.iter().any(|value| value.as_str() == Some("")))
                && subschema
                    .extra
                    .get("pattern")
                    .and_then(Value::as_str)
                    .is_none_or(|pattern| {
                        regex::Regex::new(pattern).is_ok_and(|matcher| matcher.is_match(""))
                    })
                && subschema
                    .extra
                    .get("format")
                    .and_then(Value::as_str)
                    .is_none_or(|format| crate::json_schema::format::is_valid(format, ""));
            let zero_length_capacity = u64::from(empty_admitted);
            property_names_capacity = Some(
                property_names_capacity
                    .unwrap_or(zero_length_capacity)
                    .min(zero_length_capacity),
            );
        }
    }

    if let (Some(min), Some(capacity)) = (min_properties, property_names_capacity)
        && min > capacity
    {
        return reject(format!(
            "{context}: `minProperties` ({min}) exceeds the finite `propertyNames` key-space capacity ({capacity}); no object can satisfy the floor"
        ));
    }

    // `dependentRequired` — map of trigger → dependents that must also be present.
    let mut dependency_graph = BTreeMap::<String, Vec<String>>::new();
    if let Some(dependent_required) = schema.extra.get("dependentRequired") {
        let Value::Object(map) = dependent_required else {
            return reject(format!(
                "{context}: `dependentRequired` must be an object mapping a property to the properties required alongside it"
            ));
        };
        for (trigger, deps) in map {
            let Value::Array(dep_values) = deps else {
                return reject(format!(
                    "{context}: `dependentRequired.{trigger}` must be an array of property-name strings"
                ));
            };
            let mut seen = BTreeSet::new();
            let mut dep_names = Vec::new();
            for dep in dep_values {
                let Some(dep) = dep.as_str() else {
                    return reject(format!(
                        "{context}: `dependentRequired.{trigger}` must contain only property-name strings"
                    ));
                };
                if !seen.insert(dep.to_string()) {
                    return reject(format!(
                        "{context}: `dependentRequired.{trigger}` lists `{dep}` more than once; entries must be unique"
                    ));
                }
                dep_names.push(dep.to_string());
            }
            // Trigger must be a declared property (presence check on an
            // undeclared member is undecidable, P7.1).
            if !declared.iter().any(|name| name.as_str() == trigger) {
                return reject(format!(
                    "{context}: `dependentRequired` trigger `{trigger}` is not declared in `properties`"
                ));
            }
            // Trigger in `required` → always present, so its dependents are
            // unconditionally required; move them to `required`.
            if required.iter().any(|name| name == trigger) {
                return reject(format!(
                    "{context}: `dependentRequired` trigger `{trigger}` is also in `required`; its dependents are then unconditionally required — move them to `required`"
                ));
            }
            for dep in &dep_names {
                if !declared.iter().any(|name| name.as_str() == dep) {
                    return reject(format!(
                        "{context}: `dependentRequired.{trigger}` dependent `{dep}` is not declared in `properties`"
                    ));
                }
                if required.iter().any(|name| name == dep) {
                    return reject(format!(
                        "{context}: `dependentRequired.{trigger}` dependent `{dep}` is already in `required` (the dependency is vacuous); remove it from `dependentRequired`"
                    ));
                }
            }
            dependency_graph.insert(trigger.clone(), dep_names);
        }
    }

    // A present trigger forces its entire transitive dependency closure *in
    // addition to* every always-present `required` key. Compare the union:
    // checking either set alone misses e.g. required [d] plus a -> b -> c under
    // maxProperties 3.
    if let Some(max) = max_properties {
        for trigger in dependency_graph.keys() {
            let mut closure = BTreeSet::from([trigger.clone()]);
            let mut pending = vec![trigger.clone()];
            while let Some(member) = pending.pop() {
                if let Some(dependents) = dependency_graph.get(&member) {
                    for dependent in dependents {
                        if closure.insert(dependent.clone()) {
                            pending.push(dependent.clone());
                        }
                    }
                }
            }
            let mut forced = closure;
            forced.extend(required.iter().cloned());
            if forced.len() as u64 > max {
                return reject(format!(
                    "{context}: `maxProperties` ({max}) is below the {}-member closure forced by `dependentRequired` trigger `{trigger}` together with always-required members ({}); the object can never satisfy the cap",
                    forced.len(),
                    forced.into_iter().collect::<Vec<_>>().join(", ")
                ));
            }
        }
    }

    Ok(())
}

/// Load-time validation of the `const` and `enum` keywords (the closed
/// value-set primitives). See `specs/json-schema/features/const.md` and
/// `specs/json-schema/features/enum.md` for the authoritative rules. Both keep their
/// values in the schema `extra` map for the backends; this rejects statically
/// unsatisfiable / unsupported / degenerate forms with fix-its. Target-specific
/// value-to-identifier checks belong to the emitted-name manifest, not here.
fn validate_const_enum(path: &Path, schema: &Schema, context: &str) -> Result<()> {
    let has_const = schema.extra.contains_key("const");
    let has_enum = schema.extra.contains_key("enum");
    if !has_const && !has_enum {
        return Ok(());
    }

    let reject = |reason: String| {
        Err(Error::InvalidJsonSchema {
            path: path.to_path_buf(),
            reason,
        })
    };

    // `const` and `enum` are mutually exclusive (a `const` is a single-value
    // `enum`; pick one spelling).
    if has_const && has_enum {
        return reject(format!(
            "{context}: `const` and `enum` are mutually exclusive (a `const` is a single-value `enum`); use one spelling"
        ));
    }

    let ty = schema.ty.as_ref().and_then(Value::as_str);
    let declared_scalar = scalar_type(ty);

    // Validate one scalar member value against the declared type + identifier
    // encoding. `source` names the keyword for diagnostics.
    let check_member = |value: &Value, source: &str| -> Result<()> {
        // `null` member/value → reject (degenerate; use the nullability pattern).
        if value.is_null() {
            return reject(format!(
                "{context}: `{source}: null` is not supported; a field that is always null carries no information (use the nullability pattern for a nullable field, or omit it)"
            ));
        }
        // Composite (object/array) member/value → deferred.
        let Some(value_kind) = scalar_value_kind(value) else {
            return reject(format!(
                "{context}: a composite (object/array) `{source}` value is not yet supported; deep structural equality is deferred (scalar values only)"
            ));
        };
        // Type compatibility (P7.1): the value must be assignable to the
        // declared type. A non-scalar declared type can never hold a scalar.
        match declared_scalar {
            Some(declared) if !scalar_value_assignable(declared, value_kind) => {
                return reject(format!(
                    "{context}: `{source}` value {value} (of kind `{value_kind}`) is incompatible with `type: {}`",
                    ty.unwrap_or("")
                ));
            }
            None if ty.is_some() => {
                return reject(format!(
                    "{context}: `{source}` value {value} (of kind `{value_kind}`) is incompatible with `type: {}`",
                    ty.unwrap_or("")
                ));
            }
            _ => {}
        }
        if declared_scalar == Some("integer")
            && let Value::Number(number) = value
            && number
                .as_f64()
                .is_some_and(|number| number.abs() > 9_007_199_254_740_991.0)
        {
            return reject(format!(
                "{context}: `{source}` integer value {value} exceeds the portable ±9007199254740991 integer cap"
            ));
        }
        // String values are restricted to ASCII without whitespace (keeps the
        // identifier front-end to the Stage 1 word-splitter).
        if let Value::String(text) = value {
            if !text.is_ascii() {
                return reject(format!(
                    "{context}: `{source}` string value {text:?} must be ASCII (non-ASCII values are not supported)"
                ));
            }
            if text.chars().any(|c| c.is_whitespace()) {
                return reject(format!(
                    "{context}: `{source}` string value {text:?} must not contain whitespace"
                ));
            }
        }
        Ok(())
    };

    if has_const {
        // `const` and `default` are mutually exclusive (redundant or
        // contradictory — a const already fixes the value).
        if schema.extra.contains_key("default") {
            return reject(format!(
                "{context}: `const` and `default` are mutually exclusive; a `const` already fixes the value — drop the `default`"
            ));
        }
        let value = &schema.extra["const"];
        check_member(value, "const")?;
    }

    if has_enum {
        let Some(members) = schema.extra["enum"].as_array() else {
            return reject(format!("{context}: `enum` must be an array of values"));
        };
        // An empty `enum` is statically unsatisfiable.
        if members.is_empty() {
            return reject(format!(
                "{context}: `enum` must not be empty (an empty value set can never be satisfied)"
            ));
        }
        // Every member: scalar, type-compatible, ASCII/no-whitespace, encodable.
        for value in members {
            check_member(value, "enum")?;
        }
        // Duplicate members (wire-distinct but redundant) → reject. Compared by
        // *value*, not by JSON representation: `serde_json::Number`'s `PartialEq`
        // separates `PosInt(1)` from `Float(1.0)` and `0` from `-0.0`, so
        // `enum: [1, 1.0]` used to load and Go emitted two `switch` cases for
        // one value (a compile error). P1 makes `5`, `5.0` and `5e0` one number.
        for (index, value) in members.iter().enumerate() {
            if members[..index]
                .iter()
                .any(|previous| json_values_equal(previous, value))
            {
                return reject(format!(
                    "{context}: `enum` lists {value} more than once; members must be unique"
                ));
            }
        }
        // Value-to-identifier legality and collisions are target-specific P15
        // concerns: only Go and Java synthesize constants. They are checked on
        // the resolved emitted model by `build_name_manifest`, alongside every
        // other identifier in the actual target scope.
        // A `default` alongside `enum` must itself be a member of the set.
        if let Some(default) = schema.extra.get("default")
            && !members
                .iter()
                .any(|member| json_values_equal(member, default))
        {
            return reject(format!(
                "{context}: the `default` value {default} is not a member of the `enum` set"
            ));
        }
    }

    Ok(())
}

/// Value equality for JSON scalars, as **P1** defines it: `5`, `5.0` and `5e0`
/// are one number, and `0` and `-0.0` are one number.
///
/// `serde_json::Number`'s own `PartialEq` compares the *representation*, so it
/// separates all of those — which is how `enum: [1, 1.0]` used to pass the
/// uniqueness check. Integers are still compared exactly, so two distinct values
/// beyond 2^53 do not fold together.
fn json_values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(left), Value::Number(right)) => {
            if let (Some(left), Some(right)) = (left.as_i64(), right.as_i64()) {
                return left == right;
            }
            if let (Some(left), Some(right)) = (left.as_u64(), right.as_u64()) {
                return left == right;
            }
            match (left.as_f64(), right.as_f64()) {
                (Some(left), Some(right)) => left == right,
                _ => a == b,
            }
        }
        _ => a == b,
    }
}

/// What an instance of a schema is *forced* to contain, as a boolean expression
/// over named models.
///
/// A plain conjunction is not enough: a `oneOf` is a **choice**, so a union has
/// an instance as soon as *one* branch does. Flattening it away (or, as before,
/// treating every `oneOf` edge as terminating) makes a sum-type recursion whose
/// every branch reenters the cycle look satisfiable.
#[derive(Debug, Clone)]
enum Requirement {
    /// A `$ref` to a named model: an instance exists iff the target's does.
    Target(TypeKey),
    /// Every part must hold. The empty conjunction is trivially satisfiable, and
    /// is what an optional/collection/scalar-only shape reduces to.
    All(Vec<Requirement>),
    /// Some part must hold — a `oneOf`. The empty disjunction is unsatisfiable.
    Any(Vec<Requirement>),
}

/// Builds the [`Requirement`] of one schema. Collection-wrapped, optional and
/// nullable edges contribute nothing (they can terminate the recursion), so the
/// result stays conservative: anything it proves unsatisfiable really is.
/// Ref-resolution errors are ignored here — they surface in
/// [`validate_model_refs`].
fn schema_requirement(
    path: &Path,
    canonical_path: &Path,
    schema: &Schema,
    doc_paths: &BTreeSet<PathBuf>,
) -> Requirement {
    if schema.is_bare_ref() {
        return match &schema.reference {
            Some(reference) => match resolve_ref_key(path, canonical_path, reference, doc_paths) {
                Ok(key) => Requirement::Target(key),
                Err(_) => Requirement::All(Vec::new()),
            },
            None => Requirement::All(Vec::new()),
        };
    }
    if let Some(branches) = &schema.one_of {
        return Requirement::Any(
            branches
                .iter()
                .map(|branch| schema_requirement(path, canonical_path, branch, doc_paths))
                .collect(),
        );
    }
    if schema.ty.as_ref().and_then(Value::as_str) != Some("object") {
        return Requirement::All(Vec::new());
    }
    let Some(properties) = &schema.properties else {
        return Requirement::All(Vec::new());
    };
    let required: BTreeSet<&str> = schema
        .required
        .as_ref()
        .and_then(Value::as_array)
        .map(|values| values.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let mut parts = Vec::new();
    for (name, property) in properties {
        if !required.contains(name.as_str()) {
            continue;
        }
        // `array` / `additionalProperties` / scalar members terminate the chain,
        // and `schema_requirement` reduces each of them to the empty conjunction.
        parts.push(schema_requirement(
            path,
            canonical_path,
            property,
            doc_paths,
        ));
    }
    Requirement::All(parts)
}

/// Whether a requirement holds given what is already known to be instantiable.
/// An unknown target (unresolved or external) is treated as instantiable, which
/// keeps the pass conservative.
fn requirement_holds(
    requirement: &Requirement,
    models: &BTreeMap<TypeKey, JsonModel>,
    instantiable: &BTreeMap<TypeKey, bool>,
) -> bool {
    match requirement {
        Requirement::Target(key) => match instantiable.get(key) {
            Some(known) => *known,
            None => !models.contains_key(key),
        },
        Requirement::All(parts) => parts
            .iter()
            .all(|part| requirement_holds(part, models, instantiable)),
        Requirement::Any(parts) => parts
            .iter()
            .any(|part| requirement_holds(part, models, instantiable)),
    }
}

/// The first target inside a requirement that is not (yet) known instantiable —
/// the next hop of the witness path in the diagnostic.
fn blocking_target<'a>(
    requirement: &'a Requirement,
    models: &BTreeMap<TypeKey, JsonModel>,
    instantiable: &BTreeMap<TypeKey, bool>,
) -> Option<&'a TypeKey> {
    match requirement {
        Requirement::Target(key) => (models.contains_key(key)
            && !instantiable.get(key).copied().unwrap_or(false))
        .then_some(key),
        Requirement::All(parts) | Requirement::Any(parts) => parts
            .iter()
            .find_map(|part| blocking_target(part, models, instantiable)),
    }
}

/// Rejects a recursion no finite value can satisfy — one where following only
/// mandatory, single-valued (required + non-nullable + non-collection) edges can
/// never bottom out. See `specs/json-schema/features/ref.md` (Recursion &
/// satisfiability).
///
/// Computed as a least fixed point rather than a cycle search, because a `oneOf`
/// is a disjunction: a union is instantiable as soon as one branch is, and a
/// model is instantiable once every edge it forces is. Whatever the fixed point
/// never reaches has no finite instance. Conservative: only edges the loader can
/// prove mandatory participate, and an unresolvable or external target counts as
/// instantiable.
fn validate_reference_satisfiability(
    docs: &IndexMap<PathBuf, (PathBuf, Document)>,
    models: &BTreeMap<TypeKey, JsonModel>,
) -> Result<()> {
    let doc_paths: BTreeSet<PathBuf> = docs.keys().cloned().collect();
    let mut requirements: BTreeMap<TypeKey, Requirement> = BTreeMap::new();
    for (key, model) in models {
        let source_path = docs
            .get(&model.canonical_path)
            .map(|(path, _)| path.clone())
            .unwrap_or_else(|| model.canonical_path.clone());
        requirements.insert(
            key.clone(),
            schema_requirement(
                &source_path,
                &model.canonical_path,
                &model.schema,
                &doc_paths,
            ),
        );
    }

    let mut instantiable: BTreeMap<TypeKey, bool> =
        models.keys().map(|key| (key.clone(), false)).collect();
    loop {
        let mut changed = false;
        for (key, requirement) in &requirements {
            if instantiable.get(key).copied().unwrap_or(false) {
                continue;
            }
            if requirement_holds(requirement, models, &instantiable) {
                instantiable.insert(key.clone(), true);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    let Some(start) = models
        .keys()
        .find(|key| !instantiable.get(*key).copied().unwrap_or(false))
    else {
        return Ok(());
    };

    // Witness path: from the offending model, follow one blocking edge at a time
    // until a model repeats. Every hop is mandatory, so the repeat is the cycle
    // the author has to break.
    let mut cycle = vec![start.clone()];
    let mut current = start.clone();
    loop {
        let Some(next) = requirements
            .get(&current)
            .and_then(|requirement| blocking_target(requirement, models, &instantiable))
        else {
            break;
        };
        cycle.push(next.clone());
        if cycle[..cycle.len() - 1].contains(next) {
            break;
        }
        current = next.clone();
    }

    let display = |type_key: &TypeKey| {
        models
            .get(type_key)
            .map(|model| model.full_name.clone())
            .unwrap_or_else(|| match type_key {
                TypeKey::Root(path) => root_type_name(path),
                TypeKey::Def(_, names) => names
                    .last()
                    .cloned()
                    .unwrap_or_else(|| "<definition>".to_string()),
            })
    };
    let path = cycle.iter().map(display).collect::<Vec<_>>().join(" → ");
    let report_path = model_key_path(start)
        .cloned()
        .unwrap_or_else(|| PathBuf::from("<json-schema>"));
    Err(Error::InvalidJsonSchema {
        path: report_path,
        reason: format!(
            "unsatisfiable recursion cycle `{path}`: every path out of it is a required, non-nullable, single-valued `$ref`, so no finite value can satisfy it — break the cycle by making an edge optional, nullable (`oneOf: [{{...}}, {{type: \"null\"}}]`), or wrapping it in an array"
        ),
    })
}

fn validate_model_refs(
    docs: &IndexMap<PathBuf, (PathBuf, Document)>,
    models: &BTreeMap<TypeKey, JsonModel>,
) -> Result<()> {
    for model in models.values() {
        let path = docs
            .get(&model.canonical_path)
            .map(|(path, _)| path.as_path())
            .unwrap_or(model.canonical_path.as_path());
        validate_schema_refs(
            path,
            &model.canonical_path,
            &model.schema,
            &model.full_name,
            docs,
            models,
        )?;
    }
    for (canonical_path, (path, doc)) in docs {
        if let Some(services) = &doc.services {
            for (service_name, service) in services {
                for (operation_name, operation) in &service.operations {
                    for (label, schema) in
                        [("input", &operation.input), ("output", &operation.output)]
                    {
                        if let Some(schema) = schema {
                            validate_schema_refs(
                                path,
                                canonical_path,
                                schema,
                                &format!(
                                    "services.{service_name}.operations.{operation_name}.{label}"
                                ),
                                docs,
                                models,
                            )?;
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// Whether a property schema admits JSON `null`, following a bare `$ref` alias
/// chain when necessary. Presence-sensitive object assertions cannot safely be
/// combined with an optional nullable property: Go, Java, and Python deliberately
/// collapse absent and explicit-null into one in-memory state, while TypeScript
/// retains both (PRINCIPLES P1/P8 and `nullability.md`).
fn schema_accepts_null<'a>(
    path: &Path,
    canonical_path: &'a Path,
    schema: &'a Schema,
    docs: &'a IndexMap<PathBuf, (PathBuf, Document)>,
    models: &'a BTreeMap<TypeKey, JsonModel>,
) -> Result<bool> {
    let mut current_path = canonical_path;
    let mut current = schema;
    let mut seen = BTreeSet::new();
    loop {
        if current
            .one_of
            .as_ref()
            .is_some_and(|branches| branches.iter().any(schema_type_is_null))
        {
            return Ok(true);
        }
        let Some(reference) = current.reference.as_deref() else {
            return Ok(false);
        };
        if !seen.insert((current_path.to_path_buf(), reference.to_string())) {
            return Ok(false);
        }
        let target = resolve_ref(path, current_path, reference, docs, models)?;
        current_path = &target.canonical_path;
        current = &target.schema;
    }
}

fn validate_optional_nullable_presence_interactions(
    path: &Path,
    canonical_path: &Path,
    schema: &Schema,
    context: &str,
    docs: &IndexMap<PathBuf, (PathBuf, Document)>,
    models: &BTreeMap<TypeKey, JsonModel>,
) -> Result<()> {
    let Some(properties) = &schema.properties else {
        return Ok(());
    };
    let required: BTreeSet<&str> = schema
        .required
        .as_ref()
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let optional_nullable = |name: &str| -> Result<bool> {
        let Some(property) = properties.get(name) else {
            return Ok(false);
        };
        Ok(!required.contains(name)
            && schema_accepts_null(path, canonical_path, property, docs, models)?)
    };

    if schema
        .extra
        .get("minProperties")
        .and_then(Value::as_u64)
        .is_some_and(|minimum| minimum > 0)
    {
        for (name, property) in properties {
            if !required.contains(name.as_str())
                && schema_accepts_null(path, canonical_path, property, docs, models)?
            {
                return Err(Error::InvalidJsonSchema {
                    path: path.to_path_buf(),
                    reason: format!(
                        "{context}: `minProperties` cannot be combined with optional nullable property `{name}`; Go, Java, and Python collapse an explicit null to absence, so the outbound wire count would differ — make `{name}` required or non-nullable, or remove `minProperties`"
                    ),
                });
            }
        }
    }

    if let Some(Value::Object(dependencies)) = schema.extra.get("dependentRequired") {
        for (trigger, dependents) in dependencies {
            let Some(dependents) = dependents.as_array() else {
                continue;
            };
            if dependents.is_empty() {
                continue;
            }
            if optional_nullable(trigger)? {
                return Err(Error::InvalidJsonSchema {
                    path: path.to_path_buf(),
                    reason: format!(
                        "{context}: `dependentRequired` trigger `{trigger}` cannot be optional and nullable; Go, Java, and Python collapse explicit null to absence — make it non-nullable or remove this dependency"
                    ),
                });
            }
            for dependent in dependents.iter().filter_map(Value::as_str) {
                if optional_nullable(dependent)? {
                    return Err(Error::InvalidJsonSchema {
                        path: path.to_path_buf(),
                        reason: format!(
                            "{context}: `dependentRequired.{trigger}` dependent `{dependent}` cannot be optional and nullable; Go, Java, and Python collapse explicit null to absence — make it non-nullable or remove this dependency"
                        ),
                    });
                }
            }
        }
    }
    Ok(())
}

fn validate_schema_refs(
    path: &Path,
    canonical_path: &Path,
    schema: &Schema,
    context: &str,
    docs: &IndexMap<PathBuf, (PathBuf, Document)>,
    models: &BTreeMap<TypeKey, JsonModel>,
) -> Result<()> {
    validate_schema_common(path, schema, context)?;
    validate_optional_nullable_presence_interactions(
        path,
        canonical_path,
        schema,
        context,
        docs,
        models,
    )?;
    if let Some(reference) = &schema.reference {
        let _ = resolve_ref_at(path, canonical_path, reference, context, docs, models)?;
        return Ok(());
    }
    if let Some(properties) = &schema.properties {
        for (name, property) in properties {
            validate_schema_refs(
                path,
                canonical_path,
                property,
                &format!("{context}.properties.{name}"),
                docs,
                models,
            )?;
        }
    }
    if let Some(items) = &schema.items {
        validate_schema_refs(
            path,
            canonical_path,
            items,
            &format!("{context}.items"),
            docs,
            models,
        )?;
    }
    if let Some(Value::Object(members)) = &schema.additional_properties {
        let additional: Schema =
            serde_json::from_value(Value::Object(members.clone())).map_err(|error| {
                Error::InvalidJsonSchema {
                    path: path.to_path_buf(),
                    reason: format!(
                        "{context}.additionalProperties is not a valid schema object: {error}"
                    ),
                }
            })?;
        validate_schema_refs(
            path,
            canonical_path,
            &additional,
            &format!("{context}.additionalProperties"),
            docs,
            models,
        )?;
    }
    if let Some(one_of) = &schema.one_of {
        for branch in one_of {
            validate_schema_refs(
                path,
                canonical_path,
                branch,
                &format!("{context}.oneOf"),
                docs,
                models,
            )?;
        }
    }
    Ok(())
}

/// The JSON kind that acts as the outer selector for a `oneOf` branch.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum BranchKind {
    Null,
    Boolean,
    String,
    Integer,
    Number,
    Array,
    Object,
}

impl BranchKind {
    fn label(self) -> &'static str {
        match self {
            BranchKind::Null => "null",
            BranchKind::Boolean => "boolean",
            BranchKind::String => "string",
            BranchKind::Integer => "integer",
            BranchKind::Number => "number",
            BranchKind::Array => "array",
            BranchKind::Object => "object",
        }
    }
}

/// Walks every model (root + `$defs`) and operation input/output, classifying
/// each `oneOf` node and rejecting the forms with no decidable selector. Runs
/// after `validate_model_refs` so `$ref` branches resolve to their target kind.
fn validate_all_unions(
    docs: &IndexMap<PathBuf, (PathBuf, Document)>,
    models: &BTreeMap<TypeKey, JsonModel>,
) -> Result<()> {
    for model in models.values() {
        let path = docs
            .get(&model.canonical_path)
            .map(|(path, _)| path.as_path())
            .unwrap_or(model.canonical_path.as_path());
        validate_schema_unions(
            path,
            &model.canonical_path,
            &model.schema,
            &model.full_name,
            docs,
            models,
        )?;
    }
    for (canonical_path, (path, doc)) in docs {
        if let Some(services) = &doc.services {
            for (service_name, service) in services {
                for (operation_name, operation) in &service.operations {
                    for (label, schema) in
                        [("input", &operation.input), ("output", &operation.output)]
                    {
                        if let Some(schema) = schema {
                            validate_schema_unions(
                                path,
                                canonical_path,
                                schema,
                                &format!(
                                    "services.{service_name}.operations.{operation_name}.{label}"
                                ),
                                docs,
                                models,
                            )?;
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// Recursively validates every `oneOf` node in a schema tree as a sum type.
fn validate_schema_unions(
    path: &Path,
    canonical_path: &Path,
    schema: &Schema,
    context: &str,
    docs: &IndexMap<PathBuf, (PathBuf, Document)>,
    models: &BTreeMap<TypeKey, JsonModel>,
) -> Result<()> {
    if let Some(one_of) = &schema.one_of {
        validate_one_of(path, canonical_path, schema, context, docs, models)?;
        for branch in one_of {
            validate_schema_unions(
                path,
                canonical_path,
                branch,
                &format!("{context}.oneOf"),
                docs,
                models,
            )?;
        }
    }
    if let Some(properties) = &schema.properties {
        for (name, property) in properties {
            validate_schema_unions(
                path,
                canonical_path,
                property,
                &format!("{context}.properties.{name}"),
                docs,
                models,
            )?;
        }
    }
    if let Some(items) = &schema.items {
        validate_schema_unions(
            path,
            canonical_path,
            items,
            &format!("{context}.items"),
            docs,
            models,
        )?;
    }
    if let Some(additional) = &schema.additional_properties
        && additional.is_object()
        && let Ok(additional_schema) = serde_json::from_value::<Schema>(additional.clone())
    {
        validate_schema_unions(
            path,
            canonical_path,
            &additional_schema,
            &format!("{context}.additionalProperties"),
            docs,
            models,
        )?;
    }
    Ok(())
}

/// The effective schema a `oneOf` branch selects on: a `$ref` branch resolves to
/// its target model schema, any other branch is itself.
fn resolve_branch_schema(
    branch: &Schema,
    path: &Path,
    canonical_path: &Path,
    context: &str,
    docs: &IndexMap<PathBuf, (PathBuf, Document)>,
    models: &BTreeMap<TypeKey, JsonModel>,
) -> Result<Schema> {
    if let Some(reference) = &branch.reference {
        let target = resolve_ref_at(path, canonical_path, reference, context, docs, models)?;
        Ok(target.schema.clone())
    } else {
        Ok(branch.clone())
    }
}

/// Classifies a single `oneOf` branch into its JSON kind, rejecting a branch
/// with no classifiable kind (typeless / boolean schema / nested combinator).
fn one_of_branch_kind(
    branch: &Schema,
    resolved: &Schema,
    path: &Path,
    context: &str,
) -> Result<BranchKind> {
    let reject = |reason: String| {
        Err(Error::InvalidJsonSchema {
            path: path.to_path_buf(),
            reason,
        })
    };
    if resolved.one_of.is_some() {
        return reject(format!(
            "{context}: a `oneOf` branch cannot itself be a `oneOf` union (a branch must declare a single recognized `type`)"
        ));
    }
    match resolved.ty.as_ref() {
        Some(Value::String(ty)) => match ty.as_str() {
            "null" => Ok(BranchKind::Null),
            "boolean" => Ok(BranchKind::Boolean),
            "string" => Ok(BranchKind::String),
            "integer" => Ok(BranchKind::Integer),
            "number" => Ok(BranchKind::Number),
            "array" => Ok(BranchKind::Array),
            "object" => Ok(BranchKind::Object),
            other => reject(format!(
                "{context}: a `oneOf` branch has unrecognized `type: {other}`"
            )),
        },
        Some(_) => reject(format!(
            "{context}: a `oneOf` branch must declare a single string `type` (an array `type` has no single selector kind)"
        )),
        None => {
            let hint = if branch.reference.is_some() {
                " (its `$ref` target declares no single `type`)"
            } else {
                ""
            };
            reject(format!(
                "{context}: a `oneOf` branch has no classifiable kind{hint}; every branch must declare a single recognized `type` (or `$ref` a typed definition)"
            ))
        }
    }
}

/// The scalar `const` value of a property (a bare `const`, or a single-member
/// `enum`), used as a discriminator tag value. `None` when the property carries
/// no single fixed scalar value.
fn discriminator_const(property: &Schema) -> Option<Value> {
    if let Some(value) = property.extra.get("const") {
        return scalar_value_kind(value).map(|_| value.clone());
    }
    if let Some(Value::Array(members)) = property.extra.get("enum")
        && members.len() == 1
        && scalar_value_kind(&members[0]).is_some()
    {
        return Some(members[0].clone());
    }
    None
}

/// The set of property names that qualify as a discriminator tag for an object
/// branch: present in the branch's `required` array and carrying a scalar
/// `const`. Maps each qualifying name to its `const` value.
fn branch_discriminator_tags(object: &Schema) -> BTreeMap<String, Value> {
    let required: BTreeSet<String> = object
        .required
        .as_ref()
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let mut tags = BTreeMap::new();
    if let Some(properties) = &object.properties {
        for (name, property) in properties {
            if required.contains(name)
                && let Some(value) = discriminator_const(property)
            {
                tags.insert(name.clone(), value);
            }
        }
    }
    tags
}

/// True when a schema is the free-form object — `type: object` carrying nothing
/// but `additionalProperties: true`, i.e. an open bag of unconstrained members.
fn is_free_form_object(schema: &Schema) -> bool {
    schema.additional_properties.as_ref() == Some(&Value::Bool(true))
        && schema
            .properties
            .as_ref()
            .is_none_or(|properties| properties.is_empty())
}

/// True when a schema is an object written inline — every object in a value
/// position needs a name, whatever it declares. Even the free-form object does:
/// [[additionalProperties]] emits every object as a *named aggregate* holding its
/// members in a catch-all field, so that later adding `properties` to it only
/// adds fields instead of changing the emitted type's kind (P13). Naming it is
/// also what makes the inline form emit identically to the `$defs` + `$ref` form.
/// A `oneOf` branch is the one position where the free-form object stays inline:
/// there it is the union's *object kind*, which TypeScript and Python express
/// structurally inside the value union ([[oneOf]]).
fn is_inline_object_shape(schema: &Schema) -> bool {
    schema.reference.is_none() && schema.ty.as_ref().and_then(Value::as_str) == Some("object")
}

/// Moves every inline object shape — and every inline element union — into a
/// synthesized `$defs` entry, rewriting the position it was written in to a
/// `$ref` at it. Every target has to materialize a *type* for such a shape: Go a
/// struct (plus a defined type to carry a union's marker method), Java a class
/// (to `implement` a union interface), Python a `BaseModel` for Pydantic to
/// select, TypeScript an interface plus the converter that validates its members
/// — so the shape needs a name; and once it has one, a named definition is
/// exactly what every target already emits. Hoisting is therefore the whole
/// feature: downstream the position holds an ordinary `$ref` and its target an
/// ordinary model, so validation, ref resolution, P15, module exports, and
/// emission all apply unchanged, and the inline form emits byte-identical code to
/// the `$defs` + `$ref` form. See
/// `specs/json-schema/features/properties.md` §"Naming an inline object shape"
/// and `specs/json-schema/features/oneOf.md` §"Object branches — naming the
/// inline shape".
///
/// The one object left inline is the **free-form** object as a `oneOf` *branch*:
/// there it is the union's object kind rather than a value position of its own, so
/// TypeScript and Python express it structurally inside the value union
/// (`Record<string, unknown>` / `dict[str, Any]`) and Go/Java wrap it as the
/// union's `<Union>Object` variant ([[oneOf]], [[additionalProperties]]).
///
/// Ordering: after `normalize_document` (so an `allOf` branch is already merged),
/// after per-model validation (so a defect inside a shape is reported at the
/// position the user wrote it), and before models are collected (so a hoisted
/// definition is picked up as one).
fn hoist_inline_object_shapes(
    language: Language,
    docs: &mut IndexMap<PathBuf, (PathBuf, Document)>,
    ref_fold_annotations: &RefFoldAnnotations,
) -> Result<()> {
    for (path, doc) in docs.values_mut() {
        let document_ref_folds = ref_fold_annotations.get(path);
        // Definitions inserted by this pass remain in `doc.defs` on later
        // fixpoint iterations. Keep their authored origins separately so a
        // collision with another synthesized shape does not falsely claim the
        // first name was authored in `$defs`.
        let mut synthesized_origins = BTreeMap::<String, String>::new();
        // The type name the file's root schema derives from its file name, when
        // the file has a root type at all (a Nexus-document envelope and a
        // definitions-only file have none). A synthesized name that coincides
        // with it is a P15 collision, checked where the shape is inserted below.
        let root_model = (root_is_schema_shaped(&doc.root) && !doc.root.is_bare_ref())
            .then(|| root_model_name(path));
        // Fixpoint: a hoisted definition is walked on the next pass, so a union
        // nested in a hoisted branch's property is hoisted too. Each pass
        // replaces at least one inline branch with a `$ref` (and never
        // introduces one), so the walk terminates.
        loop {
            let mut hoisted: Vec<HoistedDef> = Vec::new();
            if let Some(defs) = doc.defs.as_mut() {
                hoist_def_inline_shapes(language, path, defs, &[], &mut hoisted)?;
            }
            if let Some(model_name) = &root_model {
                hoist_model_inline_shapes(
                    language,
                    path,
                    model_name,
                    "root schema",
                    &mut doc.root,
                    &mut hoisted,
                )?;
            }
            if let Some(services) = doc.services.as_mut() {
                for (service_name, service) in services.iter_mut() {
                    for (operation_name, operation) in service.operations.iter_mut() {
                        for (suffix, schema) in [
                            ("Input", operation.input.as_mut()),
                            ("Output", operation.output.as_mut()),
                        ] {
                            // A `$ref` I/O carries no inline schema of its own;
                            // its target is walked as a `$defs` model.
                            let Some(schema) = schema.filter(|schema| schema.reference.is_none())
                            else {
                                continue;
                            };
                            hoist_model_inline_shapes(
                                language,
                                path,
                                &format!("{}{suffix}", operation_name.to_upper_camel_case()),
                                &format!(
                                    "services.{service_name}.operations.{operation_name}.{}",
                                    suffix.to_lowercase()
                                ),
                                schema,
                                &mut hoisted,
                            )?;
                        }
                    }
                }
            }
            if hoisted.is_empty() {
                break;
            }
            let defs = doc.defs.get_or_insert_with(IndexMap::new);
            for HoistedDef {
                name,
                origin,
                schema,
            } in hoisted
            {
                if root_model.as_deref() == Some(name.as_str()) {
                    let remedy = annotated_ref_collision_remedy(
                        document_ref_folds,
                        &[origin.as_str()],
                    )
                    .map(|remedy| {
                        format!(
                            "{remedy}, or rename the file so the root schema derives a different name"
                        )
                    })
                    .unwrap_or_else(|| {
                        format!(
                            "name the inline shape with an `{}` override where it takes one (a `oneOf` branch, an array element, a map member), move it into `$defs` under a name of your own and `$ref` it, or rename the file so the root schema derives a different name",
                            lang_name_keyword(language).unwrap_or("x-<lang>-name"),
                        )
                    });
                    return Err(Error::InvalidJsonSchema {
                        path: path.to_path_buf(),
                        reason: format!(
                            "the name `{name}` synthesized for the inline shape at `{origin}` is the type name the root schema derives from the file name `{}`; the two are different schemas that would emit one type. {remedy} (P15 — the generator never auto-mangles)",
                            root_file_name(path),
                        ),
                    });
                }
                if defs.contains_key(&name) {
                    let previous_origin = synthesized_origins.get(&name);
                    let detail = previous_origin.map_or_else(
                        || format!("is already declared in `$defs`"),
                        |previous_origin| {
                            format!(
                                "was already synthesized for the inline shape at `{previous_origin}`"
                            )
                        },
                    );
                    let mut origins = vec![origin.as_str()];
                    if let Some(previous_origin) = previous_origin {
                        origins.push(previous_origin.as_str());
                    }
                    let remedy = annotated_ref_collision_remedy(document_ref_folds, &origins)
                        .map(|remedy| {
                            format!(
                                "{remedy}, or rename the conflicting `$defs` declaration"
                            )
                        })
                        .unwrap_or_else(|| {
                            format!(
                                "rename either one, name the inline shape with an `{}` override where it takes one (a `oneOf` branch, an array element, a map member), or move it into `$defs` under a distinct name and `$ref` it",
                                lang_name_keyword(language).unwrap_or("x-<lang>-name"),
                            )
                        });
                    return Err(Error::InvalidJsonSchema {
                        path: path.to_path_buf(),
                        reason: format!(
                            "the name `{name}` synthesized for the inline shape at `{origin}` {detail}; {remedy} (P15 — the generator never auto-mangles)",
                        ),
                    });
                }
                synthesized_origins.insert(name.clone(), origin);
                defs.insert(name, schema);
            }
        }
    }
    Ok(())
}

fn annotated_ref_collision_remedy(
    annotations: Option<&BTreeMap<String, Vec<String>>>,
    origins: &[&str],
) -> Option<String> {
    let (origin, keywords) = origins.iter().find_map(|origin| {
        annotations?
            .get(*origin)
            .map(|keywords| (*origin, keywords))
    })?;
    let keywords = keywords
        .iter()
        .map(|keyword| format!("`{keyword}`"))
        .collect::<Vec<_>>()
        .join(", ");
    let noun = if keywords.contains(',') {
        "annotations"
    } else {
        "annotation"
    };
    Some(format!(
        "the `$ref` at `{origin}` was materialized under this position-derived name because it carries the {keywords} {noun}; remove the {noun} from that use site so it remains a reference, or relocate the {noun} to the referenced declaration"
    ))
}

/// One inline shape queued for insertion into `$defs` by
/// [`hoist_inline_object_shapes`]: the name synthesized for it, the authored
/// position it was written in, and the shape itself. The origin travels with the
/// name so a collision diagnostic can say *where* the synthesized name came from
/// — the author never wrote the name itself, so naming only the identifier would
/// leave them hunting for the shape that produced it.
struct HoistedDef {
    /// The synthesized `$defs` key (or the shape's own `x-<lang>-name`).
    name: String,
    /// The authored position, as a keyword breadcrumb — for example
    /// `$defs.User.properties.profile` or `root schema.items`.
    origin: String,
    /// The shape moved out of that position.
    schema: Schema,
}

fn hoist_def_inline_shapes(
    language: Language,
    path: &Path,
    defs: &mut IndexMap<String, Schema>,
    parent_names: &[String],
    hoisted: &mut Vec<HoistedDef>,
) -> Result<()> {
    for (name, schema) in defs.iter_mut() {
        let mut names = parent_names.to_vec();
        names.push(name.clone());
        let context = def_context(&names);
        hoist_model_inline_shapes(
            language,
            path,
            &name.to_upper_camel_case(),
            &context,
            schema,
            hoisted,
        )?;
        if let Some(value) = schema.extra.shift_remove("$defs") {
            let mut nested: IndexMap<String, Schema> =
                serde_json::from_value(value).map_err(|error| Error::InvalidJsonSchema {
                    path: path.to_path_buf(),
                    reason: format!("{context}: `$defs` is not an object of schemas: {error}"),
                })?;
            hoist_def_inline_shapes(language, path, &mut nested, &names, hoisted)?;
            schema.extra.insert(
                "$defs".to_string(),
                serde_json::to_value(nested).map_err(|error| Error::InvalidJsonSchema {
                    path: path.to_path_buf(),
                    reason: format!("{context}: failed to preserve nested `$defs`: {error}"),
                })?,
            );
        }
    }
    Ok(())
}

/// Hoists the inline shapes a model declares that need a name: the object
/// branches of its unions — its own (a named `$defs` union) and each property's
/// (an anonymous union, named `<Model><Property>` — the [[properties]]
/// synthesized-name rule) — the object a property declares directly
/// ([`hoist_property_shape`]), and every shape written inline in a subschema
/// position ([`hoist_subschema_shapes`]).
fn hoist_model_inline_shapes(
    language: Language,
    path: &Path,
    model_name: &str,
    context: &str,
    schema: &mut Schema,
    hoisted: &mut Vec<HoistedDef>,
) -> Result<()> {
    if let Some(branches) = schema.one_of.as_mut() {
        // The model *is* the union, so the union carries its own name and its
        // inline object branches derive `<Model>Object`.
        hoist_union_object_branches(
            language,
            path,
            &format!("{model_name}Object"),
            context,
            branches,
            hoisted,
        )?;
        // An array branch (`<Union>Array`) is a subschema position of its own.
        for branch in branches.iter_mut() {
            hoist_subschema_shapes(
                language,
                path,
                &format!("{model_name}Array"),
                &format!("{context}.oneOf"),
                branch,
                hoisted,
            )?;
        }
    }
    if let Some(properties) = schema.properties.as_mut() {
        for (json_name, property) in properties.iter_mut() {
            let property_name = format!("{model_name}{}", json_name.to_upper_camel_case());
            let property_context = format!("{context}.properties.{json_name}");
            hoist_property_shape(
                language,
                path,
                &property_name,
                &property_context,
                property,
                hoisted,
            )?;
            hoist_subschema_shapes(
                language,
                path,
                &property_name,
                &property_context,
                property,
                hoisted,
            )?;
        }
    }
    // The model's own element positions: a map-shaped model's members, or a
    // struct's typed catch-all.
    hoist_subschema_shapes(language, path, model_name, context, schema, hoisted)?;
    Ok(())
}

/// Names and hoists the inline object shape a **property** declares: the object
/// branches of a property-level union, the object inside a nullability `oneOf`
/// wrapper, or an object written directly on the property.
///
/// Which name the shape takes follows the position: a *sum type* occupies the
/// property's own synthesized name (the emitted union type), so its branches
/// derive `<Model><Property>Object`, while a nullability wrapper emits no type of
/// its own — every target expresses it structurally on the value — so the object
/// inside it takes `<Model><Property>` directly, exactly as a plainly-written one
/// does. Adding or removing nullability therefore never renames the type.
fn hoist_property_shape(
    language: Language,
    path: &Path,
    property_name: &str,
    context: &str,
    property: &mut Schema,
    hoisted: &mut Vec<HoistedDef>,
) -> Result<()> {
    if is_sum_type_union(property) {
        let branches = property.one_of.as_mut().expect("a union has branches");
        hoist_union_object_branches(
            language,
            path,
            &format!("{property_name}Object"),
            context,
            branches,
            hoisted,
        )?;
        for branch in branches {
            hoist_subschema_shapes(
                language,
                path,
                &format!("{property_name}Array"),
                &format!("{context}.oneOf"),
                branch,
                hoisted,
            )?;
        }
        return Ok(());
    }
    if hoist_nullable_object_branch(language, property_name, context, property, hoisted)? {
        return Ok(());
    }
    if !is_inline_object_shape(property) {
        return Ok(());
    }
    let mut shape = std::mem::take(property);
    // The shape's doc text travels with it: it describes the object, which is now
    // a type of its own, and the member falls back to its synthesized doc line —
    // exactly what authoring the shape in `$defs` and `$ref`ing it produces. An
    // `x-<lang>-name`, by contrast, is the [[properties]] Stage 4 escape hatch for
    // the *member* identifier, and the same keyword names a *type* in `$defs`, so
    // it stays behind on the property.
    *property = Schema {
        reference: Some(format!("#/$defs/{property_name}")),
        ..Schema::default()
    };
    for keyword in LANG_NAME_KEYWORDS {
        if let Some(value) = shape.extra.shift_remove(keyword) {
            property.extra.insert(keyword.to_string(), value);
        }
    }
    hoisted.push(HoistedDef {
        name: property_name.to_string(),
        origin: context.to_string(),
        schema: shape,
    });
    Ok(())
}

/// Names and hoists every shape written inline in a **subschema position** that
/// needs a name — a `oneOf` sum type or an object — in an array's `items` (at any
/// depth) or an object's typed `additionalProperties`, the same way
/// [`hoist_union_object_branches`] names an inline object branch and for the same
/// reason: Go and Java need a *type* for the element (a struct, or a sealed
/// interface with its dispatcher), so the shape needs a name, and a named `$defs`
/// model is what every target already emits. The synthesized name is the
/// enclosing name plus the position — `<Enclosing>Item` for `items`,
/// `<Enclosing>Value` for `additionalProperties` — or the shape's own
/// `x-<lang>-name`. See `specs/json-schema/features/oneOf.md` §"Unions in element
/// positions".
///
/// A nested object written in one of these positions is hoisted the same way and
/// takes the same name; the walk stops there rather than descending into its
/// `properties`, because the shape is now a `$defs` model that the next fixpoint
/// pass walks in its own right.
fn hoist_subschema_shapes(
    language: Language,
    path: &Path,
    base_name: &str,
    context: &str,
    schema: &mut Schema,
    hoisted: &mut Vec<HoistedDef>,
) -> Result<()> {
    if let Some(items) = schema.items.as_mut() {
        hoist_subschema_shape(
            language,
            path,
            &format!("{base_name}Item"),
            &format!("{context}.items"),
            items,
            hoisted,
        )?;
    }
    if let Some(Value::Object(members)) = &schema.additional_properties {
        let mut value: Schema =
            serde_json::from_value(Value::Object(members.clone())).map_err(|error| {
                Error::InvalidJsonSchema {
                    path: path.to_path_buf(),
                    reason: format!("{context}.additionalProperties is invalid: {error}"),
                }
            })?;
        hoist_subschema_shape(
            language,
            path,
            &format!("{base_name}Value"),
            &format!("{context}.additionalProperties"),
            &mut value,
            hoisted,
        )?;
        schema.additional_properties =
            Some(
                serde_json::to_value(&value).map_err(|error| Error::InvalidJsonSchema {
                    path: path.to_path_buf(),
                    reason: format!("{context}: failed to preserve additionalProperties: {error}"),
                })?,
            );
    }
    Ok(())
}

/// Hoists one subschema slot: the shape occupying the slot when it needs a name
/// — a sum type, or an object — otherwise the object inside a nullability
/// wrapper, otherwise recursing into the slot's own element positions (a nested
/// array, a map).
fn hoist_subschema_shape(
    language: Language,
    path: &Path,
    name: &str,
    context: &str,
    slot: &mut Schema,
    hoisted: &mut Vec<HoistedDef>,
) -> Result<()> {
    if is_sum_type_union(slot) || is_inline_object_shape(slot) {
        let name = resolve_shape_name(language, name, slot, context)?;
        move_into_defs(slot, name, context.to_string(), hoisted);
        return Ok(());
    }
    if hoist_nullable_object_branch(language, name, context, slot, hoisted)? {
        return Ok(());
    }
    hoist_subschema_shapes(language, path, name, context, slot, hoisted)
}

/// Hoists the object inside a **nullability wrapper** (`oneOf: [T, null]`) under
/// the position's own name, leaving the wrapper in place over a `$ref`. The
/// wrapper emits no type of its own — every target expresses it structurally on
/// the value — so the object it wraps occupies the position, exactly as a
/// plainly-written one does; adding or removing nullability therefore never
/// renames the type. Returns whether it hoisted.
fn hoist_nullable_object_branch(
    language: Language,
    derived: &str,
    context: &str,
    slot: &mut Schema,
    hoisted: &mut Vec<HoistedDef>,
) -> Result<bool> {
    if is_sum_type_union(slot) {
        return Ok(false);
    }
    let Some(branch) = slot
        .one_of
        .as_mut()
        .and_then(|branches| branches.iter_mut().find(|b| is_inline_object_shape(b)))
    else {
        return Ok(false);
    };
    let name = resolve_shape_name(language, derived, branch, context)?;
    move_into_defs(branch, name, context.to_string(), hoisted);
    Ok(true)
}

/// The name a hoisted shape takes: its own `x-<lang>-name` for the active target
/// when it carries one, else the name derived from its position.
fn resolve_shape_name(
    language: Language,
    derived: &str,
    schema: &Schema,
    context: &str,
) -> Result<String> {
    match (lang_name_keyword(language), override_name(language, schema)) {
        (Some(keyword), Some(value)) => {
            validate_override(
                language,
                keyword,
                &Value::String(value.to_string()),
                context,
            )?;
            Ok(value.to_string())
        }
        _ => Ok(derived.to_string()),
    }
}

/// Replaces a schema position with a `$ref` at `name` and queues the shape that
/// was written there for insertion into `$defs`.
fn move_into_defs(slot: &mut Schema, name: String, origin: String, hoisted: &mut Vec<HoistedDef>) {
    let shape = std::mem::take(slot);
    *slot = Schema {
        reference: Some(format!("#/$defs/{name}")),
        ..Schema::default()
    };
    hoisted.push(HoistedDef {
        name,
        origin,
        schema: shape,
    });
}

/// True when a `oneOf` node is a **sum type** — two or more non-`null` branches
/// — as opposed to the degenerate nullability pattern (`oneOf: [T, null]`),
/// which every target expresses structurally on the element itself and which
/// therefore needs no name.
fn is_sum_type_union(schema: &Schema) -> bool {
    schema.one_of.as_ref().is_some_and(|branches| {
        branches
            .iter()
            .filter(|branch| !schema_type_is_null(branch))
            .count()
            >= 2
    })
}

/// The single non-`null` branch of a nullability wrapper (`oneOf: [T,
/// {"type": "null"}]`), or `None` for anything else — a sum type, or a schema
/// with no `oneOf` at all.
///
/// The wrapper itself declares no `type` and no keywords, so every rule that
/// asks "what kind is this value?" has to look through it. Mirrors
/// `nullable_non_null_schema` in the Go emitter.
fn nullable_non_null_schema(schema: &Schema) -> Option<&Schema> {
    let branches = schema.one_of.as_ref()?;
    if !branches.iter().any(schema_type_is_null) {
        return None;
    }
    let mut non_null = branches
        .iter()
        .filter(|branch| !schema_type_is_null(branch));
    let first = non_null.next()?;
    non_null.next().is_none().then_some(first)
}

/// True when a schema's `type` is exactly `"null"`.
fn schema_type_is_null(schema: &Schema) -> bool {
    schema.ty.as_ref().and_then(Value::as_str) == Some("null")
}

/// Names and hoists one union's inline object branches. A lone branch takes
/// `derived` — the name the union's position yields for it; two or more must each
/// carry the target's `x-<lang>-name`, because every branch would derive the same
/// name and nothing in a branch yields a *distinguishing* one (the discriminator
/// `const` is a wire value, not an identifier, and ordinals reorder silently when
/// a branch is inserted).
fn hoist_union_object_branches(
    language: Language,
    path: &Path,
    derived: &str,
    context: &str,
    branches: &mut [Schema],
    hoisted: &mut Vec<HoistedDef>,
) -> Result<()> {
    let inline: Vec<usize> = branches
        .iter()
        .enumerate()
        .filter(|(_, branch)| is_inline_object_shape(branch) && !is_free_form_object(branch))
        .map(|(index, _)| index)
        .collect();
    if inline.is_empty() {
        return Ok(());
    }
    let keyword = lang_name_keyword(language);
    for index in inline.iter().copied() {
        let branch = &branches[index];
        let override_ident = match (keyword, override_name(language, branch)) {
            (Some(keyword), Some(value)) => {
                validate_override(
                    language,
                    keyword,
                    &Value::String(value.to_string()),
                    &format!("{context}.oneOf[{index}]"),
                )?;
                Some(value.to_string())
            }
            _ => None,
        };
        let name = match (inline.len(), override_ident) {
            (_, Some(ident)) => ident,
            (1, None) => derived.to_string(),
            (_, None) => {
                return Err(Error::InvalidJsonSchema {
                    path: path.to_path_buf(),
                    reason: format!(
                        "{context}.oneOf[{index}]: a union with two or more inline object branches must name each one with `{}` (every branch would otherwise derive `{derived}`); name the branches, or move them into `$defs` and `$ref` them",
                        keyword.unwrap_or("x-<lang>-name"),
                    ),
                });
            }
        };
        move_into_defs(
            &mut branches[index],
            name,
            format!("{context}.oneOf[{index}]"),
            hoisted,
        );
    }
    Ok(())
}

/// Validates a `oneOf` as a supported closed sum type (or the degenerate
/// nullability pattern, which [[nullability]] owns). See
/// `specs/json-schema/features/oneOf.md` for the full acceptance rules.
fn validate_one_of(
    path: &Path,
    canonical_path: &Path,
    schema: &Schema,
    context: &str,
    docs: &IndexMap<PathBuf, (PathBuf, Document)>,
    models: &BTreeMap<TypeKey, JsonModel>,
) -> Result<()> {
    let branches = schema
        .one_of
        .as_deref()
        .expect("validate_one_of is called only for a oneOf schema");
    let reject = |reason: String| {
        Err(Error::InvalidJsonSchema {
            path: path.to_path_buf(),
            reason,
        })
    };

    if branches.is_empty() {
        return reject(format!(
            "{context}: `oneOf` must be a non-empty array of branches"
        ));
    }
    if branches.len() == 1 {
        return reject(format!(
            "{context}: a single-branch `oneOf` is a pointless wrapper; use the branch directly"
        ));
    }

    // Classify every branch by kind, resolving `$ref` branches to their target.
    let mut kinds: Vec<BranchKind> = Vec::with_capacity(branches.len());
    let mut resolved_schemas: Vec<Schema> = Vec::with_capacity(branches.len());
    let mut object_schemas: Vec<Schema> = Vec::new();
    let mut non_object_schemas: Vec<Schema> = Vec::new();
    for (index, branch) in branches.iter().enumerate() {
        let branch_context = format!("{context}.oneOf[{index}]");
        let resolved =
            resolve_branch_schema(branch, path, canonical_path, &branch_context, docs, models)?;
        let kind = one_of_branch_kind(branch, &resolved, path, &branch_context)?;
        if kind == BranchKind::Null
            && branch
                != &(Schema {
                    ty: Some(Value::String("null".to_string())),
                    ..Schema::default()
                })
        {
            return reject(format!(
                "{context}: a null branch must be exactly `{{type: \"null\"}}` with no sibling keywords"
            ));
        }
        if kind != BranchKind::Object && kind != BranchKind::Null {
            non_object_schemas.push(resolved.clone());
        }
        if kind == BranchKind::Object {
            // An inline object branch that declares a shape is named and moved
            // into `$defs` by `hoist_inline_object_shapes`, so by now it is a
            // `$ref` branch; the free-form object — which needs no name — is the
            // only one still written inline. Anything else means the branch sits
            // in a position the hoist does not reach.
            if branch.reference.is_none() && !is_free_form_object(&resolved) {
                return reject(format!(
                    "{context}: an inline object `oneOf` branch is not named in this position; make it a free-form object (`type: object` with `additionalProperties: true`), or move it into `$defs` and `$ref` it"
                ));
            }
            object_schemas.push(resolved.clone());
        }
        resolved_schemas.push(resolved);
        kinds.push(kind);
    }

    // At most one branch per non-object kind (a same-kind scalar choice is an
    // `enum`, not a `oneOf`; duplicate `null` is a tautology).
    for kind in [
        BranchKind::Null,
        BranchKind::Boolean,
        BranchKind::String,
        BranchKind::Integer,
        BranchKind::Number,
        BranchKind::Array,
    ] {
        let count = kinds.iter().filter(|value| **value == kind).count();
        if count > 1 {
            if matches!(
                kind,
                BranchKind::Boolean | BranchKind::String | BranchKind::Integer | BranchKind::Number
            ) {
                return reject(format!(
                    "{context}: two `oneOf` branches share the `{}` kind; a same-kind scalar choice is an `enum` (or `const` union), not a `oneOf`",
                    kind.label()
                ));
            }
            return reject(format!(
                "{context}: two `oneOf` branches share the `{}` kind, which has no decidable selector",
                kind.label()
            ));
        }
    }

    // `integer` + `number` overlap: any integer satisfies both, so exactly-one
    // is unsatisfiable (no discriminator can fix a numeric-token overlap).
    if kinds.contains(&BranchKind::Integer) && kinds.contains(&BranchKind::Number) {
        return reject(format!(
            "{context}: a `oneOf` cannot mix `integer` and `number` branches (both are the JSON number token and every integer is a number, so exactly-one is unsatisfiable)"
        ));
    }

    // Two or more object branches require a shared required-`const` discriminator.
    if object_schemas.len() >= 2 {
        let mut shared: Option<BTreeMap<String, Value>> = None;
        for object in &object_schemas {
            let tags = branch_discriminator_tags(object);
            shared = Some(match shared {
                None => tags,
                Some(existing) => existing
                    .into_iter()
                    .filter(|(name, _)| tags.contains_key(name))
                    .collect(),
            });
        }
        let shared = shared.unwrap_or_default();
        // Keep only names whose `const` values are pairwise-distinct across all
        // object branches, retaining one concrete duplicate for the diagnostic
        // when a shared tag exists but its values do not distinguish branches.
        let mut qualifying: Vec<&String> = Vec::new();
        let mut duplicate_values: Vec<(&String, Value)> = Vec::new();
        for name in shared.keys() {
            let values: Vec<Value> = object_schemas
                .iter()
                .filter_map(|object| branch_discriminator_tags(object).get(name).cloned())
                .collect();
            let duplicate = values.iter().enumerate().find_map(|(index, value)| {
                values[..index]
                    .iter()
                    .any(|existing| json_values_equal(existing, value))
                    .then(|| value.clone())
            });
            if let Some(value) = duplicate {
                duplicate_values.push((name, value));
            } else {
                qualifying.push(name);
            }
        }
        match qualifying.len() {
            0 => {
                if let Some((name, value)) = duplicate_values.first() {
                    return reject(format!(
                        "{context}: the object `oneOf` branches all declare `{name}` as a required `const` discriminator, but two branches share the value {value}; give every branch a distinct `{name}` tag value"
                    ));
                }
                return reject(format!(
                    "{context}: two or more object `oneOf` branches share no required `const` discriminator property with pairwise-distinct values; add a shared required `const`-tagged property (e.g. `kind`) to each branch"
                ));
            }
            1 => {}
            _ => {
                let names = qualifying
                    .iter()
                    .map(|name| format!("`{name}`"))
                    .collect::<Vec<_>>()
                    .join(", ");
                return reject(format!(
                    "{context}: the object `oneOf` branches have more than one qualifying `const` discriminator ({names}); the intended tag is ambiguous — keep exactly one shared required `const` tag property"
                ));
            }
        }
    }

    // Presence/nullable bookkeeping: a lone non-null branch paired with `null`
    // is the degenerate nullability pattern ([[nullability]] owns it); a lone
    // non-null branch with no `null` is a single-branch wrapper (already
    // rejected above). Two or more non-null branches form the sum type.
    let non_null = kinds
        .iter()
        .filter(|kind| **kind != BranchKind::Null)
        .count();
    if branches.len() == 2
        && non_null == 1
        && let Some(default) = schema.extra.get("default")
        && let Some((_, non_null_schema)) = kinds
            .iter()
            .zip(&resolved_schemas)
            .find(|(kind, _)| **kind != BranchKind::Null)
    {
        let mut schema_with_default = non_null_schema.clone();
        schema_with_default
            .extra
            .insert("default".to_string(), default.clone());
        validate_schema_node(
            path,
            &schema_with_default,
            &format!("{context}.default"),
            false,
        )?;
    }
    if non_null >= 2 {
        // P7.1 (decision D5): a `default` on a *sum type* is neither validated
        // nor lowered. No spec defines which branch it names, and Go emits
        // `return *m.F` against a sealed interface — the package does not
        // compile. Only the nullability wrapper above has a defined lowering.
        if schema.extra.contains_key("default") {
            return reject(format!(
                "{context}: a `default` on a `oneOf` sum type has no defined meaning (it names no branch); move the `default` onto the branch that should supply it, or drop it"
            ));
        }
        for branch in &non_object_schemas {
            reject_materialized_branch_keyword(path, branch, context)?;
        }
    }
    Ok(())
}

/// Rejects a **materializing** keyword on a non-object branch of a `oneOf` *sum
/// type*: a temporal [[format]] or a [[contentEncoding]]. Both replace the wire
/// `string` with a native typed value (`time.Time` / `OffsetDateTime` /
/// `datetime` / `Temporal.*`, `[]byte` / `byte[]` / `bytes`), and the synthesized
/// `<Union><Kind>` wrapper has no such type today — Python would materialize the
/// branch while Go, TypeScript, and Java carried an unvalidated `string`, which is
/// exactly the silent per-target divergence **P1** forbids. Deferred loudly (**P6**)
/// rather than approximated; see `specs/json-schema/features/oneOf.md` §Deferred.
///
/// Scoped to the sum type: the [[nullability]] pattern `oneOf:[{T},{null}]` has a
/// single non-null branch and no wrapper at all, so a materialized nullable
/// field keeps working ([[format]], [[contentEncoding]]).
fn reject_materialized_branch_keyword(path: &Path, branch: &Schema, context: &str) -> Result<()> {
    let reject = |keyword: &str, value: &str, native: &str| {
        Err(Error::InvalidJsonSchema {
            path: path.to_path_buf(),
            reason: format!(
                "{context}: a `oneOf` branch cannot declare `{keyword}: {value}` — it materializes a native {native} value, which a `oneOf` branch has no wrapper type for yet; drop the `{keyword}` to keep the branch a plain `string`, or carry the value as a property of an object branch"
            ),
        })
    };
    if let Some(Value::String(format)) = branch.extra.get("format")
        && crate::json_schema::format::TEMPORAL_FORMATS.contains(&format.as_str())
    {
        return reject("format", format, "date/time");
    }
    if let Some(Value::String(encoding)) = branch.extra.get("contentEncoding") {
        return reject("contentEncoding", encoding, "binary");
    }
    Ok(())
}

/// Whether a service/operation key matches its identifier regex (see
/// `specs/json-schema/services.md`): `^[A-Z][a-zA-Z\d]+$` for services (`first_upper`)
/// and `^[a-z][a-zA-Z\d]+$` for operations — a leading letter of the required
/// case followed by one or more ASCII alphanumerics.
fn name_matches(name: &str, first_upper: bool) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    let first_ok = if first_upper {
        first.is_ascii_uppercase()
    } else {
        first.is_ascii_lowercase()
    };
    let rest: Vec<char> = chars.collect();
    first_ok && !rest.is_empty() && rest.iter().all(char::is_ascii_alphanumeric)
}

/// P7.1: an `fqn` present but empty names the operation/service with the empty
/// string on the wire. Every target emits it verbatim, so the binding is
/// unaddressable in all four rather than in none.
fn reject_empty_fqn(path: &Path, fqn: Option<&str>, context: &str) -> Result<()> {
    if fqn.is_some_and(str::is_empty) {
        return Err(Error::InvalidJsonSchema {
            path: path.to_path_buf(),
            reason: format!(
                "{context}: `fqn` is empty; give it a wire name, or omit `fqn` to derive one from the declared name"
            ),
        });
    }
    Ok(())
}

/// The Stage-3 validity + collision pass over one service's operation keys.
///
/// A service's operations share one scope in every target — a Go interface's
/// method set, a TypeScript object's keys, a Python class's attributes, a Java
/// interface's methods — and one wire-name space. Neither was checked: the key
/// grammar (`^[a-z][a-zA-Z\d]+$`) admits both `getId` and `getID`, which recase
/// to a single identifier *and* derive a single default wire name, so the two
/// bindings silently collapsed into one; and it admits `import`, which Java
/// emits verbatim as `void import(In)` while the other three auto-mangle to
/// `import_` — the mangling P15 forbids outright.
///
/// The emitted identifier is the operation's `x-<lang>-name` used verbatim when
/// present (the P15 escape hatch), else the recased key — exactly what
/// `go_operation_field` / the Java, Python and TypeScript service planners
/// derive.
fn validate_operation_scope(
    path: &Path,
    language: Language,
    service_key: &str,
    service: &Service,
) -> Result<()> {
    let reject = |reason: String| -> Error {
        Error::InvalidJsonSchema {
            path: path.to_path_buf(),
            reason,
        }
    };
    let mut idents: BTreeMap<String, String> = BTreeMap::new();
    let mut wire_names: BTreeMap<String, String> = BTreeMap::new();
    for (operation_key, operation) in &service.operations {
        let override_ident = lang_name_keyword(language)
            .and_then(|keyword| operation.extra.get(keyword))
            .and_then(Value::as_str);
        if override_ident.is_none()
            && let Some((ident, reason)) = member_identifier_defect(language, operation_key)
        {
            return Err(reject(format!(
                "operation `{service_key}.{operation_key}` recases to `{ident}`, which {reason} in {} output; add an `{}` override with a valid identifier (P15 — the generator never auto-mangles)",
                language.as_str(),
                lang_name_keyword(language).unwrap_or("x-<lang>-name"),
            )));
        }
        let ident = override_ident
            .map(str::to_string)
            .unwrap_or_else(|| recase_member(language, operation_key));
        if let Some(previous) = idents.insert(ident.clone(), operation_key.to_string())
            && previous != *operation_key
        {
            return Err(reject(format!(
                "identifier collision in {} output: operations `{service_key}.{previous}` and `{service_key}.{operation_key}` both map to `{ident}`; disambiguate with an `{}` override (P15 — the generator never auto-mangles)",
                language.as_str(),
                lang_name_keyword(language).unwrap_or("x-<lang>-name"),
            )));
        }
        // The wire name is language-independent, so a clash here is a clash in
        // every target: two operations answering to one name over the wire.
        let wire_name = operation
            .fqn
            .clone()
            .unwrap_or_else(|| operation_key.to_upper_camel_case());
        if let Some(previous) = wire_names.insert(wire_name.clone(), operation_key.to_string())
            && previous != *operation_key
        {
            return Err(reject(format!(
                "operations `{service_key}.{previous}` and `{service_key}.{operation_key}` both bind the wire name `{wire_name}`; give one an explicit `fqn`"
            )));
        }
    }
    Ok(())
}

fn build_service(
    path: &Path,
    canonical_path: &Path,
    service_key: &str,
    service: &Service,
    docs: &IndexMap<PathBuf, (PathBuf, Document)>,
    models: &BTreeMap<TypeKey, JsonModel>,
    module_paths: Option<&BTreeMap<PathBuf, ModulePath>>,
    external_types: &mut BTreeMap<String, ExternalTypeBindingSpec>,
    language: Language,
) -> Result<ServiceSpec> {
    if !name_matches(service_key, true) {
        return Err(Error::InvalidJsonSchema {
            path: path.to_path_buf(),
            reason: format!(
                "service name `{service_key}` must match `^[A-Z][a-zA-Z\\d]+$` (start uppercase, then letters/digits); set the wire name via `fqn` if it must differ"
            ),
        });
    }
    if service.operations.is_empty() {
        return Err(Error::InvalidJsonSchema {
            path: path.to_path_buf(),
            reason: format!("service `{service_key}` must declare at least one operation"),
        });
    }
    reject_empty_fqn(
        path,
        service.fqn.as_deref(),
        &format!("service `{service_key}`"),
    )?;
    validate_operation_scope(path, language, service_key, service)?;
    let service_name = service_key.to_upper_camel_case();
    let operations = service
        .operations
        .iter()
        .map(|(operation_key, operation)| {
            build_operation(
                path,
                canonical_path,
                &service_name,
                operation_key,
                operation,
                docs,
                models,
                module_paths,
                external_types,
                language,
            )
        })
        .collect::<Result<Vec<_>>>()?;

    // A per-language `x-<lang>-name` on the service becomes the emitted code
    // identifier, verbatim (no recasing). It never affects `wire_name`.
    let code_name = if let Some(keyword) = lang_name_keyword(language)
        && let Some(value) = service.extra.get(keyword)
    {
        validate_override(
            language,
            keyword,
            value,
            &format!("service `{service_key}`"),
        )?;
        value.as_str().map(str::to_string)
    } else {
        None
    };

    Ok(ServiceSpec {
        name: service_name.clone(),
        code_name: language_string_override(language, code_name),
        wire_name: service.fqn.clone().unwrap_or(service_name),
        doc: language_string(service.description.clone()),
        namespace: LanguageStringSpec::default(),
        operations_class: LanguageStringSpec::default(),
        endpoint: service.endpoint.clone(),
        experimental: false,
        deprecated: service
            .extra
            .get("deprecated")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        delay_load_temporalio_workflow: false,
        operations,
        resources: Vec::new(),
        data: (),
    })
}

fn build_operation(
    path: &Path,
    canonical_path: &Path,
    service_name: &str,
    operation_key: &str,
    operation: &Operation,
    docs: &IndexMap<PathBuf, (PathBuf, Document)>,
    models: &BTreeMap<TypeKey, JsonModel>,
    module_paths: Option<&BTreeMap<PathBuf, ModulePath>>,
    external_types: &mut BTreeMap<String, ExternalTypeBindingSpec>,
    language: Language,
) -> Result<OperationSpec> {
    if !name_matches(operation_key, false) {
        return Err(Error::InvalidJsonSchema {
            path: path.to_path_buf(),
            reason: format!(
                "operation name `{operation_key}` must match `^[a-z][a-zA-Z\\d]+$` (start lowercase, then letters/digits); set the wire name via `fqn` if it must differ"
            ),
        });
    }
    reject_empty_fqn(
        path,
        operation.fqn.as_deref(),
        &format!("operation `{service_name}.{operation_key}`"),
    )?;
    let operation_name = operation_key.to_upper_camel_case();
    let input = operation
        .input
        .as_ref()
        .map(|schema| {
            operation_model_type(
                path,
                canonical_path,
                service_name,
                operation_key,
                "Input",
                schema,
                docs,
                models,
                module_paths,
                external_types,
            )
        })
        .transpose()?;
    let output = operation
        .output
        .as_ref()
        .map(|schema| {
            operation_model_type(
                path,
                canonical_path,
                service_name,
                operation_key,
                "Output",
                schema,
                docs,
                models,
                module_paths,
                external_types,
            )
        })
        .transpose()?;

    // A per-language `x-<lang>-name` on the operation becomes the emitted code
    // identifier, verbatim (no recasing). It never affects `wire_name`.
    let code_name = if let Some(keyword) = lang_name_keyword(language)
        && let Some(value) = operation.extra.get(keyword)
    {
        validate_override(
            language,
            keyword,
            value,
            &format!("operation `{operation_key}`"),
        )?;
        value.as_str().map(str::to_string)
    } else {
        None
    };

    Ok(OperationSpec {
        name: operation_name.clone(),
        code_name: language_string_override(language, code_name),
        wire_name: operation.fqn.clone().unwrap_or(operation_name),
        experimental: false,
        deprecated: operation
            .extra
            .get("deprecated")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        doc: language_string(operation.description.clone()),
        return_doc: LanguageStringSpec::default(),
        input,
        output,
        output_transform: None,
        serialization_context: LanguageStringSpec::default(),
        data: (),
    })
}

fn operation_model_type(
    path: &Path,
    canonical_path: &Path,
    service_name: &str,
    operation_key: &str,
    suffix: &str,
    schema: &Schema,
    docs: &IndexMap<PathBuf, (PathBuf, Document)>,
    models: &BTreeMap<TypeKey, JsonModel>,
    module_paths: Option<&BTreeMap<PathBuf, ModulePath>>,
    external_types: &mut BTreeMap<String, ExternalTypeBindingSpec>,
) -> Result<TypeSpec> {
    validate_schema_common(path, schema, &format!("operation {operation_key} {suffix}"))?;
    if let Some(reference) = &schema.reference {
        let model = resolve_ref(path, canonical_path, reference, docs, models)?;
        let model_path = docs
            .get(&model.canonical_path)
            .map(|(source_path, _)| source_path.as_path())
            .unwrap_or(model.canonical_path.as_path());
        require_object_io(
            model_path,
            &model.canonical_path,
            &model.schema,
            operation_key,
            suffix,
            docs,
            models,
        )?;
        insert_json_external_type(external_types, model, docs, models, module_paths)?;
        collect_schema_model_refs(
            &model.canonical_path,
            &model.canonical_path,
            &model.schema,
            docs,
            models,
            module_paths,
            external_types,
        )?;
        return json_model_type(model, docs, models, module_paths);
    }

    validate_model_schema(path, schema, &format!("operation {operation_key} {suffix}"))?;
    // Inline I/O must be an object (see `specs/json-schema/services.md`). After
    // `validate_model_schema` a non-`$ref` inline schema is either `type: object`
    // or a `oneOf` union; a union is not a valid operation input/output.
    require_object_io(
        path,
        canonical_path,
        schema,
        operation_key,
        suffix,
        docs,
        models,
    )?;
    let model_name = format!("{}{}", operation_key.to_upper_camel_case(), suffix);
    let model = JsonModel {
        full_name: format!("{service_name}.{model_name}"),
        canonical_path: canonical_path.to_path_buf(),
        model_name,
        schema: schema.clone(),
    };
    insert_json_external_type(external_types, &model, docs, models, module_paths)?;
    collect_schema_model_refs(
        canonical_path,
        canonical_path,
        &model.schema,
        docs,
        models,
        module_paths,
        external_types,
    )?;
    json_model_type(&model, docs, models, module_paths)
}

/// Requires an operation `input`/`output` to resolve to an object type: an
/// inline `type: object`, a `$ref` to one, or an `allOf` that merged to one
/// (merges run before this). Following bare-`$ref` chains, a target that lands
/// on a `oneOf` union or a scalar/array is a load reject — a union has no single
/// extensible shape. See `specs/json-schema/services.md`.
fn require_object_io(
    path: &Path,
    canonical_path: &Path,
    schema: &Schema,
    operation_key: &str,
    suffix: &str,
    docs: &IndexMap<PathBuf, (PathBuf, Document)>,
    models: &BTreeMap<TypeKey, JsonModel>,
) -> Result<()> {
    let mut current = schema.clone();
    let mut current_canonical = canonical_path.to_path_buf();
    let mut current_path = path.to_path_buf();
    let mut guard = 0usize;
    loop {
        if current.ty.as_ref().and_then(Value::as_str) == Some("object") {
            return Ok(());
        }
        if current.is_bare_ref() {
            let reference = current
                .reference
                .clone()
                .expect("a bare `$ref` carries a reference");
            let model = resolve_ref(&current_path, &current_canonical, &reference, docs, models)?;
            current_canonical = model.canonical_path.clone();
            current_path = docs
                .get(&model.canonical_path)
                .map(|(source_path, _)| source_path.clone())
                .unwrap_or_else(|| model.canonical_path.clone());
            current = model.schema.clone();
            guard += 1;
            if guard > models.len() + 1 {
                break;
            }
            continue;
        }
        break;
    }
    Err(Error::InvalidJsonSchema {
        path: path.to_path_buf(),
        reason: format!(
            "operation `{operation_key}` {suffix} must resolve to an object; a `oneOf` union or a scalar/array type is not a valid operation input/output — reference an object type, or wrap the value in a single-field object"
        ),
    })
}

/// Builds an `InvalidJsonSchema` error for the merge/normalization pass.
fn merge_reject(path: &Path, reason: String) -> Error {
    Error::InvalidJsonSchema {
        path: path.to_path_buf(),
        reason,
    }
}

/// Shared context threaded through the `allOf` merge: the set of input document
/// paths (for `$ref` target-file resolution) and a snapshot of the raw
/// (pre-merge) schemas keyed by [`TypeKey`] (for folding a `$ref` branch's
/// target into the merged result).
struct MergeCtx<'a> {
    doc_paths: &'a BTreeSet<PathBuf>,
    raw_models: &'a BTreeMap<TypeKey, Schema>,
}

/// Annotation siblings that caused a `$ref` use site to materialize into a
/// position-named declaration. Normalization removes the reference itself, so
/// the authored cause has to travel separately until hoist collision checking
/// can offer a remedy that is still applicable at that use site.
type RefFoldAnnotations = BTreeMap<PathBuf, BTreeMap<String, Vec<String>>>;

/// Snapshots every named model schema (each `$defs` entry and each schema-shaped
/// document root) as a raw, pre-merge [`Schema`], keyed by [`TypeKey`]. This is
/// the map the `allOf`/`$ref`-sibling fold resolves a branch `$ref` against so it
/// can inline (flatten) the target's schema.
fn collect_raw_models(
    docs: &IndexMap<PathBuf, (PathBuf, Document)>,
) -> Result<BTreeMap<TypeKey, Schema>> {
    let mut raw = BTreeMap::new();
    for (canonical_path, (path, doc)) in docs {
        if let Some(defs) = &doc.defs {
            collect_raw_defs(path, canonical_path, defs, &[], &mut raw)?;
        }
        if root_is_schema_shaped(&doc.root) {
            raw.insert(TypeKey::Root(canonical_path.clone()), doc.root.clone());
        }
    }
    Ok(raw)
}

fn collect_raw_defs(
    path: &Path,
    canonical_path: &Path,
    defs: &IndexMap<String, Schema>,
    parent_names: &[String],
    raw: &mut BTreeMap<TypeKey, Schema>,
) -> Result<()> {
    for (name, schema) in defs {
        let mut names = parent_names.to_vec();
        names.push(name.clone());
        raw.insert(
            TypeKey::Def(canonical_path.to_path_buf(), names.clone()),
            schema.clone(),
        );
        if let Some(nested) = nested_defs(path, schema, &def_context(&names))? {
            collect_raw_defs(path, canonical_path, &nested, &names, raw)?;
        }
    }
    Ok(())
}

fn collect_json_models_from_defs(
    path: &Path,
    canonical_path: &Path,
    defs: &IndexMap<String, Schema>,
    parent_names: &[String],
    models: &mut BTreeMap<TypeKey, JsonModel>,
) -> Result<()> {
    for (name, schema) in defs {
        let mut names = parent_names.to_vec();
        names.push(name.clone());
        models.insert(
            TypeKey::Def(canonical_path.to_path_buf(), names.clone()),
            JsonModel {
                full_name: names.join("."),
                canonical_path: canonical_path.to_path_buf(),
                model_name: name.to_upper_camel_case(),
                schema: schema.clone(),
            },
        );
        if let Some(nested) = nested_defs(path, schema, &def_context(&names))? {
            collect_json_models_from_defs(path, canonical_path, &nested, &names, models)?;
        }
    }
    Ok(())
}

fn nested_defs(
    path: &Path,
    schema: &Schema,
    context: &str,
) -> Result<Option<IndexMap<String, Schema>>> {
    schema
        .extra
        .get("$defs")
        .map(|value| {
            serde_json::from_value(value.clone()).map_err(|error| Error::InvalidJsonSchema {
                path: path.to_path_buf(),
                reason: format!("{context}: `$defs` is not an object of schemas: {error}"),
            })
        })
        .transpose()
}

fn def_context(names: &[String]) -> String {
    let mut context = String::new();
    for name in names {
        context.push_str("$defs.");
        context.push_str(name);
        context.push('.');
    }
    context.pop();
    context
}

/// The canonical file path a [`TypeKey`] lives in.
fn type_key_path(key: &TypeKey) -> &PathBuf {
    match key {
        TypeKey::Root(path) | TypeKey::Def(path, _) => path,
    }
}

/// Normalizes every schema in a parsed document in place: each `$defs` entry, the
/// schema-shaped root, and each service operation's input/output. Normalization
/// merges/flattens any `allOf` (and rewrites `$ref`-with-siblings to the same
/// merge) into a single materialized schema.
fn normalize_document(
    path: &Path,
    canonical_path: &Path,
    doc: &mut Document,
    ctx: &MergeCtx,
    ref_fold_annotations: &mut BTreeMap<String, Vec<String>>,
) -> Result<()> {
    if let Some(defs) = &mut doc.defs {
        for (name, schema) in defs.iter_mut() {
            // Seed the ancestry with the declared model itself. A descendant
            // `$ref`-with-siblings back to this model is a recursive merge,
            // while references to models inherited by an outer `allOf` remain
            // independent paths and must not be mistaken for ancestors.
            let mut cycle = vec![TypeKey::Def(
                canonical_path.to_path_buf(),
                vec![name.clone()],
            )];
            *schema = normalize_schema(
                path,
                canonical_path,
                schema,
                ctx,
                &mut cycle,
                &format!("$defs.{name}"),
                ref_fold_annotations,
            )?;
        }
    }
    if root_is_schema_shaped(&doc.root) && !doc.root.is_bare_ref() {
        let mut cycle = vec![TypeKey::Root(canonical_path.to_path_buf())];
        doc.root = normalize_schema(
            path,
            canonical_path,
            &doc.root,
            ctx,
            &mut cycle,
            "root schema",
            ref_fold_annotations,
        )?;
    }
    if let Some(services) = &mut doc.services {
        for (service_name, service) in services.iter_mut() {
            for (operation_name, operation) in service.operations.iter_mut() {
                if let Some(input) = &mut operation.input {
                    let mut cycle = Vec::new();
                    *input = normalize_schema(
                        path,
                        canonical_path,
                        input,
                        ctx,
                        &mut cycle,
                        &format!("services.{service_name}.operations.{operation_name}.input"),
                        ref_fold_annotations,
                    )?;
                }
                if let Some(output) = &mut operation.output {
                    let mut cycle = Vec::new();
                    *output = normalize_schema(
                        path,
                        canonical_path,
                        output,
                        ctx,
                        &mut cycle,
                        &format!("services.{service_name}.operations.{operation_name}.output"),
                        ref_fold_annotations,
                    )?;
                }
            }
        }
    }
    Ok(())
}

/// Normalizes a single schema. A node carrying `allOf` (or `$ref` with sibling
/// keywords — the implicit-`allOf` sugar) is flattened into its conjunct branches
/// and merged into one materialized schema; any other node is left as-is. In both
/// cases the schema's children (`properties`, `items`, `oneOf` branches, and a
/// schema-valued `additionalProperties`) are normalized recursively so nested
/// `allOf` deeper in the tree is merged too.
fn normalize_schema(
    path: &Path,
    canonical_path: &Path,
    schema: &Schema,
    ctx: &MergeCtx,
    cycle: &mut Vec<TypeKey>,
    context: &str,
    ref_fold_annotations: &mut BTreeMap<String, Vec<String>>,
) -> Result<Schema> {
    let has_all_of = schema.extra.contains_key("allOf");
    // An `x-<lang>-name` beside a `$ref` names the *member* and asserts nothing
    // about the value, so it is not a conjunct: folding it would clone the
    // referenced target into the use site instead of referencing it.
    let ref_with_siblings =
        schema.reference.is_some() && !schema.is_ref_with_non_conjunct_siblings_only();

    if ref_with_siblings {
        let annotations = ref_fold_annotation_keywords(schema);
        if !annotations.is_empty() {
            ref_fold_annotations.insert(context.to_string(), annotations);
        }
    }

    if has_all_of || ref_with_siblings {
        if has_all_of {
            let Some(entries) = schema.extra["allOf"].as_array() else {
                return Err(merge_reject(
                    path,
                    format!("{context}: `allOf` must be a non-empty array of schemas"),
                ));
            };
            if entries.is_empty() {
                return Err(merge_reject(
                    path,
                    format!(
                        "{context}: `allOf` must not be empty (an empty `allOf` asserts nothing; remove it)"
                    ),
                ));
            }
            if entries.len() == 1 && !ref_with_siblings && own_conjunct(schema) == Schema::default()
            {
                return Err(merge_reject(
                    path,
                    format!(
                        "{context}: a single-branch `allOf` is a pointless wrapper; inline the branch directly"
                    ),
                ));
            }
        }
        let branches = expand_branches(path, canonical_path, schema, ctx, cycle, context)?;
        if branches.is_empty() {
            return Err(merge_reject(
                path,
                format!("{context}: `allOf` merges to an empty schema (it asserts nothing)"),
            ));
        }
        let merged = merge_branch_list(path, branches, context)?;
        // A direct `$ref`-with-siblings is an implicit merge of that target, so
        // keep that one followed edge active while walking the merged target's
        // descendants. Explicit `allOf` branch targets are not all ancestors of
        // every merged child: retaining that global set falsely rejected an
        // acyclic child reference to one of a model's inherited traits.
        let stack_len = cycle.len();
        if ref_with_siblings {
            let key = resolve_ref_key_at(
                path,
                canonical_path,
                schema.reference.as_deref().expect("reference is present"),
                ctx.doc_paths,
                context,
            )?;
            if !cycle.contains(&key) {
                cycle.push(key);
            }
        }
        let normalized = normalize_children(
            path,
            canonical_path,
            merged,
            ctx,
            cycle,
            context,
            ref_fold_annotations,
        );
        cycle.truncate(stack_len);
        return normalized;
    }

    normalize_children(
        path,
        canonical_path,
        schema.clone(),
        ctx,
        cycle,
        context,
        ref_fold_annotations,
    )
}

fn ref_fold_annotation_keywords(schema: &Schema) -> Vec<String> {
    let mut annotations = Vec::new();
    if schema.title.is_some() {
        annotations.push("title".to_string());
    }
    if schema.description.is_some() {
        annotations.push("description".to_string());
    }
    if schema.extra.contains_key("default") {
        annotations.push("default".to_string());
    }
    annotations
}

/// Recursively normalizes a schema's child schemas (leaving its own keywords
/// untouched).
fn normalize_children(
    path: &Path,
    canonical_path: &Path,
    mut schema: Schema,
    ctx: &MergeCtx,
    cycle: &mut Vec<TypeKey>,
    context: &str,
    ref_fold_annotations: &mut BTreeMap<String, Vec<String>>,
) -> Result<Schema> {
    if let Some(properties) = schema.properties.take() {
        let mut normalized = IndexMap::new();
        for (name, property) in properties {
            let normalized_property = normalize_schema(
                path,
                canonical_path,
                &property,
                ctx,
                cycle,
                &format!("{context}.properties.{name}"),
                ref_fold_annotations,
            )?;
            normalized.insert(name, normalized_property);
        }
        schema.properties = Some(normalized);
    }
    if let Some(items) = schema.items.take() {
        schema.items = Some(Box::new(normalize_schema(
            path,
            canonical_path,
            &items,
            ctx,
            cycle,
            &format!("{context}.items"),
            ref_fold_annotations,
        )?));
    }
    if let Some(one_of) = schema.one_of.take() {
        let mut normalized = Vec::new();
        for branch in one_of {
            normalized.push(normalize_schema(
                path,
                canonical_path,
                &branch,
                ctx,
                cycle,
                &format!("{context}.oneOf"),
                ref_fold_annotations,
            )?);
        }
        schema.one_of = Some(normalized);
    }
    if let Some(additional) = &schema.additional_properties
        && additional.is_object()
    {
        let additional_schema: Schema =
            serde_json::from_value(additional.clone()).map_err(|error| {
                merge_reject(
                    path,
                    format!("{context}.additionalProperties is invalid: {error}"),
                )
            })?;
        let normalized = normalize_schema(
            path,
            canonical_path,
            &additional_schema,
            ctx,
            cycle,
            &format!("{context}.additionalProperties"),
            ref_fold_annotations,
        )?;
        schema.additional_properties =
            Some(serde_json::to_value(&normalized).map_err(|error| {
                merge_reject(
                    path,
                    format!("{context}: failed to preserve additionalProperties: {error}"),
                )
            })?);
    }
    if let Some(value) = schema.extra.shift_remove("$defs") {
        let defs: IndexMap<String, Schema> = serde_json::from_value(value).map_err(|error| {
            merge_reject(
                path,
                format!("{context}: `$defs` is not an object of schemas: {error}"),
            )
        })?;
        let mut normalized_defs = IndexMap::new();
        for (name, definition) in defs {
            normalized_defs.insert(
                name.clone(),
                normalize_schema(
                    path,
                    canonical_path,
                    &definition,
                    ctx,
                    cycle,
                    &format!("{context}.$defs.{name}"),
                    ref_fold_annotations,
                )?,
            );
        }
        schema.extra.insert(
            "$defs".to_string(),
            serde_json::to_value(normalized_defs).map_err(|error| {
                merge_reject(
                    path,
                    format!("{context}: failed to preserve nested `$defs`: {error}"),
                )
            })?,
        );
    }
    for keyword in ["contains", "propertyNames"] {
        if let Some(value) = schema.extra.get(keyword).cloned()
            && value.is_object()
        {
            let subschema: Schema = serde_json::from_value(value).map_err(|error| {
                merge_reject(
                    path,
                    format!("{context}.{keyword} is not a valid schema: {error}"),
                )
            })?;
            let normalized = normalize_schema(
                path,
                canonical_path,
                &subschema,
                ctx,
                cycle,
                &format!("{context}.{keyword}"),
                ref_fold_annotations,
            )?;
            schema.extra.insert(
                keyword.to_string(),
                serde_json::to_value(normalized).map_err(|error| {
                    merge_reject(
                        path,
                        format!("{context}: failed to preserve `{keyword}`: {error}"),
                    )
                })?,
            );
        }
    }
    normalize_pattern(path, &mut schema, context)?;
    normalize_count_bounds(&mut schema);
    normalize_temporal_literals(&mut schema);
    schema.extra.shift_remove("$comment");
    schema.extra.shift_remove("examples");
    Ok(schema)
}

/// Rewrites a `const`/`default`/`enum` literal on a materialized temporal node
/// to its **canonical wire string** (decision D10).
///
/// A temporal `format` replaces the wire string with a native value, so the
/// literal and the value can only be compared through one agreed spelling. The
/// loader used to keep the literal exactly as authored, which left each target
/// comparing on a different side of the codec: `const: "PT90M"` accepted the
/// wire `"PT1H30M"` in Go (native comparison) and rejected it in the other three
/// (wire-string comparison), and a model parsed from `"PT90M"` could not be
/// serialized at all in TypeScript, Python or Java. Java went further and handed
/// the raw literal to `OffsetDateTime.parse`, which throws on the lowercase
/// `t`/`z` the loader accepts.
///
/// Canonicalizing here gives all four one string to compare against. An invalid
/// literal is left untouched for `validate_format` to reject with the value the
/// user wrote.
fn normalize_temporal_literals(schema: &mut Schema) {
    let Some(Value::String(format)) = schema.extra.get("format").cloned() else {
        return;
    };
    let canonical = |value: &Value| -> Option<Value> {
        let literal = value.as_str()?;
        let canonical = crate::json_schema::format::canonicalize_for_format(&format, literal)?;
        (canonical != literal).then(|| Value::String(canonical))
    };
    for keyword in ["const", "default"] {
        if let Some(value) = schema.extra.get(keyword)
            && let Some(rewritten) = canonical(value)
        {
            schema.extra.insert(keyword.to_string(), rewritten);
        }
    }
    if let Some(Value::Array(members)) = schema.extra.get("enum") {
        let mut rewritten = members.clone();
        let mut changed = false;
        for member in &mut rewritten {
            if let Some(value) = canonical(member) {
                *member = value;
                changed = true;
            }
        }
        if changed {
            schema
                .extra
                .insert("enum".to_string(), Value::Array(rewritten));
        }
    }
}

/// The count-family keywords accept an integral float spelling (`minItems: 2.0`
/// is the integer `2` — the `1.0`-as-integer rule from `type`), but every
/// backend deserializes the planned schema's count bounds into `Option<u64>` /
/// `Option<usize>`, and serde refuses a JSON float there. Canonicalize the
/// spelling once, during the normalize pass, so the accepted-value set the
/// validators describe is the one the emitters actually see.
///
/// Out-of-range / non-integral / non-numeric spellings are left untouched: the
/// per-keyword validators own those diagnostics and run after this pass.
fn normalize_count_bounds(schema: &mut Schema) {
    const COUNT_KEYWORDS: [&str; 8] = [
        "minLength",
        "maxLength",
        "minItems",
        "maxItems",
        "minContains",
        "maxContains",
        "minProperties",
        "maxProperties",
    ];
    const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;
    for keyword in COUNT_KEYWORDS {
        let Some(Value::Number(number)) = schema.extra.get(keyword) else {
            continue;
        };
        if number.is_u64() {
            continue;
        }
        let Some(value) = number.as_f64() else {
            continue;
        };
        if !value.is_finite() || value.fract() != 0.0 || value < 0.0 || value > MAX_SAFE_INTEGER {
            continue;
        }
        schema
            .extra
            .insert(keyword.to_string(), Value::from(value as u64));
    }
}

/// Load-time gate + normalization for the `pattern` keyword, applied to the
/// node's own `pattern` during the normalize pass so the value flows to the
/// backends already normalized (`\s`/`\S` expanded to the explicit ASCII class;
/// `$` kept canonical for the per-target backend rewrite). See
/// `specs/json-schema/features/pattern.md`.
///
/// Rejects (P7 / P7.1): a non-string `pattern` value, a `pattern` on a
/// non-`string` node, a non-portable regex (backtracking / inline flags /
/// open-complement `\S`-in-class), and a `const`/`default`/`enum` string literal
/// on the same node that the pattern does not match.
fn normalize_pattern(path: &Path, schema: &mut Schema, context: &str) -> Result<()> {
    let Some(value) = schema.extra.get("pattern") else {
        return Ok(());
    };
    let Some(pattern) = value.as_str() else {
        return Err(merge_reject(
            path,
            format!("{context}: `pattern` must be a string"),
        ));
    };
    let pattern = pattern.to_string();

    // P7.1: `pattern` is a string assertion — meaningless on a non-string node.
    if schema.ty.as_ref().and_then(Value::as_str) != Some("string") {
        return Err(merge_reject(
            path,
            format!("{context}: `pattern` requires `type: string`"),
        ));
    }

    let normalized = crate::json_schema::pattern::gate_and_normalize(&pattern)
        .map_err(|error| merge_reject(path, format!("{context}: {}", error.0)))?;

    // A supplied string literal on the same node must match the pattern at load
    // (the regex half of the deferred literal-vs-constraint obligation). Matched
    // unanchored, as at runtime.
    let matcher = regex::Regex::new(&normalized).map_err(|error| {
        merge_reject(
            path,
            format!("{context}: `pattern` failed to compile after normalization: {error}"),
        )
    })?;
    let check = |literal: &str, source: &str| -> Result<()> {
        if !matcher.is_match(literal) {
            return Err(merge_reject(
                path,
                format!(
                    "{context}: `{source}` value {literal:?} does not match `pattern` {normalized:?}"
                ),
            ));
        }
        Ok(())
    };
    for literal_key in ["const", "default"] {
        if let Some(Value::String(literal)) = schema.extra.get(literal_key) {
            check(literal, literal_key)?;
        }
    }
    if let Some(Value::Array(values)) = schema.extra.get("enum") {
        for value in values {
            if let Some(literal) = value.as_str() {
                check(literal, "enum")?;
            }
        }
    }

    schema
        .extra
        .insert("pattern".to_string(), Value::String(normalized));
    Ok(())
}

/// A schema's "own" conjunct: a clone with `$ref` and `allOf` stripped, i.e. the
/// keywords declared directly on the node (which fold in as the final,
/// last-wins branch).
fn own_conjunct(schema: &Schema) -> Schema {
    let mut own = schema.clone();
    own.reference = None;
    own.extra.shift_remove("allOf");
    // These annotations are deliberately accepted-and-ignored. Removing them
    // before the fold prevents inert differences from becoming merge conflicts.
    own.extra.shift_remove("$comment");
    own.extra.shift_remove("examples");
    own
}

/// Strips the keywords that belong to a referenced **declaration** rather than
/// to the value it constrains, applied to every conjunct folded in from a
/// `$ref` branch.
///
/// - `x-<lang>-name` renames the *target type*. Carrying it into the merge site
///   would make the merged node claim the target's emitted identifier, which
///   collides with the target itself — and only for the one language whose
///   keyword was authored, so the same schema would load for three targets and
///   reject for the fourth (a P1 break).
/// - `$defs` are the target's own nested declarations. They are already
///   collected as models where the target is declared; copying them into the
///   merge site declares each of them a second time, under a name the user
///   never wrote and so cannot rename (P15 forbids a fix-it the user cannot
///   apply).
fn strip_target_declaration_keywords(schema: &mut Schema) {
    schema.extra.shift_remove("$defs");
    for keyword in LANG_NAME_KEYWORDS {
        schema.extra.shift_remove(keyword);
    }
}

/// Rejects a schema used as an `allOf` conjunct that is itself a boolean-logic
/// combinator (`oneOf`/`anyOf`/`not`/`if`) — an intersection with a union,
/// negation, or runtime fork does not collapse to a single type.
fn reject_combinator_branch(path: &Path, schema: &Schema, context: &str) -> Result<()> {
    if schema.one_of.is_some() {
        return Err(merge_reject(
            path,
            format!(
                "{context}: an `allOf` branch cannot be a `oneOf` (an intersection with a union does not collapse to a single type)"
            ),
        ));
    }
    for keyword in ["anyOf", "not", "if"] {
        if schema.extra.contains_key(keyword) {
            return Err(merge_reject(
                path,
                format!(
                    "{context}: an `allOf` branch cannot be `{keyword}` (this combinator does not collapse to a single type)"
                ),
            ));
        }
    }
    Ok(())
}

/// Flattens a schema into the ordered list of leaf conjunct schemas that must all
/// hold: a `$ref` branch is resolved and its target folded in (recursively
/// flattened, with cycle detection), nested `allOf` is inlined, `true`/`{}`
/// identity branches are dropped, and the node's own keywords fold in as the
/// final branch (so a use-site annotation wins under last-wins).
fn expand_branches(
    path: &Path,
    canonical_path: &Path,
    schema: &Schema,
    ctx: &MergeCtx,
    cycle: &mut Vec<TypeKey>,
    context: &str,
) -> Result<Vec<Schema>> {
    reject_combinator_branch(path, schema, context)?;

    let mut branches = Vec::new();

    if let Some(reference) = &schema.reference {
        let key = resolve_ref_key_at(path, canonical_path, reference, ctx.doc_paths, context)?;
        if cycle.contains(&key) {
            return Err(merge_reject(
                path,
                format!(
                    "{context}: `$ref` `{reference}` forms an `allOf` merge cycle (a type cannot be flattened into itself)"
                ),
            ));
        }
        let target = ctx
            .raw_models
            .get(&key)
            .ok_or_else(|| {
                merge_reject(
                    path,
                    unresolved_ref_reason(context, reference, &key, type_key_path(&key)),
                )
            })?
            .clone();
        let target_path = type_key_path(&key).clone();
        cycle.push(key);
        let mut sub = expand_branches(&target_path, &target_path, &target, ctx, cycle, context)?;
        cycle.pop();
        for conjunct in &mut sub {
            strip_target_declaration_keywords(conjunct);
        }
        branches.extend(sub);
    }

    if let Some(all_of) = schema.extra.get("allOf") {
        let Some(entries) = all_of.as_array() else {
            return Err(merge_reject(
                path,
                format!("{context}: `allOf` must be an array of schemas"),
            ));
        };
        for (index, entry) in entries.iter().enumerate() {
            match entry {
                Value::Bool(true) => continue,
                Value::Bool(false) => {
                    return Err(merge_reject(
                        path,
                        format!(
                            "{context}: `allOf[{index}]` is `false` (nothing can validate); remove the branch or the whole `allOf`"
                        ),
                    ));
                }
                Value::Object(_) => {
                    let entry_schema: Schema =
                        serde_json::from_value(entry.clone()).map_err(|error| {
                            merge_reject(
                                path,
                                format!(
                                    "{context}: `allOf[{index}]` is not a valid schema: {error}"
                                ),
                            )
                        })?;
                    if entry_schema == Schema::default() {
                        continue;
                    }
                    let sub = expand_branches(
                        path,
                        canonical_path,
                        &entry_schema,
                        ctx,
                        cycle,
                        &format!("{context}.allOf[{index}]"),
                    )?;
                    branches.extend(sub);
                }
                _ => {
                    return Err(merge_reject(
                        path,
                        format!("{context}: `allOf[{index}]` must be a schema object"),
                    ));
                }
            }
        }
    }

    let own = own_conjunct(schema);
    if own != Schema::default() {
        branches.push(own);
    }

    Ok(branches)
}

/// Folds an ordered list of conjunct branches into one merged schema.
fn merge_branch_list(path: &Path, branches: Vec<Schema>, context: &str) -> Result<Schema> {
    let mut iter = branches.into_iter();
    let mut acc = iter.next().expect("branch list is non-empty");
    for branch in iter {
        acc = merge_schema_pair(path, acc, &branch, context)?;
    }
    finalize_merged(path, &mut acc, context)?;
    Ok(acc)
}

/// Merges two schemas that both constrain the same value, then finalizes the
/// result (collapses cross-keyword numeric-bound pairs, resolves `const`+`enum`).
fn merge_schema_pair(path: &Path, acc: Schema, branch: &Schema, context: &str) -> Result<Schema> {
    let mut merged = merge_two(path, acc, branch, context)?;
    finalize_merged(path, &mut merged, context)?;
    Ok(merged)
}

/// Merges two schemas that occupy the same **child** position — a property of
/// the same name in both branches, the element schema, the catch-all value
/// schema.
///
/// [`merge_two`] can only fold a pair of plain leaf schemas: it has no resolver
/// and no cycle stack, so it cannot flatten a `$ref` or a nested `allOf`, and it
/// has no coherent intersection for a `oneOf`. Unlike the top-level conjunct
/// list — which [`expand_branches`] has already flattened and combinator-checked
/// — these children are the schemas **as authored** and can still carry all
/// three. Re-express such a pair as an `allOf` node instead of merging it here:
/// the enclosing [`normalize_children`] walk normalizes every child, and that
/// call does carry the merge context, so the reference resolves against the
/// target, the nested `allOf` flattens, and a `oneOf` conjunct is rejected by
/// [`reject_combinator_branch`] exactly as it would be at the top level.
fn merge_child_schemas(path: &Path, acc: Schema, branch: &Schema, context: &str) -> Result<Schema> {
    // Two identical children (the common case: both branches say `$ref: Base`)
    // merge to themselves, which keeps the reference intact rather than inlining
    // the target twice.
    if acc == *branch || *branch == Schema::default() {
        return Ok(acc);
    }
    if acc == Schema::default() {
        return Ok(branch.clone());
    }
    let unflattened = |schema: &Schema| {
        schema.reference.is_some() || schema.one_of.is_some() || schema.extra.contains_key("allOf")
    };
    if !unflattened(&acc) && !unflattened(branch) {
        return merge_schema_pair(path, acc, branch, context);
    }
    let conjuncts = [&acc, branch]
        .into_iter()
        .map(serde_json::to_value)
        .collect::<std::result::Result<Vec<Value>, _>>()
        .map_err(|error| {
            merge_reject(
                path,
                format!("{context}: failed to defer the `allOf` merge of two subschemas: {error}"),
            )
        })?;
    Ok(Schema {
        extra: IndexMap::from([("allOf".to_string(), Value::Array(conjuncts))]),
        ..Schema::default()
    })
}

/// The core pairwise merge of two conjunct schemas.
fn merge_two(path: &Path, mut acc: Schema, branch: &Schema, context: &str) -> Result<Schema> {
    // Neither side can still carry a `$ref`: a top-level conjunct had its
    // reference resolved and stripped by `expand_branches`/`own_conjunct`, and a
    // child-position pair that still holds one is deferred by
    // `merge_child_schemas` instead of reaching here. The merged node is
    // therefore standalone.
    acc.reference = None;
    acc.ty = merge_type(path, acc.ty.take(), branch.ty.clone(), context)?;
    // Metadata annotations are last-wins.
    if branch.title.is_some() {
        acc.title = branch.title.clone();
    }
    if branch.description.is_some() {
        acc.description = branch.description.clone();
    }
    acc.properties = merge_properties(
        path,
        acc.properties.take(),
        branch.properties.clone(),
        context,
    )?;
    acc.required = merge_required(acc.required.take(), branch.required.clone());
    acc.additional_properties = merge_additional_properties(
        path,
        acc.additional_properties.take(),
        branch.additional_properties.clone(),
        context,
    )?;
    acc.items = merge_items(path, acc.items.take(), branch.items.clone(), context)?;
    for (key, branch_value) in &branch.extra {
        match acc.extra.get(key) {
            None => {
                acc.extra.insert(key.clone(), branch_value.clone());
            }
            Some(acc_value) => {
                let merged = merge_extra_value(path, key, acc_value, branch_value, context)?;
                acc.extra.insert(key.clone(), merged);
            }
        }
    }
    Ok(acc)
}

/// Merges two `type` values: identical dedupes, `integer`/`number` collapses to
/// `integer`, and any other differing pair is a disjoint-type reject.
fn merge_type(
    path: &Path,
    acc: Option<Value>,
    branch: Option<Value>,
    context: &str,
) -> Result<Option<Value>> {
    match (acc, branch) {
        (None, other) | (other, None) => Ok(other),
        (Some(a), Some(b)) => {
            if a == b {
                return Ok(Some(a));
            }
            if let (Some(sa), Some(sb)) = (a.as_str(), b.as_str()) {
                if matches!((sa, sb), ("integer", "number") | ("number", "integer")) {
                    return Ok(Some(Value::String("integer".to_string())));
                }
                return Err(merge_reject(
                    path,
                    format!(
                        "{context}: `allOf` branches declare disjoint types `{sa}` and `{sb}` (no value can be both)"
                    ),
                ));
            }
            Err(merge_reject(
                path,
                format!("{context}: `allOf` branches declare incompatible `type` values"),
            ))
        }
    }
}

/// Merges two `properties` maps: the union of names, with a name present in both
/// branches merged recursively.
fn merge_properties(
    path: &Path,
    acc: Option<IndexMap<String, Schema>>,
    branch: Option<IndexMap<String, Schema>>,
    context: &str,
) -> Result<Option<IndexMap<String, Schema>>> {
    match (acc, branch) {
        (None, other) | (other, None) => Ok(other),
        (Some(mut acc), Some(branch)) => {
            for (name, branch_schema) in branch {
                if let Some(existing) = acc.get(&name).cloned() {
                    let merged = merge_child_schemas(
                        path,
                        existing,
                        &branch_schema,
                        &format!("{context}.properties.{name}"),
                    )?;
                    acc.insert(name, merged);
                } else {
                    acc.insert(name, branch_schema);
                }
            }
            Ok(Some(acc))
        }
    }
}

/// Merges two `required` arrays into their union, preserving first-seen order.
fn merge_required(acc: Option<Value>, branch: Option<Value>) -> Option<Value> {
    let mut names: Vec<Value> = Vec::new();
    for source in [acc, branch] {
        if let Some(Value::Array(items)) = source {
            for item in items {
                if !names.contains(&item) {
                    names.push(item);
                }
            }
        }
    }
    if names.is_empty() {
        None
    } else {
        Some(Value::Array(names))
    }
}

/// Merges two `additionalProperties` values. Closed (`false`) in either branch
/// closes the merged object against the union of declared properties (the
/// closed-object footgun-fix); `true` yields to a value schema; two value
/// schemas merge recursively.
fn merge_additional_properties(
    path: &Path,
    acc: Option<Value>,
    branch: Option<Value>,
    context: &str,
) -> Result<Option<Value>> {
    match (acc, branch) {
        (None, None) => Ok(None),
        (Some(Value::Bool(false)), _) | (_, Some(Value::Bool(false))) => {
            Ok(Some(Value::Bool(false)))
        }
        (None, Some(other)) | (Some(other), None) => Ok(Some(other)),
        (Some(a), Some(b)) => {
            if a == b {
                return Ok(Some(a));
            }
            match (&a, &b) {
                (Value::Bool(true), other) | (other, Value::Bool(true)) => Ok(Some(other.clone())),
                (Value::Object(_), Value::Object(_)) => {
                    let acc_schema: Schema = serde_json::from_value(a).map_err(|error| {
                        merge_reject(
                            path,
                            format!("{context}.additionalProperties is invalid: {error}"),
                        )
                    })?;
                    let branch_schema: Schema = serde_json::from_value(b).map_err(|error| {
                        merge_reject(
                            path,
                            format!("{context}.additionalProperties is invalid: {error}"),
                        )
                    })?;
                    let merged = merge_child_schemas(
                        path,
                        acc_schema,
                        &branch_schema,
                        &format!("{context}.additionalProperties"),
                    )?;
                    Ok(Some(serde_json::to_value(&merged).map_err(|error| {
                        merge_reject(
                            path,
                            format!("{context}: failed to preserve additionalProperties: {error}"),
                        )
                    })?))
                }
                _ => Ok(Some(b)),
            }
        }
    }
}

/// Merges two `items` schemas recursively.
fn merge_items(
    path: &Path,
    acc: Option<Box<Schema>>,
    branch: Option<Box<Schema>>,
    context: &str,
) -> Result<Option<Box<Schema>>> {
    match (acc, branch) {
        (None, other) | (other, None) => Ok(other),
        (Some(acc), Some(branch)) => Ok(Some(Box::new(merge_child_schemas(
            path,
            *acc,
            &branch,
            &format!("{context}.items"),
        )?))),
    }
}

/// Merges two values for the same `extra`-map keyword per the per-keyword rules.
fn merge_extra_value(
    path: &Path,
    key: &str,
    acc: &Value,
    branch: &Value,
    context: &str,
) -> Result<Value> {
    if acc == branch || (key == "const" && json_values_equal(acc, branch)) {
        return Ok(acc.clone());
    }
    match key {
        "minimum" | "exclusiveMinimum" | "minLength" | "minItems" | "minProperties"
        | "minContains" => numeric_extreme(path, key, acc, branch, true, context),
        "maximum" | "exclusiveMaximum" | "maxLength" | "maxItems" | "maxProperties"
        | "maxContains" => numeric_extreme(path, key, acc, branch, false, context),
        "multipleOf" => merge_multiple_of(path, acc, branch, context),
        "uniqueItems" => Ok(Value::Bool(
            acc.as_bool().unwrap_or(false) || branch.as_bool().unwrap_or(false),
        )),
        "enum" => intersect_enum(path, acc, branch, context),
        "const" => Err(merge_reject(
            path,
            format!(
                "{context}: `allOf` branches declare conflicting `const` values ({acc} vs {branch})"
            ),
        )),
        "format" => merge_formats(path, acc, branch, context),
        "pattern" => Err(merge_reject(
            path,
            format!(
                "{context}: `allOf` branches declare different `pattern`s ({acc} vs {branch}); two regexes are not one regex"
            ),
        )),
        "contains" => Err(merge_reject(
            path,
            format!(
                "{context}: `allOf` branches declare different `contains` matchers; two existential constraints do not merge into one"
            ),
        )),
        "deprecated" => Ok(Value::Bool(
            acc.as_bool().unwrap_or(false) || branch.as_bool().unwrap_or(false),
        )),
        "default" | "title" | "description" => Ok(branch.clone()),
        "dependentRequired" => merge_dependent_required(acc, branch),
        "patternProperties" | "propertyNames" => {
            if let (Value::Object(_), Value::Object(_)) = (acc, branch) {
                let acc_schema: Schema = serde_json::from_value(acc.clone()).map_err(|error| {
                    merge_reject(
                        path,
                        format!("{context}: `{key}` is not a valid schema: {error}"),
                    )
                })?;
                let branch_schema: Schema =
                    serde_json::from_value(branch.clone()).map_err(|error| {
                        merge_reject(
                            path,
                            format!("{context}: `{key}` is not a valid schema: {error}"),
                        )
                    })?;
                let merged = merge_schema_pair(
                    path,
                    acc_schema,
                    &branch_schema,
                    &format!("{context}.{key}"),
                )?;
                Ok(serde_json::to_value(&merged).map_err(|error| {
                    merge_reject(
                        path,
                        format!("{context}: failed to preserve `{key}`: {error}"),
                    )
                })?)
            } else {
                Ok(branch.clone())
            }
        }
        _ => Err(merge_reject(
            path,
            format!(
                "{context}: cannot merge differing `{key}` values ({acc} vs {branch}) across `allOf` branches"
            ),
        )),
    }
}

/// Narrows two asserted formats when the owning format spec establishes a
/// containment relation. The result is the narrower accepted set; overlap
/// without containment remains unmergeable.
fn merge_formats(path: &Path, acc: &Value, branch: &Value, context: &str) -> Result<Value> {
    let Some(a) = acc.as_str() else {
        return Err(merge_reject(
            path,
            format!("{context}: `format` must be a string"),
        ));
    };
    let Some(b) = branch.as_str() else {
        return Err(merge_reject(
            path,
            format!("{context}: `format` must be a string"),
        ));
    };
    if crate::json_schema::format::accepted_set_is_contained_by(a, b) {
        Ok(acc.clone())
    } else if crate::json_schema::format::accepted_set_is_contained_by(b, a) {
        Ok(branch.clone())
    } else {
        Err(merge_reject(
            path,
            format!(
                "{context}: `allOf` branches declare unrelated `format`s ({acc} vs {branch}); neither accepted set contains the other"
            ),
        ))
    }
}

/// Keeps the tighter of two numeric bounds: the greater when `keep_max` (a lower
/// bound), else the smaller (an upper bound). Preserves the original JSON number
/// form (integer vs float).
fn numeric_extreme(
    path: &Path,
    key: &str,
    acc: &Value,
    branch: &Value,
    keep_max: bool,
    context: &str,
) -> Result<Value> {
    let parse = |value: &Value| -> Result<f64> {
        value
            .as_f64()
            .ok_or_else(|| merge_reject(path, format!("{context}: `{key}` must be a number")))
    };
    let a = parse(acc)?;
    let b = parse(branch)?;
    let keep_acc = if keep_max { a >= b } else { a <= b };
    Ok(if keep_acc {
        acc.clone()
    } else {
        branch.clone()
    })
}

/// Merges two `multipleOf` divisors to their least common multiple. Both are
/// positive integers (enforced downstream by the numeric validator), so the LCM
/// is a positive integer.
fn merge_multiple_of(path: &Path, acc: &Value, branch: &Value, context: &str) -> Result<Value> {
    let parse = |value: &Value| -> Result<i64> {
        value
            .as_f64()
            .filter(|number| number.is_finite() && number.fract() == 0.0 && *number > 0.0)
            .map(|number| number as i64)
            .ok_or_else(|| {
                merge_reject(
                    path,
                    format!("{context}: `multipleOf` must be a positive integer to merge"),
                )
            })
    };
    let a = parse(acc)?;
    let b = parse(branch)?;
    let gcd = {
        let (mut x, mut y) = (a, b);
        while y != 0 {
            let t = y;
            y = x % y;
            x = t;
        }
        x
    };
    // `a / gcd * b` overflows `i64` for large coprime divisors: a debug build
    // panics and a release build wraps to a negative divisor that every backend
    // then emits. The merged divisor also has to stay exactly representable in
    // every target, so cap it at the shared safe-integer ceiling.
    const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;
    let lcm = (a / gcd)
        .checked_mul(b)
        .filter(|lcm| *lcm <= MAX_SAFE_INTEGER)
        .ok_or_else(|| {
            merge_reject(
                path,
                format!(
                    "{context}: the least common multiple of the `allOf` branches' `multipleOf` divisors ({a} and {b}) exceeds the largest exactly representable integer (9007199254740991); use divisors that share a factor, or state the combined divisor directly"
                ),
            )
        })?;
    Ok(Value::Number(serde_json::Number::from(lcm)))
}

/// Intersects two `enum` value sets (first-seen order); an empty intersection is
/// a reject.
fn intersect_enum(path: &Path, acc: &Value, branch: &Value, context: &str) -> Result<Value> {
    let (Value::Array(acc_members), Value::Array(branch_members)) = (acc, branch) else {
        return Err(merge_reject(
            path,
            format!("{context}: `enum` must be an array of values"),
        ));
    };
    let mut out: Vec<Value> = Vec::new();
    for member in acc_members {
        if branch_members
            .iter()
            .any(|branch_member| json_values_equal(branch_member, member))
            && !out
                .iter()
                .any(|existing| json_values_equal(existing, member))
        {
            out.push(member.clone());
        }
    }
    if out.is_empty() {
        return Err(merge_reject(
            path,
            format!(
                "{context}: `allOf` branches have an empty `enum` intersection (no value is in every branch)"
            ),
        ));
    }
    Ok(Value::Array(out))
}

/// Merges two `dependentRequired` maps: per trigger key, the union of the
/// dependent-name lists.
fn merge_dependent_required(acc: &Value, branch: &Value) -> Result<Value> {
    let mut out = acc.as_object().cloned().unwrap_or_default();
    if let Some(branch_map) = branch.as_object() {
        for (trigger, deps) in branch_map {
            let entry = out
                .entry(trigger.clone())
                .or_insert_with(|| Value::Array(Vec::new()));
            if let (Some(existing), Some(new_deps)) = (entry.as_array_mut(), deps.as_array()) {
                for dep in new_deps {
                    if !existing.contains(dep) {
                        existing.push(dep.clone());
                    }
                }
            }
        }
    }
    Ok(Value::Object(out))
}

/// Finalizes a merged schema: collapses a cross-keyword numeric-bound pair
/// (`minimum`+`exclusiveMinimum` or `maximum`+`exclusiveMaximum`) that arrived
/// from different branches to the single tighter bound, and resolves a
/// `const`+`enum` combination (the `const` must be a member; the `enum` is
/// dropped).
fn finalize_merged(path: &Path, schema: &mut Schema, context: &str) -> Result<()> {
    collapse_numeric_pair(schema, "minimum", "exclusiveMinimum", true);
    collapse_numeric_pair(schema, "maximum", "exclusiveMaximum", false);

    if schema.extra.contains_key("const") && schema.extra.contains_key("enum") {
        let const_value = schema.extra["const"].clone();
        let in_enum = schema.extra["enum"].as_array().is_some_and(|members| {
            members
                .iter()
                .any(|member| json_values_equal(member, &const_value))
        });
        if !in_enum {
            return Err(merge_reject(
                path,
                format!(
                    "{context}: the merged `const` {const_value} is not a member of the merged `enum` set"
                ),
            ));
        }
        schema.extra.shift_remove("enum");
    }
    Ok(())
}

/// Collapses a same-axis inclusive/exclusive bound pair to the single bound that
/// admits the smaller set. For a lower bound (`is_lower`) keep `exclusiveMinimum`
/// iff its floor is `>=` the inclusive one; for an upper bound keep
/// `exclusiveMaximum` iff its ceiling is `<=` the inclusive one.
fn collapse_numeric_pair(schema: &mut Schema, inclusive: &str, exclusive: &str, is_lower: bool) {
    let inclusive_value = schema.extra.get(inclusive).and_then(Value::as_f64);
    let exclusive_value = schema.extra.get(exclusive).and_then(Value::as_f64);
    if let (Some(incl), Some(excl)) = (inclusive_value, exclusive_value) {
        let keep_exclusive = if is_lower { excl >= incl } else { excl <= incl };
        if keep_exclusive {
            schema.extra.shift_remove(inclusive);
        } else {
            schema.extra.shift_remove(exclusive);
        }
    }
}

/// Resolves a `$ref` string to the [`TypeKey`] it names (named-target / local-file
/// rules), independent of whether a model has been built for it yet.
fn resolve_ref_key(
    path: &Path,
    canonical_path: &Path,
    reference: &str,
    doc_paths: &BTreeSet<PathBuf>,
) -> Result<TypeKey> {
    let (file_part, pointer) = reference.split_once('#').unwrap_or((reference, ""));
    let target_path = if file_part.is_empty() {
        canonical_path.to_path_buf()
    } else {
        if Path::new(file_part).is_absolute() {
            return Err(Error::InvalidJsonSchema {
                path: path.to_path_buf(),
                reason: format!(
                    "absolute-path `$ref` `{reference}` is not supported; use a path relative to the referring schema"
                ),
            });
        }
        let base = path.parent().unwrap_or_else(|| Path::new("."));
        let target = canonical(&base.join(file_part));
        if !doc_paths.contains(&target) {
            return Err(Error::InvalidJsonSchema {
                path: path.to_path_buf(),
                reason: format!(
                    "`$ref` target file `{file_part}` resolved from `{}` to `{}`, which is not in the input set",
                    path.display(),
                    target.display()
                ),
            });
        }
        target
    };

    if pointer.is_empty() {
        Ok(TypeKey::Root(target_path))
    } else {
        if pointer == "/" {
            return Err(Error::InvalidJsonSchema {
                path: path.to_path_buf(),
                reason: format!(
                    "`$ref` `{reference}` uses `#/`, which points at a property with the empty name and is not the file root; use `#` for the file root"
                ),
            });
        }
        let Some(pointer) = pointer.strip_prefix('/') else {
            return Err(Error::InvalidJsonSchema {
                path: path.to_path_buf(),
                reason: format!(
                    "`$ref` `{reference}` must use a JSON Pointer to a `$defs` entry or file root"
                ),
            });
        };
        let tokens = pointer
            .split('/')
            .map(|token| decode_json_pointer_token(path, reference, token))
            .collect::<Result<Vec<_>>>()?;
        if tokens.len() < 2
            || tokens.len() % 2 != 0
            || tokens.iter().step_by(2).any(|keyword| keyword != "$defs")
        {
            return Err(Error::InvalidJsonSchema {
                path: path.to_path_buf(),
                reason: format!(
                    "`$ref` `{reference}` must point at a `$defs` entry or file root; extract the target into `$defs` and reference it there (nested targets follow a `$defs` chain such as `#/$defs/Outer/$defs/Inner`)"
                ),
            });
        }
        Ok(TypeKey::Def(
            target_path,
            tokens.into_iter().skip(1).step_by(2).collect(),
        ))
    }
}

fn decode_json_pointer_token(path: &Path, reference: &str, token: &str) -> Result<String> {
    let mut decoded = String::with_capacity(token.len());
    let mut chars = token.chars();
    while let Some(character) = chars.next() {
        if character != '~' {
            decoded.push(character);
            continue;
        }
        match chars.next() {
            Some('0') => decoded.push('~'),
            Some('1') => decoded.push('/'),
            Some(other) => {
                return Err(Error::InvalidJsonSchema {
                    path: path.to_path_buf(),
                    reason: format!(
                        "`$ref` `{reference}` contains the invalid RFC 6901 escape `~{other}`; use `~0` for `~` or `~1` for `/`"
                    ),
                });
            }
            None => {
                return Err(Error::InvalidJsonSchema {
                    path: path.to_path_buf(),
                    reason: format!(
                        "`$ref` `{reference}` contains a trailing `~`, which is an invalid RFC 6901 escape; use `~0` for `~` or `~1` for `/`"
                    ),
                });
            }
        }
    }
    Ok(decoded)
}

fn with_ref_schema_context(error: Error, context: &str) -> Error {
    match error {
        Error::InvalidJsonSchema { path, reason } => Error::InvalidJsonSchema {
            path,
            reason: format!("{context}: {reason}"),
        },
        other => other,
    }
}

fn resolve_ref_key_at(
    path: &Path,
    canonical_path: &Path,
    reference: &str,
    doc_paths: &BTreeSet<PathBuf>,
    context: &str,
) -> Result<TypeKey> {
    resolve_ref_key(path, canonical_path, reference, doc_paths)
        .map_err(|error| with_ref_schema_context(error, context))
}

fn resolve_ref_at<'a>(
    path: &Path,
    canonical_path: &Path,
    reference: &str,
    context: &str,
    docs: &IndexMap<PathBuf, (PathBuf, Document)>,
    models: &'a BTreeMap<TypeKey, JsonModel>,
) -> Result<&'a JsonModel> {
    let doc_paths: BTreeSet<PathBuf> = docs.keys().cloned().collect();
    let key = resolve_ref_key_at(path, canonical_path, reference, &doc_paths, context)?;
    if let Some(model) = models.get(&key) {
        return Ok(model);
    }
    let target_path = model_key_path(&key).expect("every ref key names a source path");
    let target_path = docs
        .get(target_path)
        .map(|(source_path, _)| source_path.as_path())
        .unwrap_or(target_path.as_path());
    let reason = unresolved_ref_reason(context, reference, &key, target_path);
    Err(Error::InvalidJsonSchema {
        path: path.to_path_buf(),
        reason,
    })
}

fn unresolved_ref_reason(
    context: &str,
    reference: &str,
    key: &TypeKey,
    target_path: &Path,
) -> String {
    match key {
        TypeKey::Root(_) => format!(
            "{context}: `$ref` `{reference}` targets the root of `{}`, but that file declares no root schema; add a root model or point the reference at one of the file's `$defs` entries",
            target_path.display(),
        ),
        TypeKey::Def(_, names) => {
            let chain = names
                .iter()
                .map(|name| format!("$defs.{name}"))
                .collect::<Vec<_>>()
                .join(".");
            format!(
                "{context}: `$ref` `{reference}` does not resolve because `{}` declares no `{chain}` entry; add that `$defs` entry or correct the JSON Pointer",
                target_path.display(),
            )
        }
    }
}

fn resolve_ref<'a>(
    path: &Path,
    canonical_path: &Path,
    reference: &str,
    docs: &IndexMap<PathBuf, (PathBuf, Document)>,
    models: &'a BTreeMap<TypeKey, JsonModel>,
) -> Result<&'a JsonModel> {
    let doc_paths: BTreeSet<PathBuf> = docs.keys().cloned().collect();
    let key = resolve_ref_key(path, canonical_path, reference, &doc_paths)?;
    models.get(&key).ok_or_else(|| Error::InvalidJsonSchema {
        path: path.to_path_buf(),
        reason: format!("`$ref` `{reference}` does not resolve to a known JSON model"),
    })
}

fn insert_json_external_type(
    external_types: &mut BTreeMap<String, ExternalTypeBindingSpec>,
    model: &JsonModel,
    docs: &IndexMap<PathBuf, (PathBuf, Document)>,
    models: &BTreeMap<TypeKey, JsonModel>,
    module_paths: Option<&BTreeMap<PathBuf, ModulePath>>,
) -> Result<()> {
    let type_spec = json_model_spec(model, docs, models, module_paths)?;
    // The map is keyed by the model's identity, and one model is reached from
    // several positions (its own collection pass, each `$ref` at it, an
    // operation's I/O), so re-inserting the *same* model is an ordinary no-op.
    // Two *different* schemas arriving under one identity would collapse into a
    // single emitted type — the loser's shape gone, every reference to it
    // silently retargeted at the winner — so reject instead (P7.1/P15). The
    // in-file cases are caught earlier with a fix-it that names the authored
    // positions; this is the backstop that keeps any other path from collapsing
    // silently.
    match external_types.entry(type_spec.name.as_str().to_string()) {
        btree_map::Entry::Occupied(existing) => {
            if let Some(previous) = existing.get().json_model()
                && (previous.model_name != type_spec.model_name
                    || previous.schema != type_spec.schema)
            {
                return Err(Error::InvalidJsonSchema {
                    path: model.canonical_path.clone(),
                    reason: format!(
                        "two different JSON schemas share the model identity `{}` (emitted as `{}` and `{}`); rename one of them so each schema has an identity of its own (P15 — the generator never auto-mangles)",
                        type_spec.name.as_str(),
                        previous.model_name,
                        type_spec.model_name,
                    ),
                });
            }
        }
        btree_map::Entry::Vacant(slot) => {
            slot.insert(ExternalTypeBindingSpec::JsonModel(JsonModelBindingSpec {
                model: type_spec,
                type_name: language_string(Some(model.model_name.clone())),
            }));
        }
    }
    Ok(())
}

fn json_model_type(
    model: &JsonModel,
    docs: &IndexMap<PathBuf, (PathBuf, Document)>,
    models: &BTreeMap<TypeKey, JsonModel>,
    module_paths: Option<&BTreeMap<PathBuf, ModulePath>>,
) -> Result<TypeSpec> {
    Ok(TypeSpec::External(ExternalTypeSpec::Json(json_model_spec(
        model,
        docs,
        models,
        module_paths,
    )?)))
}

fn json_model_spec(
    model: &JsonModel,
    docs: &IndexMap<PathBuf, (PathBuf, Document)>,
    models: &BTreeMap<TypeKey, JsonModel>,
    module_paths: Option<&BTreeMap<PathBuf, ModulePath>>,
) -> Result<JsonModelSpec<Symbol>> {
    let schema =
        resolve_schema_refs_for_generation(model, &model.schema, docs, models, module_paths)?;
    Ok(JsonModelSpec {
        name: json_model_symbol(model, module_paths),
        model_name: model.model_name.clone(),
        schema: serde_json::to_value(&schema).map_err(|error| Error::InvalidJsonSchema {
            path: PathBuf::from("<json-schema>"),
            reason: format!(
                "failed to preserve JSON schema model `{}`: {error}",
                model.full_name
            ),
        })?,
    })
}

fn json_model_key(
    model: &JsonModel,
    module_paths: Option<&BTreeMap<PathBuf, ModulePath>>,
) -> String {
    json_model_symbol(model, module_paths).as_str().to_string()
}

fn json_model_symbol(
    model: &JsonModel,
    module_paths: Option<&BTreeMap<PathBuf, ModulePath>>,
) -> Symbol {
    let Some(module_path) = module_paths.and_then(|paths| paths.get(&model.canonical_path)) else {
        return Symbol::new(model.full_name.clone());
    };
    let full_name = if module_path.is_root() {
        model.full_name.clone()
    } else {
        format!("{}#{}", module_path.as_module_key(), model.full_name)
    };
    Symbol::qualified(module_path.clone(), full_name, model.model_name.clone())
}

fn resolve_schema_refs_for_generation(
    owner: &JsonModel,
    schema: &Schema,
    docs: &IndexMap<PathBuf, (PathBuf, Document)>,
    models: &BTreeMap<TypeKey, JsonModel>,
    module_paths: Option<&BTreeMap<PathBuf, ModulePath>>,
) -> Result<Schema> {
    let mut resolved = schema.clone();
    if let Some(reference) = &schema.reference {
        let target = resolve_ref(
            &owner.canonical_path,
            &owner.canonical_path,
            reference,
            docs,
            models,
        )?;
        let target = json_model_symbol(target, module_paths);
        resolved.reference = Some(format!("#/$defs/{}", target.as_str()));
        return Ok(resolved);
    }
    if let Some(properties) = &schema.properties {
        resolved.properties = Some(
            properties
                .iter()
                .map(|(name, property)| {
                    Ok((
                        name.clone(),
                        resolve_schema_refs_for_generation(
                            owner,
                            property,
                            docs,
                            models,
                            module_paths,
                        )?,
                    ))
                })
                .collect::<Result<IndexMap<_, _>>>()?,
        );
    }
    if let Some(items) = &schema.items {
        resolved.items = Some(Box::new(resolve_schema_refs_for_generation(
            owner,
            items,
            docs,
            models,
            module_paths,
        )?));
    }
    if let Some(one_of) = &schema.one_of {
        resolved.one_of = Some(
            one_of
                .iter()
                .map(|branch| {
                    resolve_schema_refs_for_generation(owner, branch, docs, models, module_paths)
                })
                .collect::<Result<Vec<_>>>()?,
        );
    }
    if let Some(additional) = &schema.additional_properties
        && additional.is_object()
    {
        let additional_schema =
            serde_json::from_value::<Schema>(additional.clone()).map_err(|error| {
                Error::InvalidJsonSchema {
                    path: owner.canonical_path.clone(),
                    reason: format!("additionalProperties is invalid: {error}"),
                }
            })?;
        resolved.additional_properties = Some(
            serde_json::to_value(resolve_schema_refs_for_generation(
                owner,
                &additional_schema,
                docs,
                models,
                module_paths,
            )?)
            .map_err(|error| Error::InvalidJsonSchema {
                path: owner.canonical_path.clone(),
                reason: format!("failed to preserve additionalProperties: {error}"),
            })?,
        );
    }
    Ok(resolved)
}

fn language_string(default: Option<String>) -> LanguageStringSpec {
    LanguageStringSpec {
        default,
        ..LanguageStringSpec::default()
    }
}

/// A per-language code-identifier override (`x-<lang>-name`) as a
/// [`LanguageStringSpec`] carrying the value under `language` only. The
/// JSON-schema load is per emitted target, so at most one language is ever
/// populated; emitters read it back via `for_language(language)`.
fn language_string_override(language: Language, value: Option<String>) -> LanguageStringSpec {
    let mut spec = LanguageStringSpec::default();
    if let Some(value) = value {
        spec.by_language.insert(language, value);
    }
    spec
}

fn root_is_schema_shaped(root: &Schema) -> bool {
    // A definitions-only document may carry `description` without declaring a
    // root model. Every other schema keyword, including one stored in `extra`
    // (`allOf`, `enum`, constraints, annotations), makes the root a schema.
    Schema {
        description: None,
        ..root.clone()
    } != Schema::default()
}

fn root_type_name(path: &Path) -> String {
    path.file_name()
        .map(|value| strip_json_schema_extension(&value.to_string_lossy()).to_string())
        .unwrap_or_else(|| "Root".to_string())
}

/// The type name a file's root schema derives: its base name, recased (see
/// `specs/json-schema/features/ref.md` §"Type-name derivation"). The single
/// source of the root model's identity — model collection, the hoist collision
/// check, and the root-vs-`$defs` collision check all read it from here.
fn root_model_name(path: &Path) -> String {
    root_type_name(path).to_upper_camel_case()
}

/// The input file's name as authored, for a diagnostic that has to explain that a
/// name was derived from it.
fn root_file_name(path: &Path) -> String {
    path.file_name()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string())
}

fn canonical(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| normalize(path))
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                if out.file_name().is_some() {
                    out.pop();
                } else if !out.has_root() {
                    out.push("..");
                }
            }
            std::path::Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// P15 identifier namespace + `x-<lang>-name` override escape hatch.
//
// See specs/json-schema/features/properties.md (Stage 1-4 + the override), and
// PRINCIPLES.md P15 (one identifier namespace per scope; synthesized-name
// collisions reject at load, never mangle; the escape hatch is the override).
//
// The load is per emitted target (`language`), so a coincidence that collides
// in one language may be fine in another; the pass runs once per language.
// ---------------------------------------------------------------------------

/// The `x-<lang>-name` extension keyword for a target, or `None` for a language
/// with no JSON identifier policy (Dotnet/Ruby are not part of the P15 subset).
fn lang_name_keyword(language: Language) -> Option<&'static str> {
    match language {
        Language::Go => Some("x-go-name"),
        Language::TypeScript => Some("x-ts-name"),
        Language::Python => Some("x-py-name"),
        Language::Java => Some("x-java-name"),
        _ => None,
    }
}

fn lang_const_name_keyword(language: Language) -> Option<&'static str> {
    match language {
        Language::Go => Some("x-go-const-name"),
        Language::Java => Some("x-java-const-name"),
        _ => None,
    }
}

fn lang_enum_names_keyword(language: Language) -> Option<&'static str> {
    match language {
        Language::Go => Some("x-go-enum-names"),
        Language::Java => Some("x-java-enum-names"),
        _ => None,
    }
}

/// The `x-<lang>-name` override on a schema node for the given target, if any.
fn override_name<'a>(language: Language, schema: &'a Schema) -> Option<&'a str> {
    let keyword = lang_name_keyword(language)?;
    schema.extra.get(keyword).and_then(Value::as_str)
}

/// A syntactically legal identifier for every supported target: non-empty, first
/// char an ASCII letter or `_`, remaining chars ASCII alphanumeric or `_`.
fn ident_is_syntactically_valid(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first == '_' || first.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

/// Whether `name` is a reserved word in the target language.
fn ident_is_reserved(language: Language, name: &str) -> bool {
    match language {
        Language::Go => matches!(
            name,
            "break"
                | "case"
                | "chan"
                | "const"
                | "continue"
                | "default"
                | "defer"
                | "else"
                | "fallthrough"
                | "for"
                | "func"
                | "go"
                | "goto"
                | "if"
                | "import"
                | "interface"
                | "map"
                | "package"
                | "range"
                | "return"
                | "select"
                | "struct"
                | "switch"
                | "type"
                | "var"
        ),
        Language::TypeScript => matches!(
            name,
            "break"
                | "case"
                | "catch"
                | "class"
                | "const"
                | "continue"
                | "debugger"
                | "default"
                | "delete"
                | "do"
                | "else"
                | "enum"
                | "export"
                | "extends"
                | "false"
                | "finally"
                | "for"
                | "function"
                | "if"
                | "import"
                | "in"
                | "instanceof"
                | "new"
                | "null"
                | "return"
                | "super"
                | "switch"
                | "this"
                | "throw"
                | "true"
                | "try"
                | "typeof"
                | "var"
                | "void"
                | "while"
                | "with"
                | "yield"
                | "as"
                | "implements"
                | "interface"
                | "let"
                | "package"
                | "private"
                | "protected"
                | "public"
                | "static"
        ),
        Language::Python => matches!(
            name,
            "False"
                | "None"
                | "True"
                | "and"
                | "as"
                | "assert"
                | "async"
                | "await"
                | "break"
                | "class"
                | "continue"
                | "def"
                | "del"
                | "elif"
                | "else"
                | "except"
                | "finally"
                | "for"
                | "from"
                | "global"
                | "if"
                | "import"
                | "in"
                | "is"
                | "lambda"
                | "nonlocal"
                | "not"
                | "or"
                | "pass"
                | "raise"
                | "return"
                | "try"
                | "while"
                | "with"
                | "yield"
                | "match"
                | "case"
        ),
        Language::Java => matches!(
            name,
            "abstract"
                | "assert"
                | "boolean"
                | "break"
                | "byte"
                | "case"
                | "catch"
                | "char"
                | "class"
                | "const"
                | "continue"
                | "default"
                | "do"
                | "double"
                | "else"
                | "enum"
                | "extends"
                | "final"
                | "finally"
                | "float"
                | "for"
                | "goto"
                | "if"
                | "implements"
                | "import"
                | "instanceof"
                | "int"
                | "interface"
                | "long"
                | "native"
                | "new"
                | "package"
                | "private"
                | "protected"
                | "public"
                | "return"
                | "short"
                | "static"
                | "strictfp"
                | "super"
                | "switch"
                | "synchronized"
                | "this"
                | "throw"
                | "throws"
                | "transient"
                | "try"
                | "void"
                | "volatile"
                | "while"
                | "true"
                | "false"
                | "null"
        ),
        _ => false,
    }
}

/// Recases a JSON member name to the target's idiomatic identifier (Stage 1-2),
/// mirroring each backend's field-name derivation.
fn recase_member(language: Language, json_name: &str) -> String {
    match language {
        Language::Go => {
            let camel = json_name.to_upper_camel_case();
            if ident_is_reserved(Language::Go, &camel) {
                format!("{camel}_")
            } else {
                camel
            }
        }
        Language::TypeScript => {
            let camel = json_name.to_lower_camel_case();
            if ident_is_reserved(Language::TypeScript, &camel) {
                format!("{camel}_")
            } else {
                camel
            }
        }
        Language::Python => {
            let snake = json_name.to_snake_case();
            if ident_is_reserved(Language::Python, &snake) {
                format!("{snake}_")
            } else {
                snake
            }
        }
        Language::Java => json_name.to_lower_camel_case(),
        _ => json_name.to_string(),
    }
}

/// If a JSON member name recases to an identifier that cannot be emitted as-is
/// in `language` — syntactically invalid (e.g. a leading digit) or a reserved
/// word — returns the offending recased identifier and a short reason. P15
/// forbids auto-mangling, so such a member must carry an `x-<lang>-name`
/// override; returns `None` when the recased name is directly usable.
fn member_identifier_defect(language: Language, json_name: &str) -> Option<(String, &'static str)> {
    let base = match language {
        Language::Go => json_name.to_upper_camel_case(),
        Language::TypeScript | Language::Java => json_name.to_lower_camel_case(),
        Language::Python => json_name.to_snake_case(),
        _ => return None,
    };
    if !ident_is_syntactically_valid(&base) {
        return Some((base, "is not a valid identifier"));
    }
    if ident_is_reserved(language, &base) {
        return Some((base, "is a reserved word"));
    }
    None
}

/// The emitted member identifier for a property: the `x-<lang>-name` override if
/// present, otherwise the recased JSON name.
fn member_identifier(language: Language, json_name: &str, property: &Schema) -> String {
    override_name(language, property)
        .map(str::to_string)
        .unwrap_or_else(|| recase_member(language, json_name))
}

/// The emitted type identifier for a model: the type-level `x-<lang>-name`
/// override used verbatim if present, otherwise the derived `model_name`. This
/// is the single resolution point — the manifest, the collision key, and (via
/// the manifest) the generators all agree on this identifier.
fn type_identifier(language: Language, model_name: &str, schema: &Schema) -> String {
    override_name(language, schema)
        .map(str::to_string)
        .unwrap_or_else(|| model_name.to_string())
}

/// The TypeScript identifier of a model's `TransferTypeConverter` instance,
/// derived from the model's resolved type identifier. This is the single owner of
/// the name: the P15 collision pass enters it into the module namespace here and
/// the TypeScript emitters (model declaration, cross-module value imports,
/// operation `inputType`/`outputType`) ask for it, so the derivation is never
/// spelled twice and the check can never drift from emission.
pub(crate) fn ts_transfer_type_converter_name(type_ident: &str) -> String {
    format!("{}TransferTypeConverter", type_ident.to_lower_camel_case())
}

/// Whether a property schema is a scalar closed value set (`const`/`enum`) that
/// synthesizes a Go defined type + value constants / Java value constants.
fn schema_closed_values(schema: &Schema) -> Vec<Value> {
    if let Some(value) = schema.extra.get("const") {
        vec![value.clone()]
    } else if let Some(Value::Array(values)) = schema.extra.get("enum") {
        values.clone()
    } else {
        Vec::new()
    }
}

/// The verbatim value-constant override for a `const`/`enum` value, if the
/// schema carries one: `x-<lang>-const-name` replaces the single `const`'s
/// constant, and an `x-<lang>-enum-names` entry (keyed by the wire value's
/// string form) replaces an enum member's constant. Mirrors
/// `go_value_constant_override` in `src/generator/json/go.rs` so the P15
/// collision pass and emission agree — keep the lookups identical (const gates
/// on `const`, else enum by its canonical value key).
fn value_constant_override<'a>(
    language: Language,
    schema: &'a Schema,
    value: &Value,
) -> Option<&'a str> {
    if schema.extra.contains_key("const") {
        let keyword = lang_const_name_keyword(language)?;
        schema.extra.get(keyword).and_then(Value::as_str)
    } else if let (Some(keyword), Some(key)) = (
        lang_enum_names_keyword(language),
        enum_names_lookup_key(value),
    ) {
        schema
            .extra
            .get(keyword)
            .and_then(Value::as_object)
            .and_then(|map| map.get(&key))
            .and_then(Value::as_str)
    } else {
        None
    }
}

/// The `x-<lang>-enum-names` map key for one closed value: the string itself,
/// the canonical shortest decimal for a number, or a boolean's JSON spelling.
///
/// All three lookups — this one and the Go/Java emitters' — used to match only
/// `Value::String`, so a numeric or boolean member could never be renamed: the
/// one escape hatch P15 offers for a token collision between, say, `1` and `1.0`
/// did not exist for the very values most likely to collide.
pub(crate) fn enum_names_lookup_key(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Bool(flag) => Some(flag.to_string()),
        Value::Number(number) => Some(crate::json_schema::scalar::value_token_decimal(number)),
        _ => None,
    }
}

/// The Go value-constant suffix for a scalar value (mirrors `go_value_suffix`).
fn go_value_suffix_for(value: &Value) -> String {
    match value {
        Value::String(text) => text.to_upper_camel_case(),
        Value::Bool(flag) => if *flag { "True" } else { "False" }.to_string(),
        Value::Number(number) => crate::json_schema::scalar::value_token_decimal(number)
            .replace('-', "Neg")
            .replace('.', "_"),
        _ => String::new(),
    }
}

/// Validates an `x-<lang>-name` / `x-<lang>-const-name` override string: it must
/// be a legal, non-reserved identifier in the target language.
fn validate_override(
    language: Language,
    keyword: &str,
    value: &Value,
    context: &str,
) -> Result<()> {
    let Some(text) = value.as_str() else {
        return Err(Error::InvalidJsonSchema {
            path: PathBuf::from("<json-schema>"),
            reason: format!("{context}: `{keyword}` must be a string identifier"),
        });
    };
    if !ident_is_syntactically_valid(text) || ident_is_reserved(language, text) {
        return Err(Error::InvalidJsonSchema {
            path: PathBuf::from("<json-schema>"),
            reason: format!(
                "{context}: `{keyword}` value {text:?} is not a legal, non-reserved {} identifier",
                language.as_str()
            ),
        });
    }
    Ok(())
}

/// Validates every `x-<lang>-*` override reachable in a schema subtree (for the
/// active target only): `x-<lang>-name` on any node, and
/// `x-<lang>-const-name` / `x-<lang>-enum-names` on a `const`/`enum` node.
fn validate_overrides_in_schema(language: Language, schema: &Schema, context: &str) -> Result<()> {
    if let Some(keyword) = lang_name_keyword(language)
        && let Some(value) = schema.extra.get(keyword)
    {
        validate_override(language, keyword, value, context)?;
    }
    if let Some(keyword) = lang_const_name_keyword(language)
        && let Some(value) = schema.extra.get(keyword)
    {
        if !schema.extra.contains_key("const") {
            return Err(Error::InvalidJsonSchema {
                path: PathBuf::from("<json-schema>"),
                reason: format!("{context}: `{keyword}` is only valid beside `const`"),
            });
        }
        validate_override(language, keyword, value, context)?;
    }
    if let Some(keyword) = lang_enum_names_keyword(language)
        && let Some(value) = schema.extra.get(keyword)
    {
        if !schema.extra.contains_key("enum") {
            return Err(Error::InvalidJsonSchema {
                path: PathBuf::from("<json-schema>"),
                reason: format!("{context}: `{keyword}` is only valid beside `enum`"),
            });
        }
        let Some(map) = value.as_object() else {
            return Err(Error::InvalidJsonSchema {
                path: PathBuf::from("<json-schema>"),
                reason: format!("{context}: `{keyword}` must be a map of value to identifier"),
            });
        };
        for entry in map.values() {
            validate_override(language, keyword, entry, context)?;
        }
        let member_keys = schema
            .extra
            .get("enum")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(enum_names_lookup_key)
            .collect::<BTreeSet<_>>();
        for key in map.keys() {
            if !member_keys.contains(key) {
                return Err(Error::InvalidJsonSchema {
                    path: PathBuf::from("<json-schema>"),
                    reason: format!(
                        "{context}: `{keyword}` key {key:?} does not name an `enum` member; use the member's canonical JSON spelling"
                    ),
                });
            }
        }
    }
    if let Some(properties) = &schema.properties {
        for (name, property) in properties {
            validate_overrides_in_schema(
                language,
                property,
                &format!("{context}.properties.{name}"),
            )?;
        }
    }
    if let Some(items) = &schema.items {
        validate_overrides_in_schema(language, items, &format!("{context}.items"))?;
    }
    if let Some(one_of) = &schema.one_of {
        for (index, branch) in one_of.iter().enumerate() {
            validate_overrides_in_schema(language, branch, &format!("{context}.oneOf[{index}]"))?;
        }
    }
    Ok(())
}

/// A single per-scope identifier namespace: inserting a name that is already
/// held (by a different origin) is a P15 collision → load reject.
#[derive(Default)]
struct Namespace {
    entries: BTreeMap<String, String>,
}

impl Namespace {
    /// For a scope whose names are member-derived, so `x-<lang>-name` is the
    /// remedy the diagnostic should name.
    fn insert(&mut self, language: Language, ident: String, origin: String) -> Result<()> {
        let remedy = lang_name_keyword(language).unwrap_or("x-<lang>-name");
        self.insert_with_remedy(language, ident, origin, remedy)
    }

    /// Same, for a scope the member-name override cannot reach. A **value
    /// constant** is named from the value, not the member, so `x-<lang>-name`
    /// would not move it and offering it is the lying fix-it P15 forbids; the
    /// remedy is `x-<lang>-const-name` / `x-<lang>-enum-names`.
    fn insert_with_remedy(
        &mut self,
        language: Language,
        ident: String,
        origin: String,
        remedy: &str,
    ) -> Result<()> {
        if let Some(previous) = self.entries.get(&ident)
            && previous != &origin
        {
            return Err(Error::InvalidJsonSchema {
                path: PathBuf::from("<json-schema>"),
                reason: format!(
                    "identifier collision in {} output: {previous} and {origin} both map to `{ident}`; disambiguate with an `{remedy}` override (P15 — the generator never auto-mangles)",
                    language.as_str(),
                ),
            });
        }
        self.entries.insert(ident, origin);
        Ok(())
    }
}

/// A JSON model with its emitted type identifier + decoded schema, grouped by
/// module (a module is one scope for the nesting languages; the single-input
/// path groups everything together).
struct NsModel {
    module_key: String,
    full_name: String,
    type_ident: String,
    schema: Schema,
    imports_java_offset_date_time: bool,
}

/// Resolved emitted-name manifest for one target language. Built once by
/// [`build_name_manifest`] and consumed by both the load-time collision pass
/// and the generators, so every identifier that will be emitted is resolved in
/// exactly one place — no drift between the collision check and emission.
#[derive(Debug, Default, Clone)]
pub(crate) struct NameManifest {
    /// Model full name (`Symbol::as_str` / `PlannedJsonType::full_name`) →
    /// emitted type identifier. (Service identifiers are resolved onto
    /// `ServiceSpec::code_name` at load and read from there; the manifest only
    /// enters them into the collision pass, so it needs no service map.)
    type_names: BTreeMap<String, String>,
    /// Model full name to its emitted module/package scope. This lets consumers
    /// distinguish a bare binding from a qualified cross-module reference.
    type_modules: BTreeMap<String, String>,
}

impl NameManifest {
    /// The emitted type identifier for a model, keyed by its full name (the
    /// stable identity string that also appears in resolved `$ref`s). Returns
    /// `None` for a target with no JSON identifier policy or an unknown model.
    pub(crate) fn type_name(&self, full_name: &str) -> Option<&str> {
        self.type_names.get(full_name).map(String::as_str)
    }

    fn type_module(&self, full_name: &str) -> Option<&str> {
        self.type_modules.get(full_name).map(String::as_str)
    }
}

/// One model handed to [`build_name_manifest`], adapted from either the authored
/// [`ApiSpec`] (load path) or the planned spec (generator path).
pub(crate) struct ManifestModel {
    /// Resolution + generator lookup key (`Symbol::as_str`).
    pub(crate) full_name: String,
    /// Unqualified name, for collision diagnostics.
    pub(crate) local_name: String,
    /// The derived emitted identifier before any override is applied.
    pub(crate) model_name: String,
    /// The scope (package/module) the model lives in.
    pub(crate) module_key: String,
    /// The raw model schema (carries any `x-<lang>-*` overrides).
    pub(crate) schema: Value,
}

/// One service handed to [`build_name_manifest`]. Services live in the root
/// module scope of the file that declares them (the single-input scope).
pub(crate) struct ManifestService {
    pub(crate) name: String,
    /// The verbatim per-language service code-identifier override
    /// (`x-<lang>-name`), if the active target carries one. `None` derives from
    /// `name`.
    pub(crate) code_name: Option<String>,
    /// The module the declaring file emits into — the scope this service's
    /// identifier occupies. Empty for the single-input root.
    pub(crate) module_key: String,
    /// Full model identities referenced by this service's operation I/O. Only
    /// these model names enter the generated service file and can shadow an SDK
    /// import there; unrelated `$defs` remain in their model files.
    pub(crate) io_type_refs: BTreeSet<String>,
}

impl ManifestService {
    /// The emitted service code identifier for `language`: the verbatim override
    /// when present, else the derived name.
    ///
    /// TypeScript binds a service to a lower-camel `const` (`chatService`), not a
    /// type name, so it derives through the member pipeline; Go's `var`, Python's
    /// `class`, and Java's class all carry the name as authored. Deriving all four
    /// as type names claimed a TypeScript service collided with a same-named
    /// model, which it never can — the emitted identifiers differ in case.
    fn code_ident(&self, language: Language) -> String {
        self.code_name.clone().unwrap_or_else(|| match language {
            Language::TypeScript => recase_member(Language::TypeScript, &self.name),
            _ => recase_type_name(language, &self.name),
        })
    }

    /// How this service is named in a collision diagnostic. The module qualifier
    /// matters in Go, whose scope spans every module: two same-named services in
    /// different modules are a real clash, and identical origin text would make
    /// them read as one declaration seen twice.
    fn origin_label(&self) -> String {
        if self.module_key.is_empty() {
            format!("service `{}`", self.name)
        } else {
            format!("service `{}` in module `{}`", self.name, self.module_key)
        }
    }
}

/// Builds the [`NameManifest`] for `language`: runs the P15 per-scope collision
/// pass (load reject on any coincidence, never mangling) and records the
/// resolved identifier for every model and service. Runs once per emitted
/// target. This is the single place emitted names are resolved — the load-time
/// check and the generators both go through it.
pub(crate) fn build_name_manifest(
    language: Language,
    models: &[ManifestModel],
    services: &[ManifestService],
) -> Result<NameManifest> {
    let mut manifest = NameManifest::default();
    // A target with no JSON identifier policy (Dotnet/Ruby) does not participate
    // in the P15 collision pass, but still gets identity resolution so a
    // generator can query the manifest uniformly.
    let has_policy = lang_name_keyword(language).is_some();

    let mut ns_models: Vec<NsModel> = Vec::with_capacity(models.len());
    for model in models {
        let schema: Schema = serde_json::from_value(model.schema.clone()).map_err(|error| {
            Error::InvalidJsonSchema {
                path: PathBuf::from("<json-schema>"),
                reason: format!(
                    "failed to decode JSON model `{}` for the name manifest: {error}",
                    model.full_name
                ),
            }
        })?;
        if has_policy {
            validate_overrides_in_schema(language, &schema, &format!("`{}`", model.local_name))?;
        }
        let type_ident = type_identifier(language, &model.model_name, &schema);
        manifest
            .type_names
            .insert(model.full_name.clone(), type_ident.clone());
        manifest
            .type_modules
            .insert(model.full_name.clone(), model.module_key.clone());
        ns_models.push(NsModel {
            module_key: model.module_key.clone(),
            full_name: model.full_name.clone(),
            type_ident,
            schema,
            imports_java_offset_date_time: language == Language::Java
                && java::schema_imports_offset_date_time(&model.schema),
        });
    }

    if !has_policy {
        return Ok(manifest);
    }

    // Each emitted scope gets its own top-level namespace. Which scope that is
    // depends on how the target resolves a name across the emitted file set, so
    // it is a property of the generator's layout rather than of the schema:
    //
    // - **Go, TypeScript, Python** resolve run-wide, so `None` below means "every
    //   module at once". Go flattens every module into a single package, so two
    //   same-named types in different modules are plain redeclarations. TS and
    //   Python do keep a namespace per module, but each emits a root barrel that
    //   re-exports every module's top-level names into one namespace — `index.ts`
    //   with `export *` per module, and `__init__.py` with named re-exports — so a
    //   name emitted twice collides there. TypeScript rejects the barrel (TS2308,
    //   "has already exported a member named ..."); Python silently binds whichever
    //   import runs last, which is exactly the silent incorrectness P7 forbids.
    // - **Java and .NET** resolve per module: each module lands in its own
    //   sub-package/namespace (`com.example.api.content.page`,
    //   `Nexgen.Generated.Content.Page`) and neither emits an aggregating barrel,
    //   so the same type name in two modules is two distinct qualified names.
    //
    // A module with services but no models still has a scope, so its service
    // identifiers are checked against the boilerplate.
    let module_keys: BTreeSet<String> = ns_models
        .iter()
        .map(|model| model.module_key.clone())
        .chain(services.iter().map(|service| service.module_key.clone()))
        .collect();
    let scopes: Vec<Option<String>> = if scope_is_run_wide(language) {
        vec![None]
    } else {
        module_keys.iter().cloned().map(Some).collect()
    };
    for scope in &scopes {
        let in_scope = |key: &str| scope.as_deref().is_none_or(|scope| scope == key);
        let mut top = Namespace::default();
        for model in ns_models.iter().filter(|model| in_scope(&model.module_key)) {
            top.insert(
                language,
                model.type_ident.clone(),
                format!("type `{}`", model.full_name),
            )?;
            collect_synthesized_top_level(
                language,
                model.full_name.as_str(),
                &model.type_ident,
                &model.schema,
                model.imports_java_offset_date_time,
                &mut top,
            )?;
            validate_member_scope(language, model.full_name.as_str(), &model.schema)?;
        }
        if language == Language::Go {
            let mut helpers = BTreeSet::new();
            for model in ns_models.iter().filter(|model| in_scope(&model.module_key)) {
                collect_go_semantic_helper_idents(&model.schema, false, &mut helpers);
            }
            for ident in helpers {
                top.insert(
                    language,
                    ident.clone(),
                    format!("generated semantic predicate `{ident}`"),
                )?;
            }
        }
        // The fixed runtime boilerplate each generator emits into (or imports
        // into) every module that carries models shares this top-level scope, so
        // a user type/service named after one is a P15 clash — reject it at load
        // rather than emit code that won't compile. Inserted after the user
        // types so the diagnostic names the user identifier as the prior origin.
        for ident in boilerplate_idents(language) {
            top.insert(
                language,
                (*ident).to_string(),
                format!("generated runtime identifier `{ident}`"),
            )?;
        }
        // A service's bindings live in the module scope of the file that declares
        // it — which is the root module only in single-input mode. Keying the
        // insert on an empty module key meant that in multi-input mode services
        // never entered the pass at all, so a service clashing with a model in its
        // own module generated uncompilable code without a diagnostic.
        for service in services
            .iter()
            .filter(|service| in_scope(&service.module_key))
        {
            let service_ident = service.code_ident(language);
            top.insert(language, service_ident.clone(), service.origin_label())?;
            // Go's native API mode adds two package declarations whose spelling
            // is fixed by the service identifier. The loader deliberately has
            // one mode-independent accept set, so reserve them even when this
            // invocation will not render the optional native client.
            if language == Language::Go {
                let client_ident = format!("{service_ident}Client");
                top.insert(
                    language,
                    client_ident.clone(),
                    format!("{} native client", service.origin_label()),
                )?;
                top.insert(
                    language,
                    format!("New{client_ident}"),
                    format!("{} native client constructor", service.origin_label()),
                )?;
            }
        }
        // TypeScript `DEFAULT_<FIELD>` constants share the module scope; make
        // them participate rather than silently coexist (P15). Python surfaces
        // defaults through properties and emits no module-level constant.
        if language == Language::TypeScript {
            collect_default_constants(
                language,
                ns_models.iter().filter(|model| in_scope(&model.module_key)),
                &mut top,
            )?;
        }
        // TypeScript additionally emits `<FIELD>_CONST` bindings and a per-model
        // transfer type converter into that same module scope.
        if language == Language::TypeScript {
            collect_ts_transfer_type_converters(
                ns_models.iter().filter(|model| in_scope(&model.module_key)),
                &mut top,
            )?;
        }
    }

    // TypeScript and Python have two simultaneous scopes: their exported names
    // meet again in the root barrel (the loop above), while private bindings and
    // bare imports remain local to each emitted module. Run a second pass with a
    // fresh namespace per real module so a non-root module is checked without
    // spuriously making private names in unrelated modules collide.
    if matches!(language, Language::TypeScript | Language::Python) {
        for module_key in &module_keys {
            let group = ns_models
                .iter()
                .filter(|model| &model.module_key == module_key)
                .collect::<Vec<_>>();
            let mut module = Namespace::default();
            for model in &group {
                module.insert(
                    language,
                    model.type_ident.clone(),
                    format!("type `{}`", model.full_name),
                )?;
            }
            for ident in boilerplate_idents(language) {
                module.insert(
                    language,
                    (*ident).to_string(),
                    format!("generated runtime identifier `{ident}`"),
                )?;
            }
            for service in services
                .iter()
                .filter(|service| &service.module_key == module_key)
            {
                module.insert(
                    language,
                    service.code_ident(language),
                    service.origin_label(),
                )?;
            }
            if language == Language::TypeScript {
                collect_default_constants(language, group.iter().copied(), &mut module)?;
                collect_ts_const_constants(group.iter().copied(), &mut module)?;
                collect_ts_transfer_type_converters(group.iter().copied(), &mut module)?;
                collect_ts_inline_union_serializers(group.iter().copied(), &mut module)?;
            } else {
                collect_python_module_idents(group.iter().copied(), &mut module)?;
            }
        }
    }

    validate_service_file_scopes(language, services, &manifest)?;

    Ok(manifest)
}

/// TypeScript inline unions whose wire conversion is not the identity emit a
/// module-private `serialize<Model><Member>` helper. The member half follows
/// `x-ts-name`, and the helpers share one module scope with each other.
fn collect_ts_inline_union_serializers<'a>(
    models: impl Iterator<Item = &'a NsModel>,
    top: &mut Namespace,
) -> Result<()> {
    let models = models.collect::<Vec<_>>();
    for model in &models {
        let Some(properties) = &model.schema.properties else {
            continue;
        };
        for (json_name, property) in properties {
            let Some(branches) = &property.one_of else {
                continue;
            };
            // The emitter synthesizes a helper only for a referenced object
            // branch or an array branch whose element mapper changes the wire
            // value. The shared target predicate deliberately does not treat
            // assertion-only formats (for example `email`) as transforms.
            let needs_helper = branches.iter().any(|branch| {
                branch.reference.as_ref().is_some_and(|reference| {
                    let reference = reference.trim_start_matches('.');
                    models
                        .iter()
                        .find(|candidate| candidate.full_name == reference)
                        .map(|candidate| {
                            candidate.schema.ty.as_ref().and_then(Value::as_str) == Some("object")
                        })
                        .unwrap_or(true)
                }) || (branch.ty.as_ref().and_then(Value::as_str) == Some("array")
                    && typescript::schema_serializes_non_identity(
                        &serde_json::to_value(branch)
                            .expect("validated JSON Schema re-serializes for TypeScript planning"),
                        None,
                    ))
            });
            if !needs_helper {
                continue;
            }
            let member = member_identifier(Language::TypeScript, json_name, property);
            top.insert(
                Language::TypeScript,
                format!(
                    "serialize{}{}",
                    model.type_ident,
                    member.to_upper_camel_case()
                ),
                format!("`{}.{json_name}` inline union serializer", model.full_name),
            )?;
        }
    }
    Ok(())
}

/// Validate the identifiers that actually coexist in a generated service file.
/// SDK imports do not occupy a model file merely because the same input module
/// declares a service. They collide only with the service declarations and the
/// operation I/O model names that the service file itself references.
fn validate_service_file_scopes(
    language: Language,
    services: &[ManifestService],
    manifest: &NameManifest,
) -> Result<()> {
    let validate = |group: &[&ManifestService]| -> Result<()> {
        let mut scope = Namespace::default();
        for ident in service_import_idents(language) {
            scope.insert(
                language,
                (*ident).to_string(),
                format!("generated service-file import `{ident}`"),
            )?;
        }
        for service in group {
            scope.insert(
                language,
                service.code_ident(language),
                service.origin_label(),
            )?;
            for reference in &service.io_type_refs {
                let Some(type_ident) = manifest.type_name(reference) else {
                    continue;
                };
                if language == Language::Java
                    && manifest.type_module(reference) != Some(service.module_key.as_str())
                {
                    // Java may qualify a model from another package. Its simple
                    // name therefore does not enter the declaring package's
                    // type/service scope, and two foreign packages may both
                    // contribute (for example) a `Page` operation type.
                    // SDK imports remain reserved below through the explicit
                    // check, because P15 intentionally rejects an I/O model
                    // named `Operation`/`Service` instead of qualifying around
                    // that public collision.
                    let mut imports = Namespace::default();
                    for imported in service_import_idents(language) {
                        imports.insert(
                            language,
                            (*imported).to_string(),
                            format!("generated service-file import `{imported}`"),
                        )?;
                    }
                    imports.insert(
                        language,
                        type_ident.to_string(),
                        format!("operation I/O type `{reference}`"),
                    )?;
                    continue;
                }
                scope.insert(
                    language,
                    type_ident.to_string(),
                    format!("operation I/O type `{reference}`"),
                )?;
            }
        }
        Ok(())
    };

    if language == Language::Java {
        // Java emits one compilation unit per service interface.
        for service in services {
            validate(&[service])?;
        }
    } else {
        // Go, TypeScript, and Python group a module's service declarations in a
        // single generated file.
        let module_keys = services
            .iter()
            .map(|service| service.module_key.as_str())
            .collect::<BTreeSet<_>>();
        for module_key in module_keys {
            let group = services
                .iter()
                .filter(|service| service.module_key == module_key)
                .collect::<Vec<_>>();
            validate(&group)?;
        }
    }
    Ok(())
}

/// The fixed (schema-independent) top-level identifiers a target's JSON runtime
/// emits into — or imports into — every module that carries models, and which
/// therefore share the user type/service namespace. Only identifiers in the
/// same case-class as user identifiers (which are normally `UpperCamelCase`) are
/// listed.
///
/// - Go (`src/generator/json/go.rs`): the exported runtime type `Violation`
///   lives in the models' own package; every other runtime
///   symbol is unexported (`addViolations`, `parseSpecInteger`, …) and cannot
///   collide with an exported user type.
/// - TypeScript (`src/generator/json/typescript.rs`): nexus-rpc's
///   `TransferTypeConverter` is a bare named import in every model module (the
///   contract each model's converter implements), so a user type of that name is
///   an import-versus-local-declaration conflict. `Violation` reaches
///   `models.ts` only through the namespace import `__nexgenDefinitions`, but
///   the package barrel re-exports it from `./definitions` beside `export *` of
///   the model modules, so a user binding of the same name is silently shadowed
///   out of the package surface (P7). Runtime helpers (`isPlainObject`,
///   `collect`, …) remain internal to `definitions.ts` and are not re-exported
///   from the package barrel.
/// - Python (`src/generator/json/python.rs`): `Violation` (dataclass) is imported
///   by bare name into every model module and re-exported by the root package
///   barrel; the other runtime helpers
///   are `_`-prefixed.
/// - Java (`src/generator/java.rs`): the root-package runtime classes
///   `Violation`, `SpecNumbers`, `TemporalSupport`, and `Base64Support`. The
///   latter two are emitted only when used, but remain reserved so adding a
///   format or content encoding cannot retroactively invalidate another model's
///   name. The SDK's `ApplicationFailure` is also imported into model files.
fn boilerplate_idents(language: Language) -> &'static [&'static str] {
    match language {
        Language::Go | Language::Python => &["Violation"],
        Language::TypeScript => &["Violation", "TransferTypeConverter"],
        Language::Java => &[
            "ApplicationFailure",
            "Violation",
            "SpecNumbers",
            "TemporalSupport",
            "Base64Support",
        ],
        _ => &[],
    }
}

/// Bare package/annotation identifiers imported by a generated service file.
/// These share that file's scope with the service binding itself.
fn service_import_idents(language: Language) -> &'static [&'static str] {
    match language {
        Language::Go | Language::TypeScript => &["nexus"],
        Language::Python => &["Operation", "service"],
        Language::Java => &["Operation", "Service"],
        _ => &[],
    }
}

/// Whether `language` resolves top-level names across the whole run rather than
/// per module — that is, whether two modules may each declare the same name.
///
/// This is a property of the emitted layout, not of the schema:
///
/// - Go flattens every module into one package, so a name emitted twice is a
///   redeclaration in that package.
/// - TypeScript and Python do emit a namespace per module, but both also emit a
///   root barrel (`index.ts` / `__init__.py`) that lifts every module's top-level
///   names into a single namespace, so a name emitted twice collides there.
/// - Java and .NET give each module its own sub-package/namespace and emit no
///   aggregating barrel, so the same name in two modules stays unambiguous.
const fn scope_is_run_wide(language: Language) -> bool {
    match language {
        Language::Go | Language::TypeScript | Language::Python => true,
        Language::Java | Language::Dotnet | Language::Ruby => false,
    }
}

/// Adapts an authored [`ApiSpec`] into [`build_name_manifest`] inputs for
/// `language` (which selects each service's per-language `code_name` override).
fn manifest_inputs_from_spec(
    language: Language,
    spec: &ApiSpec,
) -> (Vec<ManifestModel>, Vec<ManifestService>) {
    let mut models = Vec::new();
    for (_full_name, binding) in spec.external_types() {
        let Some(json) = binding.json_model() else {
            continue;
        };
        let module_key = json
            .name
            .module_path()
            .map(ModulePath::as_module_key)
            .unwrap_or_default();
        models.push(ManifestModel {
            full_name: json.name.as_str().to_string(),
            local_name: json.name.local_name().to_string(),
            model_name: json.model_name.clone(),
            module_key,
            schema: json.schema.clone(),
        });
    }
    let services = spec
        .services
        .iter()
        .map(|service| ManifestService {
            name: service.name.clone(),
            code_name: service.code_name.for_language(language).map(str::to_string),
            module_key: spec.module_path.as_module_key(),
            io_type_refs: service
                .operations
                .iter()
                .flat_map(|operation| [operation.input.as_ref(), operation.output.as_ref()])
                .flatten()
                .filter_map(TypeSpec::reference)
                .map(|reference| reference.trim_start_matches('.').to_string())
                .collect(),
        })
        .collect();
    (models, services)
}

/// The load-time P15 collision check: builds the manifest and discards it,
/// surfacing any collision as a load reject. Runs once per emitted target.
fn validate_identifier_namespace(language: Language, spec: &ApiSpec) -> Result<()> {
    let (models, services) = manifest_inputs_from_spec(language, spec);
    build_name_manifest(language, &models, &services)?;
    Ok(())
}

/// A service/type name is already `UpperCamelCase`; a target that lowercases
/// (none of the four for a type) would recase here. Type names are used
/// verbatim across all four targets.
fn recase_type_name(_language: Language, name: &str) -> String {
    name.to_string()
}

fn go_semantic_pattern_ident(pattern: &str) -> String {
    let mut name = String::from("_nexgenJsonSchemaPattern");
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in pattern.as_bytes() {
        name.push(HEX[(byte >> 4) as usize] as char);
        name.push(HEX[(byte & 0x0f) as usize] as char);
    }
    name
}

/// Mirrors the Go package-level semantic predicate census. These names are
/// schema-derived but package-scoped, so they participate in the same run-wide
/// P15 namespace as authored `x-go-name` bindings.
fn collect_go_semantic_helper_idents(
    schema: &Schema,
    matcher: bool,
    helpers: &mut BTreeSet<String>,
) {
    let is_string = schema.ty.as_ref().and_then(Value::as_str) == Some("string");
    if matcher || is_string {
        if let Some(pattern) = schema.extra.get("pattern").and_then(Value::as_str) {
            helpers.insert(go_semantic_pattern_ident(pattern));
        }
        if let Some(format) = schema.extra.get("format").and_then(Value::as_str)
            && crate::json_schema::format::check_for(format).is_some()
        {
            helpers.insert(format!(
                "_nexgenJsonSchema{}Format",
                recase_member(Language::Go, format)
            ));
        }
        if !matcher
            && let Some(encoding) = schema
                .extra
                .get("contentEncoding")
                .and_then(Value::as_str)
                .and_then(crate::json_schema::content_encoding::Encoding::from_name)
        {
            helpers.insert(format!(
                "_nexgenJsonSchema{}ContentEncoding",
                recase_member(Language::Go, encoding.name())
            ));
        }
    }
    if let Some(properties) = &schema.properties {
        for property in properties.values() {
            collect_go_semantic_helper_idents(property, false, helpers);
        }
    }
    if let Some(items) = &schema.items {
        collect_go_semantic_helper_idents(items, false, helpers);
    }
    if let Some(branches) = &schema.one_of {
        for branch in branches {
            collect_go_semantic_helper_idents(branch, false, helpers);
        }
    }
    if let Some(value) = &schema.additional_properties
        && value.is_object()
        && let Ok(value_schema) = serde_json::from_value::<Schema>(value.clone())
    {
        collect_go_semantic_helper_idents(&value_schema, false, helpers);
    }
    for (keyword, matcher) in [("propertyNames", false), ("contains", true)] {
        if let Some(value) = schema.extra.get(keyword)
            && let Ok(child) = serde_json::from_value::<Schema>(value.clone())
        {
            collect_go_semantic_helper_idents(&child, matcher, helpers);
        }
    }
}

/// Adds the package/module-scoped identifiers a model synthesizes to the
/// top-level namespace: Go const/enum defined types + value constants (Go is
/// flat and has no nested types, so these live at package scope).
fn collect_synthesized_top_level(
    language: Language,
    model_full_name: &str,
    type_ident: &str,
    schema: &Schema,
    imports_java_offset_date_time: bool,
    top: &mut Namespace,
) -> Result<()> {
    if language == Language::Java {
        return collect_java_nested_scope(
            model_full_name,
            type_ident,
            schema,
            imports_java_offset_date_time,
        );
    }
    if language != Language::Go {
        return Ok(());
    }
    // A named `$defs` union: the sealed interface *is* the model type, already
    // entered by the caller, so only the members it synthesizes are new.
    if is_sum_type_union(schema) {
        collect_go_union_top_level(
            &format!("`{model_full_name}` union"),
            type_ident,
            schema,
            top,
        )?;
    }
    let Some(properties) = &schema.properties else {
        return Ok(());
    };
    for (json_name, property) in properties {
        // An inline `oneOf` on a property is a union the *model* owns: the Go
        // emitter names its sealed interface `<Type><Member>` and hangs the
        // variant wrappers and dispatcher off that name. Two unions deriving one
        // name silently merged — one interface bound the other's branch set —
        // because nothing in this pass had ever seen them.
        if is_sum_type_union(property) {
            let union_ident = format!("{type_ident}{}", go_union_field_suffix(json_name, property));
            let origin = format!("`{model_full_name}.{json_name}` union");
            top.insert(language, union_ident.clone(), format!("{origin} interface"))?;
            collect_go_union_top_level(&origin, &union_ident, property, top)?;
        }
        let values = schema_closed_values(property);
        if values.is_empty() {
            continue;
        }
        // The Go closed-value defined type is `<Type><Member>` and each value
        // constant is `<definedType><valueSuffix>`. Both derive from the *emitted*
        // member identifier, so an `x-go-name` override moves them with the field
        // (P15) — and so this pass matches what the generator emits.
        let defined_type = format!(
            "{type_ident}{}",
            member_identifier(Language::Go, json_name, property)
        );
        top.insert(
            language,
            defined_type.clone(),
            format!("`{model_full_name}.{json_name}` closed-value type"),
        )?;
        let remedy = if values.len() == 1 {
            lang_const_name_keyword(language).unwrap_or("x-<lang>-const-name")
        } else {
            lang_enum_names_keyword(language).unwrap_or("x-<lang>-enum-names")
        };
        for value in &values {
            // An `x-go-const-name` / `x-go-enum-names` override replaces the
            // whole value-constant identifier verbatim (mirrors the generator).
            let const_ident = match value_constant_override(language, property, value) {
                Some(name) => name.to_string(),
                None => {
                    let suffix = go_value_suffix_for(value);
                    if suffix.is_empty() {
                        return Err(Error::InvalidJsonSchema {
                            path: PathBuf::from("<json-schema>"),
                            reason: format!(
                                "`{model_full_name}.{json_name}` value {value} does not encode to a legal Go constant identifier; provide an `{remedy}` override"
                            ),
                        });
                    }
                    format!("{defined_type}{suffix}")
                }
            };
            top.insert_with_remedy(
                language,
                const_ident,
                format!("`{model_full_name}.{json_name}` value constant for {value}"),
                remedy,
            )?;
        }
    }
    Ok(())
}

/// The P15 pass for everything the **Java** emitter nests inside a model class.
///
/// Java is the one target whose synthesized declarations are not package-scoped,
/// so `collect_synthesized_top_level` used to skip it outright and its nested
/// names were outside P15 entirely: a `const`/`enum` member named `serializer`
/// or `deserializer` emitted a second nested class of the generated
/// `Serializer`/`Deserializer`'s name; one named `violation` shadowed the
/// imported runtime `Violation` for the whole class body; two
/// `x-java-enum-names` entries could name one constant twice; and two enum
/// members folding together under Java's `UPPER_SNAKE` token slipped through
/// whenever a *Go* value-constant override was present, because the shared
/// pre-language fold check in `validate_const_enum` steps aside for an override
/// in **any** constant-synthesizing target and defers to this pass.
///
/// Three scopes, because Java keeps them apart:
/// - the compilation unit, holding the top-level model and any schema-dependent
///   bare imports;
/// - the model class's **member type** scope, holding one value class per
///   closed-value member plus the generated `Serializer`/`Deserializer`, and
///   shadowing the runtime and schema-dependent classes imported by simple
///   name; and
/// - each value class's **constant** scope.
fn collect_java_nested_scope(
    model_full_name: &str,
    type_ident: &str,
    schema: &Schema,
    imports_offset_date_time: bool,
) -> Result<()> {
    let language = Language::Java;
    let mut nested = Namespace::default();
    for (json_name, property) in schema.properties.iter().flatten() {
        if is_sum_type_union(property) {
            let member = member_identifier(language, json_name, property);
            let interface = java_upper_first(&member);
            let origin = format!("`{model_full_name}.{json_name}` inline union");
            nested.insert(language, interface.clone(), format!("{origin} interface"))?;
            for branch in property.one_of.iter().flatten() {
                if branch.reference.is_some() {
                    continue;
                }
                let Some(suffix) =
                    branch
                        .ty
                        .as_ref()
                        .and_then(Value::as_str)
                        .and_then(|ty| match ty {
                            "object" => Some("Object"),
                            "string" => Some("String"),
                            "integer" => Some("Integer"),
                            "number" => Some("Number"),
                            "boolean" => Some("Boolean"),
                            "array" => Some("Array"),
                            _ => None,
                        })
                else {
                    continue;
                };
                nested.insert(
                    language,
                    format!("{interface}{suffix}"),
                    format!("{origin} `{suffix}` variant wrapper"),
                )?;
            }
        }
        let values = schema_closed_values(property);
        if values.is_empty() {
            continue;
        }
        // The nested value class is `UpperFirst(<member>)`, so an `x-java-name`
        // moves it with the member (mirrors `resolve_model_kind`).
        let member = member_identifier(language, json_name, property);
        let class = java_upper_first(&member);
        nested.insert(
            language,
            class.clone(),
            format!("`{model_full_name}.{json_name}` closed-value class"),
        )?;
        // The constant scope is named from the *values*, so its remedy is the
        // value-constant override, not `x-java-name` (which moves the class).
        let remedy = if values.len() == 1 {
            lang_const_name_keyword(language).unwrap_or("x-<lang>-const-name")
        } else {
            lang_enum_names_keyword(language).unwrap_or("x-<lang>-enum-names")
        };
        let mut constants = Namespace::default();
        for value in &values {
            if value_constant_override(language, property, value).is_none()
                && java_value_token(value).is_empty()
            {
                return Err(Error::InvalidJsonSchema {
                    path: PathBuf::from("<json-schema>"),
                    reason: format!(
                        "`{model_full_name}.{json_name}` value {value} does not encode to a legal Java constant identifier; provide an `{remedy}` override"
                    ),
                });
            }
            let const_ident = java_value_constant_name(property, value);
            constants.insert_with_remedy(
                language,
                const_ident,
                format!("`{model_full_name}.{json_name}` value constant for {value}"),
                remedy,
            )?;
        }
    }
    // The generated nested classes and the runtime classes the model file
    // imports by simple name share this scope. Entered after the user members so
    // the diagnostic names the user's member as the prior origin.
    for ident in ["Serializer", "Deserializer"] {
        nested.insert(
            language,
            ident.to_string(),
            format!("generated nested `{ident}` class"),
        )?;
    }
    for ident in boilerplate_idents(language) {
        nested.insert(
            language,
            (*ident).to_string(),
            format!("generated runtime identifier `{ident}`"),
        )?;
    }
    if imports_offset_date_time {
        // Java imports are compilation-unit scoped. Reserve the simple name
        // against the file's top-level model and nested type declarations, but
        // not against unrelated model files in the same package.
        let origin = "generated model-file import `java.time.OffsetDateTime`";
        let mut compilation_unit = Namespace::default();
        compilation_unit.insert(
            language,
            type_ident.to_string(),
            format!("type `{model_full_name}`"),
        )?;
        compilation_unit.insert(language, "OffsetDateTime".to_string(), origin.to_string())?;
        nested.insert(language, "OffsetDateTime".to_string(), origin.to_string())?;
    }
    Ok(())
}

/// Mirrors the Java emitter's `upper_first`.
fn java_upper_first(name: &str) -> String {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Mirrors the Java emitter's `java_const_name` / `java_closed_token`: the
/// verbatim override when one is authored, else the value's own `SHOUTY_SNAKE`
/// token with no member-derived component, `V_`-guarded when it cannot lead with
/// an ASCII letter. Keep the two in step — this pass decides which schemas load,
/// so a drift either rejects a schema the emitter would have emitted fine or
/// lets two identical constants through to a Java compile error.
fn java_value_constant_name(property: &Schema, value: &Value) -> String {
    if let Some(name) = value_constant_override(Language::Java, property, value) {
        return name.to_string();
    }
    let token = java_value_token(value);
    let needs_guard =
        matches!(value, Value::Number(_)) || !token.starts_with(|c: char| c.is_ascii_alphabetic());
    if needs_guard {
        format!("V_{token}")
    } else {
        token
    }
}

fn java_value_token(value: &Value) -> String {
    match value {
        Value::String(text) => text.to_shouty_snake_case(),
        Value::Bool(flag) => if *flag { "TRUE" } else { "FALSE" }.to_string(),
        Value::Number(number) => crate::json_schema::scalar::value_token_decimal(number)
            .replace('-', "NEG_")
            .replace('.', "_"),
        _ => String::new(),
    }
}

/// The Go emitter's union-field suffix: the union's sealed interface is
/// `<Type><Member>`, and the member half is the **emitted member identifier** —
/// `x-go-name` moves the interface, its variant wrappers and its dispatcher
/// along with the property they are synthesized from (P15: a name synthesized
/// from a member moves with that member). Mirrors `go_union_field_suffix` →
/// `Schema::go_member_name` in `src/generator/json_schema/go.rs`.
///
/// Deriving it from the raw JSON name instead made this pass reject a schema
/// that already carried the `x-go-name` its own diagnostic asked for — the
/// lying fix-it P15 forbids. The flip cannot hide a collision: two members of
/// one model may not share an identifier ([`validate_member_scope`] rejects
/// that first), so distinct members still yield distinct suffixes.
fn go_union_field_suffix(json_name: &str, property: &Schema) -> String {
    member_identifier(Language::Go, json_name, property)
}

/// Adds the package-scope identifiers one Go union synthesizes: a wrapper type
/// per non-`$ref` branch (`<Union>String`, `<Union>Integer`, `<Union>Number`,
/// `<Union>Boolean`, `<Union>Array`, and `<Union>Object` for the free-form
/// object branch the hoist pass leaves inline) plus the `unmarshal<Union>`
/// dispatcher. A `$ref` branch contributes nothing: its target is a declared
/// model that already holds its own name in this scope, and the union only adds
/// a marker method to it.
fn collect_go_union_top_level(
    origin: &str,
    union_ident: &str,
    schema: &Schema,
    top: &mut Namespace,
) -> Result<()> {
    let language = Language::Go;
    for branch in schema.one_of.iter().flatten() {
        if branch.reference.is_some() {
            continue;
        }
        // Every branch declares one recognized `type` by the time the manifest
        // is built (the sum-type pass rejects anything else), and a `null`
        // branch is the nullability marker rather than a variant.
        let Some(suffix) = branch
            .ty
            .as_ref()
            .and_then(Value::as_str)
            .and_then(|ty| match ty {
                "object" => Some("Object"),
                "string" => Some("String"),
                "integer" => Some("Integer"),
                "number" => Some("Number"),
                "boolean" => Some("Boolean"),
                "array" => Some("Array"),
                _ => None,
            })
        else {
            continue;
        };
        top.insert(
            language,
            format!("{union_ident}{suffix}"),
            format!("{origin} `{suffix}` variant wrapper"),
        )?;
    }
    // The dispatcher is package-scoped too. It is lower-camel, so it can only
    // ever coincide with another union's dispatcher — but registering it keeps
    // the pass a complete description of what the union writes into the package,
    // and names the second union in the diagnostic.
    top.insert(
        language,
        format!("unmarshal{union_ident}"),
        format!("{origin} dispatch function"),
    )?;
    Ok(())
}

/// Per-model member-scope collision checks (one scope per aggregate): two
/// members that recase/override to the same identifier collide. Synthesized
/// member-scope names participate too: Go's `<Field>OrDefault()` method and
/// Python's private `_<field>` storage for a default-bearing property.
fn validate_member_scope(language: Language, model_full_name: &str, schema: &Schema) -> Result<()> {
    let Some(properties) = &schema.properties else {
        return Ok(());
    };
    let mut scope = Namespace::default();
    for (json_name, property) in properties {
        // P15: a member whose recased name is invalid/reserved is rejected, not
        // silently mangled — the `x-<lang>-name` override is the escape hatch.
        if override_name(language, property).is_none()
            && let Some((ident, reason)) = member_identifier_defect(language, json_name)
        {
            let subject = if json_name.is_empty() {
                format!("the empty JSON member name in model `{model_full_name}`")
            } else {
                format!("member `{model_full_name}.{json_name}` recases to `{ident}`")
            };
            return Err(Error::InvalidJsonSchema {
                path: PathBuf::from("<json-schema>"),
                reason: format!(
                    "{subject}, which {reason} in {} output; add an `{}` override with a valid identifier (P15 — the generator never auto-mangles)",
                    language.as_str(),
                    lang_name_keyword(language).unwrap_or("x-<lang>-name"),
                ),
            });
        }
        scope.insert(
            language,
            member_identifier(language, json_name, property),
            format!("member `{model_full_name}.{json_name}`"),
        )?;
    }
    // An open struct (anything but `additionalProperties: false`) emits a
    // synthesized catch-all member holding unknown keys; its identifier shares
    // the member scope, so a declared member colliding with it rejects (P15)
    // rather than silently overwriting the catch-all.
    let is_open = !matches!(&schema.additional_properties, Some(Value::Bool(false)));
    if is_open {
        scope.insert(
            language,
            recase_member(language, "additionalProperties"),
            format!("`{model_full_name}` additional-properties catch-all"),
        )?;
    }
    if language == Language::Java {
        collect_java_constraint_fields(model_full_name, schema, &mut scope)?;
    }
    // A Python default-bearing property stores presence in `_<field>`. The
    // backing slot and every declared member occupy the same class namespace;
    // `x-py-name` moves both the public property and its backing name.
    if language == Language::Python {
        for (json_name, property) in properties {
            let Some(default) = property.extra.get("default") else {
                continue;
            };
            if default.is_null() || default.is_object() || default.is_array() {
                continue;
            }
            let member = member_identifier(language, json_name, property);
            scope.insert(
                language,
                format!("_{member}"),
                format!("`{model_full_name}.{json_name}` default backing field"),
            )?;
        }
    }
    if language == Language::TypeScript {
        // Converter parameters/locals share a block with the per-member parse
        // slots. `undefined`, `eval`, and `arguments` are hazardous bindings in
        // strict-mode modules even though they are not TypeScript keywords.
        for local in ["arguments", "eval", "out", "raw", "undefined", "violations"] {
            scope.insert(
                language,
                local.to_string(),
                format!("generated converter binding `{local}`"),
            )?;
        }
        // An interface member of one of these names conflicts with the
        // corresponding Object member, and the converter's plain-object
        // narrowing then observes the intrinsic signature instead of the
        // schema-declared one.
        for intrinsic in [
            "__proto__",
            "constructor",
            "hasOwnProperty",
            "isPrototypeOf",
            "propertyIsEnumerable",
            "toLocaleString",
            "toString",
            "valueOf",
        ] {
            scope.insert(
                language,
                intrinsic.to_string(),
                format!("TypeScript Object member `{intrinsic}`"),
            )?;
        }
    }
    let required: BTreeSet<&str> = schema
        .required
        .as_ref()
        .and_then(Value::as_array)
        .map(|values| values.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    // A scalar `default` on an optional member synthesizes a defaulting
    // accessor: Go's `<Field>OrDefault()` and Java's `get<Field>OrDefault()`.
    let defaulted_members = || {
        properties.iter().filter(|(json_name, property)| {
            property.extra.get("default").is_some_and(|default| {
                !(default.is_null() || default.is_object() || default.is_array())
            }) && !required.contains(json_name.as_str())
        })
    };
    if language == Language::Go {
        // Go puts a struct's fields and its methods in one namespace ("type X
        // has both field and method named Validate"), so the fixed method set
        // every model carries belongs in this scope: a member named `validate`
        // recases straight onto `Validate` and the package stops compiling.
        // Entered after the declared members so the diagnostic names the user's
        // member as the prior origin.
        for (json_name, property) in defaulted_members() {
            let accessor = format!(
                "{}OrDefault",
                member_identifier(Language::Go, json_name, property)
            );
            scope.insert(
                language,
                accessor,
                format!("`{model_full_name}.{json_name}` OrDefault accessor"),
            )?;
        }
        for method in ["Validate", "MarshalJSON", "UnmarshalJSON"] {
            scope.insert(
                language,
                method.to_string(),
                format!("generated `{method}` method"),
            )?;
        }
    }
    // Java keeps fields and methods in separate namespaces, so its accessors get
    // their own scope: a field `getA` and a method `getA()` coexist, but two
    // methods of one name do not. `{a: <default>}` synthesizes
    // `getAOrDefault()`, which is exactly the getter of a sibling member named
    // `aOrDefault` — the pair Go already rejects (decision D9 keeps the
    // accessor, so its name has to participate).
    if language == Language::Java {
        let mut methods = Namespace::default();
        for (json_name, property) in properties {
            let getter = format!(
                "get{}",
                java_upper_first(&member_identifier(language, json_name, property))
            );
            methods.insert(
                language,
                getter,
                format!("`{model_full_name}.{json_name}` getter"),
            )?;
        }
        for (json_name, property) in defaulted_members() {
            let accessor = format!(
                "get{}OrDefault",
                java_upper_first(&member_identifier(language, json_name, property))
            );
            methods.insert(
                language,
                accessor,
                format!("`{model_full_name}.{json_name}` OrDefault accessor"),
            )?;
        }
        // Java's generated **local** namespace. P15 scopes by "whatever unit the
        // target actually resolves names in", and for the collecting
        // deserializer that includes its locals: the member slots are declared
        // at method scope (`String index = null;`), and Java forbids both a
        // duplicate at that scope and a nested block redeclaring an enclosing
        // local. So a member named `index` beside any array member emits
        // `variable index is already defined in method deserialize(...)`.
        //
        // The nested-array loop locals cannot be listed — the emitter mints one
        // set per nesting level — so they are matched by shape instead.
        for (json_name, property) in properties {
            let ident = member_identifier(language, json_name, property);
            if java_is_nested_level_local(&ident) {
                return Err(Error::InvalidJsonSchema {
                    path: PathBuf::from("<json-schema>"),
                    reason: format!(
                        "member `{model_full_name}.{json_name}` maps to `{ident}` in java output, which is the loop variable the generated deserializer binds for a nested array at depth {}; add an `x-java-name` override with a different identifier (P15 — the generator never auto-mangles)",
                        ident.trim_start_matches(|c: char| !c.is_ascii_digit()),
                    ),
                });
            }
            // A `const`/`enum` member's parse block binds `<member>Value` to the
            // decoded wire scalar before matching it. That is a name synthesized
            // *from the member*, so it belongs in the member scope beside the
            // slot itself — otherwise `{h: {enum: [...]}, hValue: {…}}` compiles
            // or not depending on which of the two the author wrote first.
            if !schema_closed_values(property).is_empty() {
                scope.insert(
                    language,
                    format!("{ident}Value"),
                    format!("`{model_full_name}.{json_name}` decoded-value local"),
                )?;
            }
        }
        // Entered after the declared members so a collision names the user's
        // member as the prior origin.
        for local in JAVA_DESERIALIZER_LOCALS {
            scope.insert(
                language,
                (*local).to_string(),
                format!("generated deserializer local `{local}`"),
            )?;
        }
    }
    Ok(())
}

/// Registers the compiled `Pattern` fields emitted directly in a Java model
/// class. These share the ordinary field namespace, and their position starts
/// from the emitted member name so `x-java-name` is a real escape hatch.
fn collect_java_constraint_fields(
    model_full_name: &str,
    schema: &Schema,
    scope: &mut Namespace,
) -> Result<()> {
    if let Some(Value::Object(value)) = &schema.additional_properties
        && let Ok(member) = serde_json::from_value::<Schema>(Value::Object(value.clone()))
    {
        collect_java_string_constraint_fields(
            "additionalPropertiesValue",
            &member,
            &format!("`{model_full_name}` additional-properties catch-all"),
            scope,
        )?;
    }
    if let Some(Value::Object(value)) = schema.extra.get("propertyNames")
        && let Ok(names) = serde_json::from_value::<Schema>(Value::Object(value.clone()))
    {
        collect_java_string_constraint_fields(
            "propertyName",
            &names,
            &format!("`{model_full_name}` property-name constraint"),
            scope,
        )?;
    }
    for (json_name, property) in schema.properties.iter().flatten() {
        let shape = nullable_non_null_schema(property).unwrap_or(property);
        let position = member_identifier(Language::Java, json_name, property);
        collect_java_string_constraint_fields(
            &position,
            shape,
            &format!("member `{model_full_name}.{json_name}`"),
            scope,
        )?;
    }
    Ok(())
}

fn collect_java_string_constraint_fields(
    position: &str,
    schema: &Schema,
    origin: &str,
    scope: &mut Namespace,
) -> Result<()> {
    // The Java emitter looks through the nullability wrapper before deriving
    // string constraints for every position, including a typed
    // `additionalProperties` catch-all. Planning must inspect that same shape
    // or it can miss the static field the emitter actually declares.
    let schema = java_constraint_shape(schema);
    if schema
        .extra
        .get("pattern")
        .and_then(Value::as_str)
        .is_some()
    {
        scope.insert(
            Language::Java,
            java::java_pattern_field_name(position),
            format!("{origin} `{position}` pattern field"),
        )?;
    }
    if schema
        .extra
        .get("format")
        .and_then(Value::as_str)
        .and_then(crate::json_schema::format::check_for)
        .is_some()
    {
        scope.insert(
            Language::Java,
            java::java_format_field_name(position),
            format!("{origin} `{position}` format field"),
        )?;
    }
    if let Some(Value::Object(value)) = schema.extra.get("contains")
        && let Ok(matcher) = serde_json::from_value::<Schema>(Value::Object(value.clone()))
    {
        let matcher = nullable_non_null_schema(&matcher).unwrap_or(&matcher);
        if matcher
            .extra
            .get("pattern")
            .and_then(Value::as_str)
            .is_some()
        {
            scope.insert(
                Language::Java,
                java::java_contains_pattern_field_name(position),
                format!("{origin} `{position}` contains-pattern field"),
            )?;
        }
        if matcher
            .extra
            .get("format")
            .and_then(Value::as_str)
            .and_then(crate::json_schema::format::check_for)
            .is_some()
        {
            scope.insert(
                Language::Java,
                java::java_contains_format_field_name(position),
                format!("{origin} `{position}` contains-format field"),
            )?;
        }
    }
    Ok(())
}

/// The shape the Java emitter inspects when deriving compiled constraint-field
/// names. Keep this predicate in lockstep with its
/// `nullable_non_null_schema`: the loader rejects array-valued `type`, so the
/// emitter's `schema_type_includes(branch, "null")` reduces to this string
/// comparison here.
fn java_constraint_shape(schema: &Schema) -> &Schema {
    schema
        .one_of
        .as_ref()
        .and_then(|branches| {
            branches.iter().find(|branch| {
                !schema_type_is_null(branch) && branch.extra.get("const") != Some(&Value::Null)
            })
        })
        .unwrap_or(schema)
}

/// Every identifier the generated Java object deserializer binds in the same
/// method body as the member slots: its two parameters and the locals of the
/// preamble, the per-member parse blocks, and the array/uniqueItems/contains
/// checks. Each one is a `variable … is already defined` on a member of that
/// name — measured with `javac`, not inferred.
///
/// Reserved unconditionally, even though most are emitted only for a shape the
/// model may not have. The alternative makes a model's validity depend on
/// whether some *sibling* happens to be an array today, so adding an unrelated
/// property would break a member that had always been fine — the silent
/// breakage on an unrelated edit P15 exists to prevent. Go's fixed method set
/// and `boilerplate_idents` are reserved on the same basis.
///
/// Deliberately **absent**, and measured to compile: `key`, `item` and
/// `itemPath`. All three are bound inside the catch-all parse, which the
/// emitter closes *before* the first member slot is declared, so they are out
/// of scope by the time the slots exist. If that parse ever moves after the
/// member slots, they belong here too. `additionalProperties` is absent because
/// the catch-all check above already owns it, with a better diagnostic.
const JAVA_DESERIALIZER_LOCALS: &[&str] = &[
    "context",
    "element",
    "elementPath",
    "field",
    "fieldNames",
    "index",
    "items",
    "length",
    "nestedLength",
    "nestedViolations",
    "node",
    "numberValue",
    "parsed",
    "parser",
    "priorIndex",
    "rawElement",
    "rawIndex",
    "rawKey",
    "rawMatchCount",
    "rawSeen",
    "violation",
    "violations",
];

/// True for the depth-suffixed loop locals the Java emitter mints once per
/// nested-array level — `items1`, `index1`, `element1`, `path1`, then `…2`, `…3`
/// (`render_parse_element`'s `JavaType::List` arm). The family is unbounded in
/// the schema's nesting depth, so it is matched by shape rather than listed.
/// Level numbering starts at 1, so `element0` is not one of them.
fn java_is_nested_level_local(ident: &str) -> bool {
    ["items", "index", "element", "path"]
        .into_iter()
        .any(|prefix| {
            ident.strip_prefix(prefix).is_some_and(|level| {
                !level.is_empty()
                    && !level.starts_with('0')
                    && level.bytes().all(|byte| byte.is_ascii_digit())
            })
        })
}

/// TypeScript `DEFAULT_<FIELD>` constants (exported module scope). The spelling
/// depends only on the declaring member; a second model claiming it rejects
/// instead of silently renaming the first declaration after an additive edit.
///
/// The identifier is built from the **emitted member identifier**, so an
/// `x-ts-name` override on the declaring property moves this constant with it —
/// a name synthesized *from the member* follows the member (P15). Were it built
/// from the JSON name, two members that recase alike would collide here with no
/// way to author around it: the override would move the members apart while
/// leaving both constants on the colliding name.
fn collect_default_constants<'a>(
    language: Language,
    models: impl Iterator<Item = &'a NsModel>,
    top: &mut Namespace,
) -> Result<()> {
    for model in models {
        let Some(properties) = &model.schema.properties else {
            continue;
        };
        for (json_name, property) in properties {
            let Some(default) = property.extra.get("default") else {
                continue;
            };
            if default.is_null() || default.is_object() || default.is_array() {
                continue;
            }
            let member_ident = member_identifier(language, json_name, property);
            let field_shouty = member_ident.to_shouty_snake_case();
            // A synthesized name is a function of its own origin only. Prefixing
            // it with the model name when a later model happens to use the same
            // member silently renames the already-published constant (P13/P15).
            // Keep the stable spelling and let Namespace reject the second claim.
            let ident = format!("DEFAULT_{field_shouty}");
            top.insert(
                language,
                ident,
                format!("`{}.{json_name}` DEFAULT_ constant", model.full_name),
            )?;
        }
    }
    Ok(())
}

/// TypeScript `<FIELD>_CONST` constants (module scope). A `const`-bearing member
/// emits a module-level constant holding the fixed wire value, named
/// `<FIELD>_CONST` when the member identifier is unique across the module's
/// models, else `<MODEL>_<FIELD>_CONST`. The constant is not exported, but it is
/// still a module-scope binding: a clash with any other module-scope identifier
/// is a TypeScript redeclaration error, so it belongs in the collision pass
/// (P15) rather than being emitted twice.
///
/// Like the `DEFAULT_` constant, the identifier is built from the **emitted
/// member identifier**, so an `x-ts-name` override moves it with the member.
fn collect_ts_const_constants<'a>(
    models: impl Iterator<Item = &'a NsModel>,
    top: &mut Namespace,
) -> Result<()> {
    let group: Vec<&NsModel> = models.collect();
    // How many models declare a `const` member emitting this identifier.
    let field_count = |member_ident: &str| -> usize {
        group
            .iter()
            .filter(|model| {
                model.schema.properties.as_ref().is_some_and(|properties| {
                    properties.iter().any(|(json_name, property)| {
                        member_identifier(Language::TypeScript, json_name, property) == member_ident
                            && property.extra.contains_key("const")
                    })
                })
            })
            .count()
    };
    for model in &group {
        let Some(properties) = &model.schema.properties else {
            continue;
        };
        for (json_name, property) in properties {
            if !property.extra.contains_key("const") {
                continue;
            }
            let member_ident = member_identifier(Language::TypeScript, json_name, property);
            let field_shouty = member_ident.to_shouty_snake_case();
            let ident = if field_count(&member_ident) == 1 {
                format!("{field_shouty}_CONST")
            } else {
                format!(
                    "{}_{field_shouty}_CONST",
                    model.type_ident.to_shouty_snake_case()
                )
            };
            top.insert(
                Language::TypeScript,
                ident,
                format!("`{}.{json_name}` _CONST constant", model.full_name),
            )?;
        }
    }
    Ok(())
}

/// The remaining module-scope identifiers the Python JSON-Schema generator
/// synthesizes, entered into the same namespace as the user types and services
/// so a coincidence rejects at load instead of one
/// definition silently overwriting the other (P15).
///
/// Each is named by [`build_name_manifest`]'s resolved `type_ident`, so a
/// type-level `x-py-name` override moves all of them together — and every ident
/// is computed by the *generator's* own naming helper, never re-derived here, so
/// the check cannot drift from what is emitted:
///
/// - `_<Model>TransferTypeConverter` — the converter class carrying the model's
///   whole wire contract (class models only; a union has no converter class).
/// - `_<MODEL>_DECLARED` — the declared-key `frozenset` an *open* object splits
///   its catch-all on. `to_shouty_snake_case` is not injective over the verbatim
///   overrides (`ContactPy` and `ContactPY` both shout to `CONTACT_PY`), which is
///   how a declared property used to leak into the catch-all of whichever model
///   lost the race.
/// - `_<base>_from_transfer_type` / `_<base>_to_transfer_type` — a union's
///   conversion functions. `to_snake_case` is likewise non-injective, and a named
///   union's base can also coincide with an inline (`<model>_<member>`) one.
/// - `_PATTERN_<HEX>` — the shared compiled regexes. Identical pattern text
///   *intentionally* shares one constant, so the origin is keyed by that text:
///   a repeat is deduplication (accepted), while two distinct patterns landing on
///   one name — or a user type overridden to that shape — is a collision.
/// - the converter bodies' own locals ([`PYTHON_CONVERTER_BODY_LOCALS`]).
fn collect_python_module_idents<'a>(
    models: impl Iterator<Item = &'a NsModel>,
    top: &mut Namespace,
) -> Result<()> {
    let language = Language::Python;
    // A converter body reads the module's own classes and constants by bare name
    // while binding these locals in the same scope, so a module-level identifier
    // spelled like one of them is shadowed inside every body that binds it.
    // Nothing *derived* lands here — user types are `UpperCamelCase` and the
    // synthesized names are `_`-prefixed or shouty — so this only ever fires on a
    // verbatim `x-py-name` that spells a runtime local (P15).
    for local in PYTHON_CONVERTER_BODY_LOCALS {
        top.insert(
            language,
            (*local).to_string(),
            format!("generated converter-body local `{local}`"),
        )?;
    }
    for model in models {
        let origin = |what: &str| format!("`{}` {what}", model.full_name);
        // A sum-type def is emitted as a `TypeAlias` whose conversion lives in a
        // pair of module-private free functions, so it has no converter class and
        // no declared-key set. This one predicate covers the emitter's
        // `is_python_union_model` / `is_py_union` pair: they can only disagree on a
        // branch typed `["string", "null"]`, a form the loader has already
        // rejected by the time the manifest is built.
        if is_sum_type_union(&model.schema) {
            let base = python::union_fn_base(&model.type_ident);
            top.insert(
                language,
                python::union_parse_fn(&base),
                origin("union parse function"),
            )?;
            top.insert(
                language,
                python::union_serialize_fn(&base),
                origin("union serialize function"),
            )?;
        } else if !model.schema.is_bare_ref() {
            top.insert(
                language,
                python::converter_class_name(&model.type_ident),
                origin("transfer-type converter class"),
            )?;
            if python_open_object(&model.schema) {
                top.insert(
                    language,
                    python::declared_fields_const_name(&model.type_ident),
                    origin("declared-key frozenset"),
                )?;
            }
        }
        // An inline (property-level) union gets its own function pair, named
        // `<model>_<member>` — so a member-level `x-py-name` moves it.
        for (json_name, property) in model.schema.properties.iter().flatten() {
            if !is_sum_type_union(property) {
                continue;
            }
            let base = python::inline_union_fn_base(
                &model.type_ident,
                &member_identifier(language, json_name, property),
            );
            top.insert(
                language,
                python::union_parse_fn(&base),
                origin(&format!("`{json_name}` inline union parse function")),
            )?;
            top.insert(
                language,
                python::union_serialize_fn(&base),
                origin(&format!("`{json_name}` inline union serialize function")),
            )?;
        }
        collect_python_pattern_constants(&model.schema, top)?;
    }
    Ok(())
}

/// Every fixed identifier a generated Python converter body binds or receives:
/// the accumulator and wire dictionaries, the loop and dispatch temporaries, and
/// the function parameters. The property-derived slots are absent by
/// construction — they are suffixed `_value` precisely so they cannot coincide
/// with anything here (see the generator's `parse_slot_local`).
const PYTHON_CONVERTER_BODY_LOCALS: &[&str] = &[
    "additional_properties",
    "entry",
    "error",
    "items",
    "key",
    "member",
    "narrowed",
    "number",
    "out",
    "parsed",
    "path",
    "raw",
    "self",
    "tag",
    "tagged",
    "type_hint",
    "value",
    "violations",
];

/// Mirrors the Python emitter's `is_open_object`: a declared-property object that
/// stays open to unknown members, which is what gives it the catch-all — and the
/// module-level declared-key set the catch-all is split on.
fn python_open_object(schema: &Schema) -> bool {
    schema.ty.as_ref().and_then(Value::as_str) == Some("object")
        && schema
            .properties
            .as_ref()
            .is_some_and(|properties| !properties.is_empty())
        && schema.additional_properties.as_ref() != Some(&Value::Bool(false))
}

/// Walks every string position that hoists a compiled regex — mirroring the
/// emitter's `collect_schema_patterns` — and enters each constant under an origin
/// keyed by the pattern text, so identical patterns dedupe and distinct ones
/// collide.
fn collect_python_pattern_constants(schema: &Schema, top: &mut Namespace) -> Result<()> {
    let insert = |pattern: &str, top: &mut Namespace| -> Result<()> {
        let emitted = crate::json_schema::pattern::rewrite_end_anchor(pattern, r"\Z");
        top.insert(
            Language::Python,
            python::py_pattern_const_name(&emitted),
            format!("compiled pattern constant for {emitted:?}"),
        )
    };
    if let Some(Value::String(pattern)) = schema.extra.get("pattern") {
        insert(pattern, top)?;
    }
    if let Some(Value::String(format)) = schema.extra.get("format")
        && let Some(check) = crate::json_schema::format::check_for(format)
    {
        insert(&check.pattern, top)?;
    }
    for property in schema
        .properties
        .iter()
        .flat_map(|entries| entries.values())
    {
        collect_python_pattern_constants(property, top)?;
    }
    if let Some(items) = &schema.items {
        collect_python_pattern_constants(items, top)?;
    }
    for branch in schema.one_of.iter().flatten() {
        collect_python_pattern_constants(branch, top)?;
    }
    // A key-shape subschema and a typed map's member schema are both carried as
    // raw values here; decode them the same way the emitter does.
    for nested in [
        schema.extra.get("propertyNames"),
        schema.additional_properties.as_ref(),
    ] {
        if let Some(value @ Value::Object(_)) = nested
            && let Ok(subschema) = serde_json::from_value::<Schema>(value.clone())
        {
            collect_python_pattern_constants(&subschema, top)?;
        }
    }
    Ok(())
}

/// TypeScript per-model `TransferTypeConverter` instances (module scope). The
/// identifier is derived from the model's type identifier
/// ([`ts_transfer_type_converter_name`]), and lower-camel-casing is not
/// injective over the distinct `UpperCamelCase` type names — `HTTPError` and
/// `HttpError` both derive `httpErrorTransferTypeConverter` — so the derived
/// name has to enter the shared module namespace too, or two models emit the
/// same `export const` (P15).
fn collect_ts_transfer_type_converters<'a>(
    models: impl Iterator<Item = &'a NsModel>,
    top: &mut Namespace,
) -> Result<()> {
    for model in models {
        top.insert(
            Language::TypeScript,
            ts_transfer_type_converter_name(&model.type_ident),
            format!("type `{}` transfer type converter", model.full_name),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::language::Language;

    fn parse(input: &str) -> ApiSpec {
        parse_api_spec_from_json_schema_for_language(
            Language::Python,
            input,
            PathBuf::from("api.yaml"),
        )
        .unwrap()
    }

    #[test]
    fn marks_each_source_root_and_defs_as_module_exports() {
        let spec = parse(
            r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  nested: { $ref: "#/$defs/Nested" }
$defs:
  Nested:
    type: object
    properties:
      value: { type: string }
"##,
        );
        assert!(spec.types.values().all(|entry| entry.is_module_export()));
        assert_eq!(spec.types.len(), 2);
    }

    #[test]
    fn parses_operation_refs_as_json_external_models() {
        let spec = parse(
            r##"
nexusrpc: "1.0.0"
$schema: https://json-schema.org/draft/2020-12/schema
services:
  ChatService:
    fqn: example.chat.v1.ChatService
    operations:
      sendMessage:
        fqn: SendMessage
        description: Send a message.
        input: { $ref: "#/$defs/SendMessageInput" }
        output: { $ref: "#/$defs/SendMessageOutput" }
$defs:
  SendMessageInput:
    type: object
    properties:
      roomId: { type: string }
    required: [roomId]
  SendMessageOutput:
    type: object
    properties:
      messageId: { type: string }
    required: [messageId]
"##,
        );

        assert!(spec.records().next().is_none());
        assert_eq!(spec.services[0].name, "ChatService");
        assert_eq!(spec.services[0].endpoint, None);
        let operation = &spec.services[0].operations[0];
        assert_eq!(operation.name, "SendMessage");
        assert_eq!(operation.wire_name, "SendMessage");
        assert_eq!(operation.doc.default.as_deref(), Some("Send a message."));
        let Some(TypeSpec::External(ExternalTypeSpec::Json(input))) = &operation.input else {
            panic!("input should be a JSON external model");
        };
        assert_eq!(input.name.as_str(), "SendMessageInput");
        assert_eq!(input.model_name, "SendMessageInput");
        let Some(TypeSpec::External(ExternalTypeSpec::Json(output))) = &operation.output else {
            panic!("output should be a JSON external model");
        };
        assert_eq!(output.name.as_str(), "SendMessageOutput");
        assert!(spec.external_type_binding("SendMessageInput").is_some());
        assert!(spec.external_type_binding("SendMessageOutput").is_some());
    }

    #[test]
    fn bare_ref_def_is_preserved_as_an_alias_and_resolves_for_operation_io() {
        let spec = parse(
            r##"
nexusrpc: "1.0.0"
$schema: https://json-schema.org/draft/2020-12/schema
services:
  AliasService:
    operations:
      echo:
        input: { $ref: "#/$defs/Alias" }
        output: { $ref: "#/$defs/Alias" }
$defs:
  Alias: { $ref: "#/$defs/Target" }
  Target:
    type: object
    additionalProperties: false
    required: [value]
    properties: { value: { type: string } }
"##,
        );

        let alias = spec
            .external_type_binding("Alias")
            .and_then(ExternalTypeBindingSpec::json_model)
            .expect("alias model");
        assert_eq!(alias.schema["$ref"], "#/$defs/Target");
        let operation = &spec.services[0].operations[0];
        for io in [operation.input.as_ref(), operation.output.as_ref()] {
            let Some(TypeSpec::External(ExternalTypeSpec::Json(model))) = io else {
                panic!("alias operation I/O should remain a JSON model");
            };
            assert_eq!(model.name.as_str(), "Alias");
        }
    }

    #[test]
    fn cross_file_bare_ref_root_is_a_resolvable_operation_alias() {
        let spec = api_spec_from_json_schema_sources(
            Language::Python,
            vec![
                (
                    PathBuf::from("target.yaml"),
                    "$schema: https://json-schema.org/draft/2020-12/schema\ntype: object\nadditionalProperties: false\nrequired: [value]\nproperties: { value: { type: string } }\n".to_string(),
                ),
                (
                    PathBuf::from("alias.yaml"),
                    "$schema: https://json-schema.org/draft/2020-12/schema\n$ref: target.yaml#\n".to_string(),
                ),
                (
                    PathBuf::from("service.nexusrpc.yaml"),
                    "nexusrpc: 1.0.0\nservices:\n  AliasService:\n    operations:\n      echo:\n        input: { $ref: alias.yaml# }\n        output: { $ref: alias.yaml# }\n".to_string(),
                ),
            ],
        )
        .expect("root alias should resolve through to an object");

        let alias = spec
            .external_type_binding("Alias")
            .and_then(ExternalTypeBindingSpec::json_model)
            .expect("root alias model");
        assert_eq!(alias.schema["$ref"], "#/$defs/Target");
        assert!(spec.external_type_binding("Target").is_some());
        let Some(TypeSpec::External(ExternalTypeSpec::Json(input))) =
            spec.services[0].operations[0].input.as_ref()
        else {
            panic!("operation input should resolve to the alias model");
        };
        assert_eq!(input.name.as_str(), "Alias");
    }

    #[test]
    fn inline_operation_io_is_json_external_not_record() {
        let spec = parse(
            r##"
nexusrpc: "1.0.0"
services:
  ChatService:
    operations:
      getRoom:
        input:
          type: object
          properties:
            roomId: { type: string }
          required: [roomId]
        output:
          type: object
          properties:
            displayName: { type: string }
          required: [displayName]
"##,
        );

        assert!(spec.records().next().is_none());
        let operation = &spec.services[0].operations[0];
        let Some(TypeSpec::External(ExternalTypeSpec::Json(input))) = &operation.input else {
            panic!("input should be a JSON external model");
        };
        assert_eq!(input.name.as_str(), "ChatService.GetRoomInput");
        assert_eq!(input.schema["properties"]["roomId"]["type"], "string");
        let Some(TypeSpec::External(ExternalTypeSpec::Json(output))) = &operation.output else {
            panic!("output should be a JSON external model");
        };
        assert_eq!(output.name.as_str(), "ChatService.GetRoomOutput");
    }

    #[test]
    fn missing_endpoint_is_allowed_in_parsed_spec() {
        let spec = parse(
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
        );

        assert_eq!(spec.services[0].endpoint, None);
    }

    #[test]
    fn rejects_endpoint_in_nexus_service() {
        let error = doc_reject(
            r##"
nexusrpc: "1.0.0"
services:
  ChatService:
    endpoint: __chat_service
    operations:
      ping: {}
"##,
        );
        assert!(
            error.contains("endpoint") && error.contains("not supported"),
            "{error}"
        );
    }

    #[test]
    fn omitted_operation_io_is_void() {
        let spec = parse(
            r##"
nexusrpc: "1.0.0"
services:
  ChatService:
    operations:
      ping:
        description: Liveness probe.
"##,
        );

        let operation = &spec.services[0].operations[0];
        assert_eq!(operation.name, "Ping");
        assert!(operation.input.is_none());
        assert!(operation.output.is_none());
    }

    #[test]
    fn accepts_every_one_sided_operation_io_combination() {
        let spec = parse(
            r##"
nexusrpc: "1.0.0"
services:
  ChatService:
    operations:
      sendData:
        input: { type: object, properties: {} }
      getData:
        output: { type: object, properties: {} }
      ping: {}
"##,
        );
        let operations = &spec.services[0].operations;
        for (name, has_input, has_output) in [
            ("SendData", true, false),
            ("GetData", false, true),
            ("Ping", false, false),
        ] {
            let operation = operations
                .iter()
                .find(|operation| operation.name == name)
                .unwrap_or_else(|| panic!("missing operation {name}"));
            assert_eq!(operation.input.is_some(), has_input, "{name} input");
            assert_eq!(operation.output.is_some(), has_output, "{name} output");
        }
    }

    #[test]
    fn rejects_unknown_schema_keywords_in_each_operation_io_position() {
        for label in ["input", "output"] {
            let error = doc_reject(&format!(
                r##"
nexusrpc: "1.0.0"
services:
  ChatService:
    operations:
      getData:
        {label}:
          type: object
          properties:
            nested:
              type: string
              minLenght: 2
"##
            ));
            assert!(
                error.contains("unknown schema keyword `minLenght`")
                    && error.contains(&format!(".{label}.properties.nested")),
                "{label}: {error}"
            );
        }
    }

    #[test]
    fn lowers_service_and_operation_deprecation() {
        let spec = parse(
            r##"
nexusrpc: "1.0.0"
services:
  ChatService:
    deprecated: true
    operations:
      ping:
        deprecated: true
"##,
        );
        assert!(spec.services[0].deprecated);
        assert!(spec.services[0].operations[0].deprecated);
    }

    #[test]
    fn rejects_non_boolean_service_and_operation_deprecation() {
        let service = doc_reject(
            r##"
nexusrpc: "1.0.0"
services:
  ChatService:
    deprecated: yes
    operations:
      ping: {}
"##,
        );
        assert!(
            service.contains("`deprecated` must be a boolean"),
            "{service}"
        );

        let operation = doc_reject(
            r##"
nexusrpc: "1.0.0"
services:
  ChatService:
    operations:
      ping:
        deprecated: yes
"##,
        );
        assert!(
            operation.contains("`deprecated` must be a boolean"),
            "{operation}"
        );
    }

    #[test]
    fn ref_with_sibling_keywords_merges() {
        // `$ref`-with-siblings is the implicit-`allOf` sugar: the referenced
        // target is folded in and the use-site siblings extend it (see
        // specs/json-schema/features/allOf.md). No longer a reject.
        let spec = parse(
            r##"
nexusrpc: "1.0.0"
services:
  ChatService:
    operations:
      fetchRoom:
        input:
          $ref: "#/$defs/GetRoomInput"
          properties:
            extra: { type: string }
$defs:
  GetRoomInput:
    type: object
    properties:
      roomId: { type: string }
    required: [roomId]
"##,
        );
        let Some(TypeSpec::External(ExternalTypeSpec::Json(input))) =
            &spec.services[0].operations[0].input
        else {
            panic!("input should be a JSON external model");
        };
        // The merged input carries both the folded `roomId` and the use-site
        // `extra` property, with no `$ref` residue.
        assert_eq!(input.schema["properties"]["roomId"]["type"], "string");
        assert_eq!(input.schema["properties"]["extra"]["type"], "string");
        assert_eq!(input.schema["required"], serde_json::json!(["roomId"]));
        assert!(input.schema.get("allOf").is_none());
        assert!(input.schema["$ref"].is_null());
    }

    #[test]
    fn ref_with_only_member_annotations_remains_a_reference() {
        let schema = model_schema(
            r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  value:
    $ref: "#/$defs/Value"
    deprecated: true
    $comment: use-site note
    examples: [{ text: example }]
$defs:
  Value:
    type: object
    properties:
      text: { type: string }
"##,
            "Api",
        );
        assert_eq!(schema["properties"]["value"]["$ref"], "#/$defs/Value");
        assert_eq!(schema["properties"]["value"]["deprecated"], true);
        assert!(schema["properties"]["value"].get("$comment").is_none());
        assert!(schema["properties"]["value"].get("examples").is_none());
    }

    fn doc_reject(input: &str) -> String {
        parse_api_spec_from_json_schema_for_language(
            Language::Python,
            input,
            PathBuf::from("api.yaml"),
        )
        .unwrap_err()
        .to_string()
    }

    #[test]
    fn rejects_wrong_nexusrpc_version() {
        let error = doc_reject(
            r##"
nexusrpc: "1.1.0"
services:
  ChatService:
    operations:
      ping: {}
"##,
        );
        assert!(error.contains("`nexusrpc` must be exactly"), "{error}");
        assert!(error.contains("1.0.0"), "{error}");
    }

    #[test]
    fn rejects_non_string_nexusrpc() {
        let error = doc_reject(
            r##"
nexusrpc: 1
services:
  ChatService:
    operations:
      ping: {}
"##,
        );
        assert!(error.contains("`nexusrpc` must be exactly"), "{error}");
    }

    #[test]
    fn rejects_explicit_null_document_markers_and_services() {
        let nexus = doc_reject("nexusrpc: null\nservices: {}");
        assert!(nexus.contains("`nexusrpc` must be exactly"), "{nexus}");

        let dialect =
            doc_reject("$schema: null\ntype: object\nproperties: { value: { type: string } }");
        assert!(dialect.contains("`$schema` must be"), "{dialect}");

        let services = doc_reject("nexusrpc: 1.0.0\nservices: null");
        assert!(
            services.contains("`services` must be an object") && services.contains("not null"),
            "{services}"
        );
    }

    #[test]
    fn rejects_wrong_schema_dialect() {
        let error = doc_reject(
            r##"
$schema: "http://json-schema.org/draft-07/schema#"
type: object
properties:
  a: { type: string }
"##,
        );
        assert!(error.contains("`$schema` must be"), "{error}");
        assert!(error.contains("2020-12"), "{error}");
    }

    #[test]
    fn accepts_present_but_empty_document_defs() {
        parse("$schema: https://json-schema.org/draft/2020-12/schema\n$defs: {}");
    }

    #[test]
    fn rejects_schema_shaped_root_in_nexus_doc() {
        let error = doc_reject(
            r##"
nexusrpc: "1.0.0"
type: object
properties:
  a: { type: string }
"##,
        );
        assert!(error.contains("envelope"), "{error}");
    }

    #[test]
    fn rejects_unknown_nexus_envelope_keyword() {
        let error = doc_reject(
            r##"
nexusrpc: "1.0.0"
servcies: {}
"##,
        );
        assert!(
            error.contains("unknown Nexus envelope keyword `servcies`"),
            "{error}"
        );
    }

    #[test]
    fn rejects_unknown_service_and_operation_keywords() {
        let service_error = doc_reject(
            r##"
nexusrpc: "1.0.0"
services:
  ChatService:
    timeout: 5
    operations:
      ping: {}
"##,
        );
        assert!(
            service_error.contains("service `ChatService`")
                && service_error.contains("unknown keyword `timeout`"),
            "{service_error}"
        );

        let operation_error = doc_reject(
            r##"
nexusrpc: "1.0.0"
services:
  ChatService:
    operations:
      ping:
        timeout: 5
"##,
        );
        assert!(
            operation_error.contains("operation `ping`")
                && operation_error.contains("unknown keyword `timeout`"),
            "{operation_error}"
        );
    }

    #[test]
    fn rejects_unknown_schema_keyword_and_openapi_discriminator() {
        let unknown = structural_reject("type: string\nminLenght: 2");
        assert!(
            unknown.contains("unknown schema keyword `minLenght`"),
            "{unknown}"
        );

        let discriminator = structural_reject(
            "type: object\nproperties: {}\ndiscriminator: { propertyName: kind }",
        );
        assert!(
            discriminator.contains("OpenAPI `discriminator` is not yet supported"),
            "{discriminator}"
        );
    }

    #[test]
    fn diagnoses_legacy_dependencies_by_value_form() {
        let arrays = numeric_reject(
            "type: object\nproperties: { a: { type: string }, b: { type: string } }\ndependencies: { a: [b] }",
        );
        assert!(
            arrays.contains("rename `dependencies` to `dependentRequired`"),
            "{arrays}"
        );

        let schemas = numeric_reject(
            "type: object\nproperties: { a: { type: string } }\ndependencies: { a: { required: [a] } }",
        );
        assert!(
            schemas.contains("schema-form") && schemas.contains("dependentSchemas"),
            "{schemas}"
        );
    }

    #[test]
    fn rejects_foreign_structural_shapes_before_lowering() {
        let object_items = numeric_reject("type: object\nproperties: {}\nitems: { type: string }");
        assert!(
            object_items.contains("`items` requires `type: array`"),
            "{object_items}"
        );

        let array_properties = numeric_reject(
            "type: array\nitems: { type: string }\nproperties: { value: { type: string } }",
        );
        assert!(
            array_properties.contains("`properties`/`additionalProperties` require `type: object`"),
            "{array_properties}"
        );

        let union_sibling =
            numeric_reject("type: string\noneOf:\n  - { type: string }\n  - { type: integer }");
        assert!(
            union_sibling.contains("cannot be a sibling of `oneOf`")
                && union_sibling.contains("move it into the branch"),
            "{union_sibling}"
        );
        for sibling in ["minLength: 3", "const: x", "enum: [x, y]", "required: [x]"] {
            let error = numeric_reject(&format!(
                "oneOf:\n  - {{ type: string }}\n  - {{ type: integer }}\n{sibling}"
            ));
            assert!(
                error.contains("cannot be a sibling of `oneOf`")
                    && error.contains("move it into the branch"),
                "{sibling}: {error}"
            );
        }
    }

    #[test]
    fn rejects_unknown_keywords_in_every_recursive_schema_position() {
        for (position, schema) in [
            (
                "nested property",
                "type: object\nproperties:\n  child:\n    type: string\n    minLenght: 2",
            ),
            (
                "array items",
                "type: array\nitems:\n  type: string\n  minLenght: 2",
            ),
            (
                "typed additionalProperties",
                "type: object\nadditionalProperties:\n  type: string\n  minLenght: 2",
            ),
            (
                "contains matcher",
                "type: array\nitems: { type: string }\ncontains:\n  type: string\n  minLenght: 2",
            ),
            (
                "propertyNames matcher",
                "type: object\nadditionalProperties: { type: string }\npropertyNames:\n  type: string\n  minLenght: 2",
            ),
            (
                "allOf branch",
                "allOf:\n  - { type: string, minLenght: 2 }\n  - { type: string, minLength: 1 }",
            ),
        ] {
            let error = structural_reject(schema);
            assert!(
                error.contains("unknown schema keyword `minLenght`"),
                "{position}: {error}"
            );
        }
    }

    #[test]
    fn rejects_services_without_nexusrpc() {
        let error = doc_reject(
            r##"
services:
  ChatService:
    operations:
      ping: {}
"##,
        );
        assert!(error.contains("`services` require"), "{error}");
    }

    #[test]
    fn rejects_service_without_operations() {
        let error = doc_reject(
            r##"
nexusrpc: "1.0.0"
services:
  Chat:
    operations: {}
"##,
        );
        assert!(error.contains("at least one operation"), "{error}");
    }

    #[test]
    fn rejects_empty_inline_operation_io() {
        let error = doc_reject(
            r##"
nexusrpc: "1.0.0"
services:
  Chat:
    operations:
      getRoom:
        input: {}
"##,
        );
        assert!(error.contains("must be `type: object`"), "{error}");
    }

    #[test]
    fn rejects_non_object_inline_operation_io() {
        let error = doc_reject(
            r##"
nexusrpc: "1.0.0"
services:
  Chat:
    operations:
      getRoom:
        input: { type: string }
"##,
        );
        assert!(error.contains("must be `type: object`"), "{error}");
    }

    #[test]
    fn rejects_plain_file_without_root_schema() {
        let error = doc_reject(
            r##"
description: just a description with no schema keywords
"##,
        );
        assert!(error.contains("must define a root schema"), "{error}");
    }

    #[test]
    fn rejects_empty_input_set() {
        let error = api_spec_from_json_schema_sources(Language::Python, vec![])
            .unwrap_err()
            .to_string();
        assert!(error.contains("at least one JSON schema input"), "{error}");
    }

    #[test]
    fn rejects_malformed_yaml() {
        let error = doc_reject("type: object\n  bad: : indentation: [");
        assert!(error.contains("failed to parse JSON schema"), "{error}");
    }

    fn numeric_reject(field_schema: &str) -> String {
        let input = format!(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  value:
{}
"#,
            field_schema
                .lines()
                .map(|line| format!("    {line}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
        parse_api_spec_from_json_schema_for_language(
            Language::Python,
            &input,
            PathBuf::from("api.yaml"),
        )
        .unwrap_err()
        .to_string()
    }

    #[test]
    fn rejects_empty_numeric_interval() {
        let error = numeric_reject("type: integer\nminimum: 10\nmaximum: 2");
        assert!(error.contains("empty range"), "{error}");
    }

    #[test]
    fn rejects_fractional_bound_on_integer_field() {
        let error = numeric_reject("type: integer\nmaximum: 5.5");
        assert!(error.contains("integer bound"), "{error}");
    }

    #[test]
    fn scalar_literal_assignment_is_directional() {
        parse(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  defaulted: { type: number, default: 1 }
  constant: { type: number, const: 2 }
  choices: { type: number, enum: [3, 4.5] }
"#,
        );
        for keyword in ["default: 1.5", "const: 1.5", "enum: [1, 1.5]"] {
            let error = numeric_reject(&format!("type: integer\n{keyword}"));
            assert!(error.contains("incompatible"), "{keyword}: {error}");
        }
    }

    #[test]
    fn accepts_schema_fractions_rounded_to_integral_binary64() {
        // JSON and YAML inputs share the serde_yaml load path. By the time the
        // directional literal check runs, these authored fractions have
        // rounded to the integral binary64 value 4503599627370496.
        for (path, input) in [
            (
                "api.yaml",
                r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  constant: { type: integer, const: 4503599627370496.5 }
  defaulted: { type: integer, default: 4503599627370496.5 }
  choices: { type: integer, enum: [1, 4503599627370496.5] }
"#,
            ),
            (
                "api.json",
                r#"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "properties": {
    "constant": { "type": "integer", "const": 4503599627370496.5 },
    "defaulted": { "type": "integer", "default": 4503599627370496.5 },
    "choices": { "type": "integer", "enum": [1, 4503599627370496.5] }
  }
}"#,
            ),
        ] {
            parse_api_spec_from_json_schema_for_language(
                Language::Python,
                input,
                PathBuf::from(path),
            )
            .unwrap_or_else(|error| panic!("{path}: {error}"));
        }
    }

    #[test]
    fn integer_domain_cap_participates_in_load_satisfiability() {
        for schema in [
            "type: integer\nminimum: 9007199254740992",
            "type: integer\nexclusiveMinimum: 9007199254740991",
            "type: integer\nmaximum: -9007199254740992",
            "type: integer\nexclusiveMaximum: -9007199254740991",
        ] {
            let error = numeric_reject(schema);
            assert!(
                error.contains("portable") && error.contains("cap"),
                "{schema}: {error}"
            );
        }

        // Bounds outside the domain in the non-empty direction are redundant,
        // not unsatisfiable.
        parse(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  value: { type: integer, maximum: 9007199254740992 }
"#,
        );
    }

    #[test]
    fn rejects_unsafe_integer_literals_and_string_counts() {
        for keyword in ["const: 9007199254740992", "enum: [0, -9007199254740992]"] {
            let error = numeric_reject(&format!("type: integer\n{keyword}"));
            assert!(error.contains("integer cap"), "{keyword}: {error}");
        }
        for keyword in ["minLength", "maxLength"] {
            let error = numeric_reject(&format!("type: string\n{keyword}: 9007199254740992"));
            assert!(error.contains("9007199254740991"), "{keyword}: {error}");
        }
    }

    #[test]
    fn rejects_high_magnitude_singleton_that_is_not_a_multiple() {
        let error = numeric_reject("type: number\nminimum: 1e23\nmaximum: 1e23\nmultipleOf: 5");
        assert!(error.contains("no multiple of 5"), "{error}");
    }

    /// These bounds are adjacent binary64 values. The old quotient/product
    /// witness rounded onto the lower endpoint and called it a multiple even
    /// though the runtime remainders of the two possible values are 2 and 3.
    #[test]
    fn rejects_adjacent_high_magnitude_range_without_runtime_multiple() {
        let error = numeric_reject(
            "type: number\nminimum: 1e23\nmaximum: 1.0000000000000001e23\nmultipleOf: 5",
        );
        assert!(error.contains("no multiple of 5"), "{error}");
    }

    #[test]
    fn rejects_boolean_exclusive_maximum_form() {
        let error = numeric_reject("type: integer\nmaximum: 5\nexclusiveMaximum: true");
        assert!(error.contains("boolean form"), "{error}");
    }

    #[test]
    fn rejects_zero_multiple_of() {
        let error = numeric_reject("type: integer\nmultipleOf: 0");
        assert!(error.contains("greater than 0"), "{error}");
    }

    #[test]
    fn rejects_fractional_multiple_of() {
        let error = numeric_reject("type: number\nmultipleOf: 0.1");
        assert!(error.contains("not yet supported"), "{error}");

        let tiny = numeric_reject("type: number\nmultipleOf: 1e-300");
        assert!(tiny.contains("multipleOf: 1e-300"), "{tiny}");
        assert!(tiny.len() < 500, "diagnostic expanded the exponent: {tiny}");
    }

    #[test]
    fn rejects_integer_divisor_above_the_portable_cap() {
        for (divisor, displayed) in [
            ("9007199254740992", "9007199254740992"),
            ("1e300", "1e+300"),
        ] {
            let error = numeric_reject(&format!("type: integer\nmultipleOf: {divisor}"));
            assert!(
                error.contains(displayed)
                    && error.contains("integer-divisor ceiling 9007199254740991"),
                "{divisor}: {error}"
            );
        }

        // Number divisibility deliberately uses shared binary64 `fmod`
        // semantics and does not inherit the safe-integer operand ceiling.
        parse(
            "$schema: https://json-schema.org/draft/2020-12/schema\ntype: object\nproperties:\n  value: { type: number, multipleOf: 1e300 }",
        );
    }

    #[test]
    fn rejects_redundant_same_axis_bounds() {
        let error = numeric_reject("type: integer\nmaximum: 10\nexclusiveMaximum: 12");
        assert!(error.contains("exactly one"), "{error}");
    }

    #[test]
    fn rejects_numeric_bound_on_string_field() {
        let error = numeric_reject("type: string\nmaximum: 5");
        assert!(error.contains("require `type: integer`"), "{error}");
    }

    #[test]
    fn rejects_const_violating_bound() {
        let error = numeric_reject("type: integer\nmaximum: 5\nconst: 7");
        assert!(error.contains("violates the numeric bounds"), "{error}");
    }

    #[test]
    fn rejects_non_number_numeric_bound() {
        let error = numeric_reject("type: integer\nminimum: \"0\"");
        assert!(error.contains("`minimum` must be a number"), "{error}");
    }

    #[test]
    fn rejects_non_number_multiple_of() {
        let error = numeric_reject("type: integer\nmultipleOf: \"2\"");
        assert!(error.contains("`multipleOf` must be a number"), "{error}");
    }

    #[test]
    fn rejects_redundant_minimum_exclusive_minimum() {
        let error = numeric_reject("type: integer\nminimum: 0\nexclusiveMinimum: 2");
        assert!(error.contains("exactly one"), "{error}");
    }

    #[test]
    fn rejects_boolean_exclusive_minimum_form() {
        let error = numeric_reject("type: integer\nminimum: 0\nexclusiveMinimum: true");
        assert!(error.contains("boolean form"), "{error}");
    }

    #[test]
    fn rejects_default_violating_bound() {
        let error = numeric_reject("type: integer\nmaximum: 5\ndefault: 9");
        assert!(error.contains("violates the numeric bounds"), "{error}");
    }

    #[test]
    fn rejects_unsatisfiable_integer_range_with_multiple_of() {
        let error = numeric_reject("type: integer\nminimum: 3\nmaximum: 3\nmultipleOf: 2");
        assert!(error.contains("no multiple of"), "{error}");
    }

    #[test]
    fn rejects_string_length_on_non_string_field() {
        let error = numeric_reject("type: integer\nminLength: 3");
        assert!(error.contains("require `type: string`"), "{error}");
    }

    #[test]
    fn rejects_empty_string_length_interval() {
        let error = numeric_reject("type: string\nminLength: 10\nmaxLength: 2");
        assert!(error.contains("empty range"), "{error}");
    }

    #[test]
    fn rejects_const_string_violating_max_length() {
        let error = numeric_reject("type: string\nmaxLength: 2\nconst: abc");
        assert!(error.contains("exceeding `maxLength`"), "{error}");
    }

    #[test]
    fn rejects_negative_max_length() {
        let error = numeric_reject("type: string\nmaxLength: -1");
        assert!(error.contains("non-negative integer"), "{error}");
    }

    #[test]
    fn rejects_const_below_min_length() {
        let error = numeric_reject("type: string\nminLength: 5\nconst: ab");
        assert!(error.contains("below `minLength`"), "{error}");
    }

    #[test]
    fn rejects_enum_string_violating_max_length() {
        let error = numeric_reject("type: string\nmaxLength: 2\nenum: [ok, toolong]");
        assert!(error.contains("exceeding `maxLength`"), "{error}");
    }

    #[test]
    fn accepts_zero_min_length() {
        numeric_accept("type: string\nminLength: 0");
    }

    #[test]
    fn accepts_valid_string_bounds() {
        let input = r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  code: { type: string, minLength: 2, maxLength: 5 }
  fixed: { type: string, minLength: 3, maxLength: 3 }
  slug: { type: string, maxLength: 12 }
"#;
        parse_api_spec_from_json_schema_for_language(
            Language::Python,
            input,
            PathBuf::from("api.yaml"),
        )
        .expect("valid string bounds should load");
    }

    #[test]
    fn rejects_pattern_backreference() {
        let error = numeric_reject("type: string\npattern: (a)\\1");
        assert!(error.contains("not portable"), "{error}");
    }

    #[test]
    fn rejects_pattern_lookahead() {
        let error = numeric_reject("type: string\npattern: (?=.*[A-Z]).+");
        assert!(error.contains("not portable"), "{error}");
    }

    #[test]
    fn rejects_pattern_inline_flag() {
        let error = numeric_reject("type: string\npattern: (?i)^cat$");
        assert!(error.contains("inline flag"), "{error}");
    }

    #[test]
    fn rejects_pattern_open_complement_class() {
        let error = numeric_reject("type: string\npattern: \"[\\\\S.]\"");
        assert!(error.contains("multi-member"), "{error}");
    }

    #[test]
    fn rejects_pattern_on_non_string_field() {
        let error = numeric_reject("type: integer\npattern: ^\\d+$");
        assert!(error.contains("requires `type: string`"), "{error}");
    }

    #[test]
    fn rejects_const_violating_pattern() {
        let error = numeric_reject("type: string\npattern: ^[a-z]+$\nconst: AB");
        assert!(error.contains("does not match `pattern`"), "{error}");
    }

    #[test]
    fn rejects_non_string_pattern_value() {
        let error = numeric_reject("type: string\npattern: 5");
        assert!(error.contains("`pattern` must be a string"), "{error}");
    }

    #[test]
    fn rejects_enum_violating_pattern() {
        let error = numeric_reject("type: string\npattern: \"^[a-z]+$\"\nenum: [ok, AB]");
        assert!(error.contains("does not match `pattern`"), "{error}");
    }

    #[test]
    fn accepts_empty_pattern() {
        numeric_accept("type: string\npattern: \"\"");
    }

    #[test]
    fn rejects_pattern_lookbehind() {
        let error = numeric_reject("type: string\npattern: \"(?<=x)y\"");
        assert!(error.contains("not portable"), "{error}");
    }

    #[test]
    fn accepts_supported_format() {
        let schema = model_schema(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  id: { type: string, format: uuid }
  site: { type: string, format: uri }
"#,
            "Api",
        );
        assert_eq!(schema["properties"]["id"]["format"], "uuid");
        assert_eq!(schema["properties"]["site"]["format"], "uri");
    }

    #[test]
    fn rejects_format_on_non_string_field() {
        let error = numeric_reject("type: integer\nformat: uuid");
        assert!(error.contains("requires `type: string`"), "{error}");
    }

    #[test]
    fn rejects_non_string_format_value() {
        let error = numeric_reject("type: string\nformat: 5");
        assert!(error.contains("`format` must be a string"), "{error}");
    }

    #[test]
    fn rejects_unknown_format() {
        let error = numeric_reject("type: string\nformat: phone");
        assert!(error.contains("unknown `format: phone`"), "{error}");
        assert!(error.contains("uuid"), "fix-it lists supported: {error}");
    }

    #[test]
    fn rejects_typo_format_as_unknown() {
        let error = numeric_reject("type: string\nformat: datetime");
        assert!(error.contains("unknown `format: datetime`"), "{error}");
    }

    #[test]
    fn rejects_deferred_format() {
        let error = numeric_reject("type: string\nformat: iri");
        assert!(error.contains("not yet supported (deferred)"), "{error}");
    }

    fn numeric_accept(field_schema: &str) {
        let input = format!(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  value:
{}
"#,
            field_schema
                .lines()
                .map(|line| format!("    {line}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
        parse_api_spec_from_json_schema_for_language(
            Language::Python,
            &input,
            PathBuf::from("api.yaml"),
        )
        .expect("schema should load");
    }

    #[test]
    fn accepts_materialized_temporal_formats() {
        for format in ["date-time", "date", "time", "duration"] {
            numeric_accept(&format!("type: string\nformat: {format}"));
        }
    }

    #[test]
    fn accepts_materializable_temporal_const_literals() {
        numeric_accept("type: string\nformat: date-time\nconst: \"2021-06-15T12:30:45Z\"");
        numeric_accept("type: string\nformat: duration\nconst: \"PT1H30M\"");
    }

    #[test]
    fn materialized_temporal_literals_start_at_year_one() {
        numeric_accept("type: string\nformat: date\nconst: \"0001-01-01\"");
        numeric_accept("type: string\nformat: date-time\ndefault: \"0001-01-01T00:00:00Z\"");

        for literal in [
            "type: string\nformat: date\nconst: \"0000-01-01\"",
            "type: string\nformat: date-time\ndefault: \"0000-01-01T00:00:00Z\"",
            "type: string\nformat: date\nenum: [\"0000-01-01\"]",
        ] {
            let error = numeric_reject(literal);
            assert!(error.contains("is not a valid date"), "{error}");
        }
    }

    #[test]
    fn rejects_materialized_leap_second_literal() {
        // Materialized narrowing: `:60` cannot be held by a native type.
        let error =
            numeric_reject("type: string\nformat: date-time\nconst: \"2021-12-31T23:59:60Z\"");
        assert!(error.contains("is not a valid date-time"), "{error}");
        let error = numeric_reject("type: string\nformat: time\nconst: \"23:59:60Z\"");
        assert!(error.contains("is not a valid time"), "{error}");
    }

    #[test]
    fn rejects_materialized_calendar_duration_literal() {
        // Materialized narrowing: `duration` is time-only (no Y/M/W/D).
        for literal in ["P1Y", "P4W", "P1D"] {
            let error = numeric_reject(&format!(
                "type: string\nformat: duration\nconst: \"{literal}\""
            ));
            assert!(
                error.contains("is not a valid duration"),
                "{literal}: {error}"
            );
        }
    }

    #[test]
    fn rejects_missing_offset_date_time_literal() {
        // Materialized `date-time` requires an offset.
        let error =
            numeric_reject("type: string\nformat: date-time\nconst: \"2021-06-15T12:30:45\"");
        assert!(error.contains("is not a valid date-time"), "{error}");
    }

    #[test]
    fn rejects_const_violating_format() {
        let error = numeric_reject("type: string\nformat: uuid\nconst: not-a-uuid");
        assert!(error.contains("is not a valid uuid"), "{error}");
    }

    #[test]
    fn rejects_default_violating_format() {
        let error = numeric_reject("type: string\nformat: ipv4\ndefault: 256.0.0.1");
        assert!(error.contains("is not a valid ipv4"), "{error}");
    }

    #[test]
    fn rejects_enum_violating_format() {
        let error = numeric_reject("type: string\nformat: uuid\nenum: [not-a-uuid]");
        assert!(error.contains("is not a valid uuid"), "{error}");
    }

    #[test]
    fn accepts_materialized_content_encodings() {
        for encoding in ["base64", "base64url"] {
            numeric_accept(&format!("type: string\ncontentEncoding: {encoding}"));
        }
    }

    #[test]
    fn rejects_temporal_format_alongside_content_encoding() {
        for format in ["date-time", "date", "time", "duration"] {
            let error = numeric_reject(&format!(
                "type: string\nformat: {format}\ncontentEncoding: base64"
            ));
            assert!(
                error.contains(&format!("materializing `format: {format}`")),
                "{format}: {error}"
            );
            assert!(error.contains("contentEncoding"), "{format}: {error}");
        }
    }

    #[test]
    fn accepts_string_shaped_format_alongside_content_encoding() {
        numeric_accept("type: string\nformat: uri-reference\ncontentEncoding: base64");
    }

    #[test]
    fn accepts_valid_content_encoding_const_literals() {
        // ">>>" canonical padded standard / unpadded URL-safe.
        numeric_accept("type: string\ncontentEncoding: base64\nconst: \"Pj4+\"");
        numeric_accept("type: string\ncontentEncoding: base64url\nconst: \"Pj4-\"");
    }

    #[test]
    fn rejects_closed_literals_on_nullable_content_encoding_wrapper() {
        for literal in ["const: aGk=", "enum: [aGk=]"] {
            let error = numeric_reject(&format!(
                "oneOf:\n  - {{ type: string, contentEncoding: base64 }}\n  - {{ type: 'null' }}\n{literal}"
            ));
            assert!(error.contains("cannot be a sibling of `oneOf`"), "{error}");
            assert!(error.contains("move it into the branch"), "{error}");
        }
    }

    #[test]
    fn rejects_content_encoding_on_non_string_field() {
        let error = numeric_reject("type: integer\ncontentEncoding: base64");
        assert!(error.contains("requires `type: string`"), "{error}");
    }

    #[test]
    fn rejects_non_string_content_encoding_value() {
        let error = numeric_reject("type: string\ncontentEncoding: 5");
        assert!(
            error.contains("`contentEncoding` must be a string"),
            "{error}"
        );
    }

    #[test]
    fn rejects_unsupported_content_encoding() {
        for encoding in [
            "base32",
            "base16",
            "quoted-printable",
            "7bit",
            "8bit",
            "binary",
        ] {
            let error = numeric_reject(&format!("type: string\ncontentEncoding: {encoding}"));
            assert!(
                error.contains(&format!("`contentEncoding: {encoding}` is not supported")),
                "{error}"
            );
            assert!(error.contains("base64"), "fix-it lists supported: {error}");
        }
    }

    #[test]
    fn rejects_content_media_type_alongside_content_encoding() {
        let error =
            numeric_reject("type: string\ncontentEncoding: base64\ncontentMediaType: image/png");
        assert!(error.contains("contentMediaType"), "{error}");
        assert!(error.contains("not supported"), "{error}");
    }

    #[test]
    fn rejects_const_violating_content_encoding() {
        // URL-safe chars under `base64`.
        let error = numeric_reject("type: string\ncontentEncoding: base64\nconst: \"a-b_\"");
        assert!(
            error.contains("is not valid base64-encoded data"),
            "{error}"
        );
        // Padding under `base64url`.
        let error = numeric_reject("type: string\ncontentEncoding: base64url\nconst: \"aGk=\"");
        assert!(
            error.contains("is not valid base64url-encoded data"),
            "{error}"
        );
    }

    #[test]
    fn rejects_enum_violating_content_encoding() {
        let error = numeric_reject("type: string\ncontentEncoding: base64\nenum: [\"a-b_\"]");
        assert!(
            error.contains("is not valid base64-encoded data"),
            "{error}"
        );
    }

    #[test]
    fn accepts_and_normalizes_perl_space_pattern() {
        let schema = model_schema(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  phrase: { type: string, pattern: "^\\S+\\s\\S+$" }
"#,
            "Api",
        );
        // `\s`/`\S` are expanded to the explicit ASCII class in the loader; `$`
        // stays canonical for the per-target backend rewrite.
        assert_eq!(
            schema["properties"]["phrase"]["pattern"],
            "^[^\\t\\n\\x0B\\f\\r ]+[\\t\\n\\x0B\\f\\r ][^\\t\\n\\x0B\\f\\r ]+$"
        );
    }

    #[test]
    fn rejects_array_keyword_on_non_array_field() {
        let error = numeric_reject("type: string\nminItems: 1");
        assert!(error.contains("require `type: array`"), "{error}");
    }

    #[test]
    fn rejects_empty_items_interval() {
        let error =
            numeric_reject("type: array\nitems: { type: string }\nminItems: 5\nmaxItems: 2");
        assert!(error.contains("empty range"), "{error}");
    }

    #[test]
    fn rejects_unique_items_on_object_element_array() {
        let error = numeric_reject(
            "type: array\nitems: { type: object, properties: {} }\nuniqueItems: true",
        );
        assert!(error.contains("not yet supported"), "{error}");
    }

    #[test]
    fn rejects_contains_with_composite_matcher() {
        let error = numeric_reject(
            "type: array\nitems: { type: object, properties: {} }\ncontains: { type: object }",
        );
        assert!(error.contains("not yet supported"), "{error}");
    }

    #[test]
    fn rejects_min_contains_without_contains() {
        let error = numeric_reject("type: array\nitems: { type: string }\nminContains: 2");
        assert!(error.contains("require a sibling `contains`"), "{error}");
    }

    #[test]
    fn rejects_shapeless_contains_matcher() {
        let error = numeric_reject("type: array\nitems: { type: string }\ncontains: {}");
        assert!(error.contains("not a matcher"), "{error}");
    }

    #[test]
    fn rejects_type_incompatible_contains_matcher() {
        let error =
            numeric_reject("type: array\nitems: { type: string }\ncontains: { type: integer }");
        assert!(
            error.contains("incompatible with the element type"),
            "{error}"
        );
    }

    #[test]
    fn recursively_validates_contains_scalar_matcher_constraints() {
        for (constraint, schema, expected) in [
            (
                "fractional multipleOf",
                "type: array\nitems: { type: number }\ncontains: { type: number, multipleOf: 0.1 }",
                "fractional divisors are deferred",
            ),
            (
                "fractional integer bound",
                "type: array\nitems: { type: integer }\ncontains: { type: integer, minimum: 1.5 }",
                "integer bound",
            ),
            (
                "negative string length",
                "type: array\nitems: { type: string }\ncontains: { type: string, minLength: -1 }",
                "non-negative integer",
            ),
            (
                "unknown asserted format",
                "type: array\nitems: { type: string }\ncontains: { type: string, format: made-up }",
                "unknown `format",
            ),
            (
                "literal violating matcher bound",
                "type: array\nitems: { type: integer }\ncontains: { minimum: 5, const: 2 }",
                "must be >= 5",
            ),
            (
                "categorically rejected not",
                "type: array\nitems: { type: string }\ncontains: { type: string, not: { const: x } }",
                "`not` is not supported",
            ),
            (
                "unsupported content encoding predicate",
                "type: array\nitems: { type: string }\ncontains: { type: string, contentEncoding: base64 }",
                "not supported in a scalar matcher",
            ),
            (
                "array assertion on scalar matcher",
                "type: array\nitems: { type: string }\ncontains: { type: string, minItems: 1 }",
                "require `type: array`",
            ),
            (
                "union hidden beside scalar assertion",
                "type: array\nitems: { type: string }\ncontains: { oneOf: [{ type: string }, { type: integer }], const: x }",
                "composite `contains` matcher",
            ),
        ] {
            let error = numeric_reject(schema);
            assert!(
                error.contains(expected) && error.contains(".contains"),
                "{constraint}: expected {expected}, got {error}"
            );
        }
    }

    #[test]
    fn rejects_vacuous_min_contains_zero() {
        let error = numeric_reject(
            "type: array\nitems: { type: string }\ncontains: { const: x }\nminContains: 0",
        );
        assert!(error.contains("assert nothing"), "{error}");
    }

    #[test]
    fn rejects_max_contains_zero_at_default_min() {
        let error = numeric_reject(
            "type: array\nitems: { type: string }\ncontains: { const: x }\nmaxContains: 0",
        );
        assert!(error.contains("empty range"), "{error}");
    }

    #[test]
    fn rejects_non_integer_min_items() {
        let error = numeric_reject("type: array\nitems: { type: string }\nminItems: -1");
        assert!(error.contains("non-negative integer"), "{error}");
    }

    #[test]
    fn rejects_non_integer_max_contains() {
        let error = numeric_reject(
            "type: array\nitems: { type: string }\ncontains: { const: x }\nmaxContains: -1",
        );
        assert!(error.contains("non-negative integer"), "{error}");
    }

    #[test]
    fn rejects_non_boolean_unique_items() {
        let error = numeric_reject("type: array\nitems: { type: string }\nuniqueItems: \"true\"");
        assert!(error.contains("`uniqueItems` must be a boolean"), "{error}");
    }

    #[test]
    fn rejects_min_contains_above_max_contains() {
        let error = numeric_reject(
            "type: array\nitems: { type: string }\ncontains: { const: x }\nminContains: 3\nmaxContains: 1",
        );
        assert!(error.contains("exceeds `maxContains`"), "{error}");
    }

    #[test]
    fn rejects_max_contains_without_contains() {
        let error = numeric_reject("type: array\nitems: { type: string }\nmaxContains: 2");
        assert!(error.contains("require a sibling `contains`"), "{error}");
    }

    #[test]
    fn rejects_non_schema_contains_value() {
        let error = numeric_reject("type: array\nitems: { type: string }\ncontains: 5");
        assert!(error.contains("must be a schema object"), "{error}");
    }

    #[test]
    fn accepts_valid_array_constraints() {
        let input = r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  tags: { type: array, items: { type: string }, minItems: 1, maxItems: 5 }
  aliases: { type: array, items: { type: string }, uniqueItems: true }
  roles:
    type: array
    items: { type: string }
    contains: { const: admin }
    minContains: 1
    maxContains: 2
"#;
        parse_api_spec_from_json_schema_for_language(
            Language::Python,
            input,
            PathBuf::from("api.yaml"),
        )
        .expect("valid array constraints should load");
    }

    /// An integral float count bound is accepted (`minItems: 2.0` is `2`), and
    /// the normalize pass must canonicalize it to an integer: every backend
    /// deserializes these into `Option<u64>`/`Option<usize>` and serde refuses a
    /// JSON float there.
    #[test]
    fn canonicalizes_integral_float_count_bounds() {
        let schema = model_schema(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
minProperties: 1.0
maxProperties: 8.0
properties:
  tags:
    type: array
    items: { type: string }
    minItems: 2.0
    maxItems: 4.0
    contains: { const: admin }
    minContains: 1.0
    maxContains: 2.0
  name: { type: string, minLength: 1.0, maxLength: 12.0 }
"#,
            "Api",
        );
        for (node, keyword) in [
            (&schema, "minProperties"),
            (&schema, "maxProperties"),
            (&schema["properties"]["tags"], "minItems"),
            (&schema["properties"]["tags"], "maxItems"),
            (&schema["properties"]["tags"], "minContains"),
            (&schema["properties"]["tags"], "maxContains"),
            (&schema["properties"]["name"], "minLength"),
            (&schema["properties"]["name"], "maxLength"),
        ] {
            let value = &node[keyword];
            assert!(
                value.is_u64(),
                "`{keyword}` should normalize to an integer, got {value}"
            );
        }
        assert_eq!(schema["properties"]["tags"]["minItems"], 2);
        assert_eq!(schema["properties"]["name"]["maxLength"], 12);
    }

    #[test]
    fn accepts_valid_numeric_bounds() {
        let input = r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  bounded: { type: integer, minimum: 1, maximum: 10 }
  strict: { type: integer, exclusiveMinimum: 0 }
  ratio: { type: number, minimum: 5, multipleOf: 5 }
  stepped: { type: integer, multipleOf: 3 }
"#;
        parse_api_spec_from_json_schema_for_language(
            Language::Python,
            input,
            PathBuf::from("api.yaml"),
        )
        .expect("valid numeric bounds should load");
    }

    #[test]
    fn rejects_object_keyword_on_non_object_field() {
        let error = numeric_reject("type: string\nminProperties: 1");
        assert!(error.contains("require `type: object`"), "{error}");
    }

    #[test]
    fn rejects_empty_property_interval() {
        let error = numeric_reject(
            "type: object\nadditionalProperties: true\nminProperties: 5\nmaxProperties: 2",
        );
        assert!(error.contains("empty range"), "{error}");
    }

    #[test]
    fn rejects_min_properties_above_closed_object_capacity() {
        let error = numeric_reject(
            "type: object\nadditionalProperties: false\nproperties: { a: { type: string } }\nminProperties: 2",
        );
        assert!(error.contains("closed object"), "{error}");
    }

    #[test]
    fn rejects_positive_min_properties_with_optional_nullable_members() {
        let nullable = "oneOf: [{ type: string }, { type: 'null' }]";
        let error = numeric_reject(&format!(
            "type: object\nproperties: {{ note: {{ {nullable} }} }}\nminProperties: 1"
        ));
        assert!(
            error.contains("optional nullable property `note`"),
            "{error}"
        );

        // A zero floor asserts nothing, and a required nullable key cannot
        // collapse to absence on a valid wire value.
        numeric_accept(&format!(
            "type: object\nproperties: {{ note: {{ {nullable} }} }}\nminProperties: 0"
        ));
        numeric_accept(&format!(
            "type: object\nproperties: {{ note: {{ {nullable} }} }}\nrequired: [note]\nminProperties: 1"
        ));
    }

    #[test]
    fn rejects_min_properties_with_optional_nullable_ref_alias() {
        let error = doc_reject(
            r##"
$schema: https://json-schema.org/draft/2020-12/schema
title: Root
type: object
minProperties: 1
properties:
  note: { $ref: "#/$defs/NullableAlias" }
$defs:
  NullableAlias: { $ref: "#/$defs/Nullable" }
  Nullable:
    oneOf: [{ type: string }, { type: "null" }]
"##,
        );
        assert!(
            error.contains("optional nullable property `note`"),
            "{error}"
        );
    }

    #[test]
    fn rejects_min_properties_above_finite_property_names_capacity() {
        for (matcher, capacity, floor) in [
            ("{ type: string, enum: [a, b] }", 2, 3),
            ("{ type: string, maxLength: 0 }", 1, 2),
            ("{ type: string, maxLength: 0.0 }", 1, 2),
            ("{ type: string, enum: [''], maxLength: 0 }", 1, 2),
        ] {
            let error = numeric_reject(&format!(
                "type: object\nadditionalProperties: true\nminProperties: {floor}\npropertyNames: {matcher}"
            ));
            assert!(error.contains("finite `propertyNames`"), "{error}");
            assert!(error.contains(&format!("capacity ({capacity})")), "{error}");
        }
    }

    #[test]
    fn accepts_min_properties_at_finite_property_names_capacity_and_pattern_only_space() {
        numeric_accept(
            "type: object\nadditionalProperties: true\nminProperties: 2\npropertyNames: { type: string, enum: [a, b] }",
        );
        numeric_accept(
            "type: object\nadditionalProperties: true\nminProperties: 1\npropertyNames: { type: string, maxLength: 0 }",
        );
        numeric_accept(
            "type: object\nadditionalProperties: true\nminProperties: 999\npropertyNames: { type: string, pattern: '^x' }",
        );
    }

    #[test]
    fn max_length_zero_property_names_capacity_honors_sibling_assertions() {
        for excluded_by in [
            "pattern: '^a$'",
            "pattern: 'a'",
            "format: email",
            "format: hostname",
        ] {
            let error = numeric_reject(&format!(
                "type: object\nadditionalProperties: true\nminProperties: 1\npropertyNames: {{ type: string, maxLength: 0, {excluded_by} }}"
            ));
            assert!(error.contains("capacity (0)"), "{excluded_by}: {error}");
        }

        for admitted_by in ["pattern: '^$'", "pattern: ''", "enum: ['']"] {
            numeric_accept(&format!(
                "type: object\nadditionalProperties: true\nminProperties: 1\npropertyNames: {{ type: string, maxLength: 0, {admitted_by} }}"
            ));
        }
    }

    #[test]
    fn rejects_property_names_alongside_properties() {
        let error = numeric_reject(
            "type: object\nproperties: { id: { type: string } }\npropertyNames: { type: string, maxLength: 8 }",
        );
        assert!(error.contains("map-shaped object"), "{error}");
    }

    #[test]
    fn rejects_non_string_property_names() {
        let error = numeric_reject(
            "type: object\nadditionalProperties: true\npropertyNames: { type: integer }",
        );
        assert!(error.contains("must be `type: string`"), "{error}");
    }

    #[test]
    fn rejects_shapeless_property_names() {
        let error = numeric_reject(
            "type: object\nadditionalProperties: true\npropertyNames: { type: string }",
        );
        assert!(error.contains("asserts nothing"), "{error}");
    }

    #[test]
    fn rejects_dependent_required_undeclared_reference() {
        let error = numeric_reject(
            "type: object\nproperties: { a: { type: string } }\ndependentRequired: { a: [b] }",
        );
        assert!(error.contains("not declared in `properties`"), "{error}");
    }

    #[test]
    fn rejects_dependent_required_trigger_in_required() {
        let error = numeric_reject(
            "type: object\nproperties: { a: { type: string }, b: { type: string } }\nrequired: [a]\ndependentRequired: { a: [b] }",
        );
        assert!(error.contains("also in `required`"), "{error}");
    }

    #[test]
    fn rejects_dependent_required_dependent_in_required() {
        let error = numeric_reject(
            "type: object\nproperties: { a: { type: string }, b: { type: string } }\nrequired: [b]\ndependentRequired: { a: [b] }",
        );
        assert!(error.contains("already in `required`"), "{error}");
    }

    #[test]
    fn rejects_dependent_required_non_unique_dependents() {
        let error = numeric_reject(
            "type: object\nproperties: { a: { type: string }, b: { type: string } }\ndependentRequired: { a: [b, b] }",
        );
        assert!(error.contains("more than once"), "{error}");
    }

    #[test]
    fn rejects_dependent_required_optional_nullable_edges() {
        let trigger = numeric_reject(
            "type: object\nproperties:\n  a: { oneOf: [{ type: string }, { type: 'null' }] }\n  b: { type: string }\ndependentRequired: { a: [b] }",
        );
        assert!(
            trigger.contains("trigger `a` cannot be optional and nullable"),
            "{trigger}"
        );

        let dependent = numeric_reject(
            "type: object\nproperties:\n  a: { type: string }\n  b: { oneOf: [{ type: string }, { type: 'null' }] }\ndependentRequired: { a: [b] }",
        );
        assert!(
            dependent.contains("dependent `b` cannot be optional and nullable"),
            "{dependent}"
        );

        // A vacuous edge emits no presence predicate and therefore has no
        // collapse ambiguity.
        numeric_accept(
            "type: object\nproperties:\n  a: { oneOf: [{ type: string }, { type: 'null' }] }\ndependentRequired: { a: [] }",
        );
    }

    #[test]
    fn rejects_non_integer_min_properties() {
        let error = numeric_reject("type: object\nadditionalProperties: true\nminProperties: -1");
        assert!(error.contains("non-negative integer"), "{error}");
    }

    #[test]
    fn rejects_property_count_above_safe_integer_cap() {
        let error = numeric_reject(
            "type: object\nadditionalProperties: true\nmaxProperties: 9007199254740992",
        );
        assert!(
            error.contains("maxProperties") && error.contains("9007199254740991"),
            "{error}"
        );
    }

    #[test]
    fn rejects_unsafe_property_counts_before_recursive_or_all_of_lowering() {
        for (position, schema) in [
            (
                "nested property",
                "type: object\nproperties:\n  nested:\n    type: object\n    additionalProperties: true\n    minProperties: 9007199254740992",
            ),
            (
                "array items",
                "type: array\nitems:\n  type: object\n  additionalProperties: true\n  maxProperties: 9007199254740992",
            ),
            (
                "typed additionalProperties",
                "type: object\nadditionalProperties:\n  type: object\n  additionalProperties: true\n  minProperties: 9007199254740992",
            ),
            (
                "allOf branch overwritten by a later bound",
                "allOf:\n  - { type: object, additionalProperties: true, maxProperties: 9007199254740992 }\n  - { type: object, maxProperties: 4 }",
            ),
        ] {
            let error = structural_reject(schema);
            assert!(
                error.contains("Properties") && error.contains("9007199254740991"),
                "{position}: {error}"
            );
        }

        let error = doc_reject(
            r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  value:
    $ref: "#/$defs/Base"
    maxProperties: 9007199254740992
$defs:
  Base:
    type: object
    additionalProperties: true
"##,
        );
        assert!(
            error.contains("maxProperties") && error.contains("9007199254740991"),
            "$ref sibling: {error}"
        );
    }

    #[test]
    fn rejects_max_properties_below_required_count() {
        let error = numeric_reject(
            "type: object\nproperties: { a: {type: string}, b: {type: string}, c: {type: string} }\nrequired: [a, b, c]\nmaxProperties: 2",
        );
        assert!(error.contains("is below the"), "{error}");
    }

    #[test]
    fn rejects_max_properties_below_dependent_required_closure() {
        for dependent_required in [
            "{ a: [b, c] }",
            "{ a: [b], b: [c] }",
            "{ a: [b], b: [c], c: [a] }",
        ] {
            let error = numeric_reject(&format!(
                "type: object\nproperties: {{ a: {{type: string}}, b: {{type: string}}, c: {{type: string}} }}\nmaxProperties: 2\ndependentRequired: {dependent_required}"
            ));
            assert!(error.contains("3-member closure"), "{error}");
            assert!(error.contains("maxProperties"), "{error}");
        }
    }

    #[test]
    fn accepts_dependent_required_closures_within_max_properties() {
        numeric_accept(
            "type: object\nproperties: { a: {type: string}, b: {type: string}, c: {type: string} }\nmaxProperties: 3\ndependentRequired: { a: [b], b: [c] }",
        );
        numeric_accept(
            "type: object\nproperties: { a: {type: string}, b: {type: string}, c: {type: string}, d: {type: string} }\nmaxProperties: 2\ndependentRequired: { a: [b], c: [d] }",
        );
    }

    #[test]
    fn dependent_required_capacity_includes_always_required_keys() {
        let base = "type: object\nproperties: { a: {type: string}, b: {type: string}, c: {type: string}, d: {type: string} }\nrequired: [d]\ndependentRequired: { a: [b], b: [c] }";
        let error = numeric_reject(&format!("{base}\nmaxProperties: 3"));
        assert!(error.contains("4-member closure"), "{error}");
        assert!(error.contains("a, b, c, d"), "{error}");

        numeric_accept(&format!("{base}\nmaxProperties: 4"));
    }

    #[test]
    fn rejects_property_names_without_map_host() {
        let error = numeric_reject(
            "type: object\nadditionalProperties: false\npropertyNames: { type: string, maxLength: 8 }",
        );
        assert!(error.contains("requires a map host"), "{error}");
    }

    #[test]
    fn rejects_bare_true_property_names() {
        let error = numeric_reject("type: object\nadditionalProperties: true\npropertyNames: true");
        assert!(error.contains("string schema constraining"), "{error}");
    }

    #[test]
    fn accepts_documented_property_names_assertions() {
        for matcher in [
            "{ type: string, minLength: 2, maxLength: 8 }",
            "{ type: string, pattern: \"^x\" }",
            "{ type: string, enum: [x, xy] }",
            "{ type: string, format: hostname }",
        ] {
            let input =
                format!("type: object\nadditionalProperties: true\npropertyNames: {matcher}");
            let doc = format!(
                "$schema: https://json-schema.org/draft/2020-12/schema\ntype: object\nproperties:\n  value:\n{}",
                input
                    .lines()
                    .map(|line| format!("    {line}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            );
            parse_api_spec_from_json_schema_for_language(
                Language::Python,
                &doc,
                PathBuf::from("api.yaml"),
            )
            .unwrap_or_else(|error| panic!("matcher {matcher} should load: {error}"));
        }
    }

    #[test]
    fn rejects_dependent_required_value_not_object() {
        let error = numeric_reject(
            "type: object\nproperties: { a: {type: string} }\ndependentRequired: []",
        );
        assert!(error.contains("object mapping"), "{error}");
    }

    #[test]
    fn rejects_dependent_required_value_not_array() {
        let error = numeric_reject(
            "type: object\nproperties: { a: {type: string} }\ndependentRequired: { a: b }",
        );
        assert!(
            error.contains("must be an array of property-name strings"),
            "{error}"
        );
    }

    #[test]
    fn rejects_dependent_required_non_string_element() {
        let error = numeric_reject(
            "type: object\nproperties: { a: {type: string} }\ndependentRequired: { a: [1] }",
        );
        assert!(error.contains("property-name strings"), "{error}");
    }

    #[test]
    fn rejects_dependent_required_undeclared_trigger() {
        let error = numeric_reject(
            "type: object\nproperties: { b: {type: string} }\ndependentRequired: { a: [b] }",
        );
        assert!(error.contains("trigger `a`"), "{error}");
        assert!(error.contains("not declared"), "{error}");
    }

    #[test]
    fn accepts_valid_object_constraints() {
        let input = r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
minProperties: 1
maxProperties: 6
properties:
  a: { type: string }
  b: { type: string }
  c: { type: string }
dependentRequired:
  a: [b]
"#;
        parse_api_spec_from_json_schema_for_language(
            Language::Python,
            input,
            PathBuf::from("api.yaml"),
        )
        .expect("valid object constraints should load");
    }

    #[test]
    fn accepts_valid_property_names_map() {
        let input = r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
additionalProperties: { type: string }
propertyNames: { type: string, maxLength: 8 }
"#;
        parse_api_spec_from_json_schema_for_language(
            Language::Python,
            input,
            PathBuf::from("api.yaml"),
        )
        .expect("valid propertyNames map should load");
    }

    #[test]
    fn resolves_refs_across_input_files() {
        let spec = api_spec_from_json_schema_sources(
            Language::Python,
            vec![
                (
                    PathBuf::from("main.yaml"),
                    r##"
nexusrpc: "1.0.0"
services:
  ChatService:
    operations:
      getRoom:
        input: { $ref: "types.yaml#/$defs/GetRoomInput" }
"##
                    .to_string(),
                ),
                (
                    PathBuf::from("types.yaml"),
                    r##"
nexusrpc: "1.0.0"
$defs:
  GetRoomInput:
    type: object
    properties:
      roomId: { type: string }
"##
                    .to_string(),
                ),
            ],
        )
        .unwrap();

        let Some(TypeSpec::External(ExternalTypeSpec::Json(input))) =
            &spec.services[0].operations[0].input
        else {
            panic!("input should be a JSON external model");
        };
        assert_eq!(input.name.as_str(), "GetRoomInput");
    }

    /// A minimal pure JSON Schema document whose root type is named `title`.
    fn module_collision_source(path: &str, title: &str) -> JsonSource {
        JsonSource {
            path: PathBuf::from(path),
            source_root: PathBuf::from("."),
            relative_path: PathBuf::from(path),
            input: format!("title: {title}\ntype: object\nproperties:\n  id: {{ type: string }}\n"),
        }
    }

    #[test]
    fn rejects_two_sources_with_the_same_module_path() {
        // `foo.yaml` and `foo.json` are distinct input files but both strip to
        // module path `foo`, so the second leaf collides with the first.
        let sources = vec![
            module_collision_source("foo.yaml", "FooYaml"),
            module_collision_source("foo.json", "FooJson"),
        ];
        let error = api_spec_tree_from_json_schema_sources(Language::Python, sources)
            .expect_err("two sources mapping to the same module path should be rejected")
            .to_string();
        assert!(
            error.contains("foo.yaml")
                && error.contains("foo.json")
                && error.contains("rename one input"),
            "{error}"
        );
    }

    #[test]
    fn rejects_source_module_path_conflicting_with_a_branch() {
        // `foo.yaml` occupies leaf `foo`; `foo/bar.yaml` then needs `foo` to be a
        // branch, so its insertion conflicts with the existing module.
        let sources = vec![
            module_collision_source("foo.yaml", "Foo"),
            module_collision_source("foo/bar.yaml", "Bar"),
        ];
        let error = api_spec_tree_from_json_schema_sources(Language::Python, sources)
            .expect_err("a source colliding with an existing module branch should be rejected")
            .to_string();
        assert!(
            error.contains("foo.yaml")
                && error.contains("foo/bar.yaml")
                && error.contains("rename one input"),
            "{error}"
        );

        // The diagnostic is symmetric: inserting the directory-shaped module
        // first must retain its authored source when the shorter leaf arrives.
        let sources = vec![
            module_collision_source("foo/bar.yaml", "Bar"),
            module_collision_source("foo.yaml", "Foo"),
        ];
        let error = api_spec_tree_from_json_schema_sources(Language::Python, sources)
            .expect_err("a module branch colliding with a later leaf should be rejected")
            .to_string();
        assert!(
            error.contains("foo.yaml")
                && error.contains("foo/bar.yaml")
                && error.contains("rename one input"),
            "{error}"
        );
    }

    #[test]
    fn rejects_remote_http_ref() {
        let error = numeric_reject("$ref: \"https://example.com/s.json\"");
        assert!(error.contains("remote `$ref`"), "{error}");
    }

    #[test]
    fn rejects_ref_into_non_defs() {
        let error = numeric_reject("$ref: \"#/properties/x/items\"");
        assert!(
            error.contains("must point at a `$defs` entry or file root"),
            "{error}"
        );
    }

    #[test]
    fn rejects_unresolvable_defs_ref() {
        let error = numeric_reject("$ref: \"#/$defs/Missing\"");
        assert!(
            error.contains("properties.value")
                && error.contains("declares no `$defs.Missing` entry")
                && error.contains("add that `$defs` entry")
                && error.contains("correct the JSON Pointer"),
            "{error}"
        );
    }

    #[test]
    fn validates_refs_inside_schema_valued_additional_properties() {
        for (reference, expected) in [
            ("#/$defs/Missing", "declares no `$defs.Missing` entry"),
            ("#/$defs/bad~2name", "invalid RFC 6901 escape `~2`"),
        ] {
            let error = doc_reject(&format!(
                "$schema: https://json-schema.org/draft/2020-12/schema\ntype: object\nadditionalProperties: {{ $ref: {reference:?} }}"
            ));
            assert!(
                error.contains("Api.additionalProperties") && error.contains(expected),
                "{reference}: {error}"
            );
        }

        parse(
            r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
additionalProperties: { $ref: "#/$defs/Value" }
$defs:
  Value:
    type: object
    properties: { text: { type: string } }
"##,
        );
    }

    #[test]
    fn ref_pointer_rejections_name_the_schema_position_and_remedy() {
        for (reference, detail, remedy) in [
            (
                "#/properties/x",
                "must point at a `$defs` entry or file root",
                "extract the target into `$defs`",
            ),
            (
                "#/$defs/bad~",
                "trailing `~`",
                "use `~0` for `~` or `~1` for `/`",
            ),
            (
                "#/$defs/bad~2name",
                "invalid RFC 6901 escape `~2`",
                "use `~0` for `~` or `~1` for `/`",
            ),
        ] {
            let error = numeric_reject(&format!("$ref: {reference:?}"));
            assert!(
                error.contains("properties.value")
                    && error.contains(detail)
                    && error.contains(remedy),
                "{reference}: {error}"
            );
        }
    }

    #[test]
    fn missing_ref_file_root_names_the_definitions_only_target_and_fix() {
        let temp = tempfile::tempdir().unwrap();
        let entry = temp.path().join("entry.yaml");
        let definitions = temp.path().join("definitions.yaml");
        fs::write(
            &entry,
            r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  child: { $ref: "definitions.yaml#" }
"##,
        )
        .unwrap();
        fs::write(
            &definitions,
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
$defs:
  Child:
    type: object
    properties:
      value: { type: string }
"#,
        )
        .unwrap();

        let error = load_api_spec_from_json_schema_for_language_with_inputs(
            Language::Python,
            std::slice::from_ref(&entry),
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("properties.child")
                && error.contains("definitions.yaml")
                && error.contains("declares no root schema")
                && error.contains("point the reference at one of the file's `$defs` entries"),
            "{error}"
        );
    }

    /// Parses a single object property `value` carrying `field_schema` and
    /// returns the load error string (for the `const`/`enum` reject cases).
    fn const_enum_reject(field_schema: &str) -> String {
        numeric_reject(field_schema)
    }

    /// Loads a schema with the given field schema under `properties.value` and
    /// returns the load error message (panicking if it unexpectedly succeeds).
    fn structural_reject(field_schema: &str) -> String {
        numeric_reject(field_schema)
    }

    #[test]
    fn rejects_structural_keywords_with_fixits() {
        // Combinator / applicator keywords with no supported lowering, plus the
        // metadata keywords that reject (directional / content). Each must fail
        // to load and name the offending keyword (P6/P7.1). See the reject specs.
        for (keyword, field_schema) in [
            ("anyOf", "anyOf: [{ type: string }]"),
            ("not", "type: string\nnot: { type: integer }"),
            ("if", "type: string\nif: { type: string }"),
            ("then", "type: string\nthen: { type: string }"),
            ("else", "type: string\nelse: { type: string }"),
            (
                "prefixItems",
                "type: array\nprefixItems: [{ type: string }]",
            ),
            ("unevaluatedItems", "type: array\nunevaluatedItems: false"),
            (
                "unevaluatedProperties",
                "type: object\nunevaluatedProperties: false",
            ),
            (
                "dependentSchemas",
                "type: object\ndependentSchemas: { a: { type: object } }",
            ),
            (
                "patternProperties",
                "type: object\npatternProperties: { \"^x\": { type: string } }",
            ),
            ("readOnly", "type: string\nreadOnly: true"),
            ("writeOnly", "type: string\nwriteOnly: true"),
            (
                "contentMediaType",
                "type: string\ncontentMediaType: image/png",
            ),
            (
                "contentSchema",
                "type: string\ncontentSchema: { type: object }",
            ),
            ("$id", "type: string\n$id: \"http://x\""),
            ("$anchor", "type: string\n$anchor: foo"),
            ("$dynamicRef", "type: string\n$dynamicRef: \"#foo\""),
            ("$dynamicAnchor", "type: string\n$dynamicAnchor: foo"),
            ("$vocabulary", "type: string\n$vocabulary: { \"x\": true }"),
        ] {
            let error = structural_reject(field_schema);
            assert!(
                error.contains(keyword) && error.contains("not supported"),
                "expected `{keyword}` reject, got: {error}"
            );
        }
    }

    #[test]
    fn rejects_read_only_false() {
        let error = structural_reject("type: string\nreadOnly: false");
        assert!(
            error.contains("`readOnly`/`writeOnly` is not supported"),
            "{error}"
        );
    }

    #[test]
    fn rejects_nullable_keyword() {
        let error = structural_reject("type: string\nnullable: true");
        assert!(error.contains("`nullable` is not supported"), "{error}");
    }

    #[test]
    fn rejects_array_type_form() {
        let error = structural_reject("type: [string, \"null\"]");
        assert!(error.contains("array `type`"), "{error}");
        assert!(error.contains("oneOf"), "{error}");
    }

    #[test]
    fn rejects_standalone_null_type() {
        let error = structural_reject("type: \"null\"");
        assert!(error.contains("standalone `type: \"null\"`"), "{error}");
        assert!(error.contains("oneOf"), "{error}");
    }

    #[test]
    fn accepts_null_type_in_nullability_one_of() {
        // The one legal home for `type: "null"`: a nullability `oneOf` branch.
        parse(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  middleName:
    oneOf:
      - { type: string }
      - { type: "null" }
"#,
        );
    }

    #[test]
    fn rejects_null_branch_with_sibling_keywords() {
        for (keyword, sibling) in [
            ("description", "description: not exact"),
            ("deprecated", "deprecated: true"),
            ("default", "default: null"),
            ("inert comment", "$comment: still not exact"),
            ("extension", "x-java-name: NullValue"),
        ] {
            let error = structural_reject(&format!(
                "oneOf:\n  - {{ type: string }}\n  - {{ type: \"null\", {sibling} }}"
            ));
            assert!(
                error.contains("null branch") && error.contains("exactly `{type: \"null\"}`"),
                "{keyword}: {error}"
            );
        }
    }

    #[test]
    fn rejects_nullable_default_invalid_for_non_null_branch() {
        for (constraint, schema, expected) in [
            (
                "type",
                "oneOf:\n  - { type: string }\n  - { type: \"null\" }\ndefault: 42",
                "incompatible",
            ),
            (
                "length",
                "oneOf:\n  - { type: string, minLength: 3 }\n  - { type: \"null\" }\ndefault: x",
                "minLength",
            ),
            (
                "enum",
                "oneOf:\n  - { type: string, enum: [open, closed] }\n  - { type: \"null\" }\ndefault: pending",
                "enum",
            ),
            (
                "format",
                "oneOf:\n  - { type: string, format: email }\n  - { type: \"null\" }\ndefault: not-an-email",
                "email",
            ),
            (
                "content encoding",
                "oneOf:\n  - { type: string, contentEncoding: base64 }\n  - { type: \"null\" }\ndefault: '***'",
                "base64",
            ),
        ] {
            let error = structural_reject(schema);
            assert!(
                error.contains(expected),
                "{constraint}: expected {expected}, got {error}"
            );
        }
    }

    #[test]
    fn accepts_scalar_const_and_enum() {
        parse(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  a: { type: integer, const: 3 }
  b: { type: boolean, const: true }
  c: { type: number, const: 3.14 }
  d: { type: string, enum: [active, inactive, pending] }
  e: { type: integer, enum: [1, 2, 3] }
  f: { type: number, enum: [1.5, 2.5] }
"#,
        );
    }

    #[test]
    fn rejects_const_and_enum_together() {
        let error = const_enum_reject("type: string\nconst: a\nenum: [a, b]");
        assert!(error.contains("mutually exclusive"), "{error}");
    }

    #[test]
    fn rejects_default_on_required_member() {
        let input = r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
required: [value]
properties:
  value: { type: string, default: "x" }
"#;
        let error = parse_api_spec_from_json_schema_for_language(
            Language::Python,
            input,
            PathBuf::from("api.yaml"),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("required member"), "{error}");
    }

    #[test]
    fn rejects_empty_title() {
        let error = structural_reject("type: string\ntitle: \"   \"");
        assert!(error.contains("`title` must not be empty"), "{error}");
    }

    #[test]
    fn rejects_multiline_title() {
        let error = structural_reject("type: string\ntitle: \"a\\nb\"");
        assert!(error.contains("single line"), "{error}");
    }

    #[test]
    fn rejects_non_boolean_deprecated() {
        let error = structural_reject("type: string\ndeprecated: \"true\"");
        assert!(error.contains("`deprecated` must be a boolean"), "{error}");
    }

    #[test]
    fn rejects_non_string_comment() {
        let error = structural_reject("type: string\n$comment: 42");
        assert!(error.contains("`$comment` must be a string"), "{error}");
    }

    #[test]
    fn accepts_annotations() {
        // title/description/deprecated:true, deprecated:false, examples, $comment
        // all load; examples/$comment are inert; deprecated:false is inert.
        parse(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  a:
    type: string
    title: Label
    description: A described member.
    deprecated: true
    examples: ["x", "y"]
    $comment: internal note
  b:
    type: string
    deprecated: false
"#,
        );
    }

    #[test]
    fn rejects_null_default() {
        let error = structural_reject("type: string\ndefault: null");
        assert!(error.contains("`default: null`"), "{error}");
    }

    #[test]
    fn rejects_object_default() {
        let error = structural_reject("type: object\ndefault: { a: 1 }");
        assert!(error.contains("object/array"), "{error}");
    }

    #[test]
    fn rejects_array_default() {
        let error = structural_reject("type: array\nitems: { type: string }\ndefault: [a]");
        assert!(error.contains("object/array"), "{error}");
    }

    #[test]
    fn rejects_type_incompatible_default() {
        let error = structural_reject("type: string\ndefault: 42");
        assert!(error.contains("incompatible"), "{error}");
    }

    #[test]
    fn accepts_scalar_defaults_of_each_kind() {
        parse(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  s: { type: string, default: "anon" }
  i: { type: integer, default: 0 }
  n: { type: number, default: 1.5 }
  b: { type: boolean, default: false }
"#,
        );
    }

    #[test]
    fn rejects_const_null() {
        let error = const_enum_reject("type: \"null\"\nconst: null");
        assert!(error.contains("null"), "{error}");
    }

    #[test]
    fn rejects_composite_const() {
        let error = const_enum_reject("type: object\nconst: { a: 1 }");
        assert!(error.contains("composite"), "{error}");
    }

    #[test]
    fn rejects_empty_enum() {
        let error = const_enum_reject("type: string\nenum: []");
        assert!(error.contains("must not be empty"), "{error}");
    }

    #[test]
    fn rejects_mixed_type_enum() {
        let error = const_enum_reject("type: string\nenum: [a, 1, true]");
        assert!(error.contains("incompatible"), "{error}");
    }

    #[test]
    fn rejects_type_incompatible_const() {
        let error = const_enum_reject("type: integer\nconst: x");
        assert!(error.contains("incompatible"), "{error}");
    }

    #[test]
    fn value_token_collisions_are_checked_per_emitted_target() {
        let input = r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  value: { type: string, enum: [user-admin, user_admin] }
"#;
        for language in [Language::Go, Language::Java] {
            let error = reject_for(language, input);
            assert!(
                error.contains("collision") && error.contains("enum-names"),
                "{language:?}: {error}"
            );
        }
        for language in [Language::TypeScript, Language::Python] {
            parse_for(language, input)
                .unwrap_or_else(|error| panic!("{language:?} emits no value constant: {error}"));
        }
    }

    #[test]
    fn rejects_const_with_default() {
        let error = const_enum_reject("type: string\nconst: a\ndefault: a");
        assert!(error.contains("mutually exclusive"), "{error}");
    }

    #[test]
    fn rejects_duplicate_enum_members() {
        let error = const_enum_reject("type: string\nenum: [a, a]");
        assert!(error.contains("more than once"), "{error}");
    }

    #[test]
    fn rejects_enum_default_not_in_set() {
        let error = const_enum_reject("type: string\nenum: [a, b]\ndefault: c");
        assert!(error.contains("not a member"), "{error}");
    }

    #[test]
    fn rejects_non_ascii_const() {
        let error = const_enum_reject("type: string\nconst: \"café\"");
        assert!(error.contains("must be ASCII"), "{error}");
    }

    #[test]
    fn rejects_whitespace_const() {
        let error = const_enum_reject("type: string\nconst: \"user admin\"");
        assert!(error.contains("must not contain whitespace"), "{error}");
    }

    #[test]
    fn rejects_null_enum_member() {
        let error = const_enum_reject("type: string\nenum: [a, null]");
        assert!(error.contains("`enum: null`"), "{error}");
    }

    #[test]
    fn rejects_composite_enum_member() {
        let error = const_enum_reject("type: object\nenum: [{ a: 1 }]");
        assert!(error.contains("composite"), "{error}");
    }

    // ---- `allOf` load-time merge (specs/json-schema/features/allOf.md) ----

    /// Parses `input` and returns the merged JSON schema value of the named
    /// generated model (a `$defs` entry or the document root).
    fn model_schema(input: &str, name: &str) -> Value {
        let spec = parse(input);
        let binding = spec
            .external_type_binding(name)
            .unwrap_or_else(|| panic!("no external type binding `{name}`"));
        binding
            .json_model()
            .unwrap_or_else(|| panic!("binding `{name}` is not a JSON model"))
            .schema
            .clone()
    }

    #[test]
    fn all_of_object_base_extension_merges_union() {
        let schema = model_schema(
            r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  widget: { $ref: "#/$defs/Widget" }
$defs:
  Base:
    type: object
    required: [id]
    properties:
      id: { type: string }
  Widget:
    allOf:
      - { $ref: "#/$defs/Base" }
      - type: object
        required: [name]
        properties:
          name: { type: string }
"##,
            "Widget",
        );
        // Union of properties (Base.id + own.name), copied — not referenced.
        assert_eq!(schema["properties"]["id"]["type"], "string");
        assert_eq!(schema["properties"]["name"]["type"], "string");
        // Union of required.
        assert_eq!(schema["required"], serde_json::json!(["id", "name"]));
        // No combinator / ref residue.
        assert!(schema.get("allOf").is_none());
        assert!(schema["$ref"].is_null());
    }

    #[test]
    fn all_of_tightens_same_axis_numeric_bound() {
        let schema = model_schema(
            r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  n:
    allOf:
      - { type: integer, minimum: 3 }
      - { type: integer, minimum: 4 }
"##,
            "Api",
        );
        // The greater floor wins.
        assert_eq!(schema["properties"]["n"]["minimum"], 4);
    }

    #[test]
    fn all_of_tightens_across_inclusive_exclusive() {
        let schema = model_schema(
            r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  n:
    allOf:
      - { type: integer, maximum: 10 }
      - { type: integer, exclusiveMaximum: 8 }
"##,
            "Api",
        );
        // `exclusiveMaximum: 8` dominates `maximum: 10`; the inclusive bound is
        // dropped so only one upper bound survives.
        assert_eq!(schema["properties"]["n"]["exclusiveMaximum"], 8);
        assert!(schema["properties"]["n"].get("maximum").is_none());
    }

    #[test]
    fn all_of_multiple_of_merges_to_lcm() {
        let schema = model_schema(
            r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  n:
    allOf:
      - { type: integer, multipleOf: 2 }
      - { type: integer, multipleOf: 3 }
"##,
            "Api",
        );
        assert_eq!(schema["properties"]["n"]["multipleOf"], 6);
    }

    #[test]
    fn all_of_enum_intersects() {
        let schema = model_schema(
            r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  s:
    allOf:
      - { type: string, enum: [a, b, c] }
      - { type: string, enum: [b, c, d] }
"##,
            "Api",
        );
        assert_eq!(
            schema["properties"]["s"]["enum"],
            serde_json::json!(["b", "c"])
        );
    }

    #[test]
    fn all_of_closed_base_closes_to_union() {
        const DOC: &str = r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  merged:
    allOf:
      - type: object
        additionalProperties: false
        properties:
          a: { type: string }
      - type: object
        properties:
          b: { type: string }
"##;
        // The merge runs before the inline shape is named, so the property holds
        // a `$ref` at the merged object.
        assert_eq!(
            model_schema(DOC, "Api")["properties"]["merged"]["$ref"],
            "#/$defs/ApiMerged"
        );
        let merged = model_schema(DOC, "ApiMerged");
        // Closed against the union of declared properties (footgun-fix).
        assert_eq!(merged["additionalProperties"], false);
        assert_eq!(merged["properties"]["a"]["type"], "string");
        assert_eq!(merged["properties"]["b"]["type"], "string");
    }

    /// `allOf.md`'s "Overlapping property merged recursively" row: a property
    /// declared by both branches keeps both branches' constraints.
    #[test]
    fn all_of_merges_overlapping_property_recursively() {
        let schema = model_schema(
            r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  widget: { $ref: "#/$defs/Widget" }
$defs:
  Widget:
    allOf:
      - type: object
        properties:
          n: { type: string, minLength: 2 }
      - type: object
        properties:
          n: { type: string, maxLength: 8 }
"##,
            "Widget",
        );
        assert_eq!(schema["properties"]["n"]["type"], "string");
        assert_eq!(schema["properties"]["n"]["minLength"], 2);
        assert_eq!(schema["properties"]["n"]["maxLength"], 8);
    }

    /// A `$ref` in a child position is not flattened by `expand_branches`, so the
    /// pairwise merge has to defer it to the normalize walk. Dropping it instead
    /// silently loses the referenced type's members and `required`.
    #[test]
    fn all_of_merges_ref_property_with_object_sibling() {
        let spec = parse(
            r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  widget: { $ref: "#/$defs/Widget" }
$defs:
  Base:
    type: object
    required: [id]
    properties:
      id: { type: string }
  Widget:
    allOf:
      - type: object
        properties:
          p: { $ref: "#/$defs/Base" }
      - type: object
        properties:
          p:
            type: object
            properties:
              extra: { type: string }
"##,
        );
        let json_model = |name: &str| -> Value {
            let binding = spec
                .external_type_binding(name)
                .unwrap_or_else(|| panic!("no external type binding `{name}`"));
            binding
                .json_model()
                .unwrap_or_else(|| panic!("binding `{name}` is not a JSON model"))
                .schema
                .clone()
        };
        // The merged property is a new anonymous shape, hoisted under the
        // owning model's name.
        assert_eq!(
            json_model("Widget")["properties"]["p"]["$ref"],
            "#/$defs/WidgetP"
        );
        let merged = json_model("WidgetP");
        assert_eq!(merged["properties"]["id"]["type"], "string");
        assert_eq!(merged["properties"]["extra"]["type"], "string");
        assert_eq!(merged["required"], serde_json::json!(["id"]));
    }

    /// Both branches naming the same `$ref` is the degenerate case of the above:
    /// the merge is the identity, and the property stays a reference rather than
    /// inlining a copy of the target.
    #[test]
    fn all_of_keeps_identical_ref_property_as_a_reference() {
        let schema = model_schema(
            r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  widget: { $ref: "#/$defs/Widget" }
$defs:
  Base:
    type: object
    required: [id]
    properties:
      id: { type: string }
  Widget:
    allOf:
      - type: object
        required: [p]
        properties:
          p: { $ref: "#/$defs/Base" }
      - type: object
        properties:
          p: { $ref: "#/$defs/Base" }
"##,
            "Widget",
        );
        assert_eq!(schema["properties"]["p"]["$ref"], "#/$defs/Base");
        assert_eq!(schema["required"], serde_json::json!(["p"]));
    }

    /// A `oneOf` in a child position is the same combinator `expand_branches`
    /// rejects at the top level; it must not be silently dropped just because it
    /// sits under `properties` (a dropped nullability wrapper emits a
    /// non-nullable field that rejects `null`).
    #[test]
    fn all_of_rejects_one_of_property_branch() {
        let error = doc_reject(
            r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  widget: { $ref: "#/$defs/Widget" }
$defs:
  Widget:
    allOf:
      - type: object
        properties:
          n:
            oneOf:
              - { type: string }
              - { type: "null" }
      - type: object
        properties:
          n: { type: string, minLength: 2 }
"##,
        );
        assert!(error.contains("cannot be a `oneOf`"), "{error}");
        assert!(error.contains("$defs.Widget.properties.n"), "{error}");
    }

    /// `x-<lang>-name` on a `$ref` target renames *that type*. Folding it into
    /// the merge site made the merged node claim the target's Go identifier —
    /// a P15 collision for Go only, so the identical schema loaded for the other
    /// three targets.
    #[test]
    fn all_of_ref_branch_does_not_inherit_the_target_type_name() {
        let input = r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  widget: { $ref: "#/$defs/Widget" }
$defs:
  Base:
    type: object
    x-go-name: Renamed
    properties:
      id: { type: string }
  Widget:
    allOf:
      - { $ref: "#/$defs/Base" }
      - type: object
        properties:
          name: { type: string }
"##;
        for language in [
            Language::Go,
            Language::TypeScript,
            Language::Python,
            Language::Java,
        ] {
            let spec = parse_api_spec_from_json_schema_for_language(
                language,
                input,
                PathBuf::from("api.yaml"),
            )
            .unwrap_or_else(|error| panic!("{language:?} should load: {error}"));
            let binding = spec
                .external_type_binding("Widget")
                .expect("no external type binding `Widget`");
            let model = binding.json_model().expect("`Widget` is not a JSON model");
            assert!(
                model.schema.get("x-go-name").is_none(),
                "{language:?}: the target's type name leaked into the merge site: {:?}",
                model.schema
            );
        }
    }

    /// The target's nested `$defs` are its own declarations, already collected
    /// where the target is declared. Copying them into the merge site declared
    /// each one twice, under a name (`Widget.Inner`) the user never wrote and so
    /// could not rename.
    #[test]
    fn all_of_ref_branch_does_not_duplicate_the_target_defs() {
        let spec = parse_api_spec_from_json_schema_for_language(
            Language::Go,
            r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  widget: { $ref: "#/$defs/Widget" }
$defs:
  Base:
    type: object
    properties:
      inner: { $ref: "#/$defs/Base/$defs/Inner" }
    $defs:
      Inner:
        type: object
        properties:
          v: { type: string }
  Widget:
    allOf:
      - { $ref: "#/$defs/Base" }
      - type: object
        properties:
          name: { type: string }
"##,
            PathBuf::from("api.yaml"),
        )
        .expect("the nested `$defs` of an `allOf` base must not be re-declared");
        let names: Vec<&str> = spec.external_types().map(|(name, _)| name).collect();
        assert!(names.contains(&"Base.Inner"), "{names:?}");
        assert!(!names.contains(&"Widget.Inner"), "{names:?}");
        let binding = spec
            .external_type_binding("Widget")
            .expect("no external type binding `Widget`");
        let model = binding.json_model().expect("`Widget` is not a JSON model");
        // The merged node keeps the reference to the base's declaration.
        assert_eq!(
            model.schema["properties"]["inner"]["$ref"],
            "#/$defs/Base.Inner"
        );
    }

    /// `a / gcd * b` over `i64` panics in a debug build and wraps to a negative
    /// divisor in a release build.
    #[test]
    fn all_of_rejects_multiple_of_lcm_above_the_safe_integer_cap() {
        let error = numeric_reject(
            "allOf:\n  - { type: integer, multipleOf: 4294967291 }\n  - { type: integer, multipleOf: 4294967279 }",
        );
        assert!(
            error.contains("least common multiple") && error.contains("9007199254740991"),
            "{error}"
        );
    }

    #[test]
    fn all_of_rejects_disjoint_type() {
        let error = numeric_reject("allOf:\n  - { type: string }\n  - { type: number }");
        assert!(error.contains("disjoint types"), "{error}");
    }

    #[test]
    fn all_of_rejects_disagreeing_const() {
        let error = numeric_reject(
            "allOf:\n  - { type: integer, const: 1 }\n  - { type: integer, const: 2 }",
        );
        assert!(error.contains("conflicting `const`"), "{error}");
    }

    #[test]
    fn all_of_rejects_empty_enum_intersection() {
        let error = numeric_reject(
            "allOf:\n  - { type: string, enum: [a, b] }\n  - { type: string, enum: [c, d] }",
        );
        assert!(error.contains("empty `enum` intersection"), "{error}");
    }

    #[test]
    fn all_of_rejects_false_branch() {
        let error = numeric_reject("allOf:\n  - { type: object }\n  - false");
        assert!(error.contains("`false`"), "{error}");
    }

    #[test]
    fn all_of_rejects_combinator_branch() {
        let error = numeric_reject(
            "allOf:\n  - { type: object }\n  - oneOf: [ { type: string }, { type: \"null\" } ]",
        );
        assert!(error.contains("cannot be a `oneOf`"), "{error}");
    }

    #[test]
    fn all_of_rejects_empty_array() {
        let error = numeric_reject("allOf: []");
        assert!(error.contains("must not be empty"), "{error}");
    }

    #[test]
    fn all_of_rejects_single_branch_wrapper() {
        let error = numeric_reject("allOf:\n  - { type: string }");
        assert!(error.contains("single-branch"), "{error}");
    }

    #[test]
    fn all_of_rejects_empty_numeric_interval_after_merge() {
        // The merged interval is empty; the reject is delegated to the numeric
        // validator on the merged schema.
        let error = numeric_reject(
            "allOf:\n  - { type: integer, minimum: 10 }\n  - { type: integer, maximum: 5 }",
        );
        assert!(error.contains("empty range"), "{error}");
    }

    #[test]
    fn all_of_validates_raw_branch_grammar_before_merging() {
        let required = numeric_reject(
            "allOf:\n  - { type: object, properties: { a: { type: string } }, required: a }\n  - { type: object, required: [a] }",
        );
        assert!(
            required.contains("`required` must be an array"),
            "{required}"
        );

        let dependent = numeric_reject(
            "allOf:\n  - { type: object, properties: { a: { type: string }, b: { type: string } }, dependentRequired: nope }\n  - { type: object, dependentRequired: { a: [b] } }",
        );
        assert!(
            dependent.contains("`dependentRequired` must be an object"),
            "{dependent}"
        );

        let additional = numeric_reject(
            "allOf:\n  - { type: object, additionalProperties: 5 }\n  - { type: object, additionalProperties: false }",
        );
        assert!(
            additional.contains("additionalProperties") && additional.contains("schema object"),
            "{additional}"
        );

        let unique = numeric_reject(
            "allOf:\n  - { type: array, items: { type: string }, uniqueItems: yes }\n  - { type: array, uniqueItems: true }",
        );
        assert!(
            unique.contains("`uniqueItems` must be a boolean"),
            "{unique}"
        );

        let default = numeric_reject(
            "allOf:\n  - { type: string, default: [bad] }\n  - { type: string, default: good }",
        );
        assert!(default.contains("object/array"), "{default}");
    }

    #[test]
    fn all_of_and_ref_siblings_reject_malformed_keywords_before_merge() {
        for (position, schema, expected) in [
            (
                "allOf type",
                "allOf:\n  - { type: 5 }\n  - { type: string }",
                "`type`",
            ),
            (
                "allOf numeric bound",
                "allOf:\n  - { type: integer, minimum: nope }\n  - { type: integer, minimum: 1 }",
                "`minimum`",
            ),
            (
                "allOf string count",
                "allOf:\n  - { type: string, minLength: nope }\n  - { type: string, minLength: 1 }",
                "`minLength`",
            ),
            (
                "allOf array count",
                "allOf:\n  - { type: array, items: { type: string }, minItems: nope }\n  - { type: array, minItems: 1 }",
                "`minItems`",
            ),
            (
                "allOf property count",
                "allOf:\n  - { type: object, minProperties: nope }\n  - { type: object, minProperties: 1 }",
                "`minProperties`",
            ),
            (
                "allOf pattern",
                "allOf:\n  - { type: string, pattern: 5 }\n  - { type: string, pattern: '^x' }",
                "`pattern`",
            ),
            (
                "allOf format",
                "allOf:\n  - { type: string, format: 5 }\n  - { type: string, format: email }",
                "`format`",
            ),
        ] {
            let error = numeric_reject(schema);
            assert!(
                error.contains(expected),
                "{position}: expected {expected}, got {error}"
            );
        }

        let error = doc_reject(
            r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  value:
    $ref: "#/$defs/Base"
    minimum: nope
$defs:
  Base: { type: integer, minimum: 1 }
"##,
        );
        assert!(error.contains("`minimum`"), "$ref sibling: {error}");
    }

    #[test]
    fn all_of_merges_deprecated_with_or_and_discards_inert_annotations() {
        let schema = model_schema(
            r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  value:
    allOf:
      - type: string
        deprecated: false
        $comment: first
        examples: [first]
      - type: string
        deprecated: true
        $comment: second
        examples: [second]
"##,
            "Api",
        );
        let value = &schema["properties"]["value"];
        assert_eq!(value["deprecated"], true);
        assert!(value.get("$comment").is_none(), "{value}");
        assert!(value.get("examples").is_none(), "{value}");
    }

    #[test]
    fn rejects_all_of_combinator_branch_not() {
        let error = numeric_reject("allOf:\n  - { type: object }\n  - { not: { type: integer } }");
        assert!(error.contains("`not` is not supported"), "{error}");
    }

    #[test]
    fn rejects_all_of_differing_format() {
        let error = numeric_reject(
            "allOf:\n  - { type: string, format: email }\n  - { type: string, format: uri }",
        );
        assert!(error.contains("unrelated `format`s"), "{error}");
    }

    #[test]
    fn all_of_keeps_the_narrower_contained_format() {
        for (broad, narrow) in [
            ("uri-reference", "uri"),
            ("uri-reference", "uuid"),
            ("uri-reference", "ipv4"),
            ("uri-reference", "date"),
            ("hostname", "uuid"),
        ] {
            let schema = model_schema(
                &format!(
                    "$schema: https://json-schema.org/draft/2020-12/schema\ntype: object\nproperties:\n  value:\n    allOf:\n      - {{ type: string, format: {broad} }}\n      - {{ type: string, format: {narrow} }}\n"
                ),
                "Api",
            );
            assert_eq!(schema["properties"]["value"]["format"], narrow);
        }
    }

    #[test]
    fn rejects_all_of_merge_cycle_through_child_position() {
        let error = doc_reject(
            r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  node: { $ref: "#/$defs/Node" }
$defs:
  Node:
    type: object
    properties:
      child:
        $ref: "#/$defs/Node"
        minProperties: 1
"##,
        );
        assert!(error.contains("allOf` merge cycle"), "{error}");
    }

    /// Branch targets are active only while their own ref path is expanded.
    /// `Node` inherits `Base` (which inherits `Trait`) and independently uses a
    /// constrained `Trait` child; that child edge is acyclic and must not see
    /// the outer merge's complete target set as artificial ancestry.
    #[test]
    fn accepts_acyclic_child_ref_to_an_inherited_trait() {
        parse(
            r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  node: { $ref: "#/$defs/Node" }
$defs:
  Trait:
    type: object
    properties:
      traitValue: { type: string }
  Base:
    allOf:
      - { $ref: "#/$defs/Trait" }
      - type: object
        properties:
          baseValue: { type: string }
  Node:
    allOf:
      - { $ref: "#/$defs/Base" }
      - type: object
        properties:
          trait:
            $ref: "#/$defs/Trait"
            minProperties: 1
"##,
        );
    }

    #[test]
    fn rejects_all_of_distinct_patterns() {
        let error = numeric_reject(
            "allOf:\n  - { type: string, pattern: \"^a\" }\n  - { type: string, pattern: \"z$\" }",
        );
        assert!(error.contains("different `pattern`s"), "{error}");
    }

    #[test]
    fn rejects_all_of_conflicting_const_enum() {
        let error = numeric_reject(
            "allOf:\n  - { type: integer, const: 5 }\n  - { type: integer, enum: [1, 2] }",
        );
        assert!(
            error.contains("not a member of the merged `enum`"),
            "{error}"
        );
    }

    #[test]
    fn all_of_uses_mathematical_numeric_equality_for_closed_values() {
        for branches in [
            "      - { type: number, const: 5 }\n      - { type: number, const: 5.0 }",
            "      - { type: number, const: 5 }\n      - { type: number, enum: [5.0, 6] }",
        ] {
            let schema = model_schema(
                &format!(
                    "$schema: https://json-schema.org/draft/2020-12/schema\ntype: object\nproperties:\n  value:\n    allOf:\n{branches}\n"
                ),
                "Api",
            );
            assert_eq!(schema["properties"]["value"]["const"], 5);
            assert!(schema["properties"]["value"].get("enum").is_none());
        }
    }

    #[test]
    fn rejects_all_of_unresolvable_ref_branch() {
        let error = numeric_reject(
            "allOf:\n  - { $ref: \"#/$defs/Missing\" }\n  - { type: object, properties: {} }",
        );
        assert!(
            error.contains("properties.value.allOf[0]")
                && error.contains("declares no `$defs.Missing` entry")
                && error.contains("add that `$defs` entry")
                && error.contains("correct the JSON Pointer"),
            "{error}"
        );
    }

    #[test]
    fn all_of_rejects_cyclic_ref() {
        let error = parse_api_spec_from_json_schema_for_language(
            Language::Python,
            r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  loop: { $ref: "#/$defs/Loop" }
$defs:
  Loop:
    allOf:
      - { $ref: "#/$defs/Loop" }
      - type: object
        properties:
          x: { type: string }
"##,
            PathBuf::from("api.yaml"),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("cycle"), "{error}");
    }

    // --- `oneOf` sum types (specs/json-schema/features/oneOf.md) ---

    fn union_doc_result(doc: &str) -> Result<ApiSpec> {
        union_doc_result_for(Language::Python, doc)
    }

    fn union_doc_result_for(language: Language, doc: &str) -> Result<ApiSpec> {
        parse_api_spec_from_json_schema_for_language(language, doc, PathBuf::from("api.yaml"))
    }

    /// The schema of a model in an already-loaded spec, by its emitted name.
    fn loaded_model_schema(spec: &ApiSpec, name: &str) -> Value {
        let binding = spec
            .external_type_binding(name)
            .unwrap_or_else(|| panic!("model `{name}` should be loaded"));
        let json = binding
            .json_model()
            .unwrap_or_else(|| panic!("`{name}` should be a JSON model"));
        json.schema.clone()
    }

    fn union_reject(doc: &str) -> String {
        union_doc_result(doc).unwrap_err().to_string()
    }

    #[test]
    fn accepts_disjoint_kind_union_field() {
        let spec = union_doc_result(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  value:
    oneOf:
      - { type: string }
      - { type: integer }
"#,
        )
        .expect("disjoint-kind union should load");
        let root = spec.external_type_binding("Api").expect("root model");
        let json = root.json_model().expect("root should be a JSON model");
        assert!(json.schema["properties"]["value"]["oneOf"].is_array());
    }

    #[test]
    fn accepts_discriminated_object_union_def() {
        union_doc_result(
            r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  shape: { $ref: "#/$defs/Shape" }
$defs:
  Circle:
    type: object
    required: [kind, radius]
    properties:
      kind: { type: string, const: circle }
      radius: { type: number }
  Square:
    type: object
    required: [kind, side]
    properties:
      kind: { type: string, const: square }
      side: { type: number }
  Shape:
    oneOf:
      - { $ref: "#/$defs/Circle" }
      - { $ref: "#/$defs/Square" }
"##,
        )
        .expect("discriminated object union should load");
    }

    #[test]
    fn two_branch_nullable_stays_a_plain_nullable_field() {
        let spec = union_doc_result(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  middleName:
    oneOf:
      - { type: string }
      - { type: "null" }
"#,
        )
        .expect("nullable oneOf should load");
        let root = spec.external_type_binding("Api").expect("root model");
        let json = root.json_model().expect("root should be a JSON model");
        // The degenerate two-branch pattern is preserved as-is (owned by
        // nullability), not rewritten into a sum type.
        let branches = json.schema["properties"]["middleName"]["oneOf"]
            .as_array()
            .expect("nullable oneOf branches");
        assert_eq!(branches.len(), 2);
    }

    #[test]
    fn rejects_single_branch_one_of() {
        let error = union_reject(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  value:
    oneOf:
      - { type: string }
"#,
        );
        assert!(error.contains("single-branch"), "{error}");
    }

    #[test]
    fn rejects_integer_number_overlap_union() {
        let error = union_reject(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  value:
    oneOf:
      - { type: integer }
      - { type: number }
"#,
        );
        assert!(
            error.contains("integer") && error.contains("number"),
            "{error}"
        );
    }

    #[test]
    fn rejects_non_separable_overlapping_object_union() {
        let error = union_reject(
            r##"
$schema: https://json-schema.org/draft/2020-12/schema
$defs:
  A: { type: object, properties: { a: { type: string } } }
  B: { type: object, properties: { b: { type: string } } }
type: object
properties:
  value:
    oneOf:
      - { $ref: "#/$defs/A" }
      - { $ref: "#/$defs/B" }
"##,
        );
        assert!(error.contains("discriminator"), "{error}");
    }

    #[test]
    fn names_inline_structured_object_one_of_branch() {
        // A lone inline object branch is hoisted into `$defs` under the derived
        // `<Union>Object` name, and the branch becomes a `$ref` at it — so every
        // target emits it as an ordinary named model.
        let spec = union_doc_result(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  value:
    oneOf:
      - { type: object, properties: { a: { type: string } } }
      - { type: string }
"#,
        )
        .expect("inline structured object branch should load");
        assert_eq!(
            loaded_model_schema(&spec, "Api")["properties"]["value"]["oneOf"][0]["$ref"],
            Value::String("#/$defs/ApiValueObject".to_string())
        );
        assert_eq!(
            loaded_model_schema(&spec, "ApiValueObject")["properties"]["a"]["type"],
            Value::String("string".to_string())
        );
    }

    #[test]
    fn names_inline_typed_map_one_of_branch() {
        let spec = union_doc_result(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  value:
    oneOf:
      - { type: object, additionalProperties: { type: string } }
      - { type: string }
"#,
        )
        .expect("inline typed-map object branch should load");
        assert_eq!(
            loaded_model_schema(&spec, "ApiValueObject")["additionalProperties"]["type"],
            Value::String("string".to_string())
        );
    }

    #[test]
    fn names_inline_object_one_of_branch_of_named_union() {
        // A named `$defs` union names its lone inline branch after the union
        // itself, not after any enclosing property.
        let spec = union_doc_result(
            r##"
$schema: https://json-schema.org/draft/2020-12/schema
$defs:
  Payload:
    oneOf:
      - { type: object, properties: { a: { type: string } } }
      - { type: string }
type: object
properties:
  value: { $ref: "#/$defs/Payload" }
"##,
        )
        .expect("named union with an inline object branch should load");
        assert_eq!(
            loaded_model_schema(&spec, "Payload")["oneOf"][0]["$ref"],
            Value::String("#/$defs/PayloadObject".to_string())
        );
        assert!(loaded_model_schema(&spec, "PayloadObject")["properties"]["a"].is_object());
    }

    #[test]
    fn inline_object_one_of_branch_honors_name_override() {
        let spec = union_doc_result(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  value:
    oneOf:
      - { type: object, properties: { a: { type: string } }, x-py-name: Detail }
      - { type: string }
"#,
        )
        .expect("named inline object branch should load");
        assert_eq!(
            loaded_model_schema(&spec, "Api")["properties"]["value"]["oneOf"][0]["$ref"],
            Value::String("#/$defs/Detail".to_string())
        );
        assert!(loaded_model_schema(&spec, "Detail")["properties"]["a"].is_object());
    }

    #[test]
    fn names_inline_tagged_object_one_of_branches_by_override() {
        let spec = union_doc_result_for(
            Language::Go,
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  value:
    oneOf:
      - type: object
        required: [kind]
        properties: { kind: { type: string, const: cat }, meow: { type: string } }
        x-go-name: Cat
      - type: object
        required: [kind]
        properties: { kind: { type: string, const: dog }, bark: { type: string } }
        x-go-name: Dog
"#,
        )
        .expect("self-named inline tagged object branches should load");
        assert!(loaded_model_schema(&spec, "Cat")["properties"]["meow"].is_object());
        assert!(loaded_model_schema(&spec, "Dog")["properties"]["bark"].is_object());
    }

    #[test]
    fn names_inline_object_branch_nested_in_another_inline_branch() {
        // A hoisted branch is itself walked, so a union inside it is named
        // against the branch's own name — composing deterministically.
        let spec = union_doc_result(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  outer:
    oneOf:
      - type: object
        required: [inner]
        properties:
          inner:
            oneOf:
              - { type: object, properties: { deep: { type: string } } }
              - { type: integer }
      - { type: string }
"#,
        )
        .expect("nested inline object branches should load");
        assert_eq!(
            loaded_model_schema(&spec, "ApiOuterObject")["properties"]["inner"]["oneOf"][0]["$ref"],
            Value::String("#/$defs/ApiOuterObjectInnerObject".to_string())
        );
        assert!(
            loaded_model_schema(&spec, "ApiOuterObjectInnerObject")["properties"]["deep"]
                .is_object()
        );
    }

    #[test]
    fn rejects_inline_tagged_object_one_of_branches_without_override() {
        let error = union_reject(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  value:
    oneOf:
      - type: object
        required: [kind]
        properties: { kind: { type: string, const: cat }, meow: { type: string } }
      - type: object
        required: [kind]
        properties: { kind: { type: string, const: dog }, bark: { type: string } }
"#,
        );
        assert!(
            error.contains("x-py-name") && error.contains("ApiValueObject"),
            "{error}"
        );
    }

    #[test]
    fn rejects_inline_object_one_of_branch_name_clashing_with_a_definition() {
        let error = union_reject(
            r##"
$schema: https://json-schema.org/draft/2020-12/schema
$defs:
  ApiValueObject: { type: object, properties: { b: { type: string } } }
type: object
properties:
  value:
    oneOf:
      - { type: object, properties: { a: { type: string } } }
      - { type: string }
  other: { $ref: "#/$defs/ApiValueObject" }
"##,
        );
        assert!(
            error.contains("ApiValueObject") && error.contains("already declared in `$defs`"),
            "{error}"
        );
    }

    #[test]
    fn synthesized_inline_shape_collision_names_both_authored_positions() {
        let error = reject_for(
            Language::Go,
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  bC:  { type: object, properties: { p: { type: string } } }
  b_c: { type: object, properties: { q: { type: string } } }
"#,
        );
        assert!(
            error.contains("root schema.properties.b_c")
                && error.contains(
                    "already synthesized for the inline shape at `root schema.properties.bC`"
                )
                && !error.contains("already declared in `$defs`"),
            "{error}"
        );
    }

    #[test]
    fn annotated_ref_hoist_collision_offers_an_applicable_annotation_remedy() {
        let input = r##"
$schema: https://json-schema.org/draft/2020-12/schema
$defs:
  Target:
    type: object
    properties: { value: { type: string } }
  User:
    type: object
    properties:
      profile:
        $ref: "#/$defs/Target"
        description: use-site documentation
        x-ts-name: renamedMember
  UserProfile:
    type: object
    properties: { other: { type: string } }
"##;
        let error = reject_for(Language::TypeScript, input);
        assert!(
            error.contains("$defs.User.properties.profile")
                && error.contains("position-derived name")
                && error.contains("`description` annotation")
                && error.contains("remove the annotation")
                && error.contains("relocate the annotation to the referenced declaration")
                && !error.contains("name the inline shape with an `x-ts-name` override"),
            "{error}"
        );

        let title_input = input.replace(
            "description: use-site documentation",
            "title: Use-site documentation",
        );
        let title_error = reject_for(Language::TypeScript, &title_input);
        assert!(
            title_error.contains("`title` annotation")
                && title_error.contains("remove the annotation")
                && title_error.contains("relocate the annotation"),
            "{title_error}"
        );

        let corrected = input.replace(
            "        description: use-site documentation\n        x-ts-name: renamedMember",
            "        x-ts-name: renamedMember",
        );
        parse_for(Language::TypeScript, &corrected)
            .expect("removing the use-site annotation keeps the node as a reference");

        let relocated = corrected.replace(
            "  Target:\n    type: object",
            "  Target:\n    description: declaration documentation\n    type: object",
        );
        parse_for(Language::TypeScript, &relocated)
            .expect("relocating the annotation to the target also resolves the hoist collision");
    }

    #[test]
    fn hoists_inline_union_inside_items() {
        // The element union is named `<Model><Property>Item` and moved into
        // `$defs`; its own inline object branch is then named in turn, so the
        // element position needs no `$defs` + `$ref` boilerplate from the
        // author (specs/json-schema/features/oneOf.md §"Unions in element
        // positions").
        let spec = union_doc_result(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  values:
    type: array
    items:
      oneOf:
        - { type: object, properties: { a: { type: string } } }
        - { type: string }
"#,
        )
        .expect("an inline element union should load");
        let root = loaded_model_schema(&spec, "Api");
        assert_eq!(
            root["properties"]["values"]["items"]["$ref"],
            Value::String("#/$defs/ApiValuesItem".to_string())
        );
        let element = loaded_model_schema(&spec, "ApiValuesItem");
        assert_eq!(
            element["oneOf"][0]["$ref"],
            Value::String("#/$defs/ApiValuesItemObject".to_string())
        );
        assert_eq!(
            loaded_model_schema(&spec, "ApiValuesItemObject")["properties"]["a"]["type"],
            Value::String("string".to_string())
        );
    }

    #[test]
    fn hoists_inline_union_inside_additional_properties() {
        let spec = union_doc_result(
            r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  entries: { $ref: "#/$defs/Entries" }
$defs:
  Entries:
    type: object
    additionalProperties:
      oneOf:
        - { type: string }
        - { type: integer }
"##,
        )
        .expect("an inline map-value union should load");
        assert_eq!(
            loaded_model_schema(&spec, "Entries")["additionalProperties"]["$ref"],
            Value::String("#/$defs/EntriesValue".to_string())
        );
        assert!(loaded_model_schema(&spec, "EntriesValue")["oneOf"].is_array());
    }

    #[test]
    fn names_an_inline_element_union_with_a_type_override() {
        let spec = union_doc_result(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  values:
    type: array
    items:
      x-py-name: Element
      oneOf:
        - { type: string }
        - { type: integer }
"#,
        )
        .expect("an overridden element union should load");
        assert_eq!(
            loaded_model_schema(&spec, "Api")["properties"]["values"]["items"]["$ref"],
            Value::String("#/$defs/Element".to_string())
        );
        assert!(loaded_model_schema(&spec, "Element")["oneOf"].is_array());
    }

    #[test]
    fn leaves_a_nullable_element_inline() {
        // Two branches, one of them `null`, is the nullability pattern rather
        // than a sum type: every target expresses it on the element itself, so
        // there is nothing to name.
        let spec = union_doc_result(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  values:
    type: array
    items:
      oneOf:
        - { type: string }
        - { type: "null" }
"#,
        )
        .expect("a nullable element should load");
        let root = loaded_model_schema(&spec, "Api");
        assert!(root["properties"]["values"]["items"]["oneOf"].is_array());
    }

    #[test]
    fn rejects_an_element_union_name_colliding_with_a_definition() {
        let error = union_reject(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  values:
    type: array
    items:
      oneOf:
        - { type: string }
        - { type: integer }
$defs:
  ApiValuesItem:
    type: object
    properties:
      a: { type: string }
"#,
        );
        assert!(
            error.contains("ApiValuesItem") && error.contains("already declared in `$defs`"),
            "{error}"
        );
    }

    #[test]
    fn names_an_inline_object_property() {
        // An object written directly on a property is named `<Model><Property>`,
        // moved into `$defs`, and the property becomes a `$ref` at it — so the
        // declared shape is materialized instead of collapsing to an opaque map
        // (specs/json-schema/features/properties.md §"Naming an inline object
        // shape").
        let spec = union_doc_result(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  nested:
    description: An inline nested object.
    type: object
    required: [a]
    properties:
      a: { type: string, minLength: 2 }
"#,
        )
        .expect("an inline object property should load");
        assert_eq!(
            loaded_model_schema(&spec, "Api")["properties"]["nested"]["$ref"],
            Value::String("#/$defs/ApiNested".to_string())
        );
        let nested = loaded_model_schema(&spec, "ApiNested");
        assert_eq!(nested["properties"]["a"]["minLength"], Value::from(2));
        assert_eq!(nested["required"], serde_json::json!(["a"]));
        // The doc text travels with the shape it describes.
        assert_eq!(
            nested["description"],
            Value::String("An inline nested object.".to_string())
        );
    }

    #[test]
    fn names_a_nullable_inline_object_property_the_same() {
        // A nullability wrapper emits no type of its own, so the object inside it
        // takes the property's name — adding or removing nullability never
        // renames the type.
        let spec = union_doc_result(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  nested:
    oneOf:
      - type: object
        properties: { a: { type: string } }
      - { type: "null" }
"#,
        )
        .expect("a nullable inline object property should load");
        assert_eq!(
            loaded_model_schema(&spec, "Api")["properties"]["nested"]["oneOf"][0]["$ref"],
            Value::String("#/$defs/ApiNested".to_string())
        );
        assert!(loaded_model_schema(&spec, "ApiNested")["properties"]["a"].is_object());
    }

    #[test]
    fn names_an_inline_free_form_object_property() {
        // Even the free-form object is named in a value position: every object
        // emits as a named aggregate holding its members in a catch-all, so
        // later adding `properties` only adds fields (P13), and the member-count
        // and key-shape constraints ride along with it.
        let spec = union_doc_result(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  meta:
    type: object
    additionalProperties: true
    maxProperties: 4
"#,
        )
        .expect("an inline free-form object property should load");
        assert_eq!(
            loaded_model_schema(&spec, "Api")["properties"]["meta"]["$ref"],
            Value::String("#/$defs/ApiMeta".to_string())
        );
        let meta = loaded_model_schema(&spec, "ApiMeta");
        assert_eq!(meta["additionalProperties"], Value::Bool(true));
        assert_eq!(meta["maxProperties"], Value::from(4));
    }

    #[test]
    fn names_an_inline_object_property_shape_by_fixpoint() {
        // A hoisted shape is walked in turn, so an object nested inside one is
        // named against its own name — composing deterministically.
        let spec = union_doc_result(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  outer:
    type: object
    properties:
      inner:
        type: object
        properties:
          leaf: { type: string }
"#,
        )
        .expect("nested inline object properties should load");
        assert_eq!(
            loaded_model_schema(&spec, "ApiOuter")["properties"]["inner"]["$ref"],
            Value::String("#/$defs/ApiOuterInner".to_string())
        );
        assert!(loaded_model_schema(&spec, "ApiOuterInner")["properties"]["leaf"].is_object());
    }

    #[test]
    fn names_inline_object_shapes_in_element_positions() {
        // An element and a map member take the same position-derived names their
        // unions do: `<Enclosing>Item` and `<Enclosing>Value`.
        let spec = union_doc_result(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  rows:
    type: array
    items:
      type: object
      properties: { cell: { type: string } }
  byKey:
    type: object
    additionalProperties:
      type: object
      properties: { v: { type: integer } }
"#,
        )
        .expect("inline object element and member shapes should load");
        let root = loaded_model_schema(&spec, "Api");
        assert_eq!(
            root["properties"]["rows"]["items"]["$ref"],
            Value::String("#/$defs/ApiRowsItem".to_string())
        );
        assert!(loaded_model_schema(&spec, "ApiRowsItem")["properties"]["cell"].is_object());
        assert_eq!(
            loaded_model_schema(&spec, "ApiByKey")["additionalProperties"]["$ref"],
            Value::String("#/$defs/ApiByKeyValue".to_string())
        );
        assert!(loaded_model_schema(&spec, "ApiByKeyValue")["properties"]["v"].is_object());
    }

    #[test]
    fn a_hoisted_property_keeps_its_member_override() {
        // `x-<lang>-name` on a property is the [[properties]] Stage 4 escape
        // hatch for the *member* identifier, so it stays on the property; the
        // hoisted type keeps its position-derived name.
        let spec = union_doc_result(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  class:
    type: object
    properties: { x: { type: string } }
    x-py-name: klass
"#,
        )
        .expect("a renamed inline object member should load");
        let property = &loaded_model_schema(&spec, "Api")["properties"]["class"];
        assert_eq!(
            property["$ref"],
            Value::String("#/$defs/ApiClass".to_string())
        );
        assert_eq!(property["x-py-name"], Value::String("klass".to_string()));
        let hoisted = loaded_model_schema(&spec, "ApiClass");
        assert!(hoisted["properties"]["x"].is_object());
        assert!(hoisted["x-py-name"].is_null());
    }

    #[test]
    fn accepts_a_member_override_beside_a_ref() {
        // The override names the member, not the referenced type, so it asserts
        // nothing about the value: it is the one keyword legal beside a `$ref`,
        // and it is *not* an implicit-`allOf` conjunct — the reference stands, so
        // the target is referenced rather than cloned into the use site. Without
        // this a member whose type is a `$ref` could not be renamed at all.
        let spec = union_doc_result(
            r##"
$schema: https://json-schema.org/draft/2020-12/schema
$defs:
  Inner:
    type: object
    properties: { a: { type: string } }
type: object
properties:
  class:
    $ref: "#/$defs/Inner"
    x-py-name: klass
"##,
        )
        .expect("a renamed `$ref` member should load");
        let property = &loaded_model_schema(&spec, "Api")["properties"]["class"];
        assert_eq!(property["$ref"], Value::String("#/$defs/Inner".to_string()));
        assert_eq!(property["x-py-name"], Value::String("klass".to_string()));
    }

    #[test]
    fn accepts_inline_free_form_object_one_of_branch() {
        let spec = union_doc_result(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  value:
    oneOf:
      - { type: object, additionalProperties: true }
      - { type: string }
"#,
        )
        .expect("free-form inline object branch should load");
        let root = spec.external_type_binding("Api").expect("root model");
        let json = root.json_model().expect("root should be a JSON model");
        assert!(json.schema["properties"]["value"]["oneOf"].is_array());
    }

    #[test]
    fn rejects_non_const_discriminator_union() {
        let error = union_reject(
            r##"
$schema: https://json-schema.org/draft/2020-12/schema
$defs:
  Tagged:
    type: object
    required: [kind]
    properties: { kind: { type: string, const: a } }
  Untagged:
    type: object
    required: [kind]
    properties: { kind: { type: string } }
type: object
properties:
  value:
    oneOf:
      - { $ref: "#/$defs/Tagged" }
      - { $ref: "#/$defs/Untagged" }
"##,
        );
        assert!(error.contains("discriminator"), "{error}");
    }

    #[test]
    fn rejects_non_unique_discriminator_union() {
        let error = union_reject(
            r##"
$schema: https://json-schema.org/draft/2020-12/schema
$defs:
  First:
    type: object
    required: [kind]
    properties: { kind: { type: string, const: same }, a: { type: string } }
  Second:
    type: object
    required: [kind]
    properties: { kind: { type: string, const: same }, b: { type: string } }
type: object
properties:
  value:
    oneOf:
      - { $ref: "#/$defs/First" }
      - { $ref: "#/$defs/Second" }
"##,
        );
        assert!(
            error.contains("`kind`")
                && error.contains("value \"same\"")
                && error.contains("distinct `kind` tag value"),
            "{error}"
        );
    }

    #[test]
    fn rejects_two_string_branch_union() {
        let error = union_reject(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  value:
    oneOf:
      - { type: string, minLength: 1 }
      - { type: string, maxLength: 5 }
"#,
        );
        assert!(error.contains("enum"), "{error}");
    }

    #[test]
    fn rejects_two_boolean_branches_with_the_enum_remedy() {
        let error = union_reject(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  value:
    oneOf:
      - { type: string }
      - { type: boolean, const: true }
      - { type: boolean, const: false }
"#,
        );
        assert!(
            error.contains("share the `boolean` kind")
                && error.contains("enum")
                && error.contains("not a `oneOf`"),
            "{error}"
        );
    }

    #[test]
    fn rejects_boolean_schema_positions_with_a_breadcrumb_and_type_remedy() {
        for (schema, context, detail) in [
            (
                "type: object\nproperties:\n  value: true",
                "root schema.properties.value",
                "boolean schema `true`",
            ),
            (
                "type: object\nproperties:\n  value:\n    oneOf:\n      - { type: string }\n      - false",
                "root schema.properties.value.oneOf[1]",
                "no classifiable `oneOf` kind",
            ),
            (
                "$defs:\n  Flag: false",
                "$defs.Flag",
                "boolean schema `false`",
            ),
            (
                "type: object\nproperties:\n  value: 5",
                "root schema.properties.value",
                "expected a schema object",
            ),
            (
                "type: object\nproperties:\n  value: null",
                "root schema.properties.value",
                "expected a schema object",
            ),
        ] {
            let error = doc_reject(&format!(
                "$schema: https://json-schema.org/draft/2020-12/schema\n{schema}"
            ));
            assert!(
                error.contains(context) && error.contains(detail) && error.contains("type"),
                "{schema}: {error}"
            );
            assert!(
                !error.contains("expected struct Schema"),
                "{schema}: {error}"
            );
        }

        let operation = doc_reject(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
nexusrpc: "1.0.0"
services:
  Example:
    operations:
      run:
        input: true
        output: { type: object, properties: {} }
"#,
        );
        assert!(
            operation.contains("services.Example.operations.run.input")
                && operation.contains("boolean schema `true`")
                && operation.contains("explicit `type`"),
            "{operation}"
        );
    }

    #[test]
    fn rejects_boolean_and_tuple_items_with_nested_breadcrumbs() {
        for (schema, context, detail, remedy) in [
            (
                "type: array\nitems: true",
                "root schema.items",
                "boolean schema `true`",
                "uniform element type",
            ),
            (
                "type: object\nproperties:\n  values:\n    type: array\n    items: false",
                "root schema.properties.values.items",
                "boolean schema `false`",
                "uniform element type",
            ),
            (
                "type: array\nitems:\n  type: object\n  properties:\n    bad: true",
                "root schema.items.properties.bad",
                "boolean schema `true`",
                "explicit `type`",
            ),
            (
                "type: array\nitems:\n  type: array\n  items: [{ type: string }]",
                "root schema.items.items",
                "tuple-valued `items`",
                "uniform element type",
            ),
        ] {
            let error = doc_reject(&format!(
                "$schema: https://json-schema.org/draft/2020-12/schema\n{schema}"
            ));
            assert!(
                error.contains(context) && error.contains(detail) && error.contains(remedy),
                "{schema}: {error}"
            );
        }

        parse("type: object\nproperties:\n  values:\n    type: array\n    items: { type: string }");
    }

    #[test]
    fn accepts_schema_objects_where_boolean_schemas_are_rejected() {
        parse(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  flag: { type: boolean }
  values:
    type: object
    additionalProperties: true
$defs:
  Flag:
    type: object
    properties:
      enabled: { type: boolean }
"#,
        );
    }

    #[test]
    fn rejects_empty_one_of() {
        let error = union_reject(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  value:
    oneOf: []
"#,
        );
        assert!(
            error.contains("non-empty") || error.contains("single-branch"),
            "{error}"
        );
    }

    #[test]
    fn rejects_typeless_branch_union() {
        let error = union_reject(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  value:
    oneOf:
      - { type: string }
      - { description: "no type" }
"#,
        );
        assert!(error.contains("classifiable"), "{error}");
    }

    #[test]
    fn rejects_one_of_nested_one_of() {
        let error = union_reject(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  value:
    oneOf:
      - { type: string }
      - oneOf:
          - { type: integer }
          - { type: boolean }
"#,
        );
        assert!(error.contains("cannot itself be a `oneOf`"), "{error}");
    }

    #[test]
    fn rejects_one_of_two_array_branches() {
        let error = union_reject(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  value:
    oneOf:
      - { type: array, items: { type: string } }
      - { type: array, items: { type: integer } }
"#,
        );
        assert!(error.contains("no decidable selector"), "{error}");
    }

    #[test]
    fn rejects_one_of_duplicate_null_branches() {
        let error = union_reject(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  value:
    oneOf:
      - { type: string }
      - { type: "null" }
      - { type: "null" }
"#,
        );
        assert!(error.contains("`null` kind"), "{error}");
    }

    #[test]
    fn rejects_one_of_ambiguous_discriminator() {
        let error = union_reject(
            r##"
$schema: https://json-schema.org/draft/2020-12/schema
$defs:
  First:
    type: object
    required: [kind, variant]
    properties:
      kind: { type: string, const: a }
      variant: { type: string, const: x }
  Second:
    type: object
    required: [kind, variant]
    properties:
      kind: { type: string, const: b }
      variant: { type: string, const: y }
type: object
properties:
  value:
    oneOf:
      - { $ref: "#/$defs/First" }
      - { $ref: "#/$defs/Second" }
"##,
        );
        assert!(
            error.contains("more than one qualifying")
                && error.contains("`kind`")
                && error.contains("`variant`")
                && error.contains("keep exactly one"),
            "{error}"
        );
    }

    #[test]
    fn rejects_numeric_discriminator_values_equal_as_json_numbers() {
        let error = union_reject(
            r##"
$schema: https://json-schema.org/draft/2020-12/schema
$defs:
  First:
    type: object
    required: [kind]
    properties:
      kind: { type: number, const: 1 }
  Second:
    type: object
    required: [kind]
    properties:
      kind: { type: number, const: 1.0 }
type: object
properties:
  value:
    oneOf:
      - { $ref: "#/$defs/First" }
      - { $ref: "#/$defs/Second" }
"##,
        );
        assert!(
            error.contains("`kind`")
                && error.contains("value 1.0")
                && error.contains("distinct `kind` tag value"),
            "{error}"
        );
    }

    #[test]
    fn hoists_object_items_inside_a_property_union_array_branch() {
        let spec = parse(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  value:
    oneOf:
      - type: array
        items:
          type: object
          properties:
            id: { type: string }
      - { type: string }
"#,
        );
        let binding = spec
            .external_type_binding("ApiValueArrayItem")
            .expect("array element object in property-level oneOf should be hoisted");
        assert_eq!(
            binding.json_model().unwrap().schema["properties"]["id"]["type"],
            "string"
        );
    }

    #[test]
    fn accepts_nullable_multi_kind_union() {
        union_doc_result(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  value:
    oneOf:
      - { type: string }
      - { type: array, items: { type: number } }
      - { type: "null" }
"#,
        )
        .expect("nullable multi-kind union should load");
    }

    #[test]
    fn accepts_constrained_non_object_union_branches() {
        union_doc_result(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  value:
    oneOf:
      - { type: string, minLength: 3, pattern: "^[a-z]+$", format: uuid }
      - { type: integer, minimum: 0 }
  listOrName:
    oneOf:
      - { type: array, items: { type: number }, minItems: 1, uniqueItems: true }
      - { type: string, enum: [auto, manual] }
"#,
        )
        .expect("a non-object branch may carry its own constraints");
    }

    #[test]
    fn rejects_materialized_temporal_format_on_a_sum_type_branch() {
        let error = union_reject(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  value:
    oneOf:
      - { type: string, format: date-time }
      - { type: integer }
"#,
        );
        assert!(error.contains("`format: date-time`"), "{error}");
        assert!(error.contains("no wrapper type"), "{error}");
    }

    #[test]
    fn rejects_materialized_content_encoding_on_a_sum_type_branch() {
        let error = union_reject(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  value:
    oneOf:
      - { type: string, contentEncoding: base64 }
      - { type: integer }
"#,
        );
        assert!(error.contains("`contentEncoding: base64`"), "{error}");
    }

    #[test]
    fn accepts_materialized_keywords_on_a_nullable_branch() {
        // The nullability `oneOf` has a single non-null branch and synthesizes no
        // wrapper, so a materialized nullable field is unaffected by the sum-type
        // deferral above.
        union_doc_result(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  when:
    oneOf:
      - { type: string, format: date-time }
      - { type: "null" }
  blob:
    oneOf:
      - { type: string, contentEncoding: base64 }
      - { type: "null" }
"#,
        )
        .expect("a nullable materialized field should load");
    }

    #[test]
    fn rejects_null_only_two_branch_one_of() {
        let error = union_reject(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  value:
    oneOf:
      - { type: "null" }
      - { type: "null" }
"#,
        );
        assert!(error.contains("`null` kind"), "{error}");
    }

    // ----- P15 identifier namespace + `x-<lang>-name` override -----

    fn parse_for(language: Language, input: &str) -> Result<ApiSpec> {
        parse_api_spec_from_json_schema_for_language(language, input, PathBuf::from("api.yaml"))
    }

    fn reject_for(language: Language, input: &str) -> String {
        parse_for(language, input).unwrap_err().to_string()
    }

    #[test]
    fn member_override_accepts_and_is_recognized_as_extension() {
        // `x-<lang>-name` on a member is a recognized generator extension: the
        // loader accepts it (not rejected as unknown) for every target.
        let input = r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  legacyId:
    type: string
    x-go-name: LegacyID
    x-ts-name: legacyID
    x-py-name: legacy_ident
    x-java-name: legacyID
"#;
        for language in [
            Language::Go,
            Language::TypeScript,
            Language::Python,
            Language::Java,
        ] {
            parse_for(language, input)
                .unwrap_or_else(|error| panic!("{language:?} should accept override: {error}"));
        }
    }

    #[test]
    fn rejects_member_name_collision_after_recasing() {
        // `user_id` and `userId` both recase to Go `UserId` — a member collision.
        let error = reject_for(
            Language::Go,
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  user_id: { type: string }
  userId: { type: string }
"#,
        );
        assert!(
            error.contains("collision") && error.contains("UserId"),
            "{error}"
        );
    }

    #[test]
    fn member_collision_resolved_by_override() {
        // The same clash is admitted once one member carries an `x-go-name`
        // override — and the check is per target (Python would still collide, so
        // this is asserted for Go alone).
        let input = r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  user_id: { type: string }
  userId: { type: string, x-go-name: UserIdent }
"#;
        parse_for(Language::Go, input).expect("override resolves the Go collision");
    }

    /// TypeScript binds a service to a lower-camel `const`, so a service and a
    /// model of the same name emit `thing` and `Thing` and never collide. Deriving
    /// the service identifier as a type name claimed they did, rejecting a schema
    /// that generates cleanly — while missing the clash that can actually happen,
    /// a service whose lower-camel form lands on a model's converter const.
    #[test]
    fn typescript_service_identifier_is_lower_camel() {
        let service_and_model = r##"
$schema: https://json-schema.org/draft/2020-12/schema
nexusrpc: "1.0.0"
services:
  Thing:
    fqn: example.v1.Thing
    operations:
      doIt:
        input: { $ref: "#/$defs/Thing" }
$defs:
  Thing:
    type: object
    properties:
      id: { type: string }
"##;
        parse_for(Language::TypeScript, service_and_model)
            .expect("`thing` and `Thing` are distinct TypeScript identifiers");
        // Python names the service class `Thing`, so there it is a real clash.
        let error = reject_for(Language::Python, service_and_model);
        assert!(
            error.contains("collision") && error.contains("Thing"),
            "{error}"
        );

        // The clash TypeScript does have: the service's lower-camel form is the
        // model's converter identifier.
        let converter_clash = r##"
$schema: https://json-schema.org/draft/2020-12/schema
nexusrpc: "1.0.0"
services:
  ThingTransferTypeConverter:
    fqn: example.v1.Thing
    operations:
      doIt:
        input: { $ref: "#/$defs/Thing" }
$defs:
  Thing:
    type: object
    properties:
      id: { type: string }
"##;
        let error = reject_for(Language::TypeScript, converter_clash);
        assert!(error.contains("thingTransferTypeConverter"), "{error}");
    }

    #[test]
    fn typescript_service_may_use_removed_payload_validation_helper_name() {
        let input = r##"
$schema: https://json-schema.org/draft/2020-12/schema
nexusrpc: "1.0.0"
services:
  PayloadValidationError:
    operations:
      validate:
        input: { $ref: "#/$defs/Input" }
$defs:
  Input:
    type: object
    properties:
      value: { type: string }
"##;
        parse_for(Language::TypeScript, input)
            .expect("payloadValidationError is not part of the generated package");
    }

    /// A name synthesized *from a member* follows that member's override (P15).
    /// Two default-bearing members that recase alike collide on the TS
    /// `DEFAULT_<FIELD>` constant; the override has to reach the constant, or the
    /// rejection's own fix-it cannot resolve it and the only escape left is
    /// renaming the JSON property — a change to the wire contract.
    #[test]
    fn default_constant_collision_resolved_by_override() {
        let colliding = r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  retryCount: { type: string, default: "a" }
  retry_count: { type: string, default: "b" }
"#;
        let error = reject_for(Language::TypeScript, colliding);
        assert!(error.contains("collision"), "{error}");

        let resolved = r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  retryCount: { type: string, default: "a" }
  retry_count: { type: string, default: "b", x-ts-name: retriesTwo }
"#;
        parse_for(Language::TypeScript, resolved)
            .expect("the override moves the DEFAULT_ constant with the member");
    }

    /// The Go closed-value defined type is `<Type><Member>` off the *emitted*
    /// member identifier, so an `x-go-name` override moves it out of a clash with
    /// a declared type — matching Java's nested value class, which already
    /// followed the override.
    #[test]
    fn closed_value_type_collision_resolved_by_override() {
        // The harness names the file-root model `Api`, so the synthesized
        // closed-value type is `ApiKind` — which the `$defs` entry then clashes
        // with.
        let colliding = r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  kind: { type: string, const: widget }
$defs:
  ApiKind:
    type: object
    properties:
      x: { type: string }
"#;
        let error = reject_for(Language::Go, colliding);
        assert!(
            error.contains("collision") && error.contains("ApiKind"),
            "{error}"
        );

        let resolved = r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  kind: { type: string, const: widget, x-go-name: Category }
$defs:
  ApiKind:
    type: object
    properties:
      x: { type: string }
"#;
        parse_for(Language::Go, resolved)
            .expect("the override moves the closed-value type with the member");
    }

    /// A `const` member's `<FIELD>_CONST` binding is module-scope, so it takes
    /// part in the collision pass even though it is not exported. Two of them can
    /// coincide through the model-name disambiguator — `A.kind` is prefixed
    /// (`kind` is not unique) to `A_KIND_CONST`, which is exactly what the unique
    /// `C.aKind` produces unprefixed. Emitting both is a duplicate `const` in one
    /// module, a TypeScript `SyntaxError`.
    #[test]
    fn const_constant_collision_rejects_and_is_resolved_by_override() {
        let colliding = r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
additionalProperties: false
$defs:
  A:
    type: object
    properties:
      kind: { type: string, const: one }
  B:
    type: object
    properties:
      kind: { type: string, const: two }
  C:
    type: object
    properties:
      aKind: { type: string, const: three }
"#;
        let error = reject_for(Language::TypeScript, colliding);
        assert!(
            error.contains("collision") && error.contains("A_KIND_CONST"),
            "{error}"
        );

        let resolved = r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
additionalProperties: false
$defs:
  A:
    type: object
    properties:
      kind: { type: string, const: one }
  B:
    type: object
    properties:
      kind: { type: string, const: two }
  C:
    type: object
    properties:
      aKind: { type: string, const: three, x-ts-name: cKind }
"#;
        parse_for(Language::TypeScript, resolved)
            .expect("the override moves the _CONST binding with the member");
    }

    #[test]
    fn value_constant_collision_resolved_by_enum_names_override() {
        // `"user-admin"` and `"user_admin"` both encode to the Go value constant
        // `UserAdmin` — a value-constant collision that rejects by default.
        let colliding = r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  role:
    type: string
    enum: ["user-admin", "user_admin"]
"#;
        let error = reject_for(Language::Go, colliding);
        assert!(
            error.contains("UserAdmin") && error.contains("collision"),
            "{error}"
        );

        // An `x-go-enum-names` override renames one member's constant verbatim,
        // separating the two (per target — Python has no value constant, so the
        // keyword is inert but the schema still loads).
        let overridden = r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  role:
    type: string
    enum: ["user-admin", "user_admin"]
    x-go-enum-names: { "user_admin": "UserAdminAlt" }
"#;
        parse_for(Language::Go, overridden)
            .expect("value-constant override resolves the Go collision");
    }

    #[test]
    fn value_token_legality_is_target_scoped_and_names_the_remedy() {
        let input = r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  marker: { type: string, const: "-" }
"#;
        for language in [Language::Go, Language::Java] {
            let error = reject_for(language, input);
            assert!(
                error.contains("does not encode") && error.contains("const-name"),
                "{language:?}: {error}"
            );
        }
        for language in [Language::TypeScript, Language::Python] {
            parse_for(language, input).unwrap_or_else(|error| {
                panic!("{language:?} emits a literal, not a constant: {error}")
            });
        }
    }

    #[test]
    fn go_numeric_value_constant_uses_the_canonical_decimal_token() {
        let input = r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  ratio: { type: number, const: 1.0 }
  clash: { $ref: "#/$defs/ApiRatio1" }
$defs:
  ApiRatio1: { type: object, properties: { value: { type: string } } }
"##;
        let error = reject_for(Language::Go, input);
        assert!(
            error.contains("collision") && error.contains("ApiRatio1"),
            "{error}"
        );
    }

    #[test]
    fn active_value_constant_overrides_validate_placement_and_member_keys() {
        let unmatched = r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  ratio:
    type: number
    enum: [1.0, 2.5]
    x-go-enum-names: { "1.0": One }
"#;
        let error = reject_for(Language::Go, unmatched);
        assert!(error.contains("does not name an `enum` member"), "{error}");

        let canonical = unmatched.replace("\"1.0\"", "\"1\"");
        parse_for(Language::Go, &canonical).expect("canonical numeric key selects the member");

        let wrong_keyword = r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  marker: { type: string, enum: [a, b], x-go-const-name: Marker }
"#;
        let error = reject_for(Language::Go, wrong_keyword);
        assert!(error.contains("only valid beside `const`"), "{error}");
    }

    #[test]
    fn rejects_type_name_collision_between_defs() {
        // Two `$defs` keys that recase to the same type identifier (`userProfile`
        // and `user_profile` → both `UserProfile`) collide in the package scope.
        let error = reject_for(
            Language::Go,
            r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  a: { $ref: "#/$defs/userProfile" }
  b: { $ref: "#/$defs/user_profile" }
$defs:
  userProfile:
    type: object
    properties: { x: { type: string } }
  user_profile:
    type: object
    properties: { y: { type: string } }
"##,
        );
        assert!(
            error.contains("collision") && error.contains("UserProfile"),
            "{error}"
        );
    }

    #[test]
    fn type_collision_resolved_by_type_override() {
        // A type-level `x-go-name` moves the emitted identifier and so resolves
        // the same clash — per target (Python still collides).
        let input = r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  a: { $ref: "#/$defs/userProfile" }
  b: { $ref: "#/$defs/user_profile" }
$defs:
  userProfile:
    type: object
    x-go-name: UserProfileAlt
    properties: { x: { type: string } }
  user_profile:
    type: object
    properties: { y: { type: string } }
"##;
        parse_for(Language::Go, input).expect("type override resolves the Go collision");
        let error = reject_for(Language::Python, input);
        assert!(
            error.contains("collision") && error.contains("UserProfile"),
            "{error}"
        );
    }

    #[test]
    fn rejects_root_type_name_collision_with_defs_entry() {
        // `thing.yaml`'s root schema derives the type name `Thing`, and the same
        // file declares `$defs.Thing` — two different schemas under one model
        // identity, which is a P15 collision in every target's namespace. The
        // diagnostic names the identifier and both origins (the root schema's
        // file-name derivation and the `$defs` entry), and the fix-it is a rename:
        // an `x-<lang>-name` moves the emitted identifier, not the identity.
        let input = r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  rootOnlyField: { type: string }
  nested: { $ref: "#/$defs/Thing" }
$defs:
  Thing:
    type: object
    properties: { defOnlyField: { type: integer } }
"##;
        for language in [
            Language::Go,
            Language::TypeScript,
            Language::Python,
            Language::Java,
        ] {
            let error = parse_api_spec_from_json_schema_for_language(
                language,
                input,
                PathBuf::from("thing.yaml"),
            )
            .expect_err("a root/`$defs` name collision is a load reject")
            .to_string();
            assert!(
                error.contains("`Thing`")
                    && error.contains("file name `thing.yaml`")
                    && error.contains("`$defs.Thing`")
                    && error.contains("Rename the `$defs` entry")
                    && error.contains("rename the file")
                    && error.contains("`x-<lang>-name` override cannot separate them"),
                "{language:?}: {error}"
            );
        }

        // The collision is the *root type's* name, so a definitions-only file of
        // the same base name (no file-root type) keeps loading.
        let definitions_only = r##"
$schema: https://json-schema.org/draft/2020-12/schema
$defs:
  Thing:
    type: object
    properties: { defOnlyField: { type: integer } }
"##;
        parse_api_spec_from_json_schema_for_language(
            Language::Go,
            definitions_only,
            PathBuf::from("thing.yaml"),
        )
        .expect("a definitions-only file emits no root type, so nothing collides");
    }

    #[test]
    fn rejects_hoisted_shape_name_collision_with_root_type_name() {
        // The inline object at `$defs.User.properties.profile` is named
        // `UserProfile`, which is also the type name `userProfile.yaml`'s root
        // schema derives — so the synthesized name collides with the root type.
        let error = parse_api_spec_from_json_schema_for_language(
            Language::TypeScript,
            r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  user: { $ref: "#/$defs/User" }
$defs:
  User:
    type: object
    properties:
      profile:
        type: object
        properties: { nickname: { type: string } }
"##,
            PathBuf::from("userProfile.yaml"),
        )
        .expect_err("a synthesized name that collides with the root type is a load reject")
        .to_string();
        assert!(
            error.contains("`UserProfile`")
                && error.contains("`$defs.User.properties.profile`")
                && error.contains("file name `userProfile.yaml`")
                && error.contains("`x-ts-name`")
                && error.contains("rename the file"),
            "{error}"
        );
    }

    #[test]
    fn rejects_two_schemas_sharing_one_model_identity() {
        // The backstop behind the two rejects above: whatever route two different
        // schemas take to one model identity, they never collapse into a single
        // emitted type. Here two root types derive `User` in a flat (module-less)
        // load of both files.
        let error = api_spec_from_json_schema_sources(
            Language::Python,
            vec![
                (
                    PathBuf::from("a/user.yaml"),
                    r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties: { first: { type: string } }
"#
                    .to_string(),
                ),
                (
                    PathBuf::from("b/user.yaml"),
                    r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties: { second: { type: string } }
"#
                    .to_string(),
                ),
            ],
        )
        .expect_err("two schemas under one identity is a load reject")
        .to_string();
        assert!(
            error.contains("model identity `User`") && error.contains("rename"),
            "{error}"
        );
    }

    #[test]
    fn rejects_invalid_and_reserved_overrides() {
        // A leading-digit override is not a legal identifier.
        let error = reject_for(
            Language::Go,
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  code: { type: string, x-go-name: "2fa" }
"#,
        );
        assert!(
            error.contains("x-go-name") && error.contains("legal"),
            "{error}"
        );

        // A reserved-word override is rejected.
        let error = reject_for(
            Language::Python,
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  class_field: { type: string, x-py-name: "class" }
"#,
        );
        assert!(error.contains("x-py-name"), "{error}");
    }

    #[test]
    fn rejects_synthesized_closed_type_colliding_with_declared_type_go() {
        // `Palette.color` (enum) synthesizes the Go defined type `PaletteColor`,
        // which collides with the declared `$defs/PaletteColor` — a package-scope
        // clash caught only for Go (Python closes the enum inline).
        let input = r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  palette: { $ref: "#/$defs/Palette" }
  pc: { $ref: "#/$defs/PaletteColor" }
$defs:
  Palette:
    type: object
    properties:
      color: { type: string, enum: [red, green] }
  PaletteColor:
    type: object
    properties:
      label: { type: string }
"##;
        let error = reject_for(Language::Go, input);
        assert!(
            error.contains("collision") && error.contains("PaletteColor"),
            "{error}"
        );
        // Python synthesizes no defined type, so the same schema is accepted.
        parse_for(Language::Python, input).expect("Python has no such synthesized type");
    }

    #[test]
    fn rejects_or_default_accessor_colliding_with_member_go() {
        // The Go `<Field>OrDefault()` accessor shares the struct method-set: a
        // sibling member that recases to `FooOrDefault` is a field/method clash.
        let input = r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  foo: { type: string, default: "x" }
  fooOrDefault: { type: string }
"#;
        let error = reject_for(Language::Go, input);
        assert!(
            error.contains("collision") && error.contains("FooOrDefault"),
            "{error}"
        );
        // Python surfaces the default natively (no accessor), so no clash.
        parse_for(Language::Python, input).expect("Python has no OrDefault accessor");
    }

    #[test]
    fn rejects_colliding_default_constants_typescript() {
        // TypeScript hoists a defaulted member's value to a module-level
        // `DEFAULT_<FIELD>` constant, named off the **emitted** member identifier.
        // Two members that stay distinct as identifiers (`fooBar` / `foo_bar`, held
        // apart by their overrides) still shout to one `DEFAULT_FOO_BAR`, and the
        // model-name qualification cannot separate two members of one model.
        for (language, override_key) in [(Language::TypeScript, "x-ts-name")] {
            let input = format!(
                r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
additionalProperties: false
properties:
  first: {{ type: string, default: "x", {override_key}: fooBar }}
  second: {{ type: string, default: "y", {override_key}: foo_bar }}
"##
            );
            let error = reject_for(language, &input);
            assert!(
                error.contains("collision") && error.contains("DEFAULT_FOO_BAR"),
                "{language:?}: {error}"
            );
            // Go and Java keep the default on the model (no module-level constant),
            // so the same schema is accepted there.
            parse_for(Language::Go, &input).expect("Go emits no DEFAULT_ constants");
            parse_for(Language::Java, &input).expect("Java emits no DEFAULT_ constants");

            // The escape hatch reaches the constant, because the constant follows
            // the member it was synthesized from (P15).
            let resolved = format!(
                r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
additionalProperties: false
properties:
  first: {{ type: string, default: "x", {override_key}: fooBar }}
  second: {{ type: string, default: "y", {override_key}: fooBarTwo }}
"##
            );
            parse_for(language, &resolved)
                .expect("the override moves the DEFAULT_ constant with the member");
        }

        // A second model may not rename the first model's already-exported
        // constant. The stable DEFAULT_<FIELD> spelling makes the second claim a
        // reject; an x-ts-name on either declaring member is the escape hatch.
        let across_models = r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  a: { $ref: "#/$defs/A" }
  b: { $ref: "#/$defs/B" }
$defs:
  A:
    type: object
    properties:
      fooBar: { type: string, default: "x" }
  B:
    type: object
    properties:
      foo_bar: { type: string, default: "y" }
"##;
        let error = reject_for(Language::TypeScript, across_models);
        assert!(
            error.contains("collision") && error.contains("DEFAULT_FOO_BAR"),
            "{error}"
        );
        parse_for(Language::Python, across_models)
            .expect("Python emits properties rather than DEFAULT_ constants");
    }

    #[test]
    fn checks_exported_and_private_names_at_their_real_multifile_scopes() {
        let model =
            |full_name: &str, model_name: &str, module_key: &str, schema: Value| ManifestModel {
                full_name: full_name.to_string(),
                local_name: model_name.to_string(),
                model_name: model_name.to_string(),
                module_key: module_key.to_string(),
                schema,
            };
        let plain = || serde_json::json!({"type":"object","properties":{}});

        // Exported converter values meet in the root TypeScript barrel even
        // when their model declarations live in different modules.
        let converters = vec![
            model(
                "a#HTTPError",
                "HTTPError",
                "a",
                serde_json::json!({"type":"object","x-ts-name":"HTTPError","properties":{}}),
            ),
            model("b#HttpError", "HttpError", "b", plain()),
        ];
        let error = build_name_manifest(Language::TypeScript, &converters, &[])
            .expect_err("converter exports collide in the root barrel")
            .to_string();
        assert!(error.contains("httpErrorTransferTypeConverter"), "{error}");

        // Private Python converter classes are checked in every real module,
        // but the same private spelling remains legal in two separate modules.
        let same_module = vec![
            model(
                "a#One",
                "One",
                "a",
                serde_json::json!({"type":"object","x-py-name":"Contact","properties":{}}),
            ),
            model(
                "a#Two",
                "Two",
                "a",
                serde_json::json!({"type":"object","x-py-name":"_ContactTransferTypeConverter","properties":{}}),
            ),
        ];
        let error = build_name_manifest(Language::Python, &same_module, &[])
            .expect_err("private converter collision in a non-root module")
            .to_string();
        assert!(error.contains("_ContactTransferTypeConverter"), "{error}");

        let separate_modules = vec![
            model(
                "a#One",
                "One",
                "a",
                serde_json::json!({"type":"object","x-py-name":"Contact","properties":{}}),
            ),
            model(
                "b#Two",
                "Two",
                "b",
                serde_json::json!({"type":"object","x-py-name":"ContactTwo","properties":{}}),
            ),
        ];
        build_name_manifest(Language::Python, &separate_modules, &[])
            .expect("unrelated module-private converter names remain independent");
    }

    #[test]
    fn rejects_typescript_member_bindings_and_object_intrinsics() {
        for member in [
            "arguments",
            "eval",
            "out",
            "raw",
            "undefined",
            "violations",
            "constructor",
            "toString",
        ] {
            let input = format!(
                "$schema: https://json-schema.org/draft/2020-12/schema\ntype: object\nadditionalProperties: false\nproperties:\n  {member}: {{ type: string }}\n"
            );
            let error = reject_for(Language::TypeScript, &input);
            assert!(
                error.contains(member) && error.contains("x-ts-name"),
                "{member}: {error}"
            );
        }
    }

    #[test]
    fn rejects_service_case_folds_and_service_file_import_collisions() {
        let folded = r#"
nexusrpc: "1.0.0"
services:
  HTTPService:
    fqn: example.Alpha
    operations: { ping: { input: { type: object, properties: {} } } }
  HttpService:
    fqn: example.Beta
    operations: { pong: { input: { type: object, properties: {} } } }
"#;
        for language in [
            Language::Go,
            Language::TypeScript,
            Language::Python,
            Language::Java,
        ] {
            let error = reject_for(language, folded);
            assert!(
                error.contains("HTTPService") && error.contains("HttpService"),
                "{language:?}: {error}"
            );
        }

        for (language, override_line, imported) in [
            (Language::Go, "x-go-name: nexus", "nexus"),
            (Language::TypeScript, "x-ts-name: nexus", "nexus"),
            (Language::Python, "x-py-name: Operation", "Operation"),
            (Language::Java, "x-java-name: Service", "Service"),
        ] {
            let input = format!(
                "nexusrpc: \"1.0.0\"\nservices:\n  Alpha:\n    {override_line}\n    operations:\n      ping:\n        input: {{ type: object, properties: {{}} }}\n"
            );
            let error = reject_for(language, &input);
            assert!(
                error.contains(imported) && error.contains("service-file import"),
                "{language:?}: {error}"
            );
        }

        // A model named like an SDK import is harmless until an operation's I/O
        // makes that model enter the service file. It may still be used by
        // another model in the models file without shadowing the service import.
        let unused_in_service = r##"
$schema: https://json-schema.org/draft/2020-12/schema
nexusrpc: "1.0.0"
services:
  Alpha:
    operations:
      ping:
        input: { type: object, properties: {} }
$defs:
  Operation: { type: object, properties: { value: { type: string } } }
  Witness:
    type: object
    properties:
      operation: { $ref: "#/$defs/Operation" }
"##;
        for language in [Language::Python, Language::Java] {
            parse_for(language, unused_in_service).unwrap_or_else(|error| {
                panic!("{language:?} must allow an SDK-like model absent from service I/O: {error}")
            });
        }

        let used_in_service = unused_in_service.replace(
            "input: { type: object, properties: {} }",
            "input: { $ref: \"#/$defs/Operation\" }",
        );
        for language in [Language::Python, Language::Java] {
            let error = reject_for(language, &used_in_service);
            assert!(
                error.contains("Operation") && error.contains("service-file import"),
                "{language:?}: {error}"
            );
        }
    }

    #[test]
    fn reserves_go_native_service_client_and_constructor_identifiers() {
        for declared in ["ChatClient", "NewChatClient"] {
            let input = format!(
                r##"
$schema: https://json-schema.org/draft/2020-12/schema
nexusrpc: "1.0.0"
services:
  Chat:
    operations:
      ping: {{ input: {{ $ref: "#/$defs/{declared}" }} }}
$defs:
  {declared}: {{ type: object, properties: {{ value: {{ type: string }} }} }}
"##
            );
            let error = reject_for(Language::Go, &input);
            assert!(
                error.contains(declared) && error.contains("native client"),
                "{declared}: {error}"
            );
        }
    }

    #[test]
    fn rejects_python_default_backing_field_collision() {
        let colliding = r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
additionalProperties: false
properties:
  greeting: { type: string, default: hello }
  raw: { type: string, x-py-name: _greeting }
"#;
        let error = reject_for(Language::Python, colliding);
        assert!(
            error.contains("collision") && error.contains("_greeting"),
            "{error}"
        );

        let resolved = r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
additionalProperties: false
properties:
  greeting: { type: string, default: hello, x-py-name: salutation }
  raw: { type: string, x-py-name: _greeting }
"#;
        parse_for(Language::Python, resolved)
            .expect("x-py-name moves the property and its private backing field");
    }

    #[test]
    fn rejects_colliding_declared_field_sets_python() {
        // An open object hoists its declared wire keys to a module-level
        // `_<MODEL>_DECLARED` frozenset. `to_shouty_snake_case` is not injective
        // over the verbatim type overrides — `ContactPy` and `ContactPY` both
        // shout to `CONTACT_PY` — and the loser's declared property would leak
        // into the winner's catch-all instead (P13/P15).
        let input = r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  a: { $ref: "#/$defs/Alpha" }
  b: { $ref: "#/$defs/Beta" }
$defs:
  Alpha:
    x-py-name: ContactPy
    type: object
    properties:
      count: { type: integer }
  Beta:
    x-py-name: ContactPY
    type: object
    properties:
      b: { type: string }
"##;
        let error = reject_for(Language::Python, input);
        assert!(
            error.contains("collision") && error.contains("_CONTACT_PY_DECLARED"),
            "{error}"
        );
        // The overrides are Python-only, so every other target sees `Alpha` and
        // `Beta` and is unaffected.
        for language in [Language::Go, Language::TypeScript, Language::Java] {
            parse_for(language, input)
                .unwrap_or_else(|error| panic!("{language:?} sees no override: {error}"));
        }
    }

    #[test]
    fn rejects_colliding_converter_class_python() {
        // A model's converter class is `_<Model>TransferTypeConverter`; a verbatim
        // type override can name a *type* that exact identifier.
        let input = r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  a: { $ref: "#/$defs/Contact" }
  b: { $ref: "#/$defs/Other" }
$defs:
  Contact:
    type: object
    properties:
      count: { type: integer }
  Other:
    x-py-name: _ContactTransferTypeConverter
    type: object
    properties:
      b: { type: string }
"##;
        let error = reject_for(Language::Python, input);
        assert!(
            error.contains("collision") && error.contains("_ContactTransferTypeConverter"),
            "{error}"
        );
    }

    /// Go names an inline union's sealed interface `<Type><Member>`, its variant
    /// wrappers `<Union><Kind>` and its dispatcher `unmarshal<Union>`. Nothing
    /// registered any of them, so two unions deriving one name silently merged —
    /// one interface bound the other's branch set. This is the schema
    /// `rejects_colliding_union_functions_python` asserts Python must reject.
    #[test]
    fn rejects_colliding_union_names_go() {
        let input = r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  u: { $ref: "#/$defs/FooBar" }
  f: { $ref: "#/$defs/Foo" }
$defs:
  FooBar:
    oneOf:
      - { type: string }
      - { type: integer }
  Foo:
    type: object
    additionalProperties: false
    properties:
      bar:
        oneOf:
          - { type: string }
          - { type: boolean }
"##;
        let error = reject_for(Language::Go, input);
        assert!(
            error.contains("collision") && error.contains("FooBar"),
            "{error}"
        );
        // P15's escape hatch, both halves: renaming the owning model moves the
        // inline union's interface, wrappers and dispatcher with it...
        let renamed_model = input.replace(
            "  Foo:\n    type: object",
            "  Foo:\n    x-go-name: Renamed\n    type: object",
        );
        parse_for(Language::Go, &renamed_model)
            .expect("an `x-go-name` on the owning model moves the inline union's names");
        // ...and so does renaming the *member* the union hangs off, which is the
        // remedy this pass's own diagnostic names. Deriving the suffix from the
        // raw JSON name made that fix-it a lie.
        let renamed_member = input.replace(
            "      bar:\n        oneOf:",
            "      bar:\n        x-go-name: Renamed\n        oneOf:",
        );
        parse_for(Language::Go, &renamed_member)
            .expect("an `x-go-name` on the union-typed member moves the union's names");
    }

    #[test]
    fn accepts_distinct_semantic_go_regex_helpers() {
        let input = r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  code:
    type: array
    items: { type: string, pattern: "^a" }
  codeItem: { type: string, pattern: "^b" }
"#;
        parse_for(Language::Go, input)
            .expect("distinct pattern spellings synthesize distinct semantic helper names");
    }

    #[test]
    fn rejects_position_derived_java_regex_name_collisions() {
        let input = r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  code:
    type: array
    items: { type: string }
    contains: { type: string, pattern: "^a" }
  codeContains: { type: string, pattern: "^b" }
"#;
        let error = reject_for(Language::Java, input);
        assert!(
            error.contains("CODE_CONTAINS_PATTERN")
                && error.contains("collision")
                && error.contains("x-java-name"),
            "{error}"
        );
        let renamed = input.replace(
            "codeContains: { type: string, pattern: \"^b\" }",
            "codeContains: { type: string, pattern: \"^b\", x-java-name: other }",
        );
        parse_for(Language::Java, &renamed)
            .expect("x-java-name moves the compiled Pattern field with the member");
        // Go uses semantic helper names; TypeScript and Python do not synthesize
        // position-derived identifiers here.
        for language in [Language::Go, Language::TypeScript, Language::Python] {
            parse_for(language, input)
                .unwrap_or_else(|error| panic!("{language:?} emits no colliding name: {error}"));
        }
    }

    #[test]
    fn rejects_nullable_java_typed_map_constraint_field_collisions() {
        let input = r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  additionalPropertiesValue: { type: string, pattern: "^declared$" }
additionalProperties:
  oneOf:
    - { type: string, pattern: "^catch-all$" }
    - { type: "null" }
"#;
        let error = reject_for(Language::Java, input);
        assert!(
            error.contains("ADDITIONAL_PROPERTIES_VALUE_PATTERN")
                && error.contains("collision")
                && error.contains("x-java-name"),
            "{error}"
        );

        let renamed = input.replace(
            "additionalPropertiesValue: { type: string, pattern: \"^declared$\" }",
            "additionalPropertiesValue: { type: string, pattern: \"^declared$\", x-java-name: declaredValue }",
        );
        parse_for(Language::Java, &renamed)
            .expect("x-java-name moves the declared member's compiled Pattern field");
    }

    #[test]
    fn rejects_java_inline_union_nested_type_collisions() {
        let interface_collision = r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  serializer:
    oneOf:
      - { type: string }
      - { type: integer }
"#;
        let error = reject_for(Language::Java, interface_collision);
        assert!(
            error.contains("Serializer")
                && error.contains("inline union")
                && error.contains("collision"),
            "{error}"
        );
        let renamed = interface_collision.replace(
            "  serializer:\n",
            "  serializer:\n    x-java-name: payload\n",
        );
        parse_for(Language::Java, &renamed)
            .expect("x-java-name moves the inline interface and wrappers");

        let wrapper_collision = r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  choice:
    oneOf:
      - { type: string }
      - { type: integer }
  choiceString: { type: string, const: fixed }
"#;
        let error = reject_for(Language::Java, wrapper_collision);
        assert!(
            error.contains("ChoiceString")
                && error.contains("variant wrapper")
                && error.contains("collision"),
            "{error}"
        );
    }

    /// Two members that recase alike are separated by an `x-go-name`, and the
    /// union names synthesized from them have to follow: deriving the suffix
    /// from the JSON name rejected this outright, with no remedy left.
    #[test]
    fn union_names_follow_a_member_override_go() {
        let input = r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  f: { $ref: "#/$defs/Foo" }
$defs:
  Foo:
    type: object
    additionalProperties: false
    properties:
      idOrName:
        oneOf:
          - { type: string }
          - { type: integer }
      id_or_name:
        x-go-name: IdOrNameSnake
        oneOf:
          - { type: string }
          - { type: boolean }
"##;
        parse_for(Language::Go, input)
            .expect("the member override separates both unions' synthesized names");
    }

    #[test]
    fn rejects_colliding_inline_union_serializers_typescript() {
        let input = r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  left: { $ref: "#/$defs/Foo" }
  right: { $ref: "#/$defs/FooBar" }
$defs:
  Foo:
    type: object
    properties:
      barBaz:
        oneOf:
          - { type: string }
          - { $ref: "#/$defs/Leaf" }
  FooBar:
    type: object
    properties:
      baz:
        oneOf:
          - { type: integer }
          - { $ref: "#/$defs/Other" }
  Leaf: { type: object, properties: { value: { type: string } } }
  Other: { type: object, properties: { count: { type: integer } } }
"##;
        let error = reject_for(Language::TypeScript, input);
        assert!(
            error.contains("collision") && error.contains("serializeFooBarBaz"),
            "{error}"
        );

        let renamed = input.replace(
            "      baz:\n        oneOf:",
            "      baz:\n        x-ts-name: renamedBaz\n        oneOf:",
        );
        parse_for(Language::TypeScript, &renamed)
            .expect("x-ts-name moves the inline union serializer with its member");
    }

    #[test]
    fn inline_union_serializer_manifest_matches_typescript_wire_transforms() {
        let assertion_only = r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  left: { $ref: "#/$defs/Foo" }
  right: { $ref: "#/$defs/FooBar" }
$defs:
  Foo:
    type: object
    properties:
      barBaz:
        oneOf:
          - { type: array, items: { type: string, format: email } }
          - { type: boolean }
  FooBar:
    type: object
    properties:
      baz:
        oneOf:
          - { type: array, items: { type: string, format: email } }
          - { type: integer }
"##;
        parse_for(Language::TypeScript, assertion_only)
            .expect("email validation does not emit either colliding serializer helper");

        let transforming = assertion_only.replace(
            "type: string, format: email",
            "type: string, contentEncoding: base64",
        );
        let error = reject_for(Language::TypeScript, &transforming);
        assert!(
            error.contains("collision") && error.contains("serializeFooBarBaz"),
            "{error}"
        );
    }

    /// A union's synthesized variant wrapper shares the package namespace with
    /// authored types.
    #[test]
    fn rejects_union_variant_wrapper_colliding_with_a_def_go() {
        let input = r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  u: { $ref: "#/$defs/Tag" }
  s: { $ref: "#/$defs/TagString" }
$defs:
  Tag:
    oneOf:
      - { type: string }
      - { type: integer }
  TagString:
    type: object
    properties:
      v: { type: string }
"##;
        let error = reject_for(Language::Go, input);
        assert!(
            error.contains("collision") && error.contains("TagString"),
            "{error}"
        );
    }

    /// The operation key grammar admits both `getId` and `getID`: they recase to
    /// one identifier in every target *and* derive one default wire name, so the
    /// two bindings silently collapsed into one.
    #[test]
    fn rejects_colliding_operation_identifiers() {
        let input = r#"
nexusrpc: 1.0.0
services:
  Chat:
    operations:
      getId:
        input: { type: object, properties: { a: { type: string } } }
      getID:
        input: { type: object, properties: { b: { type: string } } }
"#;
        for language in [
            Language::Go,
            Language::TypeScript,
            Language::Python,
            Language::Java,
        ] {
            let error = reject_for(language, input);
            assert!(
                error.contains("getId") && error.contains("getID"),
                "{language:?}: {error}"
            );
        }
    }

    /// Two operations deriving one wire name is a clash in every target at once,
    /// independent of the emitted identifiers.
    #[test]
    fn rejects_colliding_operation_wire_names() {
        let input = r#"
nexusrpc: 1.0.0
services:
  Chat:
    operations:
      send:
        fqn: Deliver
      deliver:
        input: { type: object, properties: { a: { type: string } } }
"#;
        let error = reject_for(Language::Go, input);
        assert!(error.contains("wire name `Deliver`"), "{error}");
    }

    /// An operation named `import` emits `void import(In)` in Java and is
    /// auto-mangled to `import_` in TypeScript and Python — the mangling P15
    /// forbids outright. Go emits the exported `Import`, which is fine.
    #[test]
    fn rejects_reserved_word_operation_name() {
        let input = r#"
nexusrpc: 1.0.0
services:
  Chat:
    operations:
      import:
        input: { type: object, properties: { a: { type: string } } }
"#;
        for language in [Language::TypeScript, Language::Python, Language::Java] {
            let error = reject_for(language, input);
            assert!(
                error.contains("reserved word") && error.contains("import"),
                "{language:?}: {error}"
            );
        }
        // Go exports the operation as `Import`, which is not reserved.
        parse_for(Language::Go, input).expect("Go emits `Import`, which is legal");
        // The escape hatch is the per-target override.
        let renamed = input.replace("      import:\n", "      import:\n        x-py-name: import_op\n        x-ts-name: importOp\n        x-java-name: importOp\n");
        for language in [Language::TypeScript, Language::Python, Language::Java] {
            parse_for(language, &renamed)
                .unwrap_or_else(|error| panic!("{language:?} override should apply: {error}"));
        }
    }

    /// D11: an `fqn` present but empty names the binding with the empty string
    /// on the wire in all four targets.
    #[test]
    fn rejects_empty_service_and_operation_fqn() {
        let service = r#"
nexusrpc: 1.0.0
services:
  Chat:
    fqn: ""
    operations:
      send:
        input: { type: object, properties: { a: { type: string } } }
"#;
        let error = reject_for(Language::Go, service);
        assert!(error.contains("`fqn` is empty"), "{error}");
        let operation = r#"
nexusrpc: 1.0.0
services:
  Chat:
    operations:
      send:
        fqn: ""
        input: { type: object, properties: { a: { type: string } } }
"#;
        let error = reject_for(Language::Go, operation);
        assert!(error.contains("`fqn` is empty"), "{error}");
    }

    /// A module-path segment is used verbatim as a directory, a file name and an
    /// import path component; only *generated-file* names were checked.
    #[test]
    fn rejects_reserved_word_module_segment() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("api");
        fs::create_dir_all(&root).unwrap();
        let plain = r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  a: { type: string }
"#;
        fs::write(root.join("ok.json"), plain).unwrap();
        fs::write(root.join("class.json"), plain).unwrap();
        let error = load_api_spec_tree_from_json_schema_for_language_with_inputs(
            Language::Python,
            std::slice::from_ref(&root),
        )
        .expect_err("`class.json` emits `from .class import Class`")
        .to_string();
        assert!(
            error.contains("class") && error.contains("reserved word"),
            "{error}"
        );
    }

    #[test]
    fn rejects_module_segment_that_is_not_an_identifier() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("api");
        fs::create_dir_all(&root).unwrap();
        let plain = r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  a: { type: string }
"#;
        fs::write(root.join("ok.json"), plain).unwrap();
        fs::write(root.join("2fa.json"), plain).unwrap();
        let error = load_api_spec_tree_from_json_schema_for_language_with_inputs(
            Language::Python,
            std::slice::from_ref(&root),
        )
        .expect_err("a leading digit is not a legal module name")
        .to_string();
        assert!(error.contains("not a valid module name"), "{error}");
    }

    /// Go puts a struct's fields and methods in one namespace, so a member named
    /// `validate` yields "type X has both field and method named Validate".
    #[test]
    fn rejects_member_colliding_with_the_go_method_set() {
        let input = r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  validate: { type: string }
"#;
        let error = reject_for(Language::Go, input);
        assert!(
            error.contains("collision") && error.contains("Validate"),
            "{error}"
        );
        // The other three emit no such method on the model.
        for language in [Language::TypeScript, Language::Python, Language::Java] {
            parse_for(language, input)
                .unwrap_or_else(|error| panic!("{language:?} should load: {error}"));
        }
        let renamed = input.replace(
            "  validate: { type: string }",
            "  validate: { type: string, x-go-name: ValidateField }",
        );
        parse_for(Language::Go, &renamed).expect("an `x-go-name` override moves the field");
    }

    /// Java nests a `Serializer`/`Deserializer` in every model class and imports
    /// `Violation` by simple name; a closed-value member named after one emits a
    /// duplicate nested class or shadows the runtime type.
    #[test]
    fn rejects_java_member_colliding_with_a_generated_nested_class() {
        for member in ["serializer", "deserializer", "violation"] {
            let input = format!(
                r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  {member}: {{ type: string, const: fixed }}
"#
            );
            let error = reject_for(Language::Java, &input);
            assert!(error.contains("collision"), "{member}: {error}");
            // Only Java nests these declarations inside the model.
            parse_for(Language::Go, &input)
                .unwrap_or_else(|error| panic!("{member}: Go should load: {error}"));
        }
    }

    /// The generated Java deserializer declares the member slots at method
    /// scope, so every local it binds in that body shares the member namespace:
    /// `index` beside any array member emits `variable index is already defined
    /// in method deserialize(...)`. Most are Java-only; `violations` is also a
    /// TypeScript converter local and is rejected there independently.
    #[test]
    fn rejects_java_member_colliding_with_a_generated_deserializer_local() {
        for member in [
            "index",
            "element",
            "field",
            "node",
            "violations",
            "length",
            "parsed",
            "parser",
            "context",
            "items",
            "rawSeen",
            "priorIndex",
            "numberValue",
            "nestedViolations",
        ] {
            let input = format!(
                r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
additionalProperties: false
properties:
  {member}: {{ type: string }}
  tags: {{ type: array, items: {{ type: string }} }}
"#
            );
            let error = reject_for(Language::Java, &input);
            assert!(
                error.contains("collision")
                    && error.contains("generated deserializer local")
                    && error.contains(member),
                "{member}: {error}"
            );
            for language in [Language::Go, Language::Python] {
                parse_for(language, &input).unwrap_or_else(|error| {
                    panic!("{member}: {language:?} binds no such local: {error}")
                });
            }
            if member == "violations" {
                let error = reject_for(Language::TypeScript, &input);
                assert!(error.contains("generated converter binding"), "{error}");
            } else {
                parse_for(Language::TypeScript, &input)
                    .unwrap_or_else(|error| panic!("{member}: TypeScript should load: {error}"));
            }
            // P15's escape hatch moves the slot, and with it the collision.
            let renamed = input.replace(
                &format!("  {member}: {{ type: string }}"),
                &format!("  {member}: {{ type: string, x-java-name: renamedMember }}"),
            );
            parse_for(Language::Java, &renamed)
                .unwrap_or_else(|error| panic!("{member}: override should apply: {error}"));
        }
    }

    /// The nested-array parse mints one loop local per level (`items1`,
    /// `index1`, `element1`, `path1`, then `…2`, `…3`), so the family is
    /// unbounded in the schema's nesting depth and is matched by shape. Level
    /// numbering starts at 1.
    #[test]
    fn rejects_java_member_colliding_with_a_nested_array_loop_local() {
        for member in ["items1", "index1", "element2", "path3", "element10"] {
            let input = format!(
                r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
additionalProperties: false
properties:
  {member}: {{ type: string }}
"#
            );
            let error = reject_for(Language::Java, &input);
            assert!(
                error.contains("loop variable") && error.contains(member),
                "{member}: {error}"
            );
            let renamed = input.replace(
                &format!("  {member}: {{ type: string }}"),
                &format!("  {member}: {{ type: string, x-java-name: renamedMember }}"),
            );
            parse_for(Language::Java, &renamed)
                .unwrap_or_else(|error| panic!("{member}: override should apply: {error}"));
        }
        // Level 0 is never emitted (the first nested level is 1), so these are
        // ordinary member names.
        for member in ["items0", "index0", "element0", "path0"] {
            let input = format!(
                r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
additionalProperties: false
properties:
  {member}: {{ type: string }}
"#
            );
            parse_for(Language::Java, &input)
                .unwrap_or_else(|error| panic!("{member} should load: {error}"));
        }
    }

    /// A `const`/`enum` member's parse block binds `<member>Value`. It is a name
    /// synthesized *from the member*, so it belongs in the member scope — the
    /// pair used to compile or not depending on which of the two was authored
    /// first.
    #[test]
    fn rejects_java_closed_value_decoded_local_collision() {
        let closed = "  h: { type: string, enum: [p, q] }";
        let sibling = "  hValue: { type: string }";
        for (first, second) in [(closed, sibling), (sibling, closed)] {
            let input = format!(
                r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
additionalProperties: false
properties:
{first}
{second}
"#
            );
            let error = reject_for(Language::Java, &input);
            assert!(
                error.contains("decoded-value local") && error.contains("hValue"),
                "{error}"
            );
        }
    }

    /// The catch-all parse closes before the first member slot is declared, so
    /// its locals are out of scope by the time the slots exist. Pinned so the
    /// reserved list stays exactly as wide as `javac` requires.
    #[test]
    fn accepts_java_members_named_after_out_of_scope_generated_locals() {
        for member in [
            "key",
            "item",
            "itemPath",
            "value",
            "entry",
            "seen",
            "matchCount",
            "that",
        ] {
            let input = format!(
                r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
additionalProperties: {{ type: array, items: {{ type: string, minLength: 3 }} }}
properties:
  {member}: {{ type: string }}
  tags: {{ type: array, items: {{ type: string }}, uniqueItems: true }}
"#
            );
            parse_for(Language::Java, &input)
                .unwrap_or_else(|error| panic!("{member} should load: {error}"));
        }
    }

    /// Two `x-java-enum-names` entries naming one constant emit the constant
    /// twice inside the value class.
    #[test]
    fn rejects_duplicate_java_enum_name_overrides() {
        let input = r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  status:
    type: string
    enum: [active, retired]
    x-java-enum-names: { active: SAME, retired: SAME }
"#;
        let error = reject_for(Language::Java, input);
        assert!(
            error.contains("collision") && error.contains("SAME"),
            "{error}"
        );
    }

    /// The shared pre-language token fold in `validate_const_enum` steps aside
    /// whenever *any* constant-synthesizing target carries an override, and
    /// defers to the per-language pass. With only a Go override present, Java's
    /// own `UPPER_SNAKE` fold went unchecked.
    #[test]
    fn rejects_java_folded_enum_tokens_behind_a_go_only_override() {
        let input = r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  role:
    type: string
    enum: [user, USER]
    x-go-enum-names: { user: RoleUserLower, USER: RoleUserUpper }
"#;
        // Go names both constants verbatim, so Go loads.
        parse_for(Language::Go, input).expect("the Go overrides separate both constants");
        let error = reject_for(Language::Java, input);
        // The constant is named from the value alone, so both fold to `USER`,
        // and the fix-it names the override that can actually separate them.
        assert!(
            error.contains("collision")
                && error.contains("`USER`")
                && error.contains("x-java-enum-names"),
            "{error}"
        );
    }

    /// D9 keeps Java's `get<Field>OrDefault()`, so its name has to participate in
    /// the member scope — Go already rejects this pair.
    #[test]
    fn rejects_java_or_default_accessor_collision() {
        let input = r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  a: { type: string, default: x }
  aOrDefault: { type: string }
"#;
        for language in [Language::Go, Language::Java] {
            let error = reject_for(language, input);
            assert!(
                error.contains("collision") && error.contains("OrDefault"),
                "{language:?}: {error}"
            );
        }
    }

    /// The `x-<lang>-enum-names` key derivation is spelled three times — here,
    /// and once in each of the Go and Java emitters, which look the same map up
    /// while rendering. Pin every scalar kind so a change to one copy has to be
    /// a deliberate change to the contract.
    #[test]
    fn enum_names_lookup_key_is_the_canonical_json_spelling() {
        use serde_json::json;
        for (value, expected) in [
            (json!("active"), Some("active")),
            (json!(""), Some("")),
            (json!(1), Some("1")),
            (json!(-2), Some("-2")),
            (json!(1.0), Some("1")),
            (json!(1.5), Some("1.5")),
            (json!(true), Some("true")),
            (json!(false), Some("false")),
            (json!(null), None),
            (json!([1]), None),
            (json!({"a": 1}), None),
        ] {
            assert_eq!(
                enum_names_lookup_key(&value).as_deref(),
                expected,
                "for {value}"
            );
        }
    }

    /// All three `x-<lang>-enum-names` lookups matched only `Value::String`, so
    /// the one escape hatch for a numeric token collision did not exist.
    #[test]
    fn enum_names_override_applies_to_numeric_and_boolean_members() {
        let input = r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  scale:
    type: number
    enum: [1.5, 1]
    x-go-enum-names: { "1.5": ScaleOneHalf, "1": ScaleOne }
"#;
        let spec = parse_for(Language::Go, input).expect("numeric members are renameable");
        assert!(spec.external_type_binding("Api").is_some());
        // The override is what admits the pair: an identifier-valued key that the
        // derivation would otherwise fold is now nameable.
        let schema = spec
            .external_type_binding("Api")
            .unwrap()
            .json_model()
            .expect("not a JSON model")
            .schema
            .clone();
        assert_eq!(
            schema["properties"]["scale"]["x-go-enum-names"]["1.5"],
            "ScaleOneHalf"
        );
    }

    #[test]
    fn rejects_colliding_union_functions_python() {
        // A union's conversion lives in `_<base>_{from,to}_transfer_type` free
        // functions: `to_snake_case` on the named union `FooBar` and the
        // `<model>_<member>` base of `Foo.bar`'s inline union both give `foo_bar`.
        let input = r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  u: { $ref: "#/$defs/FooBar" }
  f: { $ref: "#/$defs/Foo" }
$defs:
  FooBar:
    oneOf:
      - { type: string }
      - { type: integer }
  Foo:
    type: object
    additionalProperties: false
    properties:
      bar:
        oneOf:
          - { type: string }
          - { type: boolean }
"##;
        let error = reject_for(Language::Python, input);
        assert!(
            error.contains("collision") && error.contains("_foo_bar_from_transfer_type"),
            "{error}"
        );
        // P15's escape hatch has to reach the synthesized function name too: the
        // member override renames the inline union's functions with the member.
        let renamed = input.replace(
            "      bar:\n        oneOf:",
            "      bar:\n        x-py-name: renamed\n        oneOf:",
        );
        parse_for(Language::Python, &renamed)
            .expect("an `x-py-name` override moves the inline union's function names");
    }

    #[test]
    fn rejects_type_colliding_with_pattern_constant_python() {
        // A `pattern` is hoisted to a module-level compiled-regex constant named
        // `_PATTERN_<FNV-1a of the pattern text>`; `^a` hashes to this one. A
        // verbatim type override can name a type that identifier.
        let input = r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  a: { type: string, pattern: "^a" }
  b: { $ref: "#/$defs/Other" }
$defs:
  Other:
    x-py-name: _PATTERN_09572B07B5E46120
    type: object
    properties:
      b: { type: string }
"##;
        assert_eq!(
            python::py_pattern_const_name("^a"),
            "_PATTERN_09572B07B5E46120"
        );
        let error = reject_for(Language::Python, input);
        assert!(
            error.contains("collision") && error.contains("_PATTERN_09572B07B5E46120"),
            "{error}"
        );
    }

    #[test]
    fn rejects_type_named_after_a_converter_body_local_python() {
        // The mirror image of a member shadowing a runtime local: a converter body
        // reads the module's classes by bare name while binding `raw`, so a *type*
        // overridden to `raw` is shadowed inside every body that parses one.
        let input = r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  a: { $ref: "#/$defs/Other" }
$defs:
  Other:
    x-py-name: raw
    type: object
    properties:
      b: { type: string }
"##;
        let error = reject_for(Language::Python, input);
        assert!(
            error.contains("collision") && error.contains("`raw`"),
            "{error}"
        );
        // Only Python binds that local, and only the Python override renames the
        // type, so the other targets are unaffected.
        for language in [Language::Go, Language::TypeScript, Language::Java] {
            parse_for(language, input)
                .unwrap_or_else(|error| panic!("{language:?} sees no override: {error}"));
        }
    }

    #[test]
    fn accepts_repeated_pattern_across_positions_python() {
        // One compiled constant per *distinct* pattern text is deliberate
        // deduplication, not a collision: the same pattern in several positions
        // (and the same `format`'s pinned regex twice) shares one constant.
        parse_for(
            Language::Python,
            r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  a: { type: string, pattern: "^a" }
  b: { type: string, pattern: "^a" }
  c:
    type: array
    items: { type: string, pattern: "^a" }
  d: { type: string, format: "email" }
  e: { type: string, format: "email" }
"##,
        )
        .expect("identical patterns share one module constant");
    }

    #[test]
    fn rejects_synthesized_operation_input_colliding_with_defs_type() {
        // The synthesized `<Op>Input` type collides with a declared `$defs` type
        // of the same name (top-level module scope, every target).
        let error = reject_for(
            Language::Python,
            r#"
nexusrpc: "1.0.0"
services:
  Chat:
    operations:
      create:
        input:
          type: object
          properties: { a: { type: string } }
$defs:
  CreateInput:
    type: object
    properties: { b: { type: string } }
"#,
        );
        assert!(
            error.contains("collision") && error.contains("CreateInput"),
            "{error}"
        );
    }

    #[test]
    fn rejects_service_colliding_with_model() {
        // A service binding name collides with a declared model type.
        let error = reject_for(
            Language::Python,
            r#"
nexusrpc: "1.0.0"
services:
  Widget:
    operations:
      ping:
        input:
          type: object
          properties: { a: { type: string } }
$defs:
  Widget:
    type: object
    properties: { b: { type: string } }
"#,
        );
        assert!(
            error.contains("collision") && error.contains("Widget"),
            "{error}"
        );
    }

    // --- `required` load-time validation (specs/json-schema/features/required.md) ---

    #[test]
    fn rejects_required_not_array() {
        for value in ["id", "null"] {
            let error = numeric_reject(&format!(
                "type: object\nproperties:\n  a: {{ type: string }}\nrequired: {value}"
            ));
            assert!(error.contains("must be an array"), "{value}: {error}");
        }
    }

    #[test]
    fn rejects_required_non_string_element() {
        let error =
            numeric_reject("type: object\nproperties:\n  id: { type: string }\nrequired: [1]");
        assert!(error.contains("only property-name strings"), "{error}");
    }

    #[test]
    fn rejects_required_duplicate() {
        let error =
            numeric_reject("type: object\nproperties:\n  id: { type: string }\nrequired: [id, id]");
        assert!(error.contains("more than once"), "{error}");
    }

    #[test]
    fn rejects_required_name_not_in_properties() {
        let error =
            numeric_reject("type: object\nproperties:\n  id: { type: string }\nrequired: [name]");
        assert!(error.contains("not declared in `properties`"), "{error}");
    }

    // --- `type` presence / shape (validate_type_presence) ---

    #[test]
    fn rejects_missing_type_on_leaf() {
        let error = numeric_reject("description: hi");
        assert!(
            error.contains("a leaf schema requires an explicit `type`"),
            "{error}"
        );
    }

    #[test]
    fn rejects_unknown_type_name() {
        let error = numeric_reject("type: foobar");
        assert!(error.contains("unknown `type`"), "{error}");
    }

    #[test]
    fn rejects_object_without_shape() {
        let error = numeric_reject("type: object");
        assert!(error.contains("needs an explicit shape"), "{error}");
    }

    #[test]
    fn rejects_array_without_items() {
        let error = numeric_reject("type: array");
        assert!(error.contains("needs an explicit element type"), "{error}");
    }

    // --- coverage: loader-time rejects found reachable-but-untested ---

    #[test]
    fn rejects_contains_scalar_matcher_over_composite_element() {
        // The element type is a valid (empty) object, so the array itself loads;
        // it is the `contains` matcher over a composite element that is deferred.
        let error = numeric_reject(
            "type: array\nitems: { type: object, properties: {} }\ncontains: { const: x }",
        );
        assert!(
            error.contains("`contains` over a composite element type"),
            "{error}"
        );
    }

    #[test]
    fn rejects_unknown_branch_type() {
        let error = union_reject(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  value:
    oneOf:
      - { type: object, additionalProperties: true }
      - { type: qux }
"#,
        );
        assert!(error.contains("unrecognized `type: qux`"), "{error}");
    }

    #[test]
    fn rejects_non_array_enum() {
        let error = numeric_reject("type: string\nenum: 5");
        assert!(error.contains("`enum` must be an array"), "{error}");
    }

    #[test]
    fn rejects_all_of_differing_contains() {
        let error = numeric_reject(
            "allOf:\n  - { type: array, contains: { const: 1 } }\n  - { type: array, contains: { const: 2 } }",
        );
        assert!(error.contains("different `contains` matchers"), "{error}");
    }

    #[test]
    fn rejects_all_of_entry_not_a_schema() {
        let error = numeric_reject(
            "allOf:\n  - { type: object, properties: { a: { type: string } } }\n  - 5",
        );
        assert!(error.contains("must be a schema object"), "{error}");
    }

    #[test]
    fn rejects_all_of_merges_to_empty() {
        let error = numeric_reject("allOf: [true, true]");
        assert!(error.contains("empty schema"), "{error}");
    }

    #[test]
    fn rejects_exclusive_empty_integer_interval() {
        let error = numeric_reject("type: integer\nexclusiveMinimum: 1\nexclusiveMaximum: 2");
        assert!(error.contains("empty range"), "{error}");
    }

    #[test]
    fn rejects_exclusive_boundary_empty_interval() {
        let error = numeric_reject("type: number\nminimum: 5\nexclusiveMaximum: 5");
        assert!(error.contains("empty range"), "{error}");
    }

    #[test]
    fn rejects_shapeless_array_element() {
        let error = numeric_reject("type: array\nitems: {}");
        assert!(
            error.contains("a leaf schema requires an explicit `type`"),
            "{error}"
        );
    }

    #[test]
    fn rejects_out_of_subset_array_element() {
        let error = numeric_reject("type: array\nitems: { type: object }");
        assert!(error.contains("needs an explicit shape"), "{error}");
    }

    #[test]
    fn rejects_tuple_items() {
        let error = numeric_reject("type: array\nitems: [ { type: string } ]");
        assert!(
            error.contains("properties.value.items")
                && error.contains("tuple-valued `items`")
                && error.contains("uniform element type"),
            "{error}"
        );
    }

    #[test]
    fn rejects_non_schema_items() {
        let error = numeric_reject("type: array\nitems: 5");
        assert!(
            error.contains("`items` must be a schema object")
                && error.contains("uniform element type"),
            "{error}"
        );
    }

    #[test]
    fn rejects_boolean_items_with_uniform_element_fix() {
        let error = numeric_reject("type: array\nitems: true");
        assert!(
            error.contains("properties.value.items")
                && error.contains("boolean schema `true` is not supported for `items`")
                && error.contains("uniform element type"),
            "{error}"
        );
    }

    #[test]
    fn rejects_non_string_title() {
        for value in ["42", "null"] {
            let error = numeric_reject(&format!("type: string\ntitle: {value}"));
            assert!(error.contains("must be a string"), "{value}: {error}");
        }
    }

    #[test]
    fn rejects_non_string_description() {
        for value in ["42", "null"] {
            let error = numeric_reject(&format!("type: string\ndescription: {value}"));
            assert!(error.contains("must be a string"), "{value}: {error}");
        }
    }

    #[test]
    fn rejects_non_object_properties() {
        let error = numeric_reject("type: object\nproperties: []");
        assert!(error.contains("failed to parse JSON schema"), "{error}");
    }

    #[test]
    fn rejects_non_string_x_lang_name_override() {
        let error = reject_for(
            Language::Go,
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  code: { type: string, x-go-name: 42 }
"#,
        );
        assert!(
            error.contains("`x-go-name` must be a string identifier"),
            "{error}"
        );
    }

    #[test]
    fn scopes_reserved_module_names_to_their_generated_files() {
        for path in ["models.yaml", "services/x.yaml", "a/definitions.yaml"] {
            let sources = vec![
                module_collision_source(path, "Allowed"),
                module_collision_source("other.yaml", "Other"),
            ];
            api_spec_tree_from_json_schema_sources(Language::Python, sources)
                .unwrap_or_else(|error| panic!("`{path}` has no file collision: {error}"));
        }

        for path in ["index/x.yaml", "a/__init__/x.yaml"] {
            let sources = vec![
                module_collision_source(path, "Reserved"),
                module_collision_source("other.yaml", "Other"),
            ];
            let error = api_spec_tree_from_json_schema_sources(Language::Python, sources)
                .expect_err("aggregator names are reserved at every depth")
                .to_string();
            assert!(error.contains("reserved module name"), "{path}: {error}");
        }
    }

    #[test]
    fn nexusrpc_infix_is_stripped_from_module_and_root_type_names() {
        let tree = api_spec_tree_from_json_schema_sources(
            Language::Go,
            vec![
                module_collision_source("room.nexusrpc.yaml", "IgnoredTitle"),
                module_collision_source("other.yaml", "Other"),
            ],
        )
        .expect("a pure schema may use the .nexusrpc naming infix");
        let ApiSpecNode::Branch(root) = tree.root else {
            panic!("two inputs produce a branch");
        };
        let ApiSpecNode::Leaf(room) = &root.children["room"] else {
            panic!("room should be a leaf module");
        };
        assert_eq!(room.module_path.as_module_key(), "room");
        let model = room
            .spec
            .external_types()
            .find_map(|(_, binding)| binding.json_model())
            .expect("the root model uses the stripped file name");
        assert_eq!(model.model_name, "Room");
    }

    #[test]
    fn rejects_shared_runtime_module_names() {
        // Both spellings of the shared runtime module are reserved for every
        // target: `definitions` (Go/TypeScript) and `_definitions` (Python). A
        // `_definitions` input emits a package directory at the Python runtime
        // module's own import path, which shadows it and breaks every
        // `from .._definitions import ...` in the tree.
        for segment in ["definitions", "_definitions", "_recursive"] {
            for language in [
                Language::Python,
                Language::TypeScript,
                Language::Go,
                Language::Java,
            ] {
                let sources = vec![
                    module_collision_source(&format!("{segment}.yaml"), "Shadow"),
                    module_collision_source("other.yaml", "Other"),
                ];
                let error = api_spec_tree_from_json_schema_sources(language, sources)
                    .err()
                    .unwrap_or_else(|| panic!("`{segment}` must be rejected for {language:?}"))
                    .to_string();
                assert!(
                    error.contains("reserved module name") && error.contains(segment),
                    "{language:?}: {error}"
                );
            }
        }
    }

    #[test]
    fn rejects_object_keyword_on_scalar() {
        let error = numeric_reject("type: string\nproperties:\n  a: { type: string }");
        assert!(error.contains("require `type: object`"), "{error}");
    }

    #[test]
    fn rejects_items_on_scalar() {
        let error = numeric_reject("type: string\nitems: { type: string }");
        assert!(error.contains("`items` requires `type: array`"), "{error}");
    }

    #[test]
    fn accepts_empty_properties_object() {
        numeric_accept("type: object\nproperties: {}");
    }

    // --- `additionalProperties` value shape (validate_schema_node) ---

    #[test]
    fn rejects_non_schema_additional_properties() {
        for value in ["\"yes\"", "null"] {
            let error = numeric_reject(&format!("type: object\nadditionalProperties: {value}"));
            assert!(
                error.contains("must be `true`, `false`, or a schema object"),
                "{value}: {error}"
            );
        }
    }

    #[test]
    fn rejects_empty_object_additional_properties() {
        let error = numeric_reject("type: object\nadditionalProperties: {}");
        assert!(
            error.contains("write `additionalProperties: true` instead"),
            "{error}"
        );
    }

    // --- `enum` vs numeric bound (numeric literal loop) ---

    #[test]
    fn rejects_enum_violating_numeric_bound() {
        let error = numeric_reject("type: integer\nmaximum: 5\nenum: [1, 7]");
        assert!(
            error.contains("`enum` value 7 violates the numeric bounds"),
            "{error}"
        );
    }

    // --- rendered documentation annotations (validate_annotations) ---

    #[test]
    fn rejects_empty_description() {
        let error = numeric_reject("type: string\ndescription: \"\"");
        assert!(error.contains("`description` must not be empty"), "{error}");
    }

    #[test]
    fn rejects_whitespace_description() {
        let error = numeric_reject("type: string\ndescription: \"   \"");
        assert!(error.contains("`description` must not be empty"), "{error}");
    }

    #[test]
    fn rejects_control_characters_in_documentation() {
        let title = doc_reject(
            "$schema: https://json-schema.org/draft/2020-12/schema\ntype: string\ntitle: \"bad \\0 title\"",
        );
        assert!(
            title.contains("`title`") && title.contains("control character U+0000"),
            "{title}"
        );

        let schema = doc_reject(
            "$schema: https://json-schema.org/draft/2020-12/schema\ntype: string\ndescription: \"bad \\0 prose\"",
        );
        assert!(schema.contains("control character U+0000"), "{schema}");

        let service = doc_reject(
            "nexusrpc: \"1.0.0\"\nservices:\n  Chat:\n    description: \"bad \\x01 prose\"\n    operations:\n      send:\n        input: { type: object, properties: {} }",
        );
        assert!(service.contains("control character U+0001"), "{service}");

        let operation = doc_reject(
            "nexusrpc: \"1.0.0\"\nservices:\n  Chat:\n    operations:\n      send:\n        description: \"bad \\x02 prose\"\n        input: { type: object, properties: {} }",
        );
        assert!(
            operation.contains("control character U+0002"),
            "{operation}"
        );
    }

    // --- operation I/O must resolve to an object (require_object_io) ---

    #[test]
    fn rejects_ref_union_operation_io() {
        let error = doc_reject(
            r##"
nexusrpc: "1.0.0"
services:
  ChatService:
    operations:
      pick:
        input: { $ref: "#/$defs/Thing" }
$defs:
  A:
    type: object
    properties: { kind: { type: string, const: a } }
    required: [kind]
  B:
    type: object
    properties: { kind: { type: string, const: b } }
    required: [kind]
  Thing:
    oneOf:
      - { $ref: "#/$defs/A" }
      - { $ref: "#/$defs/B" }
"##,
        );
        assert!(error.contains("must resolve to an object"), "{error}");
    }

    #[test]
    fn rejects_inline_union_operation_io() {
        let error = doc_reject(
            r##"
nexusrpc: "1.0.0"
services:
  ChatService:
    operations:
      pick:
        input:
          oneOf:
            - { type: string }
            - { type: integer }
"##,
        );
        assert!(error.contains("must resolve to an object"), "{error}");
    }

    // --- service / operation names (name_matches) ---

    #[test]
    fn rejects_invalid_service_name() {
        let error = doc_reject(
            r##"
nexusrpc: "1.0.0"
services:
  chatService:
    operations:
      ping: {}
"##,
        );
        assert!(error.contains("must match `^[A-Z]"), "{error}");
    }

    #[test]
    fn rejects_invalid_operation_name() {
        let error = doc_reject(
            r##"
nexusrpc: "1.0.0"
services:
  ChatService:
    operations:
      PollMessages: {}
"##,
        );
        assert!(error.contains("must match `^[a-z]"), "{error}");
    }

    // --- reserved / invalid member identifiers (validate_member_scope) ---

    #[test]
    fn rejects_reserved_member_without_override() {
        let error = doc_reject(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  class: { type: string }
"#,
        );
        assert!(error.contains("is a reserved word"), "{error}");
    }

    #[test]
    fn rejects_invalid_member_identifier() {
        let error = doc_reject(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  "2fa": { type: string }
"#,
        );
        assert!(error.contains("is not a valid identifier"), "{error}");
    }

    #[test]
    fn empty_member_identifier_diagnostic_names_the_empty_wire_key() {
        let error = reject_for(
            Language::Go,
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  "": { type: string }
"#,
        );
        assert!(
            error.contains("the empty JSON member name in model `Api`")
                && error.contains("add an `x-go-name` override")
                && !error.contains("member `Api.` recases to ``"),
            "{error}"
        );
    }

    // --- `not` per-form diagnostics (validate_schema_common) ---

    #[test]
    fn rejects_not_empty_unsatisfiable() {
        let error = numeric_reject("not: {}");
        assert!(error.contains("unsatisfiable"), "{error}");
    }

    #[test]
    fn rejects_not_true_unsatisfiable() {
        let error = numeric_reject("not: true");
        assert!(error.contains("unsatisfiable"), "{error}");
    }

    #[test]
    fn rejects_not_false_noop() {
        let error = numeric_reject("not: false");
        assert!(error.contains("no-op"), "{error}");
    }

    #[test]
    fn rejects_not_double_negation() {
        let error = numeric_reject("not: { not: { type: string } }");
        assert!(error.contains("not supported"), "{error}");
    }

    #[test]
    fn validates_not_subschema_before_rejecting_not() {
        let unknown = numeric_reject("not: { type: string, minLenght: 2 }");
        assert!(
            unknown.contains("unknown schema keyword `minLenght`") && unknown.contains(".not"),
            "{unknown}"
        );

        let malformed = numeric_reject("not: 5");
        assert!(
            malformed.contains("`not` must be a boolean or schema object"),
            "{malformed}"
        );

        let invalid_default = numeric_reject("not: { type: string, default: 5 }");
        assert!(
            invalid_default.contains("incompatible"),
            "{invalid_default}"
        );
    }

    // --- unsatisfiable recursion cycles (validate_reference_satisfiability) ---

    // --- Wave 8: loader accepts that should reject ---

    /// A `oneOf` branch's *kind* is the sum-type pass's to check, but its shape
    /// was checked nowhere: an itemless `{type: array}` branch loaded, and Java
    /// inferred `List<String>` where the other three inferred `any`/`unknown`/
    /// `Any` — `[1, 2]` accepted by three targets and rejected by the fourth.
    #[test]
    fn rejects_shapeless_union_branch() {
        let error = numeric_reject("oneOf:\n  - { type: string }\n  - { type: array }");
        assert!(error.contains("needs an explicit element type"), "{error}");
        let error = numeric_reject("oneOf:\n  - { type: string }\n  - { type: object }");
        assert!(error.contains("needs an explicit shape"), "{error}");
    }

    /// Decision D5: a `default` on a sum type names no branch, and Go emits
    /// `return *m.F` against a sealed interface.
    #[test]
    fn rejects_default_on_a_sum_type_union() {
        let error =
            numeric_reject("oneOf:\n  - { type: string }\n  - { type: integer }\ndefault: hi");
        assert!(error.contains("has no defined meaning"), "{error}");
        // The nullability wrapper keeps its defined lowering.
        parse(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  value:
    oneOf:
      - { type: string }
      - { type: "null" }
    default: hi
"#,
        );
    }

    /// The `propertyNames` allowlist walked only `extra`, so every *typed* field
    /// passed silently — and the `$ref` case surfaced as an `allOf` diagnostic
    /// about a keyword the user never wrote.
    #[test]
    fn rejects_property_names_with_a_structural_keyword() {
        for (keyword, subschema) in [
            ("$ref", "{ $ref: \"#/$defs/Key\", type: string }"),
            (
                "properties",
                "{ type: string, properties: { a: { type: string } } }",
            ),
            ("required", "{ type: string, required: [a] }"),
            ("items", "{ type: string, items: { type: string } }"),
            (
                "oneOf",
                "{ type: string, oneOf: [ { type: string }, { type: \"null\" } ] }",
            ),
            (
                "additionalProperties",
                "{ type: string, additionalProperties: true }",
            ),
        ] {
            let input = format!(
                r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
additionalProperties: {{ type: string }}
propertyNames: {subschema}
$defs:
  Key:
    type: object
    properties:
      a: {{ type: string }}
"##
            );
            let error = parse_api_spec_from_json_schema_for_language(
                Language::Python,
                &input,
                PathBuf::from("api.yaml"),
            )
            .unwrap_err()
            .to_string();
            assert!(
                error.contains(&format!(
                    "`propertyNames` with `{keyword}` is not supported"
                )),
                "{keyword}: {error}"
            );
        }
    }

    /// Decision D6: a materializing `format` cannot assert a key — a property
    /// name is always a plain map key.
    #[test]
    fn rejects_temporal_format_in_property_names() {
        let error = numeric_reject(
            "type: object\nadditionalProperties: { type: string }\npropertyNames: { type: string, format: date-time }",
        );
        assert!(error.contains("materializes a native date/time"), "{error}");
        // A string-shaped format still asserts on a key.
        parse(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  keyed:
    type: object
    additionalProperties: { type: string }
    propertyNames: { type: string, format: uuid }
"#,
        );
    }

    /// Decision D6, matcher half: a matcher is a predicate, not a slot, so there
    /// is nothing to materialize into.
    #[test]
    fn rejects_temporal_format_in_a_contains_matcher() {
        let error = numeric_reject(
            "type: array\nitems: { type: string }\ncontains: { type: string, format: duration }",
        );
        assert!(error.contains("materializes a native date/time"), "{error}");
    }

    /// `multipleOf` restricts a `number` to the same discrete lattice it
    /// restricts an `integer` to, so the emptiness check cannot be gated on
    /// `type: integer`.
    #[test]
    fn rejects_unsatisfiable_number_range_with_multiple_of() {
        let error = numeric_reject("type: number\nminimum: 1\nmaximum: 2\nmultipleOf: 5");
        assert!(error.contains("no multiple of 5"), "{error}");
        // A multiple inside the range still loads.
        parse(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  value: { type: number, minimum: 1, maximum: 6, multipleOf: 5 }
"#,
        );
    }

    /// `(value / divisor).fract()` is always `0.0` for a quotient at or above
    /// 2^52, so every large literal read as divisible — a third divisibility
    /// semantics that no runtime shares.
    #[test]
    fn rejects_large_literal_violating_multiple_of() {
        let error = numeric_reject("type: number\nmultipleOf: 3\nconst: 1e22");
        assert!(error.contains("must be a multiple of 3"), "{error}");
    }

    /// `title`/`description` are `Option<String>`, and a YAML plain scalar
    /// reaches `deserialize_string` as its raw text — so `title: 42` was
    /// silently coerced to `"42"` everywhere except the document root.
    #[test]
    fn rejects_non_string_annotations() {
        let nested = doc_reject(
            r#"
$defs:
  R:
    type: object
    title: 42
    properties:
      a: { type: string }
"#,
        );
        assert!(nested.contains("must be a string"), "{nested}");
        let member = doc_reject(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  a: { type: string, description: true }
"#,
        );
        assert!(member.contains("must be a string"), "{member}");
        let service = doc_reject(
            r#"
nexusrpc: 1.0.0
services:
  Chat:
    description: 7
    operations:
      send:
        input: { type: object, properties: { a: { type: string } } }
"#,
        );
        assert!(service.contains("must be a string"), "{service}");
        let operation = doc_reject(
            r#"
nexusrpc: 1.0.0
services:
  Chat:
    operations:
      send:
        description: 7
        input: { type: object, properties: { a: { type: string } } }
"#,
        );
        assert!(operation.contains("must be a string"), "{operation}");
    }

    /// `serde_json::Number`'s `PartialEq` compares the representation, so
    /// `enum: [1, 1.0]` and `[0, -0.0]` loaded and Go emitted two `switch` cases
    /// for one value.
    #[test]
    fn rejects_enum_members_that_differ_only_in_numeric_spelling() {
        for members in ["[1, 1.0]", "[0, -0.0]", "[2, 2e0]"] {
            let error = numeric_reject(&format!("type: number\nenum: {members}"));
            assert!(error.contains("more than once"), "{members}: {error}");
        }
        // Two exactly-represented JSON integers stay distinct for `number`.
        parse(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  value: { type: number, enum: [9007199254740993, 9007199254740992] }
"#,
        );
    }

    /// A malformed `type` is its own defect, not an absent one — and on a node
    /// that also carries `oneOf` it was never reported at all.
    #[test]
    fn rejects_malformed_type_value() {
        for literal in ["5", "null", "true"] {
            let error = numeric_reject(&format!("type: {literal}"));
            assert!(
                error.contains("`type` must be a string naming one of"),
                "{literal}: {error}"
            );
        }
        let error = numeric_reject("type: 5\noneOf:\n  - { type: string }\n  - { type: integer }");
        assert!(
            error.contains("`type` must be a string naming one of"),
            "{error}"
        );
    }

    /// `finalize_merged` collapses a same-axis pair on purpose — that is the
    /// accepted "tighten across inclusive/exclusive" row — but the collapse also
    /// swallowed the identical typo written inside one branch.
    #[test]
    fn rejects_redundant_same_axis_bounds_inside_an_all_of_branch() {
        let error = numeric_reject(
            "allOf:\n  - { type: integer, minimum: 3, exclusiveMinimum: 4 }\n  - { type: integer, maximum: 100 }",
        );
        assert!(error.contains("exactly one of `minimum`"), "{error}");
        // Across branches the pair is still the accepted tightening row.
        parse(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  value:
    allOf:
      - { type: integer, minimum: 3 }
      - { type: integer, exclusiveMinimum: 4 }
"#,
        );
    }

    /// Decision D2: a nullable scalar element is not "composite". A `null`
    /// element never matches a scalar matcher, and two `null`s are a duplicate.
    #[test]
    fn accepts_unique_items_and_contains_over_a_nullable_scalar_element() {
        let schema = model_schema(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  tags:
    type: array
    uniqueItems: true
    items:
      oneOf:
        - { type: string }
        - { type: "null" }
  scores:
    type: array
    contains: { minimum: 3 }
    items:
      oneOf:
        - { type: integer }
        - { type: "null" }
"#,
            "Api",
        );
        assert_eq!(schema["properties"]["tags"]["uniqueItems"], true);
        assert_eq!(schema["properties"]["scores"]["contains"]["minimum"], 3);
        // A nullable *object* element is still composite.
        let error = doc_reject(
            r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  boxes:
    type: array
    uniqueItems: true
    items:
      oneOf:
        - { $ref: "#/$defs/Box" }
        - { type: "null" }
$defs:
  Box:
    type: object
    properties:
      a: { type: string }
"##,
        );
        assert!(error.contains("composite element type"), "{error}");
    }

    /// Decision D10: the emitters compare the canonical wire string, so the
    /// loader has to give them one. The literal used to reach them exactly as
    /// authored.
    #[test]
    fn canonicalizes_materialized_temporal_literals() {
        let schema = model_schema(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  span: { type: string, format: duration, const: PT90M }
  at: { type: string, format: date-time, default: "2021-06-15t12:30:45z" }
  clock:
    type: string
    format: time
    enum: ["12:30:45.000-00:00", "01:02:03Z"]
"#,
            "Api",
        );
        assert_eq!(schema["properties"]["span"]["const"], "PT1H30M");
        assert_eq!(
            schema["properties"]["at"]["default"],
            "2021-06-15T12:30:45Z"
        );
        assert_eq!(
            schema["properties"]["clock"]["enum"],
            serde_json::json!(["12:30:45Z", "01:02:03Z"])
        );
    }

    /// Canonicalization can make two authored spellings one value; the shared
    /// uniqueness check then owns the reject.
    #[test]
    fn rejects_enum_members_that_canonicalize_to_one_temporal_value() {
        let error = numeric_reject("type: string\nformat: duration\nenum: [PT90M, PT1H30M]");
        assert!(error.contains("more than once"), "{error}");
    }

    #[test]
    fn rejects_unsatisfiable_self_reference() {
        let error = doc_reject(
            r##"
$defs:
  Node:
    type: object
    properties:
      next: { $ref: "#/$defs/Node" }
    required: [next]
"##,
        );
        assert!(error.contains("unsatisfiable recursion cycle"), "{error}");
    }

    #[test]
    fn rejects_unsatisfiable_mutual_recursion() {
        let error = doc_reject(
            r##"
$defs:
  A:
    type: object
    properties:
      b: { $ref: "#/$defs/B" }
    required: [b]
  B:
    type: object
    properties:
      a: { $ref: "#/$defs/A" }
    required: [a]
"##,
        );
        assert!(
            error.contains("unsatisfiable recursion cycle `A → B → A`")
                && error.contains("making an edge optional")
                && error.contains("nullable")
                && error.contains("wrapping it in an array"),
            "{error}"
        );
    }

    /// Every `oneOf` edge used to terminate the mandatory chain, conflating the
    /// nullability wrapper with a sum type — so a recursion whose every branch
    /// reenters it loaded.
    #[test]
    fn rejects_unsatisfiable_sum_type_recursion() {
        let error = doc_reject(
            r##"
$defs:
  Node:
    type: object
    required: [next]
    properties:
      next:
        oneOf:
          - { $ref: "#/$defs/Left" }
          - { $ref: "#/$defs/Right" }
  Left:
    type: object
    required: [kind, node]
    properties:
      kind: { type: string, const: left }
      node: { $ref: "#/$defs/Node" }
  Right:
    type: object
    required: [kind, node]
    properties:
      kind: { type: string, const: right }
      node: { $ref: "#/$defs/Node" }
"##,
        );
        assert!(error.contains("unsatisfiable recursion cycle"), "{error}");
    }

    /// One terminating branch is enough: a union is instantiable as soon as any
    /// branch is.
    #[test]
    fn accepts_sum_type_recursion_with_a_terminating_branch() {
        parse(
            r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  root: { $ref: "#/$defs/Node" }
$defs:
  Node:
    type: object
    required: [next]
    properties:
      next:
        oneOf:
          - { $ref: "#/$defs/Leaf" }
          - { $ref: "#/$defs/Branch" }
  Leaf:
    type: object
    required: [kind]
    properties:
      kind: { type: string, const: leaf }
  Branch:
    type: object
    required: [kind, node]
    properties:
      kind: { type: string, const: branch }
      node: { $ref: "#/$defs/Node" }
"##,
        );
    }

    /// A nullable recursive edge terminates, as it always did.
    #[test]
    fn accepts_nullable_recursion() {
        parse(
            r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  root: { $ref: "#/$defs/Node" }
$defs:
  Node:
    type: object
    required: [next]
    properties:
      next:
        oneOf:
          - { $ref: "#/$defs/Node" }
          - { type: "null" }
"##,
        );
    }

    /// A named union def whose every branch reenters it has no instance either.
    #[test]
    fn rejects_unsatisfiable_named_union_recursion() {
        let error = doc_reject(
            r##"
$defs:
  Choice:
    oneOf:
      - { $ref: "#/$defs/Left" }
      - { $ref: "#/$defs/Right" }
  Left:
    type: object
    required: [kind, choice]
    properties:
      kind: { type: string, const: left }
      choice: { $ref: "#/$defs/Choice" }
  Right:
    type: object
    required: [kind, choice]
    properties:
      kind: { type: string, const: right }
      choice: { $ref: "#/$defs/Choice" }
"##,
        );
        assert!(error.contains("unsatisfiable recursion cycle"), "{error}");
    }

    #[test]
    fn accepts_array_wrapped_recursion() {
        parse(
            r##"
$defs:
  Tree:
    type: object
    properties:
      children:
        type: array
        items: { $ref: "#/$defs/Tree" }
    required: [children]
"##,
        );
    }

    #[test]
    fn accepts_typed_map_wrapped_recursion() {
        parse(
            r##"
$defs:
  Tree:
    type: object
    properties:
      children:
        type: object
        additionalProperties: { $ref: "#/$defs/Tree" }
    required: [children]
"##,
        );
    }

    #[test]
    fn accepts_optional_recursion() {
        parse(
            r##"
$defs:
  Node:
    type: object
    properties:
      next: { $ref: "#/$defs/Node" }
"##,
        );
    }

    // --- catch-all collision (validate_member_scope) ---

    #[test]
    fn rejects_member_colliding_with_catch_all() {
        let error = doc_reject(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  additionalProperties: { type: string }
"#,
        );
        assert!(error.contains("catch-all"), "{error}");
        assert!(error.contains("collision"), "{error}");
    }

    #[test]
    fn accepts_additional_properties_member_when_closed() {
        parse(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
additionalProperties: false
properties:
  additionalProperties: { type: string }
"#,
        );
    }

    // --- definitions-only file (validate_document) ---

    #[test]
    fn accepts_definitions_only_file() {
        let spec = parse(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
description: A definitions bucket.
$defs:
  Thing:
    type: object
    properties:
      id: { type: string }
"#,
        );
        assert!(spec.external_type_binding("Thing").is_some());
    }

    // --- cross-file `$ref` target must be in the input set (resolve_ref_key) ---

    #[test]
    fn discovers_transitive_local_ref_closure_and_recomputes_common_root() {
        let temp = tempfile::tempdir().unwrap();
        let entry_dir = temp.path().join("app");
        let shared_dir = temp.path().join("shared");
        let nested_dir = shared_dir.join("nested");
        fs::create_dir_all(&entry_dir).unwrap();
        fs::create_dir_all(&nested_dir).unwrap();
        let entry = entry_dir.join("entry.yaml");
        let middle = shared_dir.join("middle.yaml");
        let end = nested_dir.join("end.yaml");
        fs::write(
            &entry,
            r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  middle: { $ref: "../shared/middle.yaml#/$defs/Middle" }
  middleAlias: { $ref: "../shared/nested/.././middle.yaml#/$defs/Middle" }
"##,
        )
        .unwrap();
        fs::write(
            &middle,
            r##"
$schema: https://json-schema.org/draft/2020-12/schema
$defs:
  Middle:
    type: object
    properties:
      end: { $ref: "nested/end.yaml#/$defs/Outer/$defs/End" }
"##,
        )
        .unwrap();
        fs::write(
            &end,
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
$defs:
  Outer:
    type: object
    properties: {}
    $defs:
      End:
        type: object
        properties:
          value: { type: string }
"#,
        )
        .unwrap();

        let invocation_root = temp.path().to_path_buf();
        let sources = expand_json_schema_sources(std::slice::from_ref(&invocation_root)).unwrap();
        assert_eq!(
            sources
                .iter()
                .map(|source| source.relative_path.clone())
                .collect::<Vec<_>>(),
            vec![
                PathBuf::from("app/entry.yaml"),
                PathBuf::from("shared/middle.yaml"),
                PathBuf::from("shared/nested/end.yaml"),
            ]
        );
        let common_root = canonical(temp.path());
        assert!(
            sources
                .iter()
                .all(|source| source.source_root == common_root)
        );

        let flat = load_api_spec_from_json_schema_for_language_with_inputs(
            Language::Python,
            std::slice::from_ref(&invocation_root),
        )
        .expect("the flat public loader should load the complete ref closure");
        assert!(flat.external_type_binding("Middle").is_some());
        assert!(flat.external_type_binding("Outer.End").is_some());

        load_api_spec_tree_from_json_schema_for_language_with_inputs(
            Language::Python,
            &[invocation_root],
        )
        .expect("the public tree loader should load the complete ref closure");
    }

    #[test]
    fn missing_ref_target_file_reports_the_referring_schema_breadcrumb_and_remedy() {
        let temp = tempfile::tempdir().unwrap();
        let entry = temp.path().join("entry.yaml");
        fs::write(
            &entry,
            r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  child: { $ref: "missing.yaml#/$defs/Child" }
"##,
        )
        .unwrap();

        let error = load_api_spec_from_json_schema_for_language_with_inputs(
            Language::Python,
            std::slice::from_ref(&entry),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("root schema.properties.child"), "{error}");
        assert!(
            error.contains("`$ref` `missing.yaml#/$defs/Child`") && error.contains("missing.yaml"),
            "{error}"
        );
        assert!(
            error.contains("add the file") && error.contains("correct the relative `$ref`"),
            "{error}"
        );
    }

    #[test]
    fn rejects_ref_target_file_not_in_input_set() {
        let error = numeric_reject("$ref: \"missing.yaml#/$defs/X\"");
        assert!(error.contains("not in the input set"), "{error}");
    }

    #[test]
    fn resolves_nested_defs_pointer_tokens_with_rfc6901_unescaping() {
        let spec = parse(
            r##"
$schema: https://json-schema.org/draft/2020-12/schema
$defs:
  Outer:
    type: object
    properties:
      nested: { $ref: "#/$defs/Outer/$defs/inner~1value" }
    $defs:
      inner/value:
        allOf:
          - type: object
            properties:
              id: { type: string }
          - type: object
            properties:
              label: { type: string }
"##,
        );
        assert!(spec.external_type_binding("Outer").is_some());
        let nested = spec
            .external_type_binding("Outer.inner/value")
            .expect("nested definition should have its own model identity");
        let nested = nested
            .json_model()
            .expect("nested definition should remain a JSON model");
        assert!(nested.schema.get("allOf").is_none());
        assert_eq!(nested.schema["properties"]["id"]["type"], "string");
        assert_eq!(nested.schema["properties"]["label"]["type"], "string");
    }

    #[test]
    fn pointer_unescaping_happens_token_by_token() {
        let spec = parse(
            r##"
$schema: https://json-schema.org/draft/2020-12/schema
$defs:
  Outer/$defs/Inner:
    type: object
    properties: {}
  Holder:
    type: object
    properties:
      value: { $ref: "#/$defs/Outer~1$defs~1Inner" }
"##,
        );
        assert!(spec.external_type_binding("Outer/$defs/Inner").is_some());
    }

    #[test]
    fn rejects_slash_fragment_as_a_root_reference() {
        let error = numeric_reject("$ref: \"#/\"");
        assert!(error.contains("`#/`"), "{error}");
        assert!(error.contains("not the file root"), "{error}");
    }

    #[test]
    fn rejects_invalid_rfc6901_escape_in_ref_pointer() {
        let error = numeric_reject("$ref: \"#/$defs/bad~2name\"");
        assert!(
            error.contains("properties.value") && error.contains("invalid RFC 6901 escape"),
            "{error}"
        );
    }

    #[test]
    fn rejects_malformed_local_ref_pointer_structures() {
        for (reference, expected) in [
            ("#/$defs", "must point at a `$defs` entry"),
            ("#/$defs/bad~", "trailing `~`"),
            ("#anchor", "must use a JSON Pointer"),
        ] {
            let error = numeric_reject(&format!("$ref: {reference:?}"));
            assert!(
                error.contains("properties.value") && error.contains(expected),
                "{reference}: expected {expected}, got {error}"
            );
        }
    }

    #[test]
    fn validates_nested_defs_as_generated_models() {
        let error = doc_reject(
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
$defs:
  Outer:
    type: object
    properties: {}
    $defs:
      InvalidScalar: { type: string }
"#,
        );
        assert!(error.contains("InvalidScalar"), "{error}");
        assert!(error.contains("must be `type: object`"), "{error}");
    }

    #[test]
    fn rejects_defs_outside_a_model_declaration_position() {
        for schema in [
            "type: object\nproperties:\n  value:\n    type: string\n    $defs: { Hidden: { type: object, properties: {} } }",
            "type: array\nitems:\n  type: string\n  $defs: { Hidden: { type: object, properties: {} } }",
        ] {
            let error = numeric_reject(schema);
            assert!(
                error.contains("`$defs` is only allowed at a document root")
                    && error.contains("move this definition to the document's `$defs`"),
                "{schema}: {error}"
            );
        }
    }

    #[test]
    fn raw_all_of_reports_the_owning_rejected_keyword_before_merge() {
        for (keyword, expected) in [
            ("$id: urn:branch", "remove `$id`"),
            ("$vocabulary: {}", "meta-schema keyword"),
            ("dependentSchemas: {}", "conditional subschema"),
        ] {
            let error = numeric_reject(&format!(
                "allOf:\n  - {{ type: string, {keyword} }}\n  - {{ type: integer }}"
            ));
            assert!(error.contains(expected), "{keyword}: {error}");
            assert!(!error.contains("disjoint `type`s"), "{keyword}: {error}");
        }
    }

    #[test]
    fn ignores_ref_shaped_data_when_discovering_source_closure() {
        let temp = tempfile::tempdir().unwrap();
        let entry = temp.path().join("entry.yaml");
        fs::write(
            &entry,
            r#"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  value:
    type: string
    examples: [{ $ref: missing.yaml#/$defs/NotASchemaRef }]
"#,
        )
        .unwrap();
        let sources = expand_json_schema_sources(std::slice::from_ref(&entry)).unwrap();
        assert_eq!(sources.len(), 1);
    }

    #[test]
    fn rejects_absolute_and_out_of_invocation_ref_paths() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        fs::create_dir_all(&root).unwrap();
        let absolute = root.join("absolute.yaml");
        fs::write(
            &absolute,
            format!(
                "$schema: https://json-schema.org/draft/2020-12/schema\ntype: object\nproperties:\n  value: {{ $ref: {:?} }}\n",
                temp.path().join("target.yaml#/$defs/Thing").display().to_string()
            ),
        )
        .unwrap();
        let error = expand_json_schema_sources(std::slice::from_ref(&absolute))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("absolute-path `$ref`")
                && error.contains("use a path relative")
                && error.contains("outside the invocation root")
                && error.contains("widen the invocation")
                && error.contains("additional input"),
            "{error}"
        );

        let escape_root = temp.path().join("escape-root");
        fs::create_dir_all(&escape_root).unwrap();
        let upward = escape_root.join("upward.yaml");
        fs::write(
            &upward,
            "$schema: https://json-schema.org/draft/2020-12/schema\ntype: object\nproperties:\n  value: { $ref: ../target.yaml#/$defs/Thing }\n",
        )
        .unwrap();
        let error = expand_json_schema_sources(std::slice::from_ref(&escape_root))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("escapes the invocation root") && error.contains("widen the invocation"),
            "{error}"
        );
    }

    #[test]
    fn service_name_override_resolves_service_vs_model_collision() {
        // A service and a `$defs` model would both resolve to `Widget`. An
        // `x-go-name` on the service renames its emitted identifier verbatim,
        // clearing the collision for Go (and leaving the wire name untouched).
        let input = r#"
nexusrpc: "1.0.0"
services:
  Widget:
    x-go-name: WidgetService
    operations:
      ping:
        input:
          type: object
          properties: { a: { type: string } }
$defs:
  Widget:
    type: object
    properties: { b: { type: string } }
"#;
        // Go: the override resolves the collision.
        let spec = parse_for(Language::Go, input).expect("override should clear the Go collision");
        let service = &spec.services[0];
        assert_eq!(
            service.code_name.for_language(Language::Go),
            Some("WidgetService")
        );
        assert_eq!(service.name, "Widget");
        assert_eq!(service.wire_name, "Widget");

        // Python has no such override here, so the collision still rejects.
        let error = reject_for(Language::Python, input);
        assert!(
            error.contains("collision") && error.contains("Widget"),
            "{error}"
        );
    }

    #[test]
    fn accepts_validation_error_type_after_application_failure_switch() {
        // ValidationError used to be generated runtime boilerplate in Go and
        // Python. Payload validation now raises the SDK's application failure,
        // so the identifier is available to schemas again.
        let input = r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  e: { $ref: "#/$defs/ValidationError" }
$defs:
  ValidationError:
    type: object
    properties: { a: { type: string } }
"##;
        for language in [Language::Go, Language::Python, Language::Java] {
            parse_for(language, input).expect("ValidationError is no longer boilerplate");
        }
    }

    #[test]
    fn rejects_type_colliding_with_typescript_runtime_boilerplate() {
        // TypeScript imports the runtime `Violation` interface into every model
        // module, so a `$defs` type named `Violation` clashes with the import.
        let input = r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  v: { $ref: "#/$defs/Violation" }
$defs:
  Violation:
    type: object
    properties: { a: { type: string } }
"##;
        let error = reject_for(Language::TypeScript, input);
        assert!(
            error.contains("collision") && error.contains("Violation"),
            "{error}"
        );
        // Java names its aggregate error `ValidationException`; TypeScript has no
        // such symbol, so that name is accepted.
        let input = input.replace("Violation", "ValidationException");
        parse_for(Language::TypeScript, &input)
            .expect("TypeScript has no ValidationException boilerplate");
    }

    #[test]
    fn rejects_type_colliding_with_typescript_transfer_type_converter() {
        // Every TS model module imports nexus-rpc's `TransferTypeConverter` for
        // the contract its converter implements, so a `$defs` type of that name
        // conflicts with the import.
        let input = r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  c: { $ref: "#/$defs/TransferTypeConverter" }
$defs:
  TransferTypeConverter:
    type: object
    properties: { a: { type: string } }
"##;
        let error = reject_for(Language::TypeScript, input);
        assert!(
            error.contains("collision") && error.contains("TransferTypeConverter"),
            "{error}"
        );
        // The other targets import no such symbol, so the same schema is accepted.
        parse_for(Language::Go, input).expect("Go has no TransferTypeConverter boilerplate");
        parse_for(Language::Java, input).expect("Java has no TransferTypeConverter boilerplate");
    }

    #[test]
    fn rejects_typescript_transfer_type_converters_that_case_fold_together() {
        // The converter identifier is derived by lower-camel-casing the resolved
        // type name, which is not injective over the distinct type names P15
        // guarantees: both types below keep their verbatim names through an
        // override, yet derive the same `httpErrorTransferTypeConverter` — one
        // `export const` emitted twice. The derived name participates in the
        // pass, so this rejects with a fix-it instead.
        let input = r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
required: [a, b]
properties:
  a: { $ref: "#/$defs/HTTPError" }
  b: { $ref: "#/$defs/HttpError" }
$defs:
  HTTPError:
    type: object
    x-ts-name: HTTPError
    x-go-name: HTTPError
    x-py-name: HTTPError
    x-java-name: HTTPError
    properties: { m: { type: string } }
  HttpError:
    type: object
    properties: { n: { type: string } }
"##;
        let error = reject_for(Language::TypeScript, input);
        assert!(
            error.contains("collision") && error.contains("httpErrorTransferTypeConverter"),
            "{error}"
        );
        // Go and Java derive no value identifier from a type name, so the two
        // distinct type names are all they have to keep apart.
        parse_for(Language::Go, input).expect("Go derives no converter identifier");
        parse_for(Language::Java, input).expect("Java derives no converter identifier");
        // Python derives module-level names from the type name too. Its converter
        // classes stay apart (`_HTTPError…` / `_HttpError…`), but the declared-key
        // frozensets both shout to `_HTTP_ERROR_DECLARED`, so it rejects for that
        // reason rather than accepting.
        let python_error = reject_for(Language::Python, input);
        assert!(
            python_error.contains("collision") && python_error.contains("_HTTP_ERROR_DECLARED"),
            "{python_error}"
        );
    }

    #[test]
    fn rejects_service_name_colliding_with_a_transfer_type_converter() {
        // A service's TypeScript identifier shares the module scope with the
        // derived converter identifiers, so an override that lands on one is a
        // P15 collision (TS2440 plus a temporal-dead-zone `ReferenceError` if
        // emitted).
        let input = r##"
$schema: https://json-schema.org/draft/2020-12/schema
nexusrpc: "1.0.0"
services:
  Thing:
    fqn: example.t.v1.Thing
    x-ts-name: getInputTransferTypeConverter
    operations:
      get:
        input: { $ref: "#/$defs/GetInput" }
$defs:
  GetInput:
    type: object
    properties: { id: { type: string } }
"##;
        let error = reject_for(Language::TypeScript, input);
        assert!(
            error.contains("service `Thing`") && error.contains("getInputTransferTypeConverter"),
            "{error}"
        );
    }

    #[test]
    fn accepts_payload_validation_error_as_a_java_model_name() {
        // `PayloadValidationError` is only the Temporal failure type string;
        // Java emits no class with that name.
        let input = r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  e: { $ref: "#/$defs/PayloadValidationError" }
$defs:
  PayloadValidationError:
    type: object
    properties: { a: { type: string } }
"##;
        parse_for(Language::Java, input)
            .expect("Java has no PayloadValidationError boilerplate class");
    }

    #[test]
    fn rejects_type_colliding_with_java_violation_boilerplate() {
        // Java emits a public `Violation` record in the root package, imported
        // into model files, so a `$defs` type of that name clashes.
        let input = r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  v: { $ref: "#/$defs/Violation" }
$defs:
  Violation:
    type: object
    properties: { a: { type: string } }
"##;
        let error = reject_for(Language::Java, input);
        assert!(
            error.contains("collision") && error.contains("Violation"),
            "{error}"
        );
    }

    #[test]
    fn java_unconditionally_reserves_optional_runtime_class_names_for_models() {
        for reserved in ["TemporalSupport", "Base64Support"] {
            let input = format!(
                r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  helper: {{ $ref: "#/$defs/{reserved}" }}
$defs:
  {reserved}:
    type: object
    properties: {{ value: {{ type: string }} }}
"##
            );
            let error = reject_for(Language::Java, &input);
            assert!(
                error.contains("collision")
                    && error.contains(reserved)
                    && error.contains("x-java-name"),
                "{error}"
            );

            let renamed = input.replace(
                "    type: object\n    properties:",
                &format!("    x-java-name: Renamed{reserved}\n    type: object\n    properties:"),
            );
            parse_for(Language::Java, &renamed)
                .expect("a different emitted Java model name resolves the collision");
        }
    }

    #[test]
    fn java_unconditionally_reserves_optional_runtime_class_names_for_services() {
        for reserved in ["TemporalSupport", "Base64Support"] {
            let input = format!(
                r##"
$schema: https://json-schema.org/draft/2020-12/schema
nexusrpc: "1.0.0"
services:
  {reserved}:
    fqn: example.v1.{reserved}
    operations:
      ping:
        input:
          type: object
          properties: {{ value: {{ type: string }} }}
"##
            );
            let error = reject_for(Language::Java, &input);
            assert!(
                error.contains("collision")
                    && error.contains(reserved)
                    && error.contains("x-java-name"),
                "{error}"
            );

            let renamed = input.replace(
                &format!("  {reserved}:\n    fqn:"),
                &format!("  {reserved}:\n    x-java-name: Renamed{reserved}\n    fqn:"),
            );
            parse_for(Language::Java, &renamed)
                .expect("a different emitted Java service name resolves the collision");
        }
    }

    #[test]
    fn java_reserves_schema_dependent_offset_date_time_import() {
        let colliding = r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  value: { $ref: "#/$defs/OffsetDateTime" }
$defs:
  OffsetDateTime:
    type: object
    properties:
      timestamp: { type: string, format: date-time }
"##;
        let error = reject_for(Language::Java, colliding);
        assert!(
            error.contains("type `OffsetDateTime`")
                && error.contains("java.time.OffsetDateTime")
                && error.contains("x-java-name"),
            "{error}"
        );

        let renamed = colliding.replace(
            "  OffsetDateTime:\n    type: object",
            "  OffsetDateTime:\n    x-java-name: TimestampedValue\n    type: object",
        );
        parse_for(Language::Java, &renamed)
            .expect("x-java-name must move the model away from the imported simple name");

        let plain = colliding.replace(
            "      timestamp: { type: string, format: date-time }",
            "      timestamp: { type: string }",
        );
        parse_for(Language::Java, &plain)
            .expect("OffsetDateTime remains available when its file emits no such import");
    }

    #[test]
    fn rejects_type_colliding_with_python_runtime_boilerplate() {
        // Python imports the runtime `Violation` dataclass into every model
        // module by bare name, so a `$defs` type of that name clashes with the
        // import.
        let input = r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  v: { $ref: "#/$defs/Violation" }
$defs:
  Violation:
    type: object
    properties: { a: { type: string } }
"##;
        let error = reject_for(Language::Python, input);
        assert!(
            error.contains("collision") && error.contains("Violation"),
            "{error}"
        );
        // Java names its aggregate error `ValidationException`; Python has no
        // such symbol, so that name is accepted.
        let input = input.replace("Violation", "ValidationException");
        parse_for(Language::Python, &input).expect("Python has no ValidationException boilerplate");
    }
}
