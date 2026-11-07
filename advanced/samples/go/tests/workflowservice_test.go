package tests

import (
	"context"
	"errors"
	"fmt"
	"sync"
	"testing"
	"time"

	"github.com/stretchr/testify/suite"
	common "go.temporal.io/api/common/v1"
	workflowservicepb "go.temporal.io/api/workflowservice/v1"
	"go.temporal.io/sdk/client"
	"go.temporal.io/sdk/converter"
	"go.temporal.io/sdk/interceptor"
	commonconverter "go.temporal.io/sdk/internal/common/converter"
	"go.temporal.io/sdk/testsuite"
	"go.temporal.io/sdk/worker"
	"go.temporal.io/sdk/workflow"
	"google.golang.org/protobuf/proto"

	ws "go.temporal.io/sdk/advanced/samples/go/workflowservice_facade"
)

func exampleWorkflow(ctx workflow.Context, attempt int, name string) (string, error) {
	var reason string
	workflow.GetSignalChannel(ctx, "wake-up").Receive(ctx, &reason)
	return fmt.Sprintf("example:%d:%s:%s", attempt, name, reason), nil
}

func singleArgWorkflow(ctx workflow.Context, input string) (string, error) {
	var reason *string
	workflow.GetSignalChannel(ctx, "wake-up").Receive(ctx, &reason)
	if reason == nil {
		return "single:" + input + ":nil", nil
	}
	return "single:" + input + ":" + *reason, nil
}

func emptyArgsWorkflow(ctx workflow.Context) (string, error) {
	var reason string
	workflow.GetSignalChannel(ctx, "wake-up").Receive(ctx, &reason)
	return reason, nil
}

func listSignalWorkflow(ctx workflow.Context) ([]string, error) {
	var values []string
	workflow.GetSignalChannel(ctx, "wake-up").Receive(ctx, &values)
	return values, nil
}

type WorkflowServiceIntegrationSuite struct {
	suite.Suite
	server        *testsuite.DevServer
	client        client.Client
	ctx           context.Context
	taskQueue     string
	dataConverter converter.DataConverter
	interceptors  []interceptor.WorkerInterceptor
}

func TestWorkflowServiceIntegrationSuite(t *testing.T) {
	suite.Run(t, &WorkflowServiceIntegrationSuite{})
}

func (s *WorkflowServiceIntegrationSuite) SetupSuite() {
	ctx, cancel := context.WithTimeout(context.Background(), time.Minute)
	defer cancel()
	server, err := testsuite.StartDevServer(ctx, testsuite.DevServerOptions{
		CachedDownload: testsuite.CachedDownload{Version: "v1.7.1-system-nexus-operations"},
		ExtraArgs: []string{
			"--dynamic-config-value", "history.enableSignalWithStartFromWorkflow=true",
		},
	})
	s.Require().NoError(err)
	s.server = server
	s.T().Cleanup(func() { s.NoError(server.Stop()) })
}

func (s *WorkflowServiceIntegrationSuite) SetupTest() {
	ctx, cancel := context.WithTimeout(s.T().Context(), 45*time.Second)
	s.T().Cleanup(cancel)
	s.ctx = ctx
	s.taskQueue = s.T().Name()
	s.dataConverter = converter.GetDefaultDataConverter()
	s.interceptors = nil
}

func (s *WorkflowServiceIntegrationSuite) targetID(name string) string {
	return s.taskQueue + "/" + name
}

func (s *WorkflowServiceIntegrationSuite) runWorkflow(caller any, result any) {
	c, err := client.Dial(client.Options{
		HostPort: s.server.FrontendHostPort(), DataConverter: s.dataConverter,
	})
	s.Require().NoError(err)
	s.client = c
	s.T().Cleanup(c.Close)
	w := worker.New(c, s.taskQueue, worker.Options{Interceptors: s.interceptors})
	w.RegisterWorkflow(exampleWorkflow)
	w.RegisterWorkflow(singleArgWorkflow)
	w.RegisterWorkflow(emptyArgsWorkflow)
	w.RegisterWorkflow(listSignalWorkflow)
	w.RegisterWorkflow(caller)
	s.Require().NoError(w.Start())
	s.T().Cleanup(w.Stop)
	run, err := c.ExecuteWorkflow(s.ctx, client.StartWorkflowOptions{
		ID: s.targetID("caller"), TaskQueue: s.taskQueue,
	}, caller)
	s.Require().NoError(err)
	s.Require().NoError(run.Get(s.ctx, result))
}

