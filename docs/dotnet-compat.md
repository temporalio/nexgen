# Using generated C# on older .NET platforms

`nexgen` generates C# for .NET 7 (`net7.0`) or later. CI builds the generated
samples for `net7.0` and runs their tests on .NET 7, 8, and 10. Those are the
only configurations the project supports.

The Temporal .NET SDK (`Temporalio`) reaches further back: .NET Framework
4.6.2+, .NET Core 3.1+ (so .NET 5, 6, and 7), and .NET Standard 2.0. This
document explains what stops generated code from compiling or running on those
older platforms, and what you can do about it in your own project. The C#
backend emits different code for the two input formats, so JSON Schema and WIT
are covered separately.

> **Status.** Everything below is unsupported. The workarounds were checked by
> applying them, with the packages this guide names, to projects that compile
> the checked-in generated samples unchanged. The samples' test suites then
> passed on .NET Core 3.1, .NET 5, and .NET 6 (JSON Schema) and on .NET Core 3.1
> and .NET 5 (WIT). .NET Framework 4.6.2 and .NET Standard 2.0/2.1 builds were
> compiled but never executed. `nexgen` does not test any of this continuously,
> so a future release may break it without notice.

## Summary

The C# that `nexgen` generates works unchanged on .NET 7 and later. Getting it
to work on older .NET platforms requires some workarounds, and which ones
depends on the platform and on the input format you generate from:

| Target                                  | WIT input                                                   | JSON Schema input                                                                         |
| --------------------------------------- | ----------------------------------------------------------- | ----------------------------------------------------------------------------------------- |
| .NET 7 and later                        | Supported                                                   | Supported                                                                                 |
| .NET 6                                  | Works unchanged                                             | `LangVersion` 14, attribute/API polyfills                                                 |
| .NET 5                                  | Works unchanged                                             | `LangVersion` 14, attribute/API polyfills, `DateOnly` backport ¹, System.Text.Json 6.0+ ³ |
| .NET Core 3.1                           | `LangVersion` 9, attribute polyfills, System.Text.Json 5.0+ | `LangVersion` 14, attribute/API polyfills, `DateOnly` backport ¹, System.Text.Json 6.0+   |
| .NET Standard 2.0 / 2.1                 | `LangVersion` 9, attribute polyfills                        | `LangVersion` 14, attribute/API polyfills, `DateOnly` backport ¹                          |
| .NET Framework 4.6.2 – 4.8.1 ²          | `LangVersion` 9, attribute polyfills                        | `LangVersion` 14, attribute/API polyfills, `DateOnly` backport ¹                          |
| .NET Core ≤ 3.0, .NET Framework ≤ 4.6.1 | Not possible (`Temporalio` doesn't support it)              | Not possible (`Temporalio` doesn't support it)                                            |

- ¹ Only if a schema uses a temporal `format` (`date`, `date-time`, `time`, or
  `duration`).
- ² .NET Framework projects must also set `Platform` to x64 or arm64, a
  `Temporalio` requirement.
- ³ Only if a schema has a `oneOf` union.

