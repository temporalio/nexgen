package tests

import (
	"context"
	"errors"
	"fmt"
	"reflect"
	"testing"
	"time"

	"github.com/nexus-rpc/sdk-go/nexus"
	"github.com/stretchr/testify/suite"
	activitypb "go.temporal.io/api/activity/v1"
	commandpb "go.temporal.io/api/command/v1"
	commonpb "go.temporal.io/api/common/v1"
	namespacepb "go.temporal.io/api/namespace/v1"
	taskqueuepb "go.temporal.io/api/taskqueue/v1"
	workflowpb "go.temporal.io/api/workflow/v1"
	workflowservicepb "go.temporal.io/api/workflowservice/v1"
	"go.temporal.io/sdk/converter"
	"go.temporal.io/sdk/temporal"
	"go.temporal.io/sdk/testsuite"
	"go.temporal.io/sdk/workflow"
	"google.golang.org/protobuf/proto"

	tr "go.temporal.io/sdk/advanced/samples/go/typeroundtrip"
)

const typeRoundtripServiceName = "TypeRoundtripService"

type TypeRoundtripIntegrationSuite struct {
	suite.Suite
	testsuite.WorkflowTestSuite
	env   *testsuite.TestWorkflowEnvironment
	calls []nexusCall
}

func (s *TypeRoundtripIntegrationSuite) SetupTest() {
	s.env = s.NewTestWorkflowEnvironment()
	s.calls = nil

	activityOptionsOp := nexus.NewSyncOperation("ActivityOptionsOperation",
		func(ctx context.Context, input *activitypb.ActivityOptions, opts nexus.StartOperationOptions) (*activitypb.ActivityOptions, error) {
			s.calls = append(s.calls, nexusCall{"ActivityOptionsOperation", input})
			return input, nil
		})
	failureOp := nexus.NewSyncOperation("FailureOperation",
		func(ctx context.Context, input *commandpb.FailWorkflowExecutionCommandAttributes, opts nexus.StartOperationOptions) (*commandpb.FailWorkflowExecutionCommandAttributes, error) {
			if failure := input.GetFailure(); failure != nil {
				encodedAttributes, err := converter.GetDefaultDataConverter().ToPayload(map[string]any{
					"message":     "decoded outer failure",
					"stack_trace": "decoded stack",
				})
				if err != nil {
					return nil, err
				}
				failure.Message = "Encoded failure"
				failure.StackTrace = ""
				failure.EncodedAttributes = encodedAttributes
			}
			s.calls = append(s.calls, nexusCall{"FailureOperation", input})
			return input, nil
		})

	service := nexus.NewService(typeRoundtripServiceName)
	s.NoError(service.Register(activityOptionsOp))
	s.NoError(service.Register(failureOp))
	s.NoError(service.Register(nexus.NewSyncOperation("WorkflowExecutionInfoOperation",
		func(ctx context.Context, input *workflowpb.WorkflowExecutionInfo, opts nexus.StartOperationOptions) (*workflowpb.WorkflowExecutionInfo, error) {
			s.calls = append(s.calls, nexusCall{"WorkflowExecutionInfoOperation", input})
			return input, nil
		})))
	s.NoError(service.Register(nexus.NewSyncOperation("TaskQueuePartitionsOperation",
		func(ctx context.Context, input *workflowservicepb.ListTaskQueuePartitionsResponse, opts nexus.StartOperationOptions) (*workflowservicepb.ListTaskQueuePartitionsResponse, error) {
			s.calls = append(s.calls, nexusCall{"TaskQueuePartitionsOperation", input})
			return input, nil
		})))
	s.NoError(service.Register(nexus.NewSyncOperation("BadBinariesOperation",
		func(ctx context.Context, input *namespacepb.BadBinaries, opts nexus.StartOperationOptions) (*namespacepb.BadBinaries, error) {
			s.calls = append(s.calls, nexusCall{"BadBinariesOperation", input})
			return input, nil
		})))
	s.env.RegisterNexusService(service)
}

