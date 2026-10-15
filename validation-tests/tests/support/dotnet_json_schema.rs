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
use crate::support::csharp_json_case::{JsonCase, assert_has, assert_lacks};

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

    fn write_project(&self, executable: bool) {
        fs::write(
            self.root.join("Probe.csproj"),
            format!(
                r#"<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <OutputType>{}</OutputType>
    <TargetFramework>net10.0</TargetFramework>
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
                dotnet_namespace: None,
            })
            .unwrap();
            case.build();
        }
    }
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
            "[global::System.Obsolete]\n        public string? Salutation { get; init; }",
            "public string SalutationOrDefault => Salutation ?? \"hello\";",
            "public long RetriesOrDefault => Retries ?? 3L;",
        ],
    );
    case.run(
        r#"var minimal = Check.RoundTrip<Model>("{\"requiredPlain\":\"a\",\"requiredNullable\":null}");
Check.Equal<long?>(null, minimal.RequiredNullable, "required nullable");
// Defaults apply on read, never on the wire.
Check.Equal<long?>(null, minimal.Retries, "unset default");
Check.Equal(3L, minimal.RetriesOrDefault, "default");
#pragma warning disable CS0612
Check.Equal("hello", minimal.SalutationOrDefault, "renamed default");
Check.RoundTrip<Model>("{\"requiredPlain\":\"a\",\"requiredNullable\":1,\"greeting\":\"hi\",\"retries\":3}");
#pragma warning restore CS0612
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
        Metric = new Finite.MetricUnion.MetricUnionNumber(double.NegativeInfinity),
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

// ---------------------------------------------------------------------------
// .NET string and equality semantics
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Names
// ---------------------------------------------------------------------------

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
