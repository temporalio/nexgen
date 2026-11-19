package tests

import (
	"testing"

	"github.com/stretchr/testify/require"
	commonpb "go.temporal.io/api/common/v1"
	failurepb "go.temporal.io/api/failure/v1"
	updatepb "go.temporal.io/api/update/v1"
	workflowservicepb "go.temporal.io/api/workflowservice/v1"
	"go.temporal.io/sdk/converter"

	generic "go.temporal.io/sdk/advanced/samples/go/protogeneric"
	oneof "go.temporal.io/sdk/advanced/samples/go/protooneof"
)

func TestNestedProtoGeneric(t *testing.T) {
	dc := converter.GetDefaultDataConverter()
	model := generic.PayloadBackedEnvelope[completion, sourceContext]{
		Provider: generic.PayloadBackedOutput[completion]{Details: completion{"completed"}},
		Scaler:   generic.PayloadBackedContext[sourceContext]{Details: sourceContext{"workflow-id"}},
	}
	payload, err := dc.ToPayload(model)
	require.NoError(t, err)
	var decoded generic.PayloadBackedEnvelope[completion, sourceContext]
	require.NoError(t, dc.FromPayload(payload, &decoded))
	require.Equal(t, model, decoded)
}

func TestGenericPayloads(t *testing.T) {
	dc := converter.GetDefaultDataConverter()
	payload, err := dc.ToPayload(completion{"completed"})
	require.NoError(t, err)
	for _, count := range []int{0, 1, 2} {
		wire := updatepb.Outcome{Value: &updatepb.Outcome_Success{
			Success: &commonpb.Payloads{},
		}}
		for i := 0; i < count; i++ {
			wire.GetSuccess().Payloads = append(wire.GetSuccess().Payloads, payload)
		}
		encoded, err := dc.ToPayload(&wire)
		require.NoError(t, err)
		var decoded oneof.Outcome[completion]
		err = dc.FromPayload(encoded, &decoded)
		if count > 1 {
			require.ErrorContains(t, err, "Outcome.Value")
			require.ErrorContains(t, err, "got 2")
			continue
		}
		require.NoError(t, err)
		want := completion{}
		if count == 1 {
			want.Message = "completed"
		}
		require.Equal(t, oneof.OutcomeValueSuccess[completion]{Value: want}, decoded.Value)
		roundTrip, err := dc.ToPayload(decoded)
		require.NoError(t, err)
		var decodedAgain oneof.Outcome[completion]
		require.NoError(t, dc.FromPayload(roundTrip, &decodedAgain))
		require.Equal(t, decoded, decodedAgain)
	}
}

func TestOptionalGenericPayloads(t *testing.T) {
	dc := converter.GetDefaultDataConverter()
	value := completion{"completed"}
	for _, details := range []*completion{nil, &value} {
		model := generic.OptionalPayloads[completion]{Details: details}
		payload, err := dc.ToPayload(model)
		require.NoError(t, err)
		var wire failurepb.CanceledFailureInfo
		require.NoError(t, dc.FromPayload(payload, &wire))
		if details == nil {
			require.Nil(t, wire.Details)
		} else {
			require.Len(t, wire.Details.Payloads, 1)
		}
		var decoded generic.OptionalPayloads[completion]
		require.NoError(t, dc.FromPayload(payload, &decoded))
		require.Equal(t, model, decoded)
	}
}

func TestOptionalScalarOneof(t *testing.T) {
	dc := converter.GetDefaultDataConverter()
	for _, selection := range []oneof.ActivitySelection{
		nil,
		oneof.ActivitySelectionID{Value: "activity-id"},
		oneof.ActivitySelectionType{Value: "activity-type"},
	} {
		model := oneof.PauseActivityRequest{Activity: selection}
		payload, err := dc.ToPayload(model)
		require.NoError(t, err)
		var wire workflowservicepb.PauseActivityRequest
		require.NoError(t, dc.FromPayload(payload, &wire))
		switch selection.(type) {
		case oneof.ActivitySelectionID:
			require.Equal(t, "activity-id", wire.GetId())
		case oneof.ActivitySelectionType:
			require.Equal(t, "activity-type", wire.GetType())
		default:
			require.Nil(t, wire.Activity)
		}
		var decoded oneof.PauseActivityRequest
		require.NoError(t, dc.FromPayload(payload, &decoded))
		require.Equal(t, model, decoded)
	}
}
