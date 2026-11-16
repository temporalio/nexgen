using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using Temporalio.Api.Common.V1;
using Temporalio.Converters;
using Temporalio.Exceptions;
using Xunit;
using Showcase = Nexgen.JsonSchema.Showcase;
using Temporal = Nexgen.JsonSchema.Temporal;

namespace Nexgen.DotNetExamples.Tests
{
    /// <summary>
    /// The C# consumers the conformance manifest
    /// (<c>samples/conformance/json-schema.json</c>) names for its cases: each
    /// test asserts what a shared fixture <i>means</i> in C#.
    /// </summary>
    public class JsonSchemaConsumerChecks
    {
        private static readonly IPayloadConverter Converter = DataConverter.Default.PayloadConverter;

        [Fact]
        public void RecursiveCollectionsRoundTripAndRejectNonFiniteValues()
        {
            var value = Decode<Showcase.Showcase>(Fixture("showcase", "showcase-recursive-collections.json"));
            Encode(value);
            Assert.Equal(new[] { 1.0, 2.5 }, value.NumberGrid![0]);
            Assert.Equal("1 Main St", value.Addresses![0].Street);
            Assert.Equal("2 Side St", value.AddressBook!.AdditionalProperties["home"].Street);
            Assert.Equal(1, value.Dates![0].Year);
            Assert.Equal(1, value.DateIndex!.AdditionalProperties["first"].Year);
            Assert.Equal("hi"u8.ToArray(), value.Blobs![0]);
            Assert.Equal("hi"u8.ToArray(), value.BlobIndex!.AdditionalProperties["hi"]);

            var minimal = Decode<Showcase.Showcase>(Fixture("showcase", "showcase-minimal.json"));
            var cases = new (string Path, Showcase.Showcase Value)[]
            {
                ("score", minimal with { Score = double.NaN }),
                ("measurements[0]", minimal with
                {
                    Measurements = new Showcase.Showcase.MeasurementsUnionArray(new List<double> { double.NegativeInfinity }),
                }),
                ("numberGrid[0][1]", minimal with
                {
                    NumberGrid = new List<IReadOnlyList<double>> { new List<double> { 1.0, double.NaN } },
                }),
                ("metrics.cpu", minimal with
                {
                    Metrics = new Showcase.Metrics
                    {
                        AdditionalProperties = new Dictionary<string, double> { ["cpu"] = double.PositiveInfinity },
                    },
                }),
                ("metricOrLabel", minimal with
                {
                    MetricOrLabel = new Showcase.Showcase.MetricOrLabelUnionNumber(double.PositiveInfinity),
                }),
            };
            foreach (var (path, invalid) in cases)
            {
                var violations = Violations(Assert.Throws<ApplicationFailureException>(() => Encode(invalid)));
                Assert.Contains(violations, violation => violation.Path == path && violation.Reason.Contains("finite number"));
            }
        }

        [Fact]
        public void NumberSpellingsRoundTripByMathematicalValue()
        {
            var value = Decode<Showcase.Showcase>(Fixture("showcase", "showcase-number-values.json"));
            Encode(value);
            var numbers = value.NumberGrid![0];
            Assert.Equal(0.0, numbers[0]);
            Assert.Equal(5.0, numbers[1]);
            Assert.Equal(1000.0, numbers[2]);
            Assert.Equal(double.MaxValue, numbers[3]);
            Assert.Equal(double.Epsilon, numbers[4]);
        }

        [Fact]
        public void MaterializedNarrowingRejects()
        {
            // Leap second, calendar duration, invalid calendar date, missing
            // offset, and year zero in either temporal member.
            foreach (var invalid in new[]
            {
                """{"createdAt":"2021-12-31T23:59:60Z","birthday":"2000-01-01","alarm":"09:00:00","timeout":"PT0S"}""",
                """{"createdAt":"2021-06-15T12:30:45Z","birthday":"2000-01-01","alarm":"09:00:00","timeout":"P1Y"}""",
                """{"createdAt":"2021-06-15T12:30:45Z","birthday":"2021-02-29","alarm":"09:00:00","timeout":"PT0S"}""",
                """{"createdAt":"2021-06-15T12:30:45","birthday":"2000-01-01","alarm":"09:00:00","timeout":"PT0S"}""",
                """{"createdAt":"0000-01-01T00:00:00Z","birthday":"2000-01-01","alarm":"09:00:00","timeout":"PT0S"}""",
                """{"createdAt":"2021-06-15T12:30:45Z","birthday":"0000-01-01","alarm":"09:00:00","timeout":"PT0S"}""",
            })
            {
                var failure = Assert.Throws<ApplicationFailureException>(
                    () => Decode<Temporal.Temporal>(System.Text.Encoding.UTF8.GetBytes(invalid)));
                Assert.Equal("PayloadValidationError", failure.ErrorType);
            }

            var firstYear = Decode<Temporal.Temporal>(
                """{"createdAt":"0001-01-01T00:00:00Z","birthday":"0001-01-01","alarm":"00:00:00","timeout":"PT0S"}"""u8.ToArray());
            Assert.Equal(1, firstYear.CreatedAt.Year);
            Assert.Equal(1, firstYear.Birthday.Year);
        }

        private static IReadOnlyList<Showcase.Violation> Violations(ApplicationFailureException failure)
        {
            Assert.Equal("PayloadValidationError", failure.ErrorType);
            return failure.Details!.ElementAt<IReadOnlyList<Showcase.Violation>>(0);
        }

        private static T Decode<T>(byte[] json) =>
            (T)Converter.ToValue(
                new Payload
                {
                    Metadata = { ["encoding"] = Google.Protobuf.ByteString.CopyFromUtf8("json/plain") },
                    Data = Google.Protobuf.ByteString.CopyFrom(json),
                },
                typeof(T))!;

        private static byte[] Encode(object value) => Converter.ToPayload(value).Data.ToByteArray();

        private static byte[] Fixture(string suite, string name)
        {
            var directory = new DirectoryInfo(AppContext.BaseDirectory);
            while (directory is not null)
            {
                var candidate = Path.Combine(directory.FullName, "samples", "wire", "json_schema", suite, name);
                if (File.Exists(candidate))
                {
                    return File.ReadAllBytes(candidate);
                }
                directory = directory.Parent;
            }
            throw new FileNotFoundException(name);
        }
    }
}
