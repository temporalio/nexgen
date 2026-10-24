import * as common from "@temporalio/common";
import type { temporal } from "@temporalio/proto";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  createOnCompleteRequestTransferTypeConverter,
  type OnCompleteRequest,
  onCompleteRequestTransferTypeConverter as requestConverter,
  onCompleteResponseTransferTypeConverter as responseConverter,
} from "../wit/notification-service/index.ts";
import { singleValueFromPayloads } from "../support/temporal_model_converters.ts";

interface Completion {
  message: string;
}

interface SourceContext {
  workflowId: string;
}

const runtime = globalThis as typeof globalThis & {
  __TEMPORAL_ACTIVATOR__?: { payloadConverter?: common.PayloadConverter };
};
let previousActivator: typeof runtime.__TEMPORAL_ACTIVATOR__;

beforeEach(() => {
  previousActivator = runtime.__TEMPORAL_ACTIVATOR__;
  runtime.__TEMPORAL_ACTIVATOR__ = {
    payloadConverter: common.defaultPayloadConverter,
  };
});

afterEach(() => {
  if (previousActivator === undefined) {
    delete runtime.__TEMPORAL_ACTIVATOR__;
  } else {
    runtime.__TEMPORAL_ACTIVATOR__ = previousActivator;
  }
});

