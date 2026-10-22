package tests

import (
	"testing"

	"github.com/stretchr/testify/require"
	enums "go.temporal.io/api/enums/v1"
	pe "go.temporal.io/sdk/advanced/samples/go/protoenums"
)

func TestProtoEnumNumbers(t *testing.T) {
	require.Equal(t, int32(enums.NAMESPACE_STATE_REGISTERED), int32(pe.NamespaceStateNamespaceStateRegistered))
	require.Equal(t, int32(enums.SEVERITY_LOW), int32(pe.AlertLevelSeverityLow))
	require.Equal(t, int32(enums.SEVERITY_HIGH), int32(pe.AlertLevelSeverityHigh))
	require.Equal(t, int32(enums.SEVERITY_MEDIUM), int32(pe.AlertLevelSeverityMedium))
	require.Equal(t, int32(enums.SEVERITY_UNSPECIFIED), int32(pe.AlertLevelSeverityUnspecified))
	require.Equal(t, int32(enums.COMMAND_TYPE_PROTOCOL_MESSAGE), int32(pe.CommandTypeCommandTypeProtocolMessage))
	require.Equal(t, int32(enums.COMMAND_TYPE_MODIFY_WORKFLOW_PROPERTIES), int32(pe.CommandTypeCommandTypeModifyWorkflowProperties))
	require.Equal(t, int32(enums.COMMAND_TYPE_SCHEDULE_NEXUS_OPERATION), int32(pe.CommandTypeCommandTypeScheduleNexusOperation))
	require.Equal(t, int32(enums.COMMAND_TYPE_REQUEST_CANCEL_NEXUS_OPERATION), int32(pe.CommandTypeCommandTypeRequestCancelNexusOperation))
}
