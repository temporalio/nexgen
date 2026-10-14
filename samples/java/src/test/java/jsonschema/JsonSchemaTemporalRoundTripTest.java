package jsonschema;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertThrows;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.google.protobuf.ByteString;
import io.temporal.api.common.v1.Payload;
import io.temporal.common.converter.DataConverter;
import io.temporal.common.converter.DefaultDataConverter;
import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.time.Duration;
import java.time.OffsetDateTime;
import java.time.ZoneOffset;
import org.junit.jupiter.api.Test;

import json_schema.definitions.temporal.Temporal;

/**
 * Round-trips the materialized temporal formats (date-time / date / time /
 * duration) through Temporal's default data converter. date-time -&gt;
 * OffsetDateTime, date -&gt; LocalDate, duration -&gt;
 * Duration, and time -&gt; a validated + canonicalized String. Serialization is generator-owned:
 * RFC 3339, offset preserved, +00:00/-00:00 -&gt; Z, trailing fractional zeros
 * trimmed, duration canonicalized to time-only PT...H...M...S.
 */
final class JsonSchemaTemporalRoundTripTest {

    private static final ObjectMapper MAPPER = new ObjectMapper();
    private static final DataConverter CONVERTER = DefaultDataConverter.newDefaultInstance();

    private static Path fixtureDir() {
        return Paths.get(System.getProperty("user.dir"), "..", "wire", "json_schema", "temporal")
                .normalize();
    }

    private static byte[] fixtureBytes(String name) throws IOException {
        return Files.readAllBytes(fixtureDir().resolve(name));
    }

    private static Payload jsonPayload(byte[] data) {
        return Payload.newBuilder()
                .putMetadata("encoding", ByteString.copyFromUtf8("json/plain"))
                .setData(ByteString.copyFrom(data))
                .build();
    }

    private static Temporal decode(String name) throws IOException {
        return CONVERTER.fromPayload(jsonPayload(fixtureBytes(name)), Temporal.class, Temporal.class);
    }

    private static JsonNode encode(Temporal value) throws IOException {
        Payload encoded = CONVERTER.toPayload(value).orElseThrow(AssertionError::new);
        return MAPPER.readTree(encoded.getData().toByteArray());
    }

    private static Temporal roundTrip(String name) throws IOException {
        Temporal value = decode(name);
        JsonNode got = encode(value);
        JsonNode want = MAPPER.readTree(fixtureBytes(name));
        assertEquals(want, got, name);
        return value;
    }

    @Test
    void fullFixtureRoundTrips() throws IOException {
        Temporal full = roundTrip("temporal-full.json");
        assertEquals(2021, full.getCreatedAt().getYear());
        assertEquals(2 * 60 * 60, full.getCreatedAt().getOffset().getTotalSeconds());
        assertEquals(123456000, full.getCreatedAt().getNano());
        assertEquals(Duration.ofMinutes(90), full.getTimeout());
        assertNotNull(full.getDeletedAt());
    }

    @Test
    void minimalFixtureRoundTrips() throws IOException {
        roundTrip("temporal-minimal.json");
    }

    @Test
    void canonicalizationNormalizesOnReserialize() throws IOException {
        Temporal value = decode("temporal-canonicalize.json");
        JsonNode got = encode(value);
        JsonNode want = MAPPER.readTree(
                "{\"createdAt\":\"2021-06-15T12:30:45Z\",\"birthday\":\"2021-02-28\","
                        + "\"alarm\":\"12:30:45Z\",\"timeout\":\"PT1H30M\"}");
        assertEquals(want, got);
    }

    @Test
    void optionalNullableNullsAreOmitted() throws IOException {
        Temporal value = decode("temporal-nulls.json");
        assertNull(value.getDeletedAt());
        assertNull(value.getArchivedOn());
        assertEquals(Duration.ZERO, value.getTimeout());
    }

