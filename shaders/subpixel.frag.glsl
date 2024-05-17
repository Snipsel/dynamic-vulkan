#version 450
layout(location = 0) in  vec3 in_color;
layout(location = 1) in  vec2 in_uv;

layout(location = 0) out vec4 out_color;
layout(location = 1) out vec4 out_alpha;

layout(binding = 0) uniform sampler2D font_texture;

void main(){
    vec3 alpha = texture(font_texture, in_uv).xyz;
    out_color = vec4(in_color, 1.0);
    out_alpha = vec4(alpha,    1.0);
}
