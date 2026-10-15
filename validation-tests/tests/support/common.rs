use std::fs;
use std::path::{Path, PathBuf};

pub fn json_input_path(root: &Path, example_id: &str) -> PathBuf {
    let input_root = root.join("samples/schemas");
    let dir_path = input_root.join(example_id);
    if dir_path.is_dir() {
        return dir_path;
    }
    for extension in ["yaml", "yml", "json"] {
        for stem in [example_id.to_string(), format!("{example_id}.nexusrpc")] {
            let path = input_root.join(format!("{stem}.{extension}"));
            if path.is_file() {
                return path;
            }
        }
    }
    input_root.join(format!("{example_id}.yaml"))
}

pub fn wit_input_path(root: &Path, example_id: &str) -> PathBuf {
    fn collect(path: &Path, inputs: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                if path.file_name().is_some_and(|name| name == "deps") {
                    continue;
                }
                collect(&path, inputs);
            } else if path.extension().is_some_and(|extension| extension == "wit") {
                inputs.push(path);
            }
        }
    }
    let mut inputs = Vec::new();
    collect(&root.join("advanced/samples/inputs"), &mut inputs);
    inputs
        .into_iter()
        .find(|path| path.file_stem().is_some_and(|stem| stem == example_id))
        .expect("requested WIT example must exist")
}

pub fn write_bare_ref_alias_closure(root: &Path) -> PathBuf {
    let input = root.join("input");
    fs::create_dir_all(input.join("target")).unwrap();
    fs::create_dir_all(input.join("alias")).unwrap();
    fs::write(
        input.join("target/main.yaml"),
        r##"$schema: https://json-schema.org/draft/2020-12/schema
type: object
additionalProperties: false
required: [value]
properties: { value: { type: string } }
$defs:
  Mirror: { $ref: "#" }
"##,
    )
    .unwrap();
    fs::write(
        input.join("alias/alternate.yaml"),
        r#"$schema: https://json-schema.org/draft/2020-12/schema
$ref: ../target/main.yaml#
"#,
    )
    .unwrap();
    fs::write(
        input.join("service.nexusrpc.yaml"),
        r#"$schema: https://json-schema.org/draft/2020-12/schema
nexusrpc: "1.0.0"
services:
  AliasService:
    operations:
      echo:
        input: { $ref: "alias/alternate.yaml#" }
        output: { $ref: "alias/alternate.yaml#" }
$defs:
  Holder:
    type: object
    additionalProperties: false
    required: [item]
    properties:
      item: { $ref: "alias/alternate.yaml#" }
"#,
    )
    .unwrap();
    input
}
