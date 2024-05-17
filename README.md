# Dynamic Renderer
An experiment in vulkan dynamic rendering and text rendering.

This project consists of a [main application](src), which uses the [renderer](renderer) and the [text-engine](text-engine), as well as a small shared [common](common) package to render text.

This project uses [harfbuzz-sys](https://github.com/servo/rust-harfbuzz/tree/main/harfbuzz-sys) and [freetype](https://github.com/PistonDevelopers/freetype-rs) for text shaping and rasterization, and [ash](https://github.com/ash-rs/ash) for vulkan bindings.

# building/running
1) `download_fonts.sh`
2) `cargo run`
