package jsonschema;

import static org.junit.jupiter.api.Assertions.assertArrayEquals;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.google.protobuf.ByteString;
import io.temporal.api.common.v1.Payload;
import io.temporal.common.converter.DataConverter;
import io.temporal.common.converter.DefaultDataConverter;
import io.temporal.failure.ApplicationFailure;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.Map;
import org.junit.jupiter.api.Test;

import json_schema.definitions.showcase.Address;
import json_schema.definitions.showcase.Attributes;
import json_schema.definitions.showcase.BlobIndex;
import json_schema.definitions.showcase.Circle;
import json_schema.definitions.showcase.ContactJava;
import json_schema.definitions.showcase.Extras;
import json_schema.definitions.showcase.Labels;
import json_schema.definitions.showcase.ItemRules;
import json_schema.definitions.showcase.LinkNote;
import json_schema.definitions.showcase.Metrics;
import json_schema.definitions.showcase.Nicknames;
import json_schema.definitions.showcase.Quotas;
import json_schema.definitions.showcase.Settings;
import json_schema.definitions.showcase.Showcase;
import json_schema.definitions.showcase.ShowcaseDetailObject;
import json_schema.definitions.showcase.ShowcaseLedgerValue;
import json_schema.definitions.showcase.Tokens;
import json_schema.definitions.showcase.ShowcaseSegmentsItem;
import json_schema.definitions.showcase.Square;
import json_schema.definitions.showcase.TextNote;
import json_schema.definitions.showcase.Widget;

/**
 * Round-trips the showcase wire fixtures through Temporal's default data
 * converter. The showcase schema is a single pure JSON Schema file (no service)
 * exercising the whole supported keyword subset.
 *
 * <p>Equality is checked on parsed JSON trees (the {@code toPayload} bytes are
 * compact while the fixtures are pretty-printed). Java conservatively omits
 * explicit nulls on optional+nullable fields (see nullability.md), so
 * showcase-nulls.json (which carries {@code middleName: null}) is verified by
 * deserialization only.
 */
final class JsonSchemaShowcaseRoundTripTest {

    private static final ObjectMapper MAPPER = new ObjectMapper();
    private static final DataConverter CONVERTER = DefaultDataConverter.newDefaultInstance();

    private static Path fixtureDir(String suite) {
        return Paths.get(System.getProperty("user.dir"), "..", "wire", "json_schema", suite)
                .normalize();
    }

    private static byte[] fixtureBytes(String suite, String name) throws IOException {
        return Files.readAllBytes(fixtureDir(suite).resolve(name));
    }

    private static Payload jsonPayload(byte[] data) {
        return Payload.newBuilder()
                .putMetadata("encoding", ByteString.copyFromUtf8("json/plain"))
                .setData(ByteString.copyFrom(data))
                .build();
    }

    private static <T> T decode(String name, Class<T> type) throws IOException {
        return CONVERTER.fromPayload(jsonPayload(fixtureBytes("showcase", name)), type, type);
    }

    private static <T> T roundTrip(String name, Class<T> type) throws IOException {
        T value = decode(name, type);
        Payload encoded = CONVERTER.toPayload(value).orElseThrow(AssertionError::new);
        JsonNode got = MAPPER.readTree(encoded.getData().toByteArray());
        JsonNode want = MAPPER.readTree(fixtureBytes("showcase", name));
        assertEquals(want, got, name);
        return value;
    }

    @Test
    void showcaseFixturesRoundTrip() throws IOException {
        Showcase minimal = roundTrip("showcase-minimal.json", Showcase.class);
        assertEquals("showcase", minimal.getKind().getValue());
        assertEquals("showcase", Showcase.Kind.SHOWCASE.getValue());
        // Closed value-set fields (const on integer/boolean, enum on string/
        // integer/number) round-trip to their in-memory value-class constants.
        assertEquals(1L, minimal.getRevision().getValue());
        assertEquals(1L, Showcase.Revision.REVISION_JAVA.getValue());
        assertEquals(Showcase.Revision.REVISION_JAVA, minimal.getRevision());
        assertTrue(minimal.getEnabled().getValue());
        assertEquals("active", minimal.getStatus().getValue());
        assertEquals(Showcase.Status.ACTIVE_JAVA, minimal.getStatus());
        assertEquals(1L, minimal.getTier().getValue());
        assertEquals(1.5, minimal.getScale().getValue());
        assertEquals("Widget", minimal.getName());
        assertEquals(3L, minimal.getCount());
        assertTrue(minimal.getActive());
        assertNull(minimal.getRetries());
        assertEquals(3L, minimal.getRetriesOrDefault());
        // Scalar defaults of each kind: null field (unset on the wire), surfaced
        // on read via the generated getter default (materialize-on-read).
        assertNull(minimal.getGreeting());
        assertEquals("hello", minimal.getGreetingOrDefault());
        assertNull(minimal.getDebug());
        assertFalse(minimal.getDebugOrDefault());
        assertEquals("tools", minimal.getCategory());

        Showcase full = roundTrip("showcase-full.json", Showcase.class);
        assertEquals(42L, full.getCount());
        assertNotNull(full.getRetries());
        assertEquals(5L, full.getRetries());
        assertEquals("Q", full.getMiddleName());
        assertNotNull(full.getTags());
        assertEquals(2, full.getTags().size());
        assertEquals(java.util.Arrays.asList("alpha", "beta"), full.getAliases());
        assertEquals(java.util.Arrays.asList("admin", "user"), full.getRoles());
        assertNotNull(full.getAddress());
        assertEquals("1 Main St", full.getAddress().getStreet());
        assertTrue(full.getAddress().getAdditionalProperties().containsKey("region"));
        assertNotNull(full.getLabels());
        assertEquals("prod", full.getLabels().getAdditionalProperties().get("env"));
        assertNotNull(full.getSettings());
        assertEquals(14L, full.getSettings().getFontSize());

        // showcase-nulls carries middleName: null (optional+nullable), which Java
        // collapses on serialize, so only deserialization is checked.
        Showcase nulls = decode("showcase-nulls.json", Showcase.class);
        assertNull(nulls.getMiddleName());
        assertNull(nulls.getCategory());
        assertFalse(nulls.getActive());

        Address address = roundTrip("address-open.json", Address.class);
        assertEquals("1 Main St", address.getStreet());
        assertTrue(address.getAdditionalProperties().containsKey("x-extra"));

        Labels labels = roundTrip("labels.json", Labels.class);
        assertEquals("prod", labels.getAdditionalProperties().get("env"));
        assertEquals("core", labels.getAdditionalProperties().get("team"));

        Settings settings = roundTrip("settings.json", Settings.class);
        assertEquals("dark", settings.getTheme());
        assertEquals(14L, settings.getFontSize());

        // Numeric-constrained fields round-trip (integer bounds/multipleOf and a
        // number field with minimum + multipleOf).
        Showcase metrics = roundTrip("showcase-metrics.json", Showcase.class);
        assertEquals(5L, metrics.getPriority());
        assertEquals(2L, metrics.getLevel());
        assertEquals(15.0, metrics.getRatio());
        assertEquals(9L, metrics.getStep());

        // String-length-constrained fields round-trip. The astral crux: "a😀b"
        // is 3 code points but 6 UTF-8 bytes / 4 UTF-16 units; it passes code
        // (maxLength:5) only because length is counted in code points.
        Showcase strings = roundTrip("showcase-strings.json", Showcase.class);
        assertEquals("a😀b", strings.getCode());
        assertEquals("buddy", strings.getNickname());
    }

