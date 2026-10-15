use super::*;

#[test]
fn go_nested_model_encoding_bridges_pointer_arguments() {
    use prost::Message;
    use prost_types::field_descriptor_proto::{Label, Type};
    use prost_types::{
        DescriptorProto, FieldDescriptorProto, FileDescriptorProto, FileDescriptorSet, FileOptions,
        MessageOptions,
    };

    let temp_dir = unique_output_path("go-nested-models");
    fs::create_dir_all(&temp_dir).unwrap();
    let field = |name: &str, number, label: Label, kind: Type, type_name: Option<&str>| {
        FieldDescriptorProto {
            name: Some(name.into()),
            number: Some(number),
            label: Some(label as i32),
            r#type: Some(kind as i32),
            type_name: type_name.map(str::to_string),
            ..Default::default()
        }
    };
    let descriptors = FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: Some("nested.proto".into()),
            package: Some("test".into()),
            syntax: Some("proto3".into()),
            options: Some(FileOptions {
                go_package: Some("go.temporal.io/sdk/advanced/samples/go/wire;wire".into()),
                ..Default::default()
            }),
            message_type: vec![
                DescriptorProto {
                    name: Some("Child".into()),
                    field: vec![field("name", 1, Label::Optional, Type::String, None)],
                    ..Default::default()
                },
                DescriptorProto {
                    name: Some("Parent".into()),
                    field: vec![
                        field(
                            "required",
                            1,
                            Label::Optional,
                            Type::Message,
                            Some(".test.Child"),
                        ),
                        field(
                            "optional",
                            2,
                            Label::Optional,
                            Type::Message,
                            Some(".test.Child"),
                        ),
                        field(
                            "children",
                            3,
                            Label::Repeated,
                            Type::Message,
                            Some(".test.Child"),
                        ),
                        field(
                            "entries",
                            4,
                            Label::Repeated,
                            Type::Message,
                            Some(".test.Parent.EntriesEntry"),
                        ),
                    ],
                    nested_type: vec![DescriptorProto {
                        name: Some("EntriesEntry".into()),
                        field: vec![
                            field("key", 1, Label::Optional, Type::String, None),
                            field(
                                "value",
                                2,
                                Label::Optional,
                                Type::Message,
                                Some(".test.Child"),
                            ),
                        ],
                        options: Some(MessageOptions {
                            map_entry: Some(true),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
    };
    let descriptor_path = temp_dir.join("nested.bin");
    fs::write(&descriptor_path, descriptors.encode_to_vec()).unwrap();
    let input_path = temp_dir.join("nested.wit");
    fs::write(
        &input_path,
        r#"package test:nested;

interface models {
    /// @nexus.proto "test.Child"
    record child {
        name: string,
    }
    /// @nexus.proto "test.Parent"
    record parent {
        required: child,
        optional: option<child>,
        children: option<list<child>>,
        entries: option<map<string, child>>,
    }
    echo: func(request: parent) -> parent;
}

world api {
    export models;
}
"#,
    )
    .unwrap();
    let output_path = temp_dir.join("output");
    generate_to_file(&GenerateRequest {
        config: nexgen::nexgen_config::NexgenConfig {
            mode: nexgen::generator::GenerationMode::NativeApi,
            ..Default::default()
        },
        language: nexgen::language::Language::Go,
        input_paths: vec![input_path],
        support_package: Some("example.com/nexgen/support".into()),
        descriptor_paths: vec![descriptor_path],
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();
    let files = read_go_output_files(&output_path);
    let rendered = files.values().cloned().collect::<Vec<_>>().join("\n");
    assert!(!files.contains_key(&PathBuf::from("support.go")));
    let root = project_root();
    let module = fs::read_to_string(root.join("advanced/samples/go/go.mod")).unwrap();
    // The generated system Nexus factory is SDK-internal.
    fs::write(temp_dir.join("go.mod"), module).unwrap();
    fs::copy(
        root.join("advanced/samples/go/go.sum"),
        temp_dir.join("go.sum"),
    )
    .unwrap();
    fs::create_dir(temp_dir.join("wire")).unwrap();
    // Conversion helpers need only the wire shape, not protobuf serialization.
    fs::write(
        temp_dir.join("wire/wire.go"),
        r#"package wire

type Child struct {
	Name string
}

func (m *Child) GetName() string {
	if m == nil {
		return ""
	}
	return m.Name
}

type Parent struct {
	Required *Child
	Optional *Child
	Children []*Child
	Entries  map[string]*Child
}

func (m *Parent) GetRequired() *Child {
	if m == nil {
		return nil
	}
	return m.Required
}

func (m *Parent) GetOptional() *Child {
	if m == nil {
		return nil
	}
	return m.Optional
}

func (m *Parent) GetChildren() []*Child {
	if m == nil {
		return nil
	}
	return m.Children
}

func (m *Parent) GetEntries() map[string]*Child {
	if m == nil {
		return nil
	}
	return m.Entries
}
"#,
    )
    .unwrap();
    fs::write(
        output_path.join("conversion_test.go"),
        r#"package output

import (
	"reflect"
	"testing"

	"go.temporal.io/sdk/advanced/samples/go/wire"
	"go.temporal.io/sdk/converter"
	commonconverter "go.temporal.io/sdk/internal/common/converter"
)

type childView interface {
	GetName() string
}

type parentView interface {
	GetRequired() Child
	GetOptional() *Child
	GetChildren() []Child
	GetEntries() map[string]Child
}

var _ childView = Child{}
var _ parentView = Parent{}

func TestNestedConversionRoundTrip(t *testing.T) {
	cfg := &commonconverter.SystemNexusConfig{DataConverter: converter.GetDefaultDataConverter()}
	for _, tt := range []struct {
		name  string
		model Parent
		proto *wire.Parent
	}{
		{
			name: "all fields",
			model: Parent{
				Required: Child{Name: "required"},
				Optional: &Child{Name: "optional"},
				Children: []Child{{Name: "first"}, {Name: "second"}},
				Entries:  map[string]Child{"first": {Name: "first"}, "second": {Name: "second"}},
			},
			proto: &wire.Parent{
				Required: &wire.Child{Name: "required"},
				Optional: &wire.Child{Name: "optional"},
				Children: []*wire.Child{{Name: "first"}, {Name: "second"}},
				Entries:  map[string]*wire.Child{"first": {Name: "first"}, "second": {Name: "second"}},
			},
		},
		{
			name:  "required only",
			model: Parent{Required: Child{Name: "required"}},
			proto: &wire.Parent{Required: &wire.Child{Name: "required"}},
		},
	} {
		t.Run(tt.name, func(t *testing.T) {
			var view parentView = tt.model
			getters := []struct {
				name string
				got  any
				want any
			}{
				{"required", view.GetRequired(), tt.model.Required},
				{"optional", view.GetOptional(), tt.model.Optional},
				{"children", view.GetChildren(), tt.model.Children},
				{"entries", view.GetEntries(), tt.model.Entries},
			}
			for _, getter := range getters {
				if !reflect.DeepEqual(getter.got, getter.want) {
					t.Fatalf("%s getter = %#v, want %#v", getter.name, getter.got, getter.want)
				}
			}
			encoded, err := parentToProto(cfg, &tt.model)
			if err != nil {
				t.Fatal(err)
			}
			if !reflect.DeepEqual(encoded, tt.proto) {
				t.Fatalf("encoded = %#v, want %#v", encoded, tt.proto)
			}
			decoded := Parent{
				Required: Child{Name: "stale"},
				Optional: &Child{Name: "stale"},
				Children: []Child{{Name: "stale"}},
				Entries:  map[string]Child{"stale": {Name: "stale"}},
			}
			if err := parentFromProto(cfg, encoded, &decoded); err != nil {
				t.Fatal(err)
			}
			if !reflect.DeepEqual(decoded, tt.model) {
				t.Fatalf("decoded = %#v, want %#v", decoded, tt.model)
			}
		})
	}
}
"#,
    )
    .unwrap();
    let status = Command::new("go")
        .args(["test", "./output"])
        .current_dir(&temp_dir)
        .status()
        .unwrap();
    fs::remove_dir_all(temp_dir).unwrap();
    assert!(status.success(), "standalone nested Go model tests failed");

    assert!(
        rendered.contains(
            "func childToProto(cfg *commonconverter.SystemNexusConfig, m *Child) (*wire.Child, error) {"
        ),
        "{rendered}"
    );
    assert!(rendered.contains(
        "func childFromProto(cfg *commonconverter.SystemNexusConfig, proto *wire.Child, out *Child) error {"
    ));
    assert!(rendered.contains("converted, err := childToProto(cfg, &m.Required)"));
    assert!(
        rendered.contains(
            "if m.Optional != nil {\n\t\tconverted, err := childToProto(cfg, m.Optional)"
        )
    );
    assert!(rendered.contains(
        "for _, item := range m.Children {\n\t\tconverted, err := childToProto(cfg, &item)"
    ));
    assert!(
        rendered.contains(
            "for k, v := range m.Entries {\n\t\t\tconverted, err := childToProto(cfg, &v)"
        )
    );
    assert!(rendered.contains(
        "if proto.GetOptional() != nil {\n\t\tout.Optional = new(Child)\n\t\tif err := childFromProto(cfg, proto.GetOptional(), out.Optional); err != nil {"
    ));
    assert!(rendered.contains("out.Optional = new(Child)"));
    assert!(rendered.contains(
        "if err := childFromProto(cfg, proto.GetRequired(), &out.Required); err != nil {"
    ));
    assert!(rendered.contains(
        "for _, item := range proto.GetChildren() {\n\t\tvar converted Child\n\t\tif err := childFromProto(cfg, item, &converted); err != nil {"
    ));
    assert!(rendered.contains(
        "for k, v := range proto.GetEntries() {\n\t\t\tvar converted Child\n\t\t\tif err := childFromProto(cfg, v, &converted); err != nil {"
    ));
    assert!(rendered.contains("\t*out = Parent{}\n"));
    assert!(rendered.contains("out.Children = append(out.Children, converted)"));
    assert!(rendered.contains("out.Entries[k] = converted"));
    assert!(!rendered.contains("return value, err"));
    assert!(!rendered.contains(".toProto("));
}

#[test]
fn go_model_decode_uses_direct_output_and_shared_support_callbacks() {
    let root = project_root();
    let temp_dir = unique_output_path("go-model-decode");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("api.wit");
    fs::write(
        &input_path,
        r#"package test:decode;
interface service {
    use nexus:temporal-types/model@1.0.0.{user-metadata};
    echo: func(request: user-metadata) -> user-metadata;
}
world api {
    export service;
}
"#,
    )
    .unwrap();
    let output_path = temp_dir.join("output");
    generate_to_file(&GenerateRequest {
        config: nexgen::nexgen_config::NexgenConfig {
            mode: nexgen::generator::GenerationMode::NativeApi,
            ..Default::default()
        },
        language: nexgen::language::Language::Go,
        input_paths: vec![input_path, root.join("advanced/samples/inputs/deps")],
        support_package: Some("go.temporal.io/sdk/advanced/samples/go/support".into()),
        descriptor_paths: vec![descriptor_path(&root)],
        output_path: output_path.clone(),
        format: true,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();
    for name in ["go.mod", "go.sum"] {
        fs::copy(
            root.join("advanced/samples/go").join(name),
            temp_dir.join(name),
        )
        .unwrap();
    }
    fs::create_dir(temp_dir.join("support")).unwrap();
    fs::copy(
        root.join("advanced/samples/go/support/temporal_model_converters.go"),
        temp_dir.join("support/temporal_model_converters.go"),
    )
    .unwrap();
    fs::write(
        output_path.join("conversion_test.go"),
        r#"package output

import (
	"strings"
	"testing"

	common "go.temporal.io/api/common/v1"
	sdk "go.temporal.io/api/sdk/v1"
	"go.temporal.io/sdk/converter"
	commonconverter "go.temporal.io/sdk/internal/common/converter"
)

func TestUserMetadataConversion(t *testing.T) {
	dc := converter.NewCodecDataConverter(
		converter.GetDefaultDataConverter(),
		converter.NewZlibCodec(converter.ZlibCodecOptions{AlwaysEncode: true}),
	)
	cfg := &commonconverter.SystemNexusConfig{DataConverter: dc}
	want := UserMetadata{StaticSummary: "summary", StaticDetails: "details"}
	message, err := userMetadataToProto(cfg, &want)
	if err != nil {
		t.Fatalf("encode user metadata: %v", err)
	}
	for name, payload := range map[string]*common.Payload{
		"summary": message.GetSummary(),
		"details": message.GetDetails(),
	} {
		if got := string(payload.GetMetadata()[converter.MetadataEncoding]); got != "binary/zlib" {
			t.Fatalf("%s encoding = %q, want binary/zlib from supplied converter", name, got)
		}
	}
	var out UserMetadata
	if err := userMetadataFromProto(cfg, message, &out); err != nil {
		t.Fatalf("decode user metadata with supplied converter: %v", err)
	}
	if out != want {
		t.Fatalf("roundtrip = %#v, want %#v", out, want)
	}
	if err := userMetadataFromProto(cfg, &sdk.UserMetadata{}, &out); err != nil {
		t.Fatalf("decode empty user metadata: %v", err)
	}
	if out != (UserMetadata{}) {
		t.Fatalf("reused output = %#v, want zero user metadata", out)
	}

	wrongType, err := dc.ToPayload(42)
	if err != nil {
		t.Fatalf("encode wrong-type payload: %v", err)
	}
	if err := userMetadataFromProto(cfg, &sdk.UserMetadata{Summary: wrongType}, &out); err == nil ||
		!strings.Contains(err.Error(), "StaticSummary has unexpected type") {
		t.Fatalf("decode non-string summary error = %v, want field-specific type error", err)
	}
	invalid := &common.Payload{
		Metadata: map[string][]byte{converter.MetadataEncoding: []byte("binary/zlib")},
		Data:     []byte("not zlib"),
	}
	if err := userMetadataFromProto(cfg, &sdk.UserMetadata{Details: invalid}, &out); err == nil {
		t.Fatal("decode invalid compressed details succeeded, want converter error")
	}
}
"#,
    )
    .unwrap();
    let status = Command::new("go")
        .args(["test", "./output"])
        .current_dir(&temp_dir)
        .status()
        .unwrap();
    fs::remove_dir_all(temp_dir).unwrap();
    assert!(status.success(), "generated Go model runtime tests failed");
}