func (s *WorkflowServiceIntegrationSuite) TestNoWorkflowArguments() {
	var runID *string
	s.runWorkflow(func(ctx workflow.Context) (*string, error) {
		var response ws.SignalWithStartWorkflowResponse
		err := ws.SignalWithStartWorkflow(ctx,
			ws.SignalWithStartWorkflowOptions{},
			s.targetID("minimal"), "wake-up", "signal-value", emptyArgsWorkflow,
		).Get(ctx, &response)
		return response.RunID, err
	}, &runID)

	s.Require().NotNil(runID)
	s.NotEmpty(*runID)
	var result string
	s.Require().NoError(s.client.GetWorkflow(s.ctx, s.targetID("minimal"), *runID).Get(s.ctx, &result))
	s.Equal("signal-value", result)
}

func (s *WorkflowServiceIntegrationSuite) TestSignalWithStartWorkflowCallForms() {
	var runIDs []*string
	s.runWorkflow(func(ctx workflow.Context) ([]*string, error) {
		var byFunction, byName ws.SignalWithStartWorkflowResponse
		if err := ws.SignalWithStartWorkflow(ctx,
			ws.SignalWithStartWorkflowOptions{TaskQueue: s.taskQueue},
			s.targetID("function"), "wake-up", "wake-up", exampleWorkflow, 7, "nexus",
		).Get(ctx, &byFunction); err != nil {
			return nil, err
		}
		if err := ws.SignalWithStartWorkflow(ctx,
			ws.SignalWithStartWorkflowOptions{TaskQueue: s.taskQueue},
			s.targetID("name"), "wake-up", nil, "singleArgWorkflow", "positional",
		).Get(ctx, &byName); err != nil {
			return nil, err
		}
		return []*string{byFunction.RunID, byName.RunID}, nil
	}, &runIDs)

	s.Require().Len(runIDs, 2)
	for _, runID := range runIDs {
		s.Require().NotNil(runID)
		s.NotEmpty(*runID)
	}
	var functionResult, nameResult string
	s.Require().NoError(s.client.GetWorkflow(s.ctx, s.targetID("function"), *runIDs[0]).Get(s.ctx, &functionResult))
	s.Require().NoError(s.client.GetWorkflow(s.ctx, s.targetID("name"), *runIDs[1]).Get(s.ctx, &nameResult))
	s.Equal("example:7:nexus:wake-up", functionResult)
	s.Equal("single:positional:nil", nameResult)
}

func (s *WorkflowServiceIntegrationSuite) TestListSignalArgument() {
	var runID *string
	s.runWorkflow(func(ctx workflow.Context) (*string, error) {
		var response ws.SignalWithStartWorkflowResponse
		err := ws.SignalWithStartWorkflow(ctx,
			ws.SignalWithStartWorkflowOptions{TaskQueue: s.taskQueue},
			s.targetID("list"), "wake-up", []string{"one", "two"}, listSignalWorkflow,
		).Get(ctx, &response)
		return response.RunID, err
	}, &runID)

	s.Require().NotNil(runID)
	var result []string
	s.Require().NoError(s.client.GetWorkflow(s.ctx, s.targetID("list"), *runID).Get(s.ctx, &result))
	s.Equal([]string{"one", "two"}, result)
}

func (s *WorkflowServiceIntegrationSuite) TestConversionFailureSurfacesOnTheFuture() {
	var result string
	s.runWorkflow(func(ctx workflow.Context) (string, error) {
		err := ws.SignalWithStartWorkflow(ctx,
			ws.SignalWithStartWorkflowOptions{
				TaskQueue: s.taskQueue,
				Memo:      map[string]any{"invalid": func() {}},
			},
			s.targetID("invalid"), "wake-up", "signal-value", singleArgWorkflow, "workflow-input",
		).Get(ctx, nil)
		if err == nil {
			return "", errors.New("expected a payload conversion error")
		}
		return err.Error(), nil
	}, &result)
	s.Contains(result, "unsupported type: func()")
}

// -- Serialization contexts and nexus operation registry --

// recordingDataConverter records outgoing values before they've
// been encoded, along with their serialization context.
type recordingDataConverter struct {
	converter.DataConverter
	sc     converter.SerializationContext
	record func(outgoingValue)
}

type outgoingValue struct {
	sc    converter.SerializationContext
	value any
}

func (c recordingDataConverter) WithSerializationContext(sc converter.SerializationContext) converter.DataConverter {
	c.DataConverter = converter.WithDataConverterSerializationContext(c.DataConverter, sc)
	c.sc = sc
	return c
}