func TestTypeRoundtripIntegrationSuite(t *testing.T) {
	suite.Run(t, new(TypeRoundtripIntegrationSuite))
}

type typeRoundtripResults struct {
	ActivityOption      tr.ActivityOptions
	FailureCauseMessage string
	FailureCauseType    string
	FailureDetail       string
	FailureMessage      string
	FailureNonRetryable bool
	FailureType         string
}

func (s *TypeRoundtripIntegrationSuite) TestProtoRoundTrips() {
	policy := temporal.RetryPolicy{MaximumAttempts: 3}
	priority := temporal.Priority{
		PriorityKey:    4,
		FairnessKey:    "tenant-a",
		FairnessWeight: 2.5,
	}

	s.env.ExecuteWorkflow(func(ctx workflow.Context) (*typeRoundtripResults, error) {
		var results typeRoundtripResults
		// Public Temporal SDK types are converted to protobuf at the Nexus
		// boundary, then converted back when the future resolves.
		if err := tr.ActivityOptionsOperation(ctx, tr.ActivityOptionsOperationOptions{
			RetryPolicy:            policy,
			TaskQueue:              "demo-task-queue",
			ScheduleToCloseTimeout: 7 * time.Second,
			Priority:               &priority,
		}).Get(ctx, &results.ActivityOption); err != nil {
			return nil, err
		}
		failure := temporal.NewApplicationErrorWithOptions(
			"outer failure",
			"OuterFailure",
			temporal.ApplicationErrorOptions{
				NonRetryable: true,
				Details:      []interface{}{"detail"},
				Cause: temporal.NewApplicationError(
					"inner failure",
					"InnerFailure",
				),
			},
		)
		var failureResult tr.FailureContainer
		if err := tr.FailureOperation(ctx, tr.FailureOperationOptions{Failure: failure}).Get(ctx, &failureResult); err != nil {
			return nil, err
		}
		var applicationFailure *temporal.ApplicationError
		if !errors.As(failureResult.Failure, &applicationFailure) {
			return nil, fmt.Errorf("expected application failure, got %T", failureResult.Failure)
		}
		results.FailureMessage = applicationFailure.Message()
		results.FailureType = applicationFailure.Type()
		results.FailureNonRetryable = applicationFailure.NonRetryable()
		if err := applicationFailure.Details(&results.FailureDetail); err != nil {
			return nil, err
		}
		var cause *temporal.ApplicationError
		if !errors.As(errors.Unwrap(applicationFailure), &cause) {
			return nil, fmt.Errorf("expected application failure cause, got %T", errors.Unwrap(applicationFailure))
		}
		results.FailureCauseMessage = cause.Message()
		results.FailureCauseType = cause.Type()
		return &results, nil
	})

	s.True(s.env.IsWorkflowCompleted())
	s.NoError(s.env.GetWorkflowError())
	var results typeRoundtripResults
	s.NoError(s.env.GetWorkflowResult(&results))
	s.Equal(int32(3), results.ActivityOption.RetryPolicy.MaximumAttempts)
	s.Require().NotNil(results.ActivityOption.TaskQueue)
	s.Equal("demo-task-queue", *results.ActivityOption.TaskQueue)
	s.Require().NotNil(results.ActivityOption.ScheduleToCloseTimeout)
	s.Equal(7*time.Second, *results.ActivityOption.ScheduleToCloseTimeout)
	s.Require().NotNil(results.ActivityOption.Priority)
	s.Equal(4, results.ActivityOption.Priority.PriorityKey)
	s.Equal("tenant-a", results.ActivityOption.Priority.FairnessKey)
	s.InDelta(2.5, float64(results.ActivityOption.Priority.FairnessWeight), 0.01)
	s.Equal("decoded outer failure", results.FailureMessage)
	s.Equal("OuterFailure", results.FailureType)
	s.True(results.FailureNonRetryable)
	s.Equal("detail", results.FailureDetail)
	s.Equal("inner failure", results.FailureCauseMessage)
	s.Equal("InnerFailure", results.FailureCauseType)

	s.Require().Len(s.calls, 2)
	s.Equal([]string{"ActivityOptionsOperation", "FailureOperation"}, operationNames(s.calls))
	activityInput := s.calls[0].Input.(*activitypb.ActivityOptions)
	s.Equal(int32(3), activityInput.GetRetryPolicy().GetMaximumAttempts())
	s.Equal("demo-task-queue", activityInput.GetTaskQueue().GetName())
	s.Equal(7*time.Second, activityInput.GetScheduleToCloseTimeout().AsDuration())
	s.Equal(int32(4), activityInput.GetPriority().GetPriorityKey())
	failureInput := s.calls[1].Input.(*commandpb.FailWorkflowExecutionCommandAttributes)
	s.Equal("Encoded failure", failureInput.GetFailure().GetMessage())
	s.NotNil(failureInput.GetFailure().GetEncodedAttributes())
	s.Equal("OuterFailure", failureInput.GetFailure().GetApplicationFailureInfo().GetType())
	s.True(failureInput.GetFailure().GetApplicationFailureInfo().GetNonRetryable())
	s.Equal("inner failure", failureInput.GetFailure().GetCause().GetMessage())
}

