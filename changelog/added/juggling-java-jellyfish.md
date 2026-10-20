Add Java WIT models, protobuf transfer conversion, native operation wrappers,
resources, and generated system Nexus interceptor and serialization context hooks.
Java native APIs can expose generated operations through inheritable SDK facades using the operations-class directive.
Java record-input operations take required fields as parameters and optional fields through generated options builders, constructing transfer requests internally.
Java function call-extractor hooks generate typed workflow/signal callback overloads alongside name-based operations.
Java protobuf bindings use typed field access and emit only the shared runtime helpers they need.
Generated Java transfer converters use SDK-owned converter scoping through the support package, without exposing scope management on the public workflow API.
