#version 450
layout(location = 0) out vec3 out_color;

layout(binding = 0) readonly buffer _ubo {
    vec2 pos[];
};

layout(push_constant) uniform _push {
    vec2 scale; vec2 offset;
} push;

vec2 positions[3] = vec2[](
    vec2( 0, 0 ),
    vec2( 100, 100), 
    vec2( 100, 0)
);

vec3 colors[3] = vec3[](
    vec3(1.0,  0.0,  0.0),
    vec3(0.0,  1.0,  0.0),
    vec3(0.0,  0.0,  1.0)
);

void main() {
    gl_Position = vec4(push.scale.xy*(pos[gl_VertexIndex]-push.offset), 0.0, 1.0);
    out_color   = colors[gl_VertexIndex % 3];
}
