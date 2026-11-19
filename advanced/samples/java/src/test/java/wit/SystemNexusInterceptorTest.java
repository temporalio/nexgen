package wit;

import static org.junit.jupiter.api.Assertions.*;

import io.temporal.api.common.v1.Payload;
import io.temporal.common.interceptors.WorkflowOutboundCallsInterceptor.ExecuteNexusOperationInput;
import io.temporal.common.interceptors.WorkflowOutboundCallsInterceptor.ExecuteNexusOperationOutput;
import io.temporal.payload.context.SerializationContext;
import io.temporal.payload.context.WorkflowSerializationContext;
import io.temporal.workflow.NexusOperationOptions;
import java.util.*;
import org.junit.jupiter.api.Test;
import wit.multioperationservice.*;
import wit.multioperationservice.internal.SystemNexusOperations;

class SystemNexusInterceptorTest {
  @Test
  void registryDispatchesTypedHooksThroughDirectImplementations() {
    List<String> calls = new ArrayList<>();
    ExecuteNexusOperationOutput<Response> output = new ExecuteNexusOperationOutput<>(null, null);
    SystemNexusWorkflowOutboundInterceptor terminal =
        new SystemNexusWorkflowOutboundInterceptor.Terminal() {
          @Override
          @SuppressWarnings("unchecked")
          protected <R> ExecuteNexusOperationOutput<R> executeSystemNexusOperation(
              ExecuteNexusOperationInput<R> input) {
            calls.add("transport:" + input.getOperation());
            return (ExecuteNexusOperationOutput<R>) output;
          }

          @Override
          public ExecuteNexusOperationOutput<Response> executeFirstOperation(
              ExecuteNexusOperationInput<Response> input) {
            calls.add("typed:first");
            return super.executeFirstOperation(input);
          }

          public <R> ExecuteNexusOperationOutput<R> executeNexusOperation(
              ExecuteNexusOperationInput<R> input) {
            fail("The generated terminal must use the dedicated system transport");
            return null;
          }
        };
    SystemNexusWorkflowOutboundInterceptor base =
        new SystemNexusWorkflowOutboundInterceptor.Base() {
          @Override
          protected SystemNexusWorkflowOutboundInterceptor nextSystemNexusInterceptor() {
            return terminal;
          }

          public <R> ExecuteNexusOperationOutput<R> executeNexusOperation(
              ExecuteNexusOperationInput<R> input) {
            fail("System forwarding must bypass ordinary Nexus hooks");
            return null;
          }
        };
    SystemNexusWorkflowOutboundInterceptor forwarding =
        new SystemNexusWorkflowOutboundInterceptor() {
          @Override
          public ExecuteNexusOperationOutput<Response> executeFirstOperation(
              ExecuteNexusOperationInput<Response> input) {
            return base.executeFirstOperation(input);
          }

          @Override
          public ExecuteNexusOperationOutput<Response> executeSecondOperation(
              ExecuteNexusOperationInput<Response> input) {
            return base.executeSecondOperation(input);
          }
        };
    assertSame(output, SystemNexusOperations.execute(forwarding, input("FirstOperation")));
    assertSame(output, SystemNexusOperations.execute(forwarding, input("SecondOperation")));
    assertEquals(
        Arrays.asList("typed:first", "transport:FirstOperation", "transport:SecondOperation"),
        calls);
    assertThrows(
        IllegalArgumentException.class,
        () -> SystemNexusOperations.execute(forwarding, input("Unknown")));
  }

  @Test
  void contextSelectionUsesTheResolvedDestination() {
    wit.workflowservice.SignalWithStartWorkflowRequest request =
        wit.workflowservice.SignalWithStartWorkflowRequest.newBuilder()
            .setNamespace("destination")
            .setWorkflowId("target")
            .build();
    WorkflowSerializationContext context =
        (WorkflowSerializationContext)
            wit.workflowservice.internal.SystemNexusOperations.serializationContext(
                "temporal.api.workflowservice.v1.WorkflowService",
                "SignalWithStartWorkflowExecution",
                request);
    assertEquals("destination", context.getNamespace());
    assertEquals("target", context.getWorkflowId());
    assertNull(SystemNexusOperations.serializationContext("unknown", "unknown", request));
  }

