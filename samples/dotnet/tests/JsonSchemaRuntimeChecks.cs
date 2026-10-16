using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text.Json;
using System.Text.Json.Nodes;
using Temporalio.Api.Common.V1;
using Temporalio.Converters;
using Temporalio.Exceptions;
using Xunit;
using Chat = Nexgen.JsonSchema.Chat;
using KbBlock = Nexgen.JsonSchema.Kb.Content.Block;
using KbCategory = Nexgen.JsonSchema.Kb.Tree.Category;
using KbOperations = Nexgen.JsonSchema.Kb.Kb;
using KbPage = Nexgen.JsonSchema.Kb.Content.Page;
using Showcase = Nexgen.JsonSchema.Showcase;
using Temporal = Nexgen.JsonSchema.Temporal;

namespace Nexgen.DotNetExamples.Tests
{
    /// <summary>
    /// The round-trip sweep over every canonical wire fixture in
    /// <c>samples/wire/json_schema/</c>, the cross-language contract the Go,
    /// Python, TypeScript and Java suites read too. Each fixture is decoded
    /// through Temporal's stock data converter, re-encoded, and compared to the
    /// fixture as a JSON value.
    /// </summary>
    public class JsonSchemaRuntimeChecks
    {
        private static readonly IPayloadConverter Converter = DataConverter.Default.PayloadConverter;

        /// <summary>
        /// Every fixture and the model it round-trips through. Exhaustive:
        /// <see cref="EveryWireFixtureIsDeclared"/> fails when a file is added,
        /// removed or renamed without updating this table.
        /// </summary>
        private static readonly Dictionary<(string Suite, string Name), Type> WireFixtures = new()
        {
            [("chat", "labels.json")] = typeof(Chat.Labels),
            [("chat", "message-full.json")] = typeof(Chat.Message),
            [("chat", "message-minimal.json")] = typeof(Chat.Message),
            [("chat", "room-open.json")] = typeof(Chat.Room),
            [("chat", "send-message-input.json")] = typeof(Chat.SendMessageInput),
            [("chat", "send-message-output.json")] = typeof(Chat.SendMessageOutput),
            [("kb", "block.json")] = typeof(KbBlock.Block),
            [("kb", "category-tree.json")] = typeof(KbCategory.Category),
            [("kb", "get-category-tree-input.json")] = typeof(KbOperations.GetCategoryTreeInput),
            [("kb", "get-page-input.json")] = typeof(KbOperations.GetPageInput),
            [("kb", "page.json")] = typeof(KbPage.Page),
            [("kb", "put-block-output.json")] = typeof(KbOperations.PutBlockOutput),
            [("showcase", "address-open.json")] = typeof(Showcase.Address),
            [("showcase", "attributes.json")] = typeof(Showcase.Attributes),
            [("showcase", "contact.json")] = typeof(Showcase.ContactCs),
            [("showcase", "extras.json")] = typeof(Showcase.Extras),
            [("showcase", "labels.json")] = typeof(Showcase.Labels),
            [("showcase", "settings.json")] = typeof(Showcase.Settings),
            [("showcase", "showcase-bytes.json")] = typeof(Showcase.Showcase),
            [("showcase", "showcase-detail-object.json")] = typeof(Showcase.Showcase),
            [("showcase", "showcase-detail-string.json")] = typeof(Showcase.Showcase),
            [("showcase", "showcase-element-unions.json")] = typeof(Showcase.Showcase),
            [("showcase", "showcase-format.json")] = typeof(Showcase.Showcase),
            [("showcase", "showcase-freeform-string.json")] = typeof(Showcase.Showcase),
            [("showcase", "showcase-freeform.json")] = typeof(Showcase.Showcase),
            [("showcase", "showcase-full.json")] = typeof(Showcase.Showcase),
            [("showcase", "showcase-inline-shapes.json")] = typeof(Showcase.Showcase),
            [("showcase", "showcase-measurements-array.json")] = typeof(Showcase.Showcase),
            [("showcase", "showcase-measurements-string.json")] = typeof(Showcase.Showcase),
            [("showcase", "showcase-metrics.json")] = typeof(Showcase.Showcase),
            [("showcase", "showcase-minimal.json")] = typeof(Showcase.Showcase),
            [("showcase", "showcase-number-values.json")] = typeof(Showcase.Showcase),
            [("showcase", "showcase-note-link.json")] = typeof(Showcase.Showcase),
            [("showcase", "showcase-note-text.json")] = typeof(Showcase.Showcase),
            [("showcase", "showcase-nulls.json")] = typeof(Showcase.Showcase),
            [("showcase", "showcase-patterns.json")] = typeof(Showcase.Showcase),
            [("showcase", "showcase-recursive-collections.json")] = typeof(Showcase.Showcase),
            [("showcase", "showcase-shape-circle.json")] = typeof(Showcase.Showcase),
            [("showcase", "showcase-shape-or-name-square.json")] = typeof(Showcase.Showcase),
            [("showcase", "showcase-shape-or-name-string.json")] = typeof(Showcase.Showcase),
            [("showcase", "showcase-shape-square.json")] = typeof(Showcase.Showcase),
            [("showcase", "showcase-strings.json")] = typeof(Showcase.Showcase),
            [("showcase", "showcase-union-int.json")] = typeof(Showcase.Showcase),
            [("showcase", "showcase-union-string.json")] = typeof(Showcase.Showcase),
            [("showcase", "widget.json")] = typeof(Showcase.Widget),
            [("temporal", "temporal-canonicalize.json")] = typeof(Temporal.Temporal),
            [("temporal", "temporal-full.json")] = typeof(Temporal.Temporal),
            [("temporal", "temporal-minimal.json")] = typeof(Temporal.Temporal),
            [("temporal", "temporal-nulls.json")] = typeof(Temporal.Temporal),
        };

