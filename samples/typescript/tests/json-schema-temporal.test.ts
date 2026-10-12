import { describe, expect, test } from "vitest";

import { temporalTransferTypeConverter as stringTemporalTransferTypeConverter } from "../temporal/models.ts";
import { temporalTransferTypeConverter as dateTemporalTransferTypeConverter } from "../temporal-date/models.ts";
import { temporalTransferTypeConverter as temporalTemporalTransferTypeConverter } from "../temporal-temporal/models.ts";
import {
  decodeFixture,
  encodeModel,
  exposeValidationDetails,
  fixtureBytes,
  loadFixture as loadFixtureFrom,
  roundTripFixture,
} from "./json-converter-helper.ts";

exposeValidationDetails(
  stringTemporalTransferTypeConverter,
  dateTemporalTransferTypeConverter,
  temporalTemporalTransferTypeConverter,
);

const wireFixtureDir = new URL("../../wire/json_schema/temporal/", import.meta.url);

function bytes(name: string): Uint8Array {
  return fixtureBytes(wireFixtureDir, name);
}
function load(name: string): unknown {
  return loadFixtureFrom(wireFixtureDir, name);
}

// --- --date-time-types=string (default): all four temporals are string. ---
describe("json-schema temporal (--date-time-types=string, default)", () => {
  test("materialized temporals round-trip losslessly as canonical strings", () => {
    const { value, serialized } = roundTripFixture(
      stringTemporalTransferTypeConverter,
      bytes("temporal-full.json"),
    );
    expect(serialized).toEqual(load("temporal-full.json"));
    expect(value.createdAt).toBe("2021-06-15T12:30:45.123456+02:00");
    expect(value.timeout).toBe("PT1H30M");

    const minimal = roundTripFixture(
      stringTemporalTransferTypeConverter,
      bytes("temporal-minimal.json"),
    );
    expect(minimal.serialized).toEqual(load("temporal-minimal.json"));
  });

  test("non-canonical input is canonicalized (uppercase T/Z, +00:00 -> Z, PT90M -> PT1H30M)", () => {
    const value = decodeFixture(
      stringTemporalTransferTypeConverter,
      bytes("temporal-canonicalize.json"),
    );
    expect(encodeModel(stringTemporalTransferTypeConverter, value)).toEqual({
      createdAt: "2021-06-15T12:30:45Z",
      birthday: "2021-02-28",
      alarm: "12:30:45Z",
      timeout: "PT1H30M",
    });
  });

  test("materialized narrowing rejects :60, calendar duration, bad date, missing offset", () => {
    const converter = stringTemporalTransferTypeConverter;
    for (const bad of [
      { createdAt: "2021-12-31T23:59:60Z", timeout: "PT0S" },
      { createdAt: "2021-06-15T12:30:45Z", timeout: "P1Y" },
      { createdAt: "2021-06-15T12:30:45Z", birthday: "2021-02-29", timeout: "PT0S" },
      { createdAt: "2021-06-15T12:30:45", timeout: "PT0S" },
      { createdAt: "0000-01-01T00:00:00Z", timeout: "PT0S" },
      { createdAt: "2021-06-15T12:30:45Z", birthday: "0000-01-01", timeout: "PT0S" },
      { createdAt: "2021-06-15T12:30:45+14:01", timeout: "PT0S" },
      { createdAt: "2021-06-15T12:30:45-18:00", timeout: "PT0S" },
      { createdAt: "2021-06-15T12:30:45-23:59", timeout: "PT0S" },
      { createdAt: "2021-06-15T12:30:45Z", alarm: "09:00:00+14:01", timeout: "PT0S" },
      { createdAt: "2021-06-15T12:30:45Z", alarm: "09:00:00+18:00", timeout: "PT0S" },
      { createdAt: "2021-06-15T12:30:45Z", alarm: "09:00:00-23:59", timeout: "PT0S" },
    ]) {
      const body = { birthday: "2000-01-01", alarm: "09:00:00", ...bad };
      expect(() => converter.fromTransferType(body)).toThrow();
    }

    expect(
      converter.fromTransferType({
        createdAt: "0001-01-01T00:00:00Z",
        birthday: "0001-01-01",
        alarm: "00:00:00",
        timeout: "PT0S",
      }),
    ).toMatchObject({
      createdAt: "0001-01-01T00:00:00Z",
      birthday: "0001-01-01",
    });
  });

  test("offset boundaries are enforced on parse and serialize", () => {
    for (const offset of ["+14:00", "-14:00"]) {
      const value = stringTemporalTransferTypeConverter.fromTransferType({
        createdAt: `2021-06-15T12:30:45${offset}`,
        birthday: "2000-01-01",
        alarm: `09:00:00${offset}`,
        timeout: "PT0S",
      });
      expect(encodeModel(stringTemporalTransferTypeConverter, value)).toMatchObject({
        createdAt: `2021-06-15T12:30:45${offset}`,
        alarm: `09:00:00${offset}`,
      });
    }

    const valid = stringTemporalTransferTypeConverter.fromTransferType({
      createdAt: "2021-06-15T12:30:45Z",
      birthday: "2000-01-01",
      alarm: "09:00:00",
      timeout: "PT0S",
    });
    for (const offset of ["+14:01", "-14:01", "+18:00", "-18:00", "+23:59", "-23:59"]) {
      expect(() =>
        encodeModel(stringTemporalTransferTypeConverter, {
          ...valid,
          createdAt: `2021-06-15T12:30:45${offset}`,
        }),
      ).toThrow(/createdAt.*valid date-time/);
      expect(() =>
        encodeModel(stringTemporalTransferTypeConverter, {
          ...valid,
          alarm: `09:00:00${offset}`,
        }),
      ).toThrow(/alarm.*valid time/);
    }
  });
});

