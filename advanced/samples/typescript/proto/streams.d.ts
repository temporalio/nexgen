// The protobufjs interfaces and enums of the protos in descriptors/streams/, which no release of
// `@temporalio/proto` has yet. The generated code only uses them as types.
import type Long from "long";

declare module "@temporalio/proto" {
  // temporal/api/enums/v1/stream.proto
  namespace temporal.api.enums.v1 {
    enum StreamStatus {
      STREAM_STATUS_UNSPECIFIED = 0,
      STREAM_STATUS_OPEN = 1,
      STREAM_STATUS_CLOSED = 2,
    }
    enum StreamIdConflictPolicy {
      STREAM_ID_CONFLICT_POLICY_UNSPECIFIED = 0,
      STREAM_ID_CONFLICT_POLICY_FAIL = 1,
      STREAM_ID_CONFLICT_POLICY_USE_EXISTING = 2,
    }
    enum StreamErrorReason {
      STREAM_ERROR_REASON_UNSPECIFIED = 0,
      STREAM_ERROR_REASON_NOT_FOUND = 1,
      STREAM_ERROR_REASON_EXPIRED = 2,
      STREAM_ERROR_REASON_CLOSED = 3,
      STREAM_ERROR_REASON_FENCED = 4,
      STREAM_ERROR_REASON_FORMAT_TOO_NEW = 5,
      STREAM_ERROR_REASON_OFFSET_TOO_HIGH = 6,
      STREAM_ERROR_REASON_TOO_LARGE = 7,
      STREAM_ERROR_REASON_INVALID_ARGUMENT = 8,
      STREAM_ERROR_REASON_STORE_UNAVAILABLE = 9,
      STREAM_ERROR_REASON_UNSUPPORTED = 10,
      STREAM_ERROR_REASON_ALREADY_EXISTS = 11,
      STREAM_ERROR_REASON_STORE_CORRUPT = 12,
    }
  }

  // temporal/api/stream/v1/message.proto
  namespace temporal.api.stream.v1 {
    interface IStreamRef {
      id?: string | null;
      store?: string | null;
    }
    interface IWorkflowTaskAttempt {
      startedEventId?: Long | null;
      attempt?: number | null;
      startedTime?: google.protobuf.ITimestamp | null;
    }
    interface ILocalActivityAttempt {
      task?: temporal.api.stream.v1.IWorkflowTaskAttempt | null;
      attempt?: number | null;
    }
    interface IActivityAttempt {
      attemptScheduledTime?: google.protobuf.ITimestamp | null;
      attempt?: number | null;
    }
    interface IWorkflowProducer {
      runId?: string | null;
      attempt?: temporal.api.stream.v1.IWorkflowTaskAttempt | null;
    }
    interface ILocalActivityProducer {
      runId?: string | null;
      attempt?: temporal.api.stream.v1.ILocalActivityAttempt | null;
    }
    interface IActivityProducer {
      runId?: string | null;
      attempt?: temporal.api.stream.v1.IActivityAttempt | null;
    }
    interface IClientProducer {}
    interface IProducer {
      id?: string | null;
      workflow?: temporal.api.stream.v1.IWorkflowProducer | null;
      localActivity?: temporal.api.stream.v1.ILocalActivityProducer | null;
      workflowActivity?: temporal.api.stream.v1.IActivityProducer | null;
      standaloneActivity?: temporal.api.stream.v1.IActivityProducer | null;
      client?: temporal.api.stream.v1.IClientProducer | null;
      kind?:
        | "workflow"
        | "localActivity"
        | "workflowActivity"
        | "standaloneActivity"
        | "client";
    }
    interface IAppendRecord {
      body?: temporal.api.common.v1.IPayload | null;
    }
    interface IStreamRecord {
      producer?: temporal.api.stream.v1.IProducer | null;
      body?: temporal.api.common.v1.IPayload | null;
      offset?: Uint8Array | null;
    }
    interface IStreamErrorDetails {
      reason?: temporal.api.enums.v1.StreamErrorReason | null;
    }
  }

  // temporal/api/streamservice/v1/request_response.proto
  namespace temporal.api.streamservice.v1 {
    interface ICreateStreamRequest {
      streamId?: string | null;
      idConflictPolicy?: temporal.api.enums.v1.StreamIdConflictPolicy | null;
      requestId?: string | null;
    }
    interface ICreateStreamResponse {
      created?: boolean | null;
    }
    interface IAppendRecordsRequest {
      streamId?: string | null;
      producer?: temporal.api.stream.v1.IProducer | null;
      requestId?: string | null;
      records?: temporal.api.stream.v1.IAppendRecord[] | null;
    }
    interface IAppendRecordsResponse {
      firstOffset?: Uint8Array | null;
      lastOffset?: Uint8Array | null;
    }
    interface ICloseStreamRequest {
      streamId?: string | null;
      producer?: temporal.api.stream.v1.IProducer | null;
    }
    interface ICloseStreamResponse {}
    interface IReadRecordsRequest {
      streamId?: string | null;
      fromOffset?: Uint8Array | null;
      maxRecords?: number | null;
      maxBytes?: Long | null;
      wait?: google.protobuf.IDuration | null;
    }
    interface IReadRecordsResponse {
      records?: temporal.api.stream.v1.IStreamRecord[] | null;
      lastOffset?: Uint8Array | null;
      done?: boolean | null;
    }
    interface IDescribeStreamRequest {
      streamId?: string | null;
    }
    interface IDescribeStreamResponse {
      status?: temporal.api.enums.v1.StreamStatus | null;
      lastOffset?: Uint8Array | null;
    }
  }
}
