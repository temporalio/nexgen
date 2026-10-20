//! C# generator tests for JSON Schema input that compile and run the output.
//!
//! The counterpart of the JSON Schema toolchain tests for Go, Java, Python and
//! TypeScript: the same generic scenarios (unions, presence, materialized
//! formats, cross-module references, name overrides, services), plus the edge
//! cases only .NET has — UTF-16 strings, `DateTimeOffset` resolution, `$`
//! before a final newline, value-type nullability, record equality over
//! arrays, XML doc comments, and framework type names a schema can reuse.
//!
//! Text assertions pin the emitted shape. Each test also compiles the output
//! with warnings as errors, and most run a hand-written `Program.cs` against
//! it through Temporal's default payload converter (see [`JsonCase::run`]),
//! since a text assertion cannot tell whether C# accepts what it pins. The
//! tests that need no toolchain live in `tests/generate_dotnet_json_schema.rs`.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Condvar, Mutex};

use nexgen::generator::GenerationMode;
use nexgen::language::Language;
use nexgen::nexgen_config::NexgenConfig;
use nexgen::{GenerateRequest, generate_to_file};

use crate::support::common;
use crate::support::csharp_json_case::{JsonCase, NAMESPACE, assert_has, assert_lacks};

fn project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// Bounds concurrent `dotnet` builds: each is a multi-process MSBuild run, and
/// the whole suite at once starves the machine without finishing sooner.
struct BuildSlots {
    free: Mutex<usize>,
    released: Condvar,
}

static BUILD_SLOTS: BuildSlots = BuildSlots {
    free: Mutex::new(4),
    released: Condvar::new(),
};

struct BuildSlot;

impl BuildSlot {
    fn acquire() -> Self {
        let mut free = BUILD_SLOTS.free.lock().unwrap();
        while *free == 0 {
            free = BUILD_SLOTS.released.wait(free).unwrap();
        }
        *free -= 1;
        BuildSlot
    }
}

impl Drop for BuildSlot {
    fn drop(&mut self) {
        *BUILD_SLOTS.free.lock().unwrap() += 1;
        BUILD_SLOTS.released.notify_one();
    }
}

impl JsonCase {
    /// Compiles the generated output with warnings as errors.
    fn build(&self) {
        self.dotnet(None);
    }

    /// Compiles the project as it stands (a `Program.cs` the test wrote
    /// itself) and returns the compiler output of the build it expects to fail.
    fn build_err(&self) -> String {
        self.write_project(true);
        let _slot = BuildSlot::acquire();
        let build = dotnet_command()
            .current_dir(&self.root)
            .args([
                "build",
                "Probe.csproj",
                "-o",
                "out",
                "--nologo",
                "-clp:NoSummary",
            ])
            .output()
            .unwrap();
        let output = format!(
            "{}{}",
            String::from_utf8_lossy(&build.stdout),
            String::from_utf8_lossy(&build.stderr)
        );
        assert!(!build.status.success(), "dotnet build succeeded:\n{output}");
        output
    }

    /// Compiles the generated output with `program` as the body of `Main`
    /// (with the harness's [`HARNESS`] `Check` helpers and `using Probe;` in
    /// scope) and runs it. Any exception fails the test with its message.
    fn run(&self, program: &str) {
        self.run_with_usings(&["Probe"], program);
    }

    fn run_with_usings(&self, usings: &[&str], program: &str) {
        let mut source = String::new();
        for using in usings {
            source.push_str(&format!("using {using};\n"));
        }
        source.push_str("\npublic static class Program\n{\n    public static void Main()\n    {\n");
        for line in program.lines() {
            source.push_str("        ");
            source.push_str(line);
            source.push('\n');
        }
        source.push_str("    }\n}\n");
        self.dotnet(Some(&source));
    }

    /// Compiles for net8.0, the generated code's floor (.NET 8, C# 12), and
    /// runs on the newest installed runtime.
    fn write_project(&self, executable: bool) {
        fs::write(
            self.root.join("Probe.csproj"),
            format!(
                r#"<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <OutputType>{}</OutputType>
    <TargetFramework>net8.0</TargetFramework>
    <RollForward>LatestMajor</RollForward>
    <Nullable>enable</Nullable>
    <TreatWarningsAsErrors>true</TreatWarningsAsErrors>
    <GenerateDocumentationFile>true</GenerateDocumentationFile>
    <NoWarn>CS1591</NoWarn>
    <AssemblyName>Probe</AssemblyName>
  </PropertyGroup>
  <ItemGroup>
    <Compile Remove="bin/**;obj/**;out/**;input/**" />
  </ItemGroup>
  <ItemGroup>
    <PackageReference Include="NexusRpc" Version="0.4.0" />
    <PackageReference Include="Temporalio" Version="1.19.0" />
  </ItemGroup>
</Project>
"#,
                if executable { "Exe" } else { "Library" }
            ),
        )
        .unwrap();
    }

    fn dotnet(&self, program: Option<&str>) {
        let executable = program.is_some();
        self.write_project(executable);
        if let Some(program) = program {
            fs::write(self.root.join("Program.cs"), program).unwrap();
            fs::write(self.root.join("Harness.cs"), HARNESS).unwrap();
        }
        let _slot = BuildSlot::acquire();
        let build = dotnet_command()
            .current_dir(&self.root)
            .args([
                "build",
                "Probe.csproj",
                "-o",
                "out",
                "--nologo",
                "-clp:NoSummary",
            ])
            .output()
            .unwrap();
        assert!(
            build.status.success(),
            "dotnet build failed in {}:\n{}{}",
            self.root.display(),
            String::from_utf8_lossy(&build.stdout),
            String::from_utf8_lossy(&build.stderr)
        );
        if executable {
            let run = dotnet_command()
                .current_dir(&self.root)
                .args(["out/Probe.dll"])
                .output()
                .unwrap();
            assert!(
                run.status.success(),
                "Program.cs failed in {}:\n{}{}",
                self.root.display(),
                String::from_utf8_lossy(&run.stdout),
                String::from_utf8_lossy(&run.stderr)
            );
        }
    }
}

fn dotnet_command() -> Command {
    let mut command = Command::new("dotnet");
    command
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .env("DOTNET_SKIP_FIRST_TIME_EXPERIENCE", "1")
        .env("DOTNET_NOLOGO", "1");
    command
}

/// The `Check` helpers every `Program.cs` can call. Values travel through
/// Temporal's stock `DataConverter.Default`, exactly as a workflow's would.
///
/// A violation is matched by prefix against its `path: reason` rendering, so
/// `"value: must have length"` pins the path exactly and the reason loosely.
/// The expected and actual violation lists must pair up one-to-one.
const HARNESS: &str = r#"using System;
using System.Collections;
using System.Collections.Generic;
using System.Linq;
using System.Text;
using Temporalio.Api.Common.V1;
using Temporalio.Converters;
using Temporalio.Exceptions;

internal static class Check
{
    private static readonly IPayloadConverter Converter = DataConverter.Default.PayloadConverter;

    public static T Parse<T>(string json) => (T)Converter.ToValue(Payload(json), typeof(T))!;

    public static string Write(object? value) => Converter.ToPayload(value).Data.ToStringUtf8();

    /// Parses <paramref name="json"/> and writes it back, requiring exactly
    /// <paramref name="expected"/> (default: the input) on the wire.
    public static T RoundTrip<T>(string json, string? expected = null)
    {
        var value = Parse<T>(json);
        Equal(Canonical(expected ?? json), Canonical(Write(value)), "round trip of " + json);
        return value;
    }

    /// The JSON text with every string re-escaped minimally, so two spellings
    /// of one string compare equal while member order and number tokens (which
    /// <see cref="System.Text.Json.JsonElement.WriteTo"/> copies verbatim) stay exact.
    public static string Canonical(string json)
    {
        using var document = System.Text.Json.JsonDocument.Parse(json);
        var buffer = new System.IO.MemoryStream();
        using (var writer = new System.Text.Json.Utf8JsonWriter(
            buffer,
            new System.Text.Json.JsonWriterOptions { Encoder = System.Text.Encodings.Web.JavaScriptEncoder.UnsafeRelaxedJsonEscaping }))
        {
            document.RootElement.WriteTo(writer);
        }
        return Encoding.UTF8.GetString(buffer.ToArray());
    }

    public static void ParseFails<T>(string json, params string[] expected) =>
        Violations(() => Parse<T>(json), "parsing " + json, expected);

    public static void WriteFails(object value, params string[] expected) =>
        Violations(() => Write(value), "writing " + value, expected);

    /// The violations a <c>PayloadValidationError</c> thrown by
    /// <paramref name="action"/> carries, rendered as <c>path: reason</c>.
    public static List<string> ViolationsOf(Action action, string context)
    {
        try
        {
            action();
        }
        catch (Exception error)
        {
            for (var current = error; current is not null; current = current.InnerException)
            {
                if (current is ApplicationFailureException failure
                    && failure.ErrorType == "PayloadValidationError")
                {
                    True(failure.Details.Count > 0, context + ": failure carries no details");
                    return failure.Details.ElementAt<IEnumerable>(0)
                        .Cast<object>()
                        .Select(violation => violation.ToString()!)
                        .ToList();
                }
            }
            throw new Exception(context + ": expected a PayloadValidationError, got " + error, error);
        }
        throw new Exception(context + ": expected a PayloadValidationError, but it succeeded");
    }

    private static void Violations(Action action, string context, string[] expected)
    {
        var actual = ViolationsOf(action, context);
        var unmatched = new List<string>(actual);
        var missing = new List<string>();
        foreach (var want in expected)
        {
            var index = unmatched.FindIndex(found => found.StartsWith(want, StringComparison.Ordinal));
            if (index < 0)
            {
                missing.Add(want);
            }
            else
            {
                unmatched.RemoveAt(index);
            }
        }
        if (missing.Count > 0 || unmatched.Count > 0)
        {
            throw new Exception(
                context + ":\n  expected: [" + string.Join(" | ", expected) + "]\n  actual:   ["
                + string.Join(" | ", actual) + "]");
        }
    }

    public static void Equal<T>(T expected, T actual, string context = "")
    {
        if (!EqualityComparer<T>.Default.Equals(expected, actual))
        {
            throw new Exception(context + ": expected <" + expected + "> but got <" + actual + ">");
        }
    }

    public static void True(bool condition, string context)
    {
        if (!condition)
        {
            throw new Exception("check failed: " + context);
        }
    }

    public static void Throws<TException>(Action action, string context)
        where TException : Exception
    {
        try
        {
            action();
        }
        catch (TException)
        {
            return;
        }
        throw new Exception(context + ": expected " + typeof(TException).Name);
    }

    private static Payload Payload(string json) => new()
    {
        Metadata = { ["encoding"] = Google.Protobuf.ByteString.CopyFromUtf8("json/plain") },
        Data = Google.Protobuf.ByteString.CopyFromUtf8(json),
    };
}
"#;

// ---------------------------------------------------------------------------
// Checked-in samples
// ---------------------------------------------------------------------------

#[test]
fn csharp_json_examples_build_in_both_modes() {
    for example in ["chat", "kb", "showcase", "temporal"] {
        for mode in [GenerationMode::DefinitionsOnly, GenerationMode::NativeApi] {
            let case = JsonCase::new(&format!("sample-{example}"));
            generate_to_file(&GenerateRequest {
                config: NexgenConfig {
                    mode,
                    ..Default::default()
                },
                language: Language::Dotnet,
                input_paths: vec![common::json_input_path(&project_root(), example)],
                support_package: None,
                descriptor_paths: Vec::new(),
                output_path: case.output_dir(),
                format: false,
                java_package_name: None,
                ts_date_time_types: Default::default(),
                dotnet_namespace: Some(NAMESPACE.to_string()),
            })
            .unwrap();
            case.build();
        }
    }
}

// ---------------------------------------------------------------------------
// Unions
// ---------------------------------------------------------------------------

const INLINE_OBJECT_BRANCH_SCHEMA: &str = r#"type: object
properties:
  payload:
    oneOf:
      - type: object
        required: [text]
        properties:
          text: { type: string, minLength: 1 }
      - { type: string }
"#;

#[test]
fn csharp_json_names_inline_object_union_branch() {
    let case = JsonCase::new("inline-object-branch");
    case.write("detail.yaml", INLINE_OBJECT_BRANCH_SCHEMA);
    let output = case.generate(&["detail.yaml"]);
    let models = output.file("Models.cs");
    assert_has(
        models,
        &[
            "public interface PayloadUnion",
            "public sealed class PayloadUnionString : PayloadUnion",
            "public PayloadUnion? Payload { get; init; }",
            // The object branch is a top-level record named after its owner
            // and member, implementing the owner's nested union interface.
            "public sealed record DetailPayloadObject : Detail.PayloadUnion",
            "public required string Text { get; init; }",
        ],
    );
    case.run(
        r#"var object_ = Check.RoundTrip<Detail>("{\"payload\":{\"text\":\"hi\"}}");
Check.True(object_.Payload is DetailPayloadObject { Text: "hi" }, "object branch");
var text = Check.RoundTrip<Detail>("{\"payload\":\"plain\"}");
Check.True(text.Payload is Detail.PayloadUnionString { Value: "plain" }, "string branch");
Check.ParseFails<Detail>("{\"payload\":{\"text\":\"\"}}", "payload.text: must have length >= 1");
Check.ParseFails<Detail>("{\"payload\":{}}", "payload.text: required");
Check.ParseFails<Detail>("{\"payload\":5}", "payload: ");
Check.WriteFails(new Detail { Payload = new DetailPayloadObject { Text = "" } }, "payload.text: must have length >= 1");"#,
    );
}

const BRANCH_CONSTRAINT_SCHEMA: &str = r#"type: object
properties:
  value:
    oneOf:
      - { type: string, minLength: 3, pattern: "^[a-z]+$" }
      - { type: integer, minimum: 1 }
  listOrName:
    oneOf:
      - { type: array, items: { type: number }, minItems: 1, uniqueItems: true }
      - { type: string, enum: [auto, manual] }
"#;

#[test]
fn csharp_json_validates_non_object_union_branch_constraints() {
    let case = JsonCase::new("branch-constraints");
    case.write("bc.yaml", BRANCH_CONSTRAINT_SCHEMA);
    let output = case.generate(&["bc.yaml"]);
    assert_has(
        output.file("Models.cs"),
        &[
            "public sealed class ValueUnionString : ValueUnion",
            "public sealed class ValueUnionInteger : ValueUnion",
            "public sealed class ListOrNameUnionArray : ListOrNameUnion",
            "global::System.Collections.Generic.IReadOnlyList<double> Value { get; }",
            // `$` compiles to `\z`: .NET's `$` also matches before a final newline.
            "^[a-z]+\\\\z",
            "global::System.Text.RegularExpressions.RegexOptions.CultureInvariant",
        ],
    );
    case.run(
        r#"Check.RoundTrip<Bc>("{\"value\":\"abc\",\"listOrName\":[1,2.5]}");
Check.RoundTrip<Bc>("{\"value\":7,\"listOrName\":\"auto\"}");
Check.ParseFails<Bc>("{\"value\":\"ab\"}", "value: must have length >= 3");
Check.ParseFails<Bc>("{\"value\":\"ABC\"}", "value: must match pattern");
Check.ParseFails<Bc>("{\"value\":\"abc\\n\"}", "value: must match pattern");
Check.ParseFails<Bc>("{\"value\":0}", "value: must be >= 1");
Check.ParseFails<Bc>("{\"listOrName\":[]}", "listOrName: must have at least 1 items");
Check.ParseFails<Bc>("{\"listOrName\":[1,1.0]}", "listOrName: duplicate items");
Check.ParseFails<Bc>("{\"listOrName\":\"other\"}", "listOrName: must be one of");
// The serialize side runs the same predicates over the in-memory branch.
Check.WriteFails(new Bc { Value = new Bc.ValueUnionString("ab") }, "value: must have length >= 3");
Check.WriteFails(new Bc { Value = new Bc.ValueUnionInteger(0) }, "value: must be >= 1");
Check.WriteFails(
    new Bc { ListOrName = new Bc.ListOrNameUnionArray(new double[] { 2, 2 }) },
    "listOrName: duplicate items");
Check.WriteFails(
    new Bc { ListOrName = new Bc.ListOrNameUnionString("other") },
    "listOrName: must be one of");"#,
    );
}

#[test]
fn csharp_json_inline_union_names_follow_the_emitted_member() {
    let case = JsonCase::new("renamed-union");
    case.write(
        "renamed_union.yaml",
        r#"title: RenamedUnion
type: object
properties:
  serializer:
    x-cs-name: Payload
    oneOf: [{ type: string }, { type: integer }]
"#,
    );
    let output = case.generate(&["renamed_union.yaml"]);
    let models = output.file("Models.cs");
    assert_has(
        models,
        &[
            "public interface PayloadUnion",
            "public PayloadUnion? Payload { get; init; }",
            "\"serializer\"",
        ],
    );
    assert_lacks(models, &["SerializerUnion", " Serializer {"]);
    case.run(
        r#"var parsed = Check.RoundTrip<RenamedUnion>("{\"serializer\":3}");
Check.True(parsed.Payload is RenamedUnion.PayloadUnionInteger { Value: 3 }, "integer branch");"#,
    );
}

const ELEMENT_UNION_SCHEMA: &str = r##"type: object
properties:
  segments: { type: array, items: { oneOf: [{ type: string }, { type: integer }] } }
  choices: { type: array, items: { $ref: "#/$defs/Choice" } }
  entries: { $ref: "#/$defs/Entries" }
  slots: { type: array, items: { oneOf: [{ type: string }, { type: "null" }] } }
  counts: { type: array, items: { oneOf: [{ type: integer }, { type: "null" }] } }
$defs:
  Choice: { oneOf: [{ type: string }, { type: boolean }] }
  Entries:
    type: object
    additionalProperties: { oneOf: [{ type: string }, { type: integer }] }
"##;

