package tests

import (
	"testing"

	chat "samples/go/chat"

	"github.com/stretchr/testify/require"
	"go.temporal.io/sdk/converter"
)

// TestJSONSchemaChatRuntime round-trips every chat wire fixture through the
// Temporal default data converter and asserts JSON-equality against the
// canonical fixtures, mirroring the Python and Java suites.
//
// Exception (see json-schema/nullability.md): optional+nullable fields collapse
// in Go. message-full.json carries replyToId: null (optional+nullable), which
// Go collapses on serialize, so it is verified by deserialization + field checks
// rather than exact JSON-equality, matching the Java test. (room-open.json's
// topic is required-nullable, so its explicit null survives the round-trip and
// is checked via JSON-equality.)
func TestJSONSchemaChatRuntime(t *testing.T) {
	dc := converter.GetDefaultDataConverter()

	minimal := roundTripJSONEq[chat.Message](t, dc, "chat", "message-minimal.json")
	require.Equal(t, chat.MessageKindText, minimal.Kind)
	require.Equal(t, "hi", minimal.Body)
	require.Nil(t, minimal.ReplyToID)
	require.Nil(t, minimal.Priority)
	require.Equal(t, int64(0), minimal.PriorityOrDefault())

	// message-full carries replyToId: null (optional+nullable) — deserialization only.
	full := decodeFixture[chat.Message](t, dc, "chat", "message-full.json")
	require.Nil(t, full.ReplyToID)
	require.NotNil(t, full.Priority)
	require.Equal(t, int64(7), *full.Priority)

	room := roundTripJSONEq[chat.Room](t, dc, "chat", "room-open.json")
	require.Equal(t, "r1", room.RoomID)
	require.Nil(t, room.Topic)
	require.Equal(t, []string{"a"}, room.Members)
	require.Contains(t, room.AdditionalProperties, "x-extra")

	labels := roundTripJSONEq[chat.Labels](t, dc, "chat", "labels.json")
	require.Equal(t, "prod", labels.AdditionalProperties["env"])
	require.Equal(t, "core", labels.AdditionalProperties["team"])

	input := roundTripJSONEq[chat.SendMessageInput](t, dc, "chat", "send-message-input.json")
	require.Equal(t, "r1", input.RoomID)
	require.Equal(t, "hi", input.Message.Body)

	output := roundTripJSONEq[chat.SendMessageOutput](t, dc, "chat", "send-message-output.json")
	require.Equal(t, "m1", output.MessageID)
}
