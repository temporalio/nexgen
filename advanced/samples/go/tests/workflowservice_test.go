package tests

import (
	"context"
	"errors"
	"sync"
	"testing"
	"time"

	"github.com/nexus-rpc/sdk-go/nexus"
	"github.com/stretchr/testify/suite"
	common "go.temporal.io/api/common/v1"
	enums "go.temporal.io/api/enums/v1"
	workflowservicepb "go.temporal.io/api/workflowservice/v1"
	"go.temporal.io/sdk/converter"
	"go.temporal.io/sdk/internal"
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
	mu                   *sync.Mutex
}

func newRecordingDataConverter() recordingDataConverter {
	return recordingDataConverter{
		DataConverter: converter.GetDefaultDataConverter(),
		recorded:      &[]recordedEncode{},
		mu:            &sync.Mutex{},
	}
}

func (c recordingDataConverter) WithSerializationContext(ctx converter.SerializationContext) converter.DataConverter {
	return recordingDataConverter{
		DataConverter:        converter.WithDataConverterSerializationContext(c.DataConverter, ctx),
		serializationContext: ctx,
		recorded:             c.recorded,
		mu:                   c.mu,
	}
}

func (c recordingDataConverter) ToPayload(value any) (*common.Payload, error) {
	c.mu.Lock()
	*c.recorded = append(*c.recorded, recordedEncode{c.serializationContext, value})
	c.mu.Unlock()
	return c.DataConverter.ToPayload(value)
}

func (c recordingDataConverter) ToPayloads(values ...any) (*common.Payloads, error) {
	c.mu.Lock()
	for _, value := range values {
		*c.recorded = append(*c.recorded, recordedEncode{c.serializationContext, value})
	}
	c.mu.Unlock()
	return c.DataConverter.ToPayloads(values...)
}

func (c recordingDataConverter) snapshot() []recordedEncode {
	c.mu.Lock()
	defer c.mu.Unlock()
	return append([]recordedEncode(nil), (*c.recorded)...)
}

// Binding hides WithSerializationContext, so a bound converter cannot be scoped
// again. The generated operation must select its converter from the root.
type oneShotRecordingDataConverter struct {
	recordingDataConverter
}

func (c oneShotRecordingDataConverter) WithSerializationContext(ctx converter.SerializationContext) converter.DataConverter {
	return struct{ converter.DataConverter }{c.recordingDataConverter.WithSerializationContext(ctx)}
}

func nexusOperationSerializationContext() converter.NexusSerializationContext {
	return converter.NexusSerializationContext{
		Endpoint: "__temporal_system", Service: workflowServiceName,
		Operation: "SignalWithStartWorkflowExecution",
	}
}

type WorkflowServiceIntegrationSuite struct {
	suite.Suite
	testsuite.WorkflowTestSuite
	env     *testsuite.TestWorkflowEnvironment
	calls   []*workflowservicepb.SignalWithStartWorkflowExecutionRequest
	callsMu sync.Mutex
}

