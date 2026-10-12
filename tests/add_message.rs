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
use nexgen::{
    AddRpcRequest, add_message_to_string, add_messages_to_string, add_rpc_to_file,
    add_rpc_to_string,
};
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

const TREE_WIT: &str = r#"package acme:tree@1.0.0;

world system {
  export tree-service;
}

interface tree-service {
  /// @nexus.proto "acme.tree.v1.Request"
  record request {
    child: option<child>,
  }

  /// @nexus.proto "acme.tree.v1.Response"
  record response {
    child: option<child>,
  }

  /// @nexus.proto "acme.tree.v1.Child"
  record child {
    label: string,
  }

  run: func(request: request) -> response;
}
"#;

fn write_tree_descriptor(child_has_label: bool) -> PathBuf {
    let child = DescriptorProto {
        name: Some("Child".to_string()),
        field: [
            child_has_label.then(|| proto_field("label", 1, Type::String, None, None)),
            Some(proto_field(
                "grandchild",
                2,
                Type::Message,
                Some(".acme.tree.v1.Grandchild"),
                None,
            )),
        ]
        .into_iter()
        .flatten()
        .collect(),
        ..Default::default()
    };
    let root = |name: &str| DescriptorProto {
        name: Some(name.to_string()),
        field: vec![proto_field(
            "child",
            1,
            Type::Message,
            Some(".acme.tree.v1.Child"),
            None,
        )],
        ..Default::default()
    };
    let method = |name: &str| MethodDescriptorProto {
        name: Some(name.to_string()),
        input_type: Some(".acme.tree.v1.Request".to_string()),
        output_type: Some(".acme.tree.v1.Response".to_string()),
        ..Default::default()
    };
    let set = FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: Some("acme/tree/v1/service.proto".to_string()),
            package: Some("acme.tree.v1".to_string()),
            syntax: Some("proto3".to_string()),
            message_type: vec![
                root("Request"),
                root("Response"),
                child,
                DescriptorProto {
                    name: Some("Grandchild".to_string()),
                    field: vec![proto_field("code", 1, Type::String, None, None)],
                    ..Default::default()
                },
            ],
            service: vec![ServiceDescriptorProto {
                name: Some("TreeService".to_string()),
                method: vec![method("Run"), method("Inspect")],
                ..Default::default()
            }],
            ..Default::default()
        }],
    };
    let directory = unique_temp_dir("tree-descriptor");
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("api.bin");
    fs::write(&path, set.encode_to_vec()).unwrap();
    path
}

fn record_block<'a>(wit: &'a str, name: &str) -> &'a str {
    let start = wit.find(&format!("  record {name} {{")).unwrap();
    let end = start + wit[start..].find("\n  }").unwrap() + "\n  }".len();
    &wit[start..end]
}

#[test]
fn scaffold_commands_reconcile_the_same_transitive_request_and_response_models() {
    let descriptor = write_tree_descriptor(true);
    let input = write_temp_wit("tree-partial", TREE_WIT);
    let by_message = add_messages_to_string(
        &[descriptor.clone()],
        &[
            "acme.tree.v1.Request".into(),
            "acme.tree.v1.Response".into(),
        ],
        &[],
        &[input.clone()],
    )
    .unwrap();
    let by_new_rpc = add_rpc_to_string(
        &[descriptor.clone()],
        "TreeService.Inspect",
        &[input.clone()],
    )
    .unwrap();
    let by_existing_rpc =
        add_rpc_to_string(&[descriptor.clone()], "TreeService.Run", &[input]).unwrap();

    for rendered in [&by_message, &by_new_rpc, &by_existing_rpc] {
        for name in ["request", "response", "child", "grandchild"] {
            assert_eq!(rendered.matches(&format!("record {name} {{")).count(), 1);
        }
        assert!(record_block(rendered, "child").contains("grandchild: option<grandchild>,"));
        assert!(record_block(rendered, "grandchild").contains("code: string,"));
        assert!(record_block(rendered, "child").contains("label: string,"));
        assert!(rendered.contains("run: func(request: request) -> response;"));
    }
    for name in ["request", "response", "child", "grandchild"] {
        assert_eq!(
            record_block(&by_message, name),
            record_block(&by_new_rpc, name)
        );
        assert_eq!(
            record_block(&by_message, name),
            record_block(&by_existing_rpc, name)
        );
    }
    assert_eq!(by_message.matches("inspect: func(").count(), 0);
    assert_eq!(by_existing_rpc.matches("inspect: func(").count(), 0);
    assert_eq!(by_new_rpc.matches("inspect: func(").count(), 1);

    for (name, rendered) in [
        ("message", &by_message),
        ("new-rpc", &by_new_rpc),
        ("existing-rpc", &by_existing_rpc),
    ] {
        let completed = write_temp_wit(&format!("tree-completed-{name}"), rendered);
        let again = match name {
            "message" => add_messages_to_string(
                &[descriptor.clone()],
                &[
                    "acme.tree.v1.Request".into(),
                    "acme.tree.v1.Response".into(),
                ],
                &[],
                &[completed],
            )
            .unwrap(),
            "new-rpc" => {
                add_rpc_to_string(&[descriptor.clone()], "TreeService.Inspect", &[completed])
                    .unwrap()
            }
            _ => add_rpc_to_string(&[descriptor.clone()], "TreeService.Run", &[completed]).unwrap(),
        };
        assert_eq!(&again, rendered, "{name} must be idempotent");
    }
}

