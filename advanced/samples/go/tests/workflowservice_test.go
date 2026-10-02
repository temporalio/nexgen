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

// recordedEncode captures one value handed to the data converter along with the
// serialization context that was active at the time.
type recordedEncode struct {
	Context converter.SerializationContext
	Value   any
}

// recordingDataConverter records every value it is asked to encode. It also
// implements [converter.DataConverterWithSerializationContext] so that the
// recorded entries show which serialization context the SDK applied.
type recordingDataConverter struct {
	converter.DataConverter
	serializationContext converter.SerializationContext
	recorded             *[]recordedEncode
}

func newRecordingDataConverter() recordingDataConverter {
	return recordingDataConverter{
		DataConverter: converter.GetDefaultDataConverter(),
		recorded:      &[]recordedEncode{},
	}
}

func (c recordingDataConverter) WithSerializationContext(ctx converter.SerializationContext) converter.DataConverter {
	return recordingDataConverter{
		DataConverter:        converter.WithDataConverterSerializationContext(c.DataConverter, ctx),
		serializationContext: ctx,
		recorded:             c.recorded,
	}
}

func (c recordingDataConverter) ToPayload(value any) (*common.Payload, error) {
	*c.recorded = append(*c.recorded, recordedEncode{c.serializationContext, value})
	return c.DataConverter.ToPayload(value)
}

func (c recordingDataConverter) ToPayloads(values ...any) (*common.Payloads, error) {
	for _, value := range values {
		*c.recorded = append(*c.recorded, recordedEncode{c.serializationContext, value})
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

func (s *WorkflowServiceIntegrationSuite) TestConversionFailureSurfacesOnTheFuture() {
	// Model->proto conversion now runs inside the SDK's payload converter, so a
	// conversion failure resolves the operation future with an error instead of
	// failing synchronously before the operation is scheduled.
	s.env.ExecuteWorkflow(func(ctx workflow.Context) error {
		fut := ws.SignalWithStartWorkflow(ctx, ws.SignalWithStartWorkflowOptions{ID: "workflow-id", Memo: map[string]any{"invalid": func() {}}}, "wake-up", "signal-value", signalWithStartWorkflow, "workflow-input")
		if err := fut.Get(ctx, nil); err == nil {
			return errors.New("conversion failure future returned no error")
		}
		return nil
	})

	s.NoError(s.env.GetWorkflowError())
	s.Empty(s.calls)
}

// TestModelIsConvertedInsideThePayloadConverter is the acceptance test for the
// transfer-type port. The generated code no longer builds the proto itself;
// the SDK's transfer-type machinery does it during payload conversion. The
// observable consequence is that the model's inner user payloads (Args,
// SignalArgs) are encoded by the data converter -- and under the serialization
// context -- that the SDK selected for the Nexus operation.
func (s *WorkflowServiceIntegrationSuite) TestModelIsConvertedInsideThePayloadConverter() {
	recorder := newRecordingDataConverter()
	s.env.SetDataConverter(recorder)

	s.env.ExecuteWorkflow(func(ctx workflow.Context) error {
		return ws.SignalWithStartWorkflow(
			ctx,
			ws.SignalWithStartWorkflowOptions{ID: "target-workflow-id", TaskQueue: "my-task-queue"},
			"wake-up",
			"signal-value",
			"ExampleWorkflow",
			"workflow-input",
		).Get(ctx, nil)
	})

	s.True(s.env.IsWorkflowCompleted())
	s.NoError(s.env.GetWorkflowError())
	s.Require().Len(s.calls, 1)

	recorded := *recorder.recorded
	indexOf := func(match func(recordedEncode) bool) int {
		for i, entry := range recorded {
			if match(entry) {
				return i
			}
		}
		return -1
	}
	stringAt := func(want string) int {
		return indexOf(func(entry recordedEncode) bool {
			value, ok := entry.Value.(string)
			return ok && value == want
		})
	}

	workflowArg := stringAt("workflow-input")
	signalArg := stringAt("signal-value")
	request := indexOf(func(entry recordedEncode) bool {
		_, ok := entry.Value.(*workflowservicepb.SignalWithStartWorkflowExecutionRequest)
		return ok
	})

	s.Require().NotEqual(-1, workflowArg, "workflow argument was not encoded by the SDK's data converter")
	s.Require().NotEqual(-1, signalArg, "signal argument was not encoded by the SDK's data converter")
	s.Require().NotEqual(-1, request, "the SDK's data converter never received the transfer value")

	// The inner user payloads are encoded while the SDK converts the model,
	// i.e. strictly before the resulting proto reaches the data converter.
	s.Less(workflowArg, request)
	s.Less(signalArg, request)

	// The transfer value -- the proto envelope -- is encoded under the Nexus
	// operation's serialization context.
	s.Equal(
		converter.NexusSerializationContext{
			Endpoint:  "__temporal_system",
			Service:   workflowServiceName,
			Operation: "SignalWithStartWorkflowExecution",
		},
		recorded[request].Context,
	)

	// The inner user payloads are encoded under the workflow serialization
	// context reachable from the workflow.Context handed to the transfer
	// converter. That context is the *calling* workflow's today; redirecting it
	// to the target workflow is what @nexus.serialization-context will do.
	callerContext := converter.WorkflowSerializationContext{
		Namespace:  "default-test-namespace",
		WorkflowID: "default-test-workflow-id",
	}
	s.Equal(callerContext, recorded[workflowArg].Context)
	s.Equal(callerContext, recorded[signalArg].Context)
}
