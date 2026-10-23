// The protobufjs interfaces of descriptors/streamsample/v1/stream_service.proto, kept by hand.
// The generated code only uses them as types, since a model travels as a plain object.
import type Long from "long";
import type { google, temporal } from "@temporalio/proto";

export namespace streamsample {
  namespace v1 {
    interface IStreamRef {
      id?: string | null;
      workflowId?: string | null;
    }

    interface IAppendRecord {
      topic?: string | null;
      body?: temporal.api.common.v1.IPayload | null;
    }

    interface IStreamRecord {
      offset?: Long | null;
      topic?: string | null;
      body?: temporal.api.common.v1.IPayload | null;
    }

    interface ICreateStreamRequest {
      namespace?: string | null;
      ref?: IStreamRef | null;
      retention?: google.protobuf.IDuration | null;
    }

    enum StreamStatus {
      STREAM_STATUS_UNSPECIFIED = 0,
      STREAM_STATUS_OPEN = 1,
      STREAM_STATUS_CLOSED = 2,
      STREAM_STATUS_EXPIRED = 3,
    }

    interface ICreateStreamResponse {
      stream?: IStreamRef | null;
      created?: boolean | null;
      status?: StreamStatus | null;
    }

    interface IAppendRecordsRequest {
      namespace?: string | null;
      stream?: IStreamRef | null;
      sequence?: Long | null;
      records?: IAppendRecord[] | null;
    }

    interface IAppendRecordsResponse {
      firstOffset?: Long | null;
      lastOffset?: Long | null;
    }

    interface ICloseStreamRequest {
      namespace?: string | null;
      stream?: IStreamRef | null;
      result?: temporal.api.common.v1.IPayload | null;
    }

    interface ICloseStreamResponse {}

    interface IReadRecordsRequest {
      namespace?: string | null;
      stream?: IStreamRef | null;
      fromOffset?: Long | null;
      topics?: string[] | null;
      maxRecords?: number | null;
      wait?: google.protobuf.IDuration | null;
    }

    interface IReadRecordsResponse {
      records?: IStreamRecord[] | null;
      nextOffset?: Long | null;
      done?: boolean | null;
    }

    interface IDescribeStreamRequest {
      namespace?: string | null;
      stream?: IStreamRef | null;
    }

    interface IDescribeStreamResponse {
      lastOffset?: Long | null;
      retention?: google.protobuf.IDuration | null;
    }
  }
}
