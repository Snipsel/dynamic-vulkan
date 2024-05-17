use freetype as ft;
use harfbuzz_sys as hb;
use hb::{freetype::hb_ft_font_create_referenced, hb_face_create, hb_font_get_face, hb_font_set_variations, hb_language_t};
use std::collections::HashMap;
use ash::vk;
use crate::common::{Color,Vertex,vec2,div_round,gen_quad};

// freetype integration of harfbuzz_sys 0.6.1 is missing these bindings
#[link(name="harfbuzz")]
extern{
    //fn hb_ft_font_get_face(hb_font: *mut hb::hb_font_t) -> ft::ffi::FT_Face;
    fn hb_ft_font_set_funcs(hb_font: *mut hb::hb_font_t);
    fn hb_ft_font_set_load_flags(hb_font: *mut hb::hb_font_t, load_flags: i32);
    fn hb_ft_face_create(ft_face : ft::ffi::FT_Face, destroy : hb::hb_destroy_func_t) -> *mut hb::hb_face_t;
    fn hb_ft_font_changed(font : *mut hb::hb_font_t);
}

fn gen_buffer_image_copy( buffer_offset: u64,
                          pitch : u32,
                          width : u32,
                          height: u32,
                          u: i32,
                          v: i32) -> vk::BufferImageCopy {
    vk::BufferImageCopy{
        buffer_offset,
        buffer_row_length: pitch,
        buffer_image_height: 0,
        image_offset: vk::Offset3D{x:u, y:v, z:0},
        image_extent: vk::Extent3D{width, height, depth: 1},
        image_subresource: vk::ImageSubresourceLayers{
            layer_count: 1,
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_array_layer: 0,
            mip_level: 0
        }
    }
}

#[derive(Clone)]
pub struct Style<'a>{
    pub font_idx: u32,
    pub size:     u32,
    pub weight:   u32,
    pub color:    Color,
    pub autohint: bool,
    pub subpixel: i32,
    pub features: &'a[hb::hb_feature_t],
}


#[derive(Copy,Clone)]
struct GlyphCacheEntry{
    u: u16,
    v: u16,
    left: i16,
    top:  i16,
    width: u16,
    height: u16,
}

#[derive(Copy,Clone,Eq,Hash,PartialEq)]
struct GlyphCacheKey{
    font_idx  : u32,
    glyph_idx : u32,
    font_size : u32,
    subpixel  : u32,
    autohint  : bool,
}

struct GlyphCache{
    map: HashMap<GlyphCacheKey,GlyphCacheEntry>,
    tex_size:  u16,
    current_x: u16,
    current_y: u16,
    max_y:     u16,
}
impl GlyphCache {
    fn new(tex_size:u16) -> Self { Self { map: HashMap::new(), current_x:0, current_y:0, tex_size, max_y:0 } }
    fn get(&self, key: &GlyphCacheKey) -> Option<GlyphCacheEntry> {
        self.map.get(key).copied()
    }
    fn insert(&mut self, key:GlyphCacheKey, width: u16, height:u16, left: i16, top: i16) -> (u16,u16) {
        if self.current_x+width >= self.tex_size {
            self.current_x = 0;
            self.current_y = self.max_y;
        }
        let ret = (self.current_x, self.current_y);
        self.map.insert(key, GlyphCacheEntry{
            u: ret.0, v: ret.1,
            width, height, left, top
        });
        self.current_x += width;
        self.max_y = self.max_y.max(self.current_y+height);
        ret
    }
}

#[derive(Default)]
pub struct Text{
    pub quads          : Vec<[Vertex;4]>,
    pub buffer_updates : Vec<vk::BufferImageCopy>,
    pub pixels         : Vec<u8>,
}

pub struct TextEngine{
    freetype_lib: ft::Library,
    pub glyph_cache: GlyphCache,
    buffer:      *mut hb::hb_buffer_t,
    ft_faces:    Vec<ft::Face>,
    hb_fonts:    Vec<*mut hb::hb_font_t>,
}

impl TextEngine {
    pub fn new(glyph_texture_size:u16, fontfiles: &[&str]) -> Self {
        let freetype_lib = ft::Library::init().expect("failed to initialize freetype");
        let mut hb_fonts = vec![];
        let mut ft_faces = vec![
            freetype_lib.new_face("./fonts/source-sans/upright.ttf", 0).expect("could not find font"),
            freetype_lib.new_face("./fonts/source-sans/italic.ttf",  0).expect("could not find font"),
            freetype_lib.new_face("./fonts/crimson-pro/upright.ttf", 0).expect("could not find font"),
            freetype_lib.new_face("./fonts/crimson-pro/italic.ttf",  0).expect("could not find font"),
        ];
        for ft_face in &mut ft_faces {
            let hb_font = unsafe{hb::freetype::hb_ft_font_create_referenced(ft_face.raw_mut())};
            hb_fonts.push(hb_font);
        }
        TextEngine{
            freetype_lib,
            buffer: unsafe{hb::hb_buffer_create()},
            ft_faces, hb_fonts,
            glyph_cache: GlyphCache::new(glyph_texture_size),
        }
    }

    pub fn new_locale(lang: &str, script: hb::hb_script_t, direction: hb::hb_direction_t) -> hb::hb_segment_properties_t {
        // zero initialize struct (due to reserved fields in the c-struct)
        let mut ret = unsafe{core::mem::MaybeUninit::<hb::hb_segment_properties_t>::zeroed().assume_init()};
        ret.language = unsafe{hb::hb_language_from_string(lang.as_ptr() as *const i8, lang.len() as i32)};
        ret.direction = direction;
        ret.script = script;
        return ret;
    }


