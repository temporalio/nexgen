package tests

import (
	"encoding/json"
	"testing"

	showcase "samples/go/showcase"

	"github.com/stretchr/testify/require"
	"go.temporal.io/sdk/converter"
)

// TestJSONSchemaShowcaseRuntime round-trips the showcase wire fixtures through
// the Temporal default data converter and asserts JSON-equality against the
// canonical fixtures. The showcase schema is a single pure JSON Schema file
// (no service) that exercises the whole supported keyword subset.
//
// Exception (see json-schema/nullability.md): optional+nullable fields collapse
// in Go. showcase-nulls.json carries middleName: null (optional+nullable), which
// Go collapses on serialize, so it is verified by deserialization + field checks
// rather than exact JSON-equality. (category is required+nullable, so its
// explicit null survives the round-trip.)
func TestJSONSchemaShowcaseRuntime(t *testing.T) {
	dc := converter.GetDefaultDataConverter()

	minimal := roundTripJSONEq[showcase.Showcase](t, dc, "showcase", "showcase-minimal.json")
	require.Equal(t, showcase.ShowcaseKindShowcase, minimal.Kind)
	require.Equal(t, "Widget", minimal.Name)
	require.Equal(t, int64(3), minimal.Count)
	require.True(t, minimal.Active)
	require.Nil(t, minimal.Retries)
	require.Equal(t, int64(3), minimal.RetriesOrDefault())
	// Scalar defaults of each kind: unset on the wire (nil), surfaced on read via
	// the generated OrDefault accessor (materialize-on-read, P9/P12).
	require.Nil(t, minimal.Greeting)
	require.Equal(t, "hello", minimal.GreetingOrDefault())
	require.Nil(t, minimal.Debug)
	require.Equal(t, false, minimal.DebugOrDefault())
	require.NotNil(t, minimal.Category)
	require.Equal(t, "tools", *minimal.Category)

	// Serialize side (P12): an unset default-bearing field is OMITTED on the wire
	// (no echo of the materialized default), even though it reads as the default.
	reMarshaled, err := dc.ToPayload(minimal)
	require.NoError(t, err)
	require.NotContains(t, string(reMarshaled.GetData()), "greeting")
	require.NotContains(t, string(reMarshaled.GetData()), "\"debug\"")
	require.NotContains(t, string(reMarshaled.GetData()), "retries")

	full := roundTripJSONEq[showcase.Showcase](t, dc, "showcase", "showcase-full.json")
	require.Equal(t, int64(42), full.Count)
	require.NotNil(t, full.Retries)
	require.Equal(t, int64(5), *full.Retries)
	require.NotNil(t, full.MiddleName)
	require.Equal(t, "Q", *full.MiddleName)
	require.Equal(t, []string{"a", "b"}, full.Tags)
	require.Equal(t, []string{"alpha", "beta"}, full.Aliases)
	require.Equal(t, []string{"admin", "user"}, full.Roles)
	require.NotNil(t, full.Address)
	require.Equal(t, "1 Main St", full.Address.Street)
	require.Contains(t, full.Address.AdditionalProperties, "region")
	require.NotNil(t, full.Labels)
	require.Equal(t, "prod", full.Labels.AdditionalProperties["env"])
	require.NotNil(t, full.Settings)
	require.NotNil(t, full.Settings.FontSize)
	require.Equal(t, int64(14), *full.Settings.FontSize)

	// showcase-nulls carries middleName: null (optional+nullable) — deserialization only.
	nulls := decodeFixture[showcase.Showcase](t, dc, "showcase", "showcase-nulls.json")
	require.Nil(t, nulls.MiddleName)
	require.Nil(t, nulls.Category)
	require.False(t, nulls.Active)

	address := roundTripJSONEq[showcase.Address](t, dc, "showcase", "address-open.json")
	require.Equal(t, "1 Main St", address.Street)
	require.Contains(t, address.AdditionalProperties, "x-extra")

	labels := roundTripJSONEq[showcase.Labels](t, dc, "showcase", "labels.json")
	require.Equal(t, "prod", labels.AdditionalProperties["env"])
	require.Equal(t, "core", labels.AdditionalProperties["team"])

	settings := roundTripJSONEq[showcase.Settings](t, dc, "showcase", "settings.json")
	require.NotNil(t, settings.Theme)
	require.Equal(t, "dark", *settings.Theme)
}