#[test]
fn csharp_json_decodes_element_position_unions() {
    let case = JsonCase::new("element-unions");
    case.write("bag.yaml", ELEMENT_UNION_SCHEMA);
    let output = case.generate(&["bag.yaml"]);
    assert_has(
        output.file("Models.cs"),
        &[
            // Position-derived names: `<Model><Member>Item`, `<Map>Value`.
            "public interface BagSegmentsItem",
            "public sealed class BagSegmentsItemInteger : BagSegmentsItem",
            "IReadOnlyList<BagSegmentsItem>? Segments { get; init; }",
            "IReadOnlyList<Choice>? Choices { get; init; }",
            "public interface EntriesValue",
            "IReadOnlyDictionary<string, EntriesValue> AdditionalProperties",
            // A nullable element is a nullable element type, for reference and
            // value types alike.
            "IReadOnlyList<string?>? Slots { get; init; }",
            "IReadOnlyList<long?>? Counts { get; init; }",
        ],
    );
    case.run(
        r#"var bag = Check.RoundTrip<Bag>(
    "{\"segments\":[\"a\",1],\"choices\":[\"x\",true],\"entries\":{\"k\":\"v\",\"n\":2},\"slots\":[\"a\",null],\"counts\":[1,null]}");
Check.True(bag.Segments![1] is BagSegmentsItem.BagSegmentsItemInteger { Value: 1 }, "segments[1]");
Check.True(bag.Entries!.AdditionalProperties["n"] is EntriesValue.EntriesValueInteger { Value: 2 }, "entries.n");
Check.Equal<long?>(null, bag.Counts![1], "counts[1]");
Check.ParseFails<Bag>("{\"segments\":[1.5]}", "segments[0]: ");
Check.ParseFails<Bag>("{\"segments\":[null]}", "segments[0]: ");
Check.ParseFails<Bag>("{\"entries\":{\"bad\":true}}", "entries.bad: ");
Check.ParseFails<Bag>("{\"counts\":[\"1\"]}", "counts[0]: ");
// A null smuggled into a non-nullable element is a violation, not a crash.
Check.WriteFails(new Bag { Segments = new BagSegmentsItem[] { null! } }, "segments[0]: ");
Check.WriteFails(
    new Bag { Entries = new Entries { AdditionalProperties = new System.Collections.Generic.Dictionary<string, EntriesValue> { ["bad"] = null! } } },
    "entries.bad: ");"#,
    );
}

const DISCRIMINATED_SCHEMA: &str = r##"type: object
required: [shape]
properties:
  shape: { $ref: "#/$defs/Shape" }
  score: { type: integer, const: 1.0 }
$defs:
  Shape: { oneOf: [{ $ref: "#/$defs/Circle" }, { $ref: "#/$defs/Square" }] }
  Circle:
    type: object
    required: [kind, r]
    properties: { kind: { type: integer, const: 1 }, r: { type: number } }
  Square:
    type: object
    required: [kind, side]
    properties: { kind: { type: integer, const: 2 }, side: { type: number } }
"##;

#[test]
fn csharp_json_dispatches_a_non_string_discriminant_by_value() {
    let case = JsonCase::new("numeric-discriminant");
    case.write("tagged.yaml", DISCRIMINATED_SCHEMA);
    let output = case.generate(&["tagged.yaml"]);
    let models = output.file("Models.cs");
    assert_has(
        models,
        &[
            "public interface Shape",
            "public sealed record Circle : Shape",
            "public sealed record Square : Shape",
            "public static readonly KindValue V1 = new(1L);",
            "public static readonly KindValue V2 = new(2L);",
            // An integer const spelled `1.0` is the integer 1.
            "public static readonly ScoreValue V1 = new(1L);",
        ],
    );
    case.run(
        r#"var circle = Check.RoundTrip<Tagged>("{\"shape\":{\"kind\":1,\"r\":2.5}}");
Check.True(circle.Shape is Circle { R: 2.5 }, "circle");
Check.True(Check.RoundTrip<Tagged>("{\"shape\":{\"kind\":2,\"side\":1}}").Shape is Square, "square");
// The discriminant matches by JSON value, so `1.0` selects the `1` branch
// and is re-emitted in its canonical spelling.
Check.RoundTrip<Tagged>("{\"shape\":{\"kind\":1.0,\"r\":2.5}}", "{\"shape\":{\"kind\":1,\"r\":2.5}}");
Check.RoundTrip<Tagged>("{\"shape\":{\"kind\":1,\"r\":1},\"score\":1.0}", "{\"shape\":{\"kind\":1,\"r\":1},\"score\":1}");
Check.ParseFails<Tagged>("{\"shape\":{\"kind\":3,\"side\":1}}", "shape: ");
Check.ParseFails<Tagged>("{\"shape\":{\"kind\":\"1\",\"r\":1}}", "shape: ");
Check.ParseFails<Tagged>("{\"shape\":{\"kind\":1,\"r\":1},\"score\":2}", "score: must equal 1");"#,
    );
}

#[test]
fn csharp_json_dispatches_cross_module_ref_union_branches() {
    let case = JsonCase::new("cross-module-union");
    case.write(
        "shapes.yaml",
        r##"$defs:
  Circle:
    type: object
    required: [kind]
    properties: { kind: { type: string, const: circle }, r: { type: number } }
  Square:
    type: object
    required: [kind]
    properties: { kind: { type: string, const: square }, s: { type: number } }
"##,
    );
    case.write(
        "main.yaml",
        r#"type: object
properties:
  shape:
    oneOf:
      - { $ref: "shapes.yaml#/$defs/Circle" }
      - { $ref: "shapes.yaml#/$defs/Square" }
      - { type: string }
"#,
    );
    let output = case.generate(&["main.yaml", "shapes.yaml"]);
    let shapes = output.file("shapes/Models.cs");
    // A branch declared in another module implements the owner's interface.
    assert_has(
        shapes,
        &[
            "namespace Probe.Shapes",
            "public sealed record Circle : global::Probe.Main.Main.ShapeUnion",
        ],
    );
    case.run_with_usings(
        &["Probe.Main", "Probe.Shapes"],
        r#"Check.True(Check.RoundTrip<Main>("{\"shape\":{\"kind\":\"circle\",\"r\":1}}").Shape is Circle, "circle");
Check.True(Check.RoundTrip<Main>("{\"shape\":{\"kind\":\"square\",\"s\":2}}").Shape is Square, "square");
Check.True(Check.RoundTrip<Main>("{\"shape\":\"free\"}").Shape is Main.ShapeUnionString, "string");
Check.ParseFails<Main>("{\"shape\":{\"kind\":\"tri\"}}", "shape: ");"#,
    );
}

#[test]
fn csharp_json_cross_file_named_union_is_declared_once() {
    let case = JsonCase::new("cross-file-named-union");
    case.write(
        "shapes.nexusrpc.yaml",
        r#"$defs:
  Circle:
    type: object
    required: [kind]
    properties: { kind: { type: string, const: circle }, r: { type: number } }
  Square:
    type: object
    required: [kind]
    properties: { kind: { type: string, const: square }, side: { type: number } }
"#,
    );
    case.write(
        "holder.nexusrpc.yaml",
        r#"$defs:
  Shape:
    oneOf:
      - { $ref: "shapes.nexusrpc.yaml#/$defs/Circle" }
      - { $ref: "shapes.nexusrpc.yaml#/$defs/Square" }
"#,
    );
    case.write(
        "drawing.nexusrpc.yaml",
        r#"$defs:
  Drawing:
    type: object
    properties:
      main: { $ref: "holder.nexusrpc.yaml#/$defs/Shape" }
"#,
    );
    let output = case.generate(&["."]);
    let all = output.all();
    assert_eq!(all.matches("public interface Shape\n").count(), 1, "{all}");
    assert_has(
        output.file("shapes/Models.cs"),
        &["public sealed record Circle : global::Probe.Holder.Shape"],
    );
    assert_has(
        output.file("drawing/Models.cs"),
        &["public global::Probe.Holder.Shape? Main { get; init; }"],
    );
    case.run_with_usings(
        &["Probe.Drawing", "Probe.Shapes"],
        r#"Check.True(Check.RoundTrip<Drawing>("{\"main\":{\"kind\":\"circle\",\"r\":1}}").Main is Circle, "circle");
Check.ParseFails<Drawing>("{\"main\":{\"kind\":\"tri\"}}", "main: ");"#,
    );
}

#[test]
fn csharp_json_renamed_union_reference_binds_the_interface() {
    let case = JsonCase::new("renamed-union-ref");
    case.write(
        "shapes.yaml",
        r##"$defs:
  Shape:
    x-cs-name: ShapeCs
    oneOf: [{ $ref: "#/$defs/Circle" }, { $ref: "#/$defs/Square" }]
  Circle:
    type: object
    required: [kind]
    properties: { kind: { type: string, const: circle } }
  Square:
    type: object
    required: [kind]
    properties: { kind: { type: string, const: square } }
  Holder:
    type: object
    properties: { shape: { $ref: "#/$defs/Shape" } }
"##,
    );
    let output = case.generate(&["shapes.yaml"]);
    let models = output.file("Models.cs");
    assert_has(
        models,
        &[
            "public interface ShapeCs",
            "public sealed record Circle : ShapeCs",
            "public ShapeCs? Shape { get; init; }",
        ],
    );
    assert_lacks(models, &["public interface Shape\n"]);
    case.run(
        r#"Check.True(Check.RoundTrip<Holder>("{\"shape\":{\"kind\":\"square\"}}").Shape is Square, "square");"#,
    );
}

#[test]
fn csharp_json_typed_map_union_branch_implements_the_interface() {
    let case = JsonCase::new("union-map-branch");
    case.write(
        "union_map.yaml",
        r#"type: object
required: [scalarUnion]
properties:
  scalarUnion:
    oneOf:
      - { type: string }
      - { type: integer }
      - { type: array, items: { type: string } }
      - { type: object, additionalProperties: { type: string } }
"#,
    );
    let output = case.generate(&["union_map.yaml"]);
    assert_has(
        output.file("Models.cs"),
        &["public sealed record UnionMapScalarUnionObject : UnionMap.ScalarUnionUnion"],
    );
    case.run(
        r#"var parsed = Check.RoundTrip<UnionMap>("{\"scalarUnion\":{\"a\":\"b\"}}");
Check.True(parsed.ScalarUnion is UnionMapScalarUnionObject map && map.AdditionalProperties["a"] == "b", "map branch");
Check.RoundTrip<UnionMap>("{\"scalarUnion\":[\"x\"]}");
Check.ParseFails<UnionMap>("{\"scalarUnion\":{\"a\":1}}", "scalarUnion.a: ");"#,
    );
}

#[test]
fn csharp_json_recursively_converts_union_array_branches() {
    let case = JsonCase::new("union-array-branches");
    case.write(
        "probe.yaml",
        r##"type: object
properties:
  events:
    oneOf:
      - { type: array, minItems: 1, items: { type: string, format: date-time } }
      - { type: string }
  blobs:
    oneOf:
      - { type: array, items: { type: string, contentEncoding: base64 } }
      - { type: boolean }
  children:
    oneOf:
      - { type: array, items: { $ref: "#/$defs/Child" } }
      - { type: integer }
$defs:
  Child:
    type: object
    additionalProperties: false
    required: [name]
    properties: { name: { type: string, minLength: 2 } }
"##,
    );
    let output = case.generate(&["probe.yaml"]);
    assert_has(
        output.file("Models.cs"),
        &[
            "IReadOnlyList<global::System.DateTimeOffset> Value { get; }",
            "IReadOnlyList<byte[]> Value { get; }",
            "IReadOnlyList<Child> Value { get; }",
        ],
    );
    assert!(output.has_file("TemporalSupport.cs"));
    assert!(output.has_file("Base64Support.cs"));
    case.run(
        r#"var parsed = Check.RoundTrip<Probe.Probe>(
    "{\"events\":[\"2024-01-02T03:04:05Z\"],\"blobs\":[\"AQI=\"],\"children\":[{\"name\":\"ab\"}]}");
Check.True(parsed.Events is Probe.Probe.EventsUnionArray { Value: [var at] } && at.Year == 2024, "events");
Check.True(parsed.Blobs is Probe.Probe.BlobsUnionArray { Value: [[1, 2]] }, "blobs");
Check.ParseFails<Probe.Probe>("{\"events\":[\"nope\"]}", "events[0]: must be a valid date-time");
Check.ParseFails<Probe.Probe>("{\"events\":[]}", "events: must have at least 1 items");
Check.ParseFails<Probe.Probe>("{\"blobs\":[\"*\"]}", "blobs[0]: ");
Check.ParseFails<Probe.Probe>("{\"children\":[{\"name\":\"a\"}]}", "children[0].name: must have length >= 2");
Check.WriteFails(
    new Probe.Probe { Children = new Probe.Probe.ChildrenUnionArray(new[] { new Child { Name = "a" } }) },
    "children[0].name: must have length >= 2");"#,
    );
}

#[test]
fn csharp_json_assertion_only_format_branch_stays_a_string() {
    let case = JsonCase::new("assertion-only-format");
    case.write(
        "assertion_only.yaml",
        r#"title: AssertionOnly
type: object
properties:
  contacts:
    oneOf:
      - { type: array, items: { type: string, format: email } }
      - { type: boolean }
"#,
    );
    let output = case.generate(&["assertion_only.yaml"]);
    assert_has(
        output.file("Models.cs"),
        &["IReadOnlyList<string> Value { get; }"],
    );
    assert!(!output.has_file("TemporalSupport.cs"));
    assert!(!output.has_file("Base64Support.cs"));
    case.run(
        r#"Check.RoundTrip<AssertionOnly>("{\"contacts\":[\"a@example.com\"]}");
Check.ParseFails<AssertionOnly>("{\"contacts\":[\"nope\"]}", "contacts[0]: must be a valid email");"#,
    );
}

#[test]
fn csharp_json_union_alias_keeps_its_title_and_deprecation() {
    let case = JsonCase::new("union-alias-docs");
    case.write(
        "doc.yaml",
        r##"type: object
properties:
  u: { $ref: "#/$defs/U" }
$defs:
  U:
    title: UNION TITLE
    description: UNION BODY
    deprecated: true
    oneOf: [{ type: string }, { type: integer }]
"##,
    );
    let output = case.generate(&["doc.yaml"]);
    assert_has(
        output.file("Models.cs"),
        &[
            "    /// <summary>\n    /// UNION TITLE\n    /// <para/>\n    /// UNION BODY\n    /// </summary>\n    [global::System.Obsolete(\"This type is deprecated.\")]\n",
            "    public interface U\n",
        ],
    );
    case.build();
}

// ---------------------------------------------------------------------------
// Presence, nullability and defaults
// ---------------------------------------------------------------------------

const PRESENCE_SCHEMA: &str = r#"type: object
required: [requiredPlain, requiredNullable]
properties:
  requiredPlain: { type: string }
  requiredNullable: { oneOf: [{ type: integer }, { type: "null" }] }
  optionalPlain: { type: boolean }
  optionalNullable: { oneOf: [{ type: string }, { type: "null" }] }
  nullableItems: { type: array, items: { oneOf: [{ type: string }, { type: "null" }] } }
  greeting: { type: string, default: hello, deprecated: true, x-cs-name: Salutation }
  retries: { type: integer, default: 3 }
  fallback:
    oneOf: [{ type: string }, { type: "null" }]
    default: fallback
"#;

#[test]
fn csharp_json_presence_nullability_and_defaults() {
    let case = JsonCase::new("presence");
    case.write("model.yaml", PRESENCE_SCHEMA);
    let output = case.generate(&["model.yaml"]);
    assert_has(
        output.file("Models.cs"),
        &[
            // Required members are `required`, so leaving one out at a
            // construction site is a compile error (P2).
            "public required string RequiredPlain { get; init; }",
            "public required long? RequiredNullable { get; init; }",
            // Optional value types are `Nullable<T>`: absence is never zero (P9).
            "public bool? OptionalPlain { get; init; }",
            "public string? OptionalNullable { get; init; }",
            "IReadOnlyList<string?>? NullableItems { get; init; }",
            // A default is read through `OrDefault`; the member keeps presence.
            "[global::System.Obsolete(\"This field is deprecated.\")]\n        public string? Salutation { get; init; }",
            "public string SalutationOrDefault => this.Salutation ?? \"hello\";",
            "public long RetriesOrDefault => this.Retries ?? 3L;",
        ],
    );
    case.run(
        r#"var minimal = Check.RoundTrip<Model>("{\"requiredPlain\":\"a\",\"requiredNullable\":null}");
Check.Equal<long?>(null, minimal.RequiredNullable, "required nullable");
// Defaults apply on read, never on the wire.
Check.Equal<long?>(null, minimal.Retries, "unset default");
Check.Equal(3L, minimal.RetriesOrDefault, "default");
#pragma warning disable CS0618
Check.Equal("hello", minimal.SalutationOrDefault, "renamed default");
Check.RoundTrip<Model>("{\"requiredPlain\":\"a\",\"requiredNullable\":1,\"greeting\":\"hi\",\"retries\":3}");
#pragma warning restore CS0618
// Optional+nullable collapses an explicit null to absence in memory.
Check.RoundTrip<Model>(
    "{\"requiredPlain\":\"a\",\"requiredNullable\":1,\"optionalNullable\":null}",
    "{\"requiredPlain\":\"a\",\"requiredNullable\":1}");
Check.RoundTrip<Model>("{\"requiredPlain\":\"a\",\"requiredNullable\":1,\"nullableItems\":[\"x\",null]}");
Check.RoundTrip<Model>(
    "{\"requiredPlain\":\"a\",\"requiredNullable\":1,\"fallback\":null}",
    "{\"requiredPlain\":\"a\",\"requiredNullable\":1}");
Check.ParseFails<Model>("{\"requiredNullable\":1}", "requiredPlain: required");
Check.ParseFails<Model>("{\"requiredPlain\":\"a\"}", "requiredNullable: required");
Check.ParseFails<Model>("{\"requiredPlain\":null,\"requiredNullable\":1}", "requiredPlain: explicit null not allowed");
Check.ParseFails<Model>(
    "{\"requiredPlain\":\"a\",\"requiredNullable\":1,\"optionalPlain\":null}",
    "optionalPlain: explicit null not allowed");
Check.ParseFails<Model>(
    "{\"requiredPlain\":\"a\",\"requiredNullable\":1,\"retries\":null}",
    "retries: explicit null not allowed");
// Nullable reference annotations are compile-time only, so a required
// reference forced to null is caught before writing.
Check.WriteFails(new Model { RequiredPlain = null!, RequiredNullable = null }, "requiredPlain: required");
// Records compare by value, collections included.
Check.Equal(
    Check.Parse<Model>("{\"requiredPlain\":\"a\",\"requiredNullable\":1,\"nullableItems\":[\"x\"]}"),
    Check.Parse<Model>("{\"requiredPlain\":\"a\",\"requiredNullable\":1,\"nullableItems\":[\"x\"]}"),
    "structural equality");"#,
    );
}