func (c recordingDataConverter) ToPayload(value any) (*common.Payload, error) {
	c.record(outgoingValue{c.sc, value})
	return c.DataConverter.ToPayload(value)
}

func (c recordingDataConverter) ToPayloads(values ...any) (*common.Payloads, error) {
	for _, value := range values {
		c.record(outgoingValue{c.sc, value})
	}
	return c.DataConverter.ToPayloads(values...)
}

// The sample uses the SDK's generic Nexus hook to enter its generated typed
// chain. SDK-internal generation instead attaches methods to SDK-owned receivers.
type captureNexusInputInterceptor struct {
	interceptor.WorkerInterceptorBase
	mu            sync.Mutex
	calls         []interceptor.ExecuteNexusOperationInput
	dispatchCalls int
	typedCalls    int
	onTyped       func(workflow.Context, *ws.SignalWithStartWorkflowRequest)
}

func (i *captureNexusInputInterceptor) InterceptWorkflow(ctx workflow.Context, next interceptor.WorkflowInboundInterceptor) interceptor.WorkflowInboundInterceptor {
	return &captureNexusInputInbound{
		WorkflowInboundInterceptorBase: interceptor.WorkflowInboundInterceptorBase{Next: next},
		capture:                        i,
	}
}

type captureNexusInputInbound struct {
	interceptor.WorkflowInboundInterceptorBase
	capture *captureNexusInputInterceptor
}

func (i *captureNexusInputInbound) Init(next interceptor.WorkflowOutboundInterceptor) error {
	return i.Next.Init(&captureNexusInputOutbound{
		WorkflowOutboundInterceptorBase: interceptor.WorkflowOutboundInterceptorBase{Next: next},
		capture:                         i.capture,
	})
}

type captureNexusInputOutbound struct {
	interceptor.WorkflowOutboundInterceptorBase
	capture *captureNexusInputInterceptor
}

func (i *captureNexusInputOutbound) ExecuteNexusOperation(ctx workflow.Context, input interceptor.ExecuteNexusOperationInput) workflow.NexusOperationFuture {
	if input.Client.Endpoint() != "__temporal_system" {
		return i.Next.ExecuteNexusOperation(ctx, input)
	}
	if !workflow.IsReplaying(ctx) {
		i.capture.mu.Lock()
		i.capture.dispatchCalls++
		i.capture.mu.Unlock()
	}
	terminal := &ws.WorkflowOutboundInterceptorTerminal{
		SystemNexus_WorkflowOutboundRuntime: &workflowServiceRuntime{next: i.Next, capture: i.capture},
	}
	outbound := &captureWorkflowServiceOutbound{
		WorkflowOutboundInterceptorBase: ws.WorkflowOutboundInterceptorBase{
			Next: &ws.WorkflowOutboundInterceptorBase{Next: terminal},
		},
		capture: i.capture,
	}
	operation, ok := input.Operation.(string)
	if !ok {
		return terminal.FailedFuture(ctx, fmt.Errorf("sample runtime expects a string operation name, got %T", input.Operation))
	}
	return terminal.SystemNexus_ExecuteOperation(ctx, outbound, ws.ExecuteOperationInput{
		Service:   input.Client.Service(),
		Operation: operation,
		Input:     input.Input,
	})
}

type captureWorkflowServiceOutbound struct {
	ws.WorkflowOutboundInterceptorBase
	capture *captureNexusInputInterceptor
}

func (i *captureWorkflowServiceOutbound) SignalWithStartWorkflow(ctx workflow.Context, request *ws.SignalWithStartWorkflowRequest) workflow.NexusOperationFuture {
	if !workflow.IsReplaying(ctx) {
		i.capture.mu.Lock()
		i.capture.typedCalls++
		i.capture.mu.Unlock()
	}
	if i.capture.onTyped != nil {
		i.capture.onTyped(ctx, request)
	}
	return i.WorkflowOutboundInterceptorBase.SignalWithStartWorkflow(ctx, request)
}

type workflowServiceRuntime struct {
	next    interceptor.WorkflowOutboundInterceptor
	capture *captureNexusInputInterceptor
}

func (r *workflowServiceRuntime) ExecuteNexusOperation(ctx workflow.Context, input interceptor.ExecuteNexusOperationInput) workflow.NexusOperationFuture {
	if !workflow.IsReplaying(ctx) {
		r.capture.mu.Lock()
		r.capture.calls = append(r.capture.calls, input)
		r.capture.mu.Unlock()
	}
	return r.next.ExecuteNexusOperation(ctx, input)
}

