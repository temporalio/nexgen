import { temporal } from "@temporalio/proto";
import { describe, expect, it } from "vitest";

import { AlertLevel, CommandType, NamespaceState } from "../wit/proto-enums/models.ts";
import {
  alertTransferTypeConverter as alerts,
  commandModelTransferTypeConverter as commands,
  namespaceInfoTransferTypeConverter as namespaces,
} from "../wit/proto-enums/index.ts";

describe("proto-backed enums", () => {
  it("preserves the normal ordered enum", () => {
    const members = [
      NamespaceState.NamespaceStateUnspecified,
      NamespaceState.NamespaceStateRegistered,
      NamespaceState.NamespaceStateDeprecated,
      NamespaceState.NamespaceStateDeleted,
    ];
    expect(members).toEqual([0, 1, 2, 3]);
    for (const state of members) {
      const wire = namespaces.toTransferType({ state });
      expect(wire.state).toBe(state);
      expect(namespaces.fromTransferType(wire).state).toBe(state);
    }
    expect(namespaces.toTransferType({}).state).toBe(
      temporal.api.enums.v1.NamespaceState.NAMESPACE_STATE_REGISTERED,
    );
  });

  it("uses protobuf numbers rather than WIT declaration order", () => {
    const members = [
      AlertLevel.SeverityLow,
      AlertLevel.SeverityHigh,
      AlertLevel.SeverityMedium,
      AlertLevel.SeverityUnspecified,
    ];
    expect(members).toEqual([3, 1, 2, 0]);
    for (const severity of members) {
      const wire = alerts.toTransferType({ message: "test", severity });
      expect(wire.severity).toBe(severity);
      expect(alerts.fromTransferType(wire).severity).toBe(severity);
    }
    expect(alerts.toTransferType({ message: "test" }).severity).toBe(
      temporal.api.enums.v1.Severity.SEVERITY_LOW,
    );
  });

  it("preserves the gap at 15, including the default after the gap", () => {
    const members = Object.values(CommandType).filter(
      (value): value is CommandType => typeof value === "number",
    );
    expect(members).toEqual([...Array.from({ length: 15 }, (_, i) => i), 16, 17, 18]);
    for (const commandType of members) {
      const wire = commands.toTransferType({ commandType });
      expect(wire.commandType).toBe(commandType);
      expect(commands.fromTransferType(wire).commandType).toBe(commandType);
    }
    expect(commands.toTransferType({}).commandType).toBe(
      temporal.api.enums.v1.CommandType.COMMAND_TYPE_MODIFY_WORKFLOW_PROPERTIES,
    );
  });
});
