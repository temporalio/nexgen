//! A WIT `list<u8>` field of a `@nexus.proto` record generates as proto `bytes`.

use std::fs;
use std::path::PathBuf;

use nexgen::language::Language;
use nexgen::{GenerateRequest, generate_to_file};

/// `add-rpc` scaffolds proto `bytes` as `list<u8>`.
const PROTO_BYTES_WIT: &str = r#"package temporal:nexus@1.0.0;

world system {
  export callbacks;
}

interface callbacks {
  /// @nexus.proto "temporal.api.common.v1.Callback.Internal"
  record callback-internal {
    data: list<u8>,
  }
}
"#;

fn generate_python(wit: &str) -> Result<String, String> {
    let descriptors = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("advanced/samples/descriptors/temporal_api.bin");
    let temp_dir = tempfile::tempdir().unwrap();
    let input_path = temp_dir.path().join("system.wit");
    fs::write(&input_path, wit).unwrap();
    let output_path = temp_dir.path().join("generated");
    let result = generate_to_file(&GenerateRequest {
        config: Default::default(),
        language: Language::Python,
        input_paths: vec![input_path],
        support_package: Some("temporal_support".to_string()),
        descriptor_paths: vec![descriptors],
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    });
    let mut rendered = String::new();
    if result.is_ok() {
        read_all(&output_path, &mut rendered);
    }
    result.map(|_| rendered).map_err(|err| err.to_string())
}

fn read_all(dir: &std::path::Path, out: &mut String) {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            read_all(&path, out);
        } else {
            out.push_str(&fs::read_to_string(&path).unwrap());
        }
    }
}

#[test]
fn python_reads_a_proto_backed_wit_byte_list_as_bytes() {
    let models = generate_python(PROTO_BYTES_WIT).unwrap();
    assert!(models.contains("    data: bytes\n"), "{models}");
}
