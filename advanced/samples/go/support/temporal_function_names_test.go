package support

import (
	"testing"

	"go.temporal.io/sdk/testsuite"
	"go.temporal.io/sdk/workflow"
)

func namedWorkflow(ctx workflow.Context, value string) (string, error) {
	return value, nil
}

func TestWorkflowFunctionNameResolvesRegisteredAlias(t *testing.T) {
	var suite testsuite.WorkflowTestSuite
	env := suite.NewTestWorkflowEnvironment()
	env.RegisterWorkflowWithOptions(namedWorkflow, workflow.RegisterOptions{Name: "registered-alias"})
	env.ExecuteWorkflow(func(ctx workflow.Context) ([]string, error) {
		return []string{
			WorkflowFunctionName(ctx, namedWorkflow),
			WorkflowFunctionName(ctx, "authored-name"),
		}, nil
	})
	if err := env.GetWorkflowError(); err != nil {
		t.Fatal(err)
	}
	var names []string
	if err := env.GetWorkflowResult(&names); err != nil {
		t.Fatal(err)
	}
	if len(names) != 2 || names[0] != "registered-alias" || names[1] != "authored-name" {
		t.Fatalf("unexpected workflow names: %v", names)
	}
}
