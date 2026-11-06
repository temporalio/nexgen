package tests

import (
	"errors"
	"testing"
	"time"

	"github.com/stretchr/testify/require"
	enums "go.temporal.io/api/enums/v1"
	workflowservicepb "go.temporal.io/api/workflowservice/v1"
	"go.temporal.io/sdk/converter"
	"go.temporal.io/sdk/interceptor"
	commonconverter "go.temporal.io/sdk/internal/common/converter"
	"go.temporal.io/sdk/temporal"
	"go.temporal.io/sdk/testsuite"
	"go.temporal.io/sdk/workflow"
	"google.golang.org/protobuf/types/known/durationpb"

	implementation "go.temporal.io/sdk/advanced/samples/go/workflowservice"
	ws "go.temporal.io/sdk/advanced/samples/go/workflowservice_facade"
)

type workflowServiceOutbound func(workflow.Context, ws.SignalWithStartWorkflowOptions, string, string, any, string, ...any) workflow.Future

func (i workflowServiceOutbound) SignalWithStartWorkflow(
	ctx workflow.Context,
	opts ws.SignalWithStartWorkflowOptions,
	id string,
	signal string,
	signalArg any,
	workflowType string,
	args ...any,
) workflow.Future {
	return i(ctx, opts, id, signal, signalArg, workflowType, args...)
}

type recordingWorkflowServiceRuntime struct {
	inputs []interceptor.ExecuteNexusOperationInput
	future workflow.NexusOperationFuture
}

func (r *recordingWorkflowServiceRuntime) ExecuteNexusOperation(_ workflow.Context, input interceptor.ExecuteNexusOperationInput) workflow.NexusOperationFuture {
	r.inputs = append(r.inputs, input)
	return r.future
}