#[test]
fn csharp_json_required_members_are_enforced_at_compile_time() {
    let case = JsonCase::new("required-compile");
    case.write("model.yaml", PRESENCE_SCHEMA);
    case.generate(&["model.yaml"]);
    fs::write(
        case.root.join("Program.cs"),
        "using Probe;\n\npublic static class Program\n{\n    public static void Main()\n    {\n        _ = new Model { RequiredPlain = \"a\" };\n    }\n}\n",
    )
    .unwrap();
    fs::write(case.root.join("Harness.cs"), HARNESS).unwrap();
    let error = case.build_err();
    // CS9035: required member must be set in the object initializer.
    assert_has(&error, &["CS9035", "RequiredNullable"]);
}

#[test]
fn csharp_json_nullable_properties_keep_their_shape_and_constraints() {
    let case = JsonCase::new("nullable-shape");
    case.write(
        "nullable_shape.yaml",
        r#"type: object
required: [nullInt, nullTags]
properties:
  nullName: { oneOf: [{ type: string, minLength: 3, pattern: "^[a-z]+$" }, { type: "null" }] }
  nullEnum: { oneOf: [{ type: string, enum: [a, b] }, { type: "null" }] }
  nullEmail: { oneOf: [{ type: string, format: email }, { type: "null" }] }
  nullInt: { oneOf: [{ type: integer, minimum: 1, maximum: 10 }, { type: "null" }] }
  nullRatio: { oneOf: [{ type: number, multipleOf: 2 }, { type: "null" }] }
  nullFlag: { oneOf: [{ type: boolean }, { type: "null" }] }
  nullTags:
    oneOf:
      - { type: array, items: { type: string }, minItems: 2, contains: { type: string, minLength: 2 } }
      - { type: "null" }
"#,
    );
    let output = case.generate(&["nullable_shape.yaml"]);
    assert_has(
        output.file("Models.cs"),
        &[
            "public NullEnumValue? NullEnum { get; init; }",
            "public required long? NullInt { get; init; }",
            "public double? NullRatio { get; init; }",
            "public bool? NullFlag { get; init; }",
            "public required global::System.Collections.Generic.IReadOnlyList<string>? NullTags { get; init; }",
        ],
    );
    case.run(
        r#"Check.RoundTrip<NullableShape>("{\"nullInt\":null,\"nullTags\":null}");
Check.RoundTrip<NullableShape>(
    "{\"nullName\":\"abc\",\"nullEnum\":\"a\",\"nullEmail\":\"x@example.com\",\"nullInt\":5,\"nullRatio\":4,\"nullFlag\":true,\"nullTags\":[\"ab\",\"c\"]}");
Check.ParseFails<NullableShape>(
    "{\"nullName\":\"ab\",\"nullEnum\":\"c\",\"nullEmail\":\"nope\",\"nullInt\":11,\"nullRatio\":3,\"nullTags\":[\"a\"]}",
    "nullName: must have length >= 3",
    "nullEnum: must be one of",
    "nullEmail: must be a valid email",
    "nullInt: must be <= 10",
    "nullRatio: must be a multiple of 2",
    "nullTags: must have at least 2 items",
    "nullTags: no element matches the required schema");
Check.ParseFails<NullableShape>("{\"nullName\":\"ABC\",\"nullInt\":0,\"nullTags\":null}",
    "nullName: must match pattern",
    "nullInt: must be >= 1");
Check.WriteFails(new NullableShape { NullInt = 0, NullTags = null }, "nullInt: must be >= 1");"#,
    );
}

#[test]
fn csharp_json_nullable_elements_keep_constraints_at_every_depth() {
    let case = JsonCase::new("nullable-elements");
    case.write(
        "bag.yaml",
        r#"type: object
properties:
  slots:
    type: array
    uniqueItems: true
    items: { oneOf: [{ type: string, minLength: 2 }, { type: "null" }] }
  counts:
    type: array
    contains: { type: integer, minimum: 5 }
    items: { oneOf: [{ type: integer }, { type: "null" }] }
  grid:
    type: array
    items: { type: array, items: { oneOf: [{ type: string, minLength: 2 }, { type: "null" }] } }
  lookup:
    type: object
    additionalProperties:
      type: array
      items: { oneOf: [{ type: string, minLength: 2 }, { type: "null" }] }
"#,
    );
    let output = case.generate(&["bag.yaml"]);
    assert_has(
        output.file("Models.cs"),
        &[
            "IReadOnlyList<string?>? Slots { get; init; }",
            "IReadOnlyList<long?>? Counts { get; init; }",
            "IReadOnlyList<global::System.Collections.Generic.IReadOnlyList<string?>>? Grid { get; init; }",
        ],
    );
    case.run(
        r#"Check.RoundTrip<Bag>(
    "{\"slots\":[\"ab\",null],\"counts\":[null,7],\"grid\":[[null,\"ab\"]],\"lookup\":{\"k\":[null,\"ab\"]}}");
// Two nulls are duplicates; a null never satisfies a matcher.
Check.ParseFails<Bag>("{\"slots\":[null,null]}", "slots: duplicate items");
Check.ParseFails<Bag>("{\"counts\":[null]}", "counts: no element matches the required schema");
Check.ParseFails<Bag>("{\"counts\":[null,1]}", "counts: no element matches the required schema");
Check.ParseFails<Bag>("{\"slots\":[\"a\",null]}", "slots[0]: must have length >= 2");
Check.ParseFails<Bag>("{\"grid\":[[null,\"a\"]]}", "grid[0][1]: must have length >= 2");
Check.ParseFails<Bag>("{\"lookup\":{\"k\":[\"a\"]}}", "lookup.k[0]: must have length >= 2");
Check.WriteFails(new Bag { Slots = new string?[] { null, null } }, "slots: duplicate items");
Check.WriteFails(new Bag { Counts = new long?[] { null } }, "counts: no element matches the required schema");
Check.WriteFails(new Bag { Grid = new[] { new string?[] { "a" } } }, "grid[0][0]: must have length >= 2");
Check.WriteFails(
    new Bag { Lookup = new BagLookup { AdditionalProperties = new System.Collections.Generic.Dictionary<string, System.Collections.Generic.IReadOnlyList<string?>> { ["k"] = new string?[] { "a" } } } },
    "lookup.k[0]: must have length >= 2");"#,
    );
}

// ---------------------------------------------------------------------------
// Numbers
// ---------------------------------------------------------------------------

#[test]
fn csharp_json_rejects_non_finite_numbers_in_every_serialize_position() {
    let case = JsonCase::new("non-finite");
    case.write(
        "finite.yaml",
        r##"type: object
required: [scalar, nested, metric, values]
properties:
  scalar: { type: number }
  nested: { type: array, items: { type: array, items: { type: number } } }
  metric: { oneOf: [{ type: number }, { type: string }] }
  values: { $ref: "#/$defs/Values" }
$defs:
  Values:
    type: object
    additionalProperties: { type: array, items: { type: number } }
"##,
    );
    case.generate(&["finite.yaml"]);
    case.run(
        r#"Check.WriteFails(
    new Finite
    {
        Scalar = double.NaN,
        Nested = new[] { new[] { 1.0, double.PositiveInfinity } },
        Metric = new Finite.MetricUnionNumber(double.NegativeInfinity),
        Values = new Values { AdditionalProperties = new System.Collections.Generic.Dictionary<string, System.Collections.Generic.IReadOnlyList<double>> { ["k"] = new[] { double.NaN } } },
    },
    "scalar: must be a finite number",
    "nested[0][1]: must be a finite number",
    "metric: must be a finite number",
    "values.k[0]: must be a finite number");
// The wire cannot carry a non-finite number, and an overflowing literal is not one.
Check.ParseFails<Finite>(
    "{\"scalar\":1e400,\"nested\":[],\"metric\":1,\"values\":{}}",
    "scalar: ");"#,
    );
}

#[test]
fn csharp_json_integer_domain_follows_the_shared_rules() {
    let case = JsonCase::new("integer-domain");
    case.write(
        "numbers.yaml",
        r#"type: object
properties:
  count: { type: integer }
  bounded: { type: number, maximum: 9007199254740992 }
  counts: { type: array, items: { type: number }, contains: { type: integer } }
"#,
    );
    let output = case.generate(&["numbers.yaml"]);
    assert_has(
        output.file("Models.cs"),
        &["public long? Count { get; init; }"],
    );
    case.run(
        r#"// Integral spellings are integers; the decimal text decides, not the double.
foreach (var (wire, canonical) in new[]
{
    ("1", "1"), ("1.0", "1"), ("1e2", "100"), ("100e-2", "1"), ("-0", "0"),
    ("9007199254740991", "9007199254740991"), ("-9007199254740991", "-9007199254740991"),
})
{
    Check.RoundTrip<Numbers>("{\"count\":" + wire + "}", "{\"count\":" + canonical + "}");
}
foreach (var wire in new[] { "1.5", "9007199254740992", "-9007199254740992", "4503599627370496.5", "1e400", "\"7\"", "true" })
{
    Check.ParseFails<Numbers>("{\"count\":" + wire + "}", "count: ");
}
// Number bounds compare in binary64: 2^53+1 rounds onto the bound.
Check.RoundTrip<Numbers>("{\"bounded\":9007199254740993}", "{\"bounded\":9007199254740992}");
Check.ParseFails<Numbers>("{\"bounded\":9007199254740995}", "bounded: must be <= 9007199254740992");
Check.ParseFails<Numbers>("{\"bounded\":\"1\"}", "bounded: ");
// A quoted number is a string, so it does not satisfy an integer matcher.
Check.ParseFails<Numbers>("{\"counts\":[1.5]}", "counts: no element matches the required schema");
Check.RoundTrip<Numbers>("{\"counts\":[1.5,2]}");
// A `long` past the cap cannot reach the wire.
Check.WriteFails(new Numbers { Count = 1L << 60 }, "count: exceeds ±(2^53-1) integer cap");
Check.WriteFails(new Numbers { Count = -(1L << 53) }, "count: exceeds ±(2^53-1) integer cap");"#,
    );
}

#[test]
fn csharp_json_minimum_is_inclusive_in_both_directions() {
    let case = JsonCase::new("minimum");
    case.write(
        "floor.yaml",
        "type: object\nproperties:\n  count: { type: integer, minimum: 1 }\n  ratio: { type: number, minimum: 0.5 }\n",
    );
    case.generate(&["floor.yaml"]);
    case.run(
        r#"Check.RoundTrip<Floor>("{\"count\":1,\"ratio\":0.5}");
Check.ParseFails<Floor>("{\"count\":0,\"ratio\":0.25}", "count: must be >= 1, got 0", "ratio: must be >= 0.5, got 0.25");
Check.WriteFails(new Floor { Count = 0, Ratio = 0.25 }, "count: must be >= 1, got 0", "ratio: must be >= 0.5, got 0.25");"#,
    );
}

#[test]
fn csharp_json_maximum_is_inclusive_in_both_directions() {
    let case = JsonCase::new("maximum");
    case.write(
        "ceiling.yaml",
        "type: object\nproperties:\n  count: { type: integer, maximum: 10 }\n  ratio: { type: number, maximum: 2.5 }\n",
    );
    case.generate(&["ceiling.yaml"]);
    case.run(
        r#"Check.RoundTrip<Ceiling>("{\"count\":10,\"ratio\":2.5}");
Check.ParseFails<Ceiling>("{\"count\":11,\"ratio\":2.75}", "count: must be <= 10, got 11", "ratio: must be <= 2.5, got 2.75");
Check.WriteFails(new Ceiling { Count = 11, Ratio = 2.75 }, "count: must be <= 10, got 11", "ratio: must be <= 2.5, got 2.75");"#,
    );

    // A fractional bound on an integer field is a load error, not a
    // comparison C# would round.
    let fractional = JsonCase::new("maximum-fractional");
    fractional.write(
        "ceiling.yaml",
        "type: object\nproperties:\n  count: { type: integer, maximum: 5.5 }\n",
    );
    assert_has(
        &fractional.generate_err(&["ceiling.yaml"]),
        &["`maximum` must be an integer bound on an `integer` field"],
    );
}

#[test]
fn csharp_json_exclusive_minimum_excludes_the_bound() {
    let case = JsonCase::new("exclusive-minimum");
    case.write(
        "positive.yaml",
        "type: object\nproperties:\n  count: { type: integer, exclusiveMinimum: 1 }\n  ratio: { type: number, exclusiveMinimum: 0 }\n",
    );
    case.generate(&["positive.yaml"]);
    case.run(
        r#"Check.RoundTrip<Positive>("{\"count\":2,\"ratio\":0.5}");
Check.ParseFails<Positive>("{\"count\":1,\"ratio\":0}", "count: must be > 1, got 1", "ratio: must be > 0, got 0");
Check.ParseFails<Positive>("{\"ratio\":-0}", "ratio: must be > 0");
Check.WriteFails(new Positive { Count = 1, Ratio = 0 }, "count: must be > 1, got 1", "ratio: must be > 0, got 0");"#,
    );
}

#[test]
fn csharp_json_number_multiple_of_is_ieee_fmod() {
    let case = JsonCase::new("multiple-of");
    case.write(
        "multiples.yaml",
        r#"type: object
properties:
  byFive: { type: number, multipleOf: 5 }
  byThree: { type: number, multipleOf: 3 }
  byTwo: { type: integer, multipleOf: 2 }
"#,
    );
    let output = case.generate(&["multiples.yaml"]);
    // `double % double` (IEEE fmod, as the other targets), never `decimal`.
    assert_lacks(output.file("Models.cs"), &["decimal"]);
    case.run(
        r#"Check.ParseFails<Multiples>("{\"byFive\":1e23}", "byFive: must be a multiple of 5");
Check.RoundTrip<Multiples>("{\"byThree\":1e300}", "{\"byThree\":1E+300}");
Check.ParseFails<Multiples>("{\"byThree\":1e22}", "byThree: must be a multiple of 3");
Check.RoundTrip<Multiples>("{\"byFive\":10}");
Check.RoundTrip<Multiples>("{\"byFive\":-0}");
Check.ParseFails<Multiples>("{\"byFive\":7}", "byFive: must be a multiple of 5");
Check.ParseFails<Multiples>("{\"byTwo\":3}", "byTwo: must be a multiple of 2");
Check.WriteFails(new Multiples { ByFive = 7 }, "byFive: must be a multiple of 5");"#,
    );
}

#[test]
fn csharp_json_negative_zero_keeps_its_sign_and_matches_zero() {
    let case = JsonCase::new("negative-zero");
    case.write(
        "zero.yaml",
        r#"type: object
properties:
  ratio: { type: number }
  zeroConst: { type: number, const: 0 }
  ratios: { type: array, uniqueItems: true, items: { type: number } }
"#,
    );
    case.generate(&["zero.yaml"]);
    case.run(
        r#"var parsed = Check.RoundTrip<Zero>("{\"ratio\":-0}");
Check.True(double.IsNegative(parsed.Ratio!.Value), "-0 keeps its sign");
Check.RoundTrip<Zero>("{\"zeroConst\":-0}", "{\"zeroConst\":0}");
Check.ParseFails<Zero>("{\"ratios\":[0,-0]}", "ratios: duplicate items");
Check.WriteFails(new Zero { Ratios = new[] { 0.0, -0.0 } }, "ratios: duplicate items");"#,
    );
}

#[test]
fn csharp_json_large_portable_counts_compile() {
    let case = JsonCase::new("large-counts");
    case.write(
        "counts.yaml",
        r##"type: object
minProperties: 2147483648
maxProperties: 4294967295
properties:
  keys: { $ref: "#/$defs/Keys" }
  text: { type: string, minLength: 2147483648, maxLength: 4294967295 }
  values:
    type: array
    minItems: 2147483648
    maxItems: 4294967295
    items: { type: string }
    contains: { type: string, minLength: 2147483648 }
    minContains: 2147483648
    maxContains: 4294967295
$defs:
  Keys:
    type: object
    additionalProperties: { type: string }
    propertyNames: { type: string, minLength: 2147483648, maxLength: 4294967295 }
"##,
    );
    let output = case.generate(&["counts.yaml"]);
    assert_has(
        output.file("Models.cs"),
        &["must have at least 2147483648 items", "4294967295"],
    );
    // Every bound past `int.MaxValue` must still compile against `int` counts.
    case.build();
}

