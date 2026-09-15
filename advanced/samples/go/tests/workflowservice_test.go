package tests

import (
	"context"
	"errors"
	"testing"
	"time"

	"github.com/nexus-rpc/sdk-go/nexus"
	"github.com/stretchr/testify/suite"
	common "go.temporal.io/api/common/v1"
	enums "go.temporal.io/api/enums/v1"
	workflowservicepb "go.temporal.io/api/workflowservice/v1"
	"go.temporal.io/sdk/converter"
	"go.temporal.io/sdk/temporal"
	"go.temporal.io/sdk/testsuite"
	"go.temporal.io/sdk/workflow"

	ws "go.temporal.io/sdk/advanced/samples/go/workflowservice"
)

const workflowServiceName = "temporal.api.workflowservice.v1.WorkflowService"

func signalWithStartWorkflow(ctx workflow.Context, input string) string {
	return input
}

type emptyPayloadsDataConverter struct {
	converter.DataConverter
}

func (c emptyPayloadsDataConverter) ToPayloads(values ...interface{}) (*common.Payloads, error) {
	if len(values) == 0 {
		return &common.Payloads{}, nil
	}
	return c.DataConverter.ToPayloads(values...)
}

type WorkflowServiceIntegrationSuite struct {
	suite.Suite
	testsuite.WorkflowTestSuite
	env   *testsuite.TestWorkflowEnvironment
	calls []*workflowservicepb.SignalWithStartWorkflowExecutionRequest
}

func (s *WorkflowServiceIntegrationSuite) SetupTest() {
	s.env = s.NewTestWorkflowEnvironment()
	s.calls = nil

	signalWithStart := nexus.NewSyncOperation("SignalWithStartWorkflowExecution",
		func(ctx context.Context, input *workflowservicepb.SignalWithStartWorkflowExecutionRequest, opts nexus.StartOperationOptions) (*workflowservicepb.SignalWithStartWorkflowExecutionResponse, error) {
			s.calls = append(s.calls, input)
			return &workflowservicepb.SignalWithStartWorkflowExecutionResponse{}, nil
		})

	service := nexus.NewService(workflowServiceName)
	s.NoError(service.Register(signalWithStart))
	s.env.RegisterNexusService(service)
}

func TestWorkflowServiceIntegrationSuite(t *testing.T) {
	suite.Run(t, &WorkflowServiceIntegrationSuite{})
}

