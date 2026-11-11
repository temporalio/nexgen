package tests

import (
	"errors"
	"reflect"
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
	var execution workflow.Execution
	env.ExecuteWorkflow(func(ctx workflow.Context) error {
		future, settable := workflow.NewFuture(ctx)
		settable.Set(workflowservicepb.SignalWithStartWorkflowExecutionResponse{RunId: runID}, nil)
		runtime.future = &sampleNexusFuture{Future: future}
		return implementation.SystemNexus_WorkflowOutboundInterceptorBase_SignalWithStartWorkflow(outbound, ctx,
			ws.SignalWithStartWorkflowOptions{
				TaskQueue:             "target-queue",
				WorkflowIDReusePolicy: enums.WORKFLOW_ID_REUSE_POLICY_ALLOW_DUPLICATE,
				WorkflowTaskTimeout:   5 * time.Second,
				StartDelay:            time.Hour,
			},
			"original", "wake-up", "signal-value", "singleArgWorkflow", "workflow-input",
		).Get(ctx, &execution)
	})
	require.NoError(t, env.GetWorkflowError())
	require.Equal(t, workflow.Execution{ID: "intercepted", RunID: runID}, execution)
	require.Equal(t, 1, calls)
	require.Len(t, runtime.inputs, 1)
	input := runtime.inputs[0]
	require.Equal(t, "temporal.api.workflowservice.v1.WorkflowService", input.Client.Service())
	require.Equal(t, "SignalWithStartWorkflowExecution", input.Operation)
	require.Equal(t, workflow.NexusOperationOptions{}, input.Options)
	require.Empty(t, input.NexusHeader)

	dc := commonconverter.WithDataConverterSystemNexusContext(
		commonconverter.MakeTransferAware(converter.GetDefaultDataConverter()),
		converter.GetDefaultDataConverter(),
	)
	payload, err := dc.ToPayload(input.Input)
	require.NoError(t, err)
	var wire workflowservicepb.SignalWithStartWorkflowExecutionRequest
	require.NoError(t, converter.GetDefaultDataConverter().FromPayload(payload, &wire))
	require.Equal(t, "intercepted", wire.WorkflowId)
	var signalArg string
	require.NoError(t, converter.GetDefaultDataConverter().FromPayloads(wire.SignalInput, &signalArg))
	require.Equal(t, "intercepted-signal", signalArg)
	require.Equal(t, 10*time.Second, wire.WorkflowTaskTimeout.AsDuration())
	require.Nil(t, wire.WorkflowStartDelay)
	require.Nil(t, wire.Header)
	require.NotNil(t, wire.UserMetadata)
	var summary string
	require.NoError(t, converter.GetDefaultDataConverter().FromPayload(wire.UserMetadata.Summary, &summary))
	require.Equal(t, "from interceptor", summary)
}

func workflowServiceRequest(t *testing.T, opts ws.SignalWithStartWorkflowOptions) any {
	t.Helper()
	var suite testsuite.WorkflowTestSuite
	env := suite.NewTestWorkflowEnvironment()
	runtime := &recordingWorkflowServiceRuntime{}
	env.ExecuteWorkflow(func(ctx workflow.Context) error {
		future, settable := workflow.NewFuture(ctx)
		settable.Set(workflowservicepb.SignalWithStartWorkflowExecutionResponse{}, nil)
		runtime.future = &sampleNexusFuture{Future: future}
		return implementation.SystemNexus_WorkflowEnvironmentInterceptor_SignalWithStartWorkflow(
			runtime.ExecuteNexusOperation, ctx, opts, "target", "wake-up", nil, "singleArgWorkflow",
		).Get(ctx, nil)
	})
	require.NoError(t, env.GetWorkflowError())
	require.Len(t, runtime.inputs, 1)
	return runtime.inputs[0].Input
}

