<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
<!-- Copyright (c) 2026 dravr.ai -->

# Documentation

Documentation has moved to [`book/`](../book/src/).

## Quick Links

- [Introduction](../book/src/introduction.md)
- [Getting Started](../book/src/getting-started.md)
- [Architecture](../book/src/architecture.md)
- [Tools Reference](../book/src/tools-reference.md)

## Building Documentation

```bash
# Install mdbook
cargo install mdbook

# Build and serve locally
cd book
mdbook serve --open
```

## API Specification

The OpenAPI specification is located in this directory: [openapi.yaml](openapi.yaml)
