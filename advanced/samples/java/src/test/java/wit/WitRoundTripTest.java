package wit;

import static org.junit.jupiter.api.Assertions.*;

import com.google.protobuf.ByteString;
import io.temporal.api.common.v1.Payload;
import io.temporal.api.common.v1.Payloads;
import io.temporal.api.failure.v1.Failure;
import io.temporal.common.converter.DefaultDataConverter;
import java.util.*;
import org.junit.jupiter.api.Test;
import wit.protogeneric.*;
import wit.protooneof.*;
import wit.typeshowcase.*;

class WitRoundTripTest {
  private static Payload opaquePayload(String value) {
    return Payload.newBuilder().putMetadata("encoding", ByteString.copyFromUtf8("unknown/custom"))
        .setData(ByteString.copyFromUtf8(value)).build();
  }

  @Test void nestedProtoModelsPreserveOpaquePayloadsWithoutTypeHints() {
    Payload output = opaquePayload("output");
    Payload context = opaquePayload("context");
    PayloadBackedEnvelope model = PayloadBackedEnvelope.newBuilder()
        .setProvider(PayloadBackedOutput.newBuilder().setDetails(output).build())
        .setScaler(PayloadBackedContext.newBuilder().setDetails(context).build()).build();
    PayloadBackedEnvelope decoded = PayloadBackedEnvelope.fromProto(model.toProto());
    assertEquals(output, decoded.getProvider().getDetails());
    assertEquals(context, decoded.getScaler().getDetails());
  }

  @Test void protobufOneofsSelectAndDecodeTheCorrectCase() {
    Payloads success = Payloads.newBuilder().addPayloads(opaquePayload("packed-success")).build();
    Outcome model = Outcome.newBuilder().setValue(OutcomeValue.success(success)).build();
    Outcome decoded = Outcome.fromProto(model.toProto());
    assertEquals(OutcomeValue.Tag.SUCCESS, decoded.getValue().getTag());
    assertEquals(success, decoded.getValue().getValue());
    PauseActivityRequest request = PauseActivityRequest.newBuilder().setActivity(ActivitySelection.type("activity")).build();
    assertEquals(ActivitySelection.Tag.TYPE, PauseActivityRequest.fromProto(request.toProto()).getActivity().getTag());
  }

  @Test void embeddedFailuresPreserveDetailsAndEncodedAttributes() {
    Payloads details = Payloads.newBuilder().addPayloads(opaquePayload("packed-details")).build();
    Failure failure = Failure.newBuilder().setMessage("opaque")
        .setEncodedAttributes(opaquePayload("attributes"))
        .setApplicationFailureInfo(io.temporal.api.failure.v1.ApplicationFailureInfo.newBuilder().setDetails(details))
        .setCause(Failure.newBuilder().setEncodedAttributes(opaquePayload("cause"))).build();
    Outcome model = Outcome.newBuilder().setValue(OutcomeValue.failure(failure)).build();
    assertEquals(failure, Outcome.fromProto(model.toProto()).getValue().getValue());
    wit.typeroundtrip.FailureContainer container = wit.typeroundtrip.FailureContainer.newBuilder().setFailure(failure).build();
    assertEquals(failure, wit.typeroundtrip.FailureContainer.fromProto(container.toProto()).getFailure());
  }

  @Test void copyBuildersPreserveNestedFieldsWhenSettingFlattenedValues() {
    wit.workflowservice.SignalWithStartWorkflowRequest original = wit.workflowservice.SignalWithStartWorkflowRequest.newBuilder()
        .setNamespace("namespace").setId("original").setStaticSummary(opaquePayload("summary")).build();
    wit.workflowservice.SignalWithStartWorkflowRequest copy = original.toBuilder().setId("copy").setStaticDetails(opaquePayload("details")).build();
    assertEquals("original", original.getId());
    assertEquals("copy", copy.getId());
    assertEquals(opaquePayload("summary"), copy.getUserMetadata().getStaticSummary());
    assertEquals(opaquePayload("details"), copy.getUserMetadata().getStaticDetails());
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
