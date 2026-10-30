package tests

import (
	"context"
	"testing"
	"time"

	"github.com/nexus-rpc/sdk-go/nexus"
	"github.com/stretchr/testify/require"
	"go.temporal.io/api/common/v1"
	"go.temporal.io/api/enums/v1"
	namespacepb "go.temporal.io/api/namespace/v1"
	versionpb "go.temporal.io/api/version/v1"
	workflowservicepb "go.temporal.io/api/workflowservice/v1"
	fe "go.temporal.io/sdk/advanced/samples/go/functionexecution"
	gm "go.temporal.io/sdk/advanced/samples/go/genericmodels"
	tr "go.temporal.io/sdk/advanced/samples/go/protobackedshowcase"
	sw "go.temporal.io/sdk/advanced/samples/go/startworkflow"
	"go.temporal.io/sdk/converter"
	"go.temporal.io/sdk/testsuite"
	"go.temporal.io/sdk/workflow"
)

func TestProtoBackedEnumAndBytes(t *testing.T) {
	dc := converter.GetDefaultDataConverter()
	for _, level := range []tr.AlertLevel{
		tr.AlertLevelSeverityUnspecified, tr.AlertLevelSeverityHigh,
		tr.AlertLevelSeverityMedium, tr.AlertLevelSeverityLow,
	} {
		model := tr.Alert{Message: "alert", Severity: &level}
		payload, err := dc.ToPayload(model)
		require.NoError(t, err)
		var wire versionpb.Alert
		require.NoError(t, dc.FromPayload(payload, &wire))
		require.Equal(t, enums.Severity(level), wire.Severity)
		var decoded tr.Alert
		require.NoError(t, dc.FromPayload(payload, &decoded))
		require.Equal(t, model, decoded)
	}
	payload, err := dc.ToPayload(tr.Alert{Message: "optional"})
	require.NoError(t, err)
	var wire versionpb.Alert
	require.NoError(t, dc.FromPayload(payload, &wire))
	require.Equal(t, enums.SEVERITY_UNSPECIFIED, wire.Severity)

	for _, data := range [][]byte{nil, {0, 1, 127, 128, 255}} {
		payload, err := dc.ToPayload(tr.CallbackInternal{Data: data})
		require.NoError(t, err)
		var wire common.Callback_Internal
		require.NoError(t, dc.FromPayload(payload, &wire))
		require.Equal(t, data, wire.Data)
		var decoded tr.CallbackInternal
		require.NoError(t, dc.FromPayload(payload, &decoded))
		require.Equal(t, data, decoded.Data)
	}
}

func TestGenericValueReceiverGetters(t *testing.T) {
	// Elapsed's direct field override must supply the generated package's
	// time import; no type alias in the sample supplies it indirectly.
	other := "other"
	model := gm.GetterProbe[string]{
		Value: "value", Other: &other, History: []string{"past"},
		TupleValue: gm.Tuple2[string, string]{First: "first", Second: "second"},
		Lookup:     map[string]int32{"key": 7}, Elapsed: time.Second,
		Namespace: "namespace", Type: "type",
	}
	model.ResultValue.Result = "result"
	model.ResultValue.Error = "error"
	// The value, not just *GetterProbe, must implement the getter surface.
	var value interface {
		GetValue() string
		GetOther() *string
		GetLookup() map[string]int32
		GetElapsed() time.Duration
	} = model
	require.Equal(t, "value", value.GetValue())
	require.Equal(t, &other, value.GetOther())
	require.Equal(t, map[string]int32{"key": 7}, value.GetLookup())
	require.Equal(t, time.Second, value.GetElapsed())
	require.Equal(t, []string{"past"}, model.GetHistory())
	require.Equal(t, "first", model.GetTupleValue().GetFirst())
	require.Equal(t, "second", model.GetTupleValue().GetSecond())
	require.Equal(t, "result", model.GetResultValue().GetResult())
	require.Equal(t, "error", model.GetResultValue().GetError())
	require.Equal(t, "namespace", model.GetNamespace())
	require.Equal(t, "type", model.GetType())

	pair := gm.GetterPair[string, int]{
		Value: "value", History: []string{"past"},
		TupleValue: gm.Tuple2[string, int]{First: "first", Second: 42},
	}
	pair.ResultValue.Result = "result"
	pair.ResultValue.Error = 7
	require.Equal(t, "value", pair.GetValue())
	require.Nil(t, pair.GetOther())
	require.Equal(t, []string{"past"}, pair.GetHistory())
	require.Equal(t, "first", pair.GetTupleValue().GetFirst())
	require.Equal(t, 42, pair.GetTupleValue().GetSecond())
	require.Equal(t, 7, pair.GetResultValue().GetError())
}

