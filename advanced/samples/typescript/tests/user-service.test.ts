import { fileURLToPath } from "node:url";
import { describe, expect, test } from "vitest";
import * as nexus from "nexus-rpc";

import { User } from "../wit/user-service/resources.ts";
import { userService } from "../wit/user-service/services.ts";
import { executeWorkflowWithNexus, withWorkflowEnvironment } from "./helpers.ts";

const workflowsPath = fileURLToPath(
  new URL("./workflows/user-service.ts", import.meta.url),
);

describe("user-service generated output", () => {
  test("exposes basic WIT-native service metadata", () => {
    expect(userService.name).toBe("UserService");
    expect(userService.operations.getUser.name).toBe("GetUser");
    expect(userService.operations.updateEmail.name).toBe("UpdateEmail");
  });

  test("passes WIT records through a real Nexus client", async () => {
    await withWorkflowEnvironment(async (env) => {
      const calls: Array<[string, unknown]> = [];
      const handler = nexus.serviceHandler(userService, {
        async getUser(_ctx, input) {
          calls.push(["GetUser", input]);
          return new User(input.userId, "old@example.com");
        },
        async updateEmail(_ctx, input) {
          calls.push(["UpdateEmail", input]);
          return new User(input.userId, input.email);
        },
        async notify() {},
      });

      const result = await executeWorkflowWithNexus<{
        initialEmail: string;
        updatedEmail: string;
        userId: string;
      }>(env, {
        endpoint: "user-service",
        nexusServices: [handler],
        workflowType: "userServiceCaller",
        workflowsPath,
      });

      expect(result).toEqual({
        initialEmail: "old@example.com",
        updatedEmail: "new@example.com",
        userId: "user-123",
      });
      expect(calls).toEqual([
        [
          "GetUser",
          {
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
      ]);
    });
  });
});
