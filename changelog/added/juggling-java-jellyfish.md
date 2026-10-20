Add Java WIT models, protobuf transfer conversion, native operation wrappers,
resources, and generated system Nexus interceptor and serialization context hooks.
Java native APIs can expose generated operations through inheritable SDK facades using the operations-class directive.
Java protobuf bindings use typed field access and emit only the shared runtime helpers they need.
Java protobuf models preserve embedded payloads, batches, metadata, and failures as serialized protobuf values; transfer conversion needs no user converter, scope, or payload type hints.
