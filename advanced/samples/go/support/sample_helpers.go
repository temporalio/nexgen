package support

import (
	"context"
	"fmt"
	"time"

	"go.temporal.io/sdk/workflow"
)

func NamespaceData(ctx workflow.Context) map[string]string {
	return map[string]string{"namespace": WorkflowNamespace(ctx)}
}

func RequestType(ctx workflow.Context) string { return "getter-probe" }

// DescriptorName deliberately accepts the composite public descriptor values,
// rather than their wire strings. The sample checks that the wrapper invokes it.
func DescriptorName(ctx workflow.Context, value any) string {
	switch value := value.(type) {
	case []time.Duration:
		return fmt.Sprintf("durations:%v", value)
	case func(context.Context) any:
		return value(context.Background()).(string)
	case func(...any) string:
		return value("resolved")
	default:
		panic(fmt.Sprintf("unexpected descriptor %T", value))
	}
}
