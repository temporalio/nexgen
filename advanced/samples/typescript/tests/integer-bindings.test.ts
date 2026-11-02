import Long from "long";
import { beforeEach, describe, expect, test, vi } from "vitest";

import { Clock, Counter } from "../wit/integer-bindings/resources.ts";
import {
  timestampTransferTypeConverter,
  nestedRequestTransferTypeConverter,
} from "../wit/integer-bindings/models.ts";
import { getClock } from "../wit/integer-bindings/operations/get-clock.ts";

const startOperation = vi.hoisted(() => vi.fn());
vi.mock("@temporalio/workflow", () => ({
  createNexusServiceClient: () => ({ startOperation }),
}));

describe("binding-aware 64-bit resource values", () => {
  beforeEach(() => {
    startOperation.mockReset();
  });

  test("uses Long for constructor fields and nested proto parameters", async () => {
    startOperation.mockResolvedValue({ result: async () => undefined });
    const seconds = Long.fromNumber(1000);
    const clock = new Clock(seconds, 42);
    await clock.touch();
    await clock.setTime(seconds.add(1), 43);

    const direct = startOperation.mock.calls[0][1];
    const nested = startOperation.mock.calls[1][1].at;
    expect(direct.seconds).toBe(seconds);
    expect(nested.seconds.toString()).toBe("1001");
    for (const model of [direct, nested]) {
      const proto = timestampTransferTypeConverter.toTransferType(model);
      expect(
        timestampTransferTypeConverter
          .fromTransferType(proto)
          .seconds?.equals(model.seconds),
      ).toBe(true);
    }
    const nestedProto = nestedRequestTransferTypeConverter.toTransferType({
      at: nested,
    });
    expect(nestedProto.createTime?.seconds?.toString()).toBe("1001");
  });

  test("keeps JSON-only constructor fields and parameters as numbers", async () => {
    startOperation.mockResolvedValue({ result: async () => undefined });
    const counter = new Counter(10);
    await counter.add(5);
    expect(counter.total).toBe(10);
    expect(startOperation.mock.calls[0][1]).toEqual({ total: 10, amount: 5 });
  });

  test("preserves proto request fields when an operation returns a resource", async () => {
    const seconds = Long.fromNumber(3600);
    startOperation.mockResolvedValue({
      result: async () => ({}),
    });
    const clock = await getClock({ seconds, nanos: 42 });
    expect(clock).toBeInstanceOf(Clock);
    expect(clock.seconds?.toString()).toBe("3600");
    expect(clock.nanos).toBe(42);
  });
});
