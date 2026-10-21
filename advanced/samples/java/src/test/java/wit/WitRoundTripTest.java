package wit;

import static org.junit.jupiter.api.Assertions.*;

import com.fasterxml.jackson.core.type.TypeReference;
import io.temporal.common.converter.DefaultDataConverter;
import java.util.*;
import org.junit.jupiter.api.Test;
import wit.protogeneric.*;
import wit.protooneof.*;
import wit.typeshowcase.*;

class WitRoundTripTest {
  @Test void requiredGenericPayloadRetainsNull() {
    PayloadBackedOutput<Object> model = PayloadBackedOutput.<Object>newBuilder()
        .setDetails(null).build();
    io.temporal.api.compute.v1.ComputeProvider wire = model.toProto(DefaultDataConverter.STANDARD_INSTANCE);
    assertTrue(wire.hasDetails());
    assertEquals("binary/null", wire.getDetails().getMetadataOrThrow("encoding").toStringUtf8());
    assertNull(PayloadBackedOutput.fromProto(wire, DefaultDataConverter.STANDARD_INSTANCE,
        PayloadBackedOutput.class).getDetails());
  }

  @Test void nestedGenericsRetainTheirConcretePayloadTypes() {
    PayloadBackedEnvelope<String, List<Integer>> model = PayloadBackedEnvelope.<String, List<Integer>>newBuilder()
        .setProvider(PayloadBackedOutput.<String>newBuilder().setDetails("output").build())
        .setScaler(PayloadBackedContext.<List<Integer>>newBuilder().setDetails(Arrays.asList(1, 2)).build()).build();
    java.lang.reflect.Type type = new TypeReference<PayloadBackedEnvelope<String, List<Integer>>>() {}.getType();
    PayloadBackedEnvelope<String, List<Integer>> decoded = PayloadBackedEnvelope.fromProto(model.toProto(DefaultDataConverter.STANDARD_INSTANCE), DefaultDataConverter.STANDARD_INSTANCE, type);
    assertEquals("output", decoded.getProvider().getDetails());
    assertEquals(Arrays.asList(1, 2), decoded.getScaler().getDetails());
    assertInstanceOf(Integer.class, decoded.getScaler().getDetails().get(0));
  }

  @Test void protobufOneofsSelectAndDecodeTheCorrectCase() {
    Outcome<String> model = Outcome.<String>newBuilder().setValue(OutcomeValue.success("success")).build();
    Outcome<String> decoded = Outcome.fromProto(model.toProto(DefaultDataConverter.STANDARD_INSTANCE), DefaultDataConverter.STANDARD_INSTANCE, new TypeReference<Outcome<String>>() {}.getType());
    assertEquals(OutcomeValue.Tag.SUCCESS, decoded.getValue().getTag());
    assertEquals("success", decoded.getValue().getValue());
    PauseActivityRequest request = PauseActivityRequest.newBuilder().setActivity(ActivitySelection.type("activity")).build();
    assertEquals(ActivitySelection.Tag.TYPE, PauseActivityRequest.fromProto(request.toProto(DefaultDataConverter.STANDARD_INSTANCE), DefaultDataConverter.STANDARD_INSTANCE, PauseActivityRequest.class).getActivity().getTag());
  }

  @Test void genericListValueIsOnePayloadRatherThanASequenceOfArguments() {
    Outcome<List<Integer>> model = Outcome.<List<Integer>>newBuilder().setValue(OutcomeValue.success(Arrays.asList(1, 2))).build();
    Outcome<List<Integer>> decoded = Outcome.fromProto(model.toProto(DefaultDataConverter.STANDARD_INSTANCE), DefaultDataConverter.STANDARD_INSTANCE, new TypeReference<Outcome<List<Integer>>>() {}.getType());
    assertEquals(Arrays.asList(1, 2), decoded.getValue().getValue());
  }

  @Test void copyBuildersPreserveNestedFieldsWhenSettingFlattenedValues() {
    wit.workflowservice.SignalWithStartWorkflowRequest original = wit.workflowservice.SignalWithStartWorkflowRequest.newBuilder()
        .setNamespace("namespace").setId("original").setStaticSummary("summary").build();
    wit.workflowservice.SignalWithStartWorkflowRequest copy = original.toBuilder().setId("copy").setStaticDetails("details").build();
    assertEquals("original", original.getId());
    assertEquals("copy", copy.getId());
    assertEquals("summary", copy.getUserMetadata().getStaticSummary());
    assertEquals("details", copy.getUserMetadata().getStaticDetails());
  }

  @Test void functionReferencesSupportTypedCallbacksAndNames() {
    wit.functionexecution.ExecuteNamedFunctionRequest callback = wit.functionexecution.ExecuteNamedFunctionRequest.newBuilder()
        .setFunctionCall((name, enabled) -> name + enabled).setName("name").setEnabled(true).build();
    assertEquals("nametrue", callback.getFunction().getCallable().apply("name", true));
    assertEquals("named", callback.toBuilder().setFunctionName("named").build().getFunction().getName());
  }

  @Test void resourceValuesCanBeDecodedFromJson() {
    wit.userservice.User value = new wit.userservice.User("id", "email", "endpoint");
    wit.userservice.User decoded = DefaultDataConverter.STANDARD_INSTANCE.fromPayload(DefaultDataConverter.STANDARD_INSTANCE.toPayload(value).get(), wit.userservice.User.class, wit.userservice.User.class);
    assertEquals("id", decoded.getUserId());
    assertEquals("email", decoded.getEmail());
  }

  @Test void jsonModelsRoundTripVariantsResultsFlagsTuplesAndCollections() {
    UserProfile profile = UserProfile.newBuilder().setTags(Arrays.asList("one", "two"))
        .setMetadata(Collections.singletonMap("key", "value"))
        .setCapabilities(EnumSet.of(UserCapability.READ_PROFILE))
        .setSyncState(wit.typeshowcase.NexgenRuntime.Result.ok("ready"))
        .setNotificationTarget(NotificationTarget.email("a@example.com"))
        .setAddress(PostalAddress.newBuilder().setStreet("street").setCity("city").setCountry("country")
            .setCoordinates(new wit.typeshowcase.NexgenRuntime.Tuple2<>(1.0, 2.0)).build()).build();
    UserProfile decoded = DefaultDataConverter.STANDARD_INSTANCE.fromPayload(DefaultDataConverter.STANDARD_INSTANCE.toPayload(profile).get(), UserProfile.class, UserProfile.class);
    assertEquals(profile.getTags(), decoded.getTags());
    assertEquals(profile.getMetadata(), decoded.getMetadata());
    assertEquals(profile.getCapabilities(), decoded.getCapabilities());
    assertEquals("ready", decoded.getSyncState().getOk());
    assertEquals("a@example.com", decoded.getNotificationTarget().getValue());
    assertEquals(1.0, decoded.getAddress().getCoordinates().item0);
  }
}
