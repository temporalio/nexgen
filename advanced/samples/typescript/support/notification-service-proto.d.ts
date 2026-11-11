import "@temporalio/proto";

// TEMPORARY: Remove once the TypeScript SDK exports these models.
// This sample-only augmentation supplies structural wire types, not protobuf
// constructors or binary encoding. Keep it aligned with the checked-in descriptor.
declare module "@temporalio/proto" {
  namespace temporal.api.notificationservice.v1 {
    interface IOnCompleteRequest {
      result?: "success" | "failure";
      success?: temporal.api.common.v1.IPayload | null;
      failure?: temporal.api.failure.v1.IFailure | null;
      sourceContext?: temporal.api.common.v1.IPayload | null;
    }

    interface IOnCompleteResponse {}
  }
}
