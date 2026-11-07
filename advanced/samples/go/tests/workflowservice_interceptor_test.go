package tests

import (
	"testing"
	"time"

	"github.com/stretchr/testify/require"
	common "go.temporal.io/api/common/v1"
	enums "go.temporal.io/api/enums/v1"
	workflowservicepb "go.temporal.io/api/workflowservice/v1"
	"go.temporal.io/sdk/converter"
	"go.temporal.io/sdk/interceptor"
	commonconverter "go.temporal.io/sdk/internal/common/converter"
	"go.temporal.io/sdk/temporal"
	"go.temporal.io/sdk/testsuite"
	"go.temporal.io/sdk/workflow"
	"google.golang.org/protobuf/proto"
	"google.golang.org/protobuf/types/known/durationpb"

	ws "go.temporal.io/sdk/advanced/samples/go/workflowservice_facade"
)

func (s *WorkflowServiceIntegrationSuite) TestTypedInterceptors() {
	header, err := converter.GetDefaultDataConverter().ToPayload("intercepted-header")
	s.Require().NoError(err)
	capture := &captureNexusInputInterceptor{
		onTyped: func(ctx workflow.Context, request *ws.SignalWithStartWorkflowRequest) {
			s.Equal("singleArgWorkflow", request.Workflow)
			s.Equal([]any{"workflow-input"}, request.Args)
			s.Equal([]any{"signal-value"}, request.SignalArgs)
			s.Nil(request.Headers)
			s.Equal(5*time.Second, request.WorkflowTaskTimeout)
			s.Equal(time.Hour, request.StartDelay)
			request.ID = s.targetID("intercepted")
			request.SignalArgs = []any{"intercepted-signal"}
			request.Headers = map[string]*common.Payload{"trace": header}
			request.WorkflowTaskTimeout = 10 * time.Second
			request.StartDelay = 0
		},
	}
	s.interceptors = []interceptor.WorkerInterceptor{capture}
	var result string
	s.runWorkflow(func(ctx workflow.Context) (string, error) {
		err := ws.SignalWithStartWorkflow(ctx,
			ws.SignalWithStartWorkflowOptions{
				WorkflowTaskTimeout: 5 * time.Second, StartDelay: time.Hour,
			},
			s.targetID("original"), "wake-up", "signal-value", singleArgWorkflow, "workflow-input",
		).Get(ctx, nil)
		return "caller-result", err
	}, &result)
	s.Equal("caller-result", result)
	var targetResult string
	s.Require().NoError(s.client.GetWorkflow(s.ctx, s.targetID("intercepted"), "").Get(s.ctx, &targetResult))
	s.Equal("single:workflow-input:intercepted-signal", targetResult)

	capture.mu.Lock()
	calls := append([]interceptor.ExecuteNexusOperationInput(nil), capture.calls...)
	dispatchCalls, typedCalls := capture.dispatchCalls, capture.typedCalls
	capture.mu.Unlock()
	s.Equal(1, dispatchCalls)
	s.Equal(1, typedCalls)
	s.Require().Len(calls, 1)
	s.Equal("temporal.api.workflowservice.v1.WorkflowService", calls[0].Client.Service())
	s.Equal("SignalWithStartWorkflowExecution", calls[0].Operation)
	s.Equal(workflow.NexusOperationOptions{}, calls[0].Options)
	s.Empty(calls[0].NexusHeader)

	dc := commonconverter.WithDataConverterSystemNexusContext(
		commonconverter.MakeTransferAware(converter.GetDefaultDataConverter()),
		converter.GetDefaultDataConverter(),
	)
	payload, err := dc.ToPayload(calls[0].Input)
	s.Require().NoError(err)
	var wire workflowservicepb.SignalWithStartWorkflowExecutionRequest
	s.Require().NoError(converter.GetDefaultDataConverter().FromPayload(payload, &wire))
	s.Equal(s.targetID("intercepted"), wire.WorkflowId)
	s.Equal(10*time.Second, wire.WorkflowTaskTimeout.AsDuration())
	s.Nil(wire.WorkflowStartDelay)
	s.Require().NotNil(wire.Header)
	s.True(proto.Equal(header, wire.Header.Fields["trace"]), "workflow header changed during payload conversion")
}

