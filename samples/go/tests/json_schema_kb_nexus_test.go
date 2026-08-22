package tests

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"io"
	"testing"

	"github.com/nexus-rpc/sdk-go/nexus"
	"github.com/stretchr/testify/require"
	"github.com/stretchr/testify/suite"
	"go.temporal.io/sdk/testsuite"
	"go.temporal.io/sdk/workflow"

	"samples/go/kb"
)

// KBNexusIntegrationSuite drives the generated KnowledgeBaseService *service and
// operation definitions* end-to-end through Temporal's in-memory Nexus test
// environment. The caller workflow uses the Temporal SDK's built-in Nexus
// client directly — there is no generated API client — and passes the generated
// operation references (kb.KnowledgeBaseService.GetPage, ...) so the SDK
// type-checks each request/response against the generated definition.
type KBNexusIntegrationSuite struct {
	suite.Suite
	testsuite.WorkflowTestSuite
	env   *testsuite.TestWorkflowEnvironment
	calls []string
}

func (s *KBNexusIntegrationSuite) loadModel(name string, out any) {
	s.Require().NoError(json.Unmarshal(jsonSchemaFixtureBytes(s.T(), "kb", name), out))
}

func (s *KBNexusIntegrationSuite) SetupTest() {
	s.env = s.NewTestWorkflowEnvironment()
	s.calls = nil

	getPage := nexus.NewSyncOperation(kb.KnowledgeBaseService.GetPage.Name(),
		func(_ context.Context, input kb.GetPageInput, _ nexus.StartOperationOptions) (kb.Page, error) {
			s.calls = append(s.calls, "GetPage")
			s.Equal("page-1", input.PageId)
			var page kb.Page
			s.loadModel("page.json", &page)
			return page, nil
		})

	putBlock := nexus.NewSyncOperation(kb.KnowledgeBaseService.PutBlock.Name(),
		func(_ context.Context, input kb.Block, _ nexus.StartOperationOptions) (kb.PutBlockOutput, error) {
			s.calls = append(s.calls, "PutBlock")
			s.Equal("block-1", input.BlockId)
			s.Require().NotNil(input.Style)
			s.Require().NotNil(input.Style.Bold)
			s.True(*input.Style.Bold)
			var out kb.PutBlockOutput
			s.loadModel("put-block-output.json", &out)
			return out, nil
		})

	getCategoryTree := nexus.NewSyncOperation(kb.KnowledgeBaseService.GetCategoryTree.Name(),
		func(_ context.Context, input kb.GetCategoryTreeInput, _ nexus.StartOperationOptions) (kb.Category, error) {
			s.calls = append(s.calls, "GetCategoryTree")
			s.Equal("root", input.RootId)
			var category kb.Category
			s.loadModel("category-tree.json", &category)
			return category, nil
		})

	service := nexus.NewService(kb.KnowledgeBaseService.ServiceName)
	s.Require().NoError(service.Register(getPage, putBlock, getCategoryTree))
	s.env.RegisterNexusService(service)
}

func TestKBNexusIntegrationSuite(t *testing.T) {
	suite.Run(t, &KBNexusIntegrationSuite{})
}

func TestKBNexusInvalidGeneratedInputIsBadRequest(t *testing.T) {
	called := false
	operation := nexus.NewSyncOperation(kb.KnowledgeBaseService.GetPage.Name(),
		func(_ context.Context, _ kb.GetPageInput, _ nexus.StartOperationOptions) (kb.Page, error) {
			called = true
			return kb.Page{}, nil
		})
	service := nexus.NewService(kb.KnowledgeBaseService.ServiceName)
	require.NoError(t, service.Register(operation))
	registry := nexus.NewServiceRegistry()
	require.NoError(t, registry.Register(service))
	handler, err := registry.NewHandler()
	require.NoError(t, err)

	input := nexus.NewLazyValue(nexus.DefaultSerializer(), &nexus.Reader{
		ReadCloser: io.NopCloser(bytes.NewReader([]byte(`{"pageId":null}`))),
		Header:     nexus.Header{"type": "application/json"},
	})
	ctx := nexus.WithHandlerContext(context.Background(), nexus.HandlerInfo{
		Service:   kb.KnowledgeBaseService.ServiceName,
		Operation: kb.KnowledgeBaseService.GetPage.Name(),
	})
	_, err = handler.StartOperation(
		ctx,
		kb.KnowledgeBaseService.ServiceName,
		kb.KnowledgeBaseService.GetPage.Name(),
		input,
		nexus.StartOperationOptions{},
	)
	require.Error(t, err)
	var handlerError *nexus.HandlerError
	require.ErrorAs(t, err, &handlerError)
	require.Equal(t, nexus.HandlerErrorTypeBadRequest, handlerError.Type)
	require.False(t, called, "operation handler must not run for invalid generated input")
}

type kbResult struct {
	BlockId         string
	CategoryChildId string
	PageId          string
	Revision        int64
}

func (s *KBNexusIntegrationSuite) TestOperationsUseRealNexusClient() {
	s.env.ExecuteWorkflow(func(ctx workflow.Context) (kbResult, error) {
		client := workflow.NewNexusClient("knowledge-base", kb.KnowledgeBaseService.ServiceName)

		var page kb.Page
		if err := client.ExecuteOperation(
			ctx, kb.KnowledgeBaseService.GetPage, kb.GetPageInput{PageId: "page-1"},
			workflow.NexusOperationOptions{},
		).Get(ctx, &page); err != nil {
			return kbResult{}, err
		}
		if len(page.Blocks) == 0 {
			return kbResult{}, errors.New("expected page block")
		}

		var putBlockOutput kb.PutBlockOutput
		if err := client.ExecuteOperation(
			ctx, kb.KnowledgeBaseService.PutBlock, page.Blocks[0],
			workflow.NexusOperationOptions{},
		).Get(ctx, &putBlockOutput); err != nil {
			return kbResult{}, err
		}

		var category kb.Category
		if err := client.ExecuteOperation(
			ctx, kb.KnowledgeBaseService.GetCategoryTree, kb.GetCategoryTreeInput{RootId: "root"},
			workflow.NexusOperationOptions{},
		).Get(ctx, &category); err != nil {
			return kbResult{}, err
		}

		var categoryChildID string
		if len(category.Children) > 0 {
			categoryChildID = category.Children[0].Id
		}

		return kbResult{
			BlockId:         putBlockOutput.BlockId,
			CategoryChildId: categoryChildID,
			PageId:          page.PageId,
			Revision:        putBlockOutput.Revision,
		}, nil
	})

	s.True(s.env.IsWorkflowCompleted())
	s.NoError(s.env.GetWorkflowError())

	var result kbResult
	s.NoError(s.env.GetWorkflowResult(&result))
	s.Equal(kbResult{
		BlockId:         "block-1",
		CategoryChildId: "child",
		PageId:          "page-1",
		Revision:        7,
	}, result)
	s.Equal([]string{"GetPage", "PutBlock", "GetCategoryTree"}, s.calls)
}
