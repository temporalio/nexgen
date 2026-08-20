package tests

import (
	gm "advanced/samples/go/genericmodels"
	"go.temporal.io/sdk/workflow"
)

var _ func(workflow.Context, gm.CompleteOptions[string]) workflow.Future = gm.Complete[string]

var success gm.OperationCompletionResult[int] = gm.OperationCompletionResultSuccess[int]{
	Value: 42,
}

var failure gm.OperationCompletionResult[int] = gm.OperationCompletionResultFailure[int]{
	Value: "failed",
}

var _ = gm.GenericResponse[string, int, bool]{
	Context:    "context",
	Completion: success,
	Metadata:   boolPtr(true),
}

func boolPtr(value bool) *bool {
	return &value
}