func (s *WorkflowServiceIntegrationSuite) TestSignalWithStartWorkflowCallForms() {
	retryPolicy := &temporal.RetryPolicy{MaximumAttempts: 3}
	searchKey := temporal.NewSearchAttributeKeyKeyword("CustomKeyword")
	searchAttributes := temporal.NewSearchAttributes(searchKey.ValueSet("search-value"))

	s.env.ExecuteWorkflow(func(ctx workflow.Context) error {
		priority := temporal.Priority{PriorityKey: 7}
		opts := ws.SignalWithStartWorkflowOptions{
			ID:                       "workflow-id",
			TaskQueue:                "my-task-queue",
			WorkflowExecutionTimeout: 3 * time.Hour,
			WorkflowRunTimeout:       2 * time.Hour,
			WorkflowTaskTimeout:      time.Minute,
			WorkflowIDReusePolicy:    enums.WORKFLOW_ID_REUSE_POLICY_REJECT_DUPLICATE,
			RetryPolicy:              retryPolicy,
			CronSchedule:             "0 * * * *",
			Memo:                     map[string]any{"memo-key": "memo-value"},
			TypedSearchAttributes:    searchAttributes,
			Priority:                 &priority,
		}
		var typedResult ws.SignalWithStartWorkflowResponse
		typedFuture := ws.SignalWithStartWorkflowTyped(
			ctx,
			opts,
			"wake-up",
			"signal-value",
			signalWithStartWorkflow,
			"workflow-input",
		)
		selector := workflow.NewSelector(ctx)
		selected := false
		var typedErr error
		selector.AddFuture(typedFuture, func(ready workflow.Future) {
			selected = true
			typedErr = ready.Get(ctx, &typedResult)
		}).Select(ctx)
		if !selected {
			return errors.New("selector did not select the transformed future")
		}
		if typedErr != nil {
			return typedErr
		}

		var variadicResult ws.SignalWithStartWorkflowResponse
		return ws.SignalWithStartWorkflow(
			ctx,
			opts,
			"wake-up",
			nil,
			"ExampleWorkflow",
			"one",
			"two",
		).Get(ctx, &variadicResult)
	})

	s.True(s.env.IsWorkflowCompleted())
	s.NoError(s.env.GetWorkflowError())
	s.Require().Len(s.calls, 2)

	typedRequest := s.calls[0]
	s.Equal("wake-up", typedRequest.GetSignalName())
	s.Require().NotNil(typedRequest.GetSignalInput())
	s.Len(typedRequest.GetSignalInput().GetPayloads(), 1)
	s.Require().NotNil(typedRequest.GetInput())
	s.Len(typedRequest.GetInput().GetPayloads(), 1)
	s.Equal("default-test-namespace", typedRequest.GetNamespace())
	s.Equal("workflow-id", typedRequest.GetWorkflowId())
	s.Equal("my-task-queue", typedRequest.GetTaskQueue().GetName())
	s.Equal(3*time.Hour, typedRequest.GetWorkflowExecutionTimeout().AsDuration())
	s.Equal(2*time.Hour, typedRequest.GetWorkflowRunTimeout().AsDuration())
	s.Equal(time.Minute, typedRequest.GetWorkflowTaskTimeout().AsDuration())
	s.Equal(enums.WORKFLOW_ID_REUSE_POLICY_REJECT_DUPLICATE, typedRequest.GetWorkflowIdReusePolicy())
	s.Equal(int32(3), typedRequest.GetRetryPolicy().GetMaximumAttempts())
	s.Equal("0 * * * *", typedRequest.GetCronSchedule())
	s.Contains(typedRequest.GetMemo().GetFields(), "memo-key")
	s.Contains(typedRequest.GetSearchAttributes().GetIndexedFields(), "CustomKeyword")
	s.Equal(int32(7), typedRequest.GetPriority().GetPriorityKey())

	variadicRequest := s.calls[1]
	// A nil signal argument is still one argument and therefore one payload;
	// it does not mean that the signal has no arguments.
	s.Require().NotNil(variadicRequest.GetSignalInput())
	s.Len(variadicRequest.GetSignalInput().GetPayloads(), 1)
	s.Require().NotNil(variadicRequest.GetInput())
	s.Len(variadicRequest.GetInput().GetPayloads(), 2)
}

func (s *WorkflowServiceIntegrationSuite) TestEmptyPayloadsAreDelegatedToDataConverter() {
	s.env.SetDataConverter(emptyPayloadsDataConverter{converter.GetDefaultDataConverter()})
	s.env.ExecuteWorkflow(func(ctx workflow.Context) (*ws.SignalWithStartWorkflowResponse, error) {
		var result ws.SignalWithStartWorkflowResponse
		return &result, ws.SignalWithStartWorkflow(
			ctx,
			ws.SignalWithStartWorkflowOptions{ID: "workflow-id"},
			"wake-up",
			"signal-value",
			"ExampleWorkflow",
		).Get(ctx, &result)
	})

	s.True(s.env.IsWorkflowCompleted())
	s.NoError(s.env.GetWorkflowError())
	s.Require().Len(s.calls, 1)
	s.NotNil(s.calls[0].Input)
	s.Empty(s.calls[0].Input.Payloads)
}

func (s *WorkflowServiceIntegrationSuite) TestCanceledContextDoesNotScheduleOperation() {
	s.env.ExecuteWorkflow(func(ctx workflow.Context) error {
		ctx, cancel := workflow.WithCancel(ctx)
		cancel()
		return ws.SignalWithStartWorkflow(ctx, ws.SignalWithStartWorkflowOptions{ID: "workflow-id"}, "wake-up", "signal-value", signalWithStartWorkflow, "workflow-input").Get(ctx, nil)
	})

	s.Error(s.env.GetWorkflowError())
	s.Empty(s.calls)
}

func (s *WorkflowServiceIntegrationSuite) TestConversionFailureReturnsReadyFuture() {
	s.env.ExecuteWorkflow(func(ctx workflow.Context) error {
		fut := ws.SignalWithStartWorkflow(ctx, ws.SignalWithStartWorkflowOptions{ID: "workflow-id", Memo: map[string]any{"invalid": func() {}}}, "wake-up", "signal-value", signalWithStartWorkflow, "workflow-input")
		if !fut.IsReady() {
			return errors.New("conversion failure future is not ready")
		}
		if err := fut.Get(ctx, nil); err == nil {
			return errors.New("conversion failure future returned no error")
		}
		return nil
	})

	s.NoError(s.env.GetWorkflowError())
	s.Empty(s.calls)
}
