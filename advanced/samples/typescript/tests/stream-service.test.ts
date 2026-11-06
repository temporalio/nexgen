import * as common from "@temporalio/common";
import type { temporal } from "@temporalio/proto";
import Long from "long";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

import {
  type ReadRecordsResponse,
  StreamStatus,
  appendRecordsRequestTransferTypeConverter,
  createReadRecordsResponseTransferTypeConverter,
  describeStreamResponseTransferTypeConverter,
  readRecordsRequestTransferTypeConverter,
  readRecordsResponseTransferTypeConverter,
} from "../wit/stream-service/workflow/models.ts";
import { Stream } from "../wit/stream-service/workflow/resources.ts";

const startOperation = vi.hoisted(() => vi.fn());
vi.mock("@temporalio/workflow", () => ({
  createNexusServiceClient: () => ({ startOperation }),
}));

class Turn {
  constructor(readonly role: string) {}
}

const turnType: common.TypeInfo<Turn, { role: string }> = {
  transferTypeConverter: {
    toTransferType: (turn) => ({ role: turn.role }),
    fromTransferType: (value) => new Turn(value.role),
  },
};

// A body goes through the payload converter of the Workflow.
beforeEach(() => {
  vi.stubGlobal("__TEMPORAL_ACTIVATOR__", {
    payloadConverter: common.defaultPayloadConverter,
  });
});

afterEach(() => vi.unstubAllGlobals());

describe("stream-service generated output", () => {
  beforeEach(() => {
    startOperation.mockReset();
  });

  // No SDK has these protos yet, so proto/streams.d.ts declares them. The converters fail to
  // compile when those declarations drift from what they read and write.
  test("maps each read item kind through the proto oneof", () => {
    const model: ReadRecordsResponse<string> = {
      records: [
        {
          item: {
            tag: "data",
            value: { body: "hi" },
          },
          offset: Long.fromNumber(1),
        },
        {
          item: {
            tag: "superseded",
            value: {
              writer: {
                producer: { id: "workflow-1" },
                failedTasks: [Long.fromNumber(7)],
              },
            },
          },
          offset: Long.fromNumber(2),
        },
      ],
      nextOffset: Long.fromNumber(3),
      done: true,
    };

    const proto: temporal.api.streamservice.v1.IReadRecordsResponse =
      readRecordsResponseTransferTypeConverter.toTransferType(model);

    expect(proto.records?.map((record) => record.item)).toEqual(["data", "superseded"]);
    expect(proto.records?.[1].superseded?.writer?.failedTasks?.[0].toNumber()).toBe(7);
    expect(readRecordsResponseTransferTypeConverter.fromTransferType(proto)).toEqual(
      model,
    );
  });

  // A Stream<Turn> read gives its runtime type to this converter.
  test("decodes the bodies of a read with the runtime type of its stream", () => {
    const proto: temporal.api.streamservice.v1.IReadRecordsResponse = {
      records: [
        {
          item: "data",
          data: { body: common.defaultPayloadConverter.toPayload({ role: "user" }) },
          offset: Long.fromNumber(1),
        },
      ],
    };

    const model = createReadRecordsResponseTransferTypeConverter({
      bodyT: turnType,
    }).fromTransferType(proto);

    const item = model.records?.[0].item;
    expect(item?.tag === "data" ? item.value.body : undefined).toStrictEqual(
      new Turn("user"),
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
      ref: { id: "stream-1", owner: { businessId: "workflow-1" } },
      fromOffset: Long.fromNumber(7),
      maxRecords: 10,
    };

    const model = readRecordsRequestTransferTypeConverter.fromTransferType(proto);

    expect(model.ref.id).toBe("stream-1");
    expect(model.ref.owner?.businessId).toBe("workflow-1");
    expect(model.fromOffset?.toNumber()).toBe(7);
    expect(model.maxRecords).toBe(10);
    expect(readRecordsRequestTransferTypeConverter.toTransferType(model).ref?.id).toBe(
      "stream-1",
    );
  });

  test("wraps each value of an append in its record on the wire", () => {
    const proto = appendRecordsRequestTransferTypeConverter.toTransferType({
      ref: { id: "stream-1" },
      records: ["a", "b"],
      writer: { producer: { id: "client:sample" }, failedTasks: [] },
    });

    // `append-record` flattens, so the model holds the values and the proto holds the records.
    expect(
      proto.records?.map((record) =>
        common.defaultPayloadConverter.fromPayload(record.body!),
      ),
    ).toEqual(["a", "b"]);
    expect(
      appendRecordsRequestTransferTypeConverter.fromTransferType(proto).records,
    ).toEqual(["a", "b"]);
  });

  test("preserves offsets above 2^53 through resource calls and proto conversion", async () => {
    const offset = Long.fromString("1880158843699200005");
    startOperation.mockResolvedValue({
      result: async () => ({ records: [], nextOffset: offset.add(1) }),
    });
    const stream = new Stream({ id: "stream-1" }, undefined);

    const page = await stream.read(offset);
    await stream.read(page.nextOffset);

    const requests = startOperation.mock.calls.map(([, request]) => request);
    expect(requests.map((request) => request.fromOffset.toString())).toEqual([
      "1880158843699200005",
      "1880158843699200006",
    ]);
    for (const request of requests) {
      const proto = readRecordsRequestTransferTypeConverter.toTransferType(request);
      const model = readRecordsRequestTransferTypeConverter.fromTransferType(proto);
      expect(model.fromOffset?.equals(request.fromOffset)).toBe(true);
    }
  });

  test("decodes an enum number that it doesn't know as the fallback case", () => {
    // A newer server can send a Stream status that this code doesn't know.
    const proto: temporal.api.streamservice.v1.IDescribeStreamResponse = {
      status: 9 as temporal.api.enums.v1.StreamStatus,
    };

    const model = describeStreamResponseTransferTypeConverter.fromTransferType(proto);

    expect(model.status).toBe(StreamStatus.StreamStatusUnspecified);
  });
});