func (s *TypeRoundtripIntegrationSuite) TestFailureOperationPreservesAbsence() {
	s.env.ExecuteWorkflow(func(ctx workflow.Context) (bool, error) {
		var result tr.FailureContainer
		if err := tr.FailureOperation(ctx, tr.FailureOperationOptions{}).Get(ctx, &result); err != nil {
			return false, err
		}
		return result.Failure == nil, nil
	})

	s.True(s.env.IsWorkflowCompleted())
	s.NoError(s.env.GetWorkflowError())
	var result bool
	s.NoError(s.env.GetWorkflowResult(&result))
	s.True(result)
	s.Require().Len(s.calls, 1)
	s.Nil(s.calls[0].Input.(*commandpb.FailWorkflowExecutionCommandAttributes).GetFailure())
}

func (s *TypeRoundtripIntegrationSuite) TestActivityOptionsOperationRequiredOnly() {
	policy := temporal.RetryPolicy{MaximumAttempts: 5}

	s.env.ExecuteWorkflow(func(ctx workflow.Context) (*tr.ActivityOptions, error) {
		var result tr.ActivityOptions
		return &result, tr.ActivityOptionsOperation(ctx, tr.ActivityOptionsOperationOptions{RetryPolicy: policy}).Get(ctx, &result)
	})

	s.True(s.env.IsWorkflowCompleted())
	s.NoError(s.env.GetWorkflowError())
	var result tr.ActivityOptions
	s.NoError(s.env.GetWorkflowResult(&result))
	s.Equal(int32(5), result.RetryPolicy.MaximumAttempts)
	// Optional fields that were never supplied remain absent after the round
	// trip, which is distinct from present fields containing their zero value.
	s.Nil(result.TaskQueue)
	s.Nil(result.ScheduleToCloseTimeout)
	s.Nil(result.Priority)

	s.Require().Len(s.calls, 1)
	s.Equal("ActivityOptionsOperation", s.calls[0].Operation)
	handlerInput := s.calls[0].Input.(*activitypb.ActivityOptions)
	s.Equal(int32(5), handlerInput.GetRetryPolicy().GetMaximumAttempts())
}

