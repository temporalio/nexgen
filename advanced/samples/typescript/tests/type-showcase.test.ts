import { describe, expect, test } from "vitest";
import { fileURLToPath } from "node:url";
import * as nexus from "nexus-rpc";

import type { RecordSyncRequest } from "../wit/type-showcase/index.ts";
import { UserCapability, UserStatus } from "../wit/type-showcase/models.ts";
import { User } from "../wit/type-showcase/resources.ts";
import { typeShowcase } from "../wit/type-showcase/services.ts";
import { executeWorkflowWithNexus, withWorkflowEnvironment } from "./helpers.ts";

const workflowsPath = fileURLToPath(
  new URL("./workflows/type-showcase.ts", import.meta.url),
);

const userProfile = () => ({
  capabilities: UserCapability.ReadProfile | UserCapability.UpdateEmail,
  notificationTarget: { tag: "email" as const, value: "old@example.com" },
  syncState: { tag: "ok" as const, value: "synced" },
  address: {
    street: "1 Main St",
    city: "Portland",
    country: "US",
    coordinates: [45.5152, -122.6784] as [number, number],
  },
  metadata: { tier: "enterprise" },
  tags: ["admin", "beta"],
});

const userResource = (email: string, displayName: string) =>
  new User("user-123", email, displayName, UserStatus.Active, userProfile());

describe("type-showcase generated output", () => {
  test("exposes WIT-native type showcase metadata", () => {
    expect(typeShowcase.name).toBe("TypeShowcase");
    expect(typeShowcase.operations.getUser.name).toBe("GetUser");
    expect(typeShowcase.operations.updateEmail.name).toBe("UpdateEmail");
    expect(typeShowcase.operations.rename.name).toBe("Rename");
    expect(typeShowcase.operations.setProfile.name).toBe("SetProfile");
    expect(typeShowcase.operations.recordSync.name).toBe("RecordSync");
    expect(typeShowcase.operations.deactivate.name).toBe("Deactivate");
    expect(UserStatus.Active).toBe(0);
    expect(UserCapability.ReadProfile).toBe(1);
    expect(UserCapability.UpdateEmail).toBe(2);
  });

  test("passes WIT records through a real Nexus client", async () => {
    await withWorkflowEnvironment(async (env) => {
      const calls: Array<[string, unknown]> = [];
      const handler = nexus.serviceHandler(typeShowcase, {
        async getUser(_ctx, input) {
          calls.push(["GetUser", input]);
          return userResource("old@example.com", "Old Name");
        },
        async updateEmail(_ctx, input) {
          calls.push(["UpdateEmail", input]);
          return userResource(input.email, "Old Name");
        },
        async rename(_ctx, input) {
          calls.push(["Rename", input]);
          return userResource("new@example.com", input.displayName);
        },
        async setProfile(_ctx, input) {
          calls.push(["SetProfile", input]);
          return userResource("new@example.com", "New Name");
        },
        async recordSync(_ctx, input) {
          calls.push(["RecordSync", input]);
        },
        async deactivate(_ctx, input) {
          calls.push(["Deactivate", input]);
        },
        async updateNotificationSettings() {},
        async getSyncSettings(_ctx, input) {
          return { region: input.region, checkpoint: input.checkpoint };
        },
      });

      const result = await executeWorkflowWithNexus<{
        deactivated: boolean;
        displayName: string;
        email: string;
        hasReadProfile: boolean;
        notificationTag: string;
        userId: string;
      }>(env, {
        endpoint: "type-showcase",
        nexusServices: [handler],
        workflowType: "typeShowcaseCaller",
        workflowsPath,
      });

      expect(result).toEqual({
        deactivated: true,
        displayName: "New Name",
        email: "new@example.com",
        hasReadProfile: true,
        notificationTag: "email",
        userId: "user-123",
      });
      expect(calls).toEqual([
        [
          "GetUser",
          {
            consistencyToken: "read-123",
            userId: "user-123",
          },
        ],
        [
          "UpdateEmail",
          {
            email: "new@example.com",
            userId: "user-123",
          },
        ],
        [
          "Rename",
          {
            displayName: "New Name",
            userId: "user-123",
          },
        ],
        [
          "Deactivate",
          {
            reason: "requested",
            userId: "user-123",
          },
        ],
        [
          "RecordSync",
          {
            report: {
              route: [
                [45.5152, -122.6784],
                [47.6062, -122.3321],
              ],
              attempts: [
                { tag: "ok", value: "synced" },
                { tag: "err", value: "timeout" },
              ],
              regionStatus: {
                "us-west": { tag: "ok", value: "healthy" },
                "eu-central": { tag: "err", value: "degraded" },
              },
              itemCount: 3,
              byteCount: 5_000_000_000,
              clockSkewMs: -5_000_000_000,
            },
            userId: "user-123",
          },
        ],
      ]);
    });
  });

  test("constructs WIT-native models for common WIT shapes", () => {
    const expected: RecordSyncRequest = {
      userId: "user-123",
      report: {
        route: [
          [45.5152, -122.6784],
          [47.6062, -122.3321],
        ],
        attempts: [
          { tag: "ok", value: "synced" },
          { tag: "err", value: "timeout" },
        ],
        regionStatus: {
          "us-west": { tag: "ok", value: "healthy" },
          "eu-central": { tag: "err", value: "degraded" },
        },
        itemCount: 3,
        byteCount: 5_000_000_000,
        clockSkewMs: -5_000_000_000,
      },
    };
    expect(expected.report.regionStatus?.["us-west"]).toEqual({
      tag: "ok",
      value: "healthy",
    });
  });
});
