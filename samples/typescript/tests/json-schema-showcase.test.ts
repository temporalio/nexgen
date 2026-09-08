import { describe, expect, test } from "vitest";
import { ApplicationFailure } from "@temporalio/common";
import type { TransferTypeConverter } from "nexus-rpc";

import {
  addressTransferTypeConverter,
  attributesTransferTypeConverter,
  contactTsTransferTypeConverter,
  DEFAULT_DEBUG,
  DEFAULT_GREETING,
  DEFAULT_RETRIES,
  extrasTransferTypeConverter,
  labelsTransferTypeConverter,
  itemRulesTransferTypeConverter,
  nicknamesTransferTypeConverter,
  quotasTransferTypeConverter,
  settingsTransferTypeConverter,
  showcaseTransferTypeConverter,
  tokensTransferTypeConverter,
  widgetTransferTypeConverter,
  type LinkNote,
  type Showcase,
  type ShowcaseDetailObject,
  type TextNote,
  type Widget,
} from "../showcase/index.ts";
import {
  exposeValidationDetails,
  fixtureBytes,
  loadFixture as loadFixtureFrom,
  roundTripFixture,
} from "./json-converter-helper.ts";

exposeValidationDetails(
  addressTransferTypeConverter,
  attributesTransferTypeConverter,
  contactTsTransferTypeConverter,
  extrasTransferTypeConverter,
  labelsTransferTypeConverter,
  itemRulesTransferTypeConverter,
  nicknamesTransferTypeConverter,
  quotasTransferTypeConverter,
  settingsTransferTypeConverter,
  showcaseTransferTypeConverter,
  tokensTransferTypeConverter,
  widgetTransferTypeConverter,
);

const wireFixtureDir = new URL("../../wire/json_schema/showcase/", import.meta.url);

function loadFixture(name: string): unknown {
  return loadFixtureFrom(wireFixtureDir, name);
}

// TS converters preserve explicit nulls, so all showcase fixtures round-trip with
// exact JSON-equality (no optional+nullable collapse — unlike Go/Java).
function expectRoundTrip<T>(name: string, converter: TransferTypeConverter<T>): T {
  const { value, serialized } = roundTripFixture(
    converter,
    fixtureBytes(wireFixtureDir, name),
  );
  expect(serialized).toEqual(loadFixture(name));
  return value;
}

// The structured violations a rejected payload produces, in order — for the
// assertions that pin an exact `{ path, reason }` set rather than one message.
function parseViolations(raw: unknown): { path: string; reason: string }[] {
  try {
    showcaseTransferTypeConverter.fromTransferType(raw);
  } catch (error) {
    if (
      error instanceof ApplicationFailure &&
      error.type === "PayloadValidationError" &&
      Array.isArray(error.details?.[0])
    ) {
      // The generated helper retained this array as detail 0; the cast performs
      // no serialization.
      return (error.details[0] as { path: string; reason: string }[]).map(
        ({ path, reason }) => ({ path, reason }),
      );
    }
    throw error;
  }
  throw new Error("expected the payload to be rejected");
}

function itemRuleViolations(run: () => unknown): { path: string; reason: string }[] {
  try {
    run();
  } catch (error) {
    if (
      error instanceof ApplicationFailure &&
      error.type === "PayloadValidationError" &&
      Array.isArray(error.details?.[0])
    ) {
      return error.details[0] as { path: string; reason: string }[];
    }
    throw error;
  }
  throw new Error("expected ItemRules to be rejected");
}

