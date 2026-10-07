package tests

import (
	"context"
	"errors"
	"reflect"
	"sync"
	"testing"
	"time"

	"github.com/nexus-rpc/sdk-go/nexus"
	"github.com/stretchr/testify/suite"
	common "go.temporal.io/api/common/v1"
	enums "go.temporal.io/api/enums/v1"
	workflowservicepb "go.temporal.io/api/workflowservice/v1"
	"go.temporal.io/sdk/converter"
	"go.temporal.io/sdk/interceptor"
	"go.temporal.io/sdk/internal"
	"go.temporal.io/sdk/temporal"
	"go.temporal.io/sdk/testsuite"
	"go.temporal.io/sdk/worker"
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
// again. The SDK must select the operation's inner converter from the root.
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

// replaceNexusInputInterceptor lets the test replace a native input before the
// SDK selects its policy, without exposing the generated request type.
type replaceNexusInputInterceptor struct {
	interceptor.WorkerInterceptorBase
	replace func(any) any
}

func (i *replaceNexusInputInterceptor) InterceptWorkflow(ctx workflow.Context, next interceptor.WorkflowInboundInterceptor) interceptor.WorkflowInboundInterceptor {
	return &replaceNexusInputInbound{
		WorkflowInboundInterceptorBase: interceptor.WorkflowInboundInterceptorBase{Next: next},
		replace:                        i.replace,
	}
}

type replaceNexusInputInbound struct {
	interceptor.WorkflowInboundInterceptorBase
	replace func(any) any
}

func (i *replaceNexusInputInbound) Init(next interceptor.WorkflowOutboundInterceptor) error {
	return i.Next.Init(&replaceNexusInputOutbound{
		WorkflowOutboundInterceptorBase: interceptor.WorkflowOutboundInterceptorBase{Next: next},
		replace:                         i.replace,
	})
}

type replaceNexusInputOutbound struct {
	interceptor.WorkflowOutboundInterceptorBase
	replace func(any) any
}

func (i *replaceNexusInputOutbound) ExecuteNexusOperation(ctx workflow.Context, input interceptor.ExecuteNexusOperationInput) workflow.NexusOperationFuture {
	input.Input = i.replace(input.Input)
	return payloadContextForwardingFuture{i.Next.ExecuteNexusOperation(ctx, input)}
}

// Embedding the public future alone does not forward this optional SDK method.
type payloadContextForwardingFuture struct {
	workflow.NexusOperationFuture
}

func (f payloadContextForwardingFuture) NexusOperationPayloadContext() workflow.Context {
	if carrier, ok := f.NexusOperationFuture.(interface {
		NexusOperationPayloadContext() workflow.Context
	}); ok {
		return carrier.NexusOperationPayloadContext()
	}
	return nil
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

// The SDK retains the generated registry map. Tests that replace entries must
// restore them and remain sequential, with no concurrent lookup or execution.
func (s *WorkflowServiceIntegrationSuite) TestOperationRegistryUsesWireKeyAndNativeRequest() {
	key := internal.NexusOperationKey{
		Service: workflowServiceName, Operation: "SignalWithStartWorkflowExecution",
	}
	original, ok := ws.NexusOperationRegistry[key]
	s.Require().True(ok, "registry must use the wire service and operation names")
	s.Require().NotNil(original.SerializationContext)
	s.T().Cleanup(func() { ws.NexusOperationRegistry[key] = original })

	var requests []any
	var contexts []converter.SerializationContext
	wrapped := original
	wrapped.SerializationContext = func(request any) converter.SerializationContext {
		requests = append(requests, request)
		selected := original.SerializationContext(request)
		contexts = append(contexts, selected)
		return selected
	}
	ws.NexusOperationRegistry[key] = wrapped

	s.checkSerializationContexts(true)

	s.Require().Len(requests, 1, "SDK must invoke the registry callback exactly once")
	s.Equal([]converter.SerializationContext{converter.WorkflowSerializationContext{
		Namespace: "default-test-namespace", WorkflowID: "target-workflow-id",
	}}, contexts)
	// The native request type is intentionally unexported. Inspect its fields
	// without converting it to a proto or adding a public model API for tests.
	request := reflect.ValueOf(requests[0])
	s.Require().Equal(reflect.Struct, request.Kind(), "callback must receive a native request value")
	s.Equal(request.Type(), original.InputType, "generated adapter must record the native input type")
	s.Equal(reflect.TypeOf(ws.SignalWithStartWorkflowOptions{}).PkgPath(), request.Type().PkgPath())
	for name, want := range map[string]string{
		"namespace": "default-test-namespace", "ID": "target-workflow-id",
		"TaskQueue": "my-task-queue", "Workflow": "ExampleWorkflow", "Signal": "wake-up",
	} {
		field := request.FieldByName(name)
		s.Require().True(field.IsValid(), "native request field %s is missing", name)
		s.Require().Equal(reflect.String, field.Kind(), name)
		s.Equal(want, field.String(), name)
	}
	for name, want := range map[string][]any{
		"Args": {"workflow-input"}, "SignalArgs": {"signal-value"},
	} {
		field := request.FieldByName(name)
		s.Require().True(field.IsValid(), "native request field %s is missing", name)
		s.Equal(want, field.Interface(), name)
	}
}

func (s *WorkflowServiceIntegrationSuite) TestRawExecuteOperationUsesRegisteredNativeRequest() {
	key := internal.NexusOperationKey{
		Service: workflowServiceName, Operation: "SignalWithStartWorkflowExecution",
	}
	original, ok := ws.NexusOperationRegistry[key]
	s.Require().True(ok)
	s.Require().NotNil(original.SerializationContext)
	s.T().Cleanup(func() { ws.NexusOperationRegistry[key] = original })

	// Capture the unexported native model without inventing a public constructor
	// or substituting a protobuf that would bypass the native policy.
	var nativeRequest any
	selections := 0
	entry := original
	entry.SerializationContext = func(request any) converter.SerializationContext {
		nativeRequest = request
		selections++
		return original.SerializationContext(request)
	}
	ws.NexusOperationRegistry[key] = entry
	recorder := newRecordingDataConverter()
	s.env.SetDataConverter(oneShotRecordingDataConverter{recorder})
	endpoints := []string{"__temporal_system", "temporal-system", "ordinary-endpoint"}
	s.env.SetWorkerOptions(worker.Options{Interceptors: []interceptor.WorkerInterceptor{
		&replaceNexusInputInterceptor{replace: func(input any) any {
			// Preserve the unexported sourced fields while replacing the exported ID.
			value := reflect.ValueOf(input)
			replacement := reflect.New(value.Type()).Elem()
			replacement.Set(value)
			replacement.FieldByName("ID").SetString("intercepted-target")
			return replacement.Interface()
		}},
	}})

	s.env.ExecuteWorkflow(func(ctx workflow.Context) error {
		if err := ws.SignalWithStartWorkflow(ctx,
			ws.SignalWithStartWorkflowOptions{ID: "raw-target"},
			"wake-up", "raw-signal", "ExampleWorkflow", "raw-input",
		).Get(ctx, nil); err != nil {
			return err
		}
		if nativeRequest == nil {
			return errors.New("SDK registry callback did not capture the native request")
		}
		for _, endpoint := range endpoints {
			// The public constructor rejects the reserved canonical endpoint.
			var client workflow.NexusClient
			if endpoint == "__temporal_system" {
				client = internal.NewSystemNexusClient(workflowServiceName)
			} else {
				client = workflow.NewNexusClient(endpoint, workflowServiceName)
			}
			future := client.ExecuteOperation(ctx, key.Operation, nativeRequest, workflow.NexusOperationOptions{})
			var response ws.SignalWithStartWorkflowResponse
			if err := future.Get(ctx, &response); err != nil {
				return err
			}
			// An opaque wrapper must not be mistaken for an explicit carrier.
			opaque := struct{ workflow.NexusOperationFuture }{future}
			if internal.NexusOperationPayloadContext(ctx, opaque) != ctx {
				return errors.New("opaque future wrapper unexpectedly forwarded inner context")
			}
			inner := internal.NexusOperationPayloadContext(ctx, future)
			if _, err := internal.GetDataConverterFromWorkflowContext(inner).ToPayload("scope-" + endpoint); err != nil {
				return err
			}
		}
		return nil
	})

	s.Require().NoError(s.env.GetWorkflowError())
	s.Require().Len(s.calls, 4)
	for _, call := range s.calls {
		s.Equal("intercepted-target", call.GetWorkflowId())
		s.Equal("default-test-namespace", call.GetNamespace())
	}
	s.Equal(3, selections, "SDK selects once for the wrapper and each raw system call, never for an ordinary endpoint")
	target := converter.WorkflowSerializationContext{
		Namespace: "default-test-namespace", WorkflowID: "intercepted-target",
	}
	caller := converter.WorkflowSerializationContext{
		Namespace: "default-test-namespace", WorkflowID: "default-test-workflow-id",
	}
	innerEncodes := map[converter.SerializationContext]int{}
	scopes := map[string]converter.SerializationContext{}
	envelopes := map[string]int{}
	for _, encoded := range recorder.snapshot() {
		switch value := encoded.Value.(type) {
		case string:
			if value == "raw-input" {
				innerEncodes[encoded.Context]++
			}
			for _, endpoint := range endpoints {
				if value == "scope-"+endpoint {
					scopes[endpoint] = encoded.Context
				}
			}
		case *workflowservicepb.SignalWithStartWorkflowExecutionRequest:
			if sc, ok := encoded.Context.(converter.NexusSerializationContext); ok {
				s.Equal(workflowServiceName, sc.Service)
				s.Equal(key.Operation, sc.Operation)
				envelopes[sc.Endpoint]++
			}
		}
	}
	s.Equal(map[converter.SerializationContext]int{target: 3, caller: 1}, innerEncodes)
	s.Equal(map[string]converter.SerializationContext{
		"__temporal_system": target, "temporal-system": target, "ordinary-endpoint": caller,
	}, scopes, "SDK future must retain inner scope; non-system calls keep caller scope")
	s.Equal(map[string]int{
		"__temporal_system": 2, "temporal-system": 1, "ordinary-endpoint": 1,
	}, envelopes, "outer envelopes remain Nexus-scoped on every endpoint")
}

func (s *WorkflowServiceIntegrationSuite) TestRawProtobufSkipsNativeRegistryCallbacks() {
	key := internal.NexusOperationKey{
		Service: workflowServiceName, Operation: "SignalWithStartWorkflowExecution",
	}
	original, ok := ws.NexusOperationRegistry[key]
	s.Require().True(ok)
	s.Require().NotNil(original.InputType, "generated registry must guard the native callbacks")
	s.Require().NotEqual(reflect.TypeFor[*workflowservicepb.SignalWithStartWorkflowExecutionRequest](), original.InputType)
	s.T().Cleanup(func() { ws.NexusOperationRegistry[key] = original })

	selections, conversions := 0, 0
	entry := original
	entry.SerializationContext = func(any) converter.SerializationContext {
		selections++
		return converter.WorkflowSerializationContext{Namespace: "unexpected", WorkflowID: "unexpected"}
	}
	// Keep the generated InputType while installing a callback that would reject
	// wire inputs. The SDK must skip both callbacks, not just policy selection.
	entry.InputToTransfer = func(workflow.Context, any) (any, error) {
		conversions++
		return nil, errors.New("raw protobuf reached native input conversion")
	}
	ws.NexusOperationRegistry[key] = entry
	payloads, err := converter.GetDefaultDataConverter().ToPayloads("already-encoded-input")
	s.Require().NoError(err)
	request := &workflowservicepb.SignalWithStartWorkflowExecutionRequest{
		Namespace: "wire-namespace", WorkflowId: "wire-workflow", SignalName: "wake-up",
		Input: payloads, SignalInput: payloads,
	}
	recorder := newRecordingDataConverter()
	s.env.SetDataConverter(oneShotRecordingDataConverter{recorder})
	s.env.ExecuteWorkflow(func(ctx workflow.Context) error {
		clients := []workflow.NexusClient{
			internal.NewSystemNexusClient(workflowServiceName),
			workflow.NewNexusClient("temporal-system", workflowServiceName),
		}
		for _, client := range clients {
			future := client.ExecuteOperation(ctx, key.Operation, request, workflow.NexusOperationOptions{})
			var response workflowservicepb.SignalWithStartWorkflowExecutionResponse
			if err := future.Get(ctx, &response); err != nil {
				return err
			}
			if internal.NexusOperationPayloadContext(ctx, future) != ctx {
				return errors.New("raw protobuf unexpectedly selected an inner context")
			}
		}
		return nil
	})

	s.Require().NoError(s.env.GetWorkflowError())
	s.Zero(selections, "nonmatching wire input must skip the native policy")
	s.Zero(conversions, "nonmatching wire input must skip external conversion")
	s.Require().Len(s.calls, 2)
	for _, call := range s.calls {
		s.Equal(request.Namespace, call.Namespace)
		s.Equal(request.WorkflowId, call.WorkflowId)
		s.Equal(request.SignalName, call.SignalName)
		s.Equal(payloads, call.Input)
		s.Equal(payloads, call.SignalInput)
	}
	envelopes := map[string]int{}
	for _, encoded := range recorder.snapshot() {
		if sc, ok := encoded.Context.(converter.NexusSerializationContext); ok {
			if _, ok := encoded.Value.(*workflowservicepb.SignalWithStartWorkflowExecutionRequest); ok {
				s.Equal(workflowServiceName, sc.Service)
				s.Equal(key.Operation, sc.Operation)
				envelopes[sc.Endpoint]++
			}
		}
	}
	s.Equal(map[string]int{"__temporal_system": 1, "temporal-system": 1}, envelopes)
}

func (s *WorkflowServiceIntegrationSuite) TestOperationRegistryNilCallbackPreservesCallerContext() {
	s.checkRegistryWithoutOverride(false)
}

func (s *WorkflowServiceIntegrationSuite) TestOperationRegistryNilResultPreservesCallerContext() {
	s.checkRegistryWithoutOverride(true)
}

func (s *WorkflowServiceIntegrationSuite) checkRegistryWithoutOverride(returnNil bool) {
	key := internal.NexusOperationKey{
		Service: workflowServiceName, Operation: "SignalWithStartWorkflowExecution",
	}
	original, ok := ws.NexusOperationRegistry[key]
	s.Require().True(ok)
	s.T().Cleanup(func() { ws.NexusOperationRegistry[key] = original })
	entry := original
	entry.SerializationContext = nil
	calls := 0
	if returnNil {
		entry.SerializationContext = func(any) converter.SerializationContext {
			calls++
			return nil
		}
	}
	ws.NexusOperationRegistry[key] = entry

	s.checkSerializationContextsWithTarget(true, converter.WorkflowSerializationContext{
		Namespace: "default-test-namespace", WorkflowID: "default-test-workflow-id",
	})
	if returnNil {
		s.Equal(1, calls, "nil-returning callback must be invoked exactly once")
	} else {
		s.Zero(calls)
	}
}

func (s *WorkflowServiceIntegrationSuite) checkSerializationContexts(oneShot bool) {
	s.checkSerializationContextsWithTarget(oneShot, converter.WorkflowSerializationContext{
		Namespace: "default-test-namespace", WorkflowID: "target-workflow-id",
	})
}

func (s *WorkflowServiceIntegrationSuite) checkSerializationContextsWithTarget(oneShot bool, targetContext converter.WorkflowSerializationContext) {
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