func TestWorkflowServiceTypedInterceptors(t *testing.T) {
	var suite testsuite.WorkflowTestSuite
	env := suite.NewTestWorkflowEnvironment()
	runtime := &recordingWorkflowServiceRuntime{}
	terminal := workflowServiceOutbound(func(ctx workflow.Context, opts ws.SignalWithStartWorkflowOptions, id string, signal string, signalArg any, workflowType string, args ...any) workflow.Future {
		return implementation.SystemNexus_WorkflowEnvironmentInterceptor_SignalWithStartWorkflow(runtime.ExecuteNexusOperation, ctx, opts, id, signal, signalArg, workflowType, args...)
	})
	calls := 0
	outbound := workflowServiceOutbound(func(ctx workflow.Context, opts ws.SignalWithStartWorkflowOptions, id string, signal string, signalArg any, workflowType string, args ...any) workflow.Future {
		calls++
		require.Equal(t, "original", id)
		require.Equal(t, "singleArgWorkflow", workflowType)
		require.Equal(t, []any{"workflow-input"}, args)
		require.Equal(t, "wake-up", signal)
		require.Equal(t, "signal-value", signalArg)
		require.Equal(t, "target-queue", opts.TaskQueue)
		require.Equal(t, enums.WORKFLOW_ID_REUSE_POLICY_ALLOW_DUPLICATE, opts.WorkflowIDReusePolicy)
		require.Equal(t, 5*time.Second, opts.WorkflowTaskTimeout)
		require.Equal(t, time.Hour, opts.StartDelay)
		require.Empty(t, runtime.inputs, "the terminal must not schedule the request before interception")
		opts.WorkflowTaskTimeout = 10 * time.Second
		opts.StartDelay = 0
		opts.StaticSummary = "from interceptor"
		return implementation.SystemNexus_WorkflowOutboundInterceptorBase_SignalWithStartWorkflow(terminal, ctx, opts, "intercepted", signal, "intercepted-signal", workflowType, args...)
	})
	runID := "target-run"
	var response ws.SignalWithStartWorkflowResponse
	env.ExecuteWorkflow(func(ctx workflow.Context) error {
		future, settable := workflow.NewFuture(ctx)
		settable.Set(ws.SignalWithStartWorkflowResponse{RunID: &runID}, nil)
		runtime.future = &sampleNexusFuture{Future: future}
		return implementation.SystemNexus_WorkflowOutboundInterceptorBase_SignalWithStartWorkflow(outbound, ctx,
			ws.SignalWithStartWorkflowOptions{
				TaskQueue:             "target-queue",
				WorkflowIDReusePolicy: enums.WORKFLOW_ID_REUSE_POLICY_ALLOW_DUPLICATE,
				WorkflowTaskTimeout:   5 * time.Second,
				StartDelay:            time.Hour,
			},
			"original", "wake-up", "signal-value", "singleArgWorkflow", "workflow-input",
		).Get(ctx, &response)
	})
	require.NoError(t, env.GetWorkflowError())
	require.Equal(t, &runID, response.RunID)
	require.Equal(t, 1, calls)
	require.Len(t, runtime.inputs, 1)
	input := runtime.inputs[0]
	require.Equal(t, "temporal.api.workflowservice.v1.WorkflowService", input.Client.Service())
	require.Equal(t, "SignalWithStartWorkflowExecution", input.Operation)
	require.Equal(t, workflow.NexusOperationOptions{}, input.Options)
	require.Empty(t, input.NexusHeader)
	request := input.Input.(implementation.SystemNexus_SignalWithStartWorkflowRequest)
	require.Equal(t, "intercepted", request.ID)
	require.Equal(t, []any{"intercepted-signal"}, request.SignalArgs)

	dc := commonconverter.WithDataConverterSystemNexusContext(
		commonconverter.MakeTransferAware(converter.GetDefaultDataConverter()),
		converter.GetDefaultDataConverter(),
	)
	payload, err := dc.ToPayload(input.Input)
	require.NoError(t, err)
	var wire workflowservicepb.SignalWithStartWorkflowExecutionRequest
	require.NoError(t, converter.GetDefaultDataConverter().FromPayload(payload, &wire))
	require.Equal(t, "intercepted", wire.WorkflowId)
	require.Equal(t, 10*time.Second, wire.WorkflowTaskTimeout.AsDuration())
	require.Nil(t, wire.WorkflowStartDelay)
	require.Nil(t, wire.Header)
	require.NotNil(t, wire.UserMetadata)
	var summary string
	require.NoError(t, converter.GetDefaultDataConverter().FromPayload(wire.UserMetadata.Summary, &summary))
	require.Equal(t, "from interceptor", summary)
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

func TestWorkflowServiceInterceptorShortCircuit(t *testing.T) {
	for _, tc := range []struct {
		name    string
		options ws.SignalWithStartWorkflowOptions
		fail    bool
	}{
		{
			name: "zero options",
		},
		{
			name: "explicit options",
			options: ws.SignalWithStartWorkflowOptions{
				TaskQueue:             "target-queue",
				WorkflowIDReusePolicy: enums.WORKFLOW_ID_REUSE_POLICY_REJECT_DUPLICATE,
			},
		},
		{
			name: "error",
			fail: true,
		},
	} {
		t.Run(tc.name, func(t *testing.T) {
			var suite testsuite.WorkflowTestSuite
			env := suite.NewTestWorkflowEnvironment()
			calls := 0
			var interceptedFuture workflow.Future
			outbound := workflowServiceOutbound(func(ctx workflow.Context, opts ws.SignalWithStartWorkflowOptions, id string, signal string, signalArg any, workflowType string, args ...any) workflow.Future {
				calls++
				require.Equal(t, tc.options, opts)
				require.Equal(t, "original", id)
				require.Equal(t, "wake-up", signal)
				require.Equal(t, "signal", signalArg)
				require.Equal(t, "registered-target", workflowType)
				require.Equal(t, []any{"input", 42}, args)
				future, settable := workflow.NewFuture(ctx)
				if tc.fail {
					settable.SetError(errors.New("interceptor rejected the call"))
				} else {
					runID := "intercepted-run"
					settable.Set(ws.SignalWithStartWorkflowResponse{RunID: &runID}, nil)
				}
				interceptedFuture = future
				return future
			})
			env.ExecuteWorkflow(func(ctx workflow.Context) (string, error) {
				var response ws.SignalWithStartWorkflowResponse
				future := implementation.SystemNexus_WorkflowOutboundInterceptorBase_SignalWithStartWorkflow(outbound, ctx, tc.options, "original", "wake-up", "signal", "registered-target", "input", 42)
				require.Same(t, interceptedFuture, future)
				err := future.Get(ctx, &response)
				if err != nil {
					return "", err
				}
				require.NotNil(t, response.RunID)
				return *response.RunID, nil
			})
			require.Equal(t, 1, calls)
			if tc.fail {
				require.ErrorContains(t, env.GetWorkflowError(), "interceptor rejected the call")
				return
			}
			require.NoError(t, env.GetWorkflowError())
			var runID string
			require.NoError(t, env.GetWorkflowResult(&runID))
			require.Equal(t, "intercepted-run", runID)
		})
	}
}
