# @pierre/scene-types

TypeScript types for the chart **Scene**, generated from the Rust definitions in
[`dravr-photograveur`](https://github.com/dravr-ai/dravr-photograveur) via `ts-rs`.

## Do not edit `src/*.ts`

Every file except this README is generated. The Rust types are the source of truth, because the
Scene is a wire contract between the server that resolves it and the two clients that draw it —
hand-writing the TypeScript half would let them drift the first time a node kind gained a field.

To change a type, change it in photograveur, regenerate, and re-vendor:

```bash
cd ../dravr-photograveur
./scripts/generate-ts-types.sh          # regenerate from the Rust types
cp bindings/*.ts ../dravr-platform/packages/scene-types/src/
```

photograveur's own `./scripts/generate-ts-types.sh --check` fails if its committed bindings are
stale, so the generated half cannot silently fall behind the Rust half there. This package is a
vendored copy pinned to whatever rev `crates/pierre-server/Cargo.toml` depends on.

## What a Scene is

A flat list of positioned primitives in a fixed viewBox. Consuming it is a switch over
`SceneNode["node"]` — five cases — emitting one SVG element each. There is no geometry on this side:
no scales, no ticks, no date formatting, no chart library.

Two properties make that work:

- **Text is anchored, not measured.** `Text` nodes carry `anchor` and `baseline` matching SVG's
  `text-anchor` / `dominant-baseline`, so the renderer places text with the platform's own text
  engine and nothing needs font metrics.
- **Colour is a token, not a value.** A node names `"activity"`; the renderer resolves it in the
  current theme. One Scene is therefore correct in both light and dark.

## Where they come from on the wire

`MessageResponse.scene_blocks` is a JSON-encoded `RenderBlock[]`, index-aligned with the `⟦viz:N⟧`
markers in the message content. The server resolves it on every read from the spec stored on the
message row, so improving the geometry engine improves charts already in conversation history.