#[test]
fn csharp_json_contains_matchers_are_complete_and_typed() {
    let case = JsonCase::new("contains");
    case.write(
        "matchers.yaml",
        r#"type: object
properties:
  integral: { type: array, items: { type: number }, contains: { type: integer, minimum: 2, multipleOf: 2 } }
  emails:
    type: array
    items: { type: string }
    contains: { type: string, minLength: 6, pattern: "@example\\.com$", format: email }
  names: { type: array, items: { type: string }, contains: { minLength: 2 } }
  nums: { type: array, items: { type: integer }, contains: { minimum: 5 } }
  ordered: { type: array, items: { type: number }, contains: { enum: [2, 1.5] } }
  reversed: { type: array, items: { type: number }, contains: { enum: [1.5, 2] } }
  bounded:
    type: array
    items: { type: string }
    contains: { const: ok }
    minContains: 1
    maxContains: 1
"#,
    );
    let output = case.generate(&["matchers.yaml"]);
    // A member's matcher regexes compile once, into static fields.
    assert_has(
        output.file("Models.cs"),
        &[
            "private static readonly global::System.Text.RegularExpressions.Regex EmailsContainsPattern = new(",
            "private static readonly global::System.Text.RegularExpressions.Regex EmailsContainsFormat = new(",
        ],
    );
    case.run(
        r#"Check.RoundTrip<Matchers>("{\"integral\":[1,4],\"emails\":[\"x\",\"a@example.com\"],\"names\":[\"a\",\"bc\"],\"nums\":[1,9]}");
Check.ParseFails<Matchers>("{\"integral\":[2.5,3]}", "integral: no element matches the required schema");
Check.ParseFails<Matchers>("{\"emails\":[\"a@example.org\"]}", "emails: no element matches the required schema");
Check.ParseFails<Matchers>("{\"names\":[\"a\"]}", "names: no element matches the required schema");
// A mistyped element is its own violation and never matches.
Check.ParseFails<Matchers>("{\"names\":[5]}", "names[0]: ", "names: no element matches the required schema");
Check.ParseFails<Matchers>("{\"nums\":[\"9\"]}", "nums[0]: ", "nums: no element matches the required schema");
// The matcher kind follows the element type, not the order of its literals.
Check.RoundTrip<Matchers>("{\"ordered\":[1.5],\"reversed\":[1.5]}");
Check.ParseFails<Matchers>("{\"bounded\":[\"ok\",\"ok\"]}", "bounded: ");
Check.ParseFails<Matchers>("{\"bounded\":[]}", "bounded: ");
Check.WriteFails(new Matchers { Nums = new long[] { 1 } }, "nums: no element matches the required schema");
Check.WriteFails(new Matchers { Bounded = new[] { "ok", "ok" } }, "bounded: ");"#,
    );
}

#[test]
fn csharp_json_max_items_bounds_the_element_count() {
    let case = JsonCase::new("max-items");
    case.write(
        "tagged.yaml",
        "type: object\nproperties:\n  tags: { type: array, items: { type: string }, maxItems: 2 }\n",
    );
    case.generate(&["tagged.yaml"]);
    case.run(
        r#"Check.RoundTrip<Tagged>("{\"tags\":[\"a\",\"b\"]}");
Check.ParseFails<Tagged>("{\"tags\":[\"a\",\"b\",\"c\"]}", "tags: must have at most 2 items, got 3");
Check.WriteFails(new Tagged { Tags = new[] { "a", "b", "c" } }, "tags: must have at most 2 items, got 3");"#,
    );
}

#[test]
fn csharp_json_min_contains_counts_matches() {
    let case = JsonCase::new("min-contains");
    case.write(
        "votes.yaml",
        r#"type: object
properties:
  ballots: { type: array, items: { type: string }, contains: { const: yes }, minContains: 2 }
  optional: { type: array, items: { type: string }, contains: { const: yes }, minContains: 0, maxContains: 1 }
"#,
    );
    case.generate(&["votes.yaml"]);
    case.run(
        r#"Check.RoundTrip<Votes>("{\"ballots\":[\"yes\",\"no\",\"yes\"]}");
Check.ParseFails<Votes>("{\"ballots\":[\"yes\",\"no\"]}", "ballots: too few matching items: at least 2, got 1");
Check.WriteFails(new Votes { Ballots = new[] { "yes" } }, "ballots: too few matching items: at least 2, got 1");
// `minContains: 0` lets a list with no match through.
Check.RoundTrip<Votes>("{\"optional\":[]}");
Check.RoundTrip<Votes>("{\"optional\":[\"no\"]}");"#,
    );

    // Alone, `minContains: 0` would make `contains` assert nothing.
    let vacuous = JsonCase::new("min-contains-vacuous");
    vacuous.write(
        "votes.yaml",
        "type: object\nproperties:\n  ballots: { type: array, items: { type: string }, contains: { const: yes }, minContains: 0 }\n",
    );
    assert_has(
        &vacuous.generate_err(&["votes.yaml"]),
        &["`minContains: 0` without a `maxContains`"],
    );
}

#[test]
fn csharp_json_max_contains_counts_matches() {
    let case = JsonCase::new("max-contains");
    case.write(
        "votes.yaml",
        "type: object\nproperties:\n  ballots: { type: array, items: { type: string }, contains: { const: yes }, maxContains: 1 }\n",
    );
    case.generate(&["votes.yaml"]);
    case.run(
        r#"Check.RoundTrip<Votes>("{\"ballots\":[\"yes\",\"no\"]}");
Check.ParseFails<Votes>("{\"ballots\":[\"yes\",\"yes\"]}", "ballots: too many matching items: at most 1, got 2");
// The implied `minContains: 1` still requires a match.
Check.ParseFails<Votes>("{\"ballots\":[\"no\"]}", "ballots: no element matches the required schema");
Check.WriteFails(new Votes { Ballots = new[] { "yes", "yes" } }, "ballots: too many matching items: at most 1, got 2");"#,
    );
}

// ---------------------------------------------------------------------------
// Objects
// ---------------------------------------------------------------------------

#[test]
fn csharp_json_typed_additional_properties_are_native_and_collision_safe() {
    let case = JsonCase::new("typed-extras");
    case.write(
        "mixed.yaml",
        r##"type: object
minProperties: 2
maxProperties: 3
required: [id]
properties:
  id: { type: string }
additionalProperties: { $ref: "#/$defs/Extra" }
$defs:
  Extra:
    type: object
    additionalProperties: false
    required: [score]
    properties: { score: { type: integer, minimum: 1 } }
  Dates:
    type: object
    additionalProperties: { type: array, items: { type: string, format: date } }
"##,
    );
    let output = case.generate(&["mixed.yaml"]);
    let models = output.file("Models.cs");
    assert_has(
        models,
        &[
            "IReadOnlyDictionary<string, Extra> AdditionalProperties",
            "IReadOnlyDictionary<string, global::System.Collections.Generic.IReadOnlyList<global::System.DateOnly>> AdditionalProperties",
        ],
    );
    case.run(
        r#"var parsed = Check.RoundTrip<Mixed>("{\"id\":\"x\",\"bonus\":{\"score\":2}}");
Check.Equal(2L, parsed.AdditionalProperties["bonus"].Score, "typed extra");
Check.ParseFails<Mixed>("{\"id\":\"x\",\"bonus\":{\"score\":0}}", "bonus.score: must be >= 1");
Check.ParseFails<Mixed>("{\"id\":\"x\"}", "must have at least 2 properties");
Check.ParseFails<Mixed>(
    "{\"id\":\"x\",\"a\":{\"score\":1},\"b\":{\"score\":1},\"c\":{\"score\":1}}",
    "must have at most 3 properties");
// A catch-all key that names a declared member would emit it twice.
Check.WriteFails(
    new Mixed { Id = "x", AdditionalProperties = new System.Collections.Generic.Dictionary<string, Extra> { ["id"] = new Extra { Score = 1 } } },
    "id: declared property key collision",
    "must have at least 2 properties");
Check.WriteFails(new Mixed { Id = "x" }, "must have at least 2 properties");
var dates = Check.RoundTrip<Dates>("{\"due\":[\"2024-02-29\"]}");
Check.Equal(new System.DateOnly(2024, 2, 29), dates.AdditionalProperties["due"][0], "date extra");"#,
    );
}

#[test]
fn csharp_json_object_keywords_validate_both_ways() {
    let case = JsonCase::new("object-keywords");
    case.write(
        "shapes.nexusrpc.yaml",
        r#"nexusrpc: "1.0.0"
$defs:
  Defaulted:
    type: object
    properties:
      status: { type: string, enum: [active, inactive], default: active }
  Contact:
    type: object
    additionalProperties: false
    minProperties: 1
    maxProperties: 3
    dependentRequired: { email: [name] }
    properties:
      name: { type: string }
      email: { type: string }
      phone: { type: string }
  Trigger:
    type: object
    properties: { fire: { type: string } }
    dependentRequired: { fire: [] }
  Open:
    type: object
    properties: { a: { type: string } }
  Empty:
    type: object
    additionalProperties: false
"#,
    );
    let output = case.generate(&["shapes.nexusrpc.yaml"]);
    // An enum default is typed as the closed value class.
    assert_has(
        output.file("Models.cs"),
        &["public StatusValue StatusOrDefault => this.Status ?? StatusValue.Active;"],
    );
    case.run(
        r#"Check.Equal(Defaulted.StatusValue.Active, Check.Parse<Defaulted>("{}").StatusOrDefault, "enum default");
Check.ParseFails<Contact>("{}", "must have at least 1 properties");
Check.ParseFails<Contact>("{\"email\":\"e\"}", "name: ");
Check.RoundTrip<Contact>("{\"name\":\"n\",\"email\":\"e\"}");
Check.WriteFails(new Contact(), "must have at least 1 properties");
Check.WriteFails(new Contact { Email = "e" }, "name: ");
Check.RoundTrip<Trigger>("{\"fire\":\"t\"}");
Check.ParseFails<Empty>("{\"a\":1}", "a: unknown field");
Check.RoundTrip<Empty>("{}");
var open = Check.RoundTrip<Open>("{\"a\":\"x\",\"extra\":{\"n\":1.50}}");
Check.Equal("{\"n\":1.50}", open.AdditionalProperties["extra"].GetRawText(), "unknown members stay verbatim");
Check.WriteFails(
    new Open { AdditionalProperties = new System.Collections.Generic.Dictionary<string, System.Text.Json.JsonElement> { ["a"] = System.Text.Json.JsonDocument.Parse("1").RootElement } },
    "a: ");"#,
    );
}

#[test]
fn csharp_json_min_properties_counts_distinct_keys() {
    let case = JsonCase::new("min-properties");
    case.write(
        "pair.yaml",
        "type: object\nminProperties: 2\nproperties:\n  a: { type: integer }\nadditionalProperties: { type: integer }\n",
    );
    case.generate(&["pair.yaml"]);
    case.run(
        r#"Check.RoundTrip<Pair>("{\"a\":1,\"b\":2}");
Check.ParseFails<Pair>("{\"a\":1}", "must have at least 2 properties, got 1");
// A repeated key is one property.
Check.ParseFails<Pair>("{\"a\":1,\"a\":2}", "must have at least 2 properties, got 1");
Check.WriteFails(new Pair { A = 1 }, "must have at least 2 properties, got 1");"#,
    );
}

#[test]
fn csharp_json_max_properties_counts_declared_and_extra_keys() {
    let case = JsonCase::new("max-properties");
    case.write(
        "pair.yaml",
        "type: object\nmaxProperties: 2\nproperties:\n  a: { type: integer }\nadditionalProperties: { type: integer }\n",
    );
    case.generate(&["pair.yaml"]);
    case.run(
        r#"Check.RoundTrip<Pair>("{\"a\":1,\"b\":2}");
Check.ParseFails<Pair>("{\"a\":1,\"b\":2,\"c\":3}", "must have at most 2 properties, got 3");
Check.WriteFails(
    new Pair { A = 1, AdditionalProperties = new System.Collections.Generic.Dictionary<string, long> { ["b"] = 2, ["c"] = 3 } },
    "must have at most 2 properties, got 3");"#,
    );
}

/// `allOf` merges at load: the C# output is the one record the merged schema
/// lowers to, with every branch's constraints and no trace of the combinator.
#[test]
fn csharp_json_all_of_merges_into_one_record() {
    let case = JsonCase::new("all-of");
    case.write(
        "member.yaml",
        r##"allOf:
  - { $ref: "#/$defs/Named" }
  - type: object
    required: [age]
    properties:
      age: { type: integer, minimum: 0 }
      code: { allOf: [{ type: string, minLength: 2 }, { minLength: 3, maxLength: 4 }] }
$defs:
  Named:
    type: object
    required: [name]
    properties: { name: { type: string, minLength: 1 } }
"##,
    );
    let output = case.generate(&["member.yaml"]);
    let models = output.file("Models.cs");
    assert_has(
        models,
        &[
            "public sealed record Member\n",
            "public required string Name { get; init; }",
            "public required long Age { get; init; }",
            "public string? Code { get; init; }",
        ],
    );
    assert_lacks(models, &["AllOf"]);
    case.run(
        r#"Check.RoundTrip<Member>("{\"name\":\"n\",\"age\":1,\"code\":\"abc\"}");
Check.ParseFails<Member>("{\"age\":1}", "name: required");
Check.ParseFails<Member>("{\"name\":\"\",\"age\":-1,\"code\":\"ab\"}",
    "name: must have length >= 1",
    "age: must be >= 0",
    "code: must have length >= 3");
Check.WriteFails(new Member { Name = "n", Age = 1, Code = "abcde" }, "code: must have length <= 4");"#,
    );
}

#[test]
fn csharp_json_repaths_nested_violations_on_serialize() {
    let case = JsonCase::new("nested-paths");
    case.write(
        "nested_path.yaml",
        r##"type: object
required: [address]
properties:
  address: { $ref: "#/$defs/Addr" }
  addresses: { type: array, items: { $ref: "#/$defs/Addr" } }
  byName: { type: object, additionalProperties: { $ref: "#/$defs/Addr" } }
  label: { type: string, minLength: 4 }
$defs:
  Addr:
    type: object
    properties: { zip: { type: string, minLength: 5 } }
"##,
    );
    case.generate(&["nested_path.yaml"]);
    case.run(
        r#"Check.WriteFails(
    new NestedPath
    {
        Address = new Addr { Zip = "1" },
        Addresses = new[] { new Addr { Zip = "12345" }, new Addr { Zip = "2" } },
        ByName = new NestedPathByName { AdditionalProperties = new System.Collections.Generic.Dictionary<string, Addr> { ["home"] = new Addr { Zip = "3" } } },
        Label = "abc",
    },
    "address.zip: must have length >= 5",
    "addresses[1].zip: must have length >= 5",
    "byName.home.zip: must have length >= 5",
    "label: must have length >= 4");
Check.WriteFails(new NestedPath { Address = null! }, "address: required");
Check.ParseFails<NestedPath>(
    "{\"address\":{\"zip\":\"1\"},\"addresses\":[{\"zip\":2}]}",
    "address.zip: must have length >= 5",
    "addresses[0].zip: ");"#,
    );
}

#[test]
fn csharp_json_property_names_matcher_is_complete() {
    let case = JsonCase::new("property-names");
    case.write(
        "names.yaml",
        r#"type: object
additionalProperties: { type: integer }
propertyNames:
  type: string
  minLength: 6
  maxLength: 32
  pattern: "@example\\.com$"
  format: email
  enum: [a@example.com, b@example.com]
"#,
    );
    case.generate(&["names.yaml"]);
    case.run(
        r#"Check.RoundTrip<Names>("{\"a@example.com\":1}");
Check.ParseFails<Names>("{\"c@example.com\":1}", "[\"c@example.com\"]: invalid property name \"c@example.com\": must be one of");
// Every failing predicate is reported, each under the key's prefix.
Check.ParseFails<Names>(
    "{\"a@example.org\":1}",
    "[\"a@example.org\"]: invalid property name \"a@example.org\": must match pattern",
    "[\"a@example.org\"]: invalid property name \"a@example.org\": must be one of");
Check.WriteFails(
    new Names { AdditionalProperties = new System.Collections.Generic.Dictionary<string, long> { ["x"] = 1 } },
    "x: invalid property name \"x\": must have length >= 6",
    "x: invalid property name \"x\": must match pattern",
    "x: invalid property name \"x\": must be a valid email",
    "x: invalid property name \"x\": must be one of");"#,
    );
}