func TestWorkflowServiceRequestDefaultPunning(t *testing.T) {
	dc := commonconverter.WithDataConverterSystemNexusContext(
		commonconverter.MakeTransferAware(converter.GetDefaultDataConverter()),
		converter.GetDefaultDataConverter(),
	)
	empty := ""
	for _, tc := range []struct {
		name    string
		request ws.SignalWithStartWorkflowRequest
	}{
		{name: "omitted"},
		{
			name: "nonzero",
			request: ws.SignalWithStartWorkflowRequest{
				WorkflowExecutionTimeout: time.Hour,
				WorkflowRunTimeout:       time.Minute,
				WorkflowTaskTimeout:      10 * time.Second,
				WorkflowIDConflictPolicy: enums.WORKFLOW_ID_CONFLICT_POLICY_USE_EXISTING,
				CronSchedule:             "0 * * * *",
				StartDelay:               time.Second,
			},
		},
		{
			name: "nested presence",
			request: ws.SignalWithStartWorkflowRequest{
				RetryPolicy:  &temporal.RetryPolicy{},
				Priority:     &temporal.Priority{PriorityKey: 1},
				UserMetadata: &ws.UserMetadata{StaticSummary: &empty},
			},
		},
	} {
		t.Run(tc.name, func(t *testing.T) {
			payload, err := dc.ToPayload(tc.request)
			require.NoError(t, err)
			var wire workflowservicepb.SignalWithStartWorkflowExecutionRequest
			require.NoError(t, converter.GetDefaultDataConverter().FromPayload(payload, &wire))
			for _, duration := range []struct {
				value time.Duration
				wire  *durationpb.Duration
			}{
				{tc.request.WorkflowExecutionTimeout, wire.WorkflowExecutionTimeout},
				{tc.request.WorkflowRunTimeout, wire.WorkflowRunTimeout},
				{tc.request.WorkflowTaskTimeout, wire.WorkflowTaskTimeout},
				{tc.request.StartDelay, wire.WorkflowStartDelay},
			} {
				if duration.value == 0 {
					require.Nil(t, duration.wire)
				} else {
					require.NotNil(t, duration.wire)
					require.Equal(t, duration.value, duration.wire.AsDuration())
				}
			}
			require.Equal(t, tc.request.WorkflowIDConflictPolicy, wire.WorkflowIdConflictPolicy)
			require.Equal(t, tc.request.CronSchedule, wire.CronSchedule)
			require.Equal(t, tc.request.RetryPolicy == nil, wire.RetryPolicy == nil)
			require.Equal(t, tc.request.Priority == nil, wire.Priority == nil)
			require.Equal(t, tc.request.UserMetadata == nil, wire.UserMetadata == nil)
			if tc.request.UserMetadata != nil {
				require.NotNil(t, wire.UserMetadata.Summary)
			}

			var roundTrip ws.SignalWithStartWorkflowRequest
			require.NoError(t, dc.FromPayload(payload, &roundTrip))
			require.Equal(t, tc.request, roundTrip)
		})
	}

	t.Run("present zero decodes to omission", func(t *testing.T) {
		wire := &workflowservicepb.SignalWithStartWorkflowExecutionRequest{
			WorkflowExecutionTimeout: durationpb.New(0),
			WorkflowRunTimeout:       durationpb.New(0),
			WorkflowTaskTimeout:      durationpb.New(0),
			WorkflowStartDelay:       durationpb.New(0),
		}
		payload, err := converter.GetDefaultDataConverter().ToPayload(wire)
		require.NoError(t, err)
		var request ws.SignalWithStartWorkflowRequest
		require.NoError(t, dc.FromPayload(payload, &request))
		require.Zero(t, request.WorkflowExecutionTimeout)
		require.Zero(t, request.WorkflowRunTimeout)
		require.Zero(t, request.WorkflowTaskTimeout)
		require.Zero(t, request.StartDelay)
		require.Zero(t, request.WorkflowIDReusePolicy, "decoding must not evaluate wrapper defaults")
		require.Empty(t, request.TaskQueue, "decoding must not evaluate wrapper defaults")

		payload, err = dc.ToPayload(request)
		require.NoError(t, err)
		require.NoError(t, converter.GetDefaultDataConverter().FromPayload(payload, wire))
		require.Nil(t, wire.WorkflowExecutionTimeout)
		require.Nil(t, wire.WorkflowRunTimeout)
		require.Nil(t, wire.WorkflowTaskTimeout)
		require.Nil(t, wire.WorkflowStartDelay)
	})
}