func (s *TypeRoundtripIntegrationSuite) TestNestedModelRoundTrips() {
	cases := []struct {
		name       string
		execution  tr.WorkflowExecutionInfo
		partitions tr.TaskQueuePartitions
		binaries   tr.BadBinaries
		wire       []proto.Message
	}{
		{
			name: "all fields",
			execution: tr.WorkflowExecutionInfo{
				Execution:       tr.WorkflowExecution{WorkflowID: "child", RunID: "child-run"},
				ParentExecution: &tr.WorkflowExecution{WorkflowID: "parent", RunID: "parent-run"},
			},
			partitions: tr.TaskQueuePartitions{WorkflowTaskQueuePartitions: []tr.TaskQueuePartition{
				{Key: "first", OwnerHostName: "first-host"},
				{Key: "second", OwnerHostName: "second-host"},
			}},
			binaries: tr.BadBinaries{Binaries: map[string]tr.BadBinary{
				"first":  {Reason: "first-reason", Operator: "first-operator"},
				"second": {Reason: "second-reason", Operator: "second-operator"},
			}},
			wire: []proto.Message{
				&workflowpb.WorkflowExecutionInfo{
					Execution:       &commonpb.WorkflowExecution{WorkflowId: "child", RunId: "child-run"},
					ParentExecution: &commonpb.WorkflowExecution{WorkflowId: "parent", RunId: "parent-run"},
				},
				&workflowservicepb.ListTaskQueuePartitionsResponse{WorkflowTaskQueuePartitions: []*taskqueuepb.TaskQueuePartitionMetadata{
					{Key: "first", OwnerHostName: "first-host"},
					{Key: "second", OwnerHostName: "second-host"},
				}},
				&namespacepb.BadBinaries{Binaries: map[string]*namespacepb.BadBinaryInfo{
					"first":  {Reason: "first-reason", Operator: "first-operator"},
					"second": {Reason: "second-reason", Operator: "second-operator"},
				}},
			},
		},
		{
			name:      "required only",
			execution: tr.WorkflowExecutionInfo{Execution: tr.WorkflowExecution{WorkflowID: "required", RunID: "required-run"}},
			wire: []proto.Message{
				&workflowpb.WorkflowExecutionInfo{Execution: &commonpb.WorkflowExecution{WorkflowId: "required", RunId: "required-run"}},
				&workflowservicepb.ListTaskQueuePartitionsResponse{},
				&namespacepb.BadBinaries{},
			},
		},
	}

	s.env.ExecuteWorkflow(func(ctx workflow.Context) error {
		for _, tt := range cases {
			execution := tr.WorkflowExecutionInfo{
				Execution:       tr.WorkflowExecution{WorkflowID: "stale"},
				ParentExecution: &tr.WorkflowExecution{WorkflowID: "stale"},
			}
			if err := tr.WorkflowExecutionInfoOperation(ctx, tr.WorkflowExecutionInfoOperationOptions{
				Execution: tt.execution.Execution, ParentExecution: tt.execution.ParentExecution,
			}).Get(ctx, &execution); err != nil {
				return err
			}
			if !reflect.DeepEqual(tt.execution, execution) {
				return fmt.Errorf("%s execution = %#v, want %#v", tt.name, execution, tt.execution)
			}
			partitions := tr.TaskQueuePartitions{WorkflowTaskQueuePartitions: []tr.TaskQueuePartition{{Key: "stale"}}}
			if err := tr.TaskQueuePartitionsOperation(ctx, tr.TaskQueuePartitionsOperationOptions{
				WorkflowTaskQueuePartitions: tt.partitions.WorkflowTaskQueuePartitions,
			}).Get(ctx, &partitions); err != nil {
				return err
			}
			if !reflect.DeepEqual(tt.partitions, partitions) {
				return fmt.Errorf("%s partitions = %#v, want %#v", tt.name, partitions, tt.partitions)
			}
			binaries := tr.BadBinaries{Binaries: map[string]tr.BadBinary{"stale": {Reason: "stale"}}}
			if err := tr.BadBinariesOperation(ctx, tr.BadBinariesOperationOptions{
				Binaries: tt.binaries.Binaries,
			}).Get(ctx, &binaries); err != nil {
				return err
			}
			if !reflect.DeepEqual(tt.binaries, binaries) {
				return fmt.Errorf("%s binaries = %#v, want %#v", tt.name, binaries, tt.binaries)
			}
		}
		return nil
	})

	s.Require().NoError(s.env.GetWorkflowError())
	s.Require().Len(s.calls, len(cases)*3)
	for i, tt := range cases {
		for j, want := range tt.wire {
			got := s.calls[i*3+j].Input.(proto.Message)
			s.True(proto.Equal(want, got), "%s operation %d: got %v, want %v", tt.name, j, got, want)
		}
	}
}
