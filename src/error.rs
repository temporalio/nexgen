use std::io;
use std::path::PathBuf;
use std::process::ExitStatus;

use thiserror::Error;

use crate::language::Language;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("failed to read `{path}`: {source}")]
    ReadFile {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("failed to write `{path}`: {source}")]
    WriteFile {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("generated file path `{path}` is invalid: {reason}")]
    InvalidGeneratedPath { path: PathBuf, reason: String },

    #[error("generated file path `{path}` conflicts with another generated file")]
    GeneratedFileConflict { path: PathBuf },

    #[error(
        "flattened API for `{type_name}` would generate duplicate parameter `{field}` from `{conflicting_field}`"
    )]
    FlattenedApiFieldConflict {
        type_name: String,
        field: String,
        conflicting_field: String,
    },

    #[error(
        "Python generated name `{name}` for {generated_by} conflicts with {conflicting_declaration}"
    )]
    PythonGeneratedNameConflict {
        name: String,
        generated_by: String,
        conflicting_declaration: String,
    },

    #[error("refusing to overwrite existing path `{path}`")]
    OutputPathExists { path: PathBuf },

    #[error("refusing to generate into `{path}`: the output path resolves to the filesystem root")]
    OutputPathIsRoot { path: PathBuf },

    #[error("output path `{path}`'s final component is not valid UTF-8")]
    OutputPathNotUtf8 { path: PathBuf },

    #[error(
        "refusing to delete example output path `{path}`: it is not a directory inside `{root}`"
    )]
    ExampleOutputPathOutsideRoot { path: PathBuf, root: PathBuf },

    #[error("failed to run formatter `{command}` for `{path}`: {source}")]
    RunFormatter {
        path: PathBuf,
        command: String,
        #[source]
        source: io::Error,
    },

    #[error("formatter `{command}` failed for `{path}` with status {status}")]
    FormatterFailed {
        path: PathBuf,
        command: String,
        status: ExitStatus,
    },

    #[error("failed to run command `{command}` in `{cwd}`: {source}")]
    RunCommand {
        cwd: PathBuf,
        command: String,
        #[source]
        source: io::Error,
    },

    #[error("command `{command}` failed in `{cwd}` with status {status}")]
    CommandFailed {
        cwd: PathBuf,
        command: String,
        status: ExitStatus,
    },

    #[error("failed to parse WIT from `{path}`: {message}")]
    WitParse { path: PathBuf, message: String },

    #[error("invalid WIT in `{path}`: {reason}")]
    InvalidWit { path: PathBuf, reason: String },

    #[error("failed to parse JSON schema from `{path}`: {message}")]
    JsonSchemaParse { path: PathBuf, message: String },

    #[error("invalid JSON schema in `{path}`: {reason}")]
    InvalidJsonSchema { path: PathBuf, reason: String },

    #[error("unsupported input format for `{path}`; expected `.wit`, `.json`, `.yaml`, or `.yml`")]
    UnsupportedInputFormat { path: PathBuf },

    #[error(
        "mixed input formats are not supported: first input is `{first}`, but `{path}` is `{found}`"
    )]
    MixedInputFormats {
        first: &'static str,
        path: PathBuf,
        found: &'static str,
    },

    #[error("invalid WIT directive `{directive}` on {context} in `{path}`: {reason}")]
    InvalidWitDirective {
        path: PathBuf,
        context: String,
        directive: String,
        reason: String,
    },

    #[error("failed to decode descriptor set from `{path}`: {source}")]
    DescriptorDecode {
        path: PathBuf,
        #[source]
        source: prost::DecodeError,
    },

    #[error("duplicate descriptor {kind} `{name}`")]
    DuplicateDescriptorDefinition { kind: &'static str, name: String },

    #[error("language `{language}` is not implemented yet")]
    UnsupportedLanguage { language: Language },

    #[error("{language} protobuf conversion does not yet support oneof group `{message}.{oneof}`")]
    UnsupportedProtoOneofConversion {
        language: Language,
        message: String,
        oneof: String,
    },

    #[error(
        "{language} protobuf conversion does not yet support generic carrier field `{message}.{field}`"
    )]
    UnsupportedProtoGenericCarrierConversion {
        language: Language,
        message: String,
        field: String,
    },

    #[error("Java code generation does not support protobuf-backed model `{message}`")]
    UnsupportedJavaProtoModel { message: String },

    #[error("{language} support namespace `{namespace}` is not supported")]
    UnsupportedSupportNamespace {
        language: Language,
        namespace: String,
    },

    #[error("invalid {language} support namespace `{namespace}`: {reason}")]
    InvalidSupportNamespace {
        language: Language,
        namespace: String,
        reason: String,
    },

    #[error("RPC `{name}` was not found in the descriptor set")]
    UnknownRpcName { name: String },

    #[error("RPC name `{name}` is ambiguous; matches: {matches:?}")]
    AmbiguousRpcName { name: String, matches: Vec<String> },

    #[error("message `{name}` was not found in the descriptor set")]
    UnknownMessageName { name: String },

    #[error("message name `{name}` is ambiguous; matches: {matches:?}")]
    AmbiguousMessageName { name: String, matches: Vec<String> },

    #[error("unknown {language} example `{example_id}`")]
    UnknownExampleId {
        language: Language,
        example_id: String,
    },

    #[error("cannot generate add-rpc WIT for `{context}`: {reason}")]
    UnsupportedAddRpc { context: String, reason: String },

    #[error("cannot generate add-message WIT for `{context}`: {reason}")]
    UnsupportedAddMessage { context: String, reason: String },

    #[error("service `{service}` is missing an endpoint")]
    MissingServiceEndpoint { service: String },

    #[error("resource `{service}.{resource}` is invalid: {reason}")]
    InvalidResource {
        service: String,
        resource: String,
        reason: String,
    },

    #[error("resource method `{service}.{resource}.{method}` is invalid: {reason}")]
    InvalidResourceMethod {
        service: String,
        resource: String,
        method: String,
        reason: String,
    },

    #[error(
        "service `{service}` operation `{operation}` output is missing required `type` or `transform` field"
    )]
    IncompleteOperationOutputTransform { service: String, operation: String },

    #[error(
        "service `{service}` operation `{operation}` references unknown input proto type `{type_name}`"
    )]
    UnknownOperationInputProto {
        service: String,
        operation: String,
        type_name: String,
    },

    #[error(
        "service `{service}` operation `{operation}` references unknown output proto type `{type_name}`"
    )]
    UnknownOperationOutputProto {
        service: String,
        operation: String,
        type_name: String,
    },

    #[error(
        "type override `{type_name}` is missing required `type` field; `fromProto` and `toProto` default when omitted"
    )]
    IncompleteTypeOverride { type_name: String },

    #[error("type override references unknown proto type `{type_name}`")]
    UnknownTypeOverride { type_name: String },

    #[error("type override for `{message}` references unknown field `{field}`")]
    UnknownTypeOverrideField { message: String, field: String },

    #[error("type override for `{message}.{field}` cannot be both required and omitted")]
    ConflictingTypeOverrideField { message: String, field: String },

    #[error("type override for `{message}.{field}` cannot be both omitted and customized")]
    OmittedCustomizedTypeOverrideField { message: String, field: String },

    #[error(
        "type override for `{message}` must declare field `{field}` in WIT or add that field and mark it with `@nexus.omit`"
    )]
    UndeclaredTypeOverrideField { message: String, field: String },

    #[error(
        "type override for `{type_name}.{field}` is missing required field customization; expected one of `name`, `type`, `source`, or `function`"
    )]
    IncompleteTypeOverrideField { type_name: String, field: String },

    #[error(
        "type override for `{message}.{field}` cannot combine field `{property}` with `{conflicting_property}`"
    )]
    ConflictingTypeOverrideFieldProperties {
        message: String,
        field: String,
        property: &'static str,
        conflicting_property: &'static str,
    },

    #[error("type override for `{message}.{field}` cannot use field `{property}`")]
    UnsupportedTypeOverrideFieldProperty {
        message: String,
        field: String,
        property: &'static str,
    },

    #[error("type override for `{message}.{field}` has invalid field `{property}`: {reason}")]
    InvalidTypeOverrideField {
        message: String,
        field: String,
        property: &'static str,
        reason: String,
    },

    #[error("type override for `{message}.{field}` cannot be marked required: {reason}")]
    UnsupportedRequiredTypeField {
        message: String,
        field: String,
        reason: String,
    },

    #[error("type override for `{message}.{field}` cannot use field `source`: {reason}")]
    UnsupportedSourcedTypeField {
        message: String,
        field: String,
        reason: String,
    },

    #[error("type override for enum `{enumeration}` cannot use `{property}`")]
    UnsupportedEnumTypeOverrideProperty {
        enumeration: String,
        property: &'static str,
    },

    #[error("Go code generation does not support {context}: {reason}")]
    UnsupportedGoType { context: String, reason: String },

    #[error("Go proto conversion for {context} is not supported: {reason}")]
    UnsupportedGoProtoConversion { context: String, reason: String },

    #[error(
        "`@nexus.namespace go=\"{namespace}\"` implies Go package `{expected_package}`, but the output directory resolves to package `{actual_package}`; point `--output` at a directory named `{expected_package}`"
    )]
    GoNamespacePackageMismatch {
        namespace: String,
        expected_package: String,
        actual_package: String,
    },

    #[error(
        "output directory name `{output_dir_name}` has no ASCII letters, digits, or underscores to build a Go package name from; rename the output directory"
    )]
    GoPackageNameEmpty { output_dir_name: String },

    #[error("Java code generation requires `--package-name`")]
    JavaPackageNameMissing,

    #[error(
        "`--package-name {package_name}` must end with the output directory name `{output_dir_name}`, but its last segment is `{last_segment}`; point `--output` at a directory named `{last_segment}` or change the package's last segment to `{output_dir_name}`"
    )]
    JavaPackageNameMismatch {
        package_name: String,
        last_segment: String,
        output_dir_name: String,
    },

    #[error("type override `{type_name}` cannot use `{property}`")]
    UnsupportedTypeOverrideProperty {
        type_name: String,
        property: &'static str,
    },

    #[error(
        "type override `{type_name}` cannot combine `{property}` with `{conflicting_property}`"
    )]
    ConflictingTypeOverrideProperties {
        type_name: String,
        property: &'static str,
        conflicting_property: &'static str,
    },
}
