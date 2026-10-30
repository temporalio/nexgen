import * as common from "@temporalio/common";
import type { temporal } from "@temporalio/proto";
import Long from "long";
import { describe, expect, test } from "vitest";

import {
  type ReadRecordsResponse,
  StreamCloseStatus,
  readRecordsRequestTransferTypeConverter,
  readRecordsResponseTransferTypeConverter,
} from "../wit/stream-service/workflow/models.ts";

describe("stream-service generated output", () => {
  // No SDK has these protos yet, so proto/streams.d.ts declares them. The converters fail to
  // compile when those declarations drift from what they read and write.
  test("maps each read item kind through the proto oneof", () => {
    const model: ReadRecordsResponse = {
      records: [
        {
          item: {
            tag: "data",
            value: {
              body: common.defaultPayloadConverter.toPayload("hi"),
            },
          },
          offset: Long.fromNumber(1),
        },
        {
          item: {
            tag: "superseded",
            value: { producerId: "workflow-1", failedTasks: [Long.fromNumber(7)] },
          },
          offset: Long.fromNumber(2),
        },
        {
          item: {
            tag: "close",
            value: { status: StreamCloseStatus.StreamCloseStatusCompleted },
          },
          offset: Long.fromNumber(3),
        },
      ],
      nextOffset: Long.fromNumber(4),
      done: true,
    };

    const proto: temporal.api.streamservice.v1.IReadRecordsResponse =
      readRecordsResponseTransferTypeConverter.toTransferType(model);

    expect(proto.records?.map((record) => record.item)).toEqual([
      "data",
      "superseded",
      "close",
    ]);
    expect(proto.records?.[1].superseded?.failedTasks?.[0].toNumber()).toBe(7);
    expect(readRecordsResponseTransferTypeConverter.fromTransferType(proto)).toEqual(
      model,
    );
  });

  test("gives no item for a read record whose item kind only a newer server knows", () => {
    const proto: temporal.api.streamservice.v1.IReadRecordsResponse = {
      records: [{ offset: Long.fromNumber(1) }],
    };

    const model = readRecordsResponseTransferTypeConverter.fromTransferType(proto);

    expect(model.records?.[0].item).toBeUndefined();
    expect(model.records?.[0].offset?.toNumber()).toBe(1);
  });

  test("maps every field of a read request through its proto", () => {
    const proto: temporal.api.streamservice.v1.IReadRecordsRequest = {
      stream: { id: "stream-1", owner: { businessId: "workflow-1" } },
      fromOffset: Long.fromNumber(7),
      maxRecords: 10,
    };

    const model = readRecordsRequestTransferTypeConverter.fromTransferType(proto);

    expect(model.stream.id).toBe("stream-1");
    expect(model.stream.owner?.businessId).toBe("workflow-1");
    expect(model.fromOffset?.toNumber()).toBe(7);
    expect(model.maxRecords).toBe(10);
    expect(
      readRecordsRequestTransferTypeConverter.toTransferType(model).stream?.id,
    ).toBe("stream-1");
  });
});
