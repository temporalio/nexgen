// The `add-message` CLI surface lives behind the `advanced` feature.
#![cfg(feature = "advanced")]

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use nexgen::descriptors::DescriptorIndex;
use nexgen::generator::generate_source;
use nexgen::language::Language;
use nexgen::parser::load_api_spec_from_wit_for_language_with_inputs;
use nexgen::{SupportFiles, add_message_to_string, add_rpc_to_string};
use prost::Message;
use prost_types::field_descriptor_proto::{Label, Type};
use prost_types::{
    DescriptorProto, FieldDescriptorProto, FileDescriptorProto, FileDescriptorSet,
    MethodDescriptorProto, OneofDescriptorProto, ServiceDescriptorProto,
};

fn project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn descriptor_path(root: &std::path::Path) -> PathBuf {
    root.join("advanced/samples/descriptors/temporal_api.bin")
}

fn linked_inputs_path(root: &std::path::Path) -> PathBuf {
    root.join("advanced/samples/inputs/deps")
}

/// A per-call temp directory. The clock alone is not enough: `write_oneof_descriptor`
/// passes one fixed `name` and several tests call it, so two threads that read the
/// same timestamp would land on one directory and `fs::write` the same `api.bin`
/// concurrently — a reader then sees the truncated file mid-write. The counter makes
/// the path unique per call regardless of clock granularity.
fn unique_temp_dir(name: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("nexgen-{name}-{unique}-{seq}"))
}

fn write_temp_wit(name: &str, contents: &str) -> PathBuf {
    let temp_dir = unique_temp_dir(name);
    fs::create_dir_all(&temp_dir).unwrap();
    let path = temp_dir.join("input.wit");
    fs::write(&path, contents).unwrap();
    path
}

fn proto_field(
    name: &str,
    number: i32,
    field_type: Type,
    type_name: Option<&str>,
    oneof_index: Option<i32>,
) -> FieldDescriptorProto {
    FieldDescriptorProto {
        name: Some(name.to_string()),
        number: Some(number),
        label: Some(Label::Optional as i32),
        r#type: Some(field_type as i32),
        type_name: type_name.map(str::to_string),
        oneof_index,
        ..Default::default()
    }
}