#[test]
fn csharp_json_duplicate_keys_keep_the_last_occurrence() {
    let case = JsonCase::new("duplicate-keys");
    case.write(
        "dup.yaml",
        "type: object\nproperties:\n  a: { type: string }\n",
    );
    case.generate(&["dup.yaml"]);
    case.run(r#"Check.RoundTrip<Dup>("{\"a\":\"x\",\"a\":\"y\"}", "{\"a\":\"y\"}");"#);
}

#[test]
fn csharp_json_wire_keys_need_not_be_identifiers() {
    let case = JsonCase::new("wire-keys");
    case.write(
        "wire_keys.yaml",
        r#"type: object
required: [content-type]
properties:
  content-type: { type: string }
  class: { type: string }
  event: { type: integer }
  "@type": { type: string }
  a.b: { type: string, minLength: 2 }
"#,
    );
    let output = case.generate(&["wire_keys.yaml"]);
    assert_has(
        output.file("Models.cs"),
        &[
            "public required string ContentType { get; init; }",
            "public string? Class { get; init; }",
            "public long? Event { get; init; }",
            "public string? Type { get; init; }",
            "public string? AB { get; init; }",
        ],
    );
    case.run(
        r#"Check.RoundTrip<WireKeys>("{\"content-type\":\"t\",\"class\":\"c\",\"event\":1,\"@type\":\"x\",\"a.b\":\"ab\"}");
// A key that is not identifier-like is bracket-quoted in the path (P11.2).
Check.ParseFails<WireKeys>("{\"content-type\":\"t\",\"a.b\":\"a\"}", "[\"a.b\"]: must have length >= 2");
Check.ParseFails<WireKeys>("{}", "[\"content-type\"]: required");"#,
    );
}

/// The naming policy on a caller's `JsonSerializerOptions` cannot rename wire
/// keys: each converter writes every member name itself.
#[test]
fn csharp_json_converters_own_the_wire_keys() {
    let case = JsonCase::new("naming-policy");
    case.write(
        "policy.yaml",
        "type: object\nproperties:\n  firstName: { type: string }\n  zip_code: { type: string }\n",
    );
    case.generate(&["policy.yaml"]);
    case.run(
        r#"var value = new Policy { FirstName = "a", ZipCode = "b" };
foreach (var policy in new[] { System.Text.Json.JsonNamingPolicy.SnakeCaseLower, System.Text.Json.JsonNamingPolicy.KebabCaseUpper })
{
    var options = new System.Text.Json.JsonSerializerOptions { PropertyNamingPolicy = policy };
    Check.Equal("{\"firstName\":\"a\",\"zip_code\":\"b\"}", System.Text.Json.JsonSerializer.Serialize(value, options), "naming policy");
    Check.Equal(value, System.Text.Json.JsonSerializer.Deserialize<Policy>("{\"firstName\":\"a\",\"zip_code\":\"b\"}", options), "naming policy read");
}"#,
    );
}

/// Every violation lands in one non-retryable `PayloadValidationError`
/// application failure (P11), in both directions.
#[test]
fn csharp_json_violations_aggregate_into_one_application_failure() {
    let case = JsonCase::new("failure-shape");
    case.write(
        "shape.yaml",
        r#"type: object
required: [a, b]
properties:
  a: { type: string, minLength: 2 }
  b: { type: integer, maximum: 1 }
  c: { type: array, items: { type: string, maxLength: 1 } }
"#,
    );
    let output = case.generate(&["shape.yaml"]);
    assert_has(
        output.file("Violation.cs"),
        &["public sealed record Violation(string Path, string Reason)"],
    );
    case.run(
        r#"var payload = new Temporalio.Api.Common.V1.Payload
{
    Metadata = { ["encoding"] = Google.Protobuf.ByteString.CopyFromUtf8("json/plain") },
    Data = Google.Protobuf.ByteString.CopyFromUtf8("{\"a\":\"x\",\"b\":2,\"c\":[\"ok\",\"no\"]}"),
};
try
{
    Temporalio.Converters.DataConverter.Default.PayloadConverter.ToValue(payload, typeof(Shape));
    throw new System.Exception("expected a failure");
}
catch (Temporalio.Exceptions.ApplicationFailureException failure)
{
    Check.Equal("PayloadValidationError", failure.ErrorType, "type");
    Check.Equal("Payload validation failed", failure.Message, "message");
    Check.True(failure.NonRetryable, "non-retryable");
    var violations = failure.Details.ElementAt<System.Collections.Generic.IReadOnlyList<Violation>>(0);
    Check.Equal(4, violations.Count, "every violation collected");
    Check.Equal(new Violation("c[0]", "must have length <= 1, got 2"), violations[2], "violation record");
}
Check.WriteFails(new Shape { A = "x", B = 2 }, "a: must have length >= 2", "b: must be <= 1");
Check.Equal("{\"a\":\"ab\",\"b\":1}", Check.Canonical(Check.Write(new Shape { A = "ab", B = 1 })), "valid value writes");"#,
    );
}

// ---------------------------------------------------------------------------
// Materialized formats
// ---------------------------------------------------------------------------

#[test]
fn csharp_json_materialized_formats_use_native_types_at_every_position() {
    let case = JsonCase::new("materialized");
    case.write(
        "materialized.yaml",
        r##"type: object
properties:
  stamp: { type: string, format: date-time }
  day: { type: string, format: date }
  clock: { type: string, format: time }
  span: { type: string, format: duration }
  blob: { type: string, contentEncoding: base64 }
  urlBlob: { type: string, contentEncoding: base64url }
  stamps: { type: array, items: { type: string, format: date-time } }
  days: { type: array, items: { type: array, items: { type: string, format: date } } }
  byKey: { $ref: "#/$defs/ByKey" }
  maybeStamps: { type: array, items: { oneOf: [{ type: string, format: date-time }, { type: "null" }] } }
$defs:
  ByKey:
    type: object
    additionalProperties: { type: array, items: { type: string, format: duration } }
"##,
    );
    let output = case.generate(&["materialized.yaml"]);
    assert_has(
        output.file("Models.cs"),
        &[
            "public global::System.DateTimeOffset? Stamp { get; init; }",
            "public global::System.DateOnly? Day { get; init; }",
            // No BCL type holds both offset-bearing and offset-less times.
            "public string? Clock { get; init; }",
            "public global::System.TimeSpan? Span { get; init; }",
            "public byte[]? Blob { get; init; }",
            "IReadOnlyList<global::System.DateTimeOffset>? Stamps { get; init; }",
            "IReadOnlyList<global::System.Collections.Generic.IReadOnlyList<global::System.DateOnly>>? Days { get; init; }",
            "IReadOnlyDictionary<string, global::System.Collections.Generic.IReadOnlyList<global::System.TimeSpan>> AdditionalProperties",
            // A nullable value-type element is `Nullable<T>`.
            "IReadOnlyList<global::System.DateTimeOffset?>? MaybeStamps { get; init; }",
        ],
    );
    case.run(
        r#"var parsed = Check.RoundTrip<Materialized>(
    "{\"stamp\":\"2024-01-02T03:04:05.5+05:30\",\"day\":\"2024-02-29\",\"clock\":\"12:30:00Z\",\"span\":\"PT1H30M\",\"blob\":\"AQI=\",\"urlBlob\":\"-_8\",\"stamps\":[\"2024-01-02T03:04:05Z\"],\"days\":[[\"2024-01-01\"]],\"byKey\":{\"k\":[\"PT5S\"]},\"maybeStamps\":[null,\"2024-01-02T03:04:05Z\"]}");
Check.Equal(new System.TimeSpan(5, 30, 0), parsed.Stamp!.Value.Offset, "offset is kept");
Check.Equal(new System.DateOnly(2024, 2, 29), parsed.Day!.Value, "date");
Check.Equal(System.TimeSpan.FromMinutes(90), parsed.Span!.Value, "duration");
Check.Equal(System.TimeSpan.FromSeconds(5), parsed.ByKey!.AdditionalProperties["k"][0], "map duration");
Check.True(parsed.UrlBlob is [0xFB, 0xFF], "base64url");
// Duration and time spellings normalize to the canonical wire form.
Check.RoundTrip<Materialized>("{\"span\":\"PT90M\"}", "{\"span\":\"PT1H30M\"}");
Check.RoundTrip<Materialized>("{\"span\":\"PT3600S\"}", "{\"span\":\"PT1H\"}");
Check.ParseFails<Materialized>("{\"day\":\"2023-02-29\"}", "day: must be a valid date");
Check.ParseFails<Materialized>("{\"stamps\":[\"2024-01-02\"]}", "stamps[0]: must be a valid date-time");
Check.ParseFails<Materialized>("{\"days\":[[\"x\"]]}", "days[0][0]: must be a valid date");
Check.ParseFails<Materialized>("{\"byKey\":{\"k\":[\"P1M\"]}}", "byKey.k[0]: ");
Check.ParseFails<Materialized>("{\"blob\":\"AQI\"}", "blob: ");
Check.ParseFails<Materialized>("{\"urlBlob\":\"+/8=\"}", "urlBlob: ");
// What the wire cannot carry is caught before writing.
Check.WriteFails(new Materialized { Span = System.TimeSpan.FromSeconds(-1) }, "span: ");
Check.WriteFails(new Materialized { Span = System.TimeSpan.FromMilliseconds(1500) }, "span: ");
// `DateTimeOffset` itself refuses sub-minute and beyond-±14:00 offsets and
// years past 9999, so those wire-grammar limits need no serialize check here.
Check.WriteFails(new Materialized { Clock = "25:00:00Z" }, "clock: ");"#,
    );
}

#[test]
fn csharp_json_runtime_support_is_emitted_only_when_used() {
    let case = JsonCase::new("runtime-support");
    // Prose that names the runtime types selects nothing.
    case.write(
        "prose.yaml",
        r#"description: DateTimeOffset and TemporalSupport are only mentioned here.
type: object
properties:
  note: { type: string, description: "A base64 TimeSpan mention." }
"#,
    );
    let output = case.generate(&["prose.yaml"]);
    assert!(!output.has_file("TemporalSupport.cs"));
    assert!(!output.has_file("Base64Support.cs"));
    assert_has(
        output.file("Models.cs"),
        &["DateTimeOffset and TemporalSupport"],
    );
    case.build();

    // A materialized value nested only inside a collection or map still needs it.
    let nested = JsonCase::new("runtime-support-nested");
    nested.write(
        "nested.yaml",
        r##"type: object
properties:
  dates: { type: array, items: { type: array, items: { type: string, format: date } } }
  blobs: { $ref: "#/$defs/Blobs" }
$defs:
  Blobs:
    type: object
    additionalProperties: { type: string, contentEncoding: base64 }
"##,
    );
    let output = nested.generate(&["nested.yaml"]);
    assert!(output.has_file("TemporalSupport.cs"));
    assert!(output.has_file("Base64Support.cs"));
    nested.build();
}

#[test]
fn csharp_json_temporal_constraints_cover_original_and_canonical_wire_values() {
    let case = JsonCase::new("temporal-constraints");
    case.write(
        "dates.yaml",
        r##"type: object
additionalProperties: false
required: [day, days, byName]
properties:
  day: { type: string, format: date, minLength: 10, pattern: "^2024-" }
  days: { type: array, items: { type: string, format: date, pattern: "^2024-" } }
  byName: { $ref: "#/$defs/DateMap" }
$defs:
  DateMap:
    type: object
    additionalProperties: { type: string, format: date, pattern: "^2024-" }
"##,
    );
    case.generate(&["dates.yaml"]);
    case.run(
        r#"Check.RoundTrip<Dates>("{\"day\":\"2024-01-01\",\"days\":[\"2024-02-02\"],\"byName\":{\"x\":\"2024-03-03\"}}");
Check.ParseFails<Dates>(
    "{\"day\":\"2023-01-01\",\"days\":[\"2023-02-02\"],\"byName\":{\"x\":\"2023-03-03\"}}",
    "day: must match pattern",
    "days[0]: must match pattern",
    "byName.x: must match pattern");
var old = new System.DateOnly(2023, 1, 1);
Check.WriteFails(
    new Dates { Day = old, Days = new[] { old }, ByName = new DateMap { AdditionalProperties = new System.Collections.Generic.Dictionary<string, System.DateOnly> { ["x"] = old } } },
    "day: must match pattern",
    "days[0]: must match pattern",
    "byName.x: must match pattern");"#,
    );
}

#[test]
fn csharp_json_materialized_closed_values_and_defaults_stay_native() {
    let case = JsonCase::new("materialized-closed");
    case.write(
        "closed.yaml",
        r#"type: object
additionalProperties: false
required: [epoch, token]
properties:
  epoch: { type: string, format: date, const: "2024-01-01" }
  token: { type: string, contentEncoding: base64, enum: ["YQ==", "Yg=="] }
  fallbackDay: { type: string, format: date, default: "2024-02-03" }
  fallbackBytes: { type: string, contentEncoding: base64, default: "Yw==" }
  fallbackSpan:
    oneOf: [{ type: string, format: duration }, { type: "null" }]
    default: PT1M
"#,
    );
    let output = case.generate(&["closed.yaml"]);
    assert_has(
        output.file("Models.cs"),
        &[
            "public global::System.DateOnly FallbackDayOrDefault =>",
            "public byte[] FallbackBytesOrDefault =>",
            "public global::System.TimeSpan FallbackSpanOrDefault =>",
        ],
    );
    case.run(
        r#"var parsed = Check.RoundTrip<Closed>("{\"epoch\":\"2024-01-01\",\"token\":\"YQ==\"}");
Check.Equal(new System.DateOnly(2024, 1, 1), parsed.Epoch, "date const");
Check.True(parsed.Token is [(byte)'a'], "bytes enum");
Check.Equal(new System.DateOnly(2024, 2, 3), parsed.FallbackDayOrDefault, "date default");
Check.True(parsed.FallbackBytesOrDefault is [(byte)'c'], "bytes default");
Check.Equal(System.TimeSpan.FromMinutes(1), parsed.FallbackSpanOrDefault, "nullable default");
Check.ParseFails<Closed>("{\"epoch\":\"2024-01-02\",\"token\":\"Yw==\"}", "epoch: must equal \"2024-01-01\"", "token: must be one of");
// A const on a materialized type compares canonical wire forms.
Check.ParseFails<Closed>(
    "{\"epoch\":\"2024-1-01\",\"token\":\"YQ==\"}",
    "epoch: must equal \"2024-01-01\"",
    "epoch: must be a valid date");"#,
    );
}

#[test]
fn csharp_json_materialized_values_compare_on_the_wire() {
    let case = JsonCase::new("wire-comparison");
    case.write(
        "wire.yaml",
        r#"type: object
required: [blob]
properties:
  blob: { type: string, contentEncoding: base64, minLength: 4, maxLength: 8 }
  blobConst: { type: string, contentEncoding: base64, const: "aGk=" }
  times: { type: array, uniqueItems: true, items: { type: string, format: date-time } }
  blobs: { type: array, uniqueItems: true, items: { type: string, contentEncoding: base64 } }
"#,
    );
    case.generate(&["wire.yaml"]);
    case.run(
        r#"Check.RoundTrip<Wire>("{\"blob\":\"aGkhIQ==\",\"blobConst\":\"aGk=\"}");
// Length bounds count the base64 text, not the decoded bytes.
Check.WriteFails(new Wire { Blob = new byte[] { 1, 2, 3, 4, 5, 6, 7 } }, "blob: must have length <= 8");
Check.WriteFails(new Wire { Blob = new byte[] { 1, 2, 3 }, BlobConst = new byte[] { 1 } }, "blobConst: must equal \"aGk=\"");
// One instant at two offsets is two wire values, so the elements are unique,
// although DateTimeOffset.Equals calls them equal.
var instant = new System.DateTimeOffset(2024, 1, 1, 0, 0, 0, System.TimeSpan.Zero);
var shifted = instant.ToOffset(System.TimeSpan.FromHours(1));
Check.True(instant.Equals(shifted), "DateTimeOffset equality is by instant");
Check.RoundTrip<Wire>("{\"blob\":\"AAAA\",\"times\":[\"2024-01-01T00:00:00Z\",\"2024-01-01T01:00:00+01:00\"]}");
Check.Write(new Wire { Blob = new byte[] { 0, 0, 0 }, Times = new[] { instant, shifted } });
Check.WriteFails(new Wire { Blob = new byte[] { 0, 0, 0 }, Times = new[] { instant, instant } }, "times: duplicate items");
// Byte arrays compare by content, not by reference.
Check.WriteFails(
    new Wire { Blob = new byte[] { 0, 0, 0 }, Blobs = new[] { new byte[] { 1 }, new byte[] { 1 } } },
    "blobs: duplicate items");
Check.ParseFails<Wire>("{\"blob\":\"AAAA\",\"blobs\":[\"AQ==\",\"AQ==\"]}", "blobs: duplicate items");"#,
    );
}

/// .NET's `$` (like Python's) also matches before a final newline, so every
/// generated anchor must be `\z`, in authored patterns and in the temporal
/// grammars alike — and a mismatch must be a violation, not an exception.
#[test]
fn csharp_json_trailing_newline_is_rejected_everywhere() {
    let case = JsonCase::new("trailing-newline");
    case.write(
        "newline.yaml",
        r#"type: object
properties:
  word: { type: string, pattern: "^[a-z]+$" }
  stamp: { type: string, format: date-time }
  day: { type: string, format: date }
  clock: { type: string, format: time }
  span: { type: string, format: duration }
  mail: { type: string, format: email }
  host: { type: string, format: hostname }
  id: { type: string, format: uuid }
"#,
    );
    let output = case.generate(&["newline.yaml"]);
    for (path, file) in &output.0 {
        for line in file
            .lines()
            .filter(|line| line.contains("new global::System.Text.RegularExpressions.Regex("))
        {
            assert!(
                line.contains("\\\\z\""),
                "{}: an anchor that is not `\\z`:\n{line}",
                path.display()
            );
        }
    }
    case.run(
        r#"foreach (var (member, valid) in new[]
{
    ("word", "abc"), ("stamp", "2024-01-01T00:00:00Z"), ("day", "2024-01-01"), ("clock", "12:00:00Z"),
    ("span", "PT1H"), ("mail", "a@example.com"), ("host", "example.com"), ("id", "123e4567-e89b-12d3-a456-426614174000"),
})
{
    Check.RoundTrip<Newline>("{\"" + member + "\":\"" + valid + "\"}");
    Check.ParseFails<Newline>("{\"" + member + "\":\"" + valid + "\\n\"}", member + ": ");
}
Check.WriteFails(new Newline { Word = "abc\n" }, "word: must match pattern");
Check.WriteFails(new Newline { Clock = "12:00:00Z\n" }, "clock: ");"#,
    );
}

