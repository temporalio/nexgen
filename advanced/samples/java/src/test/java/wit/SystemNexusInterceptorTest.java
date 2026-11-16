package wit;

import static org.junit.jupiter.api.Assertions.*;

import io.temporal.common.interceptors.WorkflowOutboundCallsInterceptor.ExecuteNexusOperationInput;
import io.temporal.common.interceptors.WorkflowOutboundCallsInterceptor.ExecuteNexusOperationOutput;
import io.temporal.payload.context.WorkflowSerializationContext;
import io.temporal.workflow.NexusOperationOptions;
import java.util.*;
import org.junit.jupiter.api.Test;
import wit.multioperationservice.*;

class SystemNexusInterceptorTest {
  @Test
  void registryDispatchesTypedHooksThroughForwardingBases() {
    List<String> calls = new ArrayList<>();
    ExecuteNexusOperationOutput<Response> output = new ExecuteNexusOperationOutput<>(null, null);
    SystemNexusWorkflowOutboundInterceptor terminal =
        new SystemNexusWorkflowOutboundInterceptor() {
          @Override
          @SuppressWarnings("unchecked")
          public <R> ExecuteNexusOperationOutput<R> executeNexusOperation(
              ExecuteNexusOperationInput<R> input) {
            calls.add("generic:" + input.getOperation());
            return (ExecuteNexusOperationOutput<R>) output;
          }

          @Override
          public ExecuteNexusOperationOutput<Response> executeFirstOperation(
              ExecuteNexusOperationInput<Response> input) {
            calls.add("typed:first");
            return executeNexusOperation(input);
          }
        };
    SystemNexusWorkflowOutboundInterceptor forwarding =
        new SystemNexusWorkflowOutboundInterceptor.Base() {
          @Override
          protected SystemNexusWorkflowOutboundInterceptor nextSystemNexusInterceptor() {
            return terminal;
          }

          @Override
          public <R> ExecuteNexusOperationOutput<R> executeNexusOperation(
              ExecuteNexusOperationInput<R> input) {
            fail("Typed forwarding must preserve the next interceptor's typed hook");
            return null;
          }
        };
    assertSame(output, SystemNexusOperations.execute(forwarding, input("FirstOperation")));
    assertSame(output, SystemNexusOperations.execute(forwarding, input("SecondOperation")));
    assertEquals(
        Arrays.asList("typed:first", "generic:FirstOperation", "generic:SecondOperation"), calls);
    assertThrows(
        IllegalArgumentException.class,
        () -> SystemNexusOperations.execute(forwarding, input("Unknown")));
  }

  @Test
  void contextSelectionUsesTheResolvedDestination() {
    wit.workflowservice.SignalWithStartWorkflowRequest request =
        wit.workflowservice.SignalWithStartWorkflowRequest.newBuilder()
            .setNamespace("destination")
            .setId("target")
            .build();
    WorkflowSerializationContext context =
        (WorkflowSerializationContext)
            wit.workflowservice.SystemNexusOperations.serializationContext(
                "temporal.api.workflowservice.v1.WorkflowService",
                "SignalWithStartWorkflowExecution",
                request);
    assertEquals("destination", context.getNamespace());
    assertEquals("target", context.getWorkflowId());
    assertNull(SystemNexusOperations.serializationContext("unknown", "unknown", request));
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
