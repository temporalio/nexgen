Python resource handles are plain classes, so the payload converter refuses them. Pass the identity fields, such as the stream sample's `StreamRef`, and build the handle again with the factory.
