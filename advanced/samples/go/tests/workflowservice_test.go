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
	"google.golang.org/protobuf/proto"

	ws "go.temporal.io/sdk/advanced/samples/go/workflowservice"
)

const workflowServiceName = "temporal.api.workflowservice.v1.WorkflowService"

func doNothingWorkflow(ctx workflow.Context, input string) error {
	return nil
}

type recordedEncode struct {
	sc    converter.SerializationContext
	value any
}

// recordingDataConverter records every value it is asked to encode. It also
// implements [converter.DataConverterWithSerializationContext] so that the
// recorded entries show which serialization context the SDK applied.
type recordingDataConverter struct {
	converter.DataConverter
	serializationContext converter.SerializationContext
	recorded             *[]recordedEncode
	envelopes            *[]*common.Payload
	mu                   *sync.Mutex
}

func newRecordingDataConverter() recordingDataConverter {
	return recordingDataConverter{
		DataConverter: converter.NewCodecDataConverter(converter.GetDefaultDataConverter(), envelopeRejectingCodec{
			PayloadCodec: converter.NewZlibCodec(converter.ZlibCodecOptions{AlwaysEncode: true}),
		}),
		recorded:  &[]recordedEncode{},
		envelopes: &[]*common.Payload{},
		mu:        &sync.Mutex{},
	}
}

type envelopeRejectingCodec struct {
	converter.PayloadCodec
}

func (c envelopeRejectingCodec) Encode(payloads []*common.Payload) ([]*common.Payload, error) {
	for _, payload := range payloads {
		if len(payload.Metadata[converter.MetadataMessageType]) != 0 {
			return nil, errors.New("user codec must not encode an operation envelope")
		}
	}
	return c.PayloadCodec.Encode(payloads)
}

func (c envelopeRejectingCodec) Decode(payloads []*common.Payload) ([]*common.Payload, error) {
	for _, payload := range payloads {
		if len(payload.Metadata[converter.MetadataMessageType]) != 0 {
			return nil, errors.New("user codec must not decode an operation envelope")
		}
	}
	return c.PayloadCodec.Decode(payloads)
}

func (c recordingDataConverter) WithSerializationContext(ctx converter.SerializationContext) converter.DataConverter {
	return recordingDataConverter{
		DataConverter:        converter.WithDataConverterSerializationContext(c.DataConverter, ctx),
		serializationContext: ctx,
		recorded:             c.recorded,
		envelopes:            c.envelopes,
		mu:                   c.mu,
	}
}

func (c recordingDataConverter) ToPayload(value any) (*common.Payload, error) {
	// Handler responses model the server's fixed protobuf encoding.
	if _, response := value.(*workflowservicepb.SignalWithStartWorkflowExecutionResponse); response {
		payload, err := converter.NewProtoPayloadConverter().ToPayload(value)
		if err == nil {
			payload.Metadata["__temporal_system_payload"] = []byte("true")
		}
		return payload, err
	}
	// The testsuite also rebuilds decoded requests with the caller's converter.
	if _, callerScoped := c.serializationContext.(converter.WorkflowSerializationContext); callerScoped {
		switch value.(type) {
		case *workflowservicepb.SignalWithStartWorkflowExecutionRequest:
			return converter.NewProtoPayloadConverter().ToPayload(value)
		}
	}
	c.mu.Lock()
	*c.recorded = append(*c.recorded, recordedEncode{c.serializationContext, value})
	c.mu.Unlock()
	return c.DataConverter.ToPayload(value)
}

