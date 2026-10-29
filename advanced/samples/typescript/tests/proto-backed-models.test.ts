import { describe, expect, expectTypeOf, it } from "vitest";
import { temporal } from "@temporalio/proto";
import {
  AlertLevel,
  NamespaceState,
  alertTransferTypeConverter as alerts,
  namespaceInfoTransferTypeConverter as namespaces,
  callbackInternalTransferTypeConverter as callbacks,
} from "../wit/proto-backed-showcase/models.ts";

describe("authored protobuf enum and byte models", () => {
  it("preserves enum members and their wire numbers", () => {
    for (const level of [
      AlertLevel.SeverityUnspecified,
      AlertLevel.SeverityHigh,
      AlertLevel.SeverityMedium,
      AlertLevel.SeverityLow,
    ]) {
      const model = { message: "alert", severity: level };
      const wire = alerts.toTransferType(model);
      expectTypeOf(wire.severity).toEqualTypeOf<
        temporal.api.enums.v1.Severity | null | undefined
      >();
      expect(wire.severity).toBe(level);
      const decoded = alerts.fromTransferType(
        temporal.api.version.v1.Alert.decode(
          temporal.api.version.v1.Alert.encode(wire).finish(),
        ),
      );
      expectTypeOf(decoded.severity).toEqualTypeOf<AlertLevel | undefined>();
      expect(decoded).toEqual(model);
    }
  });

  it("does not apply operation defaults during conversion", () => {
    expect(
      namespaces.toTransferType({ state: NamespaceState.NamespaceStateUnspecified })
        .state,
    ).toBe(temporal.api.enums.v1.NamespaceState.NAMESPACE_STATE_UNSPECIFIED);
    expect(namespaces.fromTransferType({ state: 0 }).state).toBe(
      NamespaceState.NamespaceStateUnspecified,
    );
    expect(() => namespaces.fromTransferType({})).toThrow(
      "missing required field NamespaceInfo.state",
    );
  });

  it("keeps the non-defaulted enum optional", () => {
    const model = alerts.fromTransferType({ message: "optional" });
    expect(model.severity).toBeUndefined();
    expect(alerts.toTransferType(model).severity).toBeUndefined();
  });

  it("round-trips byte lists including zero and non-ASCII bytes", () => {
    const data = new Uint8Array([0, 1, 127, 128, 255]);
    const wire = callbacks.toTransferType({ data });
    const decoded = callbacks.fromTransferType(
      temporal.api.common.v1.Callback.Internal.decode(
        temporal.api.common.v1.Callback.Internal.encode(wire).finish(),
      ),
    );
    expectTypeOf(decoded.data).toEqualTypeOf<Uint8Array>();
    expect(decoded.data).toBeInstanceOf(Uint8Array);
    expect(Array.from(decoded.data)).toEqual(Array.from(data));
    // Required protobuf scalar fields treat a default wire value as missing.
    expect(() => callbacks.fromTransferType({ data: new Uint8Array() })).toThrow(
      "missing required field CallbackInternal.data",
    );
  });
});
