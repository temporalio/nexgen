// Generic cross-language conformance runner for generated C# models.
//
// Driven by tests/json_schema_conformance_manifest.rs through a plan file
// (protocol in tests/toolchain/mod.rs). Models are located by name and driven
// through Temporal's default payload converter, which is what binds the
// generator's type-level [JsonConverter] — so a new conformance case needs no
// runner change.
//
// Usage: Runner <plan.json> <result.json>

using System;
using System.Collections;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Reflection;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Text.RegularExpressions;
using Temporalio.Api.Common.V1;
using Temporalio.Converters;
using Temporalio.Exceptions;

public static class Runner
{
    private static readonly IPayloadConverter Converter = DataConverter.Default.PayloadConverter;
    private static readonly Regex Segment = new(@"^([A-Za-z0-9]+)((?:\[\d+\])*)$");
    private static readonly Regex Index = new(@"\[(\d+)\]");

    public static int Main(string[] args)
    {
        var plan = JsonNode.Parse(File.ReadAllText(args[0]))!;
        var results = new JsonObject();
        var assembly = typeof(Runner).Assembly;
        foreach (var testCase in plan["cases"]!.AsArray())
        {
            var probes = new JsonObject();
            results[testCase!["id"]!.GetValue<string>()] = probes;
            var typeName = testCase["csharp_model"]!.GetValue<string>();
            var model = assembly.GetType(typeName);
            foreach (var probe in testCase["probes"]!.AsArray())
            {
                var id = probe!["id"]!.GetValue<string>();
                probes[id] = model is null
                    ? Error($"type {typeName} not found")
                    : RunProbe(model, probe);
            }
        }
        File.WriteAllText(args[1], results.ToJsonString(new JsonSerializerOptions { WriteIndented = true }));
        return 0;
    }

    private static JsonObject RunProbe(Type model, JsonNode probe)
    {
        object? value;
        try
        {
            value = Converter.ToValue(JsonPayload(probe["wire"]!.GetValue<string>()), model);
        }
        catch (Exception error)
        {
            var violations = ViolationsOf(error);
            return violations is null ? Error(Describe(error)) : Verdict("parse_rejected", violations);
        }
        if (probe["kind"]!.GetValue<string>() == "parse")
        {
            return Verdict("accepted", null);
        }
        if (probe["mutations"] is JsonArray mutations)
        {
            foreach (var mutation in mutations)
            {
                try
                {
                    ApplyMutation(value!, mutation!);
                }
                catch (Exception error)
                {
                    return Error("mutation failed: " + Describe(error));
                }
            }
        }
        string serialized;
        try
        {
            serialized = Converter.ToPayload(value).Data.ToStringUtf8();
        }
        catch (Exception error)
        {
            var violations = ViolationsOf(error);
            return violations is null ? Error(Describe(error)) : Verdict("serialize_rejected", violations);
        }
        var accepted = Verdict("accepted", null);
        try
        {
            accepted["wire"] = JsonNode.Parse(serialized)!.ToJsonString();
        }
        catch (Exception error)
        {
            accepted["wire"] = null;
            accepted["note"] = "output is not JSON: " + Describe(error);
        }
        return accepted;
    }

    private static Payload JsonPayload(string json) => new()
    {
        Metadata = { ["encoding"] = Google.Protobuf.ByteString.CopyFromUtf8("json/plain") },
        Data = Google.Protobuf.ByteString.CopyFromUtf8(json),
    };

    /// <summary>One mutation-path step: a named member, or an array index.</summary>
    private sealed record Step(string? Name, int Index);

    /// <summary><c>a.b[0][1]</c> → member a, member b, index 0, index 1.</summary>
    private static List<Step> StepsOf(string path)
    {
        var steps = new List<Step>();
        foreach (var segment in path.Split('.'))
        {
            var match = Segment.Match(segment);
            if (!match.Success)
            {
                throw new ArgumentException("unparsable path segment " + segment);
            }
            steps.Add(new Step(match.Groups[1].Value, -1));
            foreach (Match index in Index.Matches(match.Groups[2].Value))
            {
                steps.Add(new Step(null, int.Parse(index.Groups[1].Value)));
            }
        }
        return steps;
    }

    /// <summary>
    /// The property a JSON member maps to. Conformance schemas use lowerCamel
    /// ASCII names, for which the generated C# member is the PascalCase name.
    /// </summary>
    private static PropertyInfo PropertyOf(Type type, string name)
    {
        var member = char.ToUpperInvariant(name[0]) + name[1..];
        return type.GetProperty(member, BindingFlags.Public | BindingFlags.Instance)
            ?? throw new MissingMemberException(type.Name, member);
    }

    private static object? Read(object owner, Step step) =>
        step.Name is null ? ((IList)owner)[step.Index] : PropertyOf(owner.GetType(), step.Name).GetValue(owner);

