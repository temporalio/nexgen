// Test-only bridge for generated System Nexus bindings that are placed inside
// the workflow package. The real SDK supplies this module from its source tree.
// Remove this bridge once the TypeScript SDK release used by these examples
// exports startSystemNexusOperation.
export * from '@temporalio/workflow';

export declare function startSystemNexusOperation<T>(input: {
  [key: string]: unknown;
  input: any;
  inputType: unknown;
  outputType?: unknown;
  serializationContext?: (input: any) => unknown;
}): Promise<import('@temporalio/workflow').NexusOperationHandle<T>>;
