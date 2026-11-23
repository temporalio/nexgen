package tests

import (
	"testing"

	"github.com/stretchr/testify/require"
	notificationpb "go.temporal.io/api/notificationservice/v1"
	"go.temporal.io/sdk/converter"
	commonconverter "go.temporal.io/sdk/internal/common/converter"
	"go.temporal.io/sdk/temporal"
	"google.golang.org/protobuf/proto"

	ns "go.temporal.io/sdk/advanced/samples/go/notificationservice_facade"
)

func notificationDataConverter() converter.DataConverter {
	dc := converter.GetDefaultDataConverter()
	return commonconverter.WithDataConverterSystemNexusContext(commonconverter.MakeTransferAware(dc), dc)
}

type completion struct {
	Message string
}

type sourceContext struct {
	WorkflowID string
}

func TestNotificationServiceSuccess(t *testing.T) {
	dc := notificationDataConverter()
	for _, result := range []ns.OnCompleteRequestResult[completion]{
		ns.OnCompleteRequestResultSuccess[completion]{Value: completion{"completed"}},
		&ns.OnCompleteRequestResultSuccess[completion]{Value: completion{"completed"}},
	} {
		model := ns.OnCompleteRequest[completion, sourceContext]{
			Result: result, SourceContext: &sourceContext{"workflow-id"},
		}
		payload, err := dc.ToPayload(model)
		require.NoError(t, err)
		var wire notificationpb.OnCompleteRequest
		require.NoError(t, dc.FromPayload(payload, &wire))
		require.IsType(t, &notificationpb.OnCompleteRequest_Success{}, wire.Result)
		var output completion
		require.NoError(t, dc.FromPayload(wire.GetSuccess(), &output))
		require.Equal(t, completion{"completed"}, output)
		var decoded ns.OnCompleteRequest[completion, sourceContext]
		require.NoError(t, dc.FromPayload(payload, &decoded))
		require.Equal(t, model.SourceContext, decoded.SourceContext)
		require.Equal(t, ns.OnCompleteRequestResultSuccess[completion]{Value: output}, decoded.Result)

		// A different closed instantiation keeps payload bytes unconverted.
		var raw ns.OnCompleteRequest[converter.RawValue, sourceContext]
		require.NoError(t, dc.FromPayload(payload, &raw))
		success := raw.Result.(ns.OnCompleteRequestResultSuccess[converter.RawValue])
		require.True(t, proto.Equal(wire.GetSuccess(), success.Value.Payload()))
		rawPayload, err := dc.ToPayload(raw)
		require.NoError(t, err)
		var rawWire notificationpb.OnCompleteRequest
		require.NoError(t, dc.FromPayload(rawPayload, &rawWire))
		require.True(t, proto.Equal(&wire, &rawWire))
	}
}

func TestNotificationServiceFailure(t *testing.T) {
	dc := notificationDataConverter()
	model := ns.OnCompleteRequest[completion, sourceContext]{
		Result: ns.OnCompleteRequestResultFailure[completion]{
			Value: temporal.NewApplicationError("operation failed", "CompletionFailure", "detail"),
		},
		SourceContext: &sourceContext{"workflow-id"},
	}
	payload, err := dc.ToPayload(&model)
	require.NoError(t, err)
	var wire notificationpb.OnCompleteRequest
	require.NoError(t, dc.FromPayload(payload, &wire))
	require.IsType(t, &notificationpb.OnCompleteRequest_Failure{}, wire.Result)
	require.Equal(t, "operation failed", wire.GetFailure().Message)
	var decoded ns.OnCompleteRequest[completion, sourceContext]
	require.NoError(t, dc.FromPayload(payload, &decoded))
	failure := decoded.Result.(ns.OnCompleteRequestResultFailure[completion])
	var applicationError *temporal.ApplicationError
	require.ErrorAs(t, failure.Value, &applicationError)
	require.Equal(t, "CompletionFailure", applicationError.Type())
	var detail string
	require.NoError(t, applicationError.Details(&detail))
	require.Equal(t, "detail", detail)
	require.Equal(t, model.SourceContext, decoded.SourceContext)
}

func TestNotificationServiceRequiredFields(t *testing.T) {
	dc := notificationDataConverter()
	context, err := dc.ToPayload(sourceContext{"workflow-id"})
	require.NoError(t, err)
	payload, err := dc.ToPayload(&notificationpb.OnCompleteRequest{SourceContext: context})
	require.NoError(t, err)
	var decoded ns.OnCompleteRequest[completion, sourceContext]
	require.ErrorContains(t, dc.FromPayload(payload, &decoded), "OnCompleteRequest.Result")
	for _, result := range []ns.OnCompleteRequestResult[completion]{
		nil,
		(*ns.OnCompleteRequestResultSuccess[completion])(nil),
		(*ns.OnCompleteRequestResultFailure[completion])(nil),
		ns.OnCompleteRequestResultFailure[completion]{},
	} {
		_, err := dc.ToPayload(ns.OnCompleteRequest[completion, sourceContext]{Result: result})
		require.ErrorContains(t, err, "OnCompleteRequest.Result")
	}
}

func TestNotificationServiceWithoutSourceContext(t *testing.T) {
	dc := notificationDataConverter()
	model := ns.OnCompleteRequest[completion, sourceContext]{
		Result: ns.OnCompleteRequestResultSuccess[completion]{Value: completion{"completed"}},
	}
	payload, err := dc.ToPayload(model)
	require.NoError(t, err)
	var wire notificationpb.OnCompleteRequest
	require.NoError(t, dc.FromPayload(payload, &wire))
	require.Nil(t, wire.GetSourceContext())
	var decoded ns.OnCompleteRequest[completion, sourceContext]
	require.NoError(t, dc.FromPayload(payload, &decoded))
	require.Equal(t, model, decoded)
}

func TestNotificationServiceEmptyResponse(t *testing.T) {
	dc := notificationDataConverter()
	payload, err := dc.ToPayload(ns.OnCompleteResponse{})
	require.NoError(t, err)
	var wire notificationpb.OnCompleteResponse
	require.NoError(t, dc.FromPayload(payload, &wire))
	var decoded ns.OnCompleteResponse
	require.NoError(t, dc.FromPayload(payload, &decoded))
	require.Equal(t, ns.OnCompleteResponse{}, decoded)
}

func TestNotificationServiceNilOutput(t *testing.T) {
	dc := notificationDataConverter()
	model := ns.OnCompleteRequest[*completion, sourceContext]{
		Result:        ns.OnCompleteRequestResultSuccess[*completion]{},
		SourceContext: &sourceContext{"workflow-id"},
	}
	payload, err := dc.ToPayload(model)
	require.NoError(t, err)
	var wire notificationpb.OnCompleteRequest
	require.NoError(t, dc.FromPayload(payload, &wire))
	require.NotNil(t, wire.GetSuccess())
	require.Equal(t, []byte("binary/null"), wire.GetSuccess().Metadata["encoding"])
	var decoded ns.OnCompleteRequest[*completion, sourceContext]
	require.NoError(t, dc.FromPayload(payload, &decoded))
	require.Equal(t, model, decoded)
}

// Keep a compile-time check that the models opt into the public SDK contract.
var _ converter.ValueWithTransferTypeConverter = ns.OnCompleteRequest[completion, sourceContext]{}
