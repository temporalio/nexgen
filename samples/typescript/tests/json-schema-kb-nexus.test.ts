import { fileURLToPath } from "node:url";
import { describe, expect, test } from "vitest";
import * as nexus from "nexus-rpc";

import { chatService } from "../chat/services.ts";
import { knowledgeBaseService } from "../kb/kb/services.ts";
import { blockTransferTypeConverter } from "../kb/content/block/models.ts";
import type { Block } from "../kb/content/block/models.ts";
import { categoryTransferTypeConverter } from "../kb/tree/category/models.ts";
import type { Category } from "../kb/tree/category/models.ts";
import { pageTransferTypeConverter } from "../kb/content/page/models.ts";
import type { Page } from "../kb/content/page/models.ts";
import {
  getCategoryTreeInputTransferTypeConverter,
  getPageInputTransferTypeConverter,
  putBlockOutputTransferTypeConverter,
} from "../kb/kb/models.ts";
import type { PutBlockOutput } from "../kb/kb/models.ts";
import { loadFixture as loadFixtureFrom } from "./json-converter-helper.ts";
import { executeWorkflowWithNexus, withWorkflowEnvironment } from "./helpers.ts";

const workflowsPath = fileURLToPath(
  new URL("./workflows/json-schema-kb.ts", import.meta.url),
);
const wireFixtureDir = new URL("../../wire/json_schema/kb/", import.meta.url);

function loadFixture<T = unknown>(name: string): T {
  return loadFixtureFrom<T>(wireFixtureDir, name);
}

describe("json-schema KB generated Nexus service", () => {
  test("exposes generated service + operation definitions", () => {
    expect(knowledgeBaseService.name).toBe("example.kb.v1.KnowledgeBaseService");
    expect(knowledgeBaseService.operations.getPage.name).toBe("GetPage");
    expect(knowledgeBaseService.operations.putBlock.name).toBe("PutBlock");
    expect(knowledgeBaseService.operations.getCategoryTree.name).toBe(
      "GetCategoryTree",
    );
  });

  // Every operation carries its models' transfer type converters as operation
  // type info, so a protocol integration can apply the conversion without the
  // caller naming the converter. `chatService.operations.ping` (void on both
  // sides) is the counter-case: there is no value to convert, so it carries none.
  test("carries transfer type info on every operation", () => {
    expect(
      knowledgeBaseService.operations.getPage.inputType?.transferTypeConverter,
    ).toBe(getPageInputTransferTypeConverter);
    expect(
      knowledgeBaseService.operations.getPage.outputType?.transferTypeConverter,
    ).toBe(pageTransferTypeConverter);
    expect(
      knowledgeBaseService.operations.putBlock.inputType?.transferTypeConverter,
    ).toBe(blockTransferTypeConverter);
    expect(
      knowledgeBaseService.operations.putBlock.outputType?.transferTypeConverter,
    ).toBe(putBlockOutputTransferTypeConverter);
    expect(
      knowledgeBaseService.operations.getCategoryTree.inputType?.transferTypeConverter,
    ).toBe(getCategoryTreeInputTransferTypeConverter);
    expect(
      knowledgeBaseService.operations.getCategoryTree.outputType?.transferTypeConverter,
    ).toBe(categoryTransferTypeConverter);

    expect(chatService.operations.ping.inputType).toBeUndefined();
    expect(chatService.operations.ping.outputType).toBeUndefined();
  });

  // Register a handler bound to the generated service definition, then run a
  // workflow that calls every operation through the SDK's Nexus client. The SDK
  // applies the generated operation type info at both ends of the Nexus boundary,
  // so application code receives and returns typed models without naming their
  // transfer type converters.
  test("drives every operation through a real Nexus client", async () => {
    await withWorkflowEnvironment(async (env) => {
      const calls: Array<[string, unknown]> = [];
      const handler = nexus.serviceHandler(knowledgeBaseService, {
        async getPage(_ctx, input) {
          calls.push(["GetPage", input]);
          return loadFixture<Page>("page.json");
        },
        async putBlock(_ctx, input) {
          calls.push(["PutBlock", input]);
          return loadFixture<PutBlockOutput>("put-block-output.json");
        },
        async getCategoryTree(_ctx, input) {
          calls.push(["GetCategoryTree", input]);
          return loadFixture<Category>("category-tree.json");
        },
      });

      const result = await executeWorkflowWithNexus<{
        blockId: string;
        categoryChildId: string | undefined;
        pageId: string;
        revision: number;
      }>(env, {
        endpoint: "knowledge-base",
        nexusServices: [handler],
        workflowType: "jsonSchemaKbCaller",
        workflowsPath,
      });

      expect(result).toEqual({
        blockId: "block-1",
        categoryChildId: "child",
        pageId: "page-1",
        revision: 7,
      });

      expect(calls.map(([operation]) => operation)).toEqual([
        "GetPage",
        "PutBlock",
        "GetCategoryTree",
      ]);
      expect((calls[0]?.[1] as { pageId: string }).pageId).toBe("page-1");
      const putBlockInput = calls[1]?.[1] as Block;
      expect(putBlockInput.blockId).toBe("block-1");
      expect(putBlockInput.style?.bold).toBe(true);
      expect((calls[2]?.[1] as { rootId: string }).rootId).toBe("root");
    });
  });
});
