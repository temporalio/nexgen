import { readFileSync } from "node:fs";
import ts from "typescript";
import { describe, expect, it } from "vitest";

import * as notificationModels from "../wit/notification-service/models.ts";
import * as startOperation from "../wit/start-workflow/operations/start-workflow.ts";

function requiredFieldDeclarations(relativePath: string): ts.FunctionDeclaration[] {
  const path = new URL(relativePath, import.meta.url);
  const source = ts.createSourceFile(
    path.pathname,
    readFileSync(path, "utf8"),
    ts.ScriptTarget.Latest,
    true,
  );
  return source.statements.filter(
    (statement): statement is ts.FunctionDeclaration =>
      ts.isFunctionDeclaration(statement) && statement.name?.text === "requiredField",
  );
}

describe("generated private validation helpers", () => {
  it.each([
    "../wit/notification-service/models.ts",
    "../wit/proto-generic/models.ts",
    "../wit/proto-oneof/models.ts",
    "../wit/start-workflow/models.ts",
    "../wit/proto-backed-showcase/models.ts",
    "../wit/workflow-service/models.ts",
    "../wit/start-workflow/operations/start-workflow.ts",
    "../wit/start-workflow/operations/restart-workflow.ts",
  ])("declares a local helper in %s", (path) => {
    const declarations = requiredFieldDeclarations(path);
    expect(declarations).toHaveLength(1);
    const modifiers = ts.getModifiers(declarations[0]!) ?? [];
    expect(
      modifiers.some((modifier) => modifier.kind === ts.SyntaxKind.ExportKeyword),
    ).toBe(false);
  });

  it.each([
    "../wit/type-showcase/models.ts",
    "../wit/function-execution/models.ts",
    "../json_schema/api/showcase/models.ts",
  ])("omits the helper when unused in %s", (path) => {
    expect(requiredFieldDeclarations(path)).toHaveLength(0);
  });

  it("keeps the helper out of module exports", () => {
    expect(notificationModels).not.toHaveProperty("requiredField");
    expect(startOperation).not.toHaveProperty("requiredField");
  });
});

if (false) {
  // @ts-expect-error Validation helpers are not part of the model module API.
  notificationModels.requiredField;
  // @ts-expect-error Operation modules keep their validation helpers private too.
  startOperation.requiredField;
}
