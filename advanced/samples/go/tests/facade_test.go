package tests

import (
	"testing"

	"github.com/stretchr/testify/require"
	workflowservicepb "go.temporal.io/api/workflowservice/v1"
	ws "go.temporal.io/sdk/advanced/samples/go/workflowservice_facade"
	"go.temporal.io/sdk/converter"
	commonconverter "go.temporal.io/sdk/internal/common/converter"
)

func TestSystemNexusFacadeTransferType(t *testing.T) {
	runID := "target-run"
	started := true
	response := ws.SignalWithStartWorkflowResponse{
		RunID:   &runID,
		Started: &started,
	}
	dc := commonconverter.WithDataConverterSystemNexusContext(
		commonconverter.MakeTransferAware(converter.GetDefaultDataConverter()),
		converter.GetDefaultDataConverter(),
	)
	payload, err := dc.ToPayload(response)
	require.NoError(t, err)
	var wire workflowservicepb.SignalWithStartWorkflowExecutionResponse
	require.NoError(t, converter.GetDefaultDataConverter().FromPayload(payload, &wire))
	require.Equal(t, runID, wire.RunId)
	require.True(t, wire.Started)
	var decoded ws.SignalWithStartWorkflowResponse
	require.NoError(t, dc.FromPayload(payload, &decoded))
	require.Equal(t, response, decoded)
}
