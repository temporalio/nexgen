// The protobufjs interfaces and enums of the protos in descriptors/streams/, which no release of
// `@temporalio/proto` has yet. The generated code only uses them as types.
import type Long from "long";

declare module "@temporalio/proto" {
  // temporal/api/enums/v1/stream.proto
  namespace temporal.api.enums.v1 {
    enum StreamProducerKind {
      STREAM_PRODUCER_KIND_UNSPECIFIED = 0,
      STREAM_PRODUCER_KIND_WORKFLOW = 1,
      STREAM_PRODUCER_KIND_LOCAL_ACTIVITY = 2,
      STREAM_PRODUCER_KIND_ACTIVITY = 3,
      STREAM_PRODUCER_KIND_CLIENT = 4,
      STREAM_PRODUCER_KIND_STANDALONE_ACTIVITY = 5,
    }
    enum StreamCloseStatus {
      STREAM_CLOSE_STATUS_UNSPECIFIED = 0,
      STREAM_CLOSE_STATUS_COMPLETED = 1,
      STREAM_CLOSE_STATUS_FAILED = 2,
      STREAM_CLOSE_STATUS_CANCELED = 3,
      STREAM_CLOSE_STATUS_TERMINATED = 4,
      STREAM_CLOSE_STATUS_TIMED_OUT = 5,
    }
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
    enum StreamErrorReason {
      STREAM_ERROR_REASON_UNSPECIFIED = 0,
      STREAM_ERROR_REASON_NOT_FOUND = 1,
      STREAM_ERROR_REASON_EXPIRED = 2,
      STREAM_ERROR_REASON_CLOSED = 3,
      STREAM_ERROR_REASON_FENCED = 4,
      STREAM_ERROR_REASON_SEQUENCE = 5,
      STREAM_ERROR_REASON_SEQ_DIVERGENT = 6,
      STREAM_ERROR_REASON_NOT_OWNER = 7,
      STREAM_ERROR_REASON_FORMAT_TOO_NEW = 8,
      STREAM_ERROR_REASON_OFFSET = 9,
      STREAM_ERROR_REASON_TOO_LARGE = 10,
      STREAM_ERROR_REASON_INVALID = 11,
      STREAM_ERROR_REASON_STORAGE = 12,
      STREAM_ERROR_REASON_UNKNOWN_WRITER = 13,
      STREAM_ERROR_REASON_UNSUPPORTED = 14,
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
      taskStartedEventId?: Long | null;
      taskAttempt?: number | null;
      taskStartedTime?: google.protobuf.ITimestamp | null;
      attemptScheduledTime?: google.protobuf.ITimestamp | null;
      attempt?: number | null;
    }
    interface IProducer {
      kind?: temporal.api.enums.v1.StreamProducerKind | null;
      id?: string | null;
      epoch?: temporal.api.stream.v1.IProducerEpoch | null;
      runId?: string | null;
    }
    interface IWorkflowTaskEpoch {
      startedEventId?: Long | null;
      attempt?: number | null;
      startedTime?: google.protobuf.ITimestamp | null;
    }
    interface IAppendRecord {
      body?: temporal.api.common.v1.IPayload | null;
    }
    interface ISupersession {
      producerKind?: temporal.api.enums.v1.StreamProducerKind | null;
      producerId?: string | null;
      epoch?: temporal.api.stream.v1.IProducerEpoch | null;
      previousCompletedTask?: temporal.api.stream.v1.IWorkflowTaskEpoch | null;
      failedTasks?: Long[] | null;
      withdrawRun?: string | null;
      runId?: string | null;
    }
    interface IStreamData {
      producer?: temporal.api.stream.v1.IProducer | null;
      body?: temporal.api.common.v1.IPayload | null;
    }
    interface IStreamClose {
      producer?: temporal.api.stream.v1.IProducer | null;
      result?: temporal.api.common.v1.IPayload | null;
      failure?: temporal.api.failure.v1.IFailure | null;
      status?: temporal.api.enums.v1.StreamCloseStatus | null;
    }
    interface IStreamRecord {
      offset?: Long | null;
      data?: temporal.api.stream.v1.IStreamData | null;
      superseded?: temporal.api.stream.v1.ISupersession | null;
      close?: temporal.api.stream.v1.IStreamClose | null;
      item?: "data" | "superseded" | "close";
    }
    interface IStreamErrorDetails {
      reason?: temporal.api.enums.v1.StreamErrorReason | null;
    }
  }

  // temporal/api/streamservice/v1/request_response.proto
  namespace temporal.api.streamservice.v1 {
    interface ICreateStreamRequest {
      namespace?: string | null;
      streamId?: string | null;
      retention?: google.protobuf.IDuration | null;
      withdrawRun?: string | null;
      owner?: temporal.api.stream.v1.IProducer | null;
      closeWithRun?: boolean | null;
      ref?: temporal.api.stream.v1.IStreamRef | null;
      taskToken?: Uint8Array | null;
    }
    interface ICreateStreamResponse {
      stream?: temporal.api.stream.v1.IStreamRef | null;
      created?: boolean | null;
    }
    interface IAppendRecordsRequest {
      namespace?: string | null;
      stream?: temporal.api.stream.v1.IStreamRef | null;
      producer?: temporal.api.stream.v1.IProducer | null;
      sequence?: Long | null;
      records?: temporal.api.stream.v1.IAppendRecord[] | null;
      previousCompletedTask?: temporal.api.stream.v1.IWorkflowTaskEpoch | null;
      failedTasks?: Long[] | null;
      withdrawRun?: string | null;
      deferWithdrawal?: boolean | null;
      taskToken?: Uint8Array | null;
    }
    interface IAppendRecordsResponse {
      firstOffset?: Long | null;
      lastOffset?: Long | null;
    }
    interface ICloseStreamRequest {
      namespace?: string | null;
      stream?: temporal.api.stream.v1.IStreamRef | null;
      producer?: temporal.api.stream.v1.IProducer | null;
      result?: temporal.api.common.v1.IPayload | null;
      failure?: temporal.api.failure.v1.IFailure | null;
      previousCompletedTask?: temporal.api.stream.v1.IWorkflowTaskEpoch | null;
      failedTasks?: Long[] | null;
      withdrawRun?: string | null;
      status?: temporal.api.enums.v1.StreamCloseStatus | null;
      taskToken?: Uint8Array | null;
    }
    interface ICloseStreamResponse {}
    interface IReadRecordsRequest {
      namespace?: string | null;
      stream?: temporal.api.stream.v1.IStreamRef | null;
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
      stream?: temporal.api.stream.v1.IStreamRef | null;
    }
    interface IDescribeStreamResponse {
      status?: temporal.api.enums.v1.StreamStatus | null;
      lastOffset?: Long | null;
      retention?: google.protobuf.IDuration | null;
    }
  }
}