func (s *WorkflowServiceIntegrationSuite) SetupTest() {
	s.env = s.NewTestWorkflowEnvironment()
	s.calls = nil

	signalWithStart := nexus.NewSyncOperation("SignalWithStartWorkflowExecution",
		func(ctx context.Context, input *workflowservicepb.SignalWithStartWorkflowExecutionRequest, opts nexus.StartOperationOptions) (*workflowservicepb.SignalWithStartWorkflowExecutionResponse, error) {
			s.callsMu.Lock()
			s.calls = append(s.calls, input)
			s.callsMu.Unlock()
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
// inner user payloads use the target workflow context, while the outer proto
// uses the Nexus operation context.
func (s *WorkflowServiceIntegrationSuite) TestModelIsConvertedInsideThePayloadConverter() {
	s.checkSerializationContexts(false)
}

func (s *WorkflowServiceIntegrationSuite) TestOneShotConverterIsRescopedFromRoot() {
	s.checkSerializationContexts(true)
}

func (s *WorkflowServiceIntegrationSuite) checkSerializationContexts(oneShot bool) {
	recorder := newRecordingDataConverter()
	if oneShot {
		root := oneShotRecordingDataConverter{recorder}
		bound := root.WithSerializationContext(converter.WorkflowSerializationContext{})
		_, canRescope := bound.(converter.DataConverterWithSerializationContext)
		s.Require().False(canRescope, "test converter must not allow rebinding")
		s.env.SetDataConverter(root)
	} else {
		s.env.SetDataConverter(recorder)
	}

	s.env.ExecuteWorkflow(func(ctx workflow.Context) error {
		err := ws.SignalWithStartWorkflow(
			ctx,
			ws.SignalWithStartWorkflowOptions{
				ID: "target-workflow-id", TaskQueue: "my-task-queue",
				Memo:         map[string]any{"memo-key": "target-memo"},
				UserMetadata: ws.UserMetadata{StaticSummary: "target-summary", StaticDetails: "target-details"},
			},
			"wake-up",
			"signal-value",
			"ExampleWorkflow",
			"workflow-input",
		).Get(ctx, nil)
		if err != nil {
			return err
		}
		_, err = internal.GetDataConverterFromWorkflowContext(ctx).ToPayload("caller-after")
		return err
	})

	s.True(s.env.IsWorkflowCompleted())
	s.NoError(s.env.GetWorkflowError())
	s.Require().Len(s.calls, 1)
	s.Equal("default-test-namespace", s.calls[0].GetNamespace())
	s.Equal("target-workflow-id", s.calls[0].GetWorkflowId())
	// Headers are API-omitted and have no source in this sample.
	s.Nil(s.calls[0].GetHeader())
	for value, payload := range map[string]*common.Payload{
		"target-memo":    s.calls[0].GetMemo().GetFields()["memo-key"],
		"target-summary": s.calls[0].GetUserMetadata().GetSummary(),
		"target-details": s.calls[0].GetUserMetadata().GetDetails(),
	} {
		s.Require().NotNil(payload, value)
		var decoded string
		s.Require().NoError(converter.GetDefaultDataConverter().FromPayload(payload, &decoded))
		s.Equal(value, decoded)
	}

	recorded := recorder.snapshot()
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
		nexusOperationSerializationContext(),
		recorded[request].Context,
	)

	targetContext := converter.WorkflowSerializationContext{
		Namespace:  "default-test-namespace",
		WorkflowID: "target-workflow-id",
	}
	s.Equal(targetContext, recorded[workflowArg].Context)
	s.Equal(targetContext, recorded[signalArg].Context)
	// Memo uses the user converter with the current SDK memo flag enabled;
	// this sample's metadata helper also uses the workflow-context converter.
	for _, value := range []string{"target-memo", "target-summary", "target-details"} {
		i := stringAt(value)
		s.Require().NotEqual(-1, i, "%s was not encoded by the user converter", value)
		s.Less(i, request)
		s.Equal(targetContext, recorded[i].Context, value)
	}
	caller := stringAt("caller-after")
	s.Require().NotEqual(-1, caller)
	s.Equal(converter.WorkflowSerializationContext{
		Namespace: "default-test-namespace", WorkflowID: "default-test-workflow-id",
	}, recorded[caller].Context, "operation must not change the caller's converter")
}

func (s *WorkflowServiceIntegrationSuite) TestConcurrentOperationsKeepTargetContextsIsolated() {
	recorder := newRecordingDataConverter()
	s.env.SetDataConverter(oneShotRecordingDataConverter{recorder})
	targets := []string{"target-a", "target-b"}

	s.env.ExecuteWorkflow(func(ctx workflow.Context) error {
		futures := make([]workflow.Future, 0, len(targets))
		// Schedule both operations before awaiting either, then await in reverse
		// order to catch accidental reuse of the last operation's context.
		for _, target := range targets {
			futures = append(futures, ws.SignalWithStartWorkflow(ctx,
				ws.SignalWithStartWorkflowOptions{
					ID:   target,
					Memo: map[string]any{"memo-key": target + "-memo"},
					UserMetadata: ws.UserMetadata{
						StaticSummary: target + "-summary", StaticDetails: target + "-details",
					},
				},
				"wake-up", target+"-signal", "ExampleWorkflow", target+"-input",
			))
		}
		for i := len(futures) - 1; i >= 0; i-- {
			var result ws.SignalWithStartWorkflowResponse
			if err := futures[i].Get(ctx, &result); err != nil {
				return err
			}
		}
		_, err := internal.GetDataConverterFromWorkflowContext(ctx).ToPayload("caller-after")
		return err
	})

	s.True(s.env.IsWorkflowCompleted())
	s.Require().NoError(s.env.GetWorkflowError())
	s.Require().Len(s.calls, 2)
	var calledTargets []string
	for _, call := range s.calls {
		calledTargets = append(calledTargets, call.GetWorkflowId())
		s.Equal("default-test-namespace", call.GetNamespace())
		var input, signal string
		s.Require().NoError(converter.GetDefaultDataConverter().FromPayloads(call.GetInput(), &input))
		s.Require().NoError(converter.GetDefaultDataConverter().FromPayloads(call.GetSignalInput(), &signal))
		s.Equal(call.GetWorkflowId()+"-input", input)
		s.Equal(call.GetWorkflowId()+"-signal", signal)
	}
	s.ElementsMatch(targets, calledTargets)

	want := map[string]converter.SerializationContext{
		"caller-after": converter.WorkflowSerializationContext{
			Namespace: "default-test-namespace", WorkflowID: "default-test-workflow-id",
		},
	}
	for _, target := range targets {
		for _, suffix := range []string{"-input", "-signal", "-memo", "-summary", "-details"} {
			want[target+suffix] = converter.WorkflowSerializationContext{
				Namespace: "default-test-namespace", WorkflowID: target,
			}
		}
	}
	seen := make(map[string]int)
	envelopes := make(map[string]int)
	for _, entry := range recorder.snapshot() {
		switch value := entry.Value.(type) {
		case string:
			if expected, ok := want[value]; ok {
				s.Equal(expected, entry.Context, value)
				seen[value]++
			}
		case *workflowservicepb.SignalWithStartWorkflowExecutionRequest:
			target := value.GetWorkflowId()
			s.Contains(targets, target)
			if _, ok := entry.Context.(converter.NexusSerializationContext); ok {
				s.Equal(nexusOperationSerializationContext(), entry.Context, target)
				envelopes[target]++
			} else {
				// testNexusHandler.StartOperation rebuilds the decoded input as
				// a LazyValue using the test environment's caller-scoped converter.
				// This is separate from the real outer operation encoding above.
				s.Equal(want["caller-after"], entry.Context, "test harness re-encode for %s", target)
			}
		}
	}
	for value := range want {
		s.Equal(1, seen[value], "%s must be encoded once in its own context", value)
	}
	for _, target := range targets {
		s.Equal(1, envelopes[target], "outer operation envelope for %s must be encoded once in the Nexus context", target)
	}
}
