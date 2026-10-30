package tests

import (
	"context"
	"testing"
	"time"

	"github.com/nexus-rpc/sdk-go/nexus"
	"github.com/stretchr/testify/suite"
	workflowservicepb "go.temporal.io/api/workflowservice/v1"
	"go.temporal.io/sdk/testsuite"
	"go.temporal.io/sdk/workflow"

	sw "go.temporal.io/sdk/advanced/samples/go/startworkflow"
)

const startWorkflowServiceName = "StartWorkflowService"

type StartWorkflowIntegrationSuite struct {
	suite.Suite
	testsuite.WorkflowTestSuite
	env   *testsuite.TestWorkflowEnvironment
	calls []nexusCall
}

func (s *StartWorkflowIntegrationSuite) SetupTest() {
	s.env = s.NewTestWorkflowEnvironment()
	s.calls = nil

	start := nexus.NewSyncOperation("StartWorkflow",
		func(ctx context.Context, input *workflowservicepb.StartWorkflowExecutionRequest, opts nexus.StartOperationOptions) (*workflowservicepb.StartWorkflowExecutionResponse, error) {
			s.calls = append(s.calls, nexusCall{"StartWorkflow", input})
			return &workflowservicepb.StartWorkflowExecutionResponse{RunId: "start-run"}, nil
		})
	restart := nexus.NewSyncOperation("RestartWorkflow",
		func(ctx context.Context, input *workflowservicepb.StartWorkflowExecutionRequest, opts nexus.StartOperationOptions) (*workflowservicepb.StartWorkflowExecutionResponse, error) {
			s.calls = append(s.calls, nexusCall{"RestartWorkflow", input})
			return &workflowservicepb.StartWorkflowExecutionResponse{RunId: "restart-run"}, nil
		})
	cancel := nexus.NewSyncOperation("CancelWorkflow",
		func(ctx context.Context, input *workflowservicepb.RequestCancelWorkflowExecutionRequest, opts nexus.StartOperationOptions) (*workflowservicepb.RequestCancelWorkflowExecutionResponse, error) {
			s.calls = append(s.calls, nexusCall{"CancelWorkflow", input})
			return &workflowservicepb.RequestCancelWorkflowExecutionResponse{}, nil
		})

	service := nexus.NewService(startWorkflowServiceName)
	s.NoError(service.Register(start, restart, cancel))
	s.env.RegisterNexusService(service)
}

func TestStartWorkflowIntegrationSuite(t *testing.T) {
	suite.Run(t, &StartWorkflowIntegrationSuite{})
}

func (s *StartWorkflowIntegrationSuite) TestPublicOperationsAndResourceMethods() {
	s.env.ExecuteWorkflow(func(ctx workflow.Context) error {
		var start sw.StartedWorkflow
		// Get resolves to a StartedWorkflow resource rather than a client.WorkflowRun.
		future := sw.StartWorkflow(
			ctx,
			sw.StartWorkflowOptions{Workflow: "typed-workflow", WorkflowID: "start-id", TaskQueue: "start-queue", WorkflowStartDelay: time.Second},
		)
		if err := future.Get(ctx, &start); err != nil {
			return err
		}
		var restart sw.StartedWorkflow
		if err := sw.RestartWorkflow(
			ctx, sw.RestartWorkflowOptions{Workflow: "typed-restart", WorkflowID: "restart-id", TaskQueue: "restart-queue"},
		).Get(ctx, &restart); err != nil {
			return err
		}
		var cancelResult sw.CancelWorkflowResponse
		if err := sw.CancelWorkflow(ctx, sw.CancelWorkflowOptions{
			WorkflowExecution: sw.WorkflowExecution{WorkflowID: "cancel-id"}, Reason: "because",
		}).Get(ctx, &cancelResult); err != nil {
			return err
		}
		var resourceRestart sw.StartedWorkflow
		if err := start.RestartWorkflow(ctx, "resource-workflow", "resource-queue").Get(ctx, &resourceRestart); err != nil {
			return err
		}
		// The resource's namespace wins over the namespace source.
		stored := sw.NewStartedWorkflow("stored-namespace", "start-id", "start-run")
		var resourceCancel sw.CancelWorkflowResponse
		return stored.Cancel(ctx, "resource-cancel").Get(ctx, &resourceCancel)
	})

	s.True(s.env.IsWorkflowCompleted())
	s.NoError(s.env.GetWorkflowError())
	s.Require().Len(s.calls, 5)

	startRequest := s.calls[0].Input.(*workflowservicepb.StartWorkflowExecutionRequest)
	s.Equal("StartWorkflow", s.calls[0].Operation)
	s.Equal("start-id", startRequest.GetWorkflowId())
	s.Equal("typed-workflow", startRequest.GetWorkflowType().GetName())
	s.Equal("start-queue", startRequest.GetTaskQueue().GetName())
	s.Equal(time.Second, startRequest.GetWorkflowStartDelay().AsDuration())
	s.Nil(startRequest.GetInput())

	s.Equal("RestartWorkflow", s.calls[1].Operation)
	restartRequest := s.calls[1].Input.(*workflowservicepb.StartWorkflowExecutionRequest)
	s.Equal("typed-restart", restartRequest.GetWorkflowType().GetName())
	s.Nil(restartRequest.GetInput())

	cancelRequest := s.calls[2].Input.(*workflowservicepb.RequestCancelWorkflowExecutionRequest)
	s.Equal("CancelWorkflow", s.calls[2].Operation)
	s.Equal("cancel-id", cancelRequest.GetWorkflowExecution().GetWorkflowId())
	s.Equal("because", cancelRequest.GetReason())

	resourceRestart := s.calls[3].Input.(*workflowservicepb.StartWorkflowExecutionRequest)
	s.Equal("RestartWorkflow", s.calls[3].Operation)
	s.Equal("start-id", resourceRestart.GetWorkflowId())
	s.Equal("resource-queue", resourceRestart.GetTaskQueue().GetName())
	s.Equal("resource-workflow", resourceRestart.GetWorkflowType().GetName())

	resourceCancel := s.calls[4].Input.(*workflowservicepb.RequestCancelWorkflowExecutionRequest)
	s.Equal("CancelWorkflow", s.calls[4].Operation)
	s.Equal("start-id", resourceCancel.GetWorkflowExecution().GetWorkflowId())
	s.Equal("start-run", resourceCancel.GetWorkflowExecution().GetRunId())
	s.Equal("resource-cancel", resourceCancel.GetReason())
	s.Equal("stored-namespace", resourceCancel.GetNamespace())
}