describe("notification-service generated models", () => {
  it("preserves RawValue success payloads while decoding source context normally", () => {
    const converter = createOnCompleteRequestTransferTypeConverter<
      common.RawValue,
      SourceContext
    >({ outputT: common.rawValueTypeInfo });
    const payload: common.Payload = {
      metadata: { encoding: new TextEncoder().encode("unknown/opaque") },
      data: new Uint8Array([1, 2, 3]),
    };
    const sourceContext = common.defaultPayloadConverter.toPayload({
      workflowId: "workflow-id",
    });
    const decoded = converter.fromTransferType({
      result: "success",
      success: payload,
      sourceContext,
    });
    expect(decoded.sourceContext).toEqual({ workflowId: "workflow-id" });
    if (decoded.result.tag !== "success") throw new Error("expected success");
    expect(decoded.result.value).toBeInstanceOf(common.RawValue);
    expect(decoded.result.value.payload).toBe(payload);
    expect(converter.toTransferType(decoded).success).toBe(payload);

    // Configured converters are ordinary SDK transfer converters.
    const inputType: common.TypeInfo<
      OnCompleteRequest<common.RawValue, SourceContext>,
      temporal.api.notificationservice.v1.IOnCompleteRequest
    > = { transferTypeConverter: converter };
    expect(inputType.transferTypeConverter).toBe(converter);
    // @ts-expect-error A RawValue converter cannot decode a Completion.
    const wrong: common.TransferTypeConverter<
      OnCompleteRequest<Completion, SourceContext>
    > = converter;
    expect(wrong).toBe(converter);
  });

  it("binds independent transfer types and passes payload converter hints", () => {
    const outputHint = { converter: "output-test" };
    const contextHint = { converter: "context-test" };
    const toPayload = vi.fn(
      (
        value: unknown,
        _context?: common.SerializationContext,
        _hint?: common.ConverterHint,
      ) => common.defaultPayloadConverter.toPayload(value),
    );
    const fromPayload = vi.fn(
      (
        payload: common.Payload,
        _context?: common.SerializationContext,
        _hint?: common.ConverterHint,
      ) => common.defaultPayloadConverter.fromPayload(payload),
    );
    runtime.__TEMPORAL_ACTIVATOR__ = {
      payloadConverter: {
        toPayload,
        fromPayload<T>(
          payload: common.Payload,
          context?: common.SerializationContext,
          hint?: common.ConverterHint<T>,
        ): T {
          return fromPayload(payload, context, hint) as T;
        },
      },
    };
    const converter = createOnCompleteRequestTransferTypeConverter<
      Completion,
      SourceContext
    >({
      outputT: {
        payloadConverterHint: outputHint,
        transferTypeConverter: {
          toTransferType: (value: Completion) => value.message,
          fromTransferType: (value: string): Completion => ({ message: value }),
        },
      },
      sourceContextT: {
        payloadConverterHint: contextHint,
        transferTypeConverter: {
          toTransferType: (value: SourceContext) => value.workflowId,
          fromTransferType: (value: string): SourceContext => ({ workflowId: value }),
        },
      },
    });
    const model: OnCompleteRequest<Completion, SourceContext> = {
      result: { tag: "success", value: { message: "done" } },
      sourceContext: { workflowId: "workflow-id" },
    };
    const wire = converter.toTransferType(model);
    expect(toPayload).toHaveBeenNthCalledWith(1, "done", undefined, outputHint);
    expect(toPayload).toHaveBeenNthCalledWith(2, "workflow-id", undefined, contextHint);
    expect(converter.fromTransferType(wire)).toEqual(model);
    expect(fromPayload).toHaveBeenNthCalledWith(1, wire.success, undefined, outputHint);
    expect(fromPayload).toHaveBeenNthCalledWith(
      2,
      wire.sourceContext,
      undefined,
      contextHint,
    );
  });

  it("round-trips distinct generic output and source context values", () => {
    const model: OnCompleteRequest<Completion, SourceContext> = {
      result: { tag: "success", value: { message: "completed" } },
      sourceContext: { workflowId: "workflow-id" },
    };
    const wire = requestConverter.toTransferType(model);
    expect(wire.result).toBe("success");
    expect(wire.success).toBeDefined();
    expect(wire.failure).toBeUndefined();
    expect(wire.sourceContext).toBeDefined();
    expect(requestConverter.fromTransferType<Completion, SourceContext>(wire)).toEqual(
      model,
    );

    // Type erasure at runtime must not erase correlation in the public API.
    const wrong: OnCompleteRequest<Completion, SourceContext> = {
      // @ts-expect-error SourceContext is not a Completion.
      result: { tag: "success", value: { workflowId: "wrong" } },
      sourceContext: { workflowId: "workflow-id" },
    };
    expect(wrong.result.tag).toBe("success");
  });

  it("uses the configured payload converter for both application values", () => {
    const toPayload = vi.fn((value: unknown) =>
      common.defaultPayloadConverter.toPayload({ wrapped: value }),
    );
    const fromPayload = vi.fn(
      (payload: common.Payload) =>
        common.defaultPayloadConverter.fromPayload<{ wrapped: unknown }>(payload)
          .wrapped,
    );
    runtime.__TEMPORAL_ACTIVATOR__ = {
      payloadConverter: {
        toPayload,
        fromPayload<T>(payload: common.Payload): T {
          return fromPayload(payload) as T;
        },
      },
    };
    const model: OnCompleteRequest<string, number> = {
      result: { tag: "success", value: "done" },
      sourceContext: 42,
    };
    const wire = requestConverter.toTransferType(model);
    expect(common.defaultPayloadConverter.fromPayload(wire.success!)).toEqual({
      wrapped: "done",
    });
    expect(requestConverter.fromTransferType<string, number>(wire)).toEqual(model);
    expect(toPayload).toHaveBeenCalledTimes(2);
    expect(fromPayload).toHaveBeenCalledTimes(2);
  });

  it("round-trips failure details without populating success", () => {
    const model: OnCompleteRequest<Completion, SourceContext> = {
      result: {
        tag: "failure",
        value: common.ApplicationFailure.create({
          message: "operation failed",
          type: "NotificationFailure",
          details: [{ reason: "test" }],
        }),
      },
      sourceContext: { workflowId: "workflow-id" },
    };
    const wire = requestConverter.toTransferType(model);
    expect(wire.success).toBeUndefined();
    expect(wire.failure).toBeDefined();
    const decoded = requestConverter.fromTransferType<Completion, SourceContext>(wire);
    expect(decoded.sourceContext).toEqual(model.sourceContext);
    expect(decoded.result.tag).toBe("failure");
    if (decoded.result.tag !== "failure") throw new Error("expected failure");
    const failure = decoded.result.value as common.ApplicationFailure;
    expect(failure.message).toBe("operation failed");
    expect(failure.type).toBe("NotificationFailure");
    expect(failure.details).toEqual([{ reason: "test" }]);
  });

  it("preserves null and undefined application values inside required envelopes", () => {
    const model: OnCompleteRequest<undefined, null> = {
      result: { tag: "success", value: undefined },
      sourceContext: null,
    };
    const wire = requestConverter.toTransferType(model);
    expect(wire.success).toBeDefined();
    expect(wire.sourceContext).toBeDefined();
    expect(requestConverter.fromTransferType<undefined, null>(wire)).toEqual(model);
  });

  it("validates single-value Payloads carriers", () => {
    expect(singleValueFromPayloads({})).toBeUndefined();
    const payload = common.defaultPayloadConverter.toPayload({ message: "done" });
    expect(singleValueFromPayloads<Completion>({ payloads: [payload] })).toEqual({
      message: "done",
    });
    expect(() => singleValueFromPayloads({ payloads: [payload, payload] })).toThrow(
      "expected at most one payload",
    );
  });

  it("requires the discriminator and its selected wire field", () => {
    const payload = common.defaultPayloadConverter.toPayload("value");
    expect(() => requestConverter.fromTransferType({ sourceContext: payload })).toThrow(
      "missing required field OnCompleteRequest.result",
    );
    expect(() => requestConverter.fromTransferType({ success: payload })).toThrow(
      "missing required field OnCompleteRequest.result",
    );
    expect(() =>
      requestConverter.fromTransferType({ result: "success", success: payload }),
    ).toThrow("missing required field OnCompleteRequest.sourceContext");
    expect(() =>
      requestConverter.fromTransferType({ result: "success", sourceContext: payload }),
    ).toThrow("missing required field OnCompleteRequest.success");
  });

  it("uses the discriminator rather than inspecting other members", () => {
    const payload = common.defaultPayloadConverter.toPayload("value");
    const selected: temporal.api.notificationservice.v1.IOnCompleteRequest = {
      result: "success",
      success: payload,
      failure: {},
      sourceContext: payload,
    };
    expect(requestConverter.fromTransferType<string, string>(selected)).toEqual({
      result: { tag: "success", value: "value" },
      sourceContext: "value",
    });
    expect(() =>
      requestConverter.fromTransferType({
        ...selected,
        // @ts-expect-error Exercise malformed JavaScript input.
        result: "unknown",
      }),
    ).toThrow("invalid oneof discriminator for OnCompleteRequest.result");
  });

  it("rejects missing or invalid result values when encoding", () => {
    const model: OnCompleteRequest<string, string> = {
      result: { tag: "success", value: "done" },
      sourceContext: "context",
    };
    expect(() =>
      requestConverter.toTransferType({
        ...model,
        // @ts-expect-error Exercise malformed JavaScript input.
        result: null,
      }),
    ).toThrow("missing required field OnCompleteRequest.result");
    expect(() =>
      requestConverter.toTransferType({
        ...model,
        // @ts-expect-error Exercise malformed JavaScript input.
        result: { tag: "unknown", value: "value" },
      }),
    ).toThrow("invalid oneof tag for OnCompleteRequest.result");
  });

  it("round-trips an empty acknowledgement", () => {
    expect(responseConverter.toTransferType({})).toEqual({});
    expect(responseConverter.fromTransferType({})).toEqual({});
  });
});