#[test]
fn scaffold_commands_reject_incompatible_or_removed_transitive_child_fields() {
    for (name, descriptor, source) in [
        (
            "incompatible",
            write_tree_descriptor(true),
            TREE_WIT.replace("label: string,", "label: bool,"),
        ),
        (
            "removed",
            write_tree_descriptor(false),
            TREE_WIT.to_string(),
        ),
    ] {
        let input = write_temp_wit(&format!("tree-{name}"), &source);
        let errors = [
            add_message_to_string(
                &[descriptor.clone()],
                "acme.tree.v1.Request",
                &[input.clone()],
            )
            .unwrap_err()
            .to_string(),
            add_rpc_to_string(
                &[descriptor.clone()],
                "TreeService.Inspect",
                &[input.clone()],
            )
            .unwrap_err()
            .to_string(),
            add_rpc_to_string(&[descriptor], "TreeService.Run", &[input])
                .unwrap_err()
                .to_string(),
        ];
        for error in errors {
            assert!(
                error.contains("acme.tree.v1.Child") && error.contains("label"),
                "{name}: {error}"
            );
        }
    }
}

#[test]
fn scaffold_reconciliation_stops_at_omitted_and_opaque_fields() {
    let descriptor = write_tree_descriptor(true);
    let omitted = TREE_WIT.replace(
        "    child: option<child>,",
        "    /// @nexus.omit\n    child: option<child>,",
    );
    let opaque = TREE_WIT.replace(
        "  record child {\n    label: string,\n  }",
        "  type child = string;",
    );
    for source in [omitted, opaque] {
        let path = write_temp_wit("tree-substitution-boundary", &source);
        let by_message = add_message_to_string(
            &[descriptor.clone()],
            "acme.tree.v1.Request",
            &[path.clone()],
        )
        .unwrap();
        let by_rpc = add_rpc_to_string(&[descriptor.clone()], "TreeService.Run", &[path]).unwrap();
        assert_eq!(by_message, source);
        assert_eq!(by_rpc, source);
    }
}

#[test]
fn scaffold_reconciliation_rejects_unrepresentable_cycles_without_writing_output() {
    let descriptor = write_tree_descriptor(true);
    let mut set = FileDescriptorSet::decode(fs::read(&descriptor).unwrap().as_slice()).unwrap();
    set.file[0].message_type[3].field.push(proto_field(
        "parent",
        2,
        Type::Message,
        Some(".acme.tree.v1.Request"),
        None,
    ));
    fs::write(&descriptor, set.encode_to_vec()).unwrap();
    let path = write_temp_wit("tree-cycle", TREE_WIT);
    let output_path = path.with_file_name("output.wit");
    fs::write(&output_path, "leave existing output alone").unwrap();
    let error = add_rpc_to_file(&AddRpcRequest {
        descriptor_paths: vec![descriptor.clone()],
        rpc_name: "TreeService.Run".to_string(),
        input_paths: vec![path.clone()],
        output_path: Some(output_path.clone()),
    })
    .unwrap_err();
    assert!(error.to_string().contains("depends on itself"));
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "leave existing output alone"
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), TREE_WIT);
    let error = add_message_to_string(&[descriptor], "acme.tree.v1.Request", &[path]).unwrap_err();
    assert!(error.to_string().contains("depends on itself"));
}

