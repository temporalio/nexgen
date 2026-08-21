package tests

import (
	"testing"

	kb "samples/go/kb"

	"github.com/stretchr/testify/require"
	"go.temporal.io/sdk/converter"
)

// TestJSONSchemaKBRuntime round-trips every KB wire fixture through the Temporal
// default data converter and asserts JSON-equality against the canonical
// fixtures, mirroring the Python and Java suites.
//
// Exception (see json-schema/nullability.md): optional+nullable fields collapse
// in Go — an explicit wire `null` on such a field round-trips to absent. The
// fixtures carrying such nulls (page.json and block.json both hold `page: null`,
// which is optional+nullable) are therefore verified by deserialization + field
// checks rather than exact JSON-equality, matching the Java test.
func TestJSONSchemaKBRuntime(t *testing.T) {
	dc := converter.GetDefaultDataConverter()

	// page.json carries a nested block with page: null (optional+nullable),
	// which Go collapses on serialize, so verify by deserialization only.
	page := decodeFixture[kb.Page](t, dc, "kb", "page.json")
	require.Equal(t, "page-1", page.PageID)
	require.Equal(t, "nexgen", page.Meta.Author)
	require.Len(t, page.Blocks, 1)
	require.Equal(t, "block-1", page.Blocks[0].BlockID)
	require.Nil(t, page.Blocks[0].Page)
	require.NotNil(t, page.Blocks[0].Style)
	require.NotNil(t, page.Blocks[0].Style.Bold)
	require.True(t, *page.Blocks[0].Style.Bold)

	// block.json carries page: null (optional+nullable) — deserialization only.
	block := decodeFixture[kb.Block](t, dc, "kb", "block.json")
	require.Equal(t, "block-1", block.BlockID)
	require.Equal(t, int64(0), block.Order)
	require.Nil(t, block.Page)
	require.NotNil(t, block.Style)
	require.NotNil(t, block.Style.Bold)
	require.True(t, *block.Style.Bold)

	category := roundTripJSONEq[kb.Category](t, dc, "kb", "category-tree.json")
	require.Equal(t, "root", category.ID)
	require.Len(t, category.Children, 1)
	require.Equal(t, "child", category.Children[0].ID)

	getPage := roundTripJSONEq[kb.GetPageInput](t, dc, "kb", "get-page-input.json")
	require.Equal(t, "page-1", getPage.PageID)

	getTree := roundTripJSONEq[kb.GetCategoryTreeInput](t, dc, "kb", "get-category-tree-input.json")
	require.Equal(t, "root", getTree.RootID)

	putBlock := roundTripJSONEq[kb.PutBlockOutput](t, dc, "kb", "put-block-output.json")
	require.Equal(t, "block-1", putBlock.BlockID)
	require.Equal(t, int64(7), putBlock.Revision)
}
