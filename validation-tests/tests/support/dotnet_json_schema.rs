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
use crate::support::csharp_json_case::{JsonCase, assert_has};

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
// Objects
// ---------------------------------------------------------------------------

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
