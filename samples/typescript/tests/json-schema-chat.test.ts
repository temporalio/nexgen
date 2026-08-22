import { describe, expect, test } from "vitest";
import type { TransferTypeConverter } from "nexus-rpc";

import {
  DEFAULT_PRIORITY,
  labelsTransferTypeConverter,
  messageTransferTypeConverter,
  roomTransferTypeConverter,
  sendMessageInputTransferTypeConverter,
  sendMessageOutputTransferTypeConverter,
  ValidationError,
  type Labels,
  type Message,
} from "../chat/index.ts";
import {
  fixtureBytes,
  loadFixture as loadFixtureFrom,
  roundTripFixture,
} from "./json-converter-helper.ts";

const wireFixtureDir = new URL("../../wire/json_schema/chat/", import.meta.url);

function loadFixture(name: string): unknown {
  return loadFixtureFrom(wireFixtureDir, name);
}

// Round-trip a fixture through the Temporal data converter (driven by the
// generated converter) and assert the re-serialized JSON is JSON-equal to the
// fixture. TS converters preserve explicit nulls, so all chat fixtures use exact
// JSON-equality (no optional+nullable collapse — unlike Go).
function expectRoundTrip<T>(name: string, converter: TransferTypeConverter<T>): T {
  const { value, serialized } = roundTripFixture(
    converter,
    fixtureBytes(wireFixtureDir, name),
  );
  expect(serialized).toEqual(loadFixture(name));
  return value;
}

describe("json-schema chat generated definitions", () => {
  test("roundtrips canonical wire fixtures through the Temporal converter", () => {
    const message = expectRoundTrip(
      "message-minimal.json",
      messageTransferTypeConverter,
    );
    expect(message).toMatchObject<Message>({
      kind: "text",
      body: "hi",
    });
    expect(message.replyToId).toBeUndefined();
    expect(message.priority ?? DEFAULT_PRIORITY).toBe(0);

    const fullMessage = expectRoundTrip(
      "message-full.json",
      messageTransferTypeConverter,
    );
    expect(fullMessage.replyToId).toBeNull();
    expect(fullMessage.priority).toBe(7);

    const room = expectRoundTrip("room-open.json", roomTransferTypeConverter);
    expect(room.additionalProperties).toEqual({ "x-extra": 42 });

    const labels = expectRoundTrip("labels.json", labelsTransferTypeConverter);
    expect(labels).toMatchObject<Labels>({
      additionalProperties: { env: "prod", team: "core" },
    });

    const request = expectRoundTrip(
      "send-message-input.json",
      sendMessageInputTransferTypeConverter,
    );
    expect(request.message.body).toBe("hi");

    const response = expectRoundTrip(
      "send-message-output.json",
      sendMessageOutputTransferTypeConverter,
    );
    expect(response.messageId).toBe("m1");
  });

  test("reports JSON schema validation errors", () => {
    expect(() =>
      sendMessageInputTransferTypeConverter.fromTransferType({
        roomId: "r1",
        message: { kind: "text", body: "hi" },
        extra: true,
      }),
    ).toThrow(ValidationError);

    expect(() =>
      messageTransferTypeConverter.fromTransferType({ kind: "image", body: "hi" }),
    ).toThrow(ValidationError);

    expect(() => sendMessageOutputTransferTypeConverter.fromTransferType({})).toThrow(
      ValidationError,
    );

    try {
      messageTransferTypeConverter.fromTransferType({ kind: "image", body: "hi" });
      throw new Error("expected invalid payload to fail");
    } catch (error) {
      expect(error).toBeInstanceOf(ValidationError);
      expect((error as ValidationError).type).toBe("BAD_REQUEST");
    }

    expect(() =>
      roomTransferTypeConverter.toTransferType({
        roomId: "r1",
        displayName: "Room",
        topic: null,
        additionalProperties: { roomId: "shadow" },
      }),
    ).toThrow(/roomId.*collides with declared property/);
  });
});
