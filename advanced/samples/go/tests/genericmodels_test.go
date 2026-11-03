package tests

import (
	gm "go.temporal.io/sdk/advanced/samples/go/genericmodels"
	"go.temporal.io/sdk/workflow"
)

var _ func(workflow.Context, gm.CompleteOptions[string]) workflow.Future = gm.Complete[string]

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