#[test]
fn scaffold_reconciliation_checks_linked_records_without_importing_their_children() {
    let descriptor = write_tree_descriptor(true);
    let dependency = r#"package acme:models@1.0.0;

interface model {
  /// @nexus.proto "acme.tree.v1.Child"
  record child {
    label: string,
    grandchild: option<grandchild>,
  }
  /// @nexus.proto "acme.tree.v1.Grandchild"
  record grandchild {
    code: string,
  }
}
"#;
    let source = TREE_WIT.replace(
        "  /// @nexus.proto \"acme.tree.v1.Child\"\n  record child {\n    label: string,\n  }",
        "  use acme:models/model@1.0.0.{child};",
    );
    let path = write_temp_wit("tree-linked", &source);
    let linked = write_temp_wit("tree-linked-models", dependency);
    assert_eq!(
        add_rpc_to_string(
            &[descriptor.clone()],
            "TreeService.Run",
            &[path.clone(), linked.clone()],
        )
        .unwrap(),
        source,
    );
    let missing = dependency.replace("    code: string,\n", "");
    fs::write(&linked, &missing).unwrap();
    let error = add_message_to_string(
        &[descriptor],
        "acme.tree.v1.Request",
        &[path, linked.clone()],
    )
    .unwrap_err();
    assert!(error.to_string().contains("linked WIT record `grandchild`"));
    assert!(error.to_string().contains("missing descriptor fields"));
    assert_eq!(fs::read_to_string(linked).unwrap(), missing);
}

