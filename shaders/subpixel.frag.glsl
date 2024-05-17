#version 450
layout(location = 0) in  vec3 in_color;
layout(location = 1) in  vec2 in_uv;

layout(location = 0) out vec4 out_color;

layout(binding = 0) uniform sampler2D font_texture;

void main(){
    vec3 bg_color = vec3(0,0,0);
    vec3 alpha = texture(font_texture, in_uv).xyz;
    float hacky_blend = 1.0;
    if(alpha.xyz==vec3(0,0,0)){
        hacky_blend = 0;
    }
    out_color = vec4(mix(bg_color, in_color, alpha), hacky_blend);
}