// TestJSONSchemaShowcaseNumericConstraints round-trips the numeric-constrained
// fields (minimum/maximum/exclusiveMinimum/multipleOf on integer and number
// fields) and asserts the runtime validator rejects out-of-bounds and
// non-multiple values with informative reasons in both directions.
func TestJSONSchemaShowcaseNumericConstraints(t *testing.T) {
	dc := converter.GetDefaultDataConverter()

	metrics := roundTripJSONEq[showcase.Showcase](t, dc, "showcase", "showcase-metrics.json")
	require.NotNil(t, metrics.Priority)
	require.Equal(t, int64(5), *metrics.Priority)
	require.NotNil(t, metrics.Level)
	require.Equal(t, int64(2), *metrics.Level)
	require.NotNil(t, metrics.Ratio)
	require.Equal(t, float64(15), *metrics.Ratio)
	require.NotNil(t, metrics.Step)
	require.Equal(t, int64(9), *metrics.Step)

	var out showcase.Showcase
	// An integer above `maximum` is rejected on deserialize.
	err := dc.FromPayload(jsonPayload([]byte(
		`{"kind":"showcase","name":"w","count":1,"active":true,"category":"tools","priority":99}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), "must be <= 10, got 99")

	// A non-multiple integer is rejected.
	err = dc.FromPayload(jsonPayload([]byte(
		`{"kind":"showcase","name":"w","count":1,"active":true,"category":"tools","step":7}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), "must be a multiple of 3, got 7")

	// A number below `minimum` and off the multipleOf grid is rejected.
	err = dc.FromPayload(jsonPayload([]byte(
		`{"kind":"showcase","name":"w","count":1,"active":true,"category":"tools","ratio":3}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), "must be >= 5, got 3")

	// Serialize side (P12): an in-memory value past the bound fails to marshal.
	bad := metrics
	badPriority := int64(42)
	bad.Priority = &badPriority
	_, serr := dc.ToPayload(bad)
	require.Error(t, serr)
	require.Contains(t, serr.Error(), "must be <= 10, got 42")
}

// TestJSONSchemaShowcaseStringLength round-trips string-length-constrained
// fields (minLength/maxLength counted in Unicode code points) and asserts the
// runtime validator rejects too-short / over-long values with informative
// reasons in both directions. The crux: length is counted in CODE POINTS, not
// bytes — verified with a multi-byte astral value.
func TestJSONSchemaShowcaseStringLength(t *testing.T) {
	dc := converter.GetDefaultDataConverter()

	// The astral crux: "a😀b" is 3 code points but 6 UTF-8 bytes; it must pass
	// `code` maxLength:5 (a naive byte count of 6 would wrongly reject it).
	strings := roundTripJSONEq[showcase.Showcase](t, dc, "showcase", "showcase-strings.json")
	require.NotNil(t, strings.Code)
	require.Equal(t, "a😀b", *strings.Code)
	require.NotNil(t, strings.Nickname)
	require.Equal(t, "buddy", *strings.Nickname)

	var out showcase.Showcase
	// A too-short `code` (1 code point, below minLength:2) is rejected.
	err := dc.FromPayload(jsonPayload([]byte(
		`{"kind":"showcase","name":"w","count":1,"active":true,"category":"tools","code":"a"}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), "must have length >= 2, got 1")

	// An over-long `code` (6 code points, above maxLength:5) is rejected.
	err = dc.FromPayload(jsonPayload([]byte(
		`{"kind":"showcase","name":"w","count":1,"active":true,"category":"tools","code":"abcdef"}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), "must have length <= 5, got 6")

	// An over-long astral `code` (6 emoji = 6 code points, 24 bytes) is rejected
	// by code-point count, not byte count.
	err = dc.FromPayload(jsonPayload([]byte(
		`{"kind":"showcase","name":"w","count":1,"active":true,"category":"tools","code":"😀😀😀😀😀😀"}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), "must have length <= 5, got 6")

	// Serialize side (P12): an in-memory over-long string fails to marshal.
	bad := strings
	tooLong := "abcdef"
	bad.Code = &tooLong
	_, serr := dc.ToPayload(bad)
	require.Error(t, serr)
	require.Contains(t, serr.Error(), "must have length <= 5, got 6")
}

// TestJSONSchemaShowcasePattern round-trips the pattern-constrained string
// fields (sku `^[A-Z]{2,4}$` and phrase `^\S+\s\S+$`) and asserts the runtime
// validator rejects non-matching values in both directions. The cross-language
// crux: the loader normalizes `\s`/`\S` to an explicit ASCII class and rewrites
// the `$` end-anchor per target, so a Unicode space (NBSP) and a trailing
// newline are rejected here exactly as they are in TS/Python/Java (the
// engines otherwise diverge on both).
func TestJSONSchemaShowcasePattern(t *testing.T) {
	dc := converter.GetDefaultDataConverter()

	patterns := roundTripJSONEq[showcase.Showcase](t, dc, "showcase", "showcase-patterns.json")
	require.NotNil(t, patterns.Sku)
	require.Equal(t, "AB", *patterns.Sku)
	require.NotNil(t, patterns.Phrase)
	require.Equal(t, "hello world", *patterns.Phrase)

	base := `{"kind":"showcase","name":"w","count":1,"active":true,"category":"tools"`
	var out showcase.Showcase

	// Lowercase sku (not [A-Z]).
	err := dc.FromPayload(jsonPayload([]byte(base+`,"sku":"ab"}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), "must match pattern")
	require.Contains(t, err.Error(), `got "ab"`)

	// Too-long sku (5 letters, above {2,4}).
	err = dc.FromPayload(jsonPayload([]byte(base+`,"sku":"ABCDE"}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), "must match pattern")

	// phrase with no whitespace separator.
	err = dc.FromPayload(jsonPayload([]byte(base+`,"phrase":"helloworld"}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), "must match pattern")

	// `\s` ASCII-class crux: a NBSP (U+00A0) is NOT ASCII whitespace, so the
	// normalized `[\t\n\x0B\f\r ]` rejects it (JS's native Unicode `\s` would
	// have accepted it — normalization makes Go/JS agree).
	err = dc.FromPayload(jsonPayload([]byte(base+`,"phrase":"hello world"}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), "must match pattern")

	// `$` end-anchor crux: a trailing newline is rejected (Python/Java `$` would
	// have matched before it — the per-target `\Z`/`\z` rewrite makes all four
	// agree with Go/JS end-of-input `$`).
	err = dc.FromPayload(jsonPayload([]byte(base+`,"phrase":"hello world\n"}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), "must match pattern")

	// Serialize side (P12): an in-memory off-pattern value fails to marshal.
	bad := patterns
	badSku := "xyz"
	bad.Sku = &badSku
	_, serr := dc.ToPayload(bad)
	require.Error(t, serr)
	require.Contains(t, serr.Error(), "must match pattern")
}

// TestJSONSchemaShowcaseFormat round-trips the asserted string-`format` fields
// (uuid/email/hostname/uri/ipv4 — all string-typed, no materialization) and
// asserts the runtime validator rejects malformed values with the informative
// `must be a valid <format>` reason in both directions. Each format lowers to a
// pinned, generator-owned regex (+ a length guard for email/hostname), so the
// verdicts agree across all four languages.
func TestJSONSchemaShowcaseFormat(t *testing.T) {
	dc := converter.GetDefaultDataConverter()

	formats := roundTripJSONEq[showcase.Showcase](t, dc, "showcase", "showcase-format.json")
	require.NotNil(t, formats.RequestID)
	require.Equal(t, "de305d54-75b4-431b-adb2-eb6b9e546013", *formats.RequestID)
	require.NotNil(t, formats.ContactEmail)
	require.Equal(t, "user@example.com", *formats.ContactEmail)
	require.NotNil(t, formats.Host)
	require.Equal(t, "api.example.com", *formats.Host)
	require.NotNil(t, formats.Homepage)
	require.Equal(t, "https://example.com/path?q=1#frag", *formats.Homepage)
	require.NotNil(t, formats.Gateway)
	require.Equal(t, "192.168.0.1", *formats.Gateway)

	base := `{"kind":"showcase","name":"w","count":1,"active":true,"category":"tools"`
	var out showcase.Showcase

	// A malformed uuid.
	err := dc.FromPayload(jsonPayload([]byte(base+`,"requestId":"not-a-uuid"}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), `must be a valid uuid, got "not-a-uuid"`)

	// An email whose domain is a single label (user@localhost is rejected).
	err = dc.FromPayload(jsonPayload([]byte(base+`,"contactEmail":"user@localhost"}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), `must be a valid email, got "user@localhost"`)

	// An ipv4 octet out of range.
	err = dc.FromPayload(jsonPayload([]byte(base+`,"gateway":"256.0.0.1"}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), `must be a valid ipv4, got "256.0.0.1"`)

	// A uri with a double-`::` IPv6 IP-literal host (spliced ipv6 grammar rejects).
	err = dc.FromPayload(jsonPayload([]byte(base+`,"homepage":"http://[1::2::3]"}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), "must be a valid uri")

	// An over-long hostname (> 253 code points) is rejected by the length guard.
	longHost := ""
	for i := 0; i < 64; i++ {
		if i > 0 {
			longHost += "."
		}
		longHost += "abc"
	}
	err = dc.FromPayload(jsonPayload([]byte(base+`,"host":"`+longHost+`"}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), "must be a valid hostname")

	// Serialize side (P12): an in-memory malformed uuid fails to marshal.
	bad := formats
	badID := "nope"
	bad.RequestID = &badID
	_, serr := dc.ToPayload(bad)
	require.Error(t, serr)
	require.Contains(t, serr.Error(), `must be a valid uuid, got "nope"`)
}

// TestJSONSchemaShowcaseContentEncoding round-trips the materialized
// `contentEncoding` fields (blob `base64`, urlBlob `base64url`) — a JSON string
// on the wire, native `[]byte` in the model — and asserts the runtime decoder
// rejects a malformed encoded value with the informative reason. The crux: the
// same bytes (">>>") encode to a different canonical wire under each encoding
// ("Pj4+" padded standard vs "Pj4-" unpadded URL-safe), yet decode to the same
// []byte.
func TestJSONSchemaShowcaseContentEncoding(t *testing.T) {
	dc := converter.GetDefaultDataConverter()

	// Decode → native bytes → re-encode → byte-identical wire (JSON-equality).
	bytesCase := roundTripJSONEq[showcase.Showcase](t, dc, "showcase", "showcase-bytes.json")
	require.Equal(t, []byte(">>>"), bytesCase.Blob)
	require.Equal(t, []byte(">>>"), bytesCase.UrlBlob)

	base := `{"kind":"showcase","name":"w","count":1,"active":true,"category":"tools"`
	var out showcase.Showcase

	// A base64 value using the URL-safe alphabet is rejected by the pinned regex.
	err := dc.FromPayload(jsonPayload([]byte(base+`,"blob":"Pj4-"}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), `must be base64-encoded, got "Pj4-"`)

	// A base64 value missing padding is rejected.
	err = dc.FromPayload(jsonPayload([]byte(base+`,"blob":"aGk"}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), "must be base64-encoded")

	// A base64url value carrying padding (standard alphabet) is rejected.
	err = dc.FromPayload(jsonPayload([]byte(base+`,"urlBlob":"aGk="}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), `must be base64url-encoded, got "aGk="`)

	// Embedded stray character is rejected.
	err = dc.FromPayload(jsonPayload([]byte(base+`,"blob":"aGk!"}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), "must be base64-encoded")
}

// TestJSONSchemaShowcaseArrayConstraints round-trips the array-constrained
// fields (minItems/maxItems, uniqueItems, contains + minContains/maxContains)
// and asserts the runtime validator rejects too-few/too-many items, duplicates,
// a missing required contains match, and an out-of-range match count with
// informative reasons in both directions.
func TestJSONSchemaShowcaseArrayConstraints(t *testing.T) {
	dc := converter.GetDefaultDataConverter()

	base := `{"kind":"showcase","name":"w","count":1,"active":true,"category":"tools"`

	var out showcase.Showcase
	// Too few tags (minItems:1).
	err := dc.FromPayload(jsonPayload([]byte(base+`,"tags":[]}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), "must have at least 1 items, got 0")

	// Too many tags (maxItems:5).
	err = dc.FromPayload(jsonPayload([]byte(base+`,"tags":["a","b","c","d","e","f"]}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), "must have at most 5 items, got 6")

	// Duplicate aliases (uniqueItems).
	err = dc.FromPayload(jsonPayload([]byte(base+`,"aliases":["x","x"]}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), "duplicate items: element at index 1 equals index 0")

	// Missing required contains match (roles has no "admin").
	err = dc.FromPayload(jsonPayload([]byte(base+`,"roles":["user"]}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), "too few matching items: at least 1, got 0")

	// Too many contains matches (maxContains:2).
	err = dc.FromPayload(jsonPayload([]byte(base+`,"roles":["admin","admin","admin"]}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), "too many matching items: at most 2, got 3")

	// Serialize side (P12): an in-memory duplicate fails to marshal.
	valid := decodeFixture[showcase.Showcase](t, dc, "showcase", "showcase-full.json")
	bad := valid
	bad.Aliases = []string{"dup", "dup"}
	_, serr := dc.ToPayload(bad)
	require.Error(t, serr)
	require.Contains(t, serr.Error(), "duplicate items: element at index 1 equals index 0")
}

// TestJSONSchemaShowcaseObjectConstraints round-trips the object-constrained
// types (minProperties/maxProperties over the distinct wire-key count,
// propertyNames key-shape on a map, dependentRequired cross-field presence) and
// asserts the runtime validator rejects too-few/too-many members, an invalid
// property name, and a missing dependent-required member with informative
// reasons in both directions.
func TestJSONSchemaShowcaseObjectConstraints(t *testing.T) {
	dc := converter.GetDefaultDataConverter()

	attributes := roundTripJSONEq[showcase.Attributes](t, dc, "showcase", "attributes.json")
	require.Equal(t, "a", attributes.AdditionalProperties["host"])
	require.Equal(t, "8080", attributes.AdditionalProperties["port"])

	contact := roundTripJSONEq[showcase.ContactGo](t, dc, "showcase", "contact.json")
	require.NotNil(t, contact.ShippingStreet)
	require.Equal(t, "1 Main St", *contact.ShippingStreet)
	require.NotNil(t, contact.ShippingZip)
	require.Equal(t, "90210", *contact.ShippingZip)

	var attrs showcase.Attributes
	// Too few members (minProperties:1) — an empty map.
	err := dc.FromPayload(jsonPayload([]byte(`{}`)), &attrs)
	require.Error(t, err)
	require.Contains(t, err.Error(), "must have at least 1 properties, got 0")

	// Too many members (maxProperties:3).
	err = dc.FromPayload(jsonPayload([]byte(`{"a":"1","b":"2","c":"3","d":"4"}`)), &attrs)
	require.Error(t, err)
	require.Contains(t, err.Error(), "must have at most 3 properties, got 4")

	// An over-long key (propertyNames maxLength:8).
	err = dc.FromPayload(jsonPayload([]byte(`{"toolongkey":"1"}`)), &attrs)
	require.Error(t, err)
	require.Contains(t, err.Error(), `invalid property name "toolongkey": must have length <= 8, got 10`)

	var out showcase.ContactGo
	// A shipping street present without a shipping zip (dependentRequired).
	err = dc.FromPayload(jsonPayload([]byte(`{"shippingStreet":"1 Main St"}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), `property "shippingZip" is required when "shippingStreet" is present`)

	// An empty contact object is below minProperties:1.
	err = dc.FromPayload(jsonPayload([]byte(`{}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), "must have at least 1 properties, got 0")

	// Serialize side (P12): an in-memory Contact with a shipping street but no
	// zip fails to marshal.
	street := "1 Main St"
	badContact := showcase.ContactGo{ShippingStreet: &street}
	_, serr := dc.ToPayload(badContact)
	require.Error(t, serr)
	require.Contains(t, serr.Error(), `property "shippingZip" is required when "shippingStreet" is present`)
}

// TestJSONSchemaShowcaseAllOfMerge round-trips the Widget type, produced by an
// allOf base-type extension (WidgetBase folded in + an extension branch). The
// merged type is an ordinary flat object with the union of properties and
// required, and its `size` member carries a bound tightened from two allOf
// branches to [10, 20]; a value outside it is rejected in both directions.
func TestJSONSchemaShowcaseAllOfMerge(t *testing.T) {
	dc := converter.GetDefaultDataConverter()

	widget := roundTripJSONEq[showcase.Widget](t, dc, "showcase", "widget.json")
	require.Equal(t, "w-1", widget.ID)
	require.NotNil(t, widget.Kind)
	require.Equal(t, "gadget", *widget.Kind)
	require.Equal(t, "Widget One", widget.Name)
	require.NotNil(t, widget.Size)
	require.Equal(t, int64(15), *widget.Size)

	base := `{"id":"w-1","name":"Widget One"`
	var out showcase.Widget
	// A size below the merged (tightened) minimum 10 is rejected.
	err := dc.FromPayload(jsonPayload([]byte(base+`,"size":5}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), "must be >= 10, got 5")

	// A size above the merged (tightened) maximum 20 is rejected.
	err = dc.FromPayload(jsonPayload([]byte(base+`,"size":25}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), "must be <= 20, got 25")

	// A missing required member contributed by the extension branch is rejected.
	err = dc.FromPayload(jsonPayload([]byte(`{"id":"w-1"}`)), &out)
	require.Error(t, err)

	// Serialize side (P12): an in-memory value past the tightened bound fails.
	bad := widget
	badSize := int64(25)
	bad.Size = &badSize
	_, serr := dc.ToPayload(bad)
	require.Error(t, serr)
	require.Contains(t, serr.Error(), "must be <= 20, got 25")
}

// TestJSONSchemaShowcaseClosedValues round-trips the closed value-set fields
// (const on integer/boolean/string, enum on string/integer/number) and asserts
// the runtime validator rejects off-set values with informative reasons in both
// directions. Each closed value is a Go defined type + typed value constant(s).
func TestJSONSchemaShowcaseClosedValues(t *testing.T) {
	dc := converter.GetDefaultDataConverter()

	full := roundTripJSONEq[showcase.Showcase](t, dc, "showcase", "showcase-full.json")
	require.Equal(t, showcase.ShowcaseKindShowcase, full.Kind)
	require.Equal(t, showcase.RevisionGo, full.Revision)
	require.Equal(t, showcase.ShowcaseEnabledTrue, full.Enabled)
	require.Equal(t, showcase.ActiveGo, full.Status)
	require.Equal(t, showcase.ShowcaseTier2, full.Tier)
	require.Equal(t, showcase.ShowcaseScale1_5, full.Scale)

	base := `{"kind":"showcase","revision":1,"enabled":true,"status":"active","tier":1,"scale":1.5,"name":"w","count":1,"active":true,"category":"tools"`
	var out showcase.Showcase

	// Wrong integer const value.
	err := dc.FromPayload(jsonPayload([]byte(base+`,"revision":2}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), "must equal 1")

	// Wrong boolean const value.
	err = dc.FromPayload(jsonPayload([]byte(base+`,"enabled":false}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), "must equal true")

	// Out-of-set string enum.
	err = dc.FromPayload(jsonPayload([]byte(base+`,"status":"archived"}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), `must be one of ["active","inactive","pending"], got "archived"`)

	// Out-of-set integer enum.
	err = dc.FromPayload(jsonPayload([]byte(base+`,"tier":9}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), "must be one of [1,2,3], got 9")

	// Out-of-set number enum (float exactness).
	err = dc.FromPayload(jsonPayload([]byte(base+`,"scale":3.5}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), "must be one of [1.5,2.5], got 3.5")

	// Serialize side (P12): a zero-value / mutated closed field fails to marshal.
	bad := full
	bad.Status = showcase.ShowcaseStatus("archived")
	_, serr := dc.ToPayload(bad)
	require.Error(t, serr)
	require.Contains(t, serr.Error(), `must be one of ["active","inactive","pending"], got "archived"`)
}

// TestJSONSchemaShowcaseUnions round-trips each branch of the two showcase
// oneOf sum types — the disjoint-kind union `idOrName` (string | integer) and
// the discriminated union `shape` (Circle | Square) — and asserts the runtime
// dispatcher rejects an unmatchable wire token and an unknown discriminator
// value with an informative reason.
func TestJSONSchemaShowcaseUnions(t *testing.T) {
	dc := converter.GetDefaultDataConverter()

	// Disjoint-kind union: string branch.
	s := roundTripJSONEq[showcase.Showcase](t, dc, "showcase", "showcase-union-string.json")
	require.Equal(t, showcase.ShowcaseIDOrNameString("abc"), s.IDOrName)

	// Disjoint-kind union: integer branch.
	i := roundTripJSONEq[showcase.Showcase](t, dc, "showcase", "showcase-union-int.json")
	require.Equal(t, showcase.ShowcaseIDOrNameInteger(7), i.IDOrName)

	// Discriminated union: circle branch.
	circle := roundTripJSONEq[showcase.Showcase](t, dc, "showcase", "showcase-shape-circle.json")
	c, ok := circle.Shape.(showcase.Circle)
	require.True(t, ok, "shape should be a Circle")
	require.Equal(t, showcase.CircleKindCircle, c.Kind)
	require.Equal(t, 2.5, c.Radius)

	// Discriminated union: square branch.
	square := roundTripJSONEq[showcase.Showcase](t, dc, "showcase", "showcase-shape-square.json")
	sq, ok := square.Shape.(showcase.Square)
	require.True(t, ok, "shape should be a Square")
	require.Equal(t, showcase.SquareKindSquare, sq.Kind)
	require.Equal(t, float64(4), sq.Side)

	base := `{"kind":"showcase","revision":1,"enabled":true,"status":"active","tier":1,"scale":1.5,"name":"w","count":1,"active":true,"category":"tools"`
	var out showcase.Showcase

	// Disjoint-kind union: an unmatchable token (boolean) is rejected naming the
	// admissible kinds.
	err := dc.FromPayload(jsonPayload([]byte(base+`,"idOrName":true}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), "idOrName")
	require.Contains(t, err.Error(), "string")
	require.Contains(t, err.Error(), "integer")

	// Tagged union: an unknown discriminator value is rejected naming the
	// admissible values (closed value set, P13.1).
	err = dc.FromPayload(jsonPayload([]byte(base+`,"shape":{"kind":"triangle"}}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), "shape")
	require.Contains(t, err.Error(), "triangle")
	require.Contains(t, err.Error(), "circle")

	// Tagged union: an absent discriminator is rejected (it is required).
	err = dc.FromPayload(jsonPayload([]byte(base+`,"shape":{"radius":1}}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), "shape")
}

// TestJSONSchemaShowcaseBranchConstraints asserts that once the token selects a
// branch, the value is held to everything that branch declares — `idOrName`'s
// `minLength`/`minimum`, `mode`'s closed string value set — in both directions,
// with the union's own path on the violation.
func TestJSONSchemaShowcaseBranchConstraints(t *testing.T) {
	dc := converter.GetDefaultDataConverter()

	base := `{"kind":"showcase","revision":1,"enabled":true,"status":"active","tier":1,"scale":1.5,"name":"w","count":1,"active":true,"category":"tools"`
	var out showcase.Showcase

	// The selected string branch's own `minLength`.
	err := dc.FromPayload(jsonPayload([]byte(base+`,"idOrName":"ab"}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), "idOrName: must have length >= 3, got 2")

	// The selected integer branch's own `minimum` — the other branch's
	// constraints are irrelevant to it.
	err = dc.FromPayload(jsonPayload([]byte(base+`,"idOrName":0}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), "idOrName: must be >= 1, got 0")

	// A closed value set on a branch: an unknown string is rejected naming the
	// admissible values, while the integer branch stays unbounded above.
	err = dc.FromPayload(jsonPayload([]byte(base+`,"mode":"turbo"}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), `mode: must be one of ["auto","manual"], got "turbo"`)
	mode := roundTripJSONEq[showcase.Showcase](t, dc, "showcase", "showcase-full.json")
	require.Equal(t, showcase.ShowcaseModeString("auto"), mode.Mode)

	// Serialize re-runs the selected branch's constraints (P12): an in-memory
	// member violating its own branch is rejected before any bytes are written.
	invalid := mode
	invalid.IDOrName = showcase.ShowcaseIDOrNameString("ab")
	_, err = dc.ToPayload(invalid)
	require.Error(t, err)
	require.Contains(t, err.Error(), "idOrName: must have length >= 3, got 2")

	invalid = mode
	invalid.Mode = showcase.ShowcaseModeInteger(-1)
	_, err = dc.ToPayload(invalid)
	require.Error(t, err)
	require.Contains(t, err.Error(), "mode: must be >= 0, got -1")
}

// TestJSONSchemaShowcaseFreeFormObject round-trips the free-form object in both
// positions: the named `Extras` model and the inline object branch of the
// `payload` union. Members are carried verbatim, so a large integer survives
// untruncated, and the member-count bound on `Extras` is enforced both
// directions.
func TestJSONSchemaShowcaseFreeFormObject(t *testing.T) {
	dc := converter.GetDefaultDataConverter()

	// Union branch selected by the object token: members kept verbatim.
	object := roundTripJSONEq[showcase.Showcase](t, dc, "showcase", "showcase-freeform.json")
	payload, ok := object.Payload.(showcase.ShowcasePayloadObject)
	require.True(t, ok, "payload should be the object branch")
	require.Equal(t, "9007199254740992", string(payload.AdditionalProperties["big"]))
	require.NotNil(t, object.Extras)
	require.Equal(t, `"free-form"`, string(object.Extras.AdditionalProperties["note"]))

	// The same union's string branch, selected by the string token.
	text := roundTripJSONEq[showcase.Showcase](t, dc, "showcase", "showcase-freeform-string.json")
	require.Equal(t, showcase.ShowcasePayloadString("text"), text.Payload)

	// The named free-form model round-trips standalone, nested members included.
	extras := roundTripJSONEq[showcase.Extras](t, dc, "showcase", "extras.json")
	require.JSONEq(t, `{"a":1}`, string(extras.AdditionalProperties["nested"]))
	require.Equal(t, "9007199254740992", string(extras.AdditionalProperties["count"]))

	// maxProperties over the member set is enforced on parse…
	var tooMany showcase.Extras
	err := dc.FromPayload(jsonPayload([]byte(`{"a":1,"b":2,"c":3,"d":4,"e":5}`)), &tooMany)
	require.Error(t, err)
	require.Contains(t, err.Error(), "at most 4 properties")

	// …and on serialize (P12).
	_, err = dc.ToPayload(showcase.Extras{AdditionalProperties: map[string]json.RawMessage{
		"a": json.RawMessage("1"),
		"b": json.RawMessage("2"),
		"c": json.RawMessage("3"),
		"d": json.RawMessage("4"),
		"e": json.RawMessage("5"),
	}})
	require.Error(t, err)
	require.Contains(t, err.Error(), "at most 4 properties")

	// An unmatchable token for the union is rejected naming the admissible kinds.
	base := `{"kind":"showcase","revision":1,"enabled":true,"status":"active","tier":1,"scale":1.5,"name":"w","count":1,"active":true,"category":"tools"`
	var out showcase.Showcase
	err = dc.FromPayload(jsonPayload([]byte(base+`,"payload":true}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), "payload")
	require.Contains(t, err.Error(), "ShowcasePayloadObject")
}

// TestJSONSchemaShowcaseInlineObjectUnion round-trips the `note` tagged union,
// whose branches are written inline in the schema and named by their
// `x-go-name` overrides: each is emitted as a full model (fields, constraints,
// verbatim extras) that implements the union interface.
func TestJSONSchemaShowcaseInlineObjectUnion(t *testing.T) {
	dc := converter.GetDefaultDataConverter()

	text := roundTripJSONEq[showcase.Showcase](t, dc, "showcase", "showcase-note-text.json")
	note, ok := text.Note.(showcase.TextNote)
	require.True(t, ok, "note should be a TextNote")
	require.Equal(t, showcase.TextNoteKindText, note.Kind)
	require.Equal(t, "remember the milk", note.Body)
	// The branch stays open: an unknown member is preserved (P13).
	require.Equal(t, "true", string(note.AdditionalProperties["pinned"]))

	link := roundTripJSONEq[showcase.Showcase](t, dc, "showcase", "showcase-note-link.json")
	href, ok := link.Note.(showcase.LinkNote)
	require.True(t, ok, "note should be a LinkNote")
	require.Equal(t, "https://example.test/notes/1", href.Href)

	base := `{"kind":"showcase","revision":1,"enabled":true,"status":"active","tier":1,"scale":1.5,"name":"w","count":1,"active":true,"category":"tools"`
	var out showcase.Showcase

	// The branch's own constraints are enforced once the tag selects it.
	err := dc.FromPayload(jsonPayload([]byte(base+`,"note":{"kind":"text","body":""}}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), "note.body")
	require.Contains(t, err.Error(), "length >= 1")

	// An unknown tag value is rejected naming the admissible values.
	err = dc.FromPayload(jsonPayload([]byte(base+`,"note":{"kind":"audio"}}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), "note")
	require.Contains(t, err.Error(), "audio")

	// Serialize re-runs the selected branch's constraints (P12).
	invalid := link
	invalid.Note = showcase.LinkNote{Kind: showcase.LinkNoteKindLink, Href: ""}
	_, err = dc.ToPayload(invalid)
	require.Error(t, err)
	require.Contains(t, err.Error(), "note.href")
}

// TestJSONSchemaShowcasePropertyInlineObjectUnion round-trips `detail`, a union
// written inline on a property whose lone object branch is structured: the branch
// derives `ShowcaseDetailObject` from the union it belongs to and is emitted as an
// ordinary model, while the string branch still selects on its own token.
func TestJSONSchemaShowcasePropertyInlineObjectUnion(t *testing.T) {
	dc := converter.GetDefaultDataConverter()

	object := roundTripJSONEq[showcase.Showcase](t, dc, "showcase", "showcase-detail-object.json")
	detail, ok := object.Detail.(showcase.ShowcaseDetailObject)
	require.True(t, ok, "detail should be a ShowcaseDetailObject")
	require.Equal(t, "E_LIMIT", detail.Code)
	require.Equal(t, "retry later", *detail.Hint)
	// The branch stays open: an unknown member is preserved (P13).
	require.Equal(t, "250", string(detail.AdditionalProperties["retryAfterMs"]))

	text := roundTripJSONEq[showcase.Showcase](t, dc, "showcase", "showcase-detail-string.json")
	require.Equal(t, showcase.ShowcaseDetailString("E_LIMIT"), text.Detail)

	base := `{"kind":"showcase","revision":1,"enabled":true,"status":"active","tier":1,"scale":1.5,"name":"w","count":1,"active":true,"category":"tools"`
	var out showcase.Showcase

	// The object branch's own constraints are enforced.
	err := dc.FromPayload(jsonPayload([]byte(base+`,"detail":{"code":""}}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), "detail.code")
	require.Contains(t, err.Error(), "length >= 1")

	// A token admitted by no branch names the admissible ones.
	err = dc.FromPayload(jsonPayload([]byte(base+`,"detail":7}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), "detail")
	require.Contains(t, err.Error(), "ShowcaseDetailObject")

	// Serialize re-runs the selected branch's constraints (P12).
	invalid := object
	invalid.Detail = showcase.ShowcaseDetailObject{Code: ""}
	_, err = dc.ToPayload(invalid)
	require.Error(t, err)
	require.Contains(t, err.Error(), "detail.code")
}

// TestJSONSchemaShowcaseTaggedUnionWithScalarBranch round-trips `shapeOrName`, a
// union whose two selector layers compose: the wire token picks object-vs-string
// and, for the object token, the shared required `kind` const picks
// Circle-vs-Square. Circle and Square belong to the `shape` union as well, so
// each carries a second marker method.
func TestJSONSchemaShowcaseTaggedUnionWithScalarBranch(t *testing.T) {
	dc := converter.GetDefaultDataConverter()

	square := roundTripJSONEq[showcase.Showcase](t, dc, "showcase", "showcase-shape-or-name-square.json")
	sq, ok := square.ShapeOrName.(showcase.Square)
	require.True(t, ok, "shapeOrName should be a Square")
	require.Equal(t, showcase.SquareKindSquare, sq.Kind)
	require.Equal(t, float64(4), sq.Side)

	named := roundTripJSONEq[showcase.Showcase](t, dc, "showcase", "showcase-shape-or-name-string.json")
	require.Equal(t, showcase.ShowcaseShapeOrNameString("unit-square"), named.ShapeOrName)

	base := `{"kind":"showcase","revision":1,"enabled":true,"status":"active","tier":1,"scale":1.5,"name":"w","count":1,"active":true,"category":"tools"`
	var out showcase.Showcase

	// The object token routes through the discriminator, so an unknown tag is
	// rejected rather than falling back to the string branch.
	err := dc.FromPayload(jsonPayload([]byte(base+`,"shapeOrName":{"kind":"triangle"}}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), "shapeOrName")
	require.Contains(t, err.Error(), "triangle")
	require.Contains(t, err.Error(), "square")

	// A token admitted by no branch names all admissible ones.
	err = dc.FromPayload(jsonPayload([]byte(base+`,"shapeOrName":7}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), "shapeOrName")
	require.Contains(t, err.Error(), "Circle, Square, string")

	// Serialize re-runs the selected branch's constraints (P12).
	invalid := square
	invalid.ShapeOrName = showcase.Square{Kind: showcase.SquareKind("hexagon")}
	_, err = dc.ToPayload(invalid)
	require.Error(t, err)
	require.Contains(t, err.Error(), "shapeOrName.kind")
}

// TestJSONSchemaShowcaseArrayBranchUnion round-trips `measurements`, a union with
// an array branch: the array has no definition to take a name from, so Go emits
// it as the synthesized `ShowcaseMeasurementsArray` variant alongside the string
// variant, both selected by the wire token.
func TestJSONSchemaShowcaseArrayBranchUnion(t *testing.T) {
	dc := converter.GetDefaultDataConverter()

	values := roundTripJSONEq[showcase.Showcase](t, dc, "showcase", "showcase-measurements-array.json")
	require.Equal(t, showcase.ShowcaseMeasurementsArray{1.5, 2.5, 3.75}, values.Measurements)

	preset := roundTripJSONEq[showcase.Showcase](t, dc, "showcase", "showcase-measurements-string.json")
	require.Equal(t, showcase.ShowcaseMeasurementsString("auto"), preset.Measurements)

	base := `{"kind":"showcase","revision":1,"enabled":true,"status":"active","tier":1,"scale":1.5,"name":"w","count":1,"active":true,"category":"tools"`
	var out showcase.Showcase

	// A token admitted by neither branch names both admissible kinds.
	err := dc.FromPayload(jsonPayload([]byte(base+`,"measurements":true}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), "measurements")
	require.Contains(t, err.Error(), "[]float64, string")

	// The array branch's element type is enforced: a string element is not a
	// number.
	err = dc.FromPayload(jsonPayload([]byte(base+`,"measurements":["x"]}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), "measurements")
}

// TestJSONSchemaShowcaseElementUnions round-trips the union positions that are
// not a property of their own: an array element (`shapes` at a named union,
// `segments` at an inline one the loader names `ShowcaseSegmentsItem`) and a map
// member (`choices`, whose inline member union is named `ChoicesValue`).
// `encoding/json` cannot decode into a sealed interface, so each of these is
// dispatched one value at a time, with the element index / member key threaded
// into the violation path.
func TestJSONSchemaShowcaseElementUnions(t *testing.T) {
	dc := converter.GetDefaultDataConverter()

	value := roundTripJSONEq[showcase.Showcase](t, dc, "showcase", "showcase-element-unions.json")

	require.Len(t, value.Shapes, 2)
	circle, ok := value.Shapes[0].(showcase.Circle)
	require.True(t, ok, "shapes[0] should be a Circle")
	require.Equal(t, 2.5, circle.Radius)
	square, ok := value.Shapes[1].(showcase.Square)
	require.True(t, ok, "shapes[1] should be a Square")
	require.Equal(t, float64(4), square.Side)

	require.Equal(
		t,
		[]showcase.ShowcaseSegmentsItem{
			showcase.ShowcaseSegmentsItemString("alpha"),
			showcase.ShowcaseSegmentsItemInteger(7),
		},
		value.Segments,
	)

	// Element nullability is the element's own concern: the slice stays a
	// slice, its members become pointers, and an explicit null decodes to nil.
	require.Len(t, value.Slots, 3)
	require.Equal(t, "first", *value.Slots[0])
	require.Nil(t, value.Slots[1])
	require.Equal(t, "third", *value.Slots[2])

	require.NotNil(t, value.Choices)
	require.Len(t, value.Choices.AdditionalProperties, 2)
	primary, ok := value.Choices.AdditionalProperties["primary"].(showcase.Circle)
	require.True(t, ok, "choices[primary] should be a Circle")
	require.Equal(t, float64(1), primary.Radius)

	base := `{"kind":"showcase","revision":1,"enabled":true,"status":"active","tier":1,"scale":1.5,"name":"w","count":1,"active":true,"category":"tools"`
	var out showcase.Showcase

	// A bad element is reported at its own index, not at the array.
	err := dc.FromPayload(jsonPayload([]byte(base+`,"shapes":[{"kind":"circle","radius":1},true]}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), "shapes[1]")
	require.Contains(t, err.Error(), "Circle, Square")

	// An unknown discriminator inside an element is still routed by tag.
	err = dc.FromPayload(jsonPayload([]byte(base+`,"shapes":[{"kind":"triangle"}]}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), "shapes[0]")
	require.Contains(t, err.Error(), "triangle")

	// The inline element union is a closed sum type like any other.
	err = dc.FromPayload(jsonPayload([]byte(base+`,"segments":["ok",1.5]}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), "segments[1]")

	// A map member's violation carries its key.
	err = dc.FromPayload(jsonPayload([]byte(base+`,"choices":{"primary":"circle"}}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), "primary")
	require.Contains(t, err.Error(), "Circle, Square")

	// Serialize re-runs each element's own branch constraints (P12).
	invalid := value
	invalid.Shapes = []showcase.Shape{showcase.Square{Kind: showcase.SquareKind("hexagon")}}
	_, err = dc.ToPayload(invalid)
	require.Error(t, err)
	require.Contains(t, err.Error(), "shapes[0].kind")
}

// TestJSONSchemaShowcaseInlineShapes round-trips the object shapes the schema
// writes **inline** rather than in `$defs` — a property (`location`, with its own
// nested `geo`), a nullable property (`audit`), an array element (`rows`), a map
// and its member (`ledger`), and a free-form bag (`metadata`) — each of which the
// loader names after the position it occupies and emits as an ordinary model. It
// also covers the constraints a typed map's members carry (`quotas`, `tokens`,
// `nicknames`) and a nested array (`grid`), both directions.
func TestJSONSchemaShowcaseInlineShapes(t *testing.T) {
	dc := converter.GetDefaultDataConverter()

	value := roundTripJSONEq[showcase.Showcase](t, dc, "showcase", "showcase-inline-shapes.json")

	require.Equal(t, [][]int64{{1, 2}, {3}}, value.Grid)
	require.NotNil(t, value.Location)
	require.Equal(t, "Springfield", value.Location.City)
	require.NotNil(t, value.Location.Geo)
	require.Equal(t, 39.8, *value.Location.Geo.Lat)
	require.NotNil(t, value.Audit)
	require.Equal(t, "alice", value.Audit.By)
	require.Len(t, value.Rows, 2)
	require.Equal(t, "a1", value.Rows[0].Cell)
	// The member override renamed the member (`LedgerGo`); the hoisted types keep
	// their position-derived names (`ShowcaseLedger`, `ShowcaseLedgerValue`).
	require.NotNil(t, value.LedgerGo)
	require.Equal(t, int64(100), value.LedgerGo.AdditionalProperties["opening"].Amount)
	require.NotNil(t, value.Metadata)
	require.Len(t, value.Metadata.AdditionalProperties, 2)
	require.NotNil(t, value.Quotas)
	require.Equal(t, int64(20), value.Quotas.AdditionalProperties["cpu"])
	require.NotNil(t, value.Nicknames)
	require.Equal(t, "al", *value.Nicknames.AdditionalProperties["short"])
	require.Nil(t, value.Nicknames.AdditionalProperties["none"])

	base := `{"kind":"showcase","revision":1,"enabled":true,"status":"active","tier":1,"scale":1.5,"name":"w","count":1,"active":true,"category":"tools"`
	var out showcase.Showcase

	// A hoisted shape validates like any other model: its member constraints and
	// its required members are enforced, at the nested path.
	err := dc.FromPayload(jsonPayload([]byte(base+`,"location":{"city":""}}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), "location.city")

	// An element's own model validates too. Go decodes the array as a whole, so
	// the failure is reported at the array rather than at `rows[1].cell` — the
	// element type is materialized either way, and elementwise dispatch for a
	// model element is tracked with the item-constraint work.
	err = dc.FromPayload(jsonPayload([]byte(base+`,"rows":[{"cell":"ok"},{}]}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), "rows")

	// A typed map's member constraints are enforced, keyed by the member.
	err = dc.FromPayload(jsonPayload([]byte(base+`,"quotas":{"cpu":7}}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), "cpu")
	require.Contains(t, err.Error(), "multiple of 5")

	err = dc.FromPayload(jsonPayload([]byte(base+`,"tokens":{"primary":"AB"}}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), "primary")

	// A null member of a nullable map is a member, not a violation; a present one
	// still carries its constraint.
	err = dc.FromPayload(jsonPayload([]byte(base+`,"nicknames":{"tiny":"a"}}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), "tiny")

	// The free-form bag's member-count bound rides with the hoisted type.
	err = dc.FromPayload(jsonPayload([]byte(base+`,"metadata":{"a":1,"b":2,"c":3,"d":4}}`)), &out)
	require.Error(t, err)
	require.Contains(t, err.Error(), "at most 3")

	// Serialize re-runs every member's own constraints before emitting (P12).
	invalid := value
	invalid.Quotas = &showcase.Quotas{AdditionalProperties: map[string]int64{"cpu": 7}}
	_, err = dc.ToPayload(invalid)
	require.Error(t, err)
	require.Contains(t, err.Error(), "cpu")

	invalid = value
	invalid.Tokens = &showcase.Tokens{AdditionalProperties: map[string]string{"primary": "AB"}}
	_, err = dc.ToPayload(invalid)
	require.Error(t, err)
	require.Contains(t, err.Error(), "primary")
}