fn write_oneof_descriptor() -> PathBuf {
    let outcome = DescriptorProto {
        name: Some("Outcome".to_string()),
        field: vec![
            proto_field("success", 1, Type::String, None, Some(0)),
            proto_field(
                "failure",
                2,
                Type::Message,
                Some(".acme.oneof.v1.FailureDetail"),
                Some(0),
            ),
        ],
        oneof_decl: vec![OneofDescriptorProto {
            name: Some("result".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let root = DescriptorProto {
        name: Some("Root".to_string()),
        field: vec![
            proto_field(
                "outcome",
                1,
                Type::Message,
                Some(".acme.oneof.v1.Root.Outcome"),
                None,
            ),
            proto_field(
                "mixed",
                2,
                Type::Message,
                Some(".acme.oneof.v1.Mixed"),
                None,
            ),
            proto_field(
                "multiple",
                3,
                Type::Message,
                Some(".acme.oneof.v1.Multiple"),
                None,
            ),
            FieldDescriptorProto {
                proto3_optional: Some(true),
                ..proto_field("nickname", 4, Type::String, None, Some(0))
            },
        ],
        nested_type: vec![outcome],
        oneof_decl: vec![OneofDescriptorProto {
            name: Some("_nickname".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let mixed = DescriptorProto {
        name: Some("Mixed".to_string()),
        field: vec![
            proto_field("id", 1, Type::String, None, None),
            proto_field("text", 2, Type::String, None, Some(0)),
            proto_field("count", 3, Type::Int32, None, Some(0)),
        ],
        oneof_decl: vec![OneofDescriptorProto {
            name: Some("choice".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let multiple = DescriptorProto {
        name: Some("Multiple".to_string()),
        field: vec![
            proto_field("left", 1, Type::String, None, Some(0)),
            proto_field("right", 2, Type::Int64, None, Some(0)),
            proto_field("enabled", 3, Type::Bool, None, Some(1)),
        ],
        oneof_decl: vec![
            OneofDescriptorProto {
                name: Some("direction".to_string()),
                ..Default::default()
            },
            OneofDescriptorProto {
                name: Some("state".to_string()),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let set = FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: Some("acme/oneof/v1/service.proto".to_string()),
            package: Some("acme.oneof.v1".to_string()),
            syntax: Some("proto3".to_string()),
            message_type: vec![
                root,
                mixed,
                multiple,
                DescriptorProto {
                    name: Some("FailureDetail".to_string()),
                    ..Default::default()
                },
            ],
            service: vec![ServiceDescriptorProto {
                name: Some("TestService".to_string()),
                method: vec![
                    MethodDescriptorProto {
                        name: Some("Run".to_string()),
                        input_type: Some(".acme.oneof.v1.Root".to_string()),
                        output_type: Some(".acme.oneof.v1.Root".to_string()),
                        ..Default::default()
                    },
                    MethodDescriptorProto {
                        name: Some("Complete".to_string()),
                        input_type: Some(".acme.oneof.v1.Root.Outcome".to_string()),
                        output_type: Some(".acme.oneof.v1.Root.Outcome".to_string()),
                        ..Default::default()
                    },
                    MethodDescriptorProto {
                        name: Some("Choose".to_string()),
                        input_type: Some(".acme.oneof.v1.Mixed".to_string()),
                        output_type: Some(".acme.oneof.v1.Mixed".to_string()),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        }],
    };
    let temp_dir = unique_temp_dir("oneof-descriptor");
    fs::create_dir_all(&temp_dir).unwrap();
    let path = temp_dir.join("api.bin");
    fs::write(&path, set.encode_to_vec()).unwrap();
    path
}

#[test]
fn cli_add_message_generates_a_standalone_message_tree() {
    let root = project_root();
    let output = Command::new(env!("CARGO_BIN_EXE_nexgen"))
        .args([
            "add-message",
            "--descriptors",
            descriptor_path(&root).to_str().unwrap(),
            "--message",
            "WorkflowExecutionInfo",
            "--input",
            linked_inputs_path(&root).to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(output.status.success());
    let rendered = String::from_utf8(output.stdout).unwrap();
    assert!(rendered.contains("world system {\n  export workflow-execution-info;\n}"));
    assert!(rendered.contains("interface workflow-execution-info {"));
    assert!(rendered.contains("record workflow-execution-info {"));
    assert!(rendered.contains("record workflow-execution {"));
    assert!(rendered.contains("enum workflow-execution-status {"));
    assert!(rendered.contains("use nexus:temporal-types/model@1.0.0.{"));
    assert!(!rendered.contains("@nexus.endpoint"));
    assert!(!rendered.contains(": func("));
}

#[test]
fn add_message_inserts_only_missing_types_into_the_sole_exported_interface() {
    let root = project_root();
    let input_path = write_temp_wit(
        "add-message-existing",
        r#"package temporal:nexus@1.0.0;

world system {
  export models;
}

interface models {
  /// @nexus.proto "temporal.api.common.v1.WorkflowExecution"
  record workflow-execution {
    workflow-id: string,
    run-id: string,
  }
}
"#,
    );
    let rendered = add_message_to_string(
        &[descriptor_path(&root)],
        "temporal.api.common.v1.WorkflowExecution",
        &[input_path.clone(), linked_inputs_path(&root)],
    )
    .unwrap();

    assert_eq!(rendered, fs::read_to_string(input_path).unwrap());

    let input_path = write_temp_wit(
        "add-message-insert",
        r#"package temporal:nexus@1.0.0;

world system {
  export models;
}

interface models {
}
"#,
    );
    let rendered = add_message_to_string(
        &[descriptor_path(&root)],
        "temporal.api.common.v1.WorkflowExecution",
        &[input_path, linked_inputs_path(&root)],
    )
    .unwrap();
    assert!(rendered.contains("interface models {\n  /// @nexus.proto"));
    assert!(rendered.contains("record workflow-execution {"));
}

#[test]
fn add_message_renders_real_oneofs_as_variants() {
    let descriptor = write_oneof_descriptor();
    let rendered = add_message_to_string(&[descriptor], "acme.oneof.v1.Root", &[]).unwrap();

    assert!(rendered.contains(
        r#"/// Protobuf oneof `acme.oneof.v1.Root.Outcome.result`.
  /// @nexus.proto "acme.oneof.v1.Root.Outcome.result"
  variant root-outcome-result {
    success(string),
    failure(failure-detail),
  }"#
    ));
    assert!(rendered.contains(
        r#"/// @nexus.proto "acme.oneof.v1.Root.Outcome"
  record root-outcome {
    %result: option<root-outcome-result>,
  }"#
    ));
    assert!(rendered.contains(
        r#"variant mixed-choice {
    text(string),
    count(s32),
  }"#
    ));
    assert!(rendered.contains(
        r#"record mixed {
    id: string,
    choice: option<mixed-choice>,
  }"#
    ));
    assert!(rendered.contains("direction: option<multiple-direction>,"));
    assert!(rendered.contains("state: option<multiple-state>,"));
    assert!(rendered.contains("nickname: option<string>,"));
    assert!(!rendered.contains("variant root-nickname"));
    assert!(!rendered.contains("text: option<string>"));
}

#[test]
fn add_rpc_uses_the_same_oneof_variant_rendering() {
    let descriptor = write_oneof_descriptor();
    let rendered = add_rpc_to_string(&[descriptor], "TestService.Run", &[]).unwrap();

    assert!(rendered.contains("variant root-outcome-result {"));
    assert!(rendered.contains("%result: option<root-outcome-result>,"));
    assert!(rendered.contains("variant mixed-choice {"));
    assert!(rendered.contains("choice: option<mixed-choice>,"));
    assert!(rendered.contains("run: func("));
}

#[test]
fn scaffold_commands_reject_an_existing_direct_oneof_variant() {
    let descriptor = write_oneof_descriptor();
    let input_path = write_temp_wit(
        "existing-oneof-variant",
        r#"package temporal:nexus@1.0.0;

world system {
  export test-service;
}

interface test-service {
  /// @nexus.proto "acme.oneof.v1.Root.Outcome.result"
  variant root-outcome {
    success(string),
    failure(failure-detail),
  }

  /// @nexus.proto "acme.oneof.v1.FailureDetail"
  record failure-detail {
  }

  complete: func(request: root-outcome) -> root-outcome;
}
"#,
    );

    let error = add_rpc_to_string(
        &[descriptor.clone()],
        "TestService.Complete",
        &[input_path.clone()],
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("generated type name `root-outcome` would collide with an existing WIT type")
    );

    let error = add_message_to_string(&[descriptor], "acme.oneof.v1.Root.Outcome", &[input_path])
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("generated type name `root-outcome` would collide with an existing WIT type")
    );
}

#[test]
fn add_rpc_accepts_an_existing_grouped_oneof_record() {
    let descriptor = write_oneof_descriptor();
    let grouped = r#"package temporal:nexus@1.0.0;

world system {
  export test-service;
}

interface test-service {
  /// @nexus.proto "acme.oneof.v1.Mixed.choice"
  variant mixed-choice {
    text(string),
    count(s32),
  }

  /// @nexus.proto "acme.oneof.v1.Mixed"
  record mixed {
    id: string,
    choice: option<mixed-choice>,
  }

  choose: func(request: mixed) -> mixed;
}
"#;
    let grouped_path = write_temp_wit("existing-grouped-oneof", grouped);
    let rendered =
        add_rpc_to_string(&[descriptor.clone()], "TestService.Choose", &[grouped_path]).unwrap();
    assert_eq!(rendered, grouped);

    let single_group = r#"package temporal:nexus@1.0.0;

world system {
  export test-service;
}

interface test-service {
  /// @nexus.proto "acme.oneof.v1.Root.Outcome.result"
  variant root-outcome-result {
    success(string),
    failure(failure-detail),
  }

  /// @nexus.proto "acme.oneof.v1.Root.Outcome"
  record root-outcome {
    %result: option<root-outcome-result>,
  }

  /// @nexus.proto "acme.oneof.v1.FailureDetail"
  record failure-detail {
  }

  complete: func(request: root-outcome) -> root-outcome;
}
"#;
    let single_group_path = write_temp_wit("existing-single-group-oneof", single_group);
    let rendered =
        add_rpc_to_string(&[descriptor], "TestService.Complete", &[single_group_path]).unwrap();
    assert_eq!(rendered, single_group);
}

#[test]
fn add_rpc_rejects_a_grouped_oneof_variant_without_its_source() {
    let descriptor = write_oneof_descriptor();
    let grouped = r#"package temporal:nexus@1.0.0;

world system {
  export test-service;
}

interface test-service {
  variant mixed-choice {
    text(string),
    count(s32),
  }

  /// @nexus.proto "acme.oneof.v1.Mixed"
  record mixed {
    id: string,
    choice: option<mixed-choice>,
  }

  choose: func(request: mixed) -> mixed;
}
"#;
    let grouped_path = write_temp_wit("grouped-oneof-without-source", grouped);
    let error =
        add_rpc_to_string(&[descriptor], "TestService.Choose", &[grouped_path]).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("must declare `@nexus.proto \"acme.oneof.v1.Mixed.choice\"`")
    );
}

#[test]
fn scaffolded_oneof_wit_passes_compiler_planning() {
    let descriptor_path = write_oneof_descriptor();
    let rendered =
        add_message_to_string(&[descriptor_path.clone()], "acme.oneof.v1.Mixed", &[]).unwrap();
    let wit_path = write_temp_wit("planned-scaffolded-oneof", &rendered);
    let spec =
        load_api_spec_from_wit_for_language_with_inputs(Language::Python, &[wit_path]).unwrap();
    let descriptors = DescriptorIndex::load(&descriptor_path).unwrap();

    generate_source(
        Language::Python,
        spec,
        &descriptors,
        &SupportFiles::default(),
    )
    .unwrap();
}

#[test]
fn scaffold_commands_reject_existing_ungrouped_oneofs() {
    let descriptor = write_oneof_descriptor();
    let legacy = r#"package temporal:nexus@1.0.0;

world system {
  export test-service;
}

interface test-service {
  /// @nexus.proto "acme.oneof.v1.Mixed"
  record mixed {
    id: string,
    text: option<string>,
    count: option<s32>,
  }

  choose: func(request: mixed) -> mixed;
}
"#;
    let legacy_path = write_temp_wit("existing-legacy-oneof", legacy);
    let error =
        add_rpc_to_string(&[descriptor.clone()], "TestService.Choose", &[legacy_path]).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("represents this oneof as individual fields")
    );

    let without_operation = legacy.replace("  choose: func(request: mixed) -> mixed;\n", "");
    let new_operation_path = write_temp_wit("new-operation-legacy-oneof", &without_operation);
    let error = add_rpc_to_string(
        &[descriptor.clone()],
        "TestService.Choose",
        &[new_operation_path],
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("represents this oneof as individual fields")
    );

    let add_message_path = write_temp_wit("add-message-legacy-oneof", legacy);
    let error = add_message_to_string(
        &[descriptor.clone()],
        "acme.oneof.v1.Mixed",
        &[add_message_path],
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("represents this oneof as individual fields")
    );

    let linked_collection = unique_temp_dir("linked-legacy-oneof");
    let linked_package = linked_collection.join("oneof-model");
    fs::create_dir_all(&linked_package).unwrap();
    fs::write(
        linked_package.join("model.wit"),
        r#"package acme:oneof-model@1.0.0;

interface model {
  /// @nexus.proto "acme.oneof.v1.Mixed"
  record mixed {
    id: string,
    text: option<string>,
    count: option<s32>,
  }
}
"#,
    )
    .unwrap();
    let error = add_message_to_string(
        &[descriptor.clone()],
        "acme.oneof.v1.Mixed",
        &[linked_collection],
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("represents this oneof as individual fields")
    );

    let pure_record = r#"package temporal:nexus@1.0.0;

world system {
  export test-service;
}

interface test-service {
  /// @nexus.proto "acme.oneof.v1.Root.Outcome"
  record root-outcome {
    success: option<string>,
    failure: option<failure-detail>,
  }

  /// @nexus.proto "acme.oneof.v1.FailureDetail"
  record failure-detail {
  }

  complete: func(request: root-outcome) -> root-outcome;
}
"#;
    let pure_record_path = write_temp_wit("existing-pure-oneof-record", pure_record);
    let error = add_rpc_to_string(
        &[descriptor.clone()],
        "TestService.Complete",
        &[pure_record_path.clone()],
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("represents this oneof as individual fields")
    );

    let error = add_message_to_string(
        &[descriptor],
        "acme.oneof.v1.Root.Outcome",
        &[pure_record_path],
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("represents this oneof as individual fields")
    );
}

#[test]
fn add_message_rejects_malformed_and_colliding_oneofs() {
    let write_set = |name: &str, message: DescriptorProto| {
        let set = FileDescriptorSet {
            file: vec![FileDescriptorProto {
                name: Some(format!("acme/oneof/v1/{name}.proto")),
                package: Some("acme.oneof.v1".to_string()),
                syntax: Some("proto3".to_string()),
                message_type: vec![message],
                ..Default::default()
            }],
        };
        let temp_dir = unique_temp_dir(name);
        fs::create_dir_all(&temp_dir).unwrap();
        let path = temp_dir.join("api.bin");
        fs::write(&path, set.encode_to_vec()).unwrap();
        path
    };

    let collision = write_set(
        "collision",
        DescriptorProto {
            name: Some("Collision".to_string()),
            field: vec![
                proto_field("foo_bar", 1, Type::String, None, Some(0)),
                proto_field("fooBar", 2, Type::String, None, Some(0)),
            ],
            oneof_decl: vec![OneofDescriptorProto {
                name: Some("result".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        },
    );
    let error = add_message_to_string(&[collision], "Collision", &[]).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("variant case `foo-bar` would collide")
    );

    let malformed = write_set(
        "malformed",
        DescriptorProto {
            name: Some("Malformed".to_string()),
            field: vec![proto_field("value", 1, Type::String, None, Some(3))],
            oneof_decl: vec![OneofDescriptorProto {
                name: Some("result".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        },
    );
    let error = add_message_to_string(&[malformed], "Malformed", &[]).unwrap_err();
    assert!(error.to_string().contains("unknown oneof index 3"));
}
