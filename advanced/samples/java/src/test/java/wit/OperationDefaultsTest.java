package wit;

import static org.junit.jupiter.api.Assertions.*;

import io.temporal.client.WorkflowOptions;
import io.temporal.common.interceptors.*;
import io.temporal.testing.TestEnvironmentOptions;
import io.temporal.testing.TestWorkflowEnvironment;
import io.temporal.worker.Worker;
import io.temporal.worker.WorkerFactoryOptions;
import io.temporal.workflow.*;
import java.util.Optional;
import org.junit.jupiter.api.Test;
import wit.defaults.*;

class OperationDefaultsTest {
  @Test
  void defaultsAreResolvedByOperationsAndResourceMethodsNotModels() {
    assertNull(Request.newBuilder().build().getMode());
    assertEquals("", RecordWithDefaults.newBuilder().build().getLabel());
    assertNull(
        wit.workflowservice.SignalWithStartWorkflowOptions.newBuilder().build().getTaskQueue());
    try (TestWorkflowEnvironment environment =
        TestWorkflowEnvironment.newInstance(
            TestEnvironmentOptions.newBuilder()
                .setWorkerFactoryOptions(
                    WorkerFactoryOptions.newBuilder()
                        .setWorkerInterceptors(new EchoRequests())
                        .build())
                .build())) {
      Worker worker = environment.newWorker("defaults");
      worker.registerWorkflowImplementationTypes(CallerImpl.class);
      environment.start();
      Caller caller =
          environment
              .getWorkflowClient()
              .newWorkflowStub(
                  Caller.class, WorkflowOptions.newBuilder().setTaskQueue("defaults").build());
      assertEquals("done", caller.run());
    }
  }

  @WorkflowInterface
  public interface Caller {
    @WorkflowMethod
    String run();
  }

  public static class CallerImpl implements Caller {
    @Override
    public String run() {
      SubmitOptions options = SubmitOptions.newBuilder().build();
      Request defaults = DefaultsServiceOperations.submit(options);
      assertEquals(Integer.valueOf(3), defaults.getAttempts());
      assertEquals(Boolean.TRUE, defaults.getEnabled());
      assertEquals(Double.valueOf(1.5), defaults.getRatio());
      assertEquals(MyEnum.STANDARD, defaults.getMode());
      assertEquals("alias", defaults.getAliasLabel());
      assertEquals(Workflow.getInfo().getNamespace(), defaults.getNamespace());
      assertNull(defaults.getMetadata());
      assertNull(options.getAttempts());

      RecordWithDefaults retry =
          RecordWithDefaults.newBuilder().setAttempts(0).setLabel(null).build();
      Request explicit =
          DefaultsServiceOperations.submit(
              SubmitOptions.newBuilder()
                  .setAttempts(0)
                  .setEnabled(false)
                  .setDetails("details")
                  .setRetry(retry)
                  .build());
      assertEquals(Integer.valueOf(0), explicit.getAttempts());
      assertEquals(Boolean.FALSE, explicit.getEnabled());
      assertEquals("untitled", explicit.getMetadata().getSummary());
      assertEquals("details", explicit.getMetadata().getDetails());
      assertSame(retry, explicit.getRetry());
      assertNull(explicit.getRetry().getLabel());

      new Sender("token").submit();
      wit.workflowservice.SignalWithStartWorkflowOptions start =
          wit.workflowservice.SignalWithStartWorkflowOptions.newBuilder()
              .setWorkflowId("target")
              .build();
      wit.workflowservice.WorkflowOperations.signalWithStartWorkflow(
          "Target", new Object[0], "signal", new Object[0], start);
      wit.workflowservice.WorkflowOperations.signalWithStartWorkflow(
          "Target",
          new Object[0],
          "signal",
          new Object[0],
          start.toBuilder().setWorkflowId("explicit-target").setTaskQueue("explicit").build());
      assertNull(start.getTaskQueue());
      return "done";
    }
  }

  private static class EchoRequests extends WorkerInterceptorBase {
    @Override
    public WorkflowInboundCallsInterceptor interceptWorkflow(WorkflowInboundCallsInterceptor next) {
      return new WorkflowInboundCallsInterceptorBase(next) {
        @Override
        public void init(WorkflowOutboundCallsInterceptor outbound) {
          super.init(
              new WorkflowOutboundCallsInterceptorBase(outbound) {
                @Override
                @SuppressWarnings("unchecked")
                public <R> ExecuteNexusOperationOutput<R> executeNexusOperation(
                    ExecuteNexusOperationInput<R> input) {
                  if (input.getArg()
                      instanceof wit.workflowservice.SignalWithStartWorkflowRequest) {
                    wit.workflowservice.SignalWithStartWorkflowRequest request =
                        (wit.workflowservice.SignalWithStartWorkflowRequest) input.getArg();
                    assertEquals(
                        request.getWorkflowId().equals("explicit-target")
                            ? "explicit"
                            : Workflow.getInfo().getTaskQueue(),
                        request.getTaskQueue());
                    return new ExecuteNexusOperationOutput<>(
                        Workflow.newPromise(
                            (R)
                                wit.workflowservice.SignalWithStartWorkflowResponse.newBuilder()
                                    .setRunId("run")
                                    .build()),
                        Workflow.newPromise(Optional::empty));
                  }
                  Request request = (Request) input.getArg();
                  if ("SubmitFromSender".equals(input.getOperation())) {
                    assertEquals(Integer.valueOf(3), request.getAttempts());
                    assertEquals(MyEnum.STANDARD, request.getMode());
                    assertEquals(Workflow.getInfo().getNamespace(), request.getNamespace());
                  }
                  return new ExecuteNexusOperationOutput<>(
                      Workflow.newPromise((R) request), Workflow.newPromise(Optional::empty));
                }
              });
        }
      };
    }
  }
}
