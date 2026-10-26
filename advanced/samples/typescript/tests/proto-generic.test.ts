import * as common from "@temporalio/common";
import { temporal } from "@temporalio/proto";
import { afterEach, beforeEach, describe, expect, expectTypeOf, it, vi } from "vitest";

import {
  type PayloadBackedEnvelope,
  payloadBackedContextTransferTypeConverter as contextConverter,
  payloadBackedEnvelopeTransferTypeConverter as envelopeConverter,
  payloadBackedOutputTransferTypeConverter as outputConverter,
  optionalPayloadsTransferTypeConverter as optionalConverter,
} from "../wit/proto-generic/index.ts";

interface Output {
  message: string;
}

interface Context {
  workflowId: string;
}

beforeEach(() => {
  vi.stubGlobal("__TEMPORAL_ACTIVATOR__", {
    payloadConverter: common.defaultPayloadConverter,
  });
});

afterEach(() => vi.unstubAllGlobals());

describe("proto-generic generated models", () => {
  it("preserves optional generic Payloads and checks cardinality", () => {
    expect(optionalConverter.fromTransferType({})).toEqual({ details: undefined });
    expect(optionalConverter.toTransferType({})).toEqual({ details: undefined });
    expect(optionalConverter.fromTransferType({ details: { payloads: [] } })).toEqual({
      details: undefined,
    });
    const model = { details: { message: "optional" } };
    const wire = optionalConverter.toTransferType(model);
    expect(wire.details?.payloads).toHaveLength(1);
    const decoded = optionalConverter.fromTransferType<Output>(
      temporal.api.failure.v1.CanceledFailureInfo.decode(
        temporal.api.failure.v1.CanceledFailureInfo.encode(wire).finish(),
      ),
    );
    expectTypeOf(decoded.details).toEqualTypeOf<Output | undefined>();
    expect(decoded).toEqual(model);
    const payload = common.defaultPayloadConverter.toPayload(model.details);
    expect(() =>
      optionalConverter.fromTransferType({
        details: { payloads: [payload, payload] },
      }),
    ).toThrow("expected at most one payload");
  });
  it("round-trips independent type arguments through nested records and protobuf bytes", () => {
    const model: PayloadBackedEnvelope<Output, Context> = {
      provider: { details: { message: "completed" } },
      scaler: { details: { workflowId: "workflow-id" } },
    };
    const wire = envelopeConverter.toTransferType(model);
    expect(wire.provider).not.toHaveProperty("type");
    expect(wire.provider).not.toHaveProperty("nexusEndpoint");
    expect(wire.scaler).not.toHaveProperty("type");
    expect(wire).not.toHaveProperty("taskQueueTypes");

    const message = temporal.api.compute.v1.ComputeConfigScalingGroup.decode(
      temporal.api.compute.v1.ComputeConfigScalingGroup.encode(wire).finish(),
    );
    const decoded = envelopeConverter.fromTransferType<Output, Context>(message);
    expect(decoded).toEqual(model);
    expectTypeOf(decoded.provider.details).toEqualTypeOf<Output>();
    expectTypeOf(decoded.scaler.details).toEqualTypeOf<Context>();
    expect(
      common.defaultPayloadConverter.fromPayload(message.provider!.details!),
    ).toEqual(model.provider.details);
    expect(
      common.defaultPayloadConverter.fromPayload(message.scaler!.details!),
    ).toEqual(model.scaler.details);

    // @ts-expect-error The two nested records retain distinct type arguments.
    decoded.provider.details = decoded.scaler.details;
  });

  it("supports standalone converters and nullish application values", () => {
    const output = { details: { message: "completed" } };
    expect(
      outputConverter.fromTransferType<Output>(outputConverter.toTransferType(output)),
    ).toEqual(output);
    const context = { details: { workflowId: "workflow-id" } };
    expect(
      contextConverter.fromTransferType<Context>(
        contextConverter.toTransferType(context),
      ),
    ).toEqual(context);
    const nullish: PayloadBackedEnvelope<undefined, null> = {
      provider: { details: undefined },
      scaler: { details: null },
    };
    expect(
      envelopeConverter.fromTransferType<undefined, null>(
        envelopeConverter.toTransferType(nullish),
      ),
    ).toEqual(nullish);
  });

  it("uses the configured converter inside both nested records", () => {
    const toPayload = vi.fn((value: unknown) =>
      common.defaultPayloadConverter.toPayload({ wrapped: value }),
    );
    const fromPayload = vi.fn(
      (payload: common.Payload) =>
        common.defaultPayloadConverter.fromPayload<{ wrapped: unknown }>(payload)
          .wrapped,
    );
    vi.stubGlobal("__TEMPORAL_ACTIVATOR__", {
      payloadConverter: {
        toPayload,
        fromPayload<T>(payload: common.Payload): T {
          return fromPayload(payload) as T;
        },
      } satisfies common.PayloadConverter,
    });
    const model: PayloadBackedEnvelope<string, number> = {
      provider: { details: "completed" },
      scaler: { details: 42 },
    };
    expect(
      envelopeConverter.fromTransferType<string, number>(
        envelopeConverter.toTransferType(model),
      ),
    ).toEqual(model);
    expect(toPayload).toHaveBeenCalledTimes(2);
    expect(fromPayload).toHaveBeenCalledTimes(2);
  });

  it("rejects missing records and payload envelopes", () => {
    const provider = outputConverter.toTransferType({ details: "output" });
    const scaler = contextConverter.toTransferType({ details: "context" });
    expect(() => envelopeConverter.fromTransferType({ scaler })).toThrow(
      "missing required field PayloadBackedEnvelope.provider",
    );
    expect(() => envelopeConverter.fromTransferType({ provider })).toThrow(
      "missing required field PayloadBackedEnvelope.scaler",
    );
    expect(() => envelopeConverter.fromTransferType({ provider: {}, scaler })).toThrow(
      "missing required field PayloadBackedOutput.details",
    );
    expect(() => envelopeConverter.fromTransferType({ provider, scaler: {} })).toThrow(
      "missing required field PayloadBackedContext.details",
    );
  });
});
