#version 450

layout(location = 0) in  ivec2 in_pos;
layout(location = 1) in  ivec2 in_uv;
layout(location = 2) in   vec4 in_color;

layout(location = 0) out  vec3 out_color;
layout(location = 1) out  vec2 out_uv;

layout(push_constant) uniform _ {
    vec2 scale; vec2 offset;
} push;

void main() {
    gl_Position = vec4(push.scale.xy*(vec2(in_pos)-push.offset), 0.0, 1.0);
    out_color   = in_color.xyz;
    out_uv      = in_uv;
}
