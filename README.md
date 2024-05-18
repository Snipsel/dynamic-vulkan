# Dynamic Renderer
An experiment in vulkan dynamic rendering and text rendering.

This project consists of a [main application](src), which uses the [renderer](renderer) and the [text-engine](text-engine), as well as a small shared [common](common) package.

# Dependencies

## [application](src) and [renderer](renderer)
- [ash](https://github.com/ash-rs/ash) - vulkan bindings

## [text-engine](text-engine)
- [harfbuzz-sys](https://github.com/servo/rust-harfbuzz/tree/main/harfbuzz-sys) - text shaping
- [freetype](https://github.com/PistonDevelopers/freetype-rs) - text rasterization
- [icu](https://github.com/unicode-org/icu4x) - text handling

# building/running
1) `download_fonts.sh`
2) `cargo run`
