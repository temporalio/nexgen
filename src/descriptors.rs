use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use heck::ToKebabCase;
use prost::Message;
use prost_types::{
    DescriptorProto, EnumDescriptorProto, FieldDescriptorProto, FileDescriptorProto,
    FileDescriptorSet, FileOptions, MethodDescriptorProto, ServiceDescriptorProto,
};

use crate::error::{Error, Result};

#[derive(Debug, Clone)]
pub struct DescriptorIndex {
    files: Vec<FileDescriptorProto>,
    messages: HashMap<String, MessageMetadata>,
    enums: HashMap<String, EnumMetadata>,
    rpcs: Vec<RpcMetadata>,
}

impl DescriptorIndex {
    pub fn load(path: &Path) -> Result<Self> {
        Self::load_many(&[path.to_path_buf()])
    }

    pub fn load_many(paths: &[PathBuf]) -> Result<Self> {
        let mut files = Vec::new();
        for path in paths {
            let bytes = fs::read(path).map_err(|source| Error::ReadFile {
                path: path.to_path_buf(),
                source,
            })?;
            let set = FileDescriptorSet::decode(bytes.as_slice()).map_err(|source| {
                Error::DescriptorDecode {
                    path: path.to_path_buf(),
                    source,
                }
            })?;
            files.extend(set.file);
        }

        Self::from_files(files)
    }

    pub fn from_descriptor_set(set: FileDescriptorSet) -> Result<Self> {
        Self::from_files(set.file)
    }

    pub fn from_descriptor_sets(sets: Vec<FileDescriptorSet>) -> Result<Self> {
        let mut files = Vec::new();
        for set in sets {
            files.extend(set.file);
        }
        Self::from_files(files)
    }

    fn from_files(files: Vec<FileDescriptorProto>) -> Result<Self> {
        let mut messages = HashMap::new();
        let mut enums = HashMap::new();
        let mut rpcs = Vec::new();
        let mut file_names = HashSet::new();
        let mut rpc_names = HashSet::new();

        for file in &files {
            if let Some(file_name) = file.name.as_ref()
                && !file_names.insert(file_name.clone())
            {
                return Err(Error::DuplicateDescriptorDefinition {
                    kind: "file",
                    name: file_name.clone(),
                });
            }
            let package = file.package.as_deref().unwrap_or_default();
            for service in &file.service {
                index_service(file, package, service, &mut rpcs, &mut rpc_names)?;
            }
            for enumeration in &file.enum_type {
                index_enum(file, package, None, enumeration, &mut enums)?;
            }
            for message in &file.message_type {
                index_message(file, package, None, message, &mut messages, &mut enums)?;
            }
        }

        Ok(Self {
            files,
            messages,
            enums,
            rpcs,
        })
    }

    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    pub fn message(&self, full_name: &str) -> Option<&MessageMetadata> {
        let normalized = full_name.trim_start_matches('.');
        self.messages.get(normalized)
    }

    pub fn messages_in_file(&self, file_name: &str) -> Result<Vec<&MessageMetadata>> {
        if !self
            .files
            .iter()
            .any(|file| file.name.as_deref() == Some(file_name))
        {
            return Err(Error::UnknownDescriptorFile {
                name: file_name.to_string(),
            });
        }
        let mut messages = self
            .messages
            .values()
            .filter(|message| {
                message.file_name.as_deref() == Some(file_name)
                    && !message
                        .descriptor
                        .options
                        .as_ref()
                        .and_then(|options| options.map_entry)
                        .unwrap_or(false)
            })
            .collect::<Vec<_>>();
        messages.sort_by(|left, right| left.full_name.cmp(&right.full_name));
        Ok(messages)
    }

    pub fn enumeration(&self, full_name: &str) -> Option<&EnumMetadata> {
        let normalized = full_name.trim_start_matches('.');
        self.enums.get(normalized)
    }

    pub fn resolve_rpc(&self, query: &str) -> Result<&RpcMetadata> {
        let normalized_query = normalize_identifier(query);

        let exact_matches = self
            .rpcs
            .iter()
            .filter(|rpc| rpc.matches_exact(&normalized_query))
            .collect::<Vec<_>>();
        if let [rpc] = exact_matches.as_slice() {
            return Ok(*rpc);
        }
        if !exact_matches.is_empty() {
            return Err(Error::AmbiguousRpcName {
                name: query.to_string(),
                matches: exact_matches
                    .iter()
                    .map(|rpc| rpc.full_name.clone())
                    .collect(),
            });
        }

        let fuzzy_matches = self
            .rpcs
            .iter()
            .filter(|rpc| rpc.matches_fuzzy(query))
            .collect::<Vec<_>>();
        if let [rpc] = fuzzy_matches.as_slice() {
            return Ok(*rpc);
        }
        if !fuzzy_matches.is_empty() {
            return Err(Error::AmbiguousRpcName {
                name: query.to_string(),
                matches: fuzzy_matches
                    .iter()
                    .map(|rpc| rpc.full_name.clone())
                    .collect(),
            });
        }

        Err(Error::UnknownRpcName {
            name: query.to_string(),
        })
    }

