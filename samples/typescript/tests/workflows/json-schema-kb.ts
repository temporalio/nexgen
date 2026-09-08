import * as workflow from "@temporalio/workflow";

import { knowledgeBaseService } from "../../kb/kb/services.ts";

// Drive the generated Nexus service *definition* through the Temporal SDK's
// built-in Nexus client — no generated API client. The SDK applies each
// operation's generated transfer type info when encoding requests and decoding
// responses, so callers work only with the typed models.
export async function jsonSchemaKbCaller(): Promise<{
  blockId: string;
  categoryChildId: string | undefined;
  pageId: string;
  revision: number;
}> {
  const client = workflow.createNexusServiceClient({
    service: knowledgeBaseService,
    endpoint: "knowledge-base",
  });

  const pageHandle = await client.startOperation(
    knowledgeBaseService.operations.getPage,
    { pageId: "page-1" },
  );
  const page = await pageHandle.result();
  const block = page.blocks?.[0];
  if (block == null) {
    throw new Error("expected page block");
  }

  const putBlockHandle = await client.startOperation(
    knowledgeBaseService.operations.putBlock,
    block,
  );
  const putBlockOutput = await putBlockHandle.result();

  const categoryHandle = await client.startOperation(
    knowledgeBaseService.operations.getCategoryTree,
    { rootId: "root" },
  );
  const category = await categoryHandle.result();

  return {
    blockId: putBlockOutput.blockId,
    categoryChildId: category.children?.[0]?.id,
    pageId: page.pageId,
    revision: putBlockOutput.revision,
  };
}