/// `DateTimeOffset` resolves 100 ns, so a longer fraction is truncated to
/// seven digits — P1 exception (b), the open `09#8` divergence shared with
/// Python's microsecond `datetime`. Offsets reach exactly ±14:00.
#[test]
fn csharp_json_fractional_seconds_truncate_to_dotnet_resolution() {
    let case = JsonCase::new("fractional-seconds");
    case.write(
        "fractional.yaml",
        r#"type: object
properties:
  ts: { type: string, format: date-time }
  wideDefault: { type: string, format: date-time, default: "2021-06-15t12:30:45.123456789012+14:00" }
  minusBoundary: { type: string, format: date-time, default: "2021-06-15T12:30:45-14:00" }
"#,
    );
    let output = case.generate(&["fractional.yaml"]);
    assert_has(
        output.file("Models.cs"),
        &["TemporalSupport.ParseDateTimeLiteral(\"2021-06-15T12:30:45.123456789012+14:00\")"],
    );
    case.run(
        r#"Check.RoundTrip<Fractional>(
    "{\"ts\":\"2021-06-15T12:30:45.123456789Z\"}",
    "{\"ts\":\"2021-06-15T12:30:45.1234567Z\"}");
Check.RoundTrip<Fractional>("{\"ts\":\"2021-06-15T12:30:45.1234567Z\"}");
// Trailing fractional zeros are not canonical.
Check.RoundTrip<Fractional>("{\"ts\":\"2021-06-15T12:30:45.500Z\"}", "{\"ts\":\"2021-06-15T12:30:45.5Z\"}");
Check.RoundTrip<Fractional>("{\"ts\":\"2021-06-15T12:30:45+14:00\"}");
Check.RoundTrip<Fractional>("{\"ts\":\"2021-06-15T12:30:45-14:00\"}");
Check.ParseFails<Fractional>("{\"ts\":\"2021-06-15T12:30:45+14:01\"}", "ts: must be a valid date-time");
var defaults = Check.Parse<Fractional>("{}");
Check.Equal(
    new System.DateTimeOffset(2021, 6, 15, 12, 30, 45, System.TimeSpan.FromHours(14)).AddTicks(1234567),
    defaults.WideDefaultOrDefault,
    "a wide default literal truncates");
Check.Equal(System.TimeSpan.FromHours(14), defaults.WideDefaultOrDefault.Offset, "+14:00 default");
Check.Equal(System.TimeSpan.FromHours(-14), defaults.MinusBoundaryOrDefault.Offset, "-14:00 default");"#,
    );

    // An authored default outside the shared offset domain is a load error.
    let invalid = JsonCase::new("fractional-seconds-invalid");
    invalid.write(
        "fractional.yaml",
        "type: object\nproperties:\n  ts: { type: string, format: date-time, default: \"2021-06-15T12:30:45+14:01\" }\n",
    );
    assert_has(
        &invalid.generate_err(&["fractional.yaml"]),
        &["is not a valid date-time", "+14:01"],
    );
}

// ---------------------------------------------------------------------------
// .NET string and equality semantics
// ---------------------------------------------------------------------------

#[test]
fn csharp_json_min_length_is_inclusive_in_both_directions() {
    let case = JsonCase::new("min-length");
    case.write(
        "handle.yaml",
        "type: object\nproperties:\n  name: { type: string, minLength: 2 }\n",
    );
    case.generate(&["handle.yaml"]);
    case.run(
        r#"Check.RoundTrip<Handle>("{\"name\":\"ab\"}");
Check.ParseFails<Handle>("{\"name\":\"a\"}", "name: must have length >= 2, got 1");
Check.WriteFails(new Handle { Name = "" }, "name: must have length >= 2, got 0");"#,
    );

    // Bounds no string satisfies are a load error.
    let empty = JsonCase::new("min-length-empty");
    empty.write(
        "handle.yaml",
        "type: object\nproperties:\n  name: { type: string, minLength: 3, maxLength: 2 }\n",
    );
    assert_has(
        &empty.generate_err(&["handle.yaml"]),
        &["`minLength` (3) exceeds `maxLength` (2)"],
    );
}

/// `string.Length` counts UTF-16 code units; length bounds count code points.
#[test]
fn csharp_json_lengths_count_code_points_not_utf16_units() {
    let case = JsonCase::new("code-points");
    case.write(
        "lengths.yaml",
        r##"type: object
properties:
  short: { type: string, maxLength: 4 }
  long: { type: string, minLength: 2 }
  tags: { type: array, items: { type: string }, contains: { type: string, maxLength: 1 } }
  keyed: { $ref: "#/$defs/Keyed" }
$defs:
  Keyed:
    type: object
    additionalProperties: { type: integer }
    propertyNames: { type: string, maxLength: 1 }
"##,
    );
    let output = case.generate(&["lengths.yaml"]);
    assert_lacks(output.file("Models.cs"), &[".Length <", ".Length >"]);
    case.run(
        r#"// "a😀b" is three code points but four UTF-16 units.
Check.True("a😀b".Length == 4, "UTF-16 length");
Check.RoundTrip<Lengths>("{\"short\":\"a😀bc\"}");
Check.ParseFails<Lengths>("{\"short\":\"a😀bcd\"}", "short: must have length <= 4, got 5");
Check.ParseFails<Lengths>("{\"long\":\"😀\"}", "long: must have length >= 2, got 1");
Check.RoundTrip<Lengths>("{\"tags\":[\"😀\"]}");
Check.RoundTrip<Lengths>("{\"keyed\":{\"😀\":1}}");
Check.WriteFails(new Lengths { Long = "😀" }, "long: must have length >= 2, got 1");"#,
    );
}

/// .NET regexes match UTF-16 code units; the portable contract matches code
/// points, so negated classes and astral members must consume a whole
/// surrogate pair.
#[test]
fn csharp_json_patterns_match_code_points() {
    let case = JsonCase::new("astral-patterns");
    case.write(
        "patterns.yaml",
        r#"type: object
properties:
  one: { type: string, pattern: "^.$" }
  notA: { type: string, pattern: "^[^a]$" }
  emoji: { type: string, pattern: "^[😀-😂]+$" }
  repeat: { type: string, pattern: "^😀{2}$" }
"#,
    );
    case.generate(&["patterns.yaml"]);
    case.run(
        r#"Check.RoundTrip<Patterns>("{\"one\":\"😀\",\"notA\":\"😀\",\"emoji\":\"😀😁😂\",\"repeat\":\"😀😀\"}");
Check.ParseFails<Patterns>("{\"one\":\"😀😀\"}", "one: must match pattern");
Check.ParseFails<Patterns>("{\"notA\":\"a\"}", "notA: must match pattern");
Check.ParseFails<Patterns>("{\"emoji\":\"😃\"}", "emoji: must match pattern");
Check.ParseFails<Patterns>("{\"repeat\":\"😀\"}", "repeat: must match pattern");"#,
    );
}

/// A record's compiler-generated equality compares arrays, lists and
/// `JsonElement`s by reference; the generated `Equals`/`GetHashCode` compare
/// them by content, so a value equals its own round trip.
#[test]
fn csharp_json_record_equality_is_structural() {
    let case = JsonCase::new("record-equality");
    case.write(
        "equality.yaml",
        r#"type: object
properties:
  blob: { type: string, contentEncoding: base64 }
  tags: { type: array, items: { type: string } }
  grid: { type: array, items: { type: array, items: { type: integer } } }
  union: { oneOf: [{ type: array, items: { type: string } }, { type: integer }] }
"#,
    );
    case.generate(&["equality.yaml"]);
    case.run(
        r#"const string wire = "{\"blob\":\"AQI=\",\"tags\":[\"a\"],\"grid\":[[1]],\"union\":[\"x\"],\"extra\":{\"n\":[1,2]}}";
var left = Check.Parse<Equality>(wire);
var right = Check.Parse<Equality>(wire);
Check.True(!ReferenceEquals(left.Blob, right.Blob), "distinct arrays");
Check.Equal(left, right, "equal by content");
Check.Equal(left.GetHashCode(), right.GetHashCode(), "hash by content");
Check.Equal(left, Check.Parse<Equality>(Check.Write(left)), "a value equals its round trip");
Check.True(left != right with { Blob = new byte[] { 1, 3 } }, "different bytes");
Check.True(left != Check.Parse<Equality>("{\"blob\":\"AQI=\",\"tags\":[\"a\"],\"grid\":[[1]],\"union\":[\"x\"],\"extra\":{\"n\":[1,3]}}"), "different extras");"#,
    );
}

// ---------------------------------------------------------------------------
// Modules, namespaces and references
// ---------------------------------------------------------------------------

/// A two-file closure: a service whose operation output and a model member
/// both reference a model declared, and renamed, in another file.
fn write_cross_module_closure(case: &JsonCase) {
    case.write(
        "kb.nexusrpc.yaml",
        r##"nexusrpc: "1.0.0"
services:
  Pages:
    fqn: example.pages.v1.Pages
    operations:
      get:
        input: { $ref: "#/$defs/GetInput" }
        output: { $ref: "content/page.json" }
      find:
        input: { $ref: "#/$defs/GetInput" }
        output: { $ref: "#/$defs/FindOutput" }
$defs:
  GetInput:
    type: object
    additionalProperties: false
    properties: { id: { type: string } }
  FindOutput:
    type: object
    additionalProperties: false
    properties:
      page: { $ref: "content/page.json" }
      leaf: { $ref: "deep/nested/leaf.yaml" }
"##,
    );
    case.write(
        "content/page.json",
        r#"{"type":"object","x-cs-name":"RenamedPage","additionalProperties":false,"properties":{"title":{"type":"string","minLength":2}}}"#,
    );
    case.write(
        "deep/nested/leaf.yaml",
        "type: object\nproperties:\n  n: { type: integer, minimum: 0 }\n",
    );
}

#[test]
fn csharp_json_namespaces_follow_module_paths() {
    let case = JsonCase::new("namespaces");
    write_cross_module_closure(&case);
    let output = case.generate_native(&["."]);
    // One namespace per module below the root; the runtime types once, at the root.
    assert_has(output.file("kb/Models.cs"), &["namespace Probe.Kb\n"]);
    assert_has(output.file("kb/Services.cs"), &["namespace Probe.Kb\n"]);
    assert_has(
        output.file("content/page/Models.cs"),
        &["namespace Probe.Content.Page\n"],
    );
    assert_has(
        output.file("deep/nested/leaf/Models.cs"),
        &["namespace Probe.Deep.Nested.Leaf\n"],
    );
    for runtime in ["Violation.cs", "SpecJson.cs", "ValueEquality.cs"] {
        assert_has(output.file(runtime), &["namespace Probe\n"]);
        assert_eq!(
            output
                .0
                .keys()
                .filter(|path| path.ends_with(runtime))
                .count(),
            1,
            "{runtime} is emitted once"
        );
    }
    // A cross-module reference is `global::`-qualified, so neither a module's
    // own type names nor a namespace that shares a type's name can capture it.
    assert_has(
        output.file("kb/Models.cs"),
        &[
            "public global::Probe.Content.Page.RenamedPage? Page { get; init; }",
            "public global::Probe.Deep.Nested.Leaf.Leaf? Leaf { get; init; }",
        ],
    );
    case.run_with_usings(
        &["Probe.Kb"],
        r#"var parsed = Check.RoundTrip<FindOutput>("{\"page\":{\"title\":\"ok\"},\"leaf\":{\"n\":1}}");
Check.Equal("ok", parsed.Page!.Title, "cross-module member");
Check.ParseFails<FindOutput>("{\"page\":{\"title\":\"x\"},\"leaf\":{\"n\":-1}}",
    "page.title: must have length >= 2",
    "leaf.n: must be >= 0");
// The runtime types are shared by every module.
Check.True(typeof(Probe.Violation).Namespace == "Probe", "one runtime namespace");"#,
    );
}

#[test]
fn csharp_json_cross_module_cs_name_override_moves_every_reference() {
    let case = JsonCase::new("cross-module-override");
    write_cross_module_closure(&case);
    let output = case.generate_native(&["."]);
    assert_has(
        output.file("content/page/Models.cs"),
        &["public sealed record RenamedPage\n"],
    );
    assert_has(
        output.file("kb/Services.cs"),
        &[
            "global::Probe.Content.Page.RenamedPage Get(GetInput request);",
            "public async Task<global::Probe.Content.Page.RenamedPage> GetAsync(GetInput request)",
        ],
    );
    let all = output.all();
    assert_lacks(&all, &["record Page", ".Page.Page", " Page Get("]);
    case.build();
}

#[test]
fn csharp_json_bare_ref_aliases_collapse_to_the_target_type() {
    let case = JsonCase::new("bare-ref-alias");
    common::write_bare_ref_alias_closure(&case.root);
    let output = case.generate_native(&["."]);
    let all = output.all();
    // Neither the file-root alias nor the `$ref: "#"` definition is a type.
    assert_lacks(
        &all,
        &["record Alternate", "record Mirror", "interface Alternate"],
    );
    assert_has(
        output.file("service/Services.cs"),
        &["global::Probe.Target.Main.Main Echo(global::Probe.Target.Main.Main request);"],
    );
    assert_has(
        output.file("service/Models.cs"),
        &["public required global::Probe.Target.Main.Main Item { get; init; }"],
    );
    case.run_with_usings(
        &["Probe.Service"],
        r#"Check.RoundTrip<Holder>("{\"item\":{\"value\":\"ok\"}}");
Check.ParseFails<Holder>("{\"item\":{}}", "item.value: required");"#,
    );
}

#[test]
fn csharp_json_same_module_alias_chain_resolves_to_the_target() {
    let case = JsonCase::new("alias-chain");
    case.write(
        "chain.yaml",
        r##"type: object
additionalProperties: false
required: [item, longItem]
properties:
  item: { $ref: "#/$defs/A" }
  longItem: { $ref: "#/$defs/LongA" }
$defs:
  A: { $ref: "#/$defs/Z" }
  Z: { $ref: "#/$defs/Target" }
  LongA: { $ref: "#/$defs/LongM" }
  LongM: { $ref: "#/$defs/LongZ" }
  LongZ: { $ref: "#/$defs/Target" }
  Target:
    type: object
    additionalProperties: false
    required: [value]
    properties: { value: { type: string } }
"##,
    );
    let output = case.generate(&["chain.yaml"]);
    assert_has(
        output.file("Models.cs"),
        &[
            "public required Target Item { get; init; }",
            "public required Target LongItem { get; init; }",
        ],
    );
    case.run(
        r#"Check.RoundTrip<Chain>("{\"item\":{\"value\":\"a\"},\"longItem\":{\"value\":\"b\"}}");
Check.ParseFails<Chain>("{\"item\":{},\"longItem\":{\"value\":\"b\"}}", "item.value: required");"#,
    );
}

/// Each module is its own namespace, so the same type name may appear in two
/// modules — but a service file names its operation I/O types through
/// `using` directives, where two `Page`s are ambiguous, so that one scope is a
/// P15 reject with the `x-cs-name` fix-it.
#[test]
fn csharp_json_same_type_name_in_two_modules() {
    let page = r#"{"type":"object","additionalProperties":false,"required":["title"],"properties":{"title":{"type":"string"}}}"#;
    let models = JsonCase::new("same-name-models");
    models.write("a/page.json", page).write("b/page.json", page).write(
        "root.yaml",
        "type: object\nproperties:\n  first: { $ref: a/page.json }\n  second: { $ref: b/page.json }\n",
    );
    let output = models.generate(&["."]);
    assert_has(
        output.file("root/Models.cs"),
        &[
            "public global::Probe.A.Page.Page? First { get; init; }",
            "public global::Probe.B.Page.Page? Second { get; init; }",
        ],
    );
    models.run_with_usings(
        &["Probe.Root"],
        r#"Check.RoundTrip<Root>("{\"first\":{\"title\":\"a\"},\"second\":{\"title\":\"b\"}}");"#,
    );

    let service = JsonCase::new("same-name-service");
    let root = r#"nexusrpc: "1.0.0"
services:
  Svc:
    fqn: example.v1.Svc
    operations:
      one: { input: { $ref: a/page.json } }
      two: { input: { $ref: b/page.json } }
"#;
    service
        .write("a/page.json", page)
        .write("b/page.json", page)
        .write("root.nexusrpc.yaml", root);
    assert_has(
        &service.generate_err(&["."]),
        &["collision", "a/page#Page", "b/page#Page", "x-cs-name"],
    );
    service.write(
        "b/page.json",
        r#"{"type":"object","x-cs-name":"BPage","additionalProperties":false,"required":["title"],"properties":{"title":{"type":"string"}}}"#,
    );
    let output = service.generate_native(&["."]);
    assert_has(
        output.file("b/page/Models.cs"),
        &["public sealed record BPage\n"],
    );
    service.build();
}

#[test]
fn csharp_json_service_module_without_own_types_does_not_reemit_refs() {
    let case = JsonCase::new("service-only-module");
    case.write(
        "a/page.json",
        r#"{"type":"object","additionalProperties":false,"required":["title"],"properties":{"title":{"type":"string"}}}"#,
    )
    .write(
        "b/note.json",
        r#"{"type":"object","additionalProperties":false,"required":["body"],"properties":{"body":{"type":"string"}}}"#,
    )
    .write(
        "svc.nexusrpc.yaml",
        r#"nexusrpc: "1.0.0"
services:
  Svc:
    fqn: example.v1.Svc
    operations:
      one:
        input: { $ref: a/page.json }
        output: { $ref: b/note.json }
"#,
    );
    let output = case.generate_native(&["."]);
    let all = output.all();
    assert_eq!(
        all.matches("public sealed record Page\n").count(),
        1,
        "{all}"
    );
    assert_eq!(
        all.matches("public sealed record Note\n").count(),
        1,
        "{all}"
    );
    assert_has(
        output.file("svc/Services.cs"),
        &["global::Probe.B.Note.Note One(global::Probe.A.Page.Page request);"],
    );
    case.build();
}

// ---------------------------------------------------------------------------
// Names
// ---------------------------------------------------------------------------

#[test]
fn csharp_json_override_moves_member_derived_names_only() {
    let case = JsonCase::new("member-derived-names");
    case.write(
        "probe.yaml",
        r#"type: object
properties:
  retryCount: { type: integer, default: 3, x-cs-name: Attempts }
  kind: { type: string, const: widget, x-cs-name: Category }
  choice: { oneOf: [{ type: string }, { type: integer }], x-cs-name: Pick }
  address:
    type: object
    x-cs-name: Location
    properties: { street: { type: string } }
"#,
    );
    let output = case.generate(&["probe.yaml"]);
    let models = output.file("Models.cs");
    assert_has(
        models,
        &[
            // Names derived from the member follow the override...
            "public long AttemptsOrDefault => this.Attempts ?? 3L;",
            "public sealed class CategoryValue",
            "public CategoryValue? Category { get; init; }",
            "public interface PickUnion",
            // ...while a hoisted shape keeps its position-derived name (P15).
            "public ProbeAddress? Location { get; init; }",
            "public sealed record ProbeAddress\n",
        ],
    );
    assert_lacks(
        models,
        &[
            "RetryCountOrDefault",
            "KindValue",
            "ChoiceUnion",
            "ProbeLocation",
        ],
    );
    case.run(
        r#"var parsed = Check.RoundTrip<Probe.Probe>("{\"retryCount\":5,\"kind\":\"widget\",\"choice\":1,\"address\":{\"street\":\"s\"}}");
Check.Equal(5L, parsed.AttemptsOrDefault, "renamed default accessor");
Check.Equal(Probe.Probe.CategoryValue.Widget, parsed.Category, "renamed const");"#,
    );
}

