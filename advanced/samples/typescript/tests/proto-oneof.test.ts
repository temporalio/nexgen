import * as common from "@temporalio/common";
import { temporal } from "@temporalio/proto";
import { afterEach, beforeEach, describe, expect, expectTypeOf, it, vi } from "vitest";

import {
  createOutcomeTransferTypeConverter,
  type Outcome,
  type PauseActivityRequest,
  outcomeTransferTypeConverter as outcomeConverter,
  pauseActivityRequestTransferTypeConverter as pauseConverter,
} from "../wit/proto-oneof/index.ts";

beforeEach(() => {
  vi.stubGlobal("__TEMPORAL_ACTIVATOR__", {
    payloadConverter: common.defaultPayloadConverter,
  });
});

afterEach(() => vi.unstubAllGlobals());

function pauseRequest(): PauseActivityRequest {
  return {
    namespace: "default",
    execution: { workflowId: "workflow-id", runId: "run-id" },
    identity: "worker",
    reason: "test",
    requestId: "request-id",
  };
}

describe("proto-oneof generated models", () => {
  it("preserves RawValue in a generic Payloads carrier without payload conversion", () => {
    const converter = createOutcomeTransferTypeConverter({
      outputT: common.rawValueTypeInfo,
    });
    const toPayload = vi.fn(() => {
      throw new Error("must bypass payload converter");
    });
    const fromPayload = vi.fn(() => {
      throw new Error("must bypass payload converter");
    });
    vi.stubGlobal("__TEMPORAL_ACTIVATOR__", {
      payloadConverter: { toPayload, fromPayload },
    });
    const payload: common.Payload = {
      metadata: { encoding: new TextEncoder().encode("unknown/opaque") },
      data: new Uint8Array([1, 2, 3]),
    };
    const wire = converter.toTransferType({
      value: { tag: "success", value: common.RawValue.fromPayload(payload) },
    });
    expect(wire.success!.payloads).toEqual([payload]);
    const decoded = converter.fromTransferType(wire);
    if (decoded.value.tag !== "success") throw new Error("expected success");
    expectTypeOf(decoded.value.value).toEqualTypeOf<common.RawValue>();
    expect(decoded.value.value).toBeInstanceOf(common.RawValue);
    expect(decoded.value.value.payload).toBe(payload);
    expect(converter.fromTransferType({ value: "success", success: {} })).toEqual({
      value: { tag: "success", value: undefined },
    });
    expect(() =>
      converter.fromTransferType({
        value: "success",
        success: { payloads: [payload, payload] },
      }),
    ).toThrow("expected at most one payload");
    expect(toPayload).not.toHaveBeenCalled();
    expect(fromPayload).not.toHaveBeenCalled();
  });

  it("round-trips a generic success through Payloads and protobuf bytes", () => {
    const model: Outcome<{ message: string }> = {
      value: { tag: "success", value: { message: "completed" } },
    };
    const wire = outcomeConverter.toTransferType(model);
    expect(wire.value).toBe("success");
    expect(wire.failure).toBeUndefined();
    expect(wire.success?.payloads).toHaveLength(1);
    const message = temporal.api.update.v1.Outcome.decode(
      temporal.api.update.v1.Outcome.encode(wire).finish(),
    );
    const decoded = outcomeConverter.fromTransferType<{ message: string }>(message);
    expect(decoded).toEqual(model);
    if (decoded.value.tag !== "success") throw new Error("expected success");
    expectTypeOf(decoded.value.value).toEqualTypeOf<{ message: string }>();
    // @ts-expect-error Generic success values keep their declared type.
    decoded.value.value = 42;
  });

  it("round-trips failure details through protobuf bytes", () => {
    const failure = common.ApplicationFailure.create({
      message: "operation failed",
      type: "OneofFailure",
      details: [{ reason: "test" }],
    });
    const wire = outcomeConverter.toTransferType<string>({
      value: { tag: "failure", value: failure },
    });
    expect(wire.success).toBeUndefined();
    const message = temporal.api.update.v1.Outcome.decode(
      temporal.api.update.v1.Outcome.encode(wire).finish(),
    );
    const decoded = outcomeConverter.fromTransferType<string>(message);
    if (decoded.value.tag !== "failure") throw new Error("expected failure");
    const decodedFailure = decoded.value.value as common.ApplicationFailure;
    expect(decodedFailure.message).toBe(failure.message);
    expect(decodedFailure.type).toBe(failure.type);
    expect(decodedFailure.details).toEqual(failure.details);
  });

  it("supports an empty success and rejects multiple payloads", () => {
    expect(
      outcomeConverter.fromTransferType<undefined>({
        value: "success",
        success: {},
      }),
    ).toEqual({
      value: { tag: "success", value: undefined },
    });
    const payload = common.defaultPayloadConverter.toPayload("value");
    expect(() =>
      outcomeConverter.fromTransferType({
        value: "success",
        success: { payloads: [payload, payload] },
      }),
    ).toThrow("expected at most one payload");
    expect(
      outcomeConverter.fromTransferType<undefined>(
        outcomeConverter.toTransferType<undefined>({
          value: { tag: "success", value: undefined },
        }),
      ),
    ).toEqual({ value: { tag: "success", value: undefined } });
  });

  it("rejects a missing discriminator or selected member", () => {
    expect(() => outcomeConverter.fromTransferType({})).toThrow(
      "missing required field Outcome.value",
    );
    expect(() =>
      outcomeConverter.fromTransferType({ success: {}, failure: {} }),
    ).toThrow("missing required field Outcome.value");
    expect(() => outcomeConverter.fromTransferType({ value: "success" })).toThrow(
      "missing required field Outcome.success",
    );
    expect(() =>
      outcomeConverter.toTransferType({
        // @ts-expect-error Exercise malformed JavaScript input.
        value: null,
      }),
    ).toThrow("missing required field Outcome.value");
  });

  it.each(["id", "type"] as const)(
    "round-trips the optional %s case through protobuf bytes",
    (tag) => {
      const model = { ...pauseRequest(), activity: { tag, value: "activity" } };
      const wire = pauseConverter.toTransferType(model);
      expect(wire.activity).toBe(tag);
      expect(wire[tag]).toBe("activity");
      expect(wire[tag === "id" ? "type" : "id"]).toBeUndefined();
      const message = temporal.api.workflowservice.v1.PauseActivityRequest.decode(
        temporal.api.workflowservice.v1.PauseActivityRequest.encode(wire).finish(),
      );
      expect(pauseConverter.fromTransferType(message)).toEqual(model);
      expect(pauseConverter.fromTransferType(JSON.parse(JSON.stringify(wire)))).toEqual(
        model,
      );
    },
  );

  it("preserves an unset optional oneof on SDK messages and plain objects", () => {
    const model = pauseRequest();
    const wire = pauseConverter.toTransferType(model);
    expect(wire.id).toBeUndefined();
    expect(wire.type).toBeUndefined();
    expect(pauseConverter.fromTransferType(wire).activity).toBeUndefined();
    const message = temporal.api.workflowservice.v1.PauseActivityRequest.decode(
      temporal.api.workflowservice.v1.PauseActivityRequest.encode(wire).finish(),
    );
    expect(pauseConverter.fromTransferType(message)).toEqual(model);
  });

  it.each([null, undefined])(
    "handles an absent optional nested message (%s) at the caller",
    (execution) => {
      const model = { ...pauseRequest(), execution: undefined };
      const wire = pauseConverter.toTransferType(model);
      expect(wire.execution).toBeUndefined();
      expect(
        pauseConverter.fromTransferType({ ...wire, execution }).execution,
      ).toBeUndefined();
    },
  );

  it("preserves an empty scalar and follows the explicit discriminator", () => {
    const model: PauseActivityRequest = {
      ...pauseRequest(),
      activity: { tag: "id", value: "" },
    };
    const wire = pauseConverter.toTransferType(model);
    const message = temporal.api.workflowservice.v1.PauseActivityRequest.decode(
      temporal.api.workflowservice.v1.PauseActivityRequest.encode(wire).finish(),
    );
    expect(pauseConverter.fromTransferType(message)).toEqual(model);
    expect(
      pauseConverter.fromTransferType({ ...wire, id: "id", type: "type" }).activity,
    ).toEqual({ tag: "id", value: "id" });
    expect(() =>
      pauseConverter.fromTransferType({
        ...wire,
        // @ts-expect-error Exercise malformed JavaScript input.
        activity: "unknown",
      }),
    ).toThrow("invalid oneof discriminator for PauseActivityRequest.activity");
  });
});