type failedWorkflowServiceFuture struct {
	workflow.Future
	execution workflow.Future
}

func (f *failedWorkflowServiceFuture) GetNexusOperationExecution() workflow.Future {
	return f.execution
}

func (*workflowServiceRuntime) FailedFuture(ctx workflow.Context, err error) workflow.NexusOperationFuture {
	result, resultSettable := workflow.NewFuture(ctx)
	execution, executionSettable := workflow.NewFuture(ctx)
	resultSettable.SetError(err)
	executionSettable.SetError(err)
	return &failedWorkflowServiceFuture{Future: result, execution: execution}
}

func (s *WorkflowServiceIntegrationSuite) TestFlattenedMetadataPresence() {
	for _, tc := range []struct {
		name    string
		summary string
		details string
	}{
		{name: "omitted"},
		{name: "summary only", summary: "summary"},
		{name: "details only", details: "details"},
		{name: "both", summary: "summary", details: "details"},
	} {
		s.Run(tc.name, func() {
			capture := &captureNexusInputInterceptor{}
			s.interceptors = []interceptor.WorkerInterceptor{capture}
			var result string
			s.runWorkflow(func(ctx workflow.Context) (string, error) {
				err := ws.SignalWithStartWorkflow(ctx, ws.SignalWithStartWorkflowOptions{
					TaskQueue:     s.taskQueue,
					StaticSummary: tc.summary,
					StaticDetails: tc.details,
				}, s.targetID("target"), "wake-up", "signal", singleArgWorkflow, "input").Get(ctx, nil)
				return "caller-result", err
			}, &result)
			s.Require().Equal("caller-result", result)
			s.Require().NoError(s.client.GetWorkflow(s.ctx, s.targetID("target"), "").Get(s.ctx, nil))

			capture.mu.Lock()
			calls := append([]interceptor.ExecuteNexusOperationInput(nil), capture.calls...)
			capture.mu.Unlock()
			s.Require().Len(calls, 1)

			// Exercise the generated transfer converter, not just the native
			// request: absent strings must not become encoded empty payloads.
			dc := commonconverter.WithDataConverterSystemNexusContext(
				commonconverter.MakeTransferAware(converter.GetDefaultDataConverter()),
				converter.GetDefaultDataConverter(),
			)
			payload, err := dc.ToPayload(calls[0].Input)
			s.Require().NoError(err)
			var wire workflowservicepb.SignalWithStartWorkflowExecutionRequest
			s.Require().NoError(converter.GetDefaultDataConverter().FromPayload(payload, &wire))
			if tc.summary == "" && tc.details == "" {
				s.Nil(wire.UserMetadata)
				return
			}
			s.Require().NotNil(wire.UserMetadata)
			for _, field := range []struct {
				want    string
				payload *common.Payload
			}{
				{tc.summary, wire.UserMetadata.Summary},
				{tc.details, wire.UserMetadata.Details},
			} {
				if field.want == "" {
					s.Nil(field.payload)
					continue
				}
				s.Require().NotNil(field.payload)
				var value string
				s.Require().NoError(converter.GetDefaultDataConverter().FromPayload(field.payload, &value))
				s.Equal(field.want, value)
			}
		})
	}
}

