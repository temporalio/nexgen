Fix .NET protobuf-backed model conversion for list elements and map values, populating protobuf collections instead of assigning incompatible model collections.
Avoid collisions between Go protobuf import aliases and generated runtime imports.
Fix protobuf collection presence metadata and Python nested-message collection encoding, avoiding invalid presence checks and shadowing of the enclosing model.