    @Test
    void invalidValuesAreRejected() {
        // Wrong const value on the root object.
        RuntimeException badConst = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(
                                ("{\"kind\":\"nope\",\"name\":\"w\",\"count\":1,\"active\":true,\"category\":null}")
                                        .getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertFalse(messageChain(badConst).isEmpty());

        // Unknown key on a closed object.
        RuntimeException unknown = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload("{\"theme\":\"dark\",\"nope\":1}".getBytes(StandardCharsets.UTF_8)),
                        Settings.class,
                        Settings.class));
        assertTrue(messageChain(unknown).contains("unknown field"), messageChain(unknown));

        // Closed value-set (const/enum) rejections with informative reasons.
        String closedBase =
                "\"kind\":\"showcase\",\"revision\":1,\"enabled\":true,\"status\":\"active\","
                        + "\"tier\":1,\"scale\":1.5,\"name\":\"w\",\"count\":1,\"active\":true,"
                        + "\"category\":\"tools\"";
        RuntimeException badRevision = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(("{" + closedBase + ",\"revision\":2}").getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(messageChain(badRevision).contains("must equal 1"), messageChain(badRevision));

        RuntimeException badStatus = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(("{" + closedBase + ",\"status\":\"archived\"}").getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(
                messageChain(badStatus).contains("must be one of [\"active\", \"inactive\", \"pending\"], got archived"),
                messageChain(badStatus));

        RuntimeException badTier = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(("{" + closedBase + ",\"tier\":9}").getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(messageChain(badTier).contains("must be one of [1, 2, 3], got 9"), messageChain(badTier));

        RuntimeException badScale = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(("{" + closedBase + ",\"scale\":3.5}").getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(messageChain(badScale).contains("must be one of [1.5, 2.5], got 3.5"), messageChain(badScale));

        // Numeric constraints fire at runtime with informative reasons.
        String base = "\"kind\":\"showcase\",\"name\":\"w\",\"count\":1,\"active\":true,\"category\":\"tools\"";
        RuntimeException tooBig = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(("{" + base + ",\"priority\":99}").getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(messageChain(tooBig).contains("must be <= 10, got 99"), messageChain(tooBig));

        RuntimeException notMultiple = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(("{" + base + ",\"step\":7}").getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(messageChain(notMultiple).contains("must be a multiple of 3, got 7"), messageChain(notMultiple));

        RuntimeException numberOffGrid = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(("{" + base + ",\"ratio\":7}").getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(messageChain(numberOffGrid).contains("must be a multiple of 5, got 7"), messageChain(numberOffGrid));

        // String-length constraints fire at runtime, counted in code points.
        RuntimeException tooShort = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(("{" + base + ",\"code\":\"a\"}").getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(messageChain(tooShort).contains("must have length >= 2, got 1"), messageChain(tooShort));

        RuntimeException tooLong = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(("{" + base + ",\"code\":\"abcdef\"}").getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(messageChain(tooLong).contains("must have length <= 5, got 6"), messageChain(tooLong));

        // Astral: 6 emoji = 6 code points (24 bytes); rejected by code-point count.
        RuntimeException astral = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(("{" + base + ",\"code\":\"😀😀😀😀😀😀\"}").getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(messageChain(astral).contains("must have length <= 5, got 6"), messageChain(astral));

        // Array constraints fire at runtime with informative reasons.
        RuntimeException tooFewItems = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(("{" + base + ",\"tags\":[]}").getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(messageChain(tooFewItems).contains("must have at least 1 items, got 0"), messageChain(tooFewItems));

        RuntimeException tooManyItems = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(("{" + base + ",\"tags\":[\"a\",\"b\",\"c\",\"d\",\"e\",\"f\"]}").getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(messageChain(tooManyItems).contains("must have at most 5 items, got 6"), messageChain(tooManyItems));

        RuntimeException dupItems = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(("{" + base + ",\"aliases\":[\"x\",\"x\"]}").getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(messageChain(dupItems).contains("duplicate items: element at index 1 equals index 0"), messageChain(dupItems));

        RuntimeException badOnlyTag = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(("{" + base + ",\"tags\":[1]}").getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(messageChain(badOnlyTag).contains("tags[0]"), messageChain(badOnlyTag));
        assertFalse(messageChain(badOnlyTag).contains("must have at least 1 items"), messageChain(badOnlyTag));

        RuntimeException distinctBadAliases = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(("{" + base + ",\"aliases\":[1,2]}").getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertFalse(messageChain(distinctBadAliases).contains("duplicate items"), messageChain(distinctBadAliases));

        RuntimeException duplicateBadAliases = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(("{" + base + ",\"aliases\":[1,1]}").getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(messageChain(duplicateBadAliases).contains("duplicate items: element at index 1 equals index 0"), messageChain(duplicateBadAliases));

        RuntimeException badRoleWithRawMatch = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(("{" + base + ",\"roles\":[1,\"admin\"]}").getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(messageChain(badRoleWithRawMatch).contains("roles[0]"), messageChain(badRoleWithRawMatch));
        assertFalse(messageChain(badRoleWithRawMatch).contains("too few matching items"), messageChain(badRoleWithRawMatch));

        RuntimeException missingContains = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(("{" + base + ",\"roles\":[\"user\"]}").getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(messageChain(missingContains).contains("too few matching items: at least 1, got 0"), messageChain(missingContains));

        RuntimeException tooManyContains = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(("{" + base + ",\"roles\":[\"admin\",\"admin\",\"admin\"]}").getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(messageChain(tooManyContains).contains("too many matching items: at most 2, got 3"), messageChain(tooManyContains));
    }

    @Test
    void patternConstraintsRoundTripAndReject() throws IOException {
        // sku `^[A-Z]{2,4}$` and phrase `^\S+\s\S+$` round-trip.
        Showcase patterns = roundTrip("showcase-patterns.json", Showcase.class);
        assertEquals("AB", patterns.getSku());
        assertEquals("hello world", patterns.getPhrase());

        String base = "\"kind\":\"showcase\",\"name\":\"w\",\"count\":1,\"active\":true,\"category\":\"tools\"";

        // Lowercase / too-long sku.
        RuntimeException badSkuCase = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(("{" + base + ",\"sku\":\"ab\"}").getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(messageChain(badSkuCase).contains("must match pattern"), messageChain(badSkuCase));
        // The reason quotes the authored pattern (`$`), not the `\z` rewrite
        // compiled for matching — the same text Go/TS/Python print.
        assertTrue(
                messageChain(badSkuCase).contains("must match pattern \"^[A-Z]{2,4}$\", got \"ab\""),
                messageChain(badSkuCase));

        RuntimeException longSku = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(("{" + base + ",\"sku\":\"ABCDE\"}").getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(messageChain(longSku).contains("must match pattern"), messageChain(longSku));

        // phrase with no whitespace separator.
        RuntimeException noSep = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(("{" + base + ",\"phrase\":\"helloworld\"}").getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(messageChain(noSep).contains("must match pattern"), messageChain(noSep));

        // `\s` ASCII-class crux: a NBSP (U+00A0) is NOT ASCII whitespace. The
        // loader normalized `\s`/`\S` to the explicit ASCII class, so Java's
        // default-flag matcher rejects it — consistent with Go/TS/Python.
        RuntimeException nbsp = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(("{" + base + ",\"phrase\":\"hello world\"}").getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(messageChain(nbsp).contains("must match pattern"), messageChain(nbsp));

        // `$` end-anchor crux: a trailing newline is rejected. Java `$` matches
        // before a trailing `\n`; the loader rewrote `$`→`\z` (strict end), so
        // this rejects, consistent with Go/TS/Python.
        RuntimeException trailingNewline = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(("{" + base + ",\"phrase\":\"hello world\\n\"}").getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(messageChain(trailingNewline).contains("must match pattern"), messageChain(trailingNewline));
        // (A valid sku/phrase is exercised by the showcase-patterns.json round-trip above.)
    }

    @Test
    void formatConstraintsRoundTripAndReject() throws IOException {
        // uuid/email/hostname/uri/ipv4 round-trip (string-typed, no materialization).
        Showcase formats = roundTrip("showcase-format.json", Showcase.class);
        assertEquals("de305d54-75b4-431b-adb2-eb6b9e546013", formats.getRequestId());
        assertEquals("user@example.com", formats.getContactEmail());
        assertEquals("api.example.com", formats.getHost());
        assertEquals("https://example.com/path?q=1#frag", formats.getHomepage());
        assertEquals("192.168.0.1", formats.getGateway());

        String base = "\"kind\":\"showcase\",\"name\":\"w\",\"count\":1,\"active\":true,\"category\":\"tools\"";

        // A malformed uuid.
        RuntimeException badUuid = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(("{" + base + ",\"requestId\":\"not-a-uuid\"}").getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(messageChain(badUuid).contains("must be a valid uuid, got not-a-uuid"), messageChain(badUuid));

        // Single-label email domain (user@localhost) is rejected.
        RuntimeException badEmail = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(("{" + base + ",\"contactEmail\":\"user@localhost\"}").getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(messageChain(badEmail).contains("must be a valid email, got user@localhost"), messageChain(badEmail));

        // ipv4 octet out of range.
        RuntimeException badIpv4 = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(("{" + base + ",\"gateway\":\"256.0.0.1\"}").getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(messageChain(badIpv4).contains("must be a valid ipv4, got 256.0.0.1"), messageChain(badIpv4));

        // uri with a double-`::` IPv6 IP-literal host (spliced ipv6 grammar rejects).
        RuntimeException badUri = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(("{" + base + ",\"homepage\":\"http://[1::2::3]\"}").getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(messageChain(badUri).contains("must be a valid uri"), messageChain(badUri));

        // An over-long hostname (> 253 code points) is rejected by the length guard.
        StringBuilder longHost = new StringBuilder();
        for (int i = 0; i < 64; i++) {
            if (i > 0) {
                longHost.append('.');
            }
            longHost.append("abc");
        }
        RuntimeException badHost = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(("{" + base + ",\"host\":\"" + longHost + "\"}").getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(messageChain(badHost).contains("must be a valid hostname"), messageChain(badHost));
    }

    @Test
    void contentEncodingRoundTripAndReject() throws IOException {
        // blob (base64) and urlBlob (base64url) round-trip: a JSON string on the
        // wire, native byte[] in the model, re-encoded byte-identically. The same
        // bytes (">>>") encode to "Pj4+" (padded standard) vs "Pj4-" (unpadded
        // URL-safe).
        Showcase bytes = roundTrip("showcase-bytes.json", Showcase.class);
        assertArrayEquals(">>>".getBytes(StandardCharsets.US_ASCII), bytes.getBlob());
        assertArrayEquals(">>>".getBytes(StandardCharsets.US_ASCII), bytes.getUrlBlob());

        String base = "\"kind\":\"showcase\",\"name\":\"w\",\"count\":1,\"active\":true,\"category\":\"tools\"";

        // A base64 field using the URL-safe alphabet is rejected by the pinned regex.
        RuntimeException urlUnderBase64 = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(("{" + base + ",\"blob\":\"Pj4-\"}").getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(messageChain(urlUnderBase64).contains("must be base64-encoded")
                        && messageChain(urlUnderBase64).contains("Pj4-"),
                messageChain(urlUnderBase64));

        // A base64 field missing padding is rejected.
        RuntimeException unpadded = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(("{" + base + ",\"blob\":\"aGk\"}").getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(messageChain(unpadded).contains("must be base64-encoded"), messageChain(unpadded));

        // A base64url field carrying padding is rejected.
        RuntimeException paddedUnderUrl = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(("{" + base + ",\"urlBlob\":\"aGk=\"}").getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(messageChain(paddedUnderUrl).contains("must be base64url-encoded")
                        && messageChain(paddedUnderUrl).contains("aGk="),
                messageChain(paddedUnderUrl));
    }

    @Test
    void objectConstraintsRoundTripAndReject() throws IOException {
        // Valid map and object round-trip.
        Attributes attributes = roundTrip("attributes.json", Attributes.class);
        assertEquals("a", attributes.getAdditionalProperties().get("host"));
        assertEquals("8080", attributes.getAdditionalProperties().get("port"));

        ContactJava contact = roundTrip("contact.json", ContactJava.class);
        assertEquals("1 Main St", contact.getShippingStreet());
        assertEquals("90210", contact.getShippingZip());

        // minProperties:1 on a map — an empty object is too few (one number over
        // the distinct wire keys).
        RuntimeException tooFew = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload("{}".getBytes(StandardCharsets.UTF_8)),
                        Attributes.class,
                        Attributes.class));
        assertTrue(messageChain(tooFew).contains("must have at least 1 properties, got 0"), messageChain(tooFew));

        // maxProperties:3 on a map.
        RuntimeException tooMany = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload("{\"a\":\"1\",\"b\":\"2\",\"c\":\"3\",\"d\":\"4\"}".getBytes(StandardCharsets.UTF_8)),
                        Attributes.class,
                        Attributes.class));
        assertTrue(messageChain(tooMany).contains("must have at most 3 properties, got 4"), messageChain(tooMany));

        // propertyNames maxLength:8 — an over-long key (code-point length).
        RuntimeException badKey = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload("{\"toolongkey\":\"1\"}".getBytes(StandardCharsets.UTF_8)),
                        Attributes.class,
                        Attributes.class));
        assertTrue(
                messageChain(badKey).contains("invalid property name \"toolongkey\": must have length <= 8, got 10"),
                messageChain(badKey));

        // dependentRequired — a shipping street present without a shipping zip.
        RuntimeException missingDep = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload("{\"shippingStreet\":\"1 Main St\"}".getBytes(StandardCharsets.UTF_8)),
                        ContactJava.class,
                        ContactJava.class));
        assertTrue(
                messageChain(missingDep).contains("property \"shippingZip\" is required when \"shippingStreet\" is present"),
                messageChain(missingDep));

        // minProperties:1 on a declared-property object — an empty object.
        RuntimeException emptyContact = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload("{}".getBytes(StandardCharsets.UTF_8)),
                        ContactJava.class,
                        ContactJava.class));
        assertTrue(messageChain(emptyContact).contains("must have at least 1 properties, got 0"), messageChain(emptyContact));
    }

    @Test
    void allOfMergedWidgetRoundTripsAndEnforcesMergedBounds() throws IOException {
        // Widget is an allOf base-type extension (WidgetBase folded in + an
        // extension branch): a flat standalone object with the union of
        // properties ({id, kind, name, size}) and required ([id, name]).
        Widget widget = roundTrip("widget.json", Widget.class);
        assertEquals("w-1", widget.getId());
        assertEquals("gadget", widget.getKind());
        assertEquals("Widget One", widget.getName());
        assertNotNull(widget.getSize());
        assertEquals(15L, widget.getSize());

        // `size` carries a bound tightened from two allOf branches to [10, 20].
        RuntimeException tooSmall = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(
                                "{\"id\":\"w-1\",\"name\":\"Widget One\",\"size\":5}"
                                        .getBytes(StandardCharsets.UTF_8)),
                        Widget.class,
                        Widget.class));
        assertTrue(messageChain(tooSmall).contains("must be >= 10, got 5"), messageChain(tooSmall));

        RuntimeException tooBig = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(
                                "{\"id\":\"w-1\",\"name\":\"Widget One\",\"size\":25}"
                                        .getBytes(StandardCharsets.UTF_8)),
                        Widget.class,
                        Widget.class));
        assertTrue(messageChain(tooBig).contains("must be <= 20, got 25"), messageChain(tooBig));

        // A missing required member contributed by the extension branch.
        RuntimeException missingName = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload("{\"id\":\"w-1\"}".getBytes(StandardCharsets.UTF_8)),
                        Widget.class,
                        Widget.class));
        assertFalse(messageChain(missingName).isEmpty());
    }

    @Test
    void oneOfSumTypesRoundTripAndReject() throws IOException {
        // Disjoint-kind union (string | integer): each branch round-trips and is
        // selected by its wire token.
        Showcase asString = roundTrip("showcase-union-string.json", Showcase.class);
        assertTrue(asString.getIdOrName() instanceof Showcase.IdOrNameString);
        assertEquals("abc", ((Showcase.IdOrNameString) asString.getIdOrName()).getValue());
        Showcase asInt = roundTrip("showcase-union-int.json", Showcase.class);
        assertTrue(asInt.getIdOrName() instanceof Showcase.IdOrNameInteger);
        assertEquals(7L, ((Showcase.IdOrNameInteger) asInt.getIdOrName()).getValue());

        // Discriminated (tagged) union (Circle | Square) selected by `kind`.
        Showcase circle = roundTrip("showcase-shape-circle.json", Showcase.class);
        assertTrue(circle.getShape() instanceof Circle);
        assertEquals(2.5, ((Circle) circle.getShape()).getRadius());
        Showcase square = roundTrip("showcase-shape-square.json", Showcase.class);
        assertTrue(square.getShape() instanceof Square);
        assertEquals(4.0, ((Square) square.getShape()).getSide());

        String base =
                "{\"kind\":\"showcase\",\"revision\":1,\"enabled\":true,\"status\":\"active\","
                        + "\"tier\":1,\"scale\":1.5,\"name\":\"w\",\"count\":1,\"active\":true,"
                        + "\"category\":\"tools\"";

        // An unmatchable wire token (boolean) names the admissible kinds.
        RuntimeException badToken = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload((base + ",\"idOrName\":true}").getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(messageChain(badToken).contains("string, integer"), messageChain(badToken));

        // An unknown discriminator value is rejected (closed value set, P13.1).
        RuntimeException badTag = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(
                                (base + ",\"shape\":{\"kind\":\"triangle\"}}")
                                        .getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(messageChain(badTag).contains("triangle"), messageChain(badTag));
    }

    /**
     * Once the wire token selects a branch, the value is held to everything that
     * branch declares — {@code idOrName}'s length/numeric bounds, {@code mode}'s
     * closed string value set, {@code measurements}' array bounds and pattern —
     * in both directions, with the union's own path on the violation.
     */
    @Test
    void oneOfBranchConstraintsAreEnforced() throws IOException {
        String base =
                "{\"kind\":\"showcase\",\"revision\":1,\"enabled\":true,\"status\":\"active\","
                        + "\"tier\":1,\"scale\":1.5,\"name\":\"w\",\"count\":1,\"active\":true,"
                        + "\"category\":\"tools\"";

        // The string branch's own `minLength`.
        RuntimeException shortString = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload((base + ",\"idOrName\":\"ab\"}").getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(
                messageChain(shortString).contains("idOrName: must have length >= 3, got 2"),
                messageChain(shortString));

        // The integer branch's own `minimum` — the string branch's bound does not
        // apply to it.
        RuntimeException smallInt = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload((base + ",\"idOrName\":0}").getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(
                messageChain(smallInt).contains("idOrName: must be >= 1, got 0"),
                messageChain(smallInt));

        // A closed value set on a branch: an unknown string names the admissible
        // values, while the integer branch accepts any non-negative value.
        RuntimeException badMode = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload((base + ",\"mode\":\"turbo\"}").getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(
                messageChain(badMode).contains("mode: must be one of [\"auto\", \"manual\"], got turbo"),
                messageChain(badMode));
        Showcase full = roundTrip("showcase-full.json", Showcase.class);
        assertTrue(full.getMode() instanceof Showcase.ModeString);
        assertEquals("auto", ((Showcase.ModeString) full.getMode()).getValue());

        // The array branch's `minItems`/`uniqueItems` and the string branch's
        // `pattern`, on the same union.
        RuntimeException empty = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload((base + ",\"measurements\":[]}").getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(
                messageChain(empty).contains("measurements: must have at least 1 items, got 0"),
                messageChain(empty));
        RuntimeException duplicate = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(
                                (base + ",\"measurements\":[1.5,1.5]}")
                                        .getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(
                messageChain(duplicate)
                        .contains("duplicate items: element at index 1 equals index 0"),
                messageChain(duplicate));
        RuntimeException offPattern = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(
                                (base + ",\"measurements\":\"AUTO\"}")
                                        .getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(
                messageChain(offPattern).contains("measurements: must match pattern"),
                messageChain(offPattern));

        // An element union's branch constraints hold per element, under the
        // element's own index (P11).
        RuntimeException element = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(
                                (base + ",\"segments\":[\"ab\",\"c\"]}")
                                        .getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(
                messageChain(element).contains("segments[1]: must have length >= 2, got 1"),
                messageChain(element));

        // Serialize re-runs the selected branch's constraints (P12), for a
        // property-level union and for a collection's elements.
        RuntimeException serialize = assertThrows(RuntimeException.class, () ->
                CONVERTER.toPayload(
                        showcaseWith(null, null, null, null, null,
                                new Showcase.IdOrNameString("ab"))));
        assertTrue(
                messageChain(serialize).contains("idOrName: must have length >= 3, got 2"),
                messageChain(serialize));
    }

    /**
     * The free-form object in both positions: the inline object branch of the
     * {@code payload} union (a nested wrapper class holding the members verbatim)
     * and the named {@code Extras} model. Members keep their wire form, so a large
     * integer survives untruncated, and the member-count bound is enforced.
     */
    @Test
    void freeFormObjectRoundTripsAndRejects() throws IOException {
        Showcase asObject = roundTrip("showcase-freeform.json", Showcase.class);
        assertTrue(asObject.getPayload() instanceof Showcase.PayloadObject);
        Map<String, JsonNode> members =
                ((Showcase.PayloadObject) asObject.getPayload()).getValue();
        assertEquals(9007199254740992L, members.get("big").longValue());
        assertNotNull(asObject.getExtras());
        assertEquals(
                "free-form",
                asObject.getExtras().getAdditionalProperties().get("note").textValue());

        // The same union's string branch, selected by its wire token.
        Showcase asString = roundTrip("showcase-freeform-string.json", Showcase.class);
        assertTrue(asString.getPayload() instanceof Showcase.PayloadString);
        assertEquals("text", ((Showcase.PayloadString) asString.getPayload()).getValue());

        // The named free-form model round-trips standalone, nested members included.
        Extras extras = roundTrip("extras.json", Extras.class);
        assertEquals(1, extras.getAdditionalProperties().get("nested").get("a").intValue());
        assertEquals(
                9007199254740992L, extras.getAdditionalProperties().get("count").longValue());

        // maxProperties over the member set is enforced.
        RuntimeException tooMany = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(
                                "{\"a\":1,\"b\":2,\"c\":3,\"d\":4,\"e\":5}"
                                        .getBytes(StandardCharsets.UTF_8)),
                        Extras.class,
                        Extras.class));
        assertTrue(messageChain(tooMany).contains("at most 4 properties"), messageChain(tooMany));

        String base =
                "{\"kind\":\"showcase\",\"revision\":1,\"enabled\":true,\"status\":\"active\","
                        + "\"tier\":1,\"scale\":1.5,\"name\":\"w\",\"count\":1,\"active\":true,"
                        + "\"category\":\"tools\"";

        // An unmatchable wire token (boolean) names the admissible kinds.
        RuntimeException badToken = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload((base + ",\"payload\":true}").getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(messageChain(badToken).contains("object, string"), messageChain(badToken));
    }

    /**
     * The {@code note} tagged union, whose object branches are written inline in
     * the schema and named by their {@code x-java-name} overrides: each is a full
     * POJO that implements the union interface, keeping its own constraints and
     * its own verbatim member map.
     */
    @Test
    void inlineObjectUnionRoundTripsAndRejects() throws IOException {
        Showcase text = roundTrip("showcase-note-text.json", Showcase.class);
        assertTrue(text.getNote() instanceof TextNote);
        TextNote note = (TextNote) text.getNote();
        assertEquals("remember the milk", note.getBody());
        // The branch stays open: an unknown member is preserved (P13).
        assertTrue(note.getAdditionalProperties().get("pinned").booleanValue());

        Showcase link = roundTrip("showcase-note-link.json", Showcase.class);
        assertTrue(link.getNote() instanceof LinkNote);
        assertEquals(
                "https://example.test/notes/1", ((LinkNote) link.getNote()).getHref());

        String base =
                "{\"kind\":\"showcase\",\"revision\":1,\"enabled\":true,\"status\":\"active\","
                        + "\"tier\":1,\"scale\":1.5,\"name\":\"w\",\"count\":1,\"active\":true,"
                        + "\"category\":\"tools\"";

        // The selected branch's own constraints are enforced.
        RuntimeException tooShort = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(
                                (base + ",\"note\":{\"kind\":\"text\",\"body\":\"\"}}")
                                        .getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(messageChain(tooShort).contains("length >= 1"), messageChain(tooShort));

        // An unknown tag value matches no branch.
        RuntimeException badTag = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(
                                (base + ",\"note\":{\"kind\":\"audio\"}}")
                                        .getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(messageChain(badTag).contains("audio"), messageChain(badTag));
    }

    /**
     * The {@code detail} union is written inline on the property, so its interface
     * is nested in {@code Showcase}. Its lone structured object branch derives
     * {@code ShowcaseDetailObject} from the union it belongs to and is an ordinary
     * POJO implementing that nested interface, so the union's {@code fromNode}
     * delegates to the branch's own deserializer; the string branch is carried by
     * the generated {@code DetailString} wrapper.
     */
    @Test
    void propertyInlineObjectUnionRoundTripsAndRejects() throws IOException {
        Showcase object = roundTrip("showcase-detail-object.json", Showcase.class);
        assertTrue(object.getDetail() instanceof ShowcaseDetailObject);
        ShowcaseDetailObject detail = (ShowcaseDetailObject) object.getDetail();
        assertEquals("E_LIMIT", detail.getCode());
        assertEquals("retry later", detail.getHint());
        // The branch stays open: an unknown member is preserved (P13).
        assertEquals(250, detail.getAdditionalProperties().get("retryAfterMs").intValue());

        Showcase text = roundTrip("showcase-detail-string.json", Showcase.class);
        assertTrue(text.getDetail() instanceof Showcase.DetailString);
        assertEquals("E_LIMIT", ((Showcase.DetailString) text.getDetail()).getValue());

        String base =
                "{\"kind\":\"showcase\",\"revision\":1,\"enabled\":true,\"status\":\"active\","
                        + "\"tier\":1,\"scale\":1.5,\"name\":\"w\",\"count\":1,\"active\":true,"
                        + "\"category\":\"tools\"";

        // The object branch's own constraints are enforced.
        RuntimeException tooShort = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(
                                (base + ",\"detail\":{\"code\":\"\"}}")
                                        .getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(messageChain(tooShort).contains("length >= 1"), messageChain(tooShort));

        // A token admitted by no branch names the admissible ones.
        RuntimeException badToken = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(
                                (base + ",\"detail\":7}").getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(
                messageChain(badToken).contains("ShowcaseDetailObject, string"),
                messageChain(badToken));
    }

    /**
     * The {@code shapeOrName} union composes both selector layers: {@code fromNode}
     * switches on the JSON node kind to pick object-vs-string and, for an object,
     * peeks the shared required {@code kind} const to pick Circle-vs-Square. Both
     * POJOs also implement the {@code Shape} interface, so a branch type may take
     * part in more than one union.
     */
    @Test
    void taggedUnionWithScalarBranchRoundTripsAndRejects() throws IOException {
        Showcase square = roundTrip("showcase-shape-or-name-square.json", Showcase.class);
        assertTrue(square.getShapeOrName() instanceof Square);
        assertEquals(4.0, ((Square) square.getShapeOrName()).getSide());

        Showcase named = roundTrip("showcase-shape-or-name-string.json", Showcase.class);
        assertTrue(named.getShapeOrName() instanceof Showcase.ShapeOrNameString);
        assertEquals(
                "unit-square",
                ((Showcase.ShapeOrNameString) named.getShapeOrName()).getValue());

        String base =
                "{\"kind\":\"showcase\",\"revision\":1,\"enabled\":true,\"status\":\"active\","
                        + "\"tier\":1,\"scale\":1.5,\"name\":\"w\",\"count\":1,\"active\":true,"
                        + "\"category\":\"tools\"";

        // The object node routes through the discriminator, so an unknown tag is
        // rejected rather than falling back to the string branch.
        RuntimeException badTag = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(
                                (base + ",\"shapeOrName\":{\"kind\":\"triangle\"}}")
                                        .getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(messageChain(badTag).contains("triangle"), messageChain(badTag));

        // A node kind admitted by no branch names all admissible ones.
        RuntimeException badToken = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(
                                (base + ",\"shapeOrName\":7}").getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(
                messageChain(badToken).contains("Circle, Square, string"),
                messageChain(badToken));
    }

    /**
     * The {@code measurements} union has an array branch, which has no definition
     * to take a name from: Java wraps it in the generated {@code MeasurementsArray}
     * class (a {@code @JsonValue} holder, so it writes back as a bare array)
     * alongside the {@code MeasurementsString} wrapper.
     */
    @Test
    void arrayBranchUnionRoundTripsAndRejects() throws IOException {
        Showcase values = roundTrip("showcase-measurements-array.json", Showcase.class);
        assertTrue(values.getMeasurements() instanceof Showcase.MeasurementsArray);
        assertEquals(
                java.util.Arrays.asList(1.5, 2.5, 3.75),
                ((Showcase.MeasurementsArray) values.getMeasurements()).getValue());

        Showcase preset = roundTrip("showcase-measurements-string.json", Showcase.class);
        assertTrue(preset.getMeasurements() instanceof Showcase.MeasurementsString);
        assertEquals(
                "auto", ((Showcase.MeasurementsString) preset.getMeasurements()).getValue());

        String base =
                "{\"kind\":\"showcase\",\"revision\":1,\"enabled\":true,\"status\":\"active\","
                        + "\"tier\":1,\"scale\":1.5,\"name\":\"w\",\"count\":1,\"active\":true,"
                        + "\"category\":\"tools\"";

        // A node kind admitted by neither branch names both admissible ones.
        RuntimeException badToken = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(
                                (base + ",\"measurements\":true}").getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(messageChain(badToken).contains("array, string"), messageChain(badToken));

        // The array branch's element type is enforced once the branch is selected.
        RuntimeException badElement = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(
                                (base + ",\"measurements\":[\"x\"]}")
                                        .getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(
                messageChain(badElement).contains("expected number"), messageChain(badElement));
    }

    /**
     * Unions in positions with no property of their own: an array element at a
     * named union ({@code shapes}), an array element at an inline union the loader
     * names {@code ShowcaseSegmentsItem}, and a map member at an inline union
     * named {@code ChoicesValue}. Jackson cannot instantiate the sealed
     * interface, so each element/member is routed through the interface's own
     * {@code fromNode} dispatcher, which reports under the element index or the
     * member key.
     */
    @Test
    void elementPositionUnionsRoundTripAndReject() throws IOException {
        Showcase value = roundTrip("showcase-element-unions.json", Showcase.class);

        assertNotNull(value.getShapes());
        assertEquals(2, value.getShapes().size());
        assertTrue(value.getShapes().get(0) instanceof Circle);
        assertEquals(2.5, ((Circle) value.getShapes().get(0)).getRadius());
        assertTrue(value.getShapes().get(1) instanceof Square);
        assertEquals(4.0, ((Square) value.getShapes().get(1)).getSide());

        assertNotNull(value.getSegments());
        assertEquals(
                "alpha",
                ((ShowcaseSegmentsItem.ShowcaseSegmentsItemString) value.getSegments().get(0))
                        .getValue());
        assertEquals(
                7L,
                ((ShowcaseSegmentsItem.ShowcaseSegmentsItemInteger) value.getSegments().get(1))
                        .getValue());

        // Element nullability is the element's own concern: the list stays a
        // list, its members are @Nullable, and an explicit null is a member.
        assertEquals(java.util.Arrays.asList("first", null, "third"), value.getSlots());

        assertNotNull(value.getChoices());
        assertTrue(value.getChoices().getAdditionalProperties().get("primary") instanceof Circle);

        String base =
                "{\"kind\":\"showcase\",\"revision\":1,\"enabled\":true,\"status\":\"active\","
                        + "\"tier\":1,\"scale\":1.5,\"name\":\"w\",\"count\":1,\"active\":true,"
                        + "\"category\":\"tools\"";

        // A bad element is reported at its own index, not at the array.
        RuntimeException badElement = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(
                                (base + ",\"shapes\":[{\"kind\":\"circle\",\"radius\":1},true]}")
                                        .getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(messageChain(badElement).contains("shapes[1]"), messageChain(badElement));

        // An unknown discriminator inside an element is still routed by tag.
        RuntimeException badTag = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(
                                (base + ",\"shapes\":[{\"kind\":\"triangle\"}]}")
                                        .getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(messageChain(badTag).contains("triangle"), messageChain(badTag));

        // The inline element union is a closed sum type like any other.
        RuntimeException badSegment = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(
                                (base + ",\"segments\":[\"ok\",1.5]}")
                                        .getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(messageChain(badSegment).contains("segments[1]"), messageChain(badSegment));

        // A map member's violation carries its key.
        RuntimeException badMember = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(
                                (base + ",\"choices\":{\"primary\":\"circle\"}}")
                                        .getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(messageChain(badMember).contains("primary"), messageChain(badMember));
    }

    /**
     * An object written inline in a value position is named after that position and
     * emitted as an ordinary model: a property ({@code location}, with its own
     * nested {@code geo}), a nullable property ({@code audit}), an array element
     * ({@code rows}), a map and its member ({@code ledger}), and a free-form bag
     * ({@code metadata}). The same fixture covers a typed map's member constraints
     * ({@code quotas}, {@code tokens}, {@code nicknames}) and a nested array
     * ({@code grid}).
     */
    @Test
    void inlineObjectShapesRoundTripAndReject() throws IOException {
        Showcase value = roundTrip("showcase-inline-shapes.json", Showcase.class);

        assertEquals(
                java.util.Arrays.asList(
                        java.util.Arrays.asList(1L, 2L), java.util.Arrays.asList(3L)),
                value.getGrid());
        assertNotNull(value.getLocation());
        assertEquals("Springfield", value.getLocation().getCity());
        assertNotNull(value.getLocation().getGeo());
        assertEquals(39.8, value.getLocation().getGeo().getLat());
        assertNotNull(value.getAudit());
        assertEquals("alice", value.getAudit().getBy());
        assertNotNull(value.getRows());
        assertEquals("a1", value.getRows().get(0).getCell());
        // The member override renamed the accessor (`getLedgerJava`); the hoisted
        // types keep their position-derived names.
        assertNotNull(value.getLedgerJava());
        ShowcaseLedgerValue opening =
                value.getLedgerJava().getAdditionalProperties().get("opening");
        assertNotNull(opening);
        assertEquals(100L, opening.getAmount());
        assertNotNull(value.getMetadata());
        assertEquals(2, value.getMetadata().getAdditionalProperties().size());
        assertNotNull(value.getQuotas());
        assertEquals(20L, value.getQuotas().getAdditionalProperties().get("cpu"));
        // A null member of a nullable map is a member, not a violation.
        assertNotNull(value.getNicknames());
        assertEquals("al", value.getNicknames().getAdditionalProperties().get("short"));
        assertTrue(value.getNicknames().getAdditionalProperties().containsKey("none"));
        assertNull(value.getNicknames().getAdditionalProperties().get("none"));

        String base =
                "{\"kind\":\"showcase\",\"revision\":1,\"enabled\":true,\"status\":\"active\","
                        + "\"tier\":1,\"scale\":1.5,\"name\":\"w\",\"count\":1,\"active\":true,"
                        + "\"category\":\"tools\"";

        // A hoisted shape validates like any other model, at the nested path.
        RuntimeException nested = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(
                                (base + ",\"location\":{\"city\":\"\"}}")
                                        .getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(messageChain(nested).contains("location.city"), messageChain(nested));

        RuntimeException badRow = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(
                                (base + ",\"rows\":[{\"cell\":\"ok\"},{}]}")
                                        .getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(messageChain(badRow).contains("rows[1]"), messageChain(badRow));

        // A nested array reports the failing element at its own two-dimensional
        // index — each level decodes elementwise.
        RuntimeException badCell = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(
                                (base + ",\"grid\":[[1],[2,1.5]]}")
                                        .getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(messageChain(badCell).contains("grid[1][1]"), messageChain(badCell));

        // A typed map's member constraints are enforced, keyed by the member.
        RuntimeException badQuota = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(
                                (base + ",\"quotas\":{\"cpu\":7}}")
                                        .getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(messageChain(badQuota).contains("cpu"), messageChain(badQuota));
        assertTrue(
                messageChain(badQuota).contains("must be a multiple of 5"),
                messageChain(badQuota));

        RuntimeException badToken = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(
                                (base + ",\"tokens\":{\"primary\":\"AB\"}}")
                                        .getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(messageChain(badToken).contains("primary"), messageChain(badToken));

        RuntimeException badNickname = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(
                                (base + ",\"nicknames\":{\"tiny\":\"a\"}}")
                                        .getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(messageChain(badNickname).contains("tiny"), messageChain(badNickname));

        // The free-form bag's member-count bound rides with the hoisted type.
        RuntimeException tooManyMembers = assertThrows(RuntimeException.class, () ->
                CONVERTER.fromPayload(
                        jsonPayload(
                                (base + ",\"metadata\":{\"a\":1,\"b\":2,\"c\":3,\"d\":4}}")
                                        .getBytes(StandardCharsets.UTF_8)),
                        Showcase.class,
                        Showcase.class));
        assertTrue(
                messageChain(tooManyMembers).contains("at most 3"), messageChain(tooManyMembers));

        // Serialize re-runs every member's own constraints before emitting (P12).
        Map<String, Long> badQuotas = new java.util.LinkedHashMap<>();
        badQuotas.put("cpu", 7L);
        RuntimeException quotaSerialize = assertThrows(RuntimeException.class, () ->
                CONVERTER.toPayload(new Quotas(badQuotas)));
        assertTrue(messageChain(quotaSerialize).contains("cpu"), messageChain(quotaSerialize));

        Map<String, String> badTokens = new java.util.LinkedHashMap<>();
        badTokens.put("primary", "AB");
        RuntimeException tokenSerialize = assertThrows(RuntimeException.class, () ->
                CONVERTER.toPayload(new Tokens(badTokens)));
        assertTrue(messageChain(tokenSerialize).contains("primary"), messageChain(tokenSerialize));
    }

    @Test
    void recursiveCollectionsRoundTripAndRejectNonFiniteValues() throws IOException {
        Showcase value = decode("showcase-recursive-collections.json", Showcase.class);
        // Java currently writes integral `number` values with a `.0` suffix;
        // that separate byte-identity issue is outside this P0 validation fix.
        CONVERTER.toPayload(value).orElseThrow(AssertionError::new);
        assertEquals(java.util.Arrays.asList(1.0, 2.5), value.getNumberGrid().get(0));
        assertEquals("1 Main St", value.getAddresses().get(0).getStreet());
        assertEquals(
                "2 Side St",
                value.getAddressBook().getAdditionalProperties().get("home").getStreet());
        assertEquals(1, value.getDates().get(0).getYear());
        assertEquals(1, value.getDateIndex().getAdditionalProperties().get("first").getYear());
        assertArrayEquals("hi".getBytes(StandardCharsets.UTF_8), value.getBlobs().get(0));
        assertArrayEquals(
                "hi".getBytes(StandardCharsets.UTF_8),
                value.getBlobIndex().getAdditionalProperties().get("hi"));

        Map<String, Double> nonFiniteMap = new java.util.LinkedHashMap<>();
        nonFiniteMap.put("cpu", Double.POSITIVE_INFINITY);
        Object[][] cases = new Object[][] {
            {"score", showcaseWith(null, null, null, null, null, null,
                    Double.NaN, null, null, null, null)},
            {"measurements[0]", showcaseWith(null, null, null, null, null, null,
                    null, new Showcase.MeasurementsArray(java.util.Arrays.asList(Double.NEGATIVE_INFINITY)),
                    null, null, null)},
            {"numberGrid[0][1]", showcaseWith(null, null, null, null, null, null,
                    null, null, java.util.Arrays.asList(java.util.Arrays.asList(1.0, Double.NaN)),
                    null, null)},
            {"cpu", showcaseWith(null, null, null, null, null, null,
                    null, null, null, new Metrics(nonFiniteMap), null)},
            {"metricOrLabel", showcaseWith(null, null, null, null, null, null,
                    null, null, null, null,
                    new Showcase.MetricOrLabelNumber(Double.POSITIVE_INFINITY))},
        };
        for (Object[] testCase : cases) {
            RuntimeException error = assertThrows(
                    RuntimeException.class,
                    () -> CONVERTER.toPayload((Showcase) testCase[1]));
            assertTrue(messageChain(error).contains((String) testCase[0]), messageChain(error));
            assertTrue(messageChain(error).contains("finite number"), messageChain(error));
        }

        RuntimeException nullElement = assertThrows(RuntimeException.class, () ->
                CONVERTER.toPayload(showcaseWith(
                        null, null, null, null, null, null, null, null,
                        java.util.Arrays.asList(java.util.Arrays.asList((Double) null)),
                        null, null)));
        assertTrue(messageChain(nullElement).contains("numberGrid[0][0]"), messageChain(nullElement));
        assertTrue(messageChain(nullElement).contains("explicit null not allowed"), messageChain(nullElement));

        Map<String, Long> nullQuota = new java.util.LinkedHashMap<>();
        nullQuota.put("cpu", null);
        RuntimeException nullMapMember = assertThrows(
                RuntimeException.class, () -> CONVERTER.toPayload(new Quotas(nullQuota)));
        assertTrue(messageChain(nullMapMember).contains("cpu"), messageChain(nullMapMember));
        assertTrue(messageChain(nullMapMember).contains("explicit null not allowed"), messageChain(nullMapMember));

        RuntimeException nullMap = assertThrows(
                RuntimeException.class, () -> CONVERTER.toPayload(new BlobIndex(null)));
        assertTrue(messageChain(nullMap).contains("expected object"), messageChain(nullMap));

        String base =
                "{\"kind\":\"showcase\",\"revision\":1,\"enabled\":true,\"status\":\"active\","
                        + "\"tier\":1,\"scale\":1.5,\"name\":\"w\",\"count\":1,\"active\":true,"
                        + "\"category\":\"tools\"";
        String[][] overflowing = new String[][] {
            {"score", ",\"score\":1e400}"},
            {"metricOrLabel", ",\"metricOrLabel\":1e400}"},
            {"numberGrid[0][0]", ",\"numberGrid\":[[1e400]]}"},
            {"cpu", ",\"metrics\":{\"cpu\":1e400}}"},
        };
        for (String[] testCase : overflowing) {
            RuntimeException error = assertThrows(RuntimeException.class, () ->
                    CONVERTER.fromPayload(
                            jsonPayload((base + testCase[1]).getBytes(StandardCharsets.UTF_8)),
                            Showcase.class,
                            Showcase.class));
            assertTrue(messageChain(error).contains(testCase[0]), messageChain(error));
            assertTrue(messageChain(error).contains("finite number"), messageChain(error));
        }
    }

    @Test
    void numberSpellingsRoundTripByMathematicalValue() throws IOException {
        Showcase value = decode("showcase-number-values.json", Showcase.class);
        CONVERTER.toPayload(value).orElseThrow(AssertionError::new);
        java.util.List<Double> numbers = value.getNumberGrid().get(0);
        assertTrue(numbers.get(0) == 0.0d);
        assertEquals(5.0d, numbers.get(1));
        assertEquals(1000.0d, numbers.get(2));
        assertEquals(Double.MAX_VALUE, numbers.get(3));
        assertEquals(Double.MIN_VALUE, numbers.get(4));
    }

    /**
     * Builds a Showcase with all required members valid, varying only the members
     * under serialize-side test; every other optional member is left unset.
     */
    private static Showcase showcaseWith(
            String code,
            String sku,
            String requestId,
            Long priority,
            java.util.List<String> aliases) {
        return showcaseWith(code, sku, requestId, priority, aliases, null);
    }

    /**
     * As {@link #showcaseWith}, additionally varying the {@code idOrName} union
     * member so a branch's own constraints can be exercised on the serialize side.
     */
    private static Showcase showcaseWith(
            String code,
            String sku,
            String requestId,
            Long priority,
            java.util.List<String> aliases,
            Showcase.IdOrName idOrName) {
        return showcaseWith(
                code, sku, requestId, priority, aliases, idOrName,
                null, null, null, null, null);
    }

    private static Showcase showcaseWith(
            String code,
            String sku,
            String requestId,
            Long priority,
            java.util.List<String> aliases,
            Showcase.IdOrName idOrName,
            Double score,
            Showcase.Measurements measurements,
            java.util.List<java.util.List<Double>> numberGrid,
            Metrics metrics,
            Showcase.MetricOrLabel metricOrLabel) {
        // Closed value-set members can only hold a known value-class constant —
        // a wrong value cannot be constructed (private constructor), so these
        // fields are always the valid constants here.
        return new Showcase(
                Showcase.Kind.SHOWCASE, Showcase.Revision.REVISION_JAVA, Showcase.Enabled.TRUE,
                Showcase.Status.ACTIVE_JAVA, Showcase.Tier.V_1, Showcase.Scale.V_1_5,
                "w", 1L, true,
                null, code, sku, null, requestId, null, null, null, null,
                null, null, null, null, null, null, null, null, "tools",
                priority, null, null, score, null, null, aliases, null, idOrName,
                null, null, null, null, measurements, null, null, null, null,
                numberGrid, null, null, null, null, null, null, null, metrics,
                metricOrLabel, null, null, null, null, null, null, null, null,
                null, null, null, null, null, null, null, null, null, null,
                // nullableCount, nullableRatio, nullableFlag, nullableTags, nullableMode,
                // integralMeasurements, byFive, wildcard, quoted
                null, null, null, null, null, null, null, null, null);
    }

    /**
     * Serialize side (P12): constructing an in-memory model with an out-of-spec
     * value and serializing it (toPayload → the generated Serializer) is rejected
     * before any wire bytes are produced, with the same informative reason as the
     * deserializer. Mirrors the Go serialize-reject assertions.
     */
    @Test
    void invalidInMemoryValuesRejectedOnSerialize() {
        // A valid baseline serializes cleanly (no false rejection).
        Showcase valid = showcaseWith(null, null, null, null, null);
        CONVERTER.toPayload(valid).orElseThrow(AssertionError::new);

        // Numeric bound: an in-memory value past `maximum` fails to serialize.
        RuntimeException numeric = assertThrows(RuntimeException.class, () ->
                CONVERTER.toPayload(showcaseWith(null, null, null, 42L, null)));
        assertTrue(messageChain(numeric).contains("must be <= 10, got 42"), messageChain(numeric));

        // String length: an in-memory over-long string fails to serialize.
        RuntimeException length = assertThrows(RuntimeException.class, () ->
                CONVERTER.toPayload(showcaseWith("abcdef", null, null, null, null)));
        assertTrue(messageChain(length).contains("must have length <= 5, got 6"), messageChain(length));

        // Pattern: an in-memory off-pattern value fails to serialize.
        RuntimeException pattern = assertThrows(RuntimeException.class, () ->
                CONVERTER.toPayload(showcaseWith(null, "xyz", null, null, null)));
        assertTrue(messageChain(pattern).contains("must match pattern"), messageChain(pattern));

        // Format: an in-memory malformed uuid fails to serialize.
        RuntimeException format = assertThrows(RuntimeException.class, () ->
                CONVERTER.toPayload(showcaseWith(null, null, "nope", null, null)));
        assertTrue(messageChain(format).contains("must be a valid uuid, got nope"), messageChain(format));

        // Array: an in-memory duplicate (uniqueItems) fails to serialize.
        RuntimeException array = assertThrows(RuntimeException.class, () ->
                CONVERTER.toPayload(showcaseWith(null, null, null, null,
                        java.util.Arrays.asList("dup", "dup"))));
        assertTrue(messageChain(array).contains("duplicate items: element at index 1 equals index 0"), messageChain(array));

        // Closed item membership is enforced before array-level predicates in
        // both directions, yielding one deterministic aggregate.
        Exception itemParse = assertThrows(Exception.class, () ->
                MAPPER.readValue("{\"values\":[\"bad\",\"bad\"]}", ItemRules.class));
        assertItemRuleOrder(messageChain(itemParse));
        RuntimeException itemSerialize = assertThrows(RuntimeException.class, () ->
                CONVERTER.toPayload(new ItemRules(java.util.Arrays.asList("bad", "bad"))));
        assertItemRuleOrder(messageChain(itemSerialize));

        RuntimeException requiredArray = assertThrows(RuntimeException.class, () ->
                CONVERTER.toPayload(new ItemRules(null)));
        assertTrue(messageChain(requiredArray).contains("values"), messageChain(requiredArray));
        RuntimeException requiredString = assertThrows(RuntimeException.class, () ->
                CONVERTER.toPayload(new Address(null, null, null, new java.util.LinkedHashMap<>())));
        assertTrue(messageChain(requiredString).contains("street"), messageChain(requiredString));

        // Closed value-set (const/enum) members carry no serialize-side check:
        // their value class can only hold a known constant, so an out-of-set
        // value cannot be constructed in memory (a compile-time guarantee). The
        // membership check is therefore a deserialize-direction guard only —
        // exercised in invalidValuesAreRejected (badRevision/badStatus/etc.).

        // allOf-merged bound: an in-memory `size` past the tightened maximum fails.
        RuntimeException widget = assertThrows(RuntimeException.class, () ->
                CONVERTER.toPayload(new Widget(
                        "w-1", null, "Widget One", 25L, new java.util.LinkedHashMap<>())));
        assertTrue(messageChain(widget).contains("must be <= 20, got 25"), messageChain(widget));

        // Object dependentRequired: a shipping street with no zip fails to serialize.
        RuntimeException dep = assertThrows(RuntimeException.class, () ->
                CONVERTER.toPayload(new ContactJava(
                        null, "1 Main St", null, new java.util.LinkedHashMap<>())));
        assertTrue(
                messageChain(dep).contains("property \"shippingZip\" is required when \"shippingStreet\" is present"),
                messageChain(dep));

        // Object member-count: an empty map is below minProperties:1 on serialize.
        RuntimeException emptyMap = assertThrows(RuntimeException.class, () ->
                CONVERTER.toPayload(new Attributes(new java.util.LinkedHashMap<>())));
        assertTrue(messageChain(emptyMap).contains("must have at least 1 properties, got 0"), messageChain(emptyMap));

        // propertyNames key-shape: an over-long key fails to serialize.
        java.util.Map<String, String> longKey = new java.util.LinkedHashMap<>();
        longKey.put("toolongkey", "1");
        RuntimeException badKey = assertThrows(RuntimeException.class, () ->
                CONVERTER.toPayload(new Attributes(longKey)));
        assertTrue(
                messageChain(badKey).contains("invalid property name \"toolongkey\": must have length <= 8, got 10"),
                messageChain(badKey));
    }

    private static String messageChain(Throwable error) {
        StringBuilder builder = new StringBuilder();
        for (Throwable current = error; current != null; current = current.getCause()) {
            if (current instanceof ApplicationFailure) {
                ApplicationFailure failure = (ApplicationFailure) current;
                if ("PayloadValidationError".equals(failure.getType())
                        && failure.getDetails().getSize() > 0) {
                    // Locally-created failures retain the list as detail 0; this
                    // cast is cheap and performs no serialization.
                    builder.append(failure.getDetails().get(0, java.util.List.class)).append('\n');
                }
            }
            if (current.getMessage() != null) {
                builder.append(current.getMessage()).append('\n');
            }
        }
        return builder.toString();
    }

    private static void assertItemRuleOrder(String text) {
        String[] parts = {"values[0]", "values[1]", "must have at least 3 items", "duplicate items"};
        int prior = -1;
        for (String part : parts) {
            int index = text.indexOf(part);
            assertTrue(index > prior, text);
            prior = index;
        }
    }
}