func (s *WorkflowServiceIntegrationSuite) TestSerializationContextsAndOperationRegistry() {
	// Signalwithstart's arguments will be recorded into outgoingValues, along with
	// their serialization context.
	var mu sync.Mutex
	var outgoingValues []outgoingValue
	s.dataConverter = recordingDataConverter{
		DataConverter: converter.NewCodecDataConverter(converter.GetDefaultDataConverter(),
			converter.NewZlibCodec(converter.ZlibCodecOptions{AlwaysEncode: true})),
		record: func(encoded outgoingValue) {
			mu.Lock()
			defer mu.Unlock()
			outgoingValues = append(outgoingValues, encoded)
		},
	}

	// The computedProtoInterceptor interceptor will record the ExecuteNexusOperationInput proto constructed
	// by the generated SignalWithStartWorkflow function.
	computedProtoInterceptor := &captureNexusInputInterceptor{}
	s.interceptors = []interceptor.WorkerInterceptor{computedProtoInterceptor}

	var callerResult string
	s.runWorkflow(func(ctx workflow.Context) (string, error) {
		err := ws.SignalWithStartWorkflow(ctx,
			ws.SignalWithStartWorkflowOptions{
				TaskQueue: s.taskQueue,
				Memo: map[string]any{
					"memo-key": "target-memo",
				},
				StaticSummary: "target-summary",
				StaticDetails: "target-details",
			},
			s.targetID("target"), "wake-up", "signal-value", singleArgWorkflow, "workflow-input",
		).Get(ctx, nil)
		return "caller-result", err
	}, &callerResult)
	s.Equal("caller-result", callerResult)
	var targetResult string
	s.Require().NoError(s.client.GetWorkflow(s.ctx, s.targetID("target"), "").Get(s.ctx, &targetResult))
	s.Equal("single:workflow-input:signal-value", targetResult)

	// Check the fields of the computed ExecuteNexusOperationInput proto
	computedProtoInterceptor.mu.Lock()
	calls := append([]interceptor.ExecuteNexusOperationInput(nil), computedProtoInterceptor.calls...)
	computedProtoInterceptor.mu.Unlock()
	s.Require().Len(calls, 1)
	call := calls[0]
	s.Equal("__temporal_system", call.Client.Endpoint())
	s.Equal("temporal.api.workflowservice.v1.WorkflowService", call.Client.Service())
	s.Equal("SignalWithStartWorkflowExecution", call.Operation)
	request, ok := call.Input.(interface {
		GetWorkflow() string
		GetArgs() []any
		GetID() string
		GetTaskQueue() string
		GetSignal() string
		GetSignalArgs() []any
		GetNamespace() string
		TransferTypeConverter() (converter.TransferTypeConverter, error)
	})
	s.Require().True(ok, "expected a native transfer-type request")
	s.Equal("singleArgWorkflow", request.GetWorkflow())
	s.Equal([]any{"workflow-input"}, request.GetArgs())
	s.Equal(s.targetID("target"), request.GetID())
	s.Equal(s.taskQueue, request.GetTaskQueue())
	s.Equal("wake-up", request.GetSignal())
	s.Equal([]any{"signal-value"}, request.GetSignalArgs())
	s.Equal("default", request.GetNamespace())
	transferConverter, err := request.TransferTypeConverter()
	s.Require().NoError(err)
	s.NotNil(transferConverter)

	targetContext := converter.WorkflowSerializationContext{Namespace: "default", WorkflowID: s.targetID("target")}

	// Check that the arguments we passed to signalwithstart were given same serialization context
	mu.Lock()
	recorded := append([]outgoingValue(nil), outgoingValues...)
	mu.Unlock()
	want := map[string]converter.SerializationContext{
		"workflow-input": targetContext,
		"signal-value":   targetContext,
		"target-memo":    targetContext,
		"target-summary": targetContext,
		"target-details": targetContext,
		"caller-result": converter.WorkflowSerializationContext{
			Namespace: "default", WorkflowID: s.targetID("caller"),
		},
	}
	seen := make(map[string]bool)
	for _, encoded := range recorded {
		value, ok := encoded.value.(string)
		if expected, found := want[value]; ok && found {
			s.Equal(expected, encoded.sc, value)
			seen[value] = true
		}
	}
	for value := range want {
		s.True(seen[value], "%s was not encoded by the user converter", value)
	}

	dc := commonconverter.WithDataConverterSystemNexusContext(
		commonconverter.MakeTransferAware(converter.GetDefaultDataConverter()),
		converter.GetDefaultDataConverter(),
	)
	headerPayload, err := converter.GetDefaultDataConverter().ToPayload("parent")
	s.Require().NoError(err)
	wire := workflowservicepb.SignalWithStartWorkflowExecutionRequest{
		WorkflowId: s.targetID("target"),
		Header: &common.Header{
			Fields: map[string]*common.Payload{"trace": headerPayload},
		},
	}
	payload, err := converter.GetDefaultDataConverter().ToPayload(&wire)
	s.Require().NoError(err)
	var decoded ws.SignalWithStartWorkflowRequest
	s.Require().NoError(dc.FromPayload(payload, &decoded))
	s.Require().True(proto.Equal(headerPayload, decoded.Headers["trace"]), "workflow header changed during payload conversion")
	interceptedPayload, err := converter.GetDefaultDataConverter().ToPayload("intercepted")
	s.Require().NoError(err)
	decoded.Headers["trace"] = interceptedPayload
	payload, err = dc.ToPayload(&decoded)
	s.Require().NoError(err)
	var encoded workflowservicepb.SignalWithStartWorkflowExecutionRequest
	s.Require().NoError(converter.GetDefaultDataConverter().FromPayload(payload, &encoded))
	var header string
	s.Require().NoError(converter.GetDefaultDataConverter().FromPayload(encoded.Header.Fields["trace"], &header))
	s.Equal("intercepted", header)
}
