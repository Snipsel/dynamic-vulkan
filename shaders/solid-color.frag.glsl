#version 450
layout(location = 0) in  vec3 in_color;
layout(location = 1) in  vec2 in_uv;

layout(location = 0) out vec4 out_color;

layout(binding = 0) uniform sampler2D font_texture;

void main(){
    float alpha = texture(font_texture, in_uv).x;
    // vec3 color = mix(in_color, vec3(0,0,0), alpha);
    // out_color = vec4(color, 1.0);
    out_color = vec4(in_color, alpha);
}