func TestSourcedMapReachesTheWire(t *testing.T) {
	var suite testsuite.WorkflowTestSuite
	env := suite.NewTestWorkflowEnvironment()
	service := nexus.NewService(protoBackedShowcaseServiceName)
	var data map[string]string
	require.NoError(t, service.Register(nexus.NewSyncOperation("NamespaceInfoOperation",
		func(_ context.Context, input *namespacepb.NamespaceInfo, _ nexus.StartOperationOptions) (any, error) {
			data = input.Data
			require.Equal(t, enums.NAMESPACE_STATE_REGISTERED, input.State)
			return nil, nil
		})))
	env.RegisterNexusService(service)
	env.ExecuteWorkflow(func(ctx workflow.Context) error {
		return tr.NamespaceInfoOperation(ctx, tr.NamespaceInfoOperationOptions{Name: "sample"}).Get(ctx, nil)
	})
	require.NoError(t, env.GetWorkflowError())
	require.Equal(t, map[string]string{"namespace": "default-test-namespace"}, data)
}

func TestOptionalScalarResourceConstruction(t *testing.T) {
	for _, started := range []bool{false, true} {
		var suite testsuite.WorkflowTestSuite
		env := suite.NewTestWorkflowEnvironment()
		service := nexus.NewService(startWorkflowServiceName)
		require.NoError(t, service.Register(nexus.NewSyncOperation("SignalWithStart",
			func(_ context.Context, input *workflowservicepb.RequestCancelWorkflowExecutionRequest, _ nexus.StartOperationOptions) (*workflowservicepb.SignalWithStartWorkflowExecutionResponse, error) {
				require.Equal(t, "default-test-namespace", input.Namespace)
				return &workflowservicepb.SignalWithStartWorkflowExecutionResponse{Started: started}, nil
			})))
		env.RegisterNexusService(service)
		env.ExecuteWorkflow(func(ctx workflow.Context) (sw.SignalResult, error) {
			var result sw.SignalResult
			err := sw.SignalWithStart(ctx, sw.SignalWithStartOptions{
				WorkflowExecution: sw.WorkflowExecution{WorkflowID: "workflow"},
			}).Get(ctx, &result)
			return result, err
		})
		require.NoError(t, env.GetWorkflowError())
		var result sw.SignalResult
		require.NoError(t, env.GetWorkflowResult(&result))
		require.Equal(t, "default-test-namespace", result.Namespace)
		if started {
			require.NotNil(t, result.Started)
			require.True(t, *result.Started)
		} else {
			require.Nil(t, result.Started)
		}
	}
}

func TestCompositeDescriptorsAndArgumentPacking(t *testing.T) {
	var suite testsuite.WorkflowTestSuite
	env := suite.NewTestWorkflowEnvironment()
	service := nexus.NewService(functionExecutionServiceName)
	var request any
	require.NoError(t, service.Register(nexus.NewSyncOperation("ExecuteDescriptors",
		func(_ context.Context, input any, _ nexus.StartOperationOptions) (fe.ExecuteFunctionResult, error) {
			request = input
			return fe.ExecuteFunctionResult{Value: "executed"}, nil
		})))
	env.RegisterNexusService(service)
	env.ExecuteWorkflow(func(ctx workflow.Context) error {
		// Secondary descriptor groups follow authored order. The primary
		// callable and its variadic arguments come last.
		var result fe.ExecuteFunctionResult
		return fe.ExecuteDescriptors(
			ctx, fe.ExecuteDescriptorsOptions{},
			[]time.Duration{time.Second}, []string{"duration-arg"},
			func(context.Context) any { return "context-name" }, "context-arg",
			func(args ...any) string { return args[0].(string) }, "callable-arg",
			validVarargsFunction, "first", "second",
		).Get(ctx, &result)
	})
	require.NoError(t, env.GetWorkflowError())
	require.Equal(t, "durations:[1s]", stringField(request, "Durations"))
	require.Equal(t, "context-name", stringField(request, "Callback"))
	require.Equal(t, "resolved", stringField(request, "Callable"))
	require.Equal(t, "validVarargsFunction", stringField(request, "Function"))
	require.Equal(t, []string{"first", "second"}, stringSliceField(request, "Args"))
	// Descriptor arguments are single application values, not spread lists.
	require.Equal(t, []any{[]any{"duration-arg"}}, field(request, "DurationValues"))
	require.Equal(t, []any{"context-arg"}, field(request, "CallbackValues"))
	require.Equal(t, []any{"callable-arg"}, field(request, "CallableValues"))
}

