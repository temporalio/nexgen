package wit;

import static org.junit.jupiter.api.Assertions.*;

import io.temporal.api.enums.v1.WorkflowIdConflictPolicy;
import io.temporal.api.enums.v1.WorkflowIdReusePolicy;
import io.temporal.common.RetryOptions;
import io.temporal.common.SearchAttributes;
import java.lang.reflect.Method;
import java.lang.reflect.Modifier;
import java.lang.reflect.TypeVariable;
import java.time.Duration;
import org.junit.jupiter.api.Test;
import wit.protobackedshowcase.CallbackInternalOperationOptions;
import wit.workflowservice.SignalWithStartWorkflowOptions;
import wit.workflowservice.WorkflowOperations;

class OptionsApiTest {
  @Test
  void workflowOptionsUseSdkNamesAndValueSemantics() {
    RetryOptions retry = RetryOptions.newBuilder().setMaximumAttempts(3).build();
    SignalWithStartWorkflowOptions options =
        SignalWithStartWorkflowOptions.newBuilder()
            .setWorkflowId("target")
            .setWorkflowExecutionTimeout(Duration.ofHours(1))
            .setWorkflowRunTimeout(Duration.ofMinutes(5))
            .setWorkflowTaskTimeout(Duration.ofSeconds(10))
            .setWorkflowIdReusePolicy(
                WorkflowIdReusePolicy.WORKFLOW_ID_REUSE_POLICY_ALLOW_DUPLICATE)
            .setWorkflowIdConflictPolicy(
                WorkflowIdConflictPolicy.WORKFLOW_ID_CONFLICT_POLICY_USE_EXISTING)
            .setRetryOptions(retry)
            .setTypedSearchAttributes(SearchAttributes.newBuilder().build())
            .build();
    SignalWithStartWorkflowOptions copy = options.toBuilder().build();
    assertEquals(options, copy);
    assertEquals(options.hashCode(), copy.hashCode());
    assertNotEquals(options, copy.toBuilder().setWorkflowId("other").build());
    assertNotEquals(options, null);
    assertNotEquals(options, "target");
    assertEquals(retry, copy.getRetryOptions());
    assertEquals(Duration.ofMinutes(5), copy.getWorkflowRunTimeout());
    assertTrue(copy.toString().contains("workflowId=target"));
    assertEquals(
        SignalWithStartWorkflowOptions.newBuilder().build(),
        SignalWithStartWorkflowOptions.getDefaultInstance());
  }

  @Test
  void byteArrayOptionsCompareContents() {
    CallbackInternalOperationOptions first =
        CallbackInternalOperationOptions.newBuilder().setData(new byte[] {1, 2}).build();
    CallbackInternalOperationOptions second =
        CallbackInternalOperationOptions.newBuilder().setData(new byte[] {1, 2}).build();
    assertEquals(first, second);
    assertEquals(first.hashCode(), second.hashCode());
    assertNotEquals(
        first, CallbackInternalOperationOptions.newBuilder().setData(new byte[] {2, 1}).build());
    assertTrue(first.toString().contains("data=[1, 2]"));
  }

  @Test
  void callableTypeParametersFollowJavaConventions() {
    for (Method method : WorkflowOperations.class.getDeclaredMethods()) {
      for (TypeVariable<?> parameter : method.getTypeParameters()) {
        String name = parameter.getName();
        assertEquals(1, name.length());
        assertTrue(Character.isUpperCase(name.charAt(0)));
      }
    }
  }

  @Test
  void helperOnlyRuntimeIsNotPublic() throws Exception {
    assertFalse(
        Modifier.isPublic(Class.forName("wit.workflowservice.NexgenRuntime").getModifiers()));
  }
}
