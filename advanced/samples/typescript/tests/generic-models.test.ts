import { describe, expect, it } from "vitest";

import type {
  GenericRequest,
  GenericResponse,
  OperationCompletionResult,
} from "../wit/generic-models/models";

describe("generic models generated output", () => {
  it("preserves correlated model parameters", () => {
    const request: GenericRequest<string> = {
      context: "context",
      contexts: ["first"],
      byName: { primary: "value" },
      nested: { value: "nested" },
    };
    const completion: OperationCompletionResult<number> = {
      tag: "success",
      value: 42,
    };
    const response: GenericResponse<string, number, boolean> = {
      context: request.context,
      completion,
      metadata: true,
    };

    expect(response.context).toBe("context");
    expect(response.completion).toEqual({ tag: "success", value: 42 });
  });
});
