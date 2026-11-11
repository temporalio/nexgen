package tests

import (
	"encoding/json"
	"testing"

	gm "go.temporal.io/sdk/advanced/samples/go/genericmodels"
	"go.temporal.io/sdk/workflow"
)

var _ func(workflow.Context, gm.CompleteOptions[string], string, []string, map[string]string, gm.Inner[string]) workflow.Future = gm.Complete[string]

var success gm.OperationCompletionResult[int] = gm.OperationCompletionResultSuccess[int]{
	Value: gm.OperationCompletionSuccess[int]{Output: 42},
}

var failure gm.OperationCompletionResult[int] = gm.OperationCompletionResultFailure[int]{
	Value: gm.OperationCompletionFailure{Message: "failed"},
}

var _ = gm.GenericResponse[string, int, bool]{
	Context:    "context",
	Completion: success,
	Metadata:   boolPtr(true),
}

func boolPtr(value bool) *bool {
	return &value
}

func TestExplicitGenericRefRoundTrip(t *testing.T) {
	reference := gm.ContextRef[string]{ID: "context-1"}
	_ = gm.CompleteOptions[string]{Reference: &reference}
	data, err := json.Marshal(reference)
	if err != nil {
		t.Fatal(err)
	}
	var decoded gm.ContextRef[string]
	if err := json.Unmarshal(data, &decoded); err != nil {
		t.Fatal(err)
	}
	if decoded != reference {
		t.Fatalf("reference did not round trip: %#v", decoded)
	}
}
