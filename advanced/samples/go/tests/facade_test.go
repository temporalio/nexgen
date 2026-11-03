package tests

import (
	"testing"

	"github.com/stretchr/testify/require"
	enums "go.temporal.io/api/enums/v1"
	workflowservicepb "go.temporal.io/api/workflowservice/v1"
	ws "go.temporal.io/sdk/advanced/samples/go/workflowservice_facade"
	"go.temporal.io/sdk/converter"
	commonconverter "go.temporal.io/sdk/internal/common/converter"
)

func TestSystemNexusFacadeTransferType(t *testing.T) {
	summary := "facade summary"
	cron := "0 * * * *"
	conflict := enums.WORKFLOW_ID_CONFLICT_POLICY_USE_EXISTING
	request := ws.SignalWithStartWorkflowRequest{
		Workflow:                 "workflow",
		ID:                       "target",
		TaskQueue:                "queue",
		Signal:                   "wake-up",
		Namespace:                "namespace",
		Args:                     []any{"workflow input"},
		SignalArgs:               []any{"signal input"},
		Headers:                  map[string]any{"trace": "parent"},
		CronSchedule:             &cron,
		WorkflowIDConflictPolicy: &conflict,
		UserMetadata: &ws.UserMetadata{
			StaticSummary: &summary,
		},
	}
	request.Headers["trace"] = "intercepted"
	dc := commonconverter.WithDataConverterSystemNexusContext(
		commonconverter.MakeTransferAware(converter.GetDefaultDataConverter()),
		converter.GetDefaultDataConverter(),
	)
	payload, err := dc.ToPayload(request)
	require.NoError(t, err)
	var wire workflowservicepb.SignalWithStartWorkflowExecutionRequest
	require.NoError(t, converter.GetDefaultDataConverter().FromPayload(payload, &wire))
	require.Equal(t, "target", wire.WorkflowId)
	var header string
	require.NoError(t, converter.GetDefaultDataConverter().FromPayload(wire.Header.Fields["trace"], &header))
	require.Equal(t, "intercepted", header)
	var decoded ws.SignalWithStartWorkflowRequest
	require.NoError(t, dc.FromPayload(payload, &decoded))
	require.Equal(t, request, decoded)
}