func TestWorkflowServiceRequestDefaultPunning(t *testing.T) {
	dc := commonconverter.WithDataConverterSystemNexusContext(
		commonconverter.MakeTransferAware(converter.GetDefaultDataConverter()),
		converter.GetDefaultDataConverter(),
	)
	for _, tc := range []struct {
		name    string
		options ws.SignalWithStartWorkflowOptions
	}{
		{name: "omitted"},
		{
			name: "nonzero",
			options: ws.SignalWithStartWorkflowOptions{
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
			options: ws.SignalWithStartWorkflowOptions{
				RetryPolicy:   &temporal.RetryPolicy{},
				Priority:      &temporal.Priority{PriorityKey: 1},
				StaticSummary: "summary",
			},
		},
	} {
		t.Run(tc.name, func(t *testing.T) {
			request := workflowServiceRequest(t, tc.options)
			payload, err := dc.ToPayload(request)
			require.NoError(t, err)
			var wire workflowservicepb.SignalWithStartWorkflowExecutionRequest
			require.NoError(t, converter.GetDefaultDataConverter().FromPayload(payload, &wire))
			for _, duration := range []struct {
				value time.Duration
				wire  *durationpb.Duration
			}{
				{tc.options.WorkflowExecutionTimeout, wire.WorkflowExecutionTimeout},
				{tc.options.WorkflowRunTimeout, wire.WorkflowRunTimeout},
				{tc.options.WorkflowTaskTimeout, wire.WorkflowTaskTimeout},
				{tc.options.StartDelay, wire.WorkflowStartDelay},
			} {
				if duration.value == 0 {
					require.Nil(t, duration.wire)
				} else {
					require.NotNil(t, duration.wire)
					require.Equal(t, duration.value, duration.wire.AsDuration())
				}
			}
			require.Equal(t, tc.options.WorkflowIDConflictPolicy, wire.WorkflowIdConflictPolicy)
			require.Equal(t, tc.options.CronSchedule, wire.CronSchedule)
			require.Equal(t, tc.options.RetryPolicy == nil, wire.RetryPolicy == nil)
			require.Equal(t, tc.options.Priority == nil, wire.Priority == nil)
			require.Equal(t, tc.options.StaticSummary == "", wire.UserMetadata == nil)
			if tc.options.StaticSummary != "" {
				require.NotNil(t, wire.UserMetadata.Summary)
			}

			// Decode into the private envelope type captured from the terminal.
			roundTrip := reflect.New(reflect.TypeOf(request))
			require.NoError(t, dc.FromPayload(payload, roundTrip.Interface()))
			require.Equal(t, request, roundTrip.Elem().Interface())
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
		request := reflect.New(reflect.TypeOf(workflowServiceRequest(t, ws.SignalWithStartWorkflowOptions{}))).Interface()
		require.NoError(t, dc.FromPayload(payload, request))

		payload, err = dc.ToPayload(request)
		require.NoError(t, err)
		require.NoError(t, converter.GetDefaultDataConverter().FromPayload(payload, wire))
		require.Nil(t, wire.WorkflowExecutionTimeout)
		require.Nil(t, wire.WorkflowRunTimeout)
		require.Nil(t, wire.WorkflowTaskTimeout)
		require.Nil(t, wire.WorkflowStartDelay)
		require.Zero(t, wire.WorkflowIdReusePolicy, "decoding must not evaluate wrapper defaults")
		require.Empty(t, wire.GetTaskQueue().GetName(), "decoding must not evaluate wrapper defaults")
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
					settable.Set(workflow.Execution{ID: "intercepted", RunID: "intercepted-run"}, nil)
				}
				interceptedFuture = future
				return future
			})
			env.ExecuteWorkflow(func(ctx workflow.Context) (workflow.Execution, error) {
				var execution workflow.Execution
				future := implementation.SystemNexus_WorkflowOutboundInterceptorBase_SignalWithStartWorkflow(outbound, ctx, tc.options, "original", "wake-up", "signal", "registered-target", "input", 42)
				require.Same(t, interceptedFuture, future)
				err := future.Get(ctx, &execution)
				return execution, err
			})
			require.Equal(t, 1, calls)
			if tc.fail {
				require.ErrorContains(t, env.GetWorkflowError(), "interceptor rejected the call")
				return
			}
			require.NoError(t, env.GetWorkflowError())
			var execution workflow.Execution
			require.NoError(t, env.GetWorkflowResult(&execution))
			require.Equal(t, workflow.Execution{ID: "intercepted", RunID: "intercepted-run"}, execution)
		})
	}
}
