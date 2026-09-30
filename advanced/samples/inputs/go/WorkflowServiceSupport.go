package workflowservice

import "go.temporal.io/sdk/converter"

func signalWithStartWorkflowSerializationContext(request signalWithStartWorkflowRequest) converter.WorkflowSerializationContext {
	return converter.WorkflowSerializationContext{
		Namespace:  request.namespace,
		WorkflowID: request.ID,
	}
}