    private static void Write(object owner, Step step, object? value)
    {
        if (step.Name is null)
        {
            ((IList)owner)[step.Index] = value;
            return;
        }
        PropertyOf(owner.GetType(), step.Name).SetValue(owner, value);
    }

    private static Type SlotType(object owner, Step step)
    {
        if (step.Name is null)
        {
            var listType = owner.GetType();
            return listType.IsGenericType ? listType.GetGenericArguments()[0] : typeof(object);
        }
        return PropertyOf(owner.GetType(), step.Name).PropertyType;
    }

    private static void ApplyMutation(object model, JsonNode mutation)
    {
        var steps = StepsOf(mutation["path"]!.GetValue<string>());
        var owner = model;
        for (var index = 0; index < steps.Count - 1; index++)
        {
            owner = Read(owner, steps[index])!;
        }
        var last = steps[^1];
        if (mutation["duplicate_element"] is JsonNode duplicate)
        {
            var sequence = (IList)Read(owner, last)!;
            sequence.Add(sequence[duplicate.GetValue<int>()]);
            return;
        }
        if (mutation["remove_array_element"] is JsonNode remove)
        {
            ((IList)Read(owner, last)!).RemoveAt(remove.GetValue<int>());
            return;
        }
        if (mutation["put_map_entry"] is JsonNode entry)
        {
            var map = TypedMap(Read(owner, last)!);
            var valueType = map.GetType().GetGenericArguments()[1];
            map[entry["key"]!.GetValue<string>()] = entry["value"] is null
                ? null
                : JsonSerializer.Deserialize(entry["value"]!.ToJsonString(), valueType);
            return;
        }
        if (mutation["remove_map_entry"] is JsonNode removeKey)
        {
            TypedMap(Read(owner, last)!).Remove(removeKey.GetValue<string>());
            return;
        }
        object? replacement;
        var slot = Nullable.GetUnderlyingType(SlotType(owner, last)) ?? SlotType(owner, last);
        if (mutation["set_integer"] is JsonNode integer)
        {
            var text = integer.ToJsonString().Trim('"');
            replacement = slot == typeof(double) ? (object)double.Parse(text) : long.Parse(text);
        }
        else if (mutation["set_number"] is JsonNode number)
        {
            replacement = number.ToJsonString().Trim('"') switch
            {
                "nan" => double.NaN,
                "inf" => double.PositiveInfinity,
                "-inf" => double.NegativeInfinity,
                var text => double.Parse(text, System.Globalization.CultureInfo.InvariantCulture),
            };
        }
        else if (mutation["set_string"] is JsonNode text)
        {
            replacement = text.GetValue<string>();
        }
        else if (mutation["set_null"] is not null || mutation["set_absent"] is not null)
        {
            replacement = null;
        }
        else if (mutation["set_bytes"] is JsonArray bytes)
        {
            replacement = bytes.Select(item => (byte)item!.GetValue<int>()).ToArray();
        }
        else if (mutation["set_duration"] is JsonNode duration)
        {
            replacement = TimeSpan.FromSeconds(duration["seconds"]!.GetValue<long>())
                + TimeSpan.FromTicks(duration["nanoseconds"]!.GetValue<long>() / 100);
        }
        else
        {
            throw new ArgumentException("unknown mutation " + mutation.ToJsonString());
        }
        Write(owner, last, replacement);
    }

    private static IDictionary TypedMap(object value) =>
        value as IDictionary
            ?? (IDictionary)PropertyOf(value.GetType(), "additionalProperties").GetValue(value)!;

    private static JsonArray? ViolationsOf(Exception error)
    {
        for (var current = error; current is not null; current = current.InnerException)
        {
            if (current is not ApplicationFailureException failure
                || failure.ErrorType != "PayloadValidationError"
                || failure.Details is null
                || failure.Details.Count == 0)
            {
                continue;
            }
            // A locally-created failure retains the list as its first detail.
            var violations = failure.Details.ElementAt<IEnumerable>(0);
            var found = new JsonArray();
            foreach (var violation in violations)
            {
                var type = violation.GetType();
                found.Add(new JsonObject
                {
                    ["path"] = (string)type.GetProperty("Path")!.GetValue(violation)!,
                    ["reason"] = (string)type.GetProperty("Reason")!.GetValue(violation)!,
                });
            }
            return found;
        }
        return null;
    }

    private static JsonObject Verdict(string outcome, JsonArray? violations)
    {
        var verdict = new JsonObject { ["outcome"] = outcome };
        if (violations is not null)
        {
            verdict["violations"] = violations;
        }
        return verdict;
    }

    private static JsonObject Error(string message) => new()
    {
        ["outcome"] = "error",
        ["message"] = message,
    };

    private static string Describe(Exception error) => error.GetType().FullName + ": " + error.Message;
}
