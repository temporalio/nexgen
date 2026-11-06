package tests

import (
	"testing"

	"github.com/stretchr/testify/require"
	service "go.temporal.io/sdk/advanced/samples/go/multioperationservice"
	"go.temporal.io/sdk/interceptor"
	"go.temporal.io/sdk/testsuite"
	"go.temporal.io/sdk/workflow"
)

type multiOperationOutbound struct {
	runtime *multiOperationRuntime
}

func (i *multiOperationOutbound) FirstOperation(ctx workflow.Context, opts service.SystemNexus_FirstOperationOptions, id string) workflow.Future {
	return service.SystemNexus_WorkflowEnvironmentInterceptor_FirstOperation(i.runtime.ExecuteNexusOperation, ctx, opts, "intercepted-"+id)
}

func (i *multiOperationOutbound) SecondOperation(ctx workflow.Context, opts service.SystemNexus_SecondOperationOptions, id string) workflow.Future {
	return service.SystemNexus_WorkflowEnvironmentInterceptor_SecondOperation(i.runtime.ExecuteNexusOperation, ctx, opts, id)
}

type multiOperationRuntime struct {
	inputs []interceptor.ExecuteNexusOperationInput
}

func (r *multiOperationRuntime) ExecuteNexusOperation(ctx workflow.Context, input interceptor.ExecuteNexusOperationInput) workflow.NexusOperationFuture {
	r.inputs = append(r.inputs, input)
	future, settable := workflow.NewFuture(ctx)
	settable.Set(service.SystemNexus_Response{Value: input.Input.(service.SystemNexus_Request).ID}, nil)
	return &sampleNexusFuture{Future: future}
}

type sampleNexusFuture struct {
	workflow.Future
}

func (f *sampleNexusFuture) GetNexusOperationExecution() workflow.Future {
	return f.Future
}

func TestMultipleSystemNexusOperations(t *testing.T) {
	var suite testsuite.WorkflowTestSuite
	env := suite.NewTestWorkflowEnvironment()
	runtime := &multiOperationRuntime{}
	outbound := &multiOperationOutbound{runtime: runtime}
	env.ExecuteWorkflow(func(ctx workflow.Context) ([]string, error) {
		var first, second service.SystemNexus_Response
		if err := service.SystemNexus_WorkflowOutboundInterceptorBase_FirstOperation(outbound, ctx, service.SystemNexus_FirstOperationOptions{}, "first").Get(ctx, &first); err != nil {
			return nil, err
		}
		if err := service.SystemNexus_WorkflowOutboundInterceptorBase_SecondOperation(outbound, ctx, service.SystemNexus_SecondOperationOptions{}, "second").Get(ctx, &second); err != nil {
			return nil, err
		}
		return []string{first.Value, second.Value}, nil
	})
	require.NoError(t, env.GetWorkflowError())
	var result []string
	require.NoError(t, env.GetWorkflowResult(&result))
	require.Equal(t, []string{"intercepted-first", "second"}, result)
	require.Len(t, runtime.inputs, 2)
	for index, operation := range []string{"FirstOperation", "SecondOperation"} {
		input := runtime.inputs[index]
		require.Equal(t, "InterceptorService", input.Client.Service())
		require.Equal(t, operation, input.Operation)
		require.Equal(t, workflow.NexusOperationOptions{}, input.Options)
		require.Empty(t, input.NexusHeader)
	}
}
