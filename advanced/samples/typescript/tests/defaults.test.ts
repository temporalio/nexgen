import {
  afterEach,
  beforeEach,
  describe,
  expect,
  expectTypeOf,
  test,
  vi,
} from "vitest";
import * as common from "@temporalio/common";
import * as support from "../support/temporal_model_converters.ts";
import {
  MyEnum,
  MyFlags,
  type RecordWithDefaults,
  type FlattenedRecordWithDefaults,
  type Request,
} from "../wit/defaults/models.ts";
import { submit } from "../wit/defaults/operations/submit.ts";
import { Sender } from "../wit/defaults/resources.ts";

const captured = vi.hoisted(() => ({ requests: [] as unknown[] }));

vi.mock("@temporalio/workflow", () => ({
  createNexusServiceClient: () => ({
    startOperation: async (_operation: unknown, input: unknown) => {
      captured.requests.push(input);
      return { result: async () => input };
    },
  }),
}));

beforeEach(() => {
  captured.requests.length = 0;
  vi.spyOn(support, "workflowNamespace").mockReturnValue("first-namespace");
});

afterEach(() => vi.restoreAllMocks());

function lastRequest(): Request {
  return captured.requests.at(-1) as Request;
}

describe("operation-scoped defaults", () => {
  test("resource forwarding uses the same defaults", async () => {
    const sender = new Sender("unused");
    await sender.submit();
    expect(lastRequest()).toMatchObject({
      namespace: "first-namespace",
      attempts: 3,
    });
    expect(lastRequest().retry).toBeUndefined();
    expect(lastRequest().summary).toBeUndefined();
    expect(support.workflowNamespace).toHaveBeenCalledTimes(1);
  });
  test("evaluates providers once per invocation", async () => {
    expect(support.workflowNamespace).not.toHaveBeenCalled();
    await submit({});
    const first = lastRequest();
    expect(first).toMatchObject({
      namespace: "first-namespace",
      attempts: 3,
      enabled: true,
      ratio: 1.5,
      mode: MyEnum.Standard,
      permissions: 3,
      aliasLabel: "alias",
    });
    expect(first.retry).toBeUndefined();
    expect(first.summary).toBeUndefined();
    expect(support.workflowNamespace).toHaveBeenCalledTimes(1);

    vi.mocked(support.workflowNamespace).mockReturnValue("second-namespace");
    await submit({ namespace: undefined });
    const second = lastRequest();
    expect(second.namespace).toBe("second-namespace");
    expect(support.workflowNamespace).toHaveBeenCalledTimes(2);
  });

  test("preserves supplied falsy values without evaluating providers", async () => {
    await submit({
      namespace: "",
      attempts: 0,
      enabled: false,
      ratio: 0,
      mode: MyEnum.Unspecified,
      permissions: MyFlags.Read & 0,
      aliasLabel: "",
      summary: "",
    });
    expect(lastRequest()).toMatchObject({
      namespace: "",
      attempts: 0,
      enabled: false,
      ratio: 0,
      mode: MyEnum.Unspecified,
      permissions: 0,
      aliasLabel: "",
      summary: "",
    });
    expect(support.workflowNamespace).not.toHaveBeenCalled();
  });

  test("passes nested records through and defaults flattened arguments", async () => {
    const retry: RecordWithDefaults = { attempts: 0, label: "" };
    await submit({ retry, details: "details" });
    expect(lastRequest()).toMatchObject({
      retry: { attempts: 0, label: "" },
      summary: "untitled",
      details: "details",
    });
    expect(lastRequest().retry).toBe(retry);
    expect(retry).toEqual({ attempts: 0, label: "" });
  });

  test("serialization and inbound data do not apply defaults", async () => {
    await submit({});
    const inbound: Request = {
      ...lastRequest(),
      namespace: "",
      attempts: 0,
      retry: { attempts: 0, label: "" },
    };
    for (let i = 0; i < 2; i++) {
      const payload = common.defaultPayloadConverter.toPayload(inbound);
      expect(common.defaultPayloadConverter.fromPayload(payload)).toEqual(inbound);
    }
    expect(support.workflowNamespace).toHaveBeenCalledTimes(1);
  });
});

expectTypeOf<RecordWithDefaults["attempts"]>().toEqualTypeOf<number>();
expectTypeOf<RecordWithDefaults["label"]>().toEqualTypeOf<string>();
expectTypeOf<FlattenedRecordWithDefaults["summary"]>().toEqualTypeOf<string>();
expectTypeOf<Request["summary"]>().toEqualTypeOf<string | undefined>();
expectTypeOf<Parameters<typeof submit>[0]["namespace"]>().toEqualTypeOf<
  string | undefined
>();
expectTypeOf<Parameters<typeof submit>[0]["summary"]>().toEqualTypeOf<
  string | undefined
>();