    @Test
    void offsetBoundariesAreEnforcedInBothDirections() throws IOException {
        for (String offset : new String[] {"+14:00", "-14:00"}) {
            String json = "{\"createdAt\":\"2021-06-15T12:30:45" + offset
                    + "\",\"birthday\":\"2000-01-01\",\"alarm\":\"09:00:00.123456789123"
                    + offset + "\",\"timeout\":\"PT0S\"}";
            Temporal value = decodeBody(json);
            int sign = offset.charAt(0) == '-' ? -1 : 1;
            int seconds = sign * (Integer.parseInt(offset.substring(1, 3)) * 3600
                    + Integer.parseInt(offset.substring(4)) * 60);
            assertEquals(seconds, value.getCreatedAt().getOffset().getTotalSeconds());
            JsonNode encoded = encode(value);
            assertEquals("2021-06-15T12:30:45" + offset, encoded.get("createdAt").textValue());
            assertEquals("09:00:00.123456789123" + offset, encoded.get("alarm").textValue());
        }

        for (String offset : new String[] {"+14:01", "-14:01", "+18:00", "-18:00", "+23:59", "-23:59"}) {
            assertThrows(Exception.class, () -> decodeBody(
                    "{\"createdAt\":\"2021-06-15T12:30:45" + offset
                            + "\",\"birthday\":\"2000-01-01\",\"alarm\":\"09:00:00\",\"timeout\":\"PT0S\"}"));
            assertThrows(Exception.class, () -> decodeBody(
                    "{\"createdAt\":\"2021-06-15T12:30:45Z\",\"birthday\":\"2000-01-01\",\"alarm\":\"09:00:00"
                            + offset + "\",\"timeout\":\"PT0S\"}"));
        }

        Temporal valid = decode("temporal-minimal.json");
        OffsetDateTime subMinute = OffsetDateTime.of(
                2021, 6, 15, 12, 30, 45, 0, ZoneOffset.ofTotalSeconds(30));
        Temporal invalidDateTime = new Temporal(
                subMinute,
                valid.getBirthday(),
                valid.getAlarm(),
                valid.getTimeout(),
                valid.getUpdatedAt(),
                valid.getExpiresOn(),
                valid.getReminder(),
                valid.getRetryDelay(),
                valid.getDeletedAt(),
                valid.getArchivedOn());
        assertThrows(Exception.class, () -> encode(invalidDateTime));

        // OffsetDateTime admits up to +/-18:00, wider than the materialized domain.
        for (int hours : new int[] {15, -15, 18, -18}) {
            Temporal outOfRange = new Temporal(
                    OffsetDateTime.of(2021, 6, 15, 12, 30, 45, 0, ZoneOffset.ofHours(hours)),
                    valid.getBirthday(),
                    valid.getAlarm(),
                    valid.getTimeout(),
                    valid.getUpdatedAt(),
                    valid.getExpiresOn(),
                    valid.getReminder(),
                    valid.getRetryDelay(),
                    valid.getDeletedAt(),
                    valid.getArchivedOn());
            assertThrows(Exception.class, () -> encode(outOfRange));
        }

        for (String offset : new String[] {"+14:01", "-14:01", "+18:00", "-18:00", "+23:59", "-23:59"}) {
            Temporal invalidTime = new Temporal(
                    valid.getCreatedAt(),
                    valid.getBirthday(),
                    "09:00:00" + offset,
                    valid.getTimeout(),
                    valid.getUpdatedAt(),
                    valid.getExpiresOn(),
                    valid.getReminder(),
                    valid.getRetryDelay(),
                    valid.getDeletedAt(),
                    valid.getArchivedOn());
            assertThrows(Exception.class, () -> encode(invalidTime));
        }
    }

    @Test
    void materializedNarrowingRejects() {
        // leap second, calendar duration, invalid calendar date, missing offset.
        assertThrows(Exception.class, () -> decodeBody(
                "{\"createdAt\":\"2021-12-31T23:59:60Z\",\"birthday\":\"2000-01-01\",\"alarm\":\"09:00:00\",\"timeout\":\"PT0S\"}"));
        assertThrows(Exception.class, () -> decodeBody(
                "{\"createdAt\":\"2021-06-15T12:30:45Z\",\"birthday\":\"2000-01-01\",\"alarm\":\"09:00:00\",\"timeout\":\"P1Y\"}"));
        assertThrows(Exception.class, () -> decodeBody(
                "{\"createdAt\":\"2021-06-15T12:30:45Z\",\"birthday\":\"2021-02-29\",\"alarm\":\"09:00:00\",\"timeout\":\"PT0S\"}"));
        assertThrows(Exception.class, () -> decodeBody(
                "{\"createdAt\":\"2021-06-15T12:30:45\",\"birthday\":\"2000-01-01\",\"alarm\":\"09:00:00\",\"timeout\":\"PT0S\"}"));
        assertThrows(Exception.class, () -> decodeBody(
                "{\"createdAt\":\"0000-01-01T00:00:00Z\",\"birthday\":\"2000-01-01\",\"alarm\":\"09:00:00\",\"timeout\":\"PT0S\"}"));
        assertThrows(Exception.class, () -> decodeBody(
                "{\"createdAt\":\"2021-06-15T12:30:45Z\",\"birthday\":\"0000-01-01\",\"alarm\":\"09:00:00\",\"timeout\":\"PT0S\"}"));

        Temporal firstYear = decodeBody(
                "{\"createdAt\":\"0001-01-01T00:00:00Z\",\"birthday\":\"0001-01-01\",\"alarm\":\"00:00:00\",\"timeout\":\"PT0S\"}");
        assertEquals(1, firstYear.getCreatedAt().getYear());
        assertEquals(1, firstYear.getBirthday().getYear());
    }

    private static Temporal decodeBody(String json) {
        return CONVERTER.fromPayload(
                jsonPayload(json.getBytes(java.nio.charset.StandardCharsets.UTF_8)),
                Temporal.class,
                Temporal.class);
    }
}