        /// <summary>
        /// Optional+nullable members whose explicit <c>null</c> collapses onto
        /// absence in memory (<c>T?</c> has one null state), so C# re-emits them
        /// omitted, as Go, Java and Python do.
        /// </summary>
        private static readonly Dictionary<(string Suite, string Name), string[]> CollapsedNullMembers = new()
        {
            [("chat", "message-full.json")] = new[] { "replyToId" },
            [("kb", "block.json")] = new[] { "page" },
            [("kb", "page.json")] = new[] { "blocks[].page" },
            [("showcase", "showcase-nulls.json")] = new[] { "middleName" },
            [("temporal", "temporal-nulls.json")] = new[] { "deletedAt", "archivedOn" },
        };

        /// <summary>
        /// Deliberately non-canonical input whose re-emitted spelling is
        /// asserted explicitly by <see cref="TemporalInputCanonicalizes"/>.
        /// </summary>
        private static readonly HashSet<(string Suite, string Name)> NonCanonicalFixtures = new()
        {
            ("temporal", "temporal-canonicalize.json"),
        };

        [Fact]
        public void EveryWireFixtureIsDeclared()
        {
            var onDisk = new[] { "chat", "kb", "showcase", "temporal" }
                .SelectMany(suite => Directory.GetFiles(FixtureDirectory(suite), "*.json")
                    .Select(path => (suite, Path.GetFileName(path))))
                .ToHashSet();
            Assert.Equal(onDisk.OrderBy(key => key), WireFixtures.Keys.OrderBy(key => key));
            Assert.Subset(onDisk, CollapsedNullMembers.Keys.ToHashSet());
            Assert.Subset(onDisk, NonCanonicalFixtures);
        }

        [Fact]
        public void EveryWireFixtureRoundTripsByJsonValue()
        {
            foreach (var ((suite, name), type) in WireFixtures.OrderBy(entry => entry.Key))
            {
                if (NonCanonicalFixtures.Contains((suite, name)))
                {
                    continue;
                }
                var expected = JsonNode.Parse(File.ReadAllText(Path.Combine(FixtureDirectory(suite), name)))!;
                foreach (var path in CollapsedNullMembers.GetValueOrDefault((suite, name)) ?? Array.Empty<string>())
                {
                    DropPath(expected, path);
                }
                var model = Decode(type, File.ReadAllBytes(Path.Combine(FixtureDirectory(suite), name)));
                var actual = JsonNode.Parse(Encode(model))!;
                Assert.True(
                    Canonical(expected) == Canonical(actual),
                    $"{suite}/{name} did not round-trip:\nexpected {expected.ToJsonString()}\nactual   {actual.ToJsonString()}");
            }
        }

        [Fact]
        public void ReEncodingIsIdempotentAndModelsCompareByValue()
        {
            foreach (var ((suite, name), type) in WireFixtures)
            {
                var first = Decode(type, File.ReadAllBytes(Path.Combine(FixtureDirectory(suite), name)));
                var encoded = Encode(first);
                var second = Decode(type, encoded);
                Assert.Equal(encoded, Encode(second));
                Assert.Equal(first, second);
                Assert.Equal(first.GetHashCode(), second.GetHashCode());
            }
        }

        [Fact]
        public void TemporalInputCanonicalizes()
        {
            var model = Decode<Temporal.Temporal>("temporal", "temporal-canonicalize.json");
            var actual = JsonNode.Parse(Encode(model))!;
            var expected = JsonNode.Parse(
                """{"createdAt":"2021-06-15T12:30:45Z","birthday":"2021-02-28","alarm":"12:30:45Z","timeout":"PT1H30M"}""")!;
            Assert.Equal(Canonical(expected), Canonical(actual));
            Assert.Equal(TimeSpan.FromMinutes(90), model.Timeout);
        }