describe("json-schema showcase generated definitions", () => {
  test("closed items precede array-level violations", () => {
    const expected = [
      { path: "values[0]", reason: 'must equal "fixed"' },
      { path: "values[1]", reason: 'must equal "fixed"' },
      { path: "values", reason: "must have at least 3 items, got 2" },
      {
        path: "values",
        reason: "duplicate items: element at index 1 equals index 0",
      },
    ];
    expect(
      itemRuleViolations(() =>
        itemRulesTransferTypeConverter.fromTransferType({ values: ["bad", "bad"] }),
      ),
    ).toEqual(expected);
    expect(
      itemRuleViolations(() =>
        itemRulesTransferTypeConverter.toTransferType({
          values: ["bad" as "fixed", "bad" as "fixed"],
        }),
      ),
    ).toEqual(expected);
  });

  test("roundtrips canonical wire fixtures through the Temporal converter", () => {
    const minimal = expectRoundTrip(
      "showcase-minimal.json",
      showcaseTransferTypeConverter,
    );
    expect(minimal).toMatchObject<Partial<Showcase>>({
      kind: "showcase",
      revision: 1,
      enabled: true,
      status: "active",
      tier: 1,
      scale: 1.5,
      name: "Widget",
      count: 3,
      active: true,
      category: "tools",
    });
    expect(minimal.retries ?? DEFAULT_RETRIES).toBe(3);
    // Scalar defaults of each kind: unset on the wire (undefined), applied by the
    // consumer via the emitted DEFAULT_<FIELD> constant + `??` idiom.
    expect(minimal.greeting).toBeUndefined();
    expect(minimal.greeting ?? DEFAULT_GREETING).toBe("hello");
    expect(minimal.debug).toBeUndefined();
    expect(minimal.debug ?? DEFAULT_DEBUG).toBe(false);
    // Serialize side: an unset default-bearing field is OMITTED on the wire.
    const minimalWire = loadFixture("showcase-minimal.json") as Record<string, unknown>;
    expect(minimalWire).not.toHaveProperty("greeting");
    expect(minimalWire).not.toHaveProperty("debug");
    expect(minimalWire).not.toHaveProperty("retries");

    const full = expectRoundTrip("showcase-full.json", showcaseTransferTypeConverter);
    expect(full.retries).toBe(5);
    expect(full.middleName).toBe("Q");
    expect(full.tags).toEqual(["a", "b"]);
    expect(full.aliases).toEqual(["alpha", "beta"]);
    expect(full.roles).toEqual(["admin", "user"]);
    expect(full.address?.street).toBe("1 Main St");
    expect(full.address?.additionalProperties).toEqual({ region: "west" });
    expect(full.labels?.additionalProperties).toEqual({
      env: "prod",
      team: "core",
    });
    expect(full.settings?.fontSize).toBe(14);

    const nulls = expectRoundTrip("showcase-nulls.json", showcaseTransferTypeConverter);
    expect(nulls.middleName).toBeNull();
    expect(nulls.category).toBeNull();
    expect(nulls.active).toBe(false);

    const address = expectRoundTrip("address-open.json", addressTransferTypeConverter);
    expect(address.street).toBe("1 Main St");
    expect(address.additionalProperties).toEqual({ "x-extra": 7 });

    const labels = expectRoundTrip("labels.json", labelsTransferTypeConverter);
    expect(labels.additionalProperties).toEqual({ env: "prod", team: "core" });

    const settings = expectRoundTrip("settings.json", settingsTransferTypeConverter);
    expect(settings.theme).toBe("dark");
    expect(settings.fontSize).toBe(14);

    const metrics = expectRoundTrip(
      "showcase-metrics.json",
      showcaseTransferTypeConverter,
    );
    expect(metrics.priority).toBe(5);
    expect(metrics.level).toBe(2);
    expect(metrics.ratio).toBe(15);
    expect(metrics.step).toBe(9);

    // The astral crux: "a😀b" is 3 code points but 6 UTF-8 bytes / 4 UTF-16
    // units; it must round-trip through code (maxLength:5) unchanged.
    const strings = expectRoundTrip(
      "showcase-strings.json",
      showcaseTransferTypeConverter,
    );
    expect(strings.code).toBe("a😀b");
    expect(strings.nickname).toBe("buddy");
  });

  test("roundtrips the allOf-merged Widget type and enforces its merged bounds", () => {
    // Widget is an allOf base-type extension (WidgetBase folded in + an extension
    // branch): a flat standalone object with the union of properties and required.
    const widget = expectRoundTrip("widget.json", widgetTransferTypeConverter);
    expect(widget).toMatchObject<Partial<Widget>>({
      id: "w-1",
      kind: "gadget",
      name: "Widget One",
      size: 15,
    });

    // `size` carries a bound tightened from two allOf branches to [10, 20].
    expect(() =>
      widgetTransferTypeConverter.fromTransferType({
        id: "w-1",
        name: "Widget One",
        size: 5,
      }),
    ).toThrow(/must be >= 10, got 5/);
    expect(() =>
      widgetTransferTypeConverter.fromTransferType({
        id: "w-1",
        name: "Widget One",
        size: 25,
      }),
    ).toThrow(/must be <= 20, got 25/);

    // A missing required member contributed by the extension branch is rejected.
    expect(() => widgetTransferTypeConverter.fromTransferType({ id: "w-1" })).toThrow(
      ApplicationFailure,
    );
  });

  test("reports JSON schema validation errors", () => {
    // Wrong const value.
    expect(() =>
      showcaseTransferTypeConverter.fromTransferType({
        kind: "nope",
        name: "w",
        count: 1,
        active: true,
        category: null,
      }),
    ).toThrow(ApplicationFailure);

    // Missing required (required+nullable) field.
    expect(() =>
      showcaseTransferTypeConverter.fromTransferType({
        kind: "showcase",
        name: "w",
        count: 1,
        active: true,
      }),
    ).toThrow(ApplicationFailure);

    // Unknown key on a closed object.
    expect(() =>
      settingsTransferTypeConverter.fromTransferType({ theme: "dark", nope: 1 }),
    ).toThrow(ApplicationFailure);

    // Wrong integer const value.
    expect(() =>
      showcaseTransferTypeConverter.fromTransferType({
        kind: "showcase",
        revision: 2,
        enabled: true,
        status: "active",
        tier: 1,
        scale: 1.5,
        name: "w",
        count: 1,
        active: true,
        category: "tools",
      }),
    ).toThrow(/must equal 1/);

    // Numeric bounds fire at runtime with informative reasons.
    const base = {
      kind: "showcase",
      revision: 1,
      enabled: true,
      status: "active",
      tier: 1,
      scale: 1.5,
      name: "w",
      count: 1,
      active: true,
      category: "tools",
    };

    // Closed value-set (enum/const) rejections with informative reasons.
    expect(() =>
      showcaseTransferTypeConverter.fromTransferType({ ...base, status: "archived" }),
    ).toThrow(/must be one of \["active", "inactive", "pending"\], got "archived"/);
    expect(() =>
      showcaseTransferTypeConverter.fromTransferType({ ...base, tier: 9 }),
    ).toThrow(/must be one of \[1, 2, 3\], got 9/);
    expect(() =>
      showcaseTransferTypeConverter.fromTransferType({ ...base, scale: 3.5 }),
    ).toThrow(/must be one of \[1.5, 2.5\], got 3.5/);
    expect(() =>
      showcaseTransferTypeConverter.fromTransferType({ ...base, enabled: false }),
    ).toThrow(/must equal true/);
    // Valid enum/const values are accepted.
    expect(
      showcaseTransferTypeConverter.fromTransferType({
        ...base,
        status: "pending",
        tier: 3,
        scale: 2.5,
      }).status,
    ).toBe("pending");
    expect(() =>
      showcaseTransferTypeConverter.fromTransferType({ ...base, priority: 99 }),
    ).toThrow(/must be <= 10, got 99/);
    expect(() =>
      showcaseTransferTypeConverter.fromTransferType({ ...base, level: 0 }),
    ).toThrow(/must be > 0, got 0/);
    expect(() =>
      showcaseTransferTypeConverter.fromTransferType({ ...base, step: 7 }),
    ).toThrow(/must be a multiple of 3, got 7/);
    expect(() =>
      showcaseTransferTypeConverter.fromTransferType({ ...base, ratio: 7 }),
    ).toThrow(/must be a multiple of 5, got 7/);

    // String-length bounds fire at runtime, counted in code points.
    expect(() =>
      showcaseTransferTypeConverter.fromTransferType({ ...base, code: "a" }),
    ).toThrow(/must have length >= 2, got 1/);
    expect(() =>
      showcaseTransferTypeConverter.fromTransferType({ ...base, code: "abcdef" }),
    ).toThrow(/must have length <= 5, got 6/);
    // Astral: 6 emoji = 6 code points (24 bytes); rejected by code-point count.
    expect(() =>
      showcaseTransferTypeConverter.fromTransferType({ ...base, code: "😀😀😀😀😀😀" }),
    ).toThrow(/must have length <= 5, got 6/);
    // A multi-byte value within the code-point bound is accepted (byte count 6
    // would exceed maxLength:5 — proving code points, not bytes).
    expect(
      showcaseTransferTypeConverter.fromTransferType({ ...base, code: "a😀b" }).code,
    ).toBe("a😀b");

    // Array constraints fire at runtime with informative reasons.
    // Too few / too many items (minItems:1 / maxItems:5).
    expect(() =>
      showcaseTransferTypeConverter.fromTransferType({ ...base, tags: [] }),
    ).toThrow(/must have at least 1 items, got 0/);
    expect(() =>
      showcaseTransferTypeConverter.fromTransferType({
        ...base,
        tags: ["a", "b", "c", "d", "e", "f"],
      }),
    ).toThrow(/must have at most 5 items, got 6/);
    // Duplicate element (uniqueItems).
    expect(() =>
      showcaseTransferTypeConverter.fromTransferType({ ...base, aliases: ["x", "x"] }),
    ).toThrow(/duplicate items: element at index 1 equals index 0/);
    // Missing required contains match (no "admin").
    expect(() =>
      showcaseTransferTypeConverter.fromTransferType({ ...base, roles: ["user"] }),
    ).toThrow(/too few matching items: at least 1, got 0/);
    // Too many contains matches (maxContains:2).
    expect(() =>
      showcaseTransferTypeConverter.fromTransferType({
        ...base,
        roles: ["admin", "admin", "admin"],
      }),
    ).toThrow(/too many matching items: at most 2, got 3/);
    try {
      showcaseTransferTypeConverter.fromTransferType({ ...base, roles: [1, "admin"] });
      throw new Error("expected validation failure");
    } catch (error) {
      expect(error).toBeInstanceOf(ApplicationFailure);
      expect((error as ApplicationFailure).details?.[0]).toEqual([
        { path: "roles[0]", reason: "expected string" },
      ]);
    }
    // Valid arrays are accepted.
    const ok = showcaseTransferTypeConverter.fromTransferType({
      ...base,
      tags: ["a"],
      aliases: ["x", "y"],
      roles: ["admin"],
    });
    expect(ok.roles).toEqual(["admin"]);
  });

  test("a mistyped array element names the type it failed to be", () => {
    // Every element kind takes the same parse the value in that position would
    // take anywhere else, so a `string` element reads `expected string` — the
    // same reason a `string` member reports, and the one Python's element loop
    // and Java's report. The bracketed index in the path identifies the element;
    // the reason names the type (specs/json-schema/features/items.md). Because
    // the element takes that ordinary parse, a *constrained* element's own
    // `minLength`/`maxLength`/`pattern`/`format` are enforced there too.
    const base = {
      kind: "showcase",
      revision: 1,
      enabled: true,
      status: "active",
      tier: 1,
      scale: 1.5,
      name: "w",
      count: 1,
      active: true,
      category: "tools",
    } as const;

    expect(parseViolations({ ...base, tags: [1] })).toEqual([
      { path: "tags[0]", reason: "expected string" },
    ]);
    expect(parseViolations({ ...base, aliases: [1, 2] })).toEqual([
      { path: "aliases[0]", reason: "expected string" },
      { path: "aliases[1]", reason: "expected string" },
    ]);
    expect(parseViolations({ ...base, aliases: [1, 1] })).toEqual([
      { path: "aliases[0]", reason: "expected string" },
      { path: "aliases[1]", reason: "expected string" },
      {
        path: "aliases",
        reason: "duplicate items: element at index 1 equals index 0",
      },
    ]);
    expect(parseViolations({ ...base, tags: ["a", null, {}] })).toEqual([
      { path: "tags[1]", reason: "expected string" },
      { path: "tags[2]", reason: "expected string" },
    ]);
  });

  test("round-trips JSON numbers by mathematical value", () => {
    const { value, serialized } = roundTripFixture(
      showcaseTransferTypeConverter,
      fixtureBytes(wireFixtureDir, "showcase-number-values.json"),
    );
    expect(value.numberGrid?.[0]).toEqual([
      -0,
      5,
      1000,
      Number.MAX_VALUE,
      Number.MIN_VALUE,
    ]);
    const actual = (serialized as { numberGrid: number[][] }).numberGrid[0];
    const expected = (
      loadFixture("showcase-number-values.json") as { numberGrid: number[][] }
    ).numberGrid[0];
    expect(actual.every((number, index) => number === expected[index])).toBe(true);
  });

  test("enforces pattern constraints with RE2-safe portable semantics", () => {
    // sku `^[A-Z]{2,4}$` and phrase `^\S+\s\S+$` round-trip.
    const patterns = expectRoundTrip(
      "showcase-patterns.json",
      showcaseTransferTypeConverter,
    );
    expect(patterns.sku).toBe("AB");
    expect(patterns.phrase).toBe("hello world");

    const base = {
      kind: "showcase",
      revision: 1,
      enabled: true,
      status: "active",
      tier: 1,
      scale: 1.5,
      name: "w",
      count: 1,
      active: true,
      category: "tools",
    };

    // Lowercase / too-long sku.
    expect(() =>
      showcaseTransferTypeConverter.fromTransferType({ ...base, sku: "ab" }),
    ).toThrow(/must match pattern/);
    expect(() =>
      showcaseTransferTypeConverter.fromTransferType({ ...base, sku: "ABCDE" }),
    ).toThrow(/must match pattern/);

    // phrase with no whitespace separator.
    expect(() =>
      showcaseTransferTypeConverter.fromTransferType({ ...base, phrase: "helloworld" }),
    ).toThrow(/must match pattern/);

    // `\s` ASCII-class crux: a NBSP (U+00A0) is NOT ASCII whitespace. The loader
    // rewrote `\s`/`\S` to an explicit ASCII class spliced into the emitted
    // RegExp, so JS's otherwise-Unicode `\s` rejects it — consistent with
    // Go/Python/Java.
    expect(() =>
      showcaseTransferTypeConverter.fromTransferType({
        ...base,
        phrase: "hello world",
      }),
    ).toThrow(/must match pattern/);

    // `$` end-anchor crux: a trailing newline is rejected. JS `$` is already
    // end-of-input (no `\n` exception), matching the `\Z`/`\z` rewrite applied
    // for Python/Java.
    expect(() =>
      showcaseTransferTypeConverter.fromTransferType({
        ...base,
        phrase: "hello world\n",
      }),
    ).toThrow(/must match pattern/);

    // A valid ASCII-space phrase and sku are accepted.
    const okPattern = showcaseTransferTypeConverter.fromTransferType({
      ...base,
      sku: "XY",
      phrase: "hello world",
    });
    expect(okPattern.sku).toBe("XY");
    expect(okPattern.phrase).toBe("hello world");
  });

  test("enforces asserted string formats with pinned, portable checks", () => {
    // uuid/email/hostname/uri/ipv4 round-trip (string-typed, no materialization).
    const formats = expectRoundTrip(
      "showcase-format.json",
      showcaseTransferTypeConverter,
    );
    expect(formats.requestId).toBe("de305d54-75b4-431b-adb2-eb6b9e546013");
    expect(formats.contactEmail).toBe("user@example.com");
    expect(formats.host).toBe("api.example.com");
    expect(formats.homepage).toBe("https://example.com/path?q=1#frag");
    expect(formats.gateway).toBe("192.168.0.1");

    const base = {
      kind: "showcase",
      revision: 1,
      enabled: true,
      status: "active",
      tier: 1,
      scale: 1.5,
      name: "w",
      count: 1,
      active: true,
      category: "tools",
    };

    // A malformed uuid.
    expect(() =>
      showcaseTransferTypeConverter.fromTransferType({
        ...base,
        requestId: "not-a-uuid",
      }),
    ).toThrow(/must be a valid uuid, got "not-a-uuid"/);

    // Single-label email domain (user@localhost) is rejected.
    expect(() =>
      showcaseTransferTypeConverter.fromTransferType({
        ...base,
        contactEmail: "user@localhost",
      }),
    ).toThrow(/must be a valid email, got "user@localhost"/);

    // ipv4 octet out of range.
    expect(() =>
      showcaseTransferTypeConverter.fromTransferType({ ...base, gateway: "256.0.0.1" }),
    ).toThrow(/must be a valid ipv4, got "256.0.0.1"/);

    // uri with a double-`::` IPv6 IP-literal host (spliced ipv6 grammar rejects).
    expect(() =>
      showcaseTransferTypeConverter.fromTransferType({
        ...base,
        homepage: "http://[1::2::3]",
      }),
    ).toThrow(/must be a valid uri/);

    // An over-long hostname (> 253 code points) is rejected by the length guard.
    const longHost = Array.from({ length: 64 }, () => "abc").join(".");
    expect(() =>
      showcaseTransferTypeConverter.fromTransferType({ ...base, host: longHost }),
    ).toThrow(/must be a valid hostname/);
  });

  test("enforces object member-count, propertyNames, and dependentRequired", () => {
    // Valid map and object round-trip.
    const attributes = expectRoundTrip(
      "attributes.json",
      attributesTransferTypeConverter,
    );
    expect(attributes.additionalProperties).toEqual({ host: "a", port: "8080" });
    const contact = expectRoundTrip("contact.json", contactTsTransferTypeConverter);
    expect(contact.shippingStreet).toBe("1 Main St");
    expect(contact.shippingZip).toBe("90210");

    // minProperties:1 on a map — an empty object is too few.
    expect(() => attributesTransferTypeConverter.fromTransferType({})).toThrow(
      /must have at least 1 properties, got 0/,
    );
    // maxProperties:3 on a map.
    expect(() =>
      attributesTransferTypeConverter.fromTransferType({
        a: "1",
        b: "2",
        c: "3",
        d: "4",
      }),
    ).toThrow(/must have at most 3 properties, got 4/);
    // propertyNames maxLength:8 — an over-long key.
    expect(() =>
      attributesTransferTypeConverter.fromTransferType({ toolongkey: "1" }),
    ).toThrow(/invalid property name "toolongkey": must have length <= 8, got 10/);

    // dependentRequired — a shipping street present without a shipping zip.
    expect(() =>
      contactTsTransferTypeConverter.fromTransferType({ shippingStreet: "1 Main St" }),
    ).toThrow(/property "shippingZip" is required when "shippingStreet" is present/);
    // minProperties:1 on a declared-property object — an empty object.
    expect(() => contactTsTransferTypeConverter.fromTransferType({})).toThrow(
      /must have at least 1 properties, got 0/,
    );
  });

  test("round-trips oneOf sum types and rejects unmatchable/unknown members", () => {
    // Disjoint-kind union (string | number): each branch round-trips and
    // narrows natively.
    const asString = expectRoundTrip(
      "showcase-union-string.json",
      showcaseTransferTypeConverter,
    );
    expect(asString.idOrName).toBe("abc");
    const asInt = expectRoundTrip(
      "showcase-union-int.json",
      showcaseTransferTypeConverter,
    );
    expect(asInt.idOrName).toBe(7);

    // Discriminated (tagged) union (Circle | Square) selected by `kind`.
    const circle = expectRoundTrip(
      "showcase-shape-circle.json",
      showcaseTransferTypeConverter,
    );
    expect(circle.shape).toMatchObject({ kind: "circle", radius: 2.5 });
    const square = expectRoundTrip(
      "showcase-shape-square.json",
      showcaseTransferTypeConverter,
    );
    expect(square.shape).toMatchObject({ kind: "square", side: 4 });

    const base = {
      kind: "showcase",
      revision: 1,
      enabled: true,
      status: "active",
      tier: 1,
      scale: 1.5,
      name: "w",
      count: 1,
      active: true,
      category: "tools",
    } as const;

    // An unmatchable wire token (boolean) names the admissible kinds.
    expect(() =>
      showcaseTransferTypeConverter.fromTransferType({ ...base, idOrName: true }),
    ).toThrow(/expected one of: string, integer/);

    // An unknown discriminator value is rejected (closed value set, P13.1).
    expect(() =>
      showcaseTransferTypeConverter.fromTransferType({
        ...base,
        shape: { kind: "triangle" },
      }),
    ).toThrow(/unknown discriminator kind triangle/);
  });

  test("holds a union member to the constraints of the branch it selects", () => {
    const base = {
      kind: "showcase",
      revision: 1,
      enabled: true,
      status: "active",
      tier: 1,
      scale: 1.5,
      name: "w",
      count: 1,
      active: true,
      category: "tools",
    } as const;
    const converter = showcaseTransferTypeConverter;

    // The string branch's own `minLength` and the integer branch's own
    // `minimum` — each enforced only for the branch the token selects.
    expect(converter.fromTransferType({ ...base, idOrName: "abc" }).idOrName).toBe(
      "abc",
    );
    expect(converter.fromTransferType({ ...base, idOrName: 1 }).idOrName).toBe(1);
    expect(() => converter.fromTransferType({ ...base, idOrName: "ab" })).toThrow(
      /idOrName: must have length >= 3, got 2/,
    );
    expect(() => converter.fromTransferType({ ...base, idOrName: 0 })).toThrow(
      /idOrName: must be >= 1, got 0/,
    );

    // A closed value set on a branch narrows to a literal union type, and an
    // unknown string is a Violation.
    expect(converter.fromTransferType({ ...base, mode: "manual" }).mode).toBe("manual");
    expect(() => converter.fromTransferType({ ...base, mode: "turbo" })).toThrow(
      /mode: must be one of \["auto", "manual"\]/,
    );

    // The array branch's `minItems`/`uniqueItems` and the string branch's
    // `pattern`, on the same union.
    expect(() => converter.fromTransferType({ ...base, measurements: [] })).toThrow(
      /measurements: must have at least 1 items, got 0/,
    );
    expect(() =>
      converter.fromTransferType({ ...base, measurements: [1.5, 1.5] }),
    ).toThrow(/duplicate items: element at index 1 equals index 0/);
    expect(() => converter.fromTransferType({ ...base, measurements: "AUTO" })).toThrow(
      /measurements: must match pattern/,
    );

    // Serialize re-runs the selected branch's constraints (P12).
    const valid = converter.fromTransferType({ ...base, idOrName: "abc" });
    expect(() => converter.toTransferType({ ...valid, idOrName: "ab" })).toThrow(
      /idOrName: must have length >= 3, got 2/,
    );
    expect(() => converter.toTransferType({ ...valid, measurements: [2, 2] })).toThrow(
      /duplicate items: element at index 1 equals index 0/,
    );

    // A named element union validates through its own converter, in both
    // directions, with the element's index on the violation path.
    expect(
      converter.fromTransferType({ ...base, segments: ["ab", 0] }).segments,
    ).toEqual(["ab", 0]);
    expect(() => converter.fromTransferType({ ...base, segments: ["a"] })).toThrow(
      /segments\[0\]: must have length >= 2, got 1/,
    );
    expect(() => converter.toTransferType({ ...valid, segments: [-1] })).toThrow(
      /must be >= 0, got -1/,
    );
  });

  test("round-trips the free-form object as a union branch and a named model", () => {
    // The inline object branch of the `payload` union, and the named `Extras`
    // model: members are carried verbatim in both.
    const asObject = expectRoundTrip(
      "showcase-freeform.json",
      showcaseTransferTypeConverter,
    );
    expect(asObject.payload).toEqual({ note: "free-form", big: 9007199254740992 });
    expect(asObject.extras?.additionalProperties).toEqual({ note: "free-form" });

    // The same union's string branch, selected by the wire token.
    const asString = expectRoundTrip(
      "showcase-freeform-string.json",
      showcaseTransferTypeConverter,
    );
    expect(asString.payload).toBe("text");

    // The named free-form model round-trips standalone, nested members included.
    const extras = expectRoundTrip("extras.json", extrasTransferTypeConverter);
    expect(extras.additionalProperties.nested).toEqual({ a: 1 });

    // maxProperties over the member set is enforced on parse…
    expect(() =>
      extrasTransferTypeConverter.fromTransferType({ a: 1, b: 2, c: 3, d: 4, e: 5 }),
    ).toThrow(/must have at most 4 properties/);

    // …and on serialize (P12).
    expect(() =>
      extrasTransferTypeConverter.toTransferType({
        additionalProperties: { a: 1, b: 2, c: 3, d: 4, e: 5 },
      }),
    ).toThrow(/must have at most 4 properties/);

    const base = {
      kind: "showcase",
      revision: 1,
      enabled: true,
      status: "active",
      tier: 1,
      scale: 1.5,
      name: "w",
      count: 1,
      active: true,
      category: "tools",
    } as const;

    // An unmatchable wire token (boolean) names the admissible kinds.
    expect(() =>
      showcaseTransferTypeConverter.fromTransferType({ ...base, payload: true }),
    ).toThrow(/expected one of: object, string/);
  });

  test("round-trips a tagged union whose object branches are written inline", () => {
    // Each `note` branch is named by its `x-ts-name` override and emitted as an
    // interface with its own converter, so the union narrows on the `kind` literal.
    const text = expectRoundTrip(
      "showcase-note-text.json",
      showcaseTransferTypeConverter,
    );
    const note = text.note as TextNote;
    expect(note.kind).toBe("text");
    expect(note.body).toBe("remember the milk");
    // The branch stays open: an unknown member is preserved (P13).
    expect(note.additionalProperties).toEqual({ pinned: true });

    const link = expectRoundTrip(
      "showcase-note-link.json",
      showcaseTransferTypeConverter,
    );
    expect((link.note as LinkNote).href).toBe("https://example.test/notes/1");

    const base = {
      kind: "showcase",
      revision: 1,
      enabled: true,
      status: "active",
      tier: 1,
      scale: 1.5,
      name: "w",
      count: 1,
      active: true,
      category: "tools",
    } as const;

    // The selected branch's own constraints are enforced.
    expect(() =>
      showcaseTransferTypeConverter.fromTransferType({
        ...base,
        note: { kind: "text", body: "" },
      }),
    ).toThrow(/must have length >= 1/);

    // An unknown tag value matches no branch.
    expect(() =>
      showcaseTransferTypeConverter.fromTransferType({
        ...base,
        note: { kind: "audio" },
      }),
    ).toThrow(/unknown discriminator kind/);

    // Serialize dispatches on the tag and re-runs that branch's constraints (P12).
    expect(() =>
      showcaseTransferTypeConverter.toTransferType({
        ...link,
        note: { kind: "link", href: "", additionalProperties: {} },
      }),
    ).toThrow(/must have length >= 1/);
  });

  test("round-trips a union written inline on a property with an object branch", () => {
    // `detail`'s lone structured object branch derives `ShowcaseDetailObject` from
    // the union it belongs to and gets an interface + converter, so its members keep
    // their constraints while the string branch selects on its own token.
    const object = expectRoundTrip(
      "showcase-detail-object.json",
      showcaseTransferTypeConverter,
    );
    const detail = object.detail as ShowcaseDetailObject;
    expect(detail.code).toBe("E_LIMIT");
    expect(detail.hint).toBe("retry later");
    // The branch stays open: an unknown member is preserved (P13).
    expect(detail.additionalProperties).toEqual({ retryAfterMs: 250 });

    const text = expectRoundTrip(
      "showcase-detail-string.json",
      showcaseTransferTypeConverter,
    );
    expect(text.detail).toBe("E_LIMIT");

    const base = {
      kind: "showcase",
      revision: 1,
      enabled: true,
      status: "active",
      tier: 1,
      scale: 1.5,
      name: "w",
      count: 1,
      active: true,
      category: "tools",
    } as const;

    // The object branch's own constraints are enforced.
    expect(() =>
      showcaseTransferTypeConverter.fromTransferType({ ...base, detail: { code: "" } }),
    ).toThrow(/must have length >= 1/);

    // A value admitted by no branch names the admissible ones.
    expect(() =>
      showcaseTransferTypeConverter.fromTransferType({ ...base, detail: 7 }),
    ).toThrow(/expected one of: ShowcaseDetailObject, string/);

    // Serialize picks the object branch by shape and re-runs its constraints (P12).
    expect(() =>
      showcaseTransferTypeConverter.toTransferType({
        ...object,
        detail: { code: "", additionalProperties: {} },
      }),
    ).toThrow(/must have length >= 1/);
  });

  test("round-trips a tagged object union mixed with a scalar branch", () => {
    // `shapeOrName` composes both selector layers: the JSON token picks
    // object-vs-string, then the `kind` const picks Circle-vs-Square. Circle and
    // Square are the same branch types the `shape` union uses.
    const square = expectRoundTrip(
      "showcase-shape-or-name-square.json",
      showcaseTransferTypeConverter,
    );
    expect(square.shapeOrName).toMatchObject({ kind: "square", side: 4 });

    const named = expectRoundTrip(
      "showcase-shape-or-name-string.json",
      showcaseTransferTypeConverter,
    );
    expect(named.shapeOrName).toBe("unit-square");

    const base = {
      kind: "showcase",
      revision: 1,
      enabled: true,
      status: "active",
      tier: 1,
      scale: 1.5,
      name: "w",
      count: 1,
      active: true,
      category: "tools",
    } as const;

    // The object token still routes through the discriminator, so an unknown tag
    // is rejected rather than falling back to the string branch.
    expect(() =>
      showcaseTransferTypeConverter.fromTransferType({
        ...base,
        shapeOrName: { kind: "triangle" },
      }),
    ).toThrow(/unknown discriminator kind triangle/);

    // A token matching no branch names all admissible ones.
    expect(() =>
      showcaseTransferTypeConverter.fromTransferType({ ...base, shapeOrName: 7 }),
    ).toThrow(/expected one of: Circle, Square, string/);
  });

  test("round-trips a union with an array branch", () => {
    // `measurements` is `number[] | string`: TypeScript carries the array branch
    // structurally (no synthesized variant type) and narrows with Array.isArray.
    const list = expectRoundTrip(
      "showcase-measurements-array.json",
      showcaseTransferTypeConverter,
    );
    expect(list.measurements).toEqual([1.5, 2.5, 3.75]);

    const preset = expectRoundTrip(
      "showcase-measurements-string.json",
      showcaseTransferTypeConverter,
    );
    expect(preset.measurements).toBe("auto");

    const base = {
      kind: "showcase",
      revision: 1,
      enabled: true,
      status: "active",
      tier: 1,
      scale: 1.5,
      name: "w",
      count: 1,
      active: true,
      category: "tools",
    } as const;

    // A token matching neither branch names both admissible kinds.
    expect(() =>
      showcaseTransferTypeConverter.fromTransferType({ ...base, measurements: true }),
    ).toThrow(/expected one of: number\[\], string/);
  });

  test("round-trips unions in element positions", () => {
    // Three positions with no property of their own: an array element at a
    // named union (`shapes`), an array element at an inline union the loader
    // names `ShowcaseSegmentsItem`, and a map member at an inline union named
    // `ChoicesValue`. Each element runs its union's own converter, so a bad value
    // is reported at its index / key.
    const value = expectRoundTrip(
      "showcase-element-unions.json",
      showcaseTransferTypeConverter,
    );

    expect(value.shapes).toEqual([
      { kind: "circle", radius: 2.5, additionalProperties: {} },
      { kind: "square", side: 4, additionalProperties: {} },
    ]);
    expect(value.segments).toEqual(["alpha", 7]);
    // Element nullability is the element's own concern: `(string | null)[]`,
    // so an explicit null is a member rather than a violation.
    expect(value.slots).toEqual(["first", null, "third"]);
    expect(value.choices?.additionalProperties.primary).toEqual({
      kind: "circle",
      radius: 1,
      additionalProperties: {},
    });

    const base = {
      kind: "showcase",
      revision: 1,
      enabled: true,
      status: "active",
      tier: 1,
      scale: 1.5,
      name: "w",
      count: 1,
      active: true,
      category: "tools",
    } as const;

    expect(() =>
      showcaseTransferTypeConverter.fromTransferType({
        ...base,
        shapes: [{ kind: "circle", radius: 1 }, true],
      }),
    ).toThrow(/shapes\[1\]/);
    expect(() =>
      showcaseTransferTypeConverter.fromTransferType({
        ...base,
        shapes: [{ kind: "triangle" }],
      }),
    ).toThrow(/triangle/);
    expect(() =>
      showcaseTransferTypeConverter.fromTransferType({
        ...base,
        segments: ["ok", 1.5],
      }),
    ).toThrow(/segments\[1\]/);
    expect(() =>
      showcaseTransferTypeConverter.fromTransferType({
        ...base,
        choices: { primary: "circle" },
      }),
    ).toThrow(/primary/);
  });

  test("rejects invalid in-memory values on serialize (P12, both directions)", () => {
    // A valid model round-trips; mutating a single field to an out-of-spec value
    // and re-serializing (toTransferType) is rejected before any wire object is
    // produced, with the same informative reason as the parse path.
    const full = showcaseTransferTypeConverter.fromTransferType(
      loadFixture("showcase-full.json"),
    );

    // Numeric bound: an in-memory value past `maximum` fails to serialize.
    expect(() =>
      showcaseTransferTypeConverter.toTransferType({ ...full, priority: 42 }),
    ).toThrow(/must be <= 10, got 42/);

    // String length: an in-memory over-long string fails to serialize.
    expect(() =>
      showcaseTransferTypeConverter.toTransferType({ ...full, code: "abcdef" }),
    ).toThrow(/must have length <= 5, got 6/);

    // Pattern: an in-memory off-pattern value fails to serialize.
    expect(() =>
      showcaseTransferTypeConverter.toTransferType({ ...full, sku: "xyz" }),
    ).toThrow(/must match pattern/);

    // Format: an in-memory malformed uuid fails to serialize.
    expect(() =>
      showcaseTransferTypeConverter.toTransferType({ ...full, requestId: "nope" }),
    ).toThrow(/must be a valid uuid, got "nope"/);

    // Array: an in-memory duplicate (uniqueItems) fails to serialize.
    expect(() =>
      showcaseTransferTypeConverter.toTransferType({
        ...full,
        aliases: ["dup", "dup"],
      }),
    ).toThrow(/duplicate items: element at index 1 equals index 0/);

    // Closed value-set: a mutated enum member fails to serialize.
    expect(() =>
      showcaseTransferTypeConverter.toTransferType({
        ...full,
        status: "archived" as (typeof full)["status"],
      }),
    ).toThrow(/must be one of \["active", "inactive", "pending"\], got "archived"/);

    // const: a mutated integer const fails to serialize.
    expect(() =>
      showcaseTransferTypeConverter.toTransferType({
        ...full,
        revision: 2 as (typeof full)["revision"],
      }),
    ).toThrow(/must equal 1/);

    for (const [replacement, reason] of [
      [{ count: true }, /count: expected integer/],
      [{ count: 1.5 }, /count: expected integer/],
      [{ active: 1 }, /active: expected boolean/],
      [{ nickname: [] }, /nickname: expected string/],
      [{ tags: "ab" }, /tags: expected array/],
      [{ shapes: 7 }, /shapes: expected array/],
      [{ name: null }, /name: required/],
    ] as const) {
      expect(() =>
        showcaseTransferTypeConverter.toTransferType({
          ...full,
          ...replacement,
        } as unknown as Showcase),
      ).toThrow(reason);
    }
    expect(() => addressTransferTypeConverter.toTransferType(7 as never)).toThrow(
      /expected object/,
    );
    expect(() =>
      attributesTransferTypeConverter.toTransferType({
        additionalProperties: { host: [] },
      } as never),
    ).toThrow(/host: expected string/);
    expect(() =>
      attributesTransferTypeConverter.toTransferType({
        additionalProperties: null,
      } as never),
    ).toThrow(/expected object/);

    // allOf-merged bound: an in-memory `size` past the tightened maximum fails.
    const widget = widgetTransferTypeConverter.fromTransferType(
      loadFixture("widget.json"),
    );
    expect(() =>
      widgetTransferTypeConverter.toTransferType({ ...widget, size: 25 }),
    ).toThrow(/must be <= 20, got 25/);

    // Object dependentRequired: a shipping street with no zip fails to serialize.
    expect(() =>
      contactTsTransferTypeConverter.toTransferType({
        shippingStreet: "1 Main St",
        additionalProperties: {},
      }),
    ).toThrow(/property "shippingZip" is required when "shippingStreet" is present/);

    // Object member-count: an empty map is below minProperties:1 on serialize.
    expect(() =>
      attributesTransferTypeConverter.toTransferType({ additionalProperties: {} }),
    ).toThrow(/must have at least 1 properties, got 0/);
    // propertyNames key-shape: an over-long key fails to serialize.
    expect(() =>
      attributesTransferTypeConverter.toTransferType({
        additionalProperties: { toolongkey: "1" },
      }),
    ).toThrow(/invalid property name "toolongkey": must have length <= 8, got 10/);

    // A valid model still serializes cleanly (no false rejection).
    expect(() => showcaseTransferTypeConverter.toTransferType(full)).not.toThrow();
  });

  test("roundtrips materialized contentEncoding bytes and rejects malformed", () => {
    // blob (base64) and urlBlob (base64url) round-trip: a JSON string on the
    // wire, a native Uint8Array in the model, re-encoded byte-identically via the
    // pure-JS codec. The same bytes (">>>") encode to "Pj4+" vs "Pj4-".
    const bytes = expectRoundTrip("showcase-bytes.json", showcaseTransferTypeConverter);
    const expected = new Uint8Array([0x3e, 0x3e, 0x3e]);
    expect(bytes.blob).toEqual(expected);
    expect(bytes.urlBlob).toEqual(expected);

    const base = {
      kind: "showcase",
      status: "active",
      tier: 1,
      scale: 1.5,
      name: "w",
      count: 1,
      active: true,
      category: "tools",
    } as const;

    // A base64 field using the URL-safe alphabet is rejected by the pinned regex.
    expect(() =>
      showcaseTransferTypeConverter.fromTransferType({ ...base, blob: "Pj4-" }),
    ).toThrow(/must be base64-encoded, got "Pj4-"/);

    // A base64 field missing padding is rejected.
    expect(() =>
      showcaseTransferTypeConverter.fromTransferType({ ...base, blob: "aGk" }),
    ).toThrow(/must be base64-encoded/);

    // A base64url field carrying padding is rejected.
    expect(() =>
      showcaseTransferTypeConverter.fromTransferType({ ...base, urlBlob: "aGk=" }),
    ).toThrow(/must be base64url-encoded, got "aGk="/);
  });

  test("round-trips object shapes written inline", () => {
    // An object written inline in a value position is named after that position
    // and emitted as an ordinary model: a property (`location`, with its own
    // nested `geo`), a nullable property (`audit`), an array element (`rows`), a
    // map and its member (`ledger`), and a free-form bag (`metadata`). The same
    // fixture covers a typed map's member constraints (`quotas`, `tokens`,
    // `nicknames`) and a nested array (`grid`).
    const value = expectRoundTrip(
      "showcase-inline-shapes.json",
      showcaseTransferTypeConverter,
    );

    expect(value.grid).toEqual([[1, 2], [3]]);
    expect(value.location).toEqual({
      city: "Springfield",
      geo: { lat: 39.8, lon: -89.6, additionalProperties: {} },
      additionalProperties: {},
    });
    expect(value.audit).toEqual({ by: "alice", additionalProperties: {} });
    expect(value.rows?.[0]).toEqual({ cell: "a1", additionalProperties: {} });
    // The member override renamed the member (`ledgerTs`); the hoisted types keep
    // their position-derived names.
    expect(value.ledgerTs?.additionalProperties.opening).toEqual({
      amount: 100,
      additionalProperties: {},
    });
    expect(value.metadata?.additionalProperties).toEqual({
      source: "import",
      batch: 7,
    });
    expect(value.quotas?.additionalProperties).toEqual({ cpu: 20, memory: 100 });
    // A null member of a nullable map is a member, not a violation.
    expect(value.nicknames?.additionalProperties).toEqual({ short: "al", none: null });

    const base = {
      kind: "showcase",
      revision: 1,
      enabled: true,
      status: "active",
      tier: 1,
      scale: 1.5,
      name: "w",
      count: 1,
      active: true,
      category: "tools",
    } as const;

    // A hoisted shape validates like any other model, at the nested path.
    expect(() =>
      showcaseTransferTypeConverter.fromTransferType({
        ...base,
        location: { city: "" },
      }),
    ).toThrow(/location\.city/);
    expect(() =>
      showcaseTransferTypeConverter.fromTransferType({
        ...base,
        rows: [{ cell: "ok" }, {}],
      }),
    ).toThrow(/rows\[1\]\.cell/);
    // A nested array reports the failing element at its own two-dimensional index.
    expect(() =>
      showcaseTransferTypeConverter.fromTransferType({
        ...base,
        grid: [[1], [2, 1.5]],
      }),
    ).toThrow(/grid\[1\]\[1\]/);
    // A typed map's member constraints are enforced, keyed by the member.
    expect(() =>
      showcaseTransferTypeConverter.fromTransferType({ ...base, quotas: { cpu: 7 } }),
    ).toThrow(/cpu/);
    expect(() =>
      showcaseTransferTypeConverter.fromTransferType({
        ...base,
        tokens: { primary: "AB" },
      }),
    ).toThrow(/primary/);
    expect(() =>
      showcaseTransferTypeConverter.fromTransferType({
        ...base,
        nicknames: { tiny: "a" },
      }),
    ).toThrow(/tiny/);
    // The free-form bag's member-count bound rides with the hoisted type.
    expect(() =>
      showcaseTransferTypeConverter.fromTransferType({
        ...base,
        metadata: { a: 1, b: 2, c: 3, d: 4 },
      }),
    ).toThrow(/at most 3/);

    // Serialize re-runs every member's own constraints before emitting (P12).
    expect(() =>
      quotasTransferTypeConverter.toTransferType({ additionalProperties: { cpu: 7 } }),
    ).toThrow(/cpu/);
    expect(() =>
      tokensTransferTypeConverter.toTransferType({
        additionalProperties: { primary: "AB" },
      }),
    ).toThrow(/primary/);
    expect(() =>
      nicknamesTransferTypeConverter.toTransferType({
        additionalProperties: { tiny: "a" },
      }),
    ).toThrow(/tiny/);
  });

  test("recursively converts collections and rejects non-finite numbers", () => {
    const value = expectRoundTrip(
      "showcase-recursive-collections.json",
      showcaseTransferTypeConverter,
    );
    expect(value.numberGrid).toEqual([[1, 2.5], [3]]);
    expect(value.addresses?.[0]?.street).toBe("1 Main St");
    expect(value.addressBook?.additionalProperties.home?.street).toBe("2 Side St");
    expect(value.dates).toEqual(["0001-01-01", "2024-02-29"]);
    expect(value.blobs?.[0]).toEqual(new Uint8Array([104, 105]));
    expect(value.blobIndex?.additionalProperties.hi).toEqual(
      new Uint8Array([104, 105]),
    );

    const base = {
      kind: "showcase",
      revision: 1,
      enabled: true,
      status: "active",
      tier: 1,
      scale: 1.5,
      name: "w",
      count: 1,
      active: true,
      category: "tools",
    } as const;
    expect(() =>
      showcaseTransferTypeConverter.fromTransferType({
        ...base,
        slots: ["x"],
        links: ["not a uri"],
      }),
    ).toThrow(/slots\[0\][\s\S]*links\[0\]/);

    for (const [replacement, path] of [
      [{ score: Number.NaN }, "score"],
      [{ metricOrLabel: Number.POSITIVE_INFINITY }, "metricOrLabel"],
      [{ measurements: [Number.NEGATIVE_INFINITY] }, "measurements[0]"],
      [{ numberGrid: [[1, Number.NaN]] }, "numberGrid[0][1]"],
      [
        { metrics: { additionalProperties: { cpu: Number.POSITIVE_INFINITY } } },
        "metrics.cpu",
      ],
    ] as const) {
      expect(() =>
        showcaseTransferTypeConverter.toTransferType({
          ...value,
          ...replacement,
        } as Showcase),
      ).toThrow(new RegExp(`${path.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}.*finite`));
    }

    // The array branch of a union uses the ordinary model mapper: the
    // in-memory catch-all bag is flattened back into wire members.
    const address = addressTransferTypeConverter.fromTransferType({
      street: "3 Branch Ave",
      city: "Capital City",
      district: 7,
    });
    const serialized = showcaseTransferTypeConverter.toTransferType({
      ...value,
      addressListOrLabel: [address],
    }) as Record<string, unknown>;
    expect(serialized.addressListOrLabel).toEqual([
      { street: "3 Branch Ave", city: "Capital City", district: 7 },
    ]);

    expect(() =>
      showcaseTransferTypeConverter.toTransferType({
        ...value,
        addressListOrLabel: [address, null],
      } as Showcase),
    ).toThrow(/addressListOrLabel\[1\]/);
  });
});