#[test]
fn csharp_json_names_value_constants_from_the_value() {
    let case = JsonCase::new("value-constants");
    case.write(
        "naming.yaml",
        r#"type: object
properties:
  kind: { type: string, const: showcase }
  enabled: { type: boolean, const: true }
  tier: { type: integer, enum: [1, -2] }
  scale: { type: number, enum: [1.5, -2.25] }
  status: { type: string, enum: [active, "multi-word_ish"] }
"#,
    );
    let output = case.generate(&["naming.yaml"]);
    assert_has(
        output.file("Models.cs"),
        &[
            "public static readonly KindValue Showcase = new(\"showcase\");",
            "public static readonly EnabledValue True = new(true);",
            "public static readonly TierValue V1 = new(1L);",
            "public static readonly TierValue VNeg2 = new(-2L);",
            "public static readonly ScaleValue V1_5 = new(1.5);",
            "public static readonly ScaleValue VNeg2_25 = new(-2.25);",
            "public static readonly StatusValue Active = new(\"active\");",
            "public static readonly StatusValue MultiWordIsh = new(\"multi-word_ish\");",
        ],
    );
    case.run(
        r#"var parsed = Check.RoundTrip<Naming>("{\"kind\":\"showcase\",\"enabled\":true,\"tier\":-2,\"scale\":1.5,\"status\":\"multi-word_ish\"}");
Check.Equal(Naming.TierValue.VNeg2, parsed.Tier, "integer constant");
Check.True(ReferenceEquals(Naming.StatusValue.MultiWordIsh, parsed.Status), "parsed values are the constants");
Check.ParseFails<Naming>("{\"tier\":3,\"enabled\":false}", "tier: must be one of", "enabled: must equal true");"#,
    );

    // A numeric constant's name does not depend on how the literal is spelled.
    for spelling in ["1", "1.0", "1e0"] {
        let case = JsonCase::new("value-constant-spelling");
        case.write(
            "spelling.yaml",
            &format!(
                "type: object\nproperties:\n  score: {{ type: integer, const: {spelling} }}\n"
            ),
        );
        assert_has(
            case.generate(&["spelling.yaml"]).file("Models.cs"),
            &["public static readonly ScoreValue V1 = new(1L);"],
        );
    }
}

#[test]
fn csharp_json_enum_name_overrides_reach_non_string_members() {
    let case = JsonCase::new("enum-names");
    case.write(
        "enum_names.yaml",
        r#"type: object
properties:
  tier: { type: integer, enum: [1, 2], x-cs-enum-names: { "1": Bronze, "2": Silver } }
  toggle: { type: boolean, enum: [true, false], x-cs-enum-names: { "true": On, "false": Off } }
  scale: { type: number, enum: [1.5, 2.5], x-cs-enum-names: { "1.5": Half, "2.5": TwoHalf } }
  mode: { type: string, enum: [fast, slow], x-cs-enum-names: { fast: Quick } }
  edition: { type: integer, const: 7, x-cs-const-name: Lucky }
"#,
    );
    let output = case.generate(&["enum_names.yaml"]);
    let models = output.file("Models.cs");
    assert_has(
        models,
        &[
            "public static readonly TierValue Bronze = new(1L);",
            "public static readonly TierValue Silver = new(2L);",
            "public static readonly ToggleValue On = new(true);",
            "public static readonly ToggleValue Off = new(false);",
            "public static readonly ScaleValue Half = new(1.5);",
            "public static readonly ScaleValue TwoHalf = new(2.5);",
            "public static readonly ModeValue Quick = new(\"fast\");",
            // An entry the map does not name keeps its derived name.
            "public static readonly ModeValue Slow = new(\"slow\");",
            "public static readonly EditionValue Lucky = new(7L);",
        ],
    );
    assert_lacks(
        models,
        &["TierValue V1 ", "ToggleValue True ", "ScaleValue V1_5 "],
    );
    case.build();

    let collide = JsonCase::new("enum-names-collide");
    collide.write(
        "collide.yaml",
        "type: object\nproperties:\n  tier: { type: integer, enum: [1, 2], x-cs-enum-names: { \"1\": Same, \"2\": Same } }\n",
    );
    assert_has(
        &collide.generate_err(&["collide.yaml"]),
        &["both map to `Same`"],
    );
}

/// Every identifier the C# emitter owns is reserved where it lives, and a
/// schema that would reuse one is a P15 load reject naming `x-cs-name` —
/// never a silent rename and never a compile error.
#[test]
fn csharp_json_rejects_names_the_emitter_reserves() {
    let cases: &[(&str, &str)] = &[
        // Runtime types, in every module scope, used or not.
        (
            "type: object\nproperties:\n  x: { $ref: \"#/$defs/Violation\" }\n$defs:\n  Violation: { type: object, properties: { a: { type: string } } }\n",
            "generated runtime identifier `Violation`",
        ),
        (
            "type: object\nproperties:\n  x: { $ref: \"#/$defs/SpecJson\" }\n$defs:\n  SpecJson: { type: object, properties: { a: { type: string } } }\n",
            "generated runtime identifier `SpecJson`",
        ),
        (
            "type: object\nproperties:\n  x: { $ref: \"#/$defs/ValueEquality\" }\n$defs:\n  ValueEquality: { type: object, properties: { a: { type: string } } }\n",
            "generated runtime identifier `ValueEquality`",
        ),
        (
            "type: object\nproperties:\n  x: { $ref: \"#/$defs/TemporalSupport\" }\n$defs:\n  TemporalSupport: { type: object, properties: { a: { type: string } } }\n",
            "generated runtime identifier `TemporalSupport`",
        ),
        (
            "type: object\nproperties:\n  x: { $ref: \"#/$defs/Base64Support\" }\n$defs:\n  Base64Support: { type: object, properties: { a: { type: string } } }\n",
            "generated runtime identifier `Base64Support`",
        ),
        // Members a record or its converter already declares.
        (
            "type: object\nproperties:\n  converter: { type: string }\n",
            "generated record member `Converter`",
        ),
        (
            "type: object\nproperties:\n  equals: { type: string }\n",
            "generated record member `Equals`",
        ),
        (
            "type: object\nproperties:\n  getHashCode: { type: string }\n",
            "generated record member `GetHashCode`",
        ),
        (
            "type: object\nproperties:\n  toString: { type: string }\n",
            "generated record member `ToString`",
        ),
        (
            "type: object\nproperties:\n  equalityContract: { type: string }\n",
            "generated record member `EqualityContract`",
        ),
        (
            "type: object\nproperties:\n  deconstruct: { type: string }\n",
            "generated record member `Deconstruct`",
        ),
        (
            "type: object\nproperties:\n  additionalProperties: { type: string }\n",
            "additional-properties catch-all",
        ),
        // A member named after its enclosing type (CS0542).
        (
            "type: object\nproperties:\n  model: { type: string }\n",
            "`Model`",
        ),
        // Names the service file imports.
        (
            "nexusrpc: \"1.0.0\"\nservices:\n  Svc:\n    operations:\n      run: { input: { $ref: \"#/$defs/Task\" } }\n$defs:\n  Task: { type: object, properties: { a: { type: string } } }\n",
            "generated service-file import `Task`",
        ),
        // An override that cannot be a C# identifier.
        (
            "type: object\nproperties:\n  a: { type: string, x-cs-name: 1st }\n",
            "x-cs-name",
        ),
        (
            "type: object\nproperties:\n  a: { type: string, x-cs-name: \"\" }\n",
            "x-cs-name",
        ),
    ];
    // Members of every converter, which would capture a same-module type
    // name inside a converter body (CS0119/CS0120).
    for member in [
        "CanConvert",
        "GetType",
        "HandleNull",
        "MemberwiseClone",
        "Parse",
        "Read",
        "ReadAsPropertyName",
        "ReferenceEquals",
        "Type",
        "Validate",
        "Write",
        "WriteAsPropertyName",
        "WriteValue",
    ] {
        let case = JsonCase::new("reserved-converter-member");
        case.write(
            "model.yaml",
            &format!(
                "type: object\nproperties:\n  x: {{ $ref: \"#/$defs/{member}\" }}\n$defs:\n  {member}: {{ type: object, properties: {{ a: {{ type: string }} }} }}\n"
            ),
        );
        assert_has(
            &case.generate_err(&["model.yaml"]),
            &[
                &format!("generated converter member `{member}`"),
                "x-cs-name",
            ],
        );
    }
    // Inherited `object` members, which a record member or a closed-value
    // constant would hide (CS0108).
    for (property, member) in [
        ("getType", "GetType"),
        ("memberwiseClone", "MemberwiseClone"),
        ("referenceEquals", "ReferenceEquals"),
    ] {
        let case = JsonCase::new("reserved-object-member");
        case.write(
            "model.yaml",
            &format!("type: object\nproperties:\n  {property}: {{ type: string }}\n"),
        );
        assert_has(
            &case.generate_err(&["model.yaml"]),
            &[
                &format!("inherited `object` member `{member}`"),
                "x-cs-name",
            ],
        );
        let constant = JsonCase::new("reserved-object-member-constant");
        constant.write(
            "model.yaml",
            &format!(
                "type: object\nproperties:\n  mode: {{ type: string, enum: [{property}, other] }}\n"
            ),
        );
        assert_has(
            &constant.generate_err(&["model.yaml"]),
            &[
                &format!("inherited `object` member `{member}`"),
                "x-cs-enum-names",
            ],
        );
    }
    for (schema, expected) in cases {
        let case = JsonCase::new("reserved-name");
        let file = if schema.starts_with("nexusrpc") {
            "model.nexusrpc.yaml"
        } else {
            "model.yaml"
        };
        case.write(file, schema);
        let error = case.generate_err(&[file]);
        assert_has(&error, &[expected, "x-cs-name"]);
    }

    // The override is the fix-it.
    let fixed = JsonCase::new("reserved-name-fixed");
    fixed.write(
        "model.yaml",
        "type: object\nproperties:\n  converter: { type: string, x-cs-name: ConverterName }\n  model: { type: string, x-cs-name: ModelName }\n  getType: { type: string, x-cs-name: KindName }\n  mode: { type: string, enum: [referenceEquals, other], x-cs-enum-names: { referenceEquals: SameRef } }\n  type: { type: string }\n  read: { type: string }\n  x: { $ref: \"#/$defs/Type\" }\n$defs:\n  Type: { type: object, x-cs-name: TypeInfo, properties: { a: { type: string } } }\n",
    );
    fixed.generate(&["model.yaml"]);
    // Members spelled like converter members are fine: they are always
    // reached through an instance.
    fixed.run(
        r#"var parsed = Check.RoundTrip<Model>("{\"converter\":\"c\",\"model\":\"m\",\"getType\":\"g\",\"mode\":\"referenceEquals\",\"type\":\"t\",\"read\":\"r\",\"x\":{\"a\":\"b\"}}");
Check.Equal("c", parsed.ConverterName, "renamed member");
Check.Equal("g", parsed.KindName, "renamed object-member name");
Check.Equal(Model.ModeValue.SameRef, parsed.Mode, "renamed object-member constant");
Check.Equal("t", parsed.Type, "a member named Type");
Check.Equal("b", parsed.X!.A, "a renamed type");"#,
    );
}

/// Generated code spells every framework type `global::`-qualified, so a
/// schema may name its types after the BCL types the converters use.
#[test]
fn csharp_json_framework_type_names_do_not_shadow() {
    let names = [
        "String",
        "Object",
        "Exception",
        "Math",
        "Guid",
        "Uri",
        "Console",
        "Nullable",
        "Encoding",
        "List",
        "Dictionary",
        "HashSet",
        "IReadOnlyList",
        "IReadOnlyDictionary",
        "Enumerable",
        "JsonElement",
        "JsonDocument",
        "JsonValueKind",
        "JsonSerializer",
        "Utf8JsonWriter",
        "Utf8JsonReader",
        "JsonConverter",
        "Regex",
        "RegexOptions",
        "DateTimeOffset",
        "DateOnly",
        "TimeSpan",
        "Convert",
        "CultureInfo",
        "StringComparer",
        "IEquatable",
        "GeneratedCode",
        "Obsolete",
        "PayloadValidationError",
    ];
    let mut schema = String::from(
        "type: object\nrequired: [stamp]\nproperties:\n  stamp: { type: string, format: date-time }\n  day: { type: string, format: date }\n  span: { type: string, format: duration }\n  blob: { type: string, contentEncoding: base64 }\n  word: { type: string, pattern: \"^[a-z]+$\", minLength: 1 }\n  tags: { type: array, uniqueItems: true, items: { type: number } }\n",
    );
    for name in names {
        schema.push_str(&format!("  my{name}: {{ $ref: \"#/$defs/{name}\" }}\n"));
    }
    schema.push_str("$defs:\n");
    for name in names {
        schema.push_str(&format!(
            "  {name}:\n    type: object\n    properties:\n      value: {{ type: string, format: email }}\n      when: {{ type: string, format: date-time }}\n"
        ));
    }
    let case = JsonCase::new("framework-names");
    case.write("holder.yaml", &schema);
    case.generate(&["holder.yaml"]);
    case.run(
        r#"var parsed = Check.RoundTrip<Holder>(
    "{\"stamp\":\"2024-01-01T00:00:00Z\",\"day\":\"2024-01-01\",\"span\":\"PT1S\",\"blob\":\"AQI=\",\"word\":\"ab\",\"tags\":[1,2],\"myString\":{\"value\":\"a@example.com\"},\"myRegex\":{\"when\":\"2024-01-01T00:00:00Z\"}}");
Check.Equal("a@example.com", parsed.MyString!.Value, "a model named String");
Check.ParseFails<Holder>("{\"stamp\":\"x\",\"myJsonElement\":{\"value\":\"nope\"}}",
    "stamp: must be a valid date-time",
    "myJsonElement.value: must be a valid email");"#,
    );
}

/// Generated code spells the runtime support types fully qualified
/// (`global::<root namespace>.Violation`), which no member or type can capture,
/// so a member may take a runtime type's name.
#[test]
fn csharp_json_runtime_type_names_are_free_member_names() {
    let case = JsonCase::new("runtime-member-names");
    case.write(
        "holder.yaml",
        "type: object\nadditionalProperties: false\nrequired: [violation]\nproperties:\n  violation: { type: string, minLength: 2 }\n  specJson: { type: number }\n  valueEquality: { type: array, uniqueItems: true, items: { type: string } }\n  temporalSupport: { type: string, format: date-time }\n  base64Support: { type: string, contentEncoding: base64 }\n  nexgenRuntime: { $ref: \"#/$defs/NexgenRuntime\" }\n$defs:\n  NexgenRuntime:\n    type: object\n    additionalProperties: { type: integer }\n    properties:\n      violation: { type: integer }\n",
    );
    case.generate(&["holder.yaml"]);
    case.run(
        r#"var parsed = Check.RoundTrip<Holder>(
    "{\"violation\":\"vv\",\"specJson\":1.5,\"valueEquality\":[\"a\"],\"temporalSupport\":\"2024-01-01T00:00:00Z\",\"base64Support\":\"AQI=\",\"nexgenRuntime\":{\"violation\":1,\"extra\":2}}");
Check.Equal("vv", parsed.Violation, "a member named Violation");
Check.ParseFails<Holder>("{\"violation\":\"v\",\"valueEquality\":[\"a\",\"a\"],\"temporalSupport\":\"x\",\"nexgenRuntime\":{\"extra\":\"y\"}}",
    "violation: must have length >= 2",
    "valueEquality: duplicate items",
    "temporalSupport: must be a valid date-time",
    "nexgenRuntime.extra: expected integer");"#,
    );
}

// ---------------------------------------------------------------------------
// Services and documentation
// ---------------------------------------------------------------------------

const DEPRECATED_SERVICE_SCHEMA: &str = r##"nexusrpc: "1.0.0"
services:
  Legacy:
    fqn: example.legacy.v1.Legacy
    deprecated: true
    description: Legacy API.
    x-cs-name: LegacyCs
    operations:
      ping:
        deprecated: true
        description: Ping the service.
        x-cs-name: PingCs
      submit:
        input: { $ref: "#/$defs/Input" }
      fetch:
        output: { $ref: "#/$defs/Input" }
      archive:
        input: { $ref: "#/$defs/OldJob" }
        output: { $ref: "#/$defs/OldJob" }
$defs:
  Input:
    type: object
    additionalProperties: false
    properties: { id: { type: string } }
  OldJob:
    type: object
    deprecated: true
    properties: { oldId: { type: string, deprecated: true } }
"##;

