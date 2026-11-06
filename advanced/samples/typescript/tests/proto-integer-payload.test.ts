import { defaultPayloadConverter } from "@temporalio/common";
import { ProtobufBinaryPayloadConverter } from "@temporalio/common/lib/protobufs";
import proto from "@temporalio/proto";
import Long from "long";
import { describe, expect, test } from "vitest";

import {
  signedValueTransferTypeConverter,
  unsignedValueTransferTypeConverter,
} from "../wit/integer-bindings/models.ts";

describe("System Nexus integer envelope boundaries", () => {
  test.each(["9223372036854775808", "18446744073709551615"])(
    "normalizes signed Long bits for unsigned writes: %s",
    (decimal) => {
      const signed = Long.fromString(decimal);
      const wire = unsignedValueTransferTypeConverter.toTransferType({ value: signed });
      expect(wire.value?.unsigned).toBe(true);
      expect(wire.value?.toString()).toBe(decimal);
      const message = proto.google.protobuf.UInt64Value.create(wire);
      const decoded = proto.google.protobuf.UInt64Value.decode(
        proto.google.protobuf.UInt64Value.encode(message).finish(),
      );
      expect(decoded.value.toString()).toBe(decimal);
      // Conversion must not mutate the caller's Long.
      expect(signed.unsigned).toBe(false);
    },
  );

  test("preserves signed and unsigned values across the isolate JSON bridge", () => {
    // Model SDK main's System Nexus transport using real SDK serializers.
    // The installed SDK does not yet expose that transport: this covers its
    // serialization boundaries, not a live System Nexus operation.
    const binary = new ProtobufBinaryPayloadConverter(proto);
    for (const [transfer, messageType, value] of [
      [
        signedValueTransferTypeConverter,
        proto.google.protobuf.Int64Value,
        Long.fromString("9007199254740993"),
      ],
      [
        signedValueTransferTypeConverter,
        proto.google.protobuf.Int64Value,
        Long.fromString("-9007199254740993"),
      ],
      [
        unsignedValueTransferTypeConverter,
        proto.google.protobuf.UInt64Value,
        Long.fromString("18446744073709551615", true),
      ],
    ] as const) {
      const properties = transfer.toTransferType({ value });
      expect(Object.getPrototypeOf(properties)).toBe(Object.prototype);
      const inputEnvelope = defaultPayloadConverter.toPayload(properties)!;
      const workerProperties =
        defaultPayloadConverter.fromPayload<typeof properties>(inputEnvelope);
      // The Worker, not the generated converter, constructs the message.
      const message = messageType.create(workerProperties);
      const payload = binary.toPayload(message)!;
      const decoded = binary.fromPayload<typeof properties>(payload);
      expect(Long.fromValue(decoded.value!).toString()).toBe(value.toString());

      // Protobuf messages serialize their Long fields as decimal strings.
      const outputEnvelope = defaultPayloadConverter.toPayload(decoded)!;
      const workflowProperties =
        defaultPayloadConverter.fromPayload<typeof properties>(outputEnvelope);
      expect(workflowProperties.value).toBe(value.toString());
      const restored = transfer.fromTransferType(workflowProperties);
      expect(Long.isLong(restored.value)).toBe(true);
      expect(restored.value?.toString()).toBe(value.toString());
      expect(restored.value?.unsigned).toBe(value.unsigned);
      expect(transfer.fromTransferType(decoded).value?.toString()).toBe(
        value.toString(),
      );
    }
  });
});