  @Test
  void generatedTerminalPreparesHeadersAfterInterceptionWithoutMutatingTheInput() {
    Payload original =
        Payload.newBuilder().setData(com.google.protobuf.ByteString.copyFromUtf8("manual")).build();
    Payload propagated =
        Payload.newBuilder()
            .setData(com.google.protobuf.ByteString.copyFromUtf8("context"))
            .build();
    wit.workflowservice.SignalWithStartWorkflowRequest request =
        wit.workflowservice.SignalWithStartWorkflowRequest.newBuilder()
            .setNamespace("destination")
            .setWorkflowId("target")
            .setHeaders(Collections.singletonMap("manual", original))
            .build();
    ExecuteNexusOperationInput<wit.workflowservice.SignalWithStartWorkflowResponse> input =
        new ExecuteNexusOperationInput<>(
            "__temporal_system",
            "temporal.api.workflowservice.v1.WorkflowService",
            "SignalWithStartWorkflowExecution",
            wit.workflowservice.SignalWithStartWorkflowResponse.class,
            wit.workflowservice.SignalWithStartWorkflowResponse.class,
            request,
            NexusOperationOptions.newBuilder().build(),
            Collections.singletonMap("trace", "outer"));
    List<String> calls = new ArrayList<>();
    wit.workflowservice.SystemNexusWorkflowOutboundInterceptor terminal =
        new wit.workflowservice.SystemNexusWorkflowOutboundInterceptor.Terminal() {
          @Override
          protected Map<String, Payload> propagateSystemNexusHeaders(
              Map<String, Payload> headers, SerializationContext context) {
            calls.add("headers");
            assertEquals("destination", ((WorkflowSerializationContext) context).getNamespace());
            assertEquals("target", ((WorkflowSerializationContext) context).getWorkflowId());
            Map<String, Payload> merged = new HashMap<>(headers);
            merged.put("propagated", propagated);
            return merged;
          }

          @Override
          protected <R> ExecuteNexusOperationOutput<R> executeSystemNexusOperation(
              ExecuteNexusOperationInput<R> forwarded) {
            calls.add("transport");
            assertEquals(input.getEndpoint(), forwarded.getEndpoint());
            assertEquals(input.getService(), forwarded.getService());
            assertEquals(input.getOperation(), forwarded.getOperation());
            assertSame(input.getResultClass(), forwarded.getResultClass());
            assertSame(input.getResultType(), forwarded.getResultType());
            assertSame(input.getOptions(), forwarded.getOptions());
            assertEquals(input.getHeaders(), forwarded.getHeaders());
            wit.workflowservice.SignalWithStartWorkflowRequest prepared =
                (wit.workflowservice.SignalWithStartWorkflowRequest) forwarded.getArg();
            assertNotSame(request, prepared);
            assertEquals(original, prepared.getHeaders().get("manual"));
            assertEquals(propagated, prepared.getHeaders().get("propagated"));
            return new ExecuteNexusOperationOutput<>(null, null);
          }
        };
    wit.workflowservice.internal.SystemNexusOperations.execute(terminal, input);
    assertEquals(Arrays.asList("headers", "transport"), calls);
    assertEquals(Collections.singletonMap("manual", original), request.getHeaders());
    assertSame(request, input.getArg());
  }

  private static ExecuteNexusOperationInput<Response> input(String operation) {
    return new ExecuteNexusOperationInput<>(
        "__temporal_system",
        "InterceptorService",
        operation,
        Response.class,
        Response.class,
        Request.newBuilder().setId("id").build(),
        NexusOperationOptions.newBuilder().build(),
        Collections.emptyMap());
  }
}