[Targeting older .NET platforms](#targeting-older-net-platforms) breaks this
down per workaround, and [Platform workarounds](#platform-workarounds) explains
how to apply each one.

Two things to keep in mind:

- **Microsoft doesn't support raising `LangVersion` above a target's default.**
  Its
  [language version documentation](https://learn.microsoft.com/en-us/dotnet/csharp/language-reference/configure-language-version)
  states that "using a C# language version newer than the version associated
  with your target TFM isn't supported". In practice it works for the features
  generated code uses, which only need the compile-time marker types described
  below.
- **Most of these targets are end of life.** .NET Core 3.1, .NET 5, .NET 6, and
  .NET 7 no longer receive security fixes
  ([support policy](https://dotnet.microsoft.com/platform/support/policy/dotnet-core)).
  Though .NET Framework 4.6.2+ is still serviced as part of Windows.

## Key problems with backwards compatibility

Generated code runs into four kinds of problem on older platforms. WIT output
hits items 1, 2, and 4; JSON Schema output hits all four.

1. **C# language features** that older targets don't allow by default.
   - Each target framework has a default C# language version. For example, .NET
     Framework 4.6.2 defaults to C# 7.3, and .NET 8 defaults to C# 12. So even
     with a more recent .NET SDK, the compiler rejects newer syntax such as
     `record` or `required` when targeting an older platform, even though the C#
     compiler being used understands it. (An older SDK, such as the .NET Core
     3.1 SDK, ships an older compiler that doesn't understand that syntax at
     all.)
   - The compiler itself comes from the .NET SDK you build with, not from the
     target framework. With a newer SDK installed (for example the .NET 8 SDK,
     building a project that targets .NET Core 3.1), you can set `LangVersion`
     in the project file to raise the C# version. Microsoft doesn't support
     this, and it can cause hard-to-diagnose compile or runtime errors when a
     feature needs runtime support the older platform lacks.
2. **Compiler marker types** that the target platform doesn't include.
   - Some newer C# features make the compiler tag the code it produces with a
     reference to a special, empty "marker" type. For example, every `init`
     property references `System.Runtime.CompilerServices.IsExternalInit`. The
     type does nothing at runtime; the compiler only needs it to exist.
   - Older platforms don't include these types, so compilation fails. You can
     fix it by declaring an empty type with the same name in your own project.
     This is called a *polyfill*.
3. **Functionality missing from the Base Class Library (BCL).**
   - The BCL is the set of standard libraries that ships with each .NET platform
     (`System.String`, `System.Char`, `System.HashCode`, and so on). Generated
     code calls some methods and types that were only added in later platforms,
     such as `char.IsAsciiDigit` (.NET 7) and `System.DateOnly` (.NET 6).
   - A missing *instance* method (`text.StartsWith('[')`) can be added with an
     ordinary extension method. A missing *static* method
     (`char.IsAsciiDigit(c)`) can only be added with a C# 14 extension block. A
     missing *type* (`DateOnly`) can't be added with extensions at all; it needs
     a package that supplies the type.
4. **System.Text.Json features** that older System.Text.Json versions lack.

### Records are not the problem by themselves

Both backends generate their data types as C# `record`s (the
[next section](#c-language-version) shows examples of each). A
[record](https://learn.microsoft.com/en-us/dotnet/csharp/language-reference/builtin-types/record)
is a purely compile-time feature. The compiler turns it into an ordinary class
with generated `Equals`, `GetHashCode`, `ToString`, and a copy constructor, and
the resulting code runs on any runtime. Records cause trouble on old platforms
only through two things the generated types use alongside them:

- **`init` accessors (both backends).** Every property in WIT and JSON Schema
  output is `{ get; init; }`. The compiler marks every `init` setter with a
  `modreq` that references
  [`System.Runtime.CompilerServices.IsExternalInit`](https://learn.microsoft.com/en-us/dotnet/api/system.runtime.compilerservices.isexternalinit).
  That type first shipped in .NET 5. Below .NET 5, compilation fails with
  `CS0518: Predefined type 'System.Runtime.CompilerServices.IsExternalInit' is not defined or imported`.
  This would hit a plain class with
  [`init`](https://learn.microsoft.com/en-us/dotnet/csharp/language-reference/keywords/init)
  properties just as much. You fix it by declaring the type yourself (a
  polyfill), as described in [Attribute polyfills](#attribute-polyfills).
- **No parameterless constructor (WIT only).** WIT types take their required
  fields as constructor parameters, so they have no parameterless constructor.
  Temporal deserializes WIT payloads through System.Text.Json, and versions
  before 5.0 can't create an object that way (see
  [System.Text.Json: parameterized constructors](#systemtextjson-parameterized-constructors-50)).
  JSON Schema types aren't affected, because their generated converters create
  objects themselves.

Generating mutable classes, with `{ get; set; }` properties and a parameterless
constructor, would avoid both problems. It would also give up immutability and
the compile-time check for missing required fields. Both problems exist only on
platforms older than .NET 5: `IsExternalInit` and System.Text.Json 5.0 ship with
.NET 5 and later. On .NET 7 and later, the platforms `nexgen` supports, neither
problem occurs, so the generator keeps records.

### C# language version

Both backends generate data types with the same guarantees: they are immutable,
compare by value, and make a missing required field a compile-time error. They
differ in *how* a caller supplies required fields, and that choice alone decides
the C# version each needs.

WIT output takes required fields as constructor parameters, and optional fields
as `init` properties:

```csharp
public record SignalWithStartWorkflowRequest
{
    public SignalWithStartWorkflowRequest(string workflow, string id, string taskQueue, string signal, string @namespace) { ... }
    public string Workflow { get; init; }
    public IReadOnlyCollection<object?>? Args { get; init; } // optional
}

var request = new SignalWithStartWorkflowRequest("wf", "id-1", "queue", "sig", "default") { Args = args };
```

JSON Schema output marks required fields with the
[`required`](https://learn.microsoft.com/en-us/dotnet/csharp/language-reference/keywords/required)
keyword, and every field is set by name in an object initializer:

```csharp
public sealed record GetRoomInput
{
    public required string RoomId { get; init; }
}

var input = new GetRoomInput { RoomId = "room-1" };
```

|                                           | WIT                                    | JSON Schema                                                          |
| ----------------------------------------- | -------------------------------------- | -------------------------------------------------------------------- |
| Immutable                                 | Yes (`init` properties)                | Yes (`init` properties)                                              |
| Structural equality                       | Yes (record)                           | Yes (record, plus content comparison of lists and byte arrays)       |
| Missing required field is a compile error | Yes (constructor parameter)            | Yes (`required` member)                                              |
| How callers construct a value             | `new T(a, b) { Optional = x }`         | `new T { A = a, B = b, Optional = x }`                               |
| Newest C# feature used                    | records, `init`, target-typed `new`    | `required` members                                                   |
| Minimum C#                                | 9                                      | 11                                                                   |
| Platform whose default C# is high enough  | .NET 5                                 | .NET 7                                                               |

The `required` style names every field at the call site, doesn't depend on
argument order, and lets a type gain a required field without changing a
constructor signature. Its cost is C# 11 instead of C# 9. `required` is the only
C# 10 or 11 feature that JSON Schema output uses; without it, the output would
compile as C# 9.

Each target framework has a default C# version: 7.3 for .NET Framework and .NET
Standard 2.0, 8.0 for .NET Core 3.x and .NET Standard 2.1, 9 for .NET 5, 10 for
.NET 6, 11 for .NET 7, and 12 for .NET 8. On a platform below the last row of
the table, the project must set `<LangVersion>` explicitly. The compiler bundled
with the installed SDK, not the target framework, decides which values are
available.

#### The language version does not set the platform floor

It is tempting to read the table as "JSON Schema needs .NET 7 because it uses
`required`". That isn't the case:

- **`required` is cheap to work around.** On an older platform it needs
  `LangVersion` 11 and three small attribute polyfills (see
  [Compiler marker types](#compiler-marker-types)). That's the same kind of
  workaround WIT output needs for `init`.
- **The .NET 7 floor comes from BCL APIs.** JSON Schema's runtime helpers call
  `char.IsAsciiDigit` and related methods, which first shipped in .NET 7. Lower
  floors are set by `DateOnly` and `HashCode.AddBytes` (.NET 6) and by
  System.Text.Json 6.0 (see [BCL APIs](#bcl-apis-json-schema-only) and
  [System.Text.Json](#systemtextjson)).
- **The two happen to coincide.** .NET 7 is both the first platform whose
  default C# is 11 and the first with `char.IsAsciiDigit`. If JSON Schema output
  switched to constructor parameters, it would still need .NET 7 without
  workarounds. If the generator instead replaced the newer BCL calls, it would
  sit roughly where WIT does, with `required` kept.

### Compiler marker types

Some language features compile to references to attribute types that the
compiler expects to find by name. Older BCLs don't ship them, but the compiler
accepts an `internal` copy declared in your own assembly:

| Type                                                                                                                                                | Needed by                                                     | In BCL from |
| --------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------- | ----------- |
| [`IsExternalInit`](https://learn.microsoft.com/en-us/dotnet/api/system.runtime.compilerservices.isexternalinit)                                     | `init` accessors (records); WIT and JSON Schema               | .NET 5      |
| [`RequiredMemberAttribute`](https://learn.microsoft.com/en-us/dotnet/api/system.runtime.compilerservices.requiredmemberattribute)                   | `required` members; JSON Schema                               | .NET 7      |
| [`CompilerFeatureRequiredAttribute`](https://learn.microsoft.com/en-us/dotnet/api/system.runtime.compilerservices.compilerfeaturerequiredattribute) | `required` members; JSON Schema                               | .NET 7      |
| [`SetsRequiredMembersAttribute`](https://learn.microsoft.com/en-us/dotnet/api/system.diagnostics.codeanalysis.setsrequiredmembersattribute)         | the record copy constructor of a type with `required` members | .NET 7      |

Without them you get `CS0518` (for `IsExternalInit`) or
`CS0656: Missing compiler required member` (for the rest). Nothing reads these
types at runtime, so declaring them yourself is safe.

### BCL APIs (JSON Schema only)

JSON Schema output always includes the runtime helper files `SpecJson.cs`,
`ValueEquality.cs`, and `Violation.cs`. When the schema uses a temporal `format`
(`date`, `date-time`, `time`, or `duration`) it also includes
`TemporalSupport.cs`. Those helpers, plus the per-model finite-number checks,
call these newer APIs:

| API used by generated code                                     | Where                                                                     | Added in                          | Missing on                                             |
| -------------------------------------------------------------- | ------------------------------------------------------------------------- | --------------------------------- | ------------------------------------------------------ |
| `char.IsAsciiDigit`, `IsAsciiLetter`, `IsAsciiLetterOrDigit`   | `SpecJson` (number and key grammar), `TemporalSupport` (temporal grammar) | .NET 7                            | .NET 6 and earlier, all .NET Framework / .NET Standard |
| `System.DateOnly`                                              | `format: date` members; `TemporalSupport` (any temporal format)           | .NET 6                            | .NET 5 and earlier, all .NET Framework / .NET Standard |
| `HashCode.AddBytes(ReadOnlySpan<byte>)`                        | `ValueEquality` (hashing `byte[]` members)                                | .NET 6                            | .NET 5 and earlier, all .NET Framework / .NET Standard |
| `double.IsFinite`                                              | `SpecJson`; per-model `number` checks                                     | .NET Core 2.1 / .NET Standard 2.1 | .NET Standard 2.0, .NET Framework                      |
| `string.StartsWith(char)`                                      | `Violation` (path joining)                                                | .NET Core 2.0 / .NET Standard 2.1 | .NET Standard 2.0, .NET Framework                      |
| `int.Parse(ReadOnlySpan<char>, NumberStyles, IFormatProvider)` | `TemporalSupport`                                                         | .NET Core 2.1 / .NET Standard 2.1 | .NET Standard 2.0, .NET Framework                      |

`HashCode` itself is in the BCL from .NET Core 2.1. On .NET Framework and .NET
Standard 2.0 it comes from the `Microsoft.Bcl.HashCode` package, which
`Temporalio` already depends on, but that package has no `AddBytes`.

On .NET Core 3.1 one more thing shows up: its nullable annotations make
`foreach (DictionaryEntry entry in map)` in `ValueEquality` raise warning
`CS8605` (unboxing a possibly null value). This is harmless, but it breaks
builds that use `TreatWarningsAsErrors`.

### System.Text.Json

The two backends use System.Text.Json in very different ways, so they need
different minimum versions.

#### System.Text.Json: parameterized constructors (5.0)

WIT output has no serialization code of its own. Temporal's default payload
converter serializes WIT models through `JsonSerializer`'s reflection-based
path. Positional records such as `Email(string Value)` have no parameterless
constructor. Deserializing them requires System.Text.Json's support for
[immutable types and parameterized constructors](https://learn.microsoft.com/en-us/dotnet/standard/serialization/system-text-json/immutability),
which arrived in System.Text.Json 5.0. Earlier versions throw at runtime:

```text
NotSupportedException: Deserialization of reference types without parameterless
constructor is not supported. Type 'Nexgen.StreamService.Stream'
```

This failure does not show up at compile time. Code that builds cleanly fails
the first time a payload is decoded.

#### System.Text.Json: custom converters (6.0)

JSON Schema output does not rely on reflection. Every model carries its own
generated `JsonConverter<T>`, containing the logic to read, validate, and write
its JSON payload, instead of letting System.Text.Json do it automatically:

```csharp
[JsonConverter(typeof(GetRoomInput.Converter))]
public sealed record GetRoomInput
{
    public required string RoomId { get; init; }

    internal sealed class Converter : JsonConverter<GetRoomInput>
    {
        public override bool HandleNull => true;
        public override GetRoomInput Read(ref Utf8JsonReader reader, ...)
        {
            var node = JsonElement.ParseValue(ref reader);
            // ...
        }
    }
}

// A oneOf union is an interface carrying the same attribute.
[JsonConverter(typeof(ChoicesValue.Converter))]
public interface ChoicesValue { /* ... */ }
```

Most of the APIs it uses (`JsonElement`, `JsonElement.ParseValue`,
`Utf8JsonWriter`) have existed since System.Text.Json 3.0. Two have not:

| Feature                                                                                                                                 | Why generated code needs it                                                                                                                                                         | Added in |
| --------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------- |
| [`JsonConverter<T>.HandleNull`](https://learn.microsoft.com/en-us/dotnet/api/system.text.json.serialization.jsonconverter-1.handlenull) | Each converter overrides it to `true` so that it sees JSON `null` and can report an `explicit null not allowed` violation, instead of the serializer silently producing `null`.     | 5.0      |
| `[JsonConverter]` on an interface                                                                                                       | `oneOf` unions are emitted as interfaces carrying the converter attribute. Before 6.0 the attribute's `AttributeUsage` excluded interfaces, so this fails to compile with `CS0592`. | 6.0      |

So JSON Schema output needs System.Text.Json 6.0 or later (5.0 suffices if no
schema has a `oneOf` union). WIT output needs 5.0 or later.

#### Which System.Text.Json each target gets

| Target                            | System.Text.Json version you get                                                                                                         |
| --------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------- |
| .NET 5 / 6 / 7 / 8+               | The in-box version matching the runtime (5.0 / 6.0 / 7.0 / 8.0+)                                                                         |
| .NET Core 3.1                     | The in-box version from 3.1 (assembly version 4.0.1.x). `Temporalio`'s `netcoreapp3.1` dependency group does **not** add a newer package |
| .NET Standard 2.0, .NET Framework | The `System.Text.Json` 9.0.x package, a transitive dependency of `Temporalio`                                                            |

To use a newer System.Text.Json on .NET Core 3.1 or .NET 5, reference the
package directly:

```xml
<ItemGroup>
  <PackageReference Include="System.Text.Json" Version="6.0.*" />
</ItemGroup>
```

Microsoft's 6.0.x packages officially support .NET Core 3.1. Newer packages also
work (verification used 9.0.4), but on .NET Core 3.1 they emit
`... doesn't support netcoreapp3.1 and has not been tested with it` build
warnings unless you set
`<SuppressTfmSupportBuildWarnings>true</SuppressTfmSupportBuildWarnings>`.

### .NET Framework: the Temporal native library

This one has nothing to do with `nexgen`, but you will hit it. `Temporalio`
loads a native Rust library. For .NET Framework, its build targets pick the
library by the project's `Platform`, which defaults to `AnyCPU`, and no `AnyCPU`
library exists. The build fails with
`MSB3030: Could not copy the file ".../runtimes/win-AnyCPU/native/temporalio_sdk_core_c_bridge.dll"`.
Set the platform explicitly, as the
[Temporal .NET SDK README](https://github.com/temporalio/sdk-dotnet#built-in-native-shared-library)
describes:

```xml
<PropertyGroup>
  <Platform>x64</Platform> <!-- or arm64 -->
</PropertyGroup>
```

Setting only `PlatformTarget` is not enough: the targets read `Platform`.

## Targeting older .NET platforms

If you want `nexgen`'s generated code to be part of an assembly that targets a
platform older than .NET 7, you need to apply some of the
[platform workarounds](#platform-workarounds) below. Which ones depends on the
input format you generate from and how far back you want to go. The tables in
this section list what each platform needs; each column links to the workaround
that explains why and how.

Generated code can only target platforms that `Temporalio` supports. .NET Core
3.0 and earlier, and .NET Framework 4.6.1 and earlier, can't be targeted at all.

Two notes apply to both tables:

- .NET Framework projects must also set `Platform` to x64 or arm64 (see
  [Other settings](#other-settings)).
- A .NET Standard library has no runtime of its own. The application that uses
  it must also meet the requirements of the platform it runs on. For example, a
  .NET Core 3.1 application still needs the System.Text.Json package.

### JSON Schema inputs

Code generated from JSON Schema works unchanged on .NET 7 and later. Older
platforms need up to five workarounds:

| I want to target             | [`LangVersion`](#raising-the-c-language-version-langversion) | [Attribute polyfills](#attribute-polyfills) | [API polyfills](#api-polyfills) | [`DateOnly` backport](#dateonly-backport) | [System.Text.Json package](#newer-systemtextjson) |
| ---------------------------- | ------------------------------------------------------------ | ------------------------------------------- | ------------------------------- | ----------------------------------------- | ------------------------------------------------- |
| .NET 7 and later             | –                                                            | –                                           | –                               | –                                         | –                                                 |
| .NET 6                       | 14                                                           | `required`                                  | `char`                          | –                                         | –                                                 |
| .NET 5                       | 14                                                           | `required`                                  | `char`, `HashCode`              | If temporal formats                       | 6.0 or later, if `oneOf` unions                   |
| .NET Core 3.1                | 14                                                           | `required`, `IsExternalInit`                | `char`, `HashCode`              | If temporal formats                       | 6.0 or later                                      |
| .NET Standard 2.1            | 14                                                           | `required`, `IsExternalInit`                | `char`, `HashCode`              | If temporal formats                       | –                                                 |
| .NET Standard 2.0            | 14                                                           | `required`, `IsExternalInit`                | all                             | If temporal formats                       | –                                                 |
| .NET Framework 4.6.2 – 4.8.1 | 14                                                           | `required`, `IsExternalInit`                | all                             | If temporal formats                       | –                                                 |

- "–" means nothing to do.
- "If temporal formats" means only if one of your schemas uses `format: date`,
  `date-time`, `time`, or `duration`.

The polyfill files use `#if` guards, so they only add what the target platform
is missing. For any target older than .NET 7, the simplest approach is to add
both polyfill files and set `LangVersion` 14, then add the two packages where
the table calls for them. See the
[.NET Framework example](#example-json-schema-output-on-net-framework-462).

### WIT inputs

Code generated from WIT works unchanged on .NET 5 and later. The rest of the
platforms `Temporalio` supports (.NET Core 3.1, .NET Standard, and .NET
Framework) need up to three workarounds:

| I want to target             | [`LangVersion`](#raising-the-c-language-version-langversion) | [Attribute polyfills](#attribute-polyfills) | [System.Text.Json package](#newer-systemtextjson) |
| ---------------------------- | ------------------------------------------------------------ | ------------------------------------------- | ------------------------------------------------- |
| .NET 5 and later             | –                                                            | –                                           | –                                                 |
| .NET Core 3.1                | 9                                                            | `IsExternalInit`                            | 5.0 or later                                      |
| .NET Standard 2.0 / 2.1      | 9                                                            | `IsExternalInit`                            | –                                                 |
| .NET Framework 4.6.2 – 4.8.1 | 9                                                            | `IsExternalInit`                            | –                                                 |

WIT output never needs the API polyfills or the `DateOnly` backport. On .NET
Core 3.1, don't skip the System.Text.Json package: without it the project still
builds, but fails at runtime (see
[Newer System.Text.Json](#newer-systemtextjson)). See the
[.NET Core 3.1 example](#example-wit-output-on-net-core-31).

## Platform workarounds

### Raising the C# language version (`LangVersion`)

**Why:** each target platform has a default C# language version, and the
compiler rejects newer syntax (see [C# language version](#c-language-version)).
.NET Core 3.1 and .NET Standard 2.1 default to C# 8, .NET Standard 2.0 and .NET
Framework to C# 7.3, .NET 5 to C# 9, and .NET 6 to C# 10.

- **WIT output** uses records, `init` properties, and target-typed `new`, which
  are C# 9 features.
- **JSON Schema output** uses `required` members, a C# 11 feature. The
  [API polyfills](#api-polyfills) it also needs add static methods to `char` and
  `double`, which requires C# 14
  [extension blocks](https://learn.microsoft.com/en-us/dotnet/csharp/language-reference/keywords/extension).

**When:**

| Input format | Set `LangVersion` to | On targets older than | Build with                                     |
| ------------ | -------------------- | --------------------- | ---------------------------------------------- |
| WIT          | 9                    | .NET 5                | any SDK from .NET 5 on, such as the .NET 8 SDK |
| JSON Schema  | 14                   | .NET 7                | the .NET 10 SDK or later                       |

**How:** set the language version in your project file. The SDK is only the
build tool; your application still runs on the platform you target.

```xml
<PropertyGroup>
  <LangVersion>14</LangVersion> <!-- 9 is enough for WIT output -->
</PropertyGroup>
```

Microsoft doesn't support setting `LangVersion` above a target's default. The
features generated code and the polyfills use need no new runtime support, which
is why this works in practice.

### Attribute polyfills

**Why:** some C# features make the compiler reference marker types that older
platforms don't include, so compilation fails with `CS0518` or `CS0656` (see
[Compiler marker types](#compiler-marker-types)). Raising `LangVersion` alone
doesn't fix this; the compiler also needs the types to exist.

- `init` properties reference `System.Runtime.CompilerServices.IsExternalInit`,
  a marker type
  [Microsoft describes](https://learn.microsoft.com/en-us/dotnet/api/system.runtime.compilerservices.isexternalinit)
  as "reserved to be used by the compiler for tracking metadata". It has no
  members and nothing uses it at runtime, but it only ships with .NET 5 and
  later.
- `required` members reference `RequiredMemberAttribute`,
  `CompilerFeatureRequiredAttribute`, and `SetsRequiredMembersAttribute`, which
  ship with .NET 7 and later.

**When:**

| Input format | Needs                                          | On targets older than                                     |
| ------------ | ---------------------------------------------- | --------------------------------------------------------- |
| WIT          | `IsExternalInit`                               | .NET 5                                                    |
| JSON Schema  | `IsExternalInit` and the `required` attributes | .NET 5 (`IsExternalInit`), .NET 7 (`required` attributes) |

**Easiest:**

- **WIT output:** reference the
  [`IsExternalInit`](https://www.nuget.org/packages/IsExternalInit/) package
  ([source](https://github.com/manuelroemer/IsExternalInit)). It adds the type
  to your project as source code, only on targets that need it, without adding a
  runtime dependency:

  ```xml
  <PackageReference Include="IsExternalInit" Version="1.0.3">
    <PrivateAssets>all</PrivateAssets>
    <IncludeAssets>runtime; build; native; contentfiles; analyzers</IncludeAssets>
  </PackageReference>
  ```

- **JSON Schema output:** reference
  [PolySharp](https://github.com/Sergio0694/PolySharp), a source generator that
  adds whichever of the four types your target is missing:

  ```xml
  <PackageReference Include="PolySharp" Version="1.16.0">
    <PrivateAssets>all</PrivateAssets>
    <IncludeAssets>runtime; build; native; contentfiles; analyzers</IncludeAssets>
  </PackageReference>
  ```

**By hand:** add this file, `AttributePolyfills.cs`, to every project that
compiles generated code. The types are `internal`, so each assembly needs its
own copy. WIT output only needs the first block, but the whole file is safe to
use with either input format.

```csharp
// Needed by WIT and JSON Schema output below .NET 5.
#if !NET5_0_OR_GREATER
namespace System.Runtime.CompilerServices
{
    internal static class IsExternalInit { }
}
#endif

// Needed by JSON Schema output below .NET 7.
#if !NET7_0_OR_GREATER
namespace System.Runtime.CompilerServices
{
    [System.AttributeUsage(
        System.AttributeTargets.Class | System.AttributeTargets.Struct |
        System.AttributeTargets.Field | System.AttributeTargets.Property,
        AllowMultiple = false, Inherited = false)]
    internal sealed class RequiredMemberAttribute : System.Attribute { }

    [System.AttributeUsage(System.AttributeTargets.All, AllowMultiple = true, Inherited = false)]
    internal sealed class CompilerFeatureRequiredAttribute : System.Attribute
    {
        public CompilerFeatureRequiredAttribute(string featureName) { FeatureName = featureName; }
        public string FeatureName { get; }
        public bool IsOptional { get; init; }
    }
}

namespace System.Diagnostics.CodeAnalysis
{
    [System.AttributeUsage(System.AttributeTargets.Constructor, AllowMultiple = false, Inherited = false)]
    internal sealed class SetsRequiredMembersAttribute : System.Attribute { }
}
#endif
```

### API polyfills

*JSON Schema output only.*

**Why:** the runtime helper files that every JSON Schema output includes call
standard library methods that were added after the target platform shipped (see
[BCL APIs](#bcl-apis-json-schema-only)). A missing instance method can be added
with an ordinary extension method. A missing static method, such as
`char.IsAsciiDigit`, can only be added with a C# 14 extension block.

**When:**

- Below .NET 7: `char.IsAsciiDigit`, `char.IsAsciiLetter`, and
  `char.IsAsciiLetterOrDigit`.
- Below .NET 6, additionally: `HashCode.AddBytes`.
- .NET Standard 2.0 and .NET Framework, additionally: `double.IsFinite`,
  `int.Parse(ReadOnlySpan<char>, ...)`, and `string.StartsWith(char)`.

**How:** no package provides these, so add this file, `NexgenApiPolyfills.cs`,
to every project that compiles generated code. Its `#if` guards include only the
polyfills your target needs, so the same file works for every platform. It
requires `LangVersion` 14.

```csharp
#nullable enable
using System;
using System.Globalization;

#if !NET7_0_OR_GREATER
internal static class NexgenCharPolyfills
{
    extension(char)
    {
        public static bool IsAsciiDigit(char c) => c >= '0' && c <= '9';
        public static bool IsAsciiLetter(char c) => (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z');
        public static bool IsAsciiLetterOrDigit(char c) => char.IsAsciiLetter(c) || char.IsAsciiDigit(c);
    }
}
#endif

#if !NET6_0_OR_GREATER
internal static class NexgenHashCodePolyfills
{
    public static void AddBytes(ref this HashCode hash, ReadOnlySpan<byte> bytes)
    {
        foreach (var b in bytes)
        {
            hash.Add(b);
        }
    }
}
#endif

#if !NETCOREAPP2_1_OR_GREATER && !NETSTANDARD2_1_OR_GREATER
internal static class NexgenNetStandard20Polyfills
{
    extension(double)
    {
        public static bool IsFinite(double d) => !double.IsNaN(d) && !double.IsInfinity(d);
    }

    extension(int)
    {
        public static int Parse(ReadOnlySpan<char> s, NumberStyles style, IFormatProvider? provider) =>
            int.Parse(s.ToString(), style, provider);
    }

    public static bool StartsWith(this string s, char c) => s.Length > 0 && s[0] == c;
}
#endif
```

The classes live in the global namespace, so they are in scope everywhere, and
generated calls such as `char.IsAsciiDigit(c)` find them. On a platform that
already has a method, the `#if` guard leaves the polyfill out and the built-in
method is used.

### `DateOnly` backport

*JSON Schema output only.*

**Why:** generated code represents `format: date` values as `System.DateOnly`,
which first shipped in .NET 6. The helper file `TemporalSupport.cs`, which is
emitted whenever a schema uses any temporal format, also references `DateOnly`,
even if the schema only uses `date-time`.

**When:** below .NET 6, and only if one of your schemas uses `format: date`,
`date-time`, `time`, or `duration`. Otherwise, skip this step.

**Easiest:** reference the
[`Portable.System.DateTimeOnly`](https://www.nuget.org/packages/Portable.System.DateTimeOnly)
package. It defines `System.DateOnly` for .NET Standard 2.0/2.1 and .NET
Framework 4.6.2+, and on .NET 6+ it forwards to the built-in type. You don't
need its companion `Portable.System.DateTimeOnly.Json` package, because the
generated converters do their own date parsing and formatting.

```xml
<ItemGroup Condition="!$([MSBuild]::IsTargetFrameworkCompatible('$(TargetFramework)', 'net6.0'))">
  <PackageReference Include="Portable.System.DateTimeOnly" Version="9.0.2" />
</ItemGroup>
```

**By hand:** not recommended. You could declare your own `System.DateOnly`
struct with those members, but it would conflict with any other library that
brings in the backport.

### Newer System.Text.Json

**Why:** both input formats need a newer System.Text.Json than some platforms
include, for different reasons (see [System.Text.Json](#systemtextjson)):

- **WIT output** has no JSON code of its own; Temporal serializes WIT types with
  System.Text.Json's automatic, reflection-based path. WIT types take their
  required fields as constructor parameters, so they have no parameterless
  constructor, and System.Text.Json can only create such objects from version
  5.0 on.
- **JSON Schema output** has generated JSON converters that override
  `HandleNull` (added in 5.0) and put `[JsonConverter]` on `oneOf` union
  interfaces (allowed from 6.0).

**When:**

| Input format | Needs        | On                     |
| ------------ | ------------ | ---------------------- |
| WIT          | 5.0 or later | .NET Core 3.1          |
| JSON Schema  | 6.0 or later | .NET Core 3.1, .NET 5  |

The built-in System.Text.Json on those platforms is too old, and `Temporalio`
doesn't add a newer one there. .NET 6 and later have a new enough version built
in, and on .NET Standard and .NET Framework `Temporalio` already brings in
System.Text.Json 9.0.x.

**How:** reference the package directly. A 6.0.x package satisfies both input
formats:

```xml
<ItemGroup>
  <PackageReference Include="System.Text.Json" Version="6.0.*" />
</ItemGroup>
```

Microsoft's 6.0.x packages officially support .NET Core 3.1. Newer packages also
work (verification used 9.0.4), but on .NET Core 3.1 they emit
`... doesn't support netcoreapp3.1` build warnings unless you set
`<SuppressTfmSupportBuildWarnings>true</SuppressTfmSupportBuildWarnings>`.

> **For WIT output, the compiler can't check this for you.** Without the
> package, a .NET Core 3.1 project still builds. It then fails at runtime, the
> first time Temporal decodes a payload, with:
>
> ```text
> NotSupportedException: Deserialization of reference types without
> parameterless constructor is not supported.
> ```
>
> For JSON Schema output, a missing package is a compile error instead: `CS0115`
> (`HandleNull`) on .NET Core 3.1, and `CS0592` on .NET 5 when a schema has a
> `oneOf` union. On .NET 5 without unions, the in-box 5.0 is enough.

### Other settings

- **.NET Framework (both input formats):** set `<Platform>x64</Platform>` (or
  `arm64`). This is a `Temporalio` requirement, not a `nexgen` one (see
  [.NET Framework: the Temporal native library](#net-framework-the-temporal-native-library)).
- **JSON Schema output on .NET Core 3.1 with `TreatWarningsAsErrors`:** add
  `<NoWarn>$(NoWarn);CS8605</NoWarn>`. Generated code raises a harmless nullable
  warning there because of how .NET Core 3.1 annotates its libraries.

### Example: JSON Schema output on .NET Framework 4.6.2

With `AttributePolyfills.cs` and `NexgenApiPolyfills.cs` somewhere in the
project folder:

```xml
<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <TargetFramework>net462</TargetFramework>
    <LangVersion>14</LangVersion>
    <Platform>x64</Platform>
    <Nullable>enable</Nullable>
  </PropertyGroup>
  <ItemGroup>
    <PackageReference Include="Temporalio" Version="1.20.0" />
    <!-- Only needed if a schema uses a temporal format. -->
    <PackageReference Include="Portable.System.DateTimeOnly" Version="9.0.2" />
  </ItemGroup>
</Project>
```

The SDK compiles every `.cs` file under the project folder by default, so the
polyfill files need no `Compile` items. Add them explicitly only if they live
outside the project folder.

### Example: WIT output on .NET Core 3.1

```xml
<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <TargetFramework>netcoreapp3.1</TargetFramework>
    <LangVersion>9</LangVersion>
    <Nullable>enable</Nullable>
  </PropertyGroup>
  <ItemGroup>
    <PackageReference Include="Temporalio" Version="1.20.0" />
    <PackageReference Include="System.Text.Json" Version="6.0.*" />
    <PackageReference Include="IsExternalInit" Version="1.0.3">
      <PrivateAssets>all</PrivateAssets>
      <IncludeAssets>runtime; build; native; contentfiles; analyzers</IncludeAssets>
    </PackageReference>
  </ItemGroup>
</Project>
```

For .NET Framework, drop the System.Text.Json reference, change
`TargetFramework` to `net462` (or later), and add `<Platform>x64</Platform>`.