    pub fn resolve_message(&self, query: &str) -> Result<&MessageMetadata> {
        let normalized_query = normalize_identifier(query);
        let exact_matches = self
            .messages
            .values()
            .filter(|message| message.matches_exact(&normalized_query))
            .collect::<Vec<_>>();
        if let [message] = exact_matches.as_slice() {
            return Ok(*message);
        }
        if !exact_matches.is_empty() {
            let mut matches = exact_matches
                .iter()
                .map(|message| message.full_name.clone())
                .collect::<Vec<_>>();
            matches.sort();
            return Err(Error::AmbiguousMessageName {
                name: query.to_string(),
                matches,
            });
        }

        let fuzzy_matches = self
            .messages
            .values()
            .filter(|message| message.matches_fuzzy(query))
            .collect::<Vec<_>>();
        if let [message] = fuzzy_matches.as_slice() {
            return Ok(*message);
        }
        if !fuzzy_matches.is_empty() {
            let mut matches = fuzzy_matches
                .iter()
                .map(|message| message.full_name.clone())
                .collect::<Vec<_>>();
            matches.sort();
            return Err(Error::AmbiguousMessageName {
                name: query.to_string(),
                matches,
            });
        }

        Err(Error::UnknownMessageName {
            name: query.to_string(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct MessageMetadata {
    pub full_name: String,
    pub file_name: Option<String>,
    pub package: String,
    pub go_package: Option<String>,
    pub file_options: Option<FileOptions>,
    pub descriptor: DescriptorProto,
}

#[derive(Debug, Clone)]
pub(crate) struct ProtoOneofGroup<'a> {
    pub(crate) name: &'a str,
    pub(crate) fields: Vec<(usize, &'a FieldDescriptorProto)>,
}

/// Returns the real protobuf oneofs declared by `message`.
///
/// Protobuf represents `proto3 optional` with a synthetic, single-field oneof.
/// Those declarations deliberately stay ordinary optional fields everywhere in
/// nex-gen and are therefore filtered out here.
pub(crate) fn real_oneof_groups(
    message: &MessageMetadata,
) -> std::result::Result<Vec<ProtoOneofGroup<'_>>, String> {
    let mut fields_by_oneof = vec![Vec::new(); message.descriptor.oneof_decl.len()];
    for (field_index, field) in message.descriptor.field.iter().enumerate() {
        let Some(raw_index) = field.oneof_index else {
            continue;
        };
        let Ok(oneof_index) = usize::try_from(raw_index) else {
            return Err(format!(
                "field at index {field_index} has invalid oneof index {raw_index}"
            ));
        };
        let Some(fields) = fields_by_oneof.get_mut(oneof_index) else {
            return Err(format!(
                "field at index {field_index} has unknown oneof index {raw_index}"
            ));
        };
        fields.push((field_index, field));
    }

    let mut groups = Vec::new();
    for (oneof_index, oneof) in message.descriptor.oneof_decl.iter().enumerate() {
        let fields = std::mem::take(&mut fields_by_oneof[oneof_index]);
        if fields.len() == 1 && fields[0].1.proto3_optional.unwrap_or(false) {
            continue;
        }
        if fields.is_empty() {
            return Err(format!(
                "oneof declaration at index {oneof_index} has no fields"
            ));
        }
        if fields
            .iter()
            .any(|(_, field)| field.proto3_optional.unwrap_or(false))
        {
            return Err(format!(
                "oneof declaration at index {oneof_index} is malformed"
            ));
        }
        let Some(name) = oneof.name.as_deref() else {
            return Err(format!(
                "oneof declaration at index {oneof_index} is missing a name"
            ));
        };
        groups.push(ProtoOneofGroup { name, fields });
    }
    Ok(groups)
}

impl MessageMetadata {
    fn matches_exact(&self, normalized_query: &str) -> bool {
        [
            normalize_identifier(&self.full_name),
            normalize_identifier(self.full_name.rsplit('.').next().unwrap_or_default()),
        ]
        .into_iter()
        .any(|candidate| candidate == normalized_query)
    }

    fn matches_fuzzy(&self, query: &str) -> bool {
        let query_tokens = kebab_tokens(query);
        !query_tokens.is_empty()
            && (tokens_are_subsequence(&query_tokens, &kebab_tokens(&self.full_name))
                || tokens_are_subsequence(
                    &query_tokens,
                    &kebab_tokens(self.full_name.rsplit('.').next().unwrap_or_default()),
                ))
    }
}

#[derive(Debug, Clone)]
pub struct EnumMetadata {
    pub full_name: String,
    pub file_name: Option<String>,
    pub package: String,
    pub go_package: Option<String>,
    pub file_options: Option<FileOptions>,
    pub descriptor: EnumDescriptorProto,
}

#[derive(Debug, Clone)]
pub struct RpcMetadata {
    pub full_name: String,
    pub name: String,
    pub service_name: String,
    pub service_full_name: String,
    pub input_type: String,
    pub output_type: String,
    pub file_name: Option<String>,
    pub package: String,
    pub file_options: Option<FileOptions>,
    pub descriptor: MethodDescriptorProto,
}

impl RpcMetadata {
    fn matches_exact(&self, normalized_query: &str) -> bool {
        [
            normalize_identifier(&self.full_name),
            normalize_identifier(&self.service_method_name()),
            normalize_identifier(&self.name),
        ]
        .into_iter()
        .any(|candidate| candidate == normalized_query)
    }

    fn matches_fuzzy(&self, query: &str) -> bool {
        let query_tokens = kebab_tokens(query);
        !query_tokens.is_empty()
            && (tokens_are_subsequence(&query_tokens, &kebab_tokens(&self.name))
                || tokens_are_subsequence(
                    &query_tokens,
                    &kebab_tokens(&self.service_method_name()),
                )
                || tokens_are_subsequence(&query_tokens, &kebab_tokens(&self.full_name)))
    }

    fn service_method_name(&self) -> String {
        format!("{}.{}", self.service_name, self.name)
    }
}

fn index_message(
    file: &FileDescriptorProto,
    package: &str,
    parent: Option<&str>,
    descriptor: &DescriptorProto,
    messages: &mut HashMap<String, MessageMetadata>,
    enums: &mut HashMap<String, EnumMetadata>,
) -> Result<()> {
    let Some(name) = descriptor.name.as_deref() else {
        return Ok(());
    };

    let full_name = if let Some(parent) = parent {
        format!("{parent}.{name}")
    } else if package.is_empty() {
        name.to_string()
    } else {
        format!("{package}.{name}")
    };

    if messages.contains_key(&full_name) {
        return Err(Error::DuplicateDescriptorDefinition {
            kind: "message",
            name: full_name,
        });
    }
    messages.insert(
        full_name.clone(),
        MessageMetadata {
            full_name: full_name.clone(),
            file_name: file.name.clone(),
            package: package.to_string(),
            go_package: file_go_package(file),
            file_options: file.options.clone(),
            descriptor: descriptor.clone(),
        },
    );

    for enumeration in &descriptor.enum_type {
        index_enum(file, package, Some(&full_name), enumeration, enums)?;
    }

    for nested in &descriptor.nested_type {
        index_message(file, package, Some(&full_name), nested, messages, enums)?;
    }

    Ok(())
}

fn index_enum(
    file: &FileDescriptorProto,
    package: &str,
    parent: Option<&str>,
    descriptor: &EnumDescriptorProto,
    enums: &mut HashMap<String, EnumMetadata>,
) -> Result<()> {
    let Some(name) = descriptor.name.as_deref() else {
        return Ok(());
    };

    let full_name = if let Some(parent) = parent {
        format!("{parent}.{name}")
    } else if package.is_empty() {
        name.to_string()
    } else {
        format!("{package}.{name}")
    };

    if enums.contains_key(&full_name) {
        return Err(Error::DuplicateDescriptorDefinition {
            kind: "enum",
            name: full_name,
        });
    }
    enums.insert(
        full_name.clone(),
        EnumMetadata {
            full_name,
            file_name: file.name.clone(),
            package: package.to_string(),
            go_package: file_go_package(file),
            file_options: file.options.clone(),
            descriptor: descriptor.clone(),
        },
    );

    Ok(())
}

/// Reads the `go_package` file option from a proto file descriptor, if present.
///
/// The option uses the conventional `<import path>[;<package alias>]` form,
/// e.g. `go.temporal.io/api/common/v1;common`.
fn file_go_package(file: &FileDescriptorProto) -> Option<String> {
    file.options
        .as_ref()
        .and_then(|options| options.go_package.clone())
}

fn index_service(
    file: &FileDescriptorProto,
    package: &str,
    descriptor: &ServiceDescriptorProto,
    rpcs: &mut Vec<RpcMetadata>,
    rpc_names: &mut HashSet<String>,
) -> Result<()> {
    let Some(service_name) = descriptor.name.as_deref() else {
        return Ok(());
    };

    let service_full_name = if package.is_empty() {
        service_name.to_string()
    } else {
        format!("{package}.{service_name}")
    };

    for method in &descriptor.method {
        let Some(name) = method.name.as_deref() else {
            continue;
        };
        let full_name = format!("{service_full_name}.{name}");
        if !rpc_names.insert(full_name.clone()) {
            return Err(Error::DuplicateDescriptorDefinition {
                kind: "RPC",
                name: full_name,
            });
        }

        rpcs.push(RpcMetadata {
            full_name,
            name: name.to_string(),
            service_name: service_name.to_string(),
            service_full_name: service_full_name.clone(),
            input_type: method.input_type.clone().unwrap_or_default(),
            output_type: method.output_type.clone().unwrap_or_default(),
            file_name: file.name.clone(),
            package: package.to_string(),
            file_options: file.options.clone(),
            descriptor: method.clone(),
        });
    }

    Ok(())
}

fn normalize_identifier(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn kebab_tokens(value: &str) -> Vec<String> {
    value
        .trim_start_matches('.')
        .replace('.', "-")
        .to_kebab_case()
        .split('-')
        .filter(|token| !token.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn tokens_are_subsequence(query: &[String], candidate: &[String]) -> bool {
    if query.is_empty() {
        return true;
    }

    let mut index = 0usize;
    for token in candidate {
        if token == &query[index] {
            index += 1;
            if index == query.len() {
                return true;
            }
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use prost_types::{
        DescriptorProto, FileDescriptorProto, FileDescriptorSet, FileOptions,
        MethodDescriptorProto, ServiceDescriptorProto,
    };

    use super::DescriptorIndex;
    use crate::error::Error;

    fn file_with_message(file_name: &str, package: &str, message_name: &str) -> FileDescriptorSet {
        FileDescriptorSet {
            file: vec![FileDescriptorProto {
                name: Some(file_name.to_string()),
                package: Some(package.to_string()),
                message_type: vec![DescriptorProto {
                    name: Some(message_name.to_string()),
                    ..Default::default()
                }],
                ..Default::default()
            }],
        }
    }

    #[test]
    fn merges_multiple_descriptor_sets() {
        let index = DescriptorIndex::from_descriptor_sets(vec![
            file_with_message("one.proto", "pkg.one", "First"),
            file_with_message("two.proto", "pkg.two", "Second"),
        ])
        .unwrap();

        assert!(index.message("pkg.one.First").is_some());
        assert!(index.message("pkg.two.Second").is_some());
        assert_eq!(index.file_count(), 2);
    }

    #[test]
    fn rejects_duplicate_message_names_across_descriptor_sets() {
        let error = DescriptorIndex::from_descriptor_sets(vec![
            file_with_message("one.proto", "pkg", "Duplicate"),
            file_with_message("two.proto", "pkg", "Duplicate"),
        ])
        .unwrap_err();

        assert!(matches!(
            error,
            Error::DuplicateDescriptorDefinition {
                kind: "message",
                ref name,
            } if name == "pkg.Duplicate"
        ));
    }

    #[test]
    fn rejects_duplicate_rpc_names_across_descriptor_sets() {
        let make_set = |file_name: &str| FileDescriptorSet {
            file: vec![FileDescriptorProto {
                name: Some(file_name.to_string()),
                package: Some("pkg".to_string()),
                service: vec![ServiceDescriptorProto {
                    name: Some("Svc".to_string()),
                    method: vec![MethodDescriptorProto {
                        name: Some("Run".to_string()),
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        };

        let error = DescriptorIndex::from_descriptor_sets(vec![
            make_set("one.proto"),
            make_set("two.proto"),
        ])
        .unwrap_err();

        assert!(matches!(
            error,
            Error::DuplicateDescriptorDefinition {
                kind: "RPC",
                ref name,
            } if name == "pkg.Svc.Run"
        ));
    }

    #[test]
    fn records_descriptor_file_options_on_indexed_types() {
        let index = DescriptorIndex::from_descriptor_set(FileDescriptorSet {
            file: vec![FileDescriptorProto {
                name: Some("types.proto".to_string()),
                package: Some("pkg".to_string()),
                options: Some(FileOptions {
                    csharp_namespace: Some("Example.Proto".to_string()),
                    java_package: Some("example.proto".to_string()),
                    ..Default::default()
                }),
                message_type: vec![DescriptorProto {
                    name: Some("Request".to_string()),
                    ..Default::default()
                }],
                ..Default::default()
            }],
        })
        .unwrap();

        let options = index
            .message("pkg.Request")
            .and_then(|message| message.file_options.as_ref())
            .expect("message should carry descriptor file options");
        assert_eq!(options.csharp_namespace.as_deref(), Some("Example.Proto"));
        assert_eq!(options.java_package.as_deref(), Some("example.proto"));
    }
}
