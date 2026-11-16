package tests

import (
	"context"
	"fmt"
	"testing"

	"github.com/nexus-rpc/sdk-go/nexus"
	"github.com/stretchr/testify/require"
	df "go.temporal.io/sdk/advanced/samples/go/defaults"
	"go.temporal.io/sdk/converter"
	"go.temporal.io/sdk/testsuite"
	"go.temporal.io/sdk/workflow"
)

func TestOperationDefaultsAndGoPresence(t *testing.T) {
	var suite testsuite.WorkflowTestSuite
	env := suite.NewTestWorkflowEnvironment()
	var calls []df.Request
	service := nexus.NewService("DefaultsService")
	handler := func(_ context.Context, input df.Request, _ nexus.StartOperationOptions) (df.Request, error) {
		calls = append(calls, input)
		// Inbound values must not be defaulted when the result is decoded.
		input.Namespace = ""
		input.Attempts = 0
		input.Retry = &df.RecordWithDefaults{}
		return input, nil
	}
	require.NoError(t, service.Register(nexus.NewSyncOperation("Submit", handler)))
	require.NoError(t, service.Register(nexus.NewSyncOperation("SubmitFromSender", handler)))
	env.RegisterNexusService(service)

	retry := &df.RecordWithDefaults{}
	env.ExecuteWorkflow(func(ctx workflow.Context) error {
		options := []df.SubmitOptions{
			{},
			{},
			{
				Namespace: "explicit", Attempts: 7, Ratio: 2,
				Mode: df.MyEnumExpress, Permissions: df.MyFlagsRead,
				Retry: retry, Details: "details",
			},
		}
		for _, opts := range options {
			var result df.Request
			if err := df.Submit(ctx, opts).Get(ctx, &result); err != nil {
				return err
			}
			if result.Namespace != "" || result.Attempts != 0 || result.Retry == nil ||
				result.Retry.Attempts != 0 || result.Retry.Label != "" {
				return fmt.Errorf("inbound defaults changed response: %+v", result)
			}
		}
		sender := df.Sender{Token: "unused"}
		if err := sender.Submit(ctx).Get(ctx, nil); err != nil {
			return err
		}
		return nil
	})
	require.NoError(t, env.GetWorkflowError())
	require.Len(t, calls, 4)

	first := calls[0]
	require.Equal(t, "default-test-namespace", first.Namespace)
	require.Equal(t, int32(3), first.Attempts)
	require.True(t, first.Enabled, "false selects the default on a punned bool surface")
	require.Equal(t, 1.5, first.Ratio)
	require.Equal(t, df.MyEnumStandard, first.Mode)
	require.Equal(t, df.MyFlagsRead|df.MyFlagsWrite, first.Permissions)
	require.Equal(t, "alias", first.AliasLabel)
	require.Nil(t, first.Retry)
	require.Nil(t, first.Metadata)
	require.Equal(t, first, calls[1])
	require.Equal(t, first, calls[3], "resource forwarding must use operation defaults")

	explicit := calls[2]
	require.Equal(t, "explicit", explicit.Namespace)
	require.Equal(t, int32(7), explicit.Attempts)
	require.Equal(t, retry, explicit.Retry, "supplied nested records pass through unchanged")
	require.Zero(t, explicit.Retry.Attempts)
	require.Empty(t, explicit.Retry.Label)
	require.Equal(t, "untitled", explicit.Metadata.Summary)
	require.Equal(t, "details", *explicit.Metadata.Details)
	require.Zero(t, retry.Attempts)
	require.Empty(t, retry.Label)
}

func TestEncodingAndDecodingDoNotApplyDefaults(t *testing.T) {
	dc := converter.GetDefaultDataConverter()
	model := df.Request{}
	for range 2 {
		payload, err := dc.ToPayload(model)
		require.NoError(t, err)
		var decoded df.Request
		require.NoError(t, dc.FromPayload(payload, &decoded))
		require.Equal(t, model, decoded)
	}
}
