// The protobufjs interfaces and enums of the protos in descriptors/streams/, which no release of
// `@temporalio/proto` has yet.
import type Long from "long";

declare module "@temporalio/proto" {
  // temporal/api/enums/v1/stream.proto
  namespace temporal.api.enums.v1 {
    enum StreamStatus {
      STREAM_STATUS_UNSPECIFIED = 0,
      STREAM_STATUS_OPEN = 1,
      STREAM_STATUS_CLOSED = 2,
      STREAM_STATUS_EXPIRED = 3,
    }
    enum StreamReadStart {
      STREAM_READ_START_UNSPECIFIED = 0,
      STREAM_READ_START_BEGINNING = 1,
      STREAM_READ_START_END = 2,
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
      STREAM_ERROR_REASON_SEQUENCE_TOO_LOW = 5,
      STREAM_ERROR_REASON_SEQ_DIVERGENT = 6,
      STREAM_ERROR_REASON_NOT_OWNER = 7,
      STREAM_ERROR_REASON_FORMAT_TOO_NEW = 8,
      STREAM_ERROR_REASON_OFFSET_TOO_HIGH = 9,
      STREAM_ERROR_REASON_TOO_LARGE = 10,
      STREAM_ERROR_REASON_INVALID = 11,
      STREAM_ERROR_REASON_STORAGE_UNAVAILABLE = 12,
      STREAM_ERROR_REASON_UNKNOWN_WRITER = 13,
      STREAM_ERROR_REASON_UNSUPPORTED = 14,
      STREAM_ERROR_REASON_ALREADY_EXISTS = 15,
      STREAM_ERROR_REASON_STORAGE_CORRUPT = 16,
    }
  }

  // temporal/api/stream/v1/message.proto
  namespace temporal.api.stream.v1 {
    interface IStreamRef {
      id?: string | null;
      owner?: temporal.api.common.v1.IExecution | null;
      backend?: string | null;
    }
    interface IProducerEpoch {
      workflow?: temporal.api.stream.v1.IWorkflowTaskEpoch | null;
      localActivity?: temporal.api.stream.v1.ILocalActivityEpoch | null;
      activity?: temporal.api.stream.v1.IActivityEpoch | null;
      standaloneActivity?: temporal.api.stream.v1.IActivityEpoch | null;
      client?: temporal.api.stream.v1.IClientEpoch | null;
      kind?:
        | "workflow"
        | "localActivity"
        | "activity"
        | "standaloneActivity"
        | "client";
    }
    interface IWorkflowTaskEpoch {
      startedEventId?: Long | null;
      attempt?: number | null;
      startedTime?: google.protobuf.ITimestamp | null;
    }
    interface ILocalActivityEpoch {
      task?: temporal.api.stream.v1.IWorkflowTaskEpoch | null;
      attempt?: number | null;
    }
    interface IActivityEpoch {
      attemptScheduledTime?: google.protobuf.ITimestamp | null;
      attempt?: number | null;
    }
    interface IClientEpoch {
      attempt?: number | null;
    }
    interface IProducer {
      id?: string | null;
      epoch?: temporal.api.stream.v1.IProducerEpoch | null;
      runId?: string | null;
    }
    interface IStreamWriter {
      producer?: temporal.api.stream.v1.IProducer | null;
      previousCompletedTask?: temporal.api.stream.v1.IWorkflowTaskEpoch | null;
      failedTasks?: Long[] | null;
      withdrawRun?: string | null;
    }
    interface IAppendRecord {
      body?: temporal.api.common.v1.IPayload | null;
    }
    interface IStreamSuperseded {
      writer?: temporal.api.stream.v1.IStreamWriter | null;
    }
    interface IStreamData {
      producer?: temporal.api.stream.v1.IProducer | null;
      body?: temporal.api.common.v1.IPayload | null;
    }
    interface IStreamRecord {
      offset?: Long | null;
      data?: temporal.api.stream.v1.IStreamData | null;
      superseded?: temporal.api.stream.v1.IStreamSuperseded | null;
      item?: "data" | "superseded";
    }
    interface IStreamErrorDetails {
      reason?: temporal.api.enums.v1.StreamErrorReason | null;
    }
  }

  // temporal/api/streamservice/v1/request_response.proto
  namespace temporal.api.streamservice.v1 {
    interface ICreateStreamRequest {
      namespace?: string | null;
      streamRef?: temporal.api.stream.v1.IStreamRef | null;
      retention?: google.protobuf.IDuration | null;
      withdrawRun?: string | null;
      owner?: temporal.api.stream.v1.IProducer | null;
      closeWithRun?: boolean | null;
      taskToken?: Uint8Array | null;
      idConflictPolicy?: temporal.api.enums.v1.StreamIdConflictPolicy | null;
      requestId?: string | null;
    }
    interface ICreateStreamResponse {
      ref?: temporal.api.stream.v1.IStreamRef | null;
      created?: boolean | null;
    }
    interface IAppendRecordsRequest {
      namespace?: string | null;
      ref?: temporal.api.stream.v1.IStreamRef | null;
      writer?: temporal.api.stream.v1.IStreamWriter | null;
      sequence?: Long | null;
      records?: temporal.api.stream.v1.IAppendRecord[] | null;
      deferWithdrawal?: boolean | null;
      taskToken?: Uint8Array | null;
    }
    interface IAppendRecordsResponse {
      firstOffset?: Long | null;
      lastOffset?: Long | null;
    }
    interface ICloseStreamRequest {
      namespace?: string | null;
      ref?: temporal.api.stream.v1.IStreamRef | null;
      writer?: temporal.api.stream.v1.IStreamWriter | null;
      taskToken?: Uint8Array | null;
    }
    interface ICloseStreamResponse {}
    interface IReadRecordsRequest {
      namespace?: string | null;
      ref?: temporal.api.stream.v1.IStreamRef | null;
      fromOffset?: Long | null;
      start?: temporal.api.enums.v1.StreamReadStart | null;
      maxRecords?: number | null;
      wait?: google.protobuf.IDuration | null;
      createWait?: google.protobuf.IDuration | null;
      toOffset?: Long | null;
      wakeOnly?: boolean | null;
      last?: number | null;
      maxBytes?: Long | null;
      floorFrom?: Long | null;
    }
    interface IReadRecordsResponse {
      records?: temporal.api.stream.v1.IStreamRecord[] | null;
      nextOffset?: Long | null;
      done?: boolean | null;
      continueAsNewSuggested?: boolean | null;
    }
    interface IDescribeStreamRequest {
      namespace?: string | null;
      ref?: temporal.api.stream.v1.IStreamRef | null;
    }
    interface IDescribeStreamResponse {
      status?: temporal.api.enums.v1.StreamStatus | null;
      lastOffset?: Long | null;
      retention?: google.protobuf.IDuration | null;
    }
  }
}
