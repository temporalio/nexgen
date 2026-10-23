import Long from "long";
import { describe, expect, test } from "vitest";

import type { streamsample } from "#streamsample";
import { readRecordsRequestTransferTypeConverter } from "../wit/stream-service/models.ts";

describe("stream-service generated output", () => {
  // The proto interfaces are kept by hand, so this fails to compile when they drift from what
  // the generated converters read and write.
  test("maps every field of a read request through its proto", () => {
    const proto: streamsample.v1.IReadRecordsRequest = {
      stream: { id: "stream-1", workflowId: "workflow-1" },
      fromOffset: Long.fromNumber(7),
      topics: ["a"],
      maxRecords: 10,
    };

    const model = readRecordsRequestTransferTypeConverter.fromTransferType(proto);

    expect(model.stream).toEqual({ id: "stream-1", workflowId: "workflow-1" });
    expect(model.fromOffset?.toNumber()).toBe(7);
    expect(model.topics).toEqual(["a"]);
    expect(model.maxRecords).toBe(10);
    expect(
      readRecordsRequestTransferTypeConverter.toTransferType(model).stream,
    ).toEqual(proto.stream);
  });
});