func (c recordingDataConverter) FromPayload(payload *common.Payload, value any) error {
	switch value.(type) {
	case **workflowservicepb.SignalWithStartWorkflowExecutionRequest:
		if _, marked := payload.Metadata["__temporal_system_payload"]; marked {
			c.mu.Lock()
			*c.envelopes = append(*c.envelopes, proto.Clone(payload).(*common.Payload))
			c.mu.Unlock()
		}
		return converter.NewProtoPayloadConverter().FromPayload(payload, value)
	case **workflowservicepb.SignalWithStartWorkflowExecutionResponse:
		return converter.NewProtoPayloadConverter().FromPayload(payload, value)
	default:
		return c.DataConverter.FromPayload(payload, value)
	}
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
	return i.Next.ExecuteNexusOperation(ctx, input)
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
	s.env.SetDataConverter(systemNexusTestDataConverter())
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
	s.env.RegisterWorkflowWithOptions(doNothingWorkflow, workflow.RegisterOptions{
		Name: "registered-workflow",
	})
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
			doNothingWorkflow,
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
		defaultOpts := opts
		defaultOpts.WorkflowIDReusePolicy = 0
		return ws.SignalWithStartWorkflow(
			ctx,
			defaultOpts,
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
	s.Equal("registered-workflow", typedRequest.GetWorkflowType().GetName())
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
	s.Equal("ExampleWorkflow", variadicRequest.GetWorkflowType().GetName())
	s.Equal(enums.WORKFLOW_ID_REUSE_POLICY_ALLOW_DUPLICATE, variadicRequest.GetWorkflowIdReusePolicy())
	// A nil signal argument is still one argument and therefore one payload;
	// it does not mean that the signal has no arguments.
	s.Require().NotNil(variadicRequest.GetSignalInput())
	s.Len(variadicRequest.GetSignalInput().GetPayloads(), 1)
	s.Require().NotNil(variadicRequest.GetInput())
	s.Len(variadicRequest.GetInput().GetPayloads(), 2)
}

func (s *WorkflowServiceIntegrationSuite) TestSliceSignalArgumentRemainsOnePayload() {
	signalArg := []string{"one", "two"}
	s.env.ExecuteWorkflow(func(ctx workflow.Context) error {
		opts := ws.SignalWithStartWorkflowOptions{ID: "workflow-id"}
		if err := ws.SignalWithStartWorkflowTyped(
			ctx, opts, "wake-up", signalArg, doNothingWorkflow, "workflow-input",
		).Get(ctx, nil); err != nil {
			return err
		}
		return ws.SignalWithStartWorkflow(
			ctx, opts, "wake-up", signalArg, "ExampleWorkflow", "workflow-input",
		).Get(ctx, nil)
	})

	s.True(s.env.IsWorkflowCompleted())
	s.NoError(s.env.GetWorkflowError())
	s.Require().Len(s.calls, 2)
	for _, request := range s.calls {
		payloads := request.GetSignalInput()
		s.Require().NotNil(payloads)
		s.Require().Len(payloads.GetPayloads(), 1)
		var decoded []string
		s.NoError(converter.GetDefaultDataConverter().FromPayloads(payloads, &decoded))
		s.Equal(signalArg, decoded)
	}
}

// A non-nil empty result distinguishes converter delegation from a nil shortcut.
type nonNilEmptyPayloadsDataConverter struct {
	converter.DataConverter
}

func (c nonNilEmptyPayloadsDataConverter) ToPayloads(values ...any) (*common.Payloads, error) {
	if len(values) == 0 {
		return &common.Payloads{}, nil
	}
	return c.DataConverter.ToPayloads(values...)
}

func (s *WorkflowServiceIntegrationSuite) TestEmptyPayloadsAreDelegatedToDataConverter() {
	s.env.SetDataConverter(nonNilEmptyPayloadsDataConverter{systemNexusTestDataConverter()})
	s.env.ExecuteWorkflow(func(ctx workflow.Context) error {
		var result ws.SignalWithStartWorkflowResponse
		return ws.SignalWithStartWorkflow(
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
		return ws.SignalWithStartWorkflow(ctx, ws.SignalWithStartWorkflowOptions{ID: "workflow-id"}, "wake-up", "signal-value", doNothingWorkflow, "workflow-input").Get(ctx, nil)
	})

	s.Error(s.env.GetWorkflowError())
	s.Empty(s.calls)
}

func (s *WorkflowServiceIntegrationSuite) TestConversionFailureSurfacesOnTheFuture() {
	// Model->proto conversion now runs inside the SDK's payload converter, so a
	// conversion failure resolves the operation future with an error instead of
	// failing synchronously before the operation is scheduled.
	s.env.ExecuteWorkflow(func(ctx workflow.Context) error {
		fut := ws.SignalWithStartWorkflow(ctx, ws.SignalWithStartWorkflowOptions{ID: "workflow-id", Memo: map[string]any{"invalid": func() {}}}, "wake-up", "signal-value", doNothingWorkflow, "workflow-input")
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
// inner user payloads use the target workflow context and full user converter,
// while the outer proto bypasses the user converter and codec.
func (s *WorkflowServiceIntegrationSuite) TestModelIsConvertedInsideThePayloadConverter() {
	s.checkSerializationContexts(false)
}

func (s *WorkflowServiceIntegrationSuite) TestOneShotConverterIsRescopedFromRoot() {
	s.checkSerializationContexts(true)
}

func (s *WorkflowServiceIntegrationSuite) TestOperationRegistryUsesWireKeyAndNativeRequest() {
	key := internal.NexusOperationKey{
		Service: workflowServiceName, Operation: "SignalWithStartWorkflowExecution",
	}
	original, ok := ws.NexusOperationRegistry[key]
	s.Require().True(ok, "registry must use the wire service and operation names")
	s.Require().NotNil(original.SerializationContext)

	var requests []any
	s.env.SetWorkerOptions(worker.Options{Interceptors: []interceptor.WorkerInterceptor{
		&replaceNexusInputInterceptor{replace: func(input any) any {
			requests = append(requests, input)
			return input
		}},
	}})

	s.checkSerializationContexts(true)

	s.Require().Len(requests, 1)
	s.Equal(converter.WorkflowSerializationContext{
		Namespace: "default-test-namespace", WorkflowID: "target-workflow-id",
	}, original.SerializationContext(requests[0]))
	// The policy must receive the generated native model, not a protobuf.
	request := reflect.ValueOf(requests[0])
	s.Require().Equal(reflect.Struct, request.Kind(), "callback must receive a native request value")
	s.Equal(reflect.TypeOf(ws.SignalWithStartWorkflowOptions{}).PkgPath(), request.Type().PkgPath())
}

func (s *WorkflowServiceIntegrationSuite) TestRawExecuteOperationUsesRegisteredNativeRequest() {
	key := internal.NexusOperationKey{
		Service: workflowServiceName, Operation: "SignalWithStartWorkflowExecution",
	}

	// Capture the unexported native model without inventing a public constructor
	// or substituting a protobuf that would bypass the native policy.
	var nativeRequest any
	recorder := newRecordingDataConverter()
	s.env.SetDataConverter(oneShotRecordingDataConverter{recorder})
	endpoints := []string{"__temporal_system", "temporal-system", "ordinary-endpoint"}
	s.env.SetWorkerOptions(worker.Options{Interceptors: []interceptor.WorkerInterceptor{
		&replaceNexusInputInterceptor{replace: func(input any) any {
			// Preserve the sourced fields while replacing the exported ID.
			value := reflect.ValueOf(input)
			replacement := reflect.New(value.Type()).Elem()
			replacement.Set(value)
			replacement.FieldByName("ID").SetString("intercepted-target")
			nativeRequest = replacement.Interface()
			return nativeRequest
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
			return errors.New("workflow interceptor did not capture the native request")
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
			err := future.Get(ctx, &response)
			if endpoint != "__temporal_system" {
				if err == nil {
					return errors.New("ordinary endpoint accepted a system-only native request")
				}
				s.ErrorContains(err, "without a system Nexus config", endpoint)
			} else if err != nil {
				return err
			}
		}
		return nil
	})

	s.Require().NoError(s.env.GetWorkflowError())
	s.Require().Len(s.calls, 2)
	for _, call := range s.calls {
		s.Equal("intercepted-target", call.GetWorkflowId())
		s.Equal("default-test-namespace", call.GetNamespace())
	}
	target := converter.WorkflowSerializationContext{
		Namespace: "default-test-namespace", WorkflowID: "intercepted-target",
	}
	innerEncodes := map[converter.SerializationContext]int{}
	for _, encoded := range recorder.snapshot() {
		switch value := encoded.value.(type) {
		case string:
			if value == "raw-input" {
				innerEncodes[encoded.sc]++
			}
		case *workflowservicepb.SignalWithStartWorkflowExecutionRequest:
			s.Fail("user converter received an operation envelope", "context: %v", encoded.sc)
		}
	}
	s.Equal(map[converter.SerializationContext]int{target: 2}, innerEncodes)
	s.checkRawEnvelopes(recorder, 2)
}

func (s *WorkflowServiceIntegrationSuite) TestOrdinaryEndpointsIgnoreRegisteredTargetContext() {
	key := internal.NexusOperationKey{
		Service: workflowServiceName, Operation: "SignalWithStartWorkflowExecution",
	}
	s.Require().NotNil(ws.NexusOperationRegistry[key].SerializationContext)
	s.env = s.NewTestWorkflowEnvironment()
	recorder := newRecordingDataConverter()
	s.env.SetDataConverter(oneShotRecordingDataConverter{recorder})
	service := nexus.NewService(workflowServiceName)
	s.Require().NoError(service.Register(nexus.NewSyncOperation(key.Operation,
		func(ctx context.Context, input string, opts nexus.StartOperationOptions) (string, error) {
			return input, nil
		})))
	s.env.RegisterNexusService(service)
	endpoints := []string{"temporal-system", "ordinary-endpoint"}

	s.env.ExecuteWorkflow(func(ctx workflow.Context) error {
		for _, endpoint := range endpoints {
			client := workflow.NewNexusClient(endpoint, workflowServiceName)
			var result string
			if err := client.ExecuteOperation(ctx, key.Operation, endpoint+"-input", workflow.NexusOperationOptions{}).Get(ctx, &result); err != nil {
				return err
			}
			if result != endpoint+"-input" {
				return errors.New("ordinary endpoint response did not round trip")
			}
		}
		return nil
	})

	s.Require().NoError(s.env.GetWorkflowError())
	for _, endpoint := range endpoints {
		want := converter.NexusSerializationContext{
			Endpoint: endpoint, Service: key.Service, Operation: key.Operation,
		}
		found := false
		for _, encoded := range recorder.snapshot() {
			if encoded.value == endpoint+"-input" && encoded.sc == want {
				found = true
			}
		}
		s.True(found, "ordinary input must use its Nexus scope: %s", endpoint)
	}
	s.checkRawEnvelopes(recorder, 0)
}

func (s *WorkflowServiceIntegrationSuite) checkRawEnvelopes(recorder recordingDataConverter, count int) {
	recorder.mu.Lock()
	envelopes := append([]*common.Payload(nil), (*recorder.envelopes)...)
	recorder.mu.Unlock()
	s.Require().Len(envelopes, count, "server must consume marked raw protobuf envelopes")
	for _, payload := range envelopes {
		s.Equal([]byte("binary/protobuf"), payload.Metadata[converter.MetadataEncoding])
		s.Equal([]byte("temporal.api.workflowservice.v1.SignalWithStartWorkflowExecutionRequest"), payload.Metadata[converter.MetadataMessageType])
		s.Equal([]byte("true"), payload.Metadata["__temporal_system_payload"])
		var request workflowservicepb.SignalWithStartWorkflowExecutionRequest
		s.Require().NoError(proto.Unmarshal(payload.Data, &request))
		s.NotEmpty(request.GetWorkflowId())
		for _, args := range []*common.Payloads{request.GetInput(), request.GetSignalInput()} {
			for _, nested := range args.GetPayloads() {
				s.Equal([]byte("binary/zlib"), nested.Metadata[converter.MetadataEncoding], "target arguments must use the user codec")
			}
		}
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
		s.Equal([]byte("binary/zlib"), payload.Metadata[converter.MetadataEncoding], value)
		s.Require().NoError(recorder.DataConverter.FromPayload(payload, &decoded))
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
			value, ok := entry.value.(string)
			return ok && value == want
		})
	}

	workflowArg := stringAt("workflow-input")
	signalArg := stringAt("signal-value")
	s.Require().NotEqual(-1, workflowArg, "workflow argument was not encoded by the SDK's data converter")
	s.Require().NotEqual(-1, signalArg, "signal argument was not encoded by the SDK's data converter")
	for _, entry := range recorded {
		_, envelope := entry.value.(*workflowservicepb.SignalWithStartWorkflowExecutionRequest)
		s.False(envelope, "user converter must not receive the outer request")
	}
	s.checkRawEnvelopes(recorder, 1)

	s.Equal(targetContext, recorded[workflowArg].sc)
	s.Equal(targetContext, recorded[signalArg].sc)
	// Memo uses the user converter with the current SDK memo flag enabled;
	// this sample's metadata helper also uses the workflow-context converter.
	for _, value := range []string{"target-memo", "target-summary", "target-details"} {
		i := stringAt(value)
		s.Require().NotEqual(-1, i, "%s was not encoded by the user converter", value)
		s.Equal(targetContext, recorded[i].sc, value)
	}
	caller := stringAt("caller-after")
	s.Require().NotEqual(-1, caller)
	s.Equal(converter.WorkflowSerializationContext{
		Namespace: "default-test-namespace", WorkflowID: "default-test-workflow-id",
	}, recorded[caller].sc, "operation must not change the caller's converter")
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
		s.Require().NoError(recorder.DataConverter.FromPayloads(call.GetInput(), &input))
		s.Require().NoError(recorder.DataConverter.FromPayloads(call.GetSignalInput(), &signal))
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
	for _, entry := range recorder.snapshot() {
		switch value := entry.value.(type) {
		case string:
			if expected, ok := want[value]; ok {
				s.Equal(expected, entry.sc, value)
				seen[value]++
			}
		case *workflowservicepb.SignalWithStartWorkflowExecutionRequest:
			s.Fail("user converter received an operation envelope", "target: %s", value.GetWorkflowId())
		}
	}
	for value := range want {
		s.Equal(1, seen[value], "%s must be encoded once in its own context", value)
	}
	s.checkRawEnvelopes(recorder, len(targets))
}