        [Fact]
        public void ChatFixturesDecodeToTheirMeaning()
        {
            var message = Decode<Chat.Message>("chat", "message-minimal.json");
            Assert.Equal("hi", message.Body);
            Assert.Null(message.ReplyToId);
            var room = Decode<Chat.Room>("chat", "room-open.json");
            Assert.Equal("r1", room.RoomId);
            Assert.Contains("x-extra", room.AdditionalProperties.Keys);
            var request = Decode<Chat.SendMessageInput>("chat", "send-message-input.json");
            Assert.Equal("hi", request.Message.Body);
        }

        [Fact]
        public void InvalidPayloadsAggregateEveryViolation()
        {
            var failure = Assert.Throws<ApplicationFailureException>(
                () => Decode(typeof(Chat.SendMessageInput), """{"roomId": 7, "message": {"kind": "text"}, "extra": true}"""u8.ToArray()));
            Assert.Equal("PayloadValidationError", failure.ErrorType);
            Assert.True(failure.NonRetryable);
            var violations = failure.Details!.ElementAt<IReadOnlyList<Chat.Violation>>(0);
            Assert.Contains(violations, violation => violation.Path == "roomId");
            Assert.Contains(violations, violation => violation.Path == "message.body");
            Assert.Contains(violations, violation => violation.Path == "extra");
        }

        [Fact]
        public void SerializeValidatesBeforeWriting()
        {
            var valid = Decode<Chat.SendMessageInput>("chat", "send-message-input.json");
            var invalid = valid with { RoomId = null! };
            var failure = Assert.Throws<ApplicationFailureException>(() => Encode(invalid));
            Assert.Equal("PayloadValidationError", failure.ErrorType);
            var violations = failure.Details!.ElementAt<IReadOnlyList<Chat.Violation>>(0);
            Assert.Contains(violations, violation => violation.Path == "roomId");
        }

        internal static T Decode<T>(string suite, string name) =>
            (T)Decode(typeof(T), File.ReadAllBytes(Path.Combine(FixtureDirectory(suite), name)));

        private static object Decode(Type type, byte[] json) =>
            Converter.ToValue(
                new Payload
                {
                    Metadata = { ["encoding"] = Google.Protobuf.ByteString.CopyFromUtf8("json/plain") },
                    Data = Google.Protobuf.ByteString.CopyFrom(json),
                },
                type)!;

        private static byte[] Encode(object value) => Converter.ToPayload(value).Data.ToByteArray();

        /// <summary>
        /// A JSON value's canonical spelling: members sorted, numbers compared
        /// mathematically.
        /// </summary>
        private static string Canonical(JsonNode? node) => node switch
        {
            null => "null",
            JsonObject members => "{" + string.Join(",", members
                .OrderBy(member => member.Key, StringComparer.Ordinal)
                .Select(member => JsonSerializer.Serialize(member.Key) + ":" + Canonical(member.Value))) + "}",
            JsonArray items => "[" + string.Join(",", items.Select(Canonical)) + "]",
            JsonValue value when value.GetValueKind() == JsonValueKind.Number =>
                double.Parse(value.ToJsonString(), System.Globalization.CultureInfo.InvariantCulture).ToString("R", System.Globalization.CultureInfo.InvariantCulture),
            _ => node.ToJsonString(),
        };

        /// <summary>
        /// Removes a dotted member path; an <c>name[]</c> segment applies the
        /// rest of the path to every element.
        /// </summary>
        private static void DropPath(JsonNode node, string path)
        {
            var dot = path.IndexOf('.');
            var segment = dot < 0 ? path : path[..dot];
            var rest = dot < 0 ? "" : path[(dot + 1)..];
            if (segment.EndsWith("[]", StringComparison.Ordinal))
            {
                foreach (var element in node[segment[..^2]]!.AsArray())
                {
                    DropPath(element!, rest);
                }
                return;
            }
            if (rest.Length > 0)
            {
                DropPath(node[segment]!, rest);
                return;
            }
            Assert.True(node.AsObject().Remove(segment), $"collapsed member {path} is not in the fixture");
        }

        private static string FixtureDirectory(string suite)
        {
            var directory = new DirectoryInfo(AppContext.BaseDirectory);
            while (directory is not null)
            {
                var candidate = Path.Combine(directory.FullName, "samples", "wire", "json_schema", suite);
                if (Directory.Exists(candidate))
                {
                    return candidate;
                }
                directory = directory.Parent;
            }
            throw new DirectoryNotFoundException($"wire fixtures for {suite}");
        }
    }
}
