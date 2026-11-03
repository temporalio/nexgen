package internal

import (
	"go.temporal.io/sdk/converter"
	sdkinternal "go.temporal.io/sdk/internal"
)

// These aliases and wrappers supply the SDK declarations that generated files
// will share a package with when installed in the SDK.
type (
	Context                  = sdkinternal.Context
	Future                   = sdkinternal.Future
	RetryPolicy              = sdkinternal.RetryPolicy
	SearchAttributes         = sdkinternal.SearchAttributes
	Priority                 = sdkinternal.Priority
	VersioningOverride       = sdkinternal.VersioningOverride
	NexusOperationOptions    = sdkinternal.NexusOperationOptions
	SystemNexusRegistryKey   = sdkinternal.SystemNexusRegistryKey
	SystemNexusRegistryEntry = sdkinternal.SystemNexusRegistryEntry
)

func NewFuture(ctx Context) (Future, sdkinternal.Settable) {
	return sdkinternal.NewFuture(ctx)
}

func Go(ctx Context, f func(Context)) {
	sdkinternal.Go(ctx, f)
}

func NewSystemNexusClient(service string) sdkinternal.NexusClient {
	return sdkinternal.NewSystemNexusClient(service)
}

func GetWorkflowInfo(ctx Context) *sdkinternal.WorkflowInfo {
	return sdkinternal.GetWorkflowInfo(ctx)
}

func GetDataConverterFromWorkflowContext(ctx Context) converter.DataConverter {
	return sdkinternal.GetDataConverterFromWorkflowContext(ctx)
}

func UpdateSystemNexusRegistry(entries map[SystemNexusRegistryKey]SystemNexusRegistryEntry) {
	sdkinternal.UpdateSystemNexusRegistry(entries)
}
