// Test-only bridge for generated System Nexus bindings that are placed inside
// the workflow package. The real SDK supplies this module from its source tree.
export * from '@temporalio/workflow';

export interface NexusOperationHandle<T> {
  result(): Promise<T>;
}

export declare function startSystemNexusOperation<T>(input: {
  [key: string]: unknown;
  input: any;
  toProto: (input: any) => unknown;
  serializationContext?: (input: any) => unknown;
}): Promise<NexusOperationHandle<T>>;