func variadicSecondary(args ...any) string {
	return "secondary"
}

func TestSecondaryCallableGroupsAndTrailingVarargs(t *testing.T) {
	var suite testsuite.WorkflowTestSuite
	env := suite.NewTestWorkflowEnvironment()
	service := nexus.NewService(functionExecutionServiceName)
	requests := map[string]any{}
	for _, operation := range []string{
		"ExecuteSecondaryGroups", "ExecuteFixedSecondary", "ExecuteDescriptorSecondary",
	} {
		require.NoError(t, service.Register(nexus.NewSyncOperation(operation,
			func(_ context.Context, input any, _ nexus.StartOperationOptions) (fe.ExecuteFunctionResult, error) {
				requests[operation] = input
				return fe.ExecuteFunctionResult{Value: operation}, nil
			})))
	}
	env.RegisterNexusService(service)
	env.ExecuteWorkflow(func(ctx workflow.Context) error {
		// The authored order is other, signal, with the primary variadic
		// group moved to the end. Secondary argument groups remain slices.
		if err := fe.ExecuteSecondaryGroups(
			ctx, fe.ExecuteSecondaryGroupsOptions{},
			variadicSecondary, []any{"other-a", "other-b"},
			"signal-name", []any{"signal-a", "signal-b"},
			validVarargsFunction, "primary-a", "primary-b",
		).Get(ctx, nil); err != nil {
			return err
		}
		// A fixed primary keeps its place; the last secondary is variadic.
		// The authored secondary order is reversed from the operation above.
		if err := fe.ExecuteFixedSecondary(
			ctx, fe.ExecuteFixedSecondaryOptions{}, validFunction, "fixed-name", true,
			variadicSecondary, []any{"fixed-signal"},
			"other-name", "fixed-other-a", "fixed-other-b",
		).Get(ctx, nil); err != nil {
			return err
		}
		// An explicit primary descriptor takes one slice argument, not
		// varargs, allowing the secondary group to use the variadic tail.
		return fe.ExecuteDescriptorSecondary(
			ctx, fe.ExecuteDescriptorSecondaryOptions{},
			[]time.Duration{time.Second}, []string{"descriptor-primary"},
			"descriptor-signal-name", "descriptor-signal-a", "descriptor-signal-b",
		).Get(ctx, nil)
	})
	require.True(t, env.IsWorkflowCompleted())
	require.NoError(t, env.GetWorkflowError())
	require.Len(t, requests, 3)

	groups := requests["ExecuteSecondaryGroups"]
	require.Equal(t, "variadicSecondary", stringField(groups, "Other"))
	require.Equal(t, []any{"other-a", "other-b"}, field(groups, "OtherArgs"))
	require.Equal(t, "signal-name", stringField(groups, "Signal"))
	require.Equal(t, []any{"signal-a", "signal-b"}, field(groups, "SignalArgs"))
	require.Equal(t, "validVarargsFunction", stringField(groups, "Function"))
	require.Equal(t, []string{"primary-a", "primary-b"}, stringSliceField(groups, "Args"))

	fixed := requests["ExecuteFixedSecondary"]
	require.Equal(t, "validFunction", stringField(fixed, "Function"))
	require.Equal(t, "fixed-name", stringField(fixed, "Name"))
	require.Equal(t, true, field(fixed, "Enabled"))
	require.Equal(t, "variadicSecondary", stringField(fixed, "Signal"))
	require.Equal(t, []any{"fixed-signal"}, field(fixed, "SignalArgs"))
	require.Equal(t, "other-name", stringField(fixed, "Other"))
	require.Equal(t, []any{"fixed-other-a", "fixed-other-b"}, field(fixed, "OtherArgs"))

	descriptor := requests["ExecuteDescriptorSecondary"]
	require.Equal(t, "durations:[1s]", stringField(descriptor, "Function"))
	require.Equal(t, []any{[]any{"descriptor-primary"}}, field(descriptor, "PrimaryArgs"))
	require.Equal(t, "descriptor-signal-name", stringField(descriptor, "Signal"))
	require.Equal(t, []any{"descriptor-signal-a", "descriptor-signal-b"}, field(descriptor, "SignalArgs"))
}
