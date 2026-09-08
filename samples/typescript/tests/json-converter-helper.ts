import { readFileSync } from "node:fs";
import {
  ApplicationFailure,
  defaultPayloadConverter,
  fromPayloadWithTypeInfo,
  toPayloadWithTypeInfo,
} from "@temporalio/common";
import type { TransferTypeConverter } from "nexus-rpc";

const encoder = new TextEncoder();

/**
 * Make legacy reason-text assertions inspect detail 0 while preserving the
 * actual ApplicationFailure instance and metadata. This mutates test-local
 * converter instances only; reading the retained detail performs no serialization.
 */
export function exposeValidationDetails(
  ...converters: TransferTypeConverter<any>[]
): void {
  const rethrow = (error: unknown): never => {
    if (
      error instanceof ApplicationFailure &&
      error.type === "PayloadValidationError" &&
      Array.isArray(error.details?.[0])
    ) {
      error.message = (error.details[0] as { path: string; reason: string }[])
        .map(({ path, reason }) => (path ? `${path}: ${reason}` : reason))
        .join("\n");
    }
    throw error;
  };
  for (const converter of converters) {
    const fromTransferType = converter.fromTransferType.bind(converter);
    const toTransferType = converter.toTransferType.bind(converter);
    Object.defineProperties(converter, {
      fromTransferType: {
        value: (value: unknown) => {
          try {
            return fromTransferType(value);
          } catch (error) {
            return rethrow(error);
          }
        },
      },
      toTransferType: {
        value: (value: unknown) => {
          try {
            return toTransferType(value);
          } catch (error) {
            return rethrow(error);
          }
        },
      },
    });
  }
}

/** Wrap raw fixture bytes as a json/plain Temporal payload. */
function jsonPayload(data: Uint8Array) {
  return { metadata: { encoding: encoder.encode("json/plain") }, data };
}

/**
 * Deserialize fixture bytes into a generated model *through the Temporal data
 * converter*: the default payload converter decodes the json/plain bytes into a
 * plain transfer value, which the generated `TransferTypeConverter`'s
 * `fromTransferType` turns into the typed model. This proves the converter the
 * generator attaches to each operation drives converter-based deserialization.
 */
export function decodeFixture<T>(
  converter: TransferTypeConverter<T>,
  bytes: Uint8Array,
): T {
  return fromPayloadWithTypeInfo(
    defaultPayloadConverter,
    jsonPayload(bytes),
    undefined,
    { transferTypeConverter: converter },
  );
}

/**
 * Serialize a model back through the Temporal data converter and return the
 * re-encoded JSON as a generic parsed value (for JSON-equality assertions).
 */
export function encodeModel<T>(converter: TransferTypeConverter<T>, value: T): unknown {
  const payload = toPayloadWithTypeInfo(defaultPayloadConverter, value, undefined, {
    transferTypeConverter: converter,
  });
  if (payload?.data == null) {
    throw new Error("payload converter produced no data");
  }
  return JSON.parse(Buffer.from(payload.data).toString("utf8"));
}

/**
 * Round-trip a fixture through the converter: decode the bytes into the model,
 * re-encode, and return both the model and the re-serialized JSON so callers can
 * assert JSON-equality against the fixture.
 */
export function roundTripFixture<T>(
  converter: TransferTypeConverter<T>,
  bytes: Uint8Array,
): { value: T; serialized: unknown } {
  const value = decodeFixture(converter, bytes);
  return { value, serialized: encodeModel(converter, value) };
}

/** Read a canonical wire fixture as raw bytes. */
export function fixtureBytes(dir: URL, name: string): Uint8Array {
  return readFileSync(new URL(name, dir));
}

/** Read a canonical wire fixture as a parsed JSON value. */
export function loadFixture<T = unknown>(dir: URL, name: string): T {
  return JSON.parse(readFileSync(new URL(name, dir), "utf8")) as T;
}