#[test]
fn rpc_reconciliation_edits_records_only_in_the_target_interface() {
    let descriptor = write_tree_descriptor(true);
    let other = "\ninterface other {\n  record child {\n    unrelated: bool,\n  }\n}\n";
    let source = TREE_WIT.replace(
        "interface tree-service {",
        &format!("{other}\ninterface tree-service {{"),
    );
    let path = write_temp_wit("tree-other-interface", &source);
    let rendered = add_rpc_to_string(&[descriptor], "TreeService.Run", &[path]).unwrap();
    assert!(rendered.contains(other));
    let target = rendered.split("interface tree-service {").nth(1).unwrap();
    assert!(record_block(target, "child").contains("grandchild: option<grandchild>,"));
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
fn add_message_combines_names_and_file_selections_without_duplicate_definitions() {
    let descriptor = write_oneof_descriptor();
    let rendered = add_messages_to_string(
        &[descriptor],
        &["acme.oneof.v1.Root".to_string()],
        &["acme/oneof/v1/service.proto".to_string()],
        &[],
    )
    .unwrap();
    assert!(rendered.contains("world system {\n  export messages;\n}"));
    assert_eq!(rendered.matches("record root {").count(), 1);
    assert_eq!(rendered.matches("record mixed {").count(), 1);
    assert_eq!(rendered.matches("record multiple {").count(), 1);
    assert_eq!(rendered.matches("record failure-detail {").count(), 1);
    assert_eq!(rendered.matches("record root-outcome {").count(), 1);
    assert!(rendered.find("record failure-detail {") < rendered.find("record root {"));
}

#[test]
fn cli_add_message_accepts_repeated_messages_and_proto_file() {
    let descriptor = write_oneof_descriptor();
    let output = Command::new(env!("CARGO_BIN_EXE_nexgen"))
        .args([
            "add-message",
            "--descriptors",
            descriptor.to_str().unwrap(),
            "--message",
            "acme.oneof.v1.FailureDetail",
            "--message",
            "acme.oneof.v1.Mixed",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let rendered = String::from_utf8(output.stdout).unwrap();
    assert!(rendered.contains("record failure-detail {"));
    assert!(rendered.contains("record mixed {"));
    assert!(!rendered.contains("record multiple {"));

    let output = Command::new(env!("CARGO_BIN_EXE_nexgen"))
        .args([
            "add-message",
            "--descriptors",
            descriptor.to_str().unwrap(),
            "--file",
            "acme/oneof/v1/service.proto",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("record multiple {")
    );
}

#[test]
fn add_message_rejects_unknown_proto_file() {
    let error = add_messages_to_string(
        &[write_oneof_descriptor()],
        &[],
        &["missing.proto".to_string()],
        &[],
    )
    .unwrap_err();
    assert!(error.to_string().contains("missing.proto"));
}

#[test]
fn add_message_inserts_multiple_roots_into_one_existing_interface() {
    let input_path = write_temp_wit(
        "add-message-multiple",
        "package temporal:nexus@1.0.0;\n\nworld system {\n  export models;\n}\n\ninterface models {\n}\n",
    );
    let rendered = add_messages_to_string(
        &[write_oneof_descriptor()],
        &[
            "acme.oneof.v1.Mixed".to_string(),
            "acme.oneof.v1.Multiple".to_string(),
        ],
        &[],
        &[input_path],
    )
    .unwrap();
    assert_eq!(rendered.matches("interface models {").count(), 1);
    assert!(rendered.contains("record mixed {"));
    assert!(rendered.contains("record multiple {"));
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
fn add_message_restores_a_missing_record_and_missing_fields_in_one_pass() {
    let input_path = write_temp_wit(
        "add-message-partial-file",
        r#"package temporal:nexus@1.0.0;

world system {
  export models;
}

interface models {
  /// @nexus.proto "acme.oneof.v1.Mixed"
  record mixed {
    choice: option<mixed-choice>,
  }

  /// @nexus.proto "acme.oneof.v1.Mixed.choice"
  variant mixed-choice {
    text(string),
    count(s32),
  }
}
"#,
    );
    // File-wide selection includes an existing record with a missing field
    // and other messages whose entire declarations are absent.
    let rendered = add_messages_to_string(
        &[write_oneof_descriptor()],
        &[],
        &["acme/oneof/v1/service.proto".to_string()],
        &[input_path.clone()],
    )
    .unwrap();
    assert!(rendered.contains("record failure-detail {"));
    assert_eq!(rendered.matches("record mixed {").count(), 1);
    assert_eq!(rendered.matches("id: string,").count(), 1);
    assert_ne!(rendered, fs::read_to_string(input_path).unwrap());
    assert_eq!(
        add_messages_to_string(
            &[write_oneof_descriptor()],
            &[],
            &["acme/oneof/v1/service.proto".to_string()],
            &[write_temp_wit("add-message-idempotent", &rendered)],
        )
        .unwrap(),
        rendered
    );
}

#[test]
fn add_message_reconciles_missing_proto_field_without_replacing_type_parameters() {
    let root = project_root();
    let source =
        fs::read_to_string(root.join("advanced/samples/inputs/notification-service.wit")).unwrap();
    let input_path = write_temp_wit(
        "add-message-removed-source-context",
        &source.replace("    source-context: source-context-t,\n", ""),
    );
    let rendered = add_message_to_string(
        &[descriptor_path(&root)],
        "temporal.api.notificationservice.v1.OnCompleteRequest",
        &[input_path, linked_inputs_path(&root)],
    )
    .unwrap();
    assert!(rendered.contains("    source-context: "));
    assert!(rendered.contains("success(output-t)"));
    assert!(rendered.contains("%result: on-complete-request-result"));
    assert_ne!(rendered, source);
    let completed_path = write_temp_wit("add-message-refined-idempotent", &rendered);
    assert_eq!(
        add_message_to_string(
            &[descriptor_path(&root)],
            "temporal.api.notificationservice.v1.OnCompleteRequest",
            &[completed_path, linked_inputs_path(&root)],
        )
        .unwrap(),
        rendered
    );
    // The untouched authored refinement remains stable.
    let unchanged = add_message_to_string(
        &[descriptor_path(&root)],
        "temporal.api.notificationservice.v1.OnCompleteRequest",
        &[
            root.join("advanced/samples/inputs/notification-service.wit"),
            linked_inputs_path(&root),
        ],
    )
    .unwrap();
    assert_eq!(unchanged, source);
}

#[test]
fn add_message_rejects_an_incompatible_existing_field() {
    let input_path = write_temp_wit(
        "add-message-incompatible",
        r#"package temporal:nexus@1.0.0;

world system {
  export models;
}

interface models {
  /// @nexus.proto "acme.oneof.v1.Mixed"
  record mixed {
    id: bool,
    choice: option<mixed-choice>,
  }

  /// @nexus.proto "acme.oneof.v1.Mixed.choice"
  variant mixed-choice {
    text(string),
    count(s32),
  }
}
"#,
    );
    let error = add_message_to_string(
        &[write_oneof_descriptor()],
        "acme.oneof.v1.Mixed",
        &[input_path],
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("existing WIT field is `id: bool`")
    );
    assert!(error.to_string().contains("add-message"));
}

#[test]
fn add_message_rejects_an_incompatible_existing_oneof_mapping() {
    let input_path = write_temp_wit(
        "add-message-incompatible-oneof",
        r#"package temporal:nexus@1.0.0;

world system {
  export models;
}

interface models {
  /// @nexus.proto "acme.oneof.v1.Mixed"
  record mixed {
    id: string,
    choice: string,
  }
}
"#,
    );
    let error = add_message_to_string(
        &[write_oneof_descriptor()],
        "acme.oneof.v1.Mixed",
        &[input_path],
    )
    .unwrap_err();
    assert!(error.to_string().contains("existing WIT oneof field"));
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
fn scaffold_commands_restore_missing_oneof_fields_and_preserve_custom_variants() {
    let descriptor = write_oneof_descriptor();
    let existing_variant = r#"  /// @nexus.proto "acme.oneof.v1.Mixed.choice"
  variant custom-choice {
    text(string),
    count(s32),
  }
"#;
    for variant in ["", existing_variant] {
        let source = format!(
            "package acme:oneofs@1.0.0;\n\nworld system {{ export test-service; }}\n\ninterface test-service {{\n{variant}  /// @nexus.proto \"acme.oneof.v1.Mixed\"\n  record mixed {{ id: string, }}\n  choose: func(request: mixed) -> mixed;\n}}\n"
        );
        let path = write_temp_wit("missing-oneof-group", &source);
        for rendered in [
            add_message_to_string(
                &[descriptor.clone()],
                "acme.oneof.v1.Mixed",
                &[path.clone()],
            )
            .unwrap(),
            add_rpc_to_string(&[descriptor.clone()], "TestService.Choose", &[path.clone()])
                .unwrap(),
        ] {
            let name = if variant.is_empty() {
                "mixed-choice"
            } else {
                "custom-choice"
            };
            assert!(rendered.contains(&format!("choice: option<{name}>,")));
            assert_eq!(rendered.matches("variant ").count(), 1);
            let complete = write_temp_wit("missing-oneof-group-complete", &rendered);
            assert_eq!(
                add_rpc_to_string(&[descriptor.clone()], "TestService.Choose", &[complete])
                    .unwrap(),
                rendered,
            );
        }
    }
}

#[test]
fn missing_records_reuse_owned_and_linked_oneof_variants() {
    let descriptor = write_oneof_descriptor();
    let variant = r#"  /// @nexus.proto "acme.oneof.v1.Mixed.choice"
  variant shared-choice { text(string), count(s32), }
"#;
    let dependency = format!("package acme:choices@1.0.0;\n\ninterface model {{\n{variant}}}\n");
    let linked = write_temp_wit("reused-linked-variant", &dependency);
    for (local_variant, inputs) in [(variant, vec![]), ("", vec![linked.clone()])] {
        let source = format!(
            "package acme:oneofs@1.0.0;\n\nworld system {{ export test-service; }}\n\ninterface test-service {{\n{local_variant}}}\n"
        );
        let path = write_temp_wit("missing-record-with-variant", &source);
        let mut inputs = inputs;
        inputs.insert(0, path);
        for rendered in [
            add_message_to_string(&[descriptor.clone()], "acme.oneof.v1.Mixed", &inputs).unwrap(),
            add_rpc_to_string(&[descriptor.clone()], "TestService.Choose", &inputs).unwrap(),
        ] {
            assert!(rendered.contains("record mixed {"));
            assert!(rendered.contains("choice: option<shared-choice>,"));
            assert_eq!(
                rendered.matches("variant ").count(),
                if local_variant.is_empty() { 0 } else { 1 }
            );
            if local_variant.is_empty() {
                assert!(
                    rendered.contains("use acme:choices/model@1.0.0.{\n    shared-choice,\n  };")
                );
            }
            let completed = write_temp_wit("missing-record-with-variant-complete", &rendered);
            inputs[0] = completed;
            assert_eq!(
                add_message_to_string(&[descriptor.clone()], "acme.oneof.v1.Mixed", &inputs)
                    .unwrap(),
                rendered,
            );
        }
    }
}

#[test]
fn local_unmapped_oneof_cannot_borrow_a_linked_variants_mapping() {
    let descriptor = write_oneof_descriptor();
    let source = r#"package acme:oneofs@1.0.0;

world system { export test-service; }

interface test-service {
  variant mixed-choice { unrelated(bool), }
  /// @nexus.proto "acme.oneof.v1.Mixed"
  record mixed { id: string, choice: option<mixed-choice>, }
  choose: func(request: mixed) -> mixed;
}
"#;
    let dependency = r#"package acme:linked-oneofs@1.0.0;

interface model {
  /// @nexus.proto "acme.oneof.v1.Mixed.choice"
  variant mixed-choice { text(string), count(s32), }
}
"#;
    let linked = write_temp_wit("shadowed-oneof-dependency", dependency);
    for source in [
        source.to_string(),
        source.replace(
            "variant mixed-choice { unrelated(bool), }",
            "type mixed-choice = string;",
        ),
    ] {
        let path = write_temp_wit("shadowed-oneof", &source);
        let error = add_rpc_to_string(
            &[descriptor.clone()],
            "TestService.Choose",
            &[path, linked.clone()],
        )
        .unwrap_err();
        assert!(error.to_string().contains("must refer to a mapped variant"));
    }
}

#[test]
fn scaffolding_rejects_oneof_members_with_unrepresentable_wire_names() {
    let descriptor = write_oneof_descriptor();
    let mut set = FileDescriptorSet::decode(fs::read(&descriptor).unwrap().as_slice()).unwrap();
    set.file[0].message_type[1].field[1].name = Some("camelText".to_string());
    fs::write(&descriptor, set.encode_to_vec()).unwrap();
    let error = add_message_to_string(&[descriptor], "acme.oneof.v1.Mixed", &[]).unwrap_err();
    assert!(error.to_string().contains("cannot be represented"));
    assert!(error.to_string().contains("camelText"));
}

#[test]
fn generated_field_names_preserve_their_proto_mapping_on_reconciliation() {
    let descriptor = write_tree_descriptor(true);
    let mut set = FileDescriptorSet::decode(fs::read(&descriptor).unwrap().as_slice()).unwrap();
    set.file[0].message_type[3].field[0].name = Some("statusCode".to_string());
    fs::write(&descriptor, set.encode_to_vec()).unwrap();
    let rendered = add_rpc_to_string(&[descriptor.clone()], "TreeService.Run", &[]).unwrap();
    assert!(rendered.contains("/// @nexus.proto-field \"statusCode\""));
    let path = write_temp_wit("camelcase-field", &rendered);
    assert_eq!(
        add_rpc_to_string(&[descriptor], "TreeService.Run", &[path]).unwrap(),
        rendered,
    );
}

#[test]
fn scaffold_commands_preserve_explicit_compatible_repeated_field_substitutions() {
    let descriptor = write_tree_descriptor(true);
    let mut set = FileDescriptorSet::decode(fs::read(&descriptor).unwrap().as_slice()).unwrap();
    set.file[0].message_type[0].field[0].label = Some(Label::Repeated as i32);
    fs::write(&descriptor, set.encode_to_vec()).unwrap();
    let source = TREE_WIT
        .replacen("child: option<child>,", "child: list<refined>,", 1)
        .replace(
            "interface tree-service {",
            "interface tree-service {\n  use acme:refinements/model@1.0.0.{refined};",
        );
    let dependency = r#"package acme:refinements@1.0.0;

interface model {
  /// @nexus.add-rpc-compatible-with "child"
  type refined = string;
}
"#;
    let path = write_temp_wit("compatible-repeated", &source);
    let linked = write_temp_wit("compatible-repeated-dependency", dependency);
    // Only Request is a root: its explicit substitution must not traverse Child.
    assert_eq!(
        add_message_to_string(&[descriptor], "acme.tree.v1.Request", &[path, linked],).unwrap(),
        source,
    );
}

#[test]
fn scaffold_commands_reject_reused_oneof_variant_case_drift() {
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
    let descriptor = write_oneof_descriptor();
    for (name, contents) in [
        ("missing-case", grouped.replace("    count(s32),\n", "")),
        (
            "extra-case",
            grouped.replace("    count(s32),\n", "    count(s32),\n    unknown(bool),\n"),
        ),
        (
            "incompatible-payload",
            grouped.replace("    count(s32),\n", "    count(string),\n"),
        ),
    ] {
        let path = write_temp_wit(name, &contents);
        for error in [
            add_message_to_string(
                &[descriptor.clone()],
                "acme.oneof.v1.Mixed",
                &[path.clone()],
            )
            .unwrap_err()
            .to_string(),
            add_rpc_to_string(&[descriptor.clone()], "TestService.Choose", &[path.clone()])
                .unwrap_err()
                .to_string(),
        ] {
            assert!(
                error.contains("acme.oneof.v1.Mixed.choice"),
                "{name}: {error}"
            );
        }
    }
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

    generate_source(Language::Python, spec, &descriptors, "unused_support").unwrap();
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