// --- --date-time-types=date: date-time -> Date (UTC ms fold); others string. ---
describe("json-schema temporal (--date-time-types=date)", () => {
  test("date-time materializes to a Date and folds to a UTC instant on re-serialize", () => {
    const value = decodeFixture(
      dateTemporalTransferTypeConverter,
      bytes("temporal-full.json"),
    );
    expect(value.createdAt).toBeInstanceOf(Date);
    expect((value.createdAt as Date).toISOString()).toBe("2021-06-15T10:30:45.123Z");
    expect(value.birthday).toBe("2021-06-15"); // stays string
    const serialized = encodeModel(dateTemporalTransferTypeConverter, value) as Record<
      string,
      unknown
    >;
    expect(serialized.createdAt).toBe("2021-06-15T10:30:45.123Z");
    expect(serialized.timeout).toBe("PT1H30M");
  });

  test("invalid Dates fail through the aggregated path-aware validator", () => {
    const value = decodeFixture(
      dateTemporalTransferTypeConverter,
      bytes("temporal-full.json"),
    );
    value.createdAt = new Date(Number.NaN);
    expect(() => encodeModel(dateTemporalTransferTypeConverter, value)).toThrow(
      /createdAt.*valid date-time/,
    );
  });
});

// --- --date-time-types=temporal: Temporal.* types; time stays string. ---
describe("json-schema temporal (--date-time-types=temporal)", () => {
  test("temporals materialize to Temporal types and round-trip losslessly", () => {
    const { value, serialized } = roundTripFixture(
      temporalTemporalTransferTypeConverter,
      bytes("temporal-full.json"),
    );
    expect(serialized).toEqual(load("temporal-full.json"));
    // ZonedDateTime preserves the offset; PlainDate / Duration are exact; time stays string.
    expect(String(value.createdAt.offset)).toBe("+02:00");
    expect(value.birthday.toString()).toBe("2021-06-15");
    expect(typeof value.alarm).toBe("string");
    // The Temporal.Duration holds 90 minutes; the wire canonical form (asserted
    // via `serialized` above) is PT1H30M.
    expect(value.timeout.total({ unit: "seconds" })).toBe(5400);
  });

  test("non-canonical input canonicalizes through Temporal types", () => {
    const value = decodeFixture(
      temporalTemporalTransferTypeConverter,
      bytes("temporal-canonicalize.json"),
    );
    expect(encodeModel(temporalTemporalTransferTypeConverter, value)).toEqual({
      createdAt: "2021-06-15T12:30:45Z",
      birthday: "2021-02-28",
      alarm: "12:30:45Z",
      timeout: "PT1H30M",
    });
  });
});