    // Text rendering can fundamentally not be cleanly separated into parts. Everything affects
    // everything else. This means the easiest thing to do is have a monolithic function that does
    // everything. It's better to have a monolithic function as API that can hide language
    // complexities, than have a complex API.
    //
    // next features: subpixel positioning and line-breaking
    pub fn render_line_of_text(&mut self,
            ret:         &mut Text,
            locale:      &hb::hb_segment_properties_t,
            style:       &Style,
            start_position: vec2<i32>,
            text:        &str){
        use hb::*;
        assert!(style.subpixel>=1);
        assert!(style.subpixel<=64);

        let hb_font =  self.hb_fonts[style.font_idx as usize];
        let mut ft_face = &mut self.ft_faces[style.font_idx as usize];
        ft_face.set_char_size(0, (style.size as isize)*64, 0, 0);


        // TODO: assert that exactly 1 variable axis exists, and that it corresponds to font-weight
        let mut amaster : *mut ft::ffi::FT_MM_Var = core::ptr::null_mut();
        let var = (style.weight as i64) <<16;
        unsafe{ft::ffi::FT_Set_Var_Design_Coordinates(ft_face.raw_mut(), 1, &var)};

        unsafe{hb_ft_font_changed(hb_font)};
        let load_flags = if style.autohint { ft::face::LoadFlag::FORCE_AUTOHINT } else { ft::face::LoadFlag::NO_AUTOHINT };
        unsafe{hb_ft_font_set_load_flags(hb_font, load_flags.bits())};

        unsafe{
            hb_buffer_reset(self.buffer);
            hb_buffer_add_utf8(self.buffer, text.as_ptr() as *const i8, text.len() as i32, 0, -1);
            hb_buffer_set_segment_properties(self.buffer, locale);
            hb_shape(hb_font, self.buffer, if style.features.len()==0 {core::ptr::null()} else {style.features.as_ptr()}, style.features.len() as u32);
        };

        let mut glyph_info_count = 0;
        let glyph_info_ptr = unsafe{hb_buffer_get_glyph_infos(self.buffer, &mut glyph_info_count)};
        let glyph_infos = unsafe{core::slice::from_raw_parts_mut(glyph_info_ptr, glyph_info_count as usize)};

        let mut glyph_pos_count = 0;
        let glyph_pos_ptr = unsafe{hb_buffer_get_glyph_positions(self.buffer, &mut glyph_pos_count)};
        let glyph_positons = unsafe{core::slice::from_raw_parts_mut(glyph_pos_ptr, glyph_pos_count as usize)};

        assert_eq!(glyph_info_count, glyph_pos_count);

        let mut cursor = start_position;
        for (info,pos) in std::iter::zip(glyph_infos, glyph_positons) {
            let id = info.codepoint; // actually glyph index, not codepoint
            let x = div_round((cursor.0 + pos.x_offset)*style.subpixel, 64);
            let y = div_round((cursor.1 + pos.y_offset), 64);
            let x_frac = x%style.subpixel;
            let x = x/style.subpixel;

            let frac64 = (x_frac*64/style.subpixel) as u32;
            //println!("{x:4}+{x_frac:2}/{:2} = {frac64:2}/64", style.subpixel);

            if let Some(entry) = self.glyph_cache.get(&GlyphCacheKey{font_idx:style.font_idx, glyph_idx:id, font_size:style.size, autohint:style.autohint, subpixel:frac64}) {
                if !(entry.width<=0 || entry.height<=0) { // invisible character, ignore for rendering
                    ret.quads.push(
                        gen_quad(x as i16 + entry.left,
                                 y as i16 - entry.top,
                                 entry.width  as i16, 
                                 entry.height as i16,
                                 entry.u, entry.v,
                                 style.color));
                }
            }else{
                ft_face.load_glyph(id, load_flags);
                let subpixel_offset = Some(ft::Vector{x:frac64 as i64, y:0});
                let glyph  = ft_face.glyph().get_glyph().unwrap().to_bitmap(ft::render_mode::RenderMode::Lcd, subpixel_offset).unwrap();
                let bitmap = glyph.bitmap();
                let width_sub  = bitmap.width();
                let width = width_sub/3;
                let height = bitmap.rows();
                let pitch  = bitmap.pitch();
                let left   = glyph.left();
                let top    = glyph.top();
                if !(width<=0 || height<=0) { 
                    assert_eq!(ret.pixels.len()%4, 0);
                    let buffer_offset = ret.pixels.len() as u64;
                    let uv = self.glyph_cache.insert( GlyphCacheKey{ font_idx:style.font_idx, glyph_idx:id, font_size:style.size, autohint:style.autohint, subpixel:frac64}, 
                                                          width as u16, height as u16, left as i16, top as i16);
                    // convert to tightly-packed rgba
                    let bitmap_buffer = bitmap.buffer();
                    let mut pixel_counter = 0;
                    for h in 0..height {
                        for w in 0..width_sub {
                            ret.pixels.push(bitmap_buffer[(h*pitch + w) as usize]);
                            if pixel_counter%3==2 { 
                                ret.pixels.push(0xFF);
                            }
                            pixel_counter += 1;
                        }
                    }
                    
                    ret.quads.push(
                        gen_quad((x+left) as i16,
                                 (y-top)  as i16,
                                 width as i16, height as i16,
                                 uv.0, uv.1,
                                 style.color));
                    ret.buffer_updates.push(
                        gen_buffer_image_copy(
                            buffer_offset,
                            0 as u32, width as u32, height as u32,
                            uv.0 as i32,
                            uv.1 as i32));
                }
            }

            cursor.0 += pos.x_advance;
            cursor.1 += pos.y_advance;
        }
    }
}