type recordingWorkflowServiceRuntime struct {
	workflowServiceRuntime
	input  interceptor.ExecuteNexusOperationInput
	future workflow.NexusOperationFuture
}

func (r *recordingWorkflowServiceRuntime) ExecuteNexusOperation(_ workflow.Context, input interceptor.ExecuteNexusOperationInput) workflow.NexusOperationFuture {
	r.input = input
	return r.future
}

func TestWorkflowServiceInterceptorForwarding(t *testing.T) {
	future := &failedWorkflowServiceFuture{}
	runtime := &recordingWorkflowServiceRuntime{future: future}
	terminal := &ws.WorkflowOutboundInterceptorTerminal{SystemNexus_WorkflowOutboundRuntime: runtime}
	base := &ws.WorkflowOutboundInterceptorBase{Next: terminal}
	request := &ws.SignalWithStartWorkflowRequest{
		ID: "target", Args: []any{"unencoded"}, SignalArgs: []any{42},
	}
	require.Same(t, future, base.SignalWithStartWorkflow(nil, request))
	require.Equal(t, "temporal.api.workflowservice.v1.WorkflowService", runtime.input.Client.Service())
	require.Equal(t, "SignalWithStartWorkflowExecution", runtime.input.Operation)
	require.Equal(t, *request, runtime.input.Input)
	require.Equal(t, workflow.NexusOperationOptions{}, runtime.input.Options)
	require.NotNil(t, runtime.input.NexusHeader)
	require.Empty(t, runtime.input.NexusHeader)
}

func TestWorkflowServiceInterceptorDispatchErrors(t *testing.T) {
	for _, tc := range []struct {
		name      string
		service   string
		operation string
		input     any
		want      string
	}{
		{
			name: "unknown service", service: "unknown", operation: "SignalWithStartWorkflowExecution",
			want: "unsupported system Nexus operation unknown/SignalWithStartWorkflowExecution",
		},
		{
			name: "unknown operation", service: "temporal.api.workflowservice.v1.WorkflowService", operation: "unknown",
			want: "unsupported system Nexus operation temporal.api.workflowservice.v1.WorkflowService/unknown",
		},
		{
			name: "wrong request", service: "temporal.api.workflowservice.v1.WorkflowService", operation: "SignalWithStartWorkflowExecution",
			input: "not a request", want: "expects SystemNexus_SignalWithStartWorkflowRequest, got string",
		},
	} {
		t.Run(tc.name, func(t *testing.T) {
			var suite testsuite.WorkflowTestSuite
			env := suite.NewTestWorkflowEnvironment()
			env.ExecuteWorkflow(func(ctx workflow.Context) ([]string, error) {
				terminal := &ws.WorkflowOutboundInterceptorTerminal{
					SystemNexus_WorkflowOutboundRuntime: &workflowServiceRuntime{},
				}
				future := terminal.SystemNexus_ExecuteOperation(ctx, nil, ws.ExecuteOperationInput{
					Service:   tc.service,
					Operation: tc.operation, Input: tc.input,
				})
				resultErr := future.Get(ctx, nil)
				executionErr := future.GetNexusOperationExecution().Get(ctx, nil)
				if resultErr == nil || executionErr == nil {
					return nil, nil
				}
				return []string{resultErr.Error(), executionErr.Error()}, nil
			})
			require.NoError(t, env.GetWorkflowError())
			var errors []string
			require.NoError(t, env.GetWorkflowResult(&errors))
			require.Len(t, errors, 2)
			for _, err := range errors {
				require.Contains(t, err, tc.want)
			}
		})
	}
}