#[test]
fn csharp_json_services_render_docs_deprecation_and_one_sided_io() {
    for native in [false, true] {
        let case = JsonCase::new("services");
        case.write("legacy.nexusrpc.yaml", DEPRECATED_SERVICE_SCHEMA);
        let output = if native {
            case.generate_native(&["legacy.nexusrpc.yaml"])
        } else {
            case.generate(&["legacy.nexusrpc.yaml"])
        };
        let services = output.file("Services.cs");
        // The contract is public unless `--native-api` puts a client in front.
        let visibility = if native { "internal" } else { "public" };
        assert_has(
            services,
            &[&format!(
                "    /// <summary>\n    /// Legacy API.\n    /// </summary>\n    [GeneratedCode(\"nexgen\", null)]\n    [global::System.Obsolete(\"This service is deprecated.\")]\n    [NexusService(\"example.legacy.v1.Legacy\")]\n    {visibility} interface ILegacyCs\n"
            )],
        );
        assert_has(
            services,
            &[
                // The service is documented and deprecated (above); overrides
                // rename the C# identifiers and keep the wire names.
                "        /// Ping the service.\n        /// </summary>\n        [GeneratedCode(\"nexgen\", null)]\n        [global::System.Obsolete(\"This operation is deprecated.\")]\n        [NexusOperation(\"Ping\")]\n        void PingCs();\n",
                // An absent input is a no-arg method; an absent output is `void`.
                "void Submit(Input request);",
                "Input Fetch();",
                "OldJob Archive(OldJob request);",
                // The file's own references to deprecated types stay quiet.
                "#pragma warning disable CS0618",
            ],
        );
        if native {
            assert_has(
                services,
                &[
                    "[global::System.Obsolete(\"This service is deprecated.\")]\n    public class LegacyCsClient",
                    "[global::System.Obsolete(\"This operation is deprecated.\")]\n        public async Task PingCsAsync()",
                    "public async Task<Input> FetchAsync()",
                ],
            );
        }
        // Builds with warnings as errors: no CS0618 from generated code.
        case.build();
    }
}

/// Descriptions become XML doc comments, which the compiler parses
/// (CS1570 under warnings as errors): markup characters must be escaped and
/// each line must stay inside its own `///` comment.
#[test]
fn csharp_json_xml_doc_comments_escape_markup() {
    let case = JsonCase::new("xml-docs");
    case.write(
        "docs.nexusrpc.yaml",
        r##"nexusrpc: "1.0.0"
services:
  Docs:
    description: "Service <b>bold</b> & \"quoted\" </summary>"
    operations:
      read:
        description: "Reads a <T> & returns it.\n*/ not a comment end\n/// not a doc comment"
        input: { $ref: "#/$defs/Doc" }
$defs:
  Doc:
    title: "Doc <title> & co"
    description: "First line.\n\nSecond paragraph with <c>code</c> & ampersand.\n</summary><summary>"
    type: object
    properties:
      legacy:
        type: string
        title: "Legacy id"
        description: "Use `id` instead; 1 < 2 > 0."
        deprecated: true
      plain: { type: string, description: "Ends with a quote \"" }
"##,
    );
    let output = case.generate_native(&["docs.nexusrpc.yaml"]);
    let models = output.file("Models.cs");
    assert_has(
        models,
        &[
            "/// Doc &lt;title&gt; &amp; co",
            "/// Second paragraph with &lt;c&gt;code&lt;/c&gt; &amp; ampersand.",
            "/// &lt;/summary&gt;&lt;summary&gt;",
            // A member's title, description and deprecation share one block.
            "/// Legacy id\n        /// <para/>\n        /// Use `id` instead; 1 &lt; 2 &gt; 0.\n        /// </summary>\n        [global::System.Obsolete(\"This field is deprecated.\")]\n        public string? Legacy { get; init; }",
        ],
    );
    assert_has(
        output.file("Services.cs"),
        &[
            "/// Service &lt;b&gt;bold&lt;/b&gt; &amp; \"quoted\" &lt;/summary&gt;",
            "/// */ not a comment end",
        ],
    );
    case.build();
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

/// A recursive type whose cycle crosses files, through `$defs` keys that need
/// JSON Pointer escaping.
#[test]
fn csharp_json_cross_file_cycles_resolve() {
    let case = JsonCase::new("cross-file-cycle");
    case.write(
        "a.yaml",
        r##"$defs:
  "a~1b":
    type: object
    properties:
      label: { type: string, minLength: 1 }
      link: { $ref: "b.yaml#/$defs/Bee" }
"##,
    )
    .write(
        "b.yaml",
        r##"$defs:
  Bee:
    type: object
    properties:
      back: { $ref: "a.yaml#/$defs/a~01b" }
      children: { type: array, items: { $ref: "#/$defs/Bee" } }
"##,
    );
    let output = case.generate(&["."]);
    assert_has(output.file("a/Models.cs"), &["public sealed record A1b\n"]);
    case.run_with_usings(
        &["Probe.A", "Probe.B"],
        r#"var parsed = Check.RoundTrip<A1b>("{\"label\":\"x\",\"link\":{\"back\":{\"label\":\"y\"},\"children\":[{\"children\":[]}]}}");
Check.Equal("y", parsed.Link!.Back!.Label, "cycle");
Check.ParseFails<A1b>("{\"link\":{\"back\":{\"label\":\"\"},\"children\":[{\"back\":{\"label\":\"\"}}]}}",
    "link.back.label: must have length >= 1",
    "link.children[0].back.label: must have length >= 1");"#,
    );
}

/// Member names that spell the locals and parameters a converter binds.
#[test]
fn csharp_json_property_names_never_shadow_converter_locals() {
    let names = [
        "reader",
        "writer",
        "options",
        "typeToConvert",
        "path",
        "violations",
        "value",
        "node",
        "members",
        "member",
        "element",
        "entry",
        "key",
        "item0",
        "index0",
        "items0",
        "wireKeys",
        "propertyName",
        "nameLength",
        "hash",
        "other",
        "result",
        "model0",
        "length0",
        "present0",
        "candidate",
        "matchCount0",
        "seen0",
        "keyed0",
        "parsed",
    ];
    let mut schema = String::from(
        "type: object\nadditionalProperties: true\nrequired: [violations]\nproperties:\n",
    );
    for name in names {
        schema.push_str(&format!("  {name}: {{ type: string, minLength: 2 }}\n"));
    }
    schema.push_str("  list: { type: array, uniqueItems: true, items: { type: string }, contains: { minLength: 3 } }\n");
    let case = JsonCase::new("converter-locals");
    case.write("locals.yaml", &schema);
    case.generate(&["locals.yaml"]);
    case.run(
        r#"Check.RoundTrip<Locals>("{\"reader\":\"rr\",\"writer\":\"ww\",\"path\":\"pp\",\"violations\":\"vv\",\"value\":\"xx\",\"list\":[\"abc\"],\"extra\":1}");
Check.ParseFails<Locals>("{\"violations\":\"v\",\"value\":\"x\",\"members\":\"m\",\"list\":[\"a\",\"a\"]}",
    "violations: must have length >= 2",
    "value: must have length >= 2",
    "members: must have length >= 2",
    "list: duplicate items",
    "list: no element matches the required schema");
Check.WriteFails(new Locals { Violations = "v", Hash = "h" }, "violations: must have length >= 2", "hash: must have length >= 2");"#,
    );
}

/// The lowercase identifiers generated code binds — converter parameters and
/// locals, record-method parameters and locals — are not reserved: an
/// `x-cs-name` override may spell any of them. Converter locals carry a
/// member-derived prefix or are bound before any member is read, members are
/// reached through an instance (`value.Member`), and record methods qualify
/// their own members with `this.`, so no override can capture, or be captured
/// by, a generated name. The names are harvested from the generated showcase
/// rather than listed, so a new local is covered the day it appears.
#[test]
fn csharp_json_overrides_may_spell_any_generated_local() {
    let schema =
        fs::read_to_string(project_root().join("samples/schemas/showcase.nexusrpc.yaml")).unwrap();

    // Harvest: every lowercase identifier in the generated models, outside
    // comments and string literals, that the loader accepts as an override
    // (which leaves out the C# keywords).
    let plain = JsonCase::new("generated-locals-harvest");
    plain.write("showcase.nexusrpc.yaml", &schema);
    let output = plain.generate(&["showcase.nexusrpc.yaml"]);
    let probe = JsonCase::new("generated-locals-probe");
    let names: Vec<String> = lowercase_identifiers(output.file("Models.cs"))
        .into_iter()
        .filter(|name| {
            probe.write(
                "probe.yaml",
                &format!("type: object\nproperties:\n  a: {{ type: string, x-cs-name: {name} }}\n"),
            );
            probe
                .try_generate(
                    &["probe.yaml"],
                    GenerationMode::DefinitionsOnly,
                    Some(NAMESPACE),
                )
                .is_ok()
        })
        .collect();
    for expected in [
        "reader",
        "writer",
        "options",
        "value",
        "path",
        "violations",
        "node",
        "other",
        "obj",
        "hash",
        "members",
        "item0",
    ] {
        assert!(
            names.iter().any(|name| name == expected),
            "harvest missed `{expected}`: {names:?}"
        );
    }

    // One showcase whose extra members are overridden to every harvested name.
    let mut extra = String::new();
    for (index, name) in names.iter().enumerate() {
        extra.push_str(&format!(
            "      z{index}: {{ type: string, minLength: 2, x-cs-name: {name} }}\n"
        ));
    }
    let anchor = "    additionalProperties: false\n    properties:\n      kind:\n";
    assert!(schema.contains(anchor), "showcase layout changed");
    let case = JsonCase::new("generated-locals-overrides");
    case.write(
        "showcase.nexusrpc.yaml",
        &schema.replacen(
            anchor,
            &format!("    additionalProperties: false\n    properties:\n{extra}      kind:\n"),
            1,
        ),
    );
    case.generate(&["showcase.nexusrpc.yaml"]);

    let mut program = String::from(
        r#"var wire = (System.Text.Json.Nodes.JsonObject)System.Text.Json.Nodes.JsonNode.Parse(
    """{"kind":"showcase","revision":1,"enabled":true,"status":"active","tier":1,"scale":1.5,"name":"Widget","count":3,"active":true,"category":"tools"}""")!;
"#,
    );
    program.push_str(&format!(
        "for (var index = 0; index < {}; index++)\n{{\n    wire[\"z\" + index] = \"vv\";\n}}\n",
        names.len()
    ));
    program.push_str(
        r#"var json = wire.ToJsonString();
var a = Check.Parse<Showcase>(json);
var b = Check.Parse<Showcase>(json);
var failures = new List<string>();
if (!a.Equals(b)) failures.Add("equal values compare unequal");
if (a.GetHashCode() != b.GetHashCode()) failures.Add("equal values hash differently");
if (!System.Text.Json.Nodes.JsonNode.DeepEquals(System.Text.Json.Nodes.JsonNode.Parse(Check.Write(a)), wire)) failures.Add("round trip changed the value");
string Short(string member) => json.Replace("\"" + member + "\":\"vv\"", "\"" + member + "\":\"v\"");
"#,
    );
    for (index, name) in names.iter().enumerate() {
        program.push_str(&format!(
            "if (a.Equals(a with {{ {name} = \"ww\" }})) failures.Add(\"Equals ignores `{name}`\");\n\
             if (!Check.ViolationsOf(() => Check.Parse<Showcase>(Short(\"z{index}\")), \"{name}\").Exists(v => v.StartsWith(\"z{index}:\"))) failures.Add(\"no violation at z{index} (`{name}`)\");\n"
        ));
    }
    program.push_str("Check.True(failures.Count == 0, string.Join(\"; \", failures));\n");
    case.run_with_usings(&["Probe", "System.Collections.Generic"], &program);
}

/// The lowercase identifiers of `source`, skipping comments and string and
/// character literals, in first-seen order.
fn lowercase_identifiers(source: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut chars = source.chars().peekable();
    let mut previous = ' ';
    while let Some(c) = chars.next() {
        match c {
            '/' if chars.peek() == Some(&'/') => {
                for next in chars.by_ref() {
                    if next == '\n' {
                        break;
                    }
                }
                previous = '\n';
            }
            '"' | '\'' => {
                while let Some(next) = chars.next() {
                    if next == '\\' {
                        chars.next();
                    } else if next == c {
                        break;
                    }
                }
                previous = c;
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                let mut word = String::from(c);
                while let Some(&next) = chars.peek() {
                    if next.is_ascii_alphanumeric() || next == '_' {
                        word.push(next);
                        chars.next();
                    } else {
                        break;
                    }
                }
                // Member accesses (`.x`) and alias-qualified names (`x::`)
                // are not bindings.
                if c.is_ascii_lowercase() && previous != '.' && !names.contains(&word) {
                    names.push(word);
                }
                previous = 'a';
            }
            _ => previous = c,
        }
    }
    names
}

/// A union's interface is public, so a caller can implement it; writing such a
/// value must be a violation at the member's path, not a crash.
#[test]
fn csharp_json_foreign_union_implementations_are_violations() {
    let case = JsonCase::new("foreign-union");
    case.write("bc.yaml", BRANCH_CONSTRAINT_SCHEMA);
    case.generate(&["bc.yaml"]);
    fs::write(
        case.root.join("Foreign.cs"),
        "namespace Probe\n{\n    public sealed class ForeignValue : Bc.ValueUnion { }\n}\n",
    )
    .unwrap();
    case.run(r#"Check.WriteFails(new Bc { Value = new ForeignValue() }, "value: ");"#);
}

#[test]
fn csharp_json_numeric_edges() {
    let case = JsonCase::new("numeric-edges");
    case.write(
        "edges.yaml",
        r#"type: object
properties:
  bounded: { type: number, exclusiveMaximum: 10, multipleOf: 5 }
  edge: { type: number, exclusiveMaximum: -1, multipleOf: 2e1 }
  integral: { type: array, items: { type: number }, contains: { type: integer } }
  matrix: { type: array, items: { type: array, uniqueItems: true, items: { type: integer } } }
"#,
    );
    case.generate(&["edges.yaml"]);
    case.run(
        r#"Check.ParseFails<Edges>("{\"bounded\":10}", "bounded: must be < 10");
Check.RoundTrip<Edges>("{\"bounded\":-15}");
Check.ParseFails<Edges>("{\"edge\":-0.9}", "edge: must be < -1", "edge: must be a multiple of 20");
Check.RoundTrip<Edges>("{\"edge\":-20}");
// The integer matcher sees the decimal text: 1e300 is integral but past the
// cap, 2^53+1 is past the cap, 2^53-1 matches.
Check.ParseFails<Edges>("{\"integral\":[1e300]}", "integral: no element matches the required schema");
Check.ParseFails<Edges>("{\"integral\":[9007199254740993]}", "integral: no element matches the required schema");
Check.RoundTrip<Edges>("{\"integral\":[9007199254740991]}");
Check.ParseFails<Edges>("{\"matrix\":[[1],[2,2]]}", "matrix[1]: duplicate items: element at index 1 equals index 0");
Check.WriteFails(new Edges { Matrix = new[] { new long[] { 3, 3 } } }, "matrix[0]: duplicate items");"#,
    );
}

#[test]
fn csharp_json_collection_and_element_nullability_stay_independent() {
    let case = JsonCase::new("collection-nullability");
    case.write(
        "event.yaml",
        r#"type: object
required: [timestamp, nullableCollection, nullableElement]
properties:
  timestamp: { type: string, format: date-time }
  optionalTimestamp: { type: string, format: date-time }
  optionalCollection: { type: array, items: { type: string, format: date-time } }
  nullableCollection:
    oneOf: [{ type: array, items: { type: string, format: date-time } }, { type: "null" }]
  nullableElement:
    type: array
    items: { oneOf: [{ type: string, format: date-time }, { type: "null" }] }
  bothCombined:
    oneOf:
      - { type: array, items: { oneOf: [{ type: string, format: date-time }, { type: "null" }] } }
      - { type: "null" }
"#,
    );
    let output = case.generate(&["event.yaml"]);
    assert_has(
        output.file("Models.cs"),
        &[
            "public required global::System.DateTimeOffset Timestamp { get; init; }",
            "public global::System.DateTimeOffset? OptionalTimestamp { get; init; }",
            "IReadOnlyList<global::System.DateTimeOffset>? OptionalCollection { get; init; }",
            "public required global::System.Collections.Generic.IReadOnlyList<global::System.DateTimeOffset>? NullableCollection { get; init; }",
            "public required global::System.Collections.Generic.IReadOnlyList<global::System.DateTimeOffset?> NullableElement { get; init; }",
            "IReadOnlyList<global::System.DateTimeOffset?>? BothCombined { get; init; }",
        ],
    );
    case.run(
        r#"const string stamp = "2024-01-01T00:00:00Z";
Check.RoundTrip<Event>("{\"timestamp\":\"" + stamp + "\",\"nullableCollection\":null,\"nullableElement\":[null,\"" + stamp + "\"]}");
Check.RoundTrip<Event>("{\"timestamp\":\"" + stamp + "\",\"nullableCollection\":[\"" + stamp + "\"],\"nullableElement\":[],\"bothCombined\":[null]}");
Check.ParseFails<Event>("{\"timestamp\":\"" + stamp + "\",\"nullableCollection\":[null],\"nullableElement\":null}",
    "nullableCollection[0]: explicit null not allowed",
    "nullableElement: explicit null not allowed");"#,
    );
}

#[test]
fn csharp_json_optional_materialized_consts_round_trip() {
    let case = JsonCase::new("optional-consts");
    case.write(
        "pin.yaml",
        r#"type: object
properties:
  plain: { type: string, const: "x" }
  num: { type: integer, const: 3 }
  tag: { type: string, contentEncoding: base64, const: "aGk=" }
  span: { type: string, format: duration, const: "PT1H30M" }
"#,
    );
    case.generate(&["pin.yaml"]);
    case.run(
        r#"Check.RoundTrip<Pin>("{}");
Check.RoundTrip<Pin>("{\"plain\":\"x\",\"num\":3,\"tag\":\"aGk=\",\"span\":\"PT1H30M\"}");
// The closed set compares the wire spelling, as in every target: `PT90M`
// denotes the same duration but is not the constant's text.
Check.ParseFails<Pin>("{\"span\":\"PT90M\"}", "span: must equal \"PT1H30M\"");
Check.ParseFails<Pin>("{\"plain\":\"y\",\"span\":\"PT1H\"}", "plain: must equal \"x\"", "span: must equal \"PT1H30M\"");
Check.WriteFails(new Pin { Span = System.TimeSpan.FromHours(1) }, "span: must equal \"PT1H30M\"");"#,
    );
}
