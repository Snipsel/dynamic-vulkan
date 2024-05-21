#![feature(array_windows, iter_array_chunks, iter_map_windows,slice_split_once,const_mut_refs,const_trait_impl)]
#![allow(unused)]
use freetype as ft;
use harfbuzz_sys as hb;
use hb::hb_glyph_info_t;
use std::{collections::HashMap, fmt::Write};
use common::*;

// freetype integration of harfbuzz_sys 0.6.1 is missing these bindings
#[link(name="harfbuzz")]
extern{
    //fn hb_ft_font_get_face(hb_font: *mut hb::hb_font_t) -> ft::ffi::FT_Face;
    //fn hb_ft_font_set_funcs(hb_font: *mut hb::hb_font_t);
    fn hb_ft_font_set_load_flags(hb_font: *mut hb::hb_font_t, load_flags: i32);
    //fn hb_ft_face_create(ft_face : ft::ffi::FT_Face, destroy : hb::hb_destroy_func_t) -> *mut hb::hb_face_t;
    fn hb_ft_font_changed(font : *mut hb::hb_font_t);
}

pub struct BufferImageCopy{
    pub buffer_offset: u64,
    pub width : u32,
    pub height: u32,
    pub u: i32,
    pub v: i32
}

#[derive(Clone)]
pub struct Style<'a>{
    pub font_idx: u32,
    pub size:     u32,
    pub weight:   u32,
    pub color:    Color,
    pub autohint: bool,
    pub subpixel: i32,
    pub features: &'a[&'a str],
}
impl Style<'_> {
    fn load_flags(&self) -> ft::face::LoadFlag {
        if self.autohint {
            ft::face::LoadFlag::FORCE_AUTOHINT
        } else {
            ft::face::LoadFlag::NO_AUTOHINT
        }
    }
    fn features(&self) -> Vec<hb::hb_feature_t> {
        self.features.into_iter().map(|f|{
            let mut ret = unsafe{core::mem::MaybeUninit::<hb::hb_feature_t>::zeroed().assume_init()};
            if unsafe{hb::hb_feature_from_string(f.as_ptr() as *const i8, f.len() as i32, core::ptr::addr_of_mut!(ret))} != 0 {
                panic!("failed to parse feature: {f:?}");
            };
            ret
        }).collect()
    }
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

// TODO: make multi-thread friendly
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
    pub buffer_updates : Vec<BufferImageCopy>,
    pub pixels         : Vec<u8>,
}

#[derive(Clone,Copy,PartialEq)]
#[repr(u32)]
pub enum Direction{
    LeftToRight = hb::HB_DIRECTION_LTR,
    RightToLeft = hb::HB_DIRECTION_RTL,
    TopToBottom = hb::HB_DIRECTION_TTB,
    BottomToTop = hb::HB_DIRECTION_BTT,
}
impl Direction{
    pub const fn is_horizontal(self) -> bool {
        (self as u32) == (Self::LeftToRight as u32) || (self as u32) == (Self::RightToLeft as u32)
    }
    pub const fn is_vertical(self) -> bool {
        (self as u32) == (Self::TopToBottom as u32) || (self as u32) == (Self::BottomToTop as u32)
    }
}

#[repr(transparent)]
pub struct Locale{
    segment_properties: hb::hb_segment_properties_t,
}
impl Locale{
    // todo: expose language as enum/enum-like struct
    const fn with_language_tag(lang: hb::hb_language_t, script: Script, direction: Direction) -> Self {
        // reserved members in hb_segment_properties_t, make sure to zero out struct to remain forward compatible
        let mut segment_properties = unsafe{core::mem::MaybeUninit::<hb::hb_segment_properties_t>::zeroed().assume_init()};
        segment_properties.language = lang;
        segment_properties.direction = direction as hb::hb_direction_t;
        segment_properties.script = script.0;
        Locale{segment_properties}
    }
    pub fn new(lang: &str, script: Script, direction: Direction) -> Self {
        let tag = unsafe{hb::hb_language_from_string(lang.as_ptr() as *const i8, lang.len() as i32)} as hb::hb_language_t;
        Self::with_language_tag(tag, script, direction)
    }
}

/// mandatory breaks according to LB4 and LB5 of [UAX #14 rev 51](https://www.unicode.org/reports/tr14/tr14-51.html)
/// does not consider end-of-line as linebreak
fn is_mandatory_linebreak(c: char) -> bool {
    use icu::properties::LineBreak;
    let line_break_map = icu::properties::maps::line_break();
    match line_break_map.get(c) {
        LineBreak::MandatoryBreak |
        LineBreak::CarriageReturn |
        LineBreak::LineFeed |
        LineBreak::NextLine => true,
        _ => false,
    }
}


struct Font{
    ft_face: ft::Face,
    hb_font: *mut hb::hb_font_t, // hb_font_t is an opaque type
}
impl Font{
    fn from_path(lib: &ft::Library, path: &str) -> Self {
        let mut ft_face = lib.new_face(path, 0).expect("could not find font");
        let hb_font = unsafe{hb::freetype::hb_ft_font_create_referenced(ft_face.raw_mut())};
        Self{ ft_face, hb_font }
    }
    fn apply_style(&mut self, style: &Style){
        use hb::*;
        assert!(style.subpixel>=1);
        assert!(style.subpixel<=64);

        self.ft_face.set_char_size(0, (style.size as isize)*64, 0, 0).unwrap();

        // TODO: assert that exactly 1 variable axis exists, and that it corresponds to font-weight
        let var = (style.weight as i64) <<16;
        unsafe{ft::ffi::FT_Set_Var_Design_Coordinates(self.ft_face.raw_mut(), 1, &var)};

        unsafe{hb_ft_font_changed(self.hb_font)};
        unsafe{hb_ft_font_set_load_flags(self.hb_font, style.load_flags().bits())};
    }
}

#[derive(Default)]
pub struct StyledParagraph<'style>{
    text : String,
    runs : Vec<(&'style Locale,&'style Style<'style>,u32,u32)>,
}

// invariant: no leading whitespaces allowed
impl<'style> StyledParagraph<'style>{

    pub fn add(&mut self, locale:&'style Locale, style: &'style Style, input:&str){
        let begin = self.text.len() as u32;
        let input = if begin==0 { input.trim_start() } else { input };
        Self::NFD.normalize_to(input, &mut self.text);
        let end = self.text.len() as u32;
        println!("[{}]", self.str(begin,end));
        self.runs.push((&locale,style,begin,end));
    }

    // even members are linebreak starts
    // odd  members are whitespace starts
    fn linebreak_candidates(&self) -> Segmentation<()> {
        let view = self.text.trim_end();
        let line_breaker = icu::segmenter::LineSegmenter::new_dictionary();
        let index = line_breaker.segment_str(view).map_windows(|&[l,r]|{
            let m = l+view[l..r].trim_end().len();
            [l as u32, m as u32]
        }).flatten().collect();
        Segmentation{data: Vec::new(), index}
    }

    fn str(&self, begin:u32, end:u32) -> &str { &self.text[begin as usize..end as usize] }
    fn trim_end_idx(&self, begin:u32, end:u32) -> u32 { begin + self.str(begin,end).trim_end().len() as u32 }

    const NFD : icu::normalizer::DecomposingNormalizer = icu::normalizer::DecomposingNormalizer::new_nfd();
    fn _debug_print(&self, begin:u32, end:u32){
        let text = self.str(begin,end);

        // byte index
        for i in 0..text.len() {
            if i&1==0 { print!("\x1B[95m"); }else{ print!("\x1B[91m"); }
            print!("{:2}", i%100);
        } println!("\x1b[m");

        // bytes
        for (i,b) in text.bytes().enumerate() {
            if i&1==0 { print!("\x1B[95m"); }else{ print!("\x1B[91m"); }
            print!("{b:02x}");
        } println!("\x1b[m");
        
        // code points
        for (i,c) in text.chars().enumerate() {
            let u = c as u32;
            if u>0x20 && u<0x7F {
                print!("\x1B[97;1m{c} \x1B[m")
            } else {
                if i&1==0 { print!("\x1B[95m"); }else{ print!("\x1B[91m"); }
                print!("{u:0len$x}",len=2*c.len_utf8());
            }
        } println!("\x1b[m");
    }
    fn _debug_print_clusters(&self, clusters: &[u32]){
        for (color,[l,r]) in clusters.iter().map_windows(|&[l,r]|[l,r]).enumerate() {
            let len = r-l;
            if color&1==0 { print!("\x1B[95m"); }else{ print!("\x1B[91m"); }
            for i in 0..len {
                let first = if i==0     {"└"} else {"─"};
                let last  = if i==len-1 {"┘"} else {"─"};
                print!("{}{}", first, last);
            }
        } println!("\x1b[m");
    }
}

struct Segmentation<T> {
    data:  Vec<T>,
    index: Vec<u32>
}
impl<T> Segmentation<T> {
    fn iter(&self) -> impl Iterator<Item=((u32,u32),&T)> {
        self.iter_index().zip(self.data.iter())
    }
    fn iter_index(&self) -> impl Iterator<Item=(u32,u32)> + '_ {
        self.index.array_windows()
            .map(|&[l,r]| (l,r))
    }

    fn chunk2(&self) -> impl Iterator<Item=([u32;3],[&T;2])> {
        self.iter().array_chunks::<2>()
            .map(|[((l,m),a),((_,r),b)]|([l,m,r],[a,b]))
    }

    // broken?
    // fn map<U>(&self, mut f:impl FnMut((u32,u32),&T)->U) -> Segmentation<U> {
    //     let mut data : Vec<U> = Vec::with_capacity(self.data.len());
    //     for ((l,r),t) in self.iter() {
    //         let u : U = f((l,r),t);
    //         data.push(u);
    //     }
    //     Segmentation{
    //         data,
    //         index: self.index.clone(),
    //     }
    // }
}

type HbGlyph = (hb::hb_glyph_info_t,hb::hb_glyph_position_t);

fn gen_style_segmentation<'s>(styled_text :  &'s StyledParagraph,
                              shaped_glyphs: &Segmentation<HbGlyph> )
                              -> Segmentation::<&'s Style<'s>> {
    let mut styles = Segmentation::<&Style>{
        data:  Vec::default(),
        index: Vec::default()
    };
    let mut last = 0u32;
    for &(locale,style,begin,end) in styled_text.runs.iter() {
        styles.data.push(style);
        styles.index.push(begin);
        last = end;
    }
    styles.index.push(last);
    return styles;
}


// right now single-threaded, but in future per-thread state
// OPEN QUESTION: should we expose ICU? optional feature?
pub struct TextEngine{
    _freetype_lib: ft::Library,
    glyph_cache: GlyphCache, 
    buffer:      *mut hb::hb_buffer_t,
    fonts:       Vec<Font>,
}
impl TextEngine {
    pub fn new(glyph_texture_size:u16, font_file_paths: &[&str]) -> Self {
        let freetype_lib = ft::Library::init().expect("failed to initialize freetype");
        let mut fonts = Vec::with_capacity(font_file_paths.len());
        for path in font_file_paths {
            fonts.push(Font::from_path(&freetype_lib, path));
        }
        TextEngine{
            _freetype_lib: freetype_lib,
            buffer: unsafe{hb::hb_buffer_create()},
            fonts,
            glyph_cache: GlyphCache::new(glyph_texture_size),
        }
    }

    pub fn render_paragraph( &mut self,
            cursor_f:        &mut Vec2<f32>,
            max_line_width:  f32,
            parskip_factor:  f32,
            styled_text:    &StyledParagraph) -> Text {
        use hb::*;
        let mut cursor : Vec2<i32> = cursor_f.map(|o|(o*64.0).round() as i32);
        let left_margin = cursor.x;

        let max_line_width = (max_line_width*64.0).round() as i32;

        let (shaped_glyphs,max_lineskip) = self.shape_styled_paragraph(&styled_text);
        let styles = gen_style_segmentation(&styled_text, &shaped_glyphs);

        let mut break_opportunities = self.line_break_lengths(styled_text, &shaped_glyphs);
        // if odd number of elements, add empty segment at the end
        // should probably be handled by chunk2
        if break_opportunities.data.len()%2 != 0 {
            break_opportunities.index.push(*break_opportunities.index.last().unwrap());
            break_opportunities.data.push(0);
        }
        let break_opportunities = break_opportunities;

        // greedy line-break
        let mut break_points = Vec::new();
        let mut length_so_far = 0;
        for ([l,m,r],[word_width,space_width]) in break_opportunities.chunk2(){
            if length_so_far + word_width > max_line_width {
                break_points.push(l);
                length_so_far = word_width+space_width;
            }else{
                length_so_far += word_width + space_width;
            }
        }

        // render
        cursor.y += ((max_lineskip as f32)*parskip_factor).round() as i32;
        let mut ret = Text::default();
        let mut break_points_iter = break_points.iter();
        let mut next_break_point = *break_points_iter.next().unwrap();
        let mut shaped_glyph_iter = shaped_glyphs.iter();
        for ((_,style_r),&style) in styles.iter() {

            let font = &mut self.fonts[style.font_idx as usize];
            font.apply_style(style);

            for ((l,r),&(info,pos)) in &mut shaped_glyph_iter {
                self.rasterize_glyph(&mut ret, style, cursor, info, pos);

                if r==next_break_point {
                    println!("newline: {l}~{r}");
                    cursor.x = left_margin;
                    cursor.y += max_lineskip;
                    next_break_point = *break_points_iter.next().unwrap_or(&0);
                } else {
                    cursor.x += pos.x_advance;
                }

                if style_r==r { break }
            }
        }
        println!("{cursor} -> {}", cursor.map(|o|o as f32/64.0));

        *cursor_f = vec2(left_margin, cursor.y)/64.0;
        ret
    }

    // Returns array of line-break indices (inclusive), and an array of segment-lengths.
    // the array of segment lengths is one shorter than the line break lengths.
    fn line_break_lengths(&mut self, styled_text: &StyledParagraph, shaped_glyphs: &Segmentation<HbGlyph>) -> Segmentation<i32> {
        let mut shaped_glyphs_iter = shaped_glyphs.iter();
        let linebreaks = styled_text.linebreak_candidates();

        // styled_text._debug_print(0, styled_text.text.len() as u32);
        // styled_text._debug_print_clusters(shaped_glyphs.index.as_slice());
        // styled_text._debug_print_clusters(linebreaks.index.as_slice());

        let mut ret = Vec::new();
        for (l,r) in linebreaks.iter_index() {
            let mut length = 0;
            for ((begin,end),(info,pos)) in &mut shaped_glyphs_iter {
                length += pos.x_advance;
                if end>=r { break }
            }
            ret.push(length);
        }
        Segmentation{index:linebreaks.index, data:ret}
    }

    fn shape_styled_paragraph(&mut self, styled_text :&StyledParagraph) -> (Segmentation<HbGlyph>,i32) {
        let mut max_lineskip = 0;
        let mut shaped_glyphs :Vec<HbGlyph> = Vec::new();
        let mut cluster_offset = 0;
        for &(locale,style,begin,end) in styled_text.runs.iter() {
            let text = styled_text.str(begin,end);
            let font = &mut self.fonts[style.font_idx as usize];
            font.apply_style(style);

            let mut extents = unsafe{core::mem::MaybeUninit::<hb::hb_font_extents_t>::zeroed().assume_init()};
            unsafe{hb::hb_font_get_extents_for_direction(font.hb_font, locale.segment_properties.direction, core::ptr::addr_of_mut!(extents))};
            let lineskip = extents.line_gap + extents.ascender - extents.descender;
            println!("{lineskip:6} <- line gap: {gap}, asc:{asc}, desc:{desc}", gap=extents.line_gap, asc=extents.ascender, desc=extents.descender);
            max_lineskip = max_lineskip.max(lineskip);

            let features = style.features();
            let mut local_shaped_glyphs = self.shape_text_run(locale, style.font_idx, &features, text);
            for (info,pos) in local_shaped_glyphs.iter_mut() {
                info.cluster += cluster_offset;
            }
            shaped_glyphs.extend_from_slice(local_shaped_glyphs.as_slice());
            cluster_offset += end-begin;
        }
        assert_eq!(cluster_offset, styled_text.text.len() as u32);
        let shaped_glyphs_index : Vec<u32> = shaped_glyphs.iter()
            .map(|&(info,pos)|info.cluster)
            .chain(std::iter::once(styled_text.text.len() as u32))
            .collect();
        (Segmentation{ data: shaped_glyphs, index: shaped_glyphs_index },max_lineskip)
    }

    // warning: must call font::apply_style before this
    fn shape_text_run(&mut self, locale:&Locale, font_idx:u32, features: &[hb::hb_feature_t], text:&str) -> Vec<HbGlyph> {
        use hb::*;
        let font = &self.fonts[font_idx as usize];
        unsafe{
            hb_buffer_reset(self.buffer);
            hb_buffer_add_utf8(self.buffer, text.as_ptr() as *const i8, text.len() as i32, 0, -1);
            hb_buffer_set_segment_properties(self.buffer, &locale.segment_properties);
            hb_shape(font.hb_font, self.buffer, if features.len()==0 {core::ptr::null()} else {features.as_ptr()}, features.len() as u32);
        };

        let mut glyph_info_count = 0;
        let glyph_info_ptr = unsafe{hb_buffer_get_glyph_infos(self.buffer, &mut glyph_info_count)};
        let glyph_infos = unsafe{core::slice::from_raw_parts(glyph_info_ptr, glyph_info_count as usize)};

        let mut glyph_pos_count = 0;
        let glyph_pos_ptr = unsafe{hb_buffer_get_glyph_positions(self.buffer, &mut glyph_pos_count)};
        let glyph_positons = unsafe{core::slice::from_raw_parts(glyph_pos_ptr, glyph_pos_count as usize)};

        assert_eq!(glyph_info_count, glyph_pos_count);

        std::iter::zip(glyph_infos, glyph_positons).map(|(i,p)|(*i,*p)).collect()
    }

    fn rasterize_glyph(&mut self, ret: &mut Text, style: &Style, cursor: Vec2<i32>, info: hb::hb_glyph_info_t, pos: hb::hb_glyph_position_t){
        let font = &self.fonts[style.font_idx as usize];

        let id = info.codepoint; // actually glyph index, not codepoint
        let x = div_round((cursor.x + pos.x_offset)*style.subpixel, 64);
        let y = div_round( cursor.y + pos.y_offset , 64);
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
            font.ft_face.load_glyph(id, style.load_flags()).unwrap();
            let subpixel_offset = Some(ft::Vector{x:frac64 as i64, y:0});
            let glyph  = font.ft_face.glyph().get_glyph().unwrap().to_bitmap(ft::render_mode::RenderMode::Lcd, subpixel_offset).unwrap();
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
                    BufferImageCopy{
                        buffer_offset,
                        width:  width as u32,
                        height: height as u32,
                        u: uv.0 as i32,
                        v: uv.1 as i32 });
            }
        }
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// own definitions to keep HarfBuzz out of the public interface

const fn make_tag(s:[u8;4]) -> u32 {
    let mut ret = 0u32;
    ret |=  s[0] as u32;
    ret |= (s[1] as u32)<<8;
    ret |= (s[2] as u32)<<16;
    ret |= (s[3] as u32)<<24;
    return ret;
}

// ISO 15924 script tag
pub struct Script(u32);
impl Script{
    pub const fn new(s:&[u8;4]) -> Self { Script(make_tag(*s)) }
    pub const COMMON                 :Self = Self::new(b"Zyyy");
    pub const INHERITED              :Self = Self::new(b"Zinh");
    pub const UNKNOWN                :Self = Self::new(b"Zzzz");
    pub const ARABIC                 :Self = Self::new(b"Arab");
    pub const ARMENIAN               :Self = Self::new(b"Armn");
    pub const BENGALI                :Self = Self::new(b"Beng");
    pub const CYRILLIC               :Self = Self::new(b"Cyrl");
    pub const DEVANAGARI             :Self = Self::new(b"Deva");
    pub const GEORGIAN               :Self = Self::new(b"Geor");
    pub const GREEK                  :Self = Self::new(b"Grek");
    pub const GUJARATI               :Self = Self::new(b"Gujr");
    pub const GURMUKHI               :Self = Self::new(b"Guru");
    pub const HANGUL                 :Self = Self::new(b"Hang");
    pub const HAN                    :Self = Self::new(b"Hani");
    pub const HEBREW                 :Self = Self::new(b"Hebr");
    pub const HIRAGANA               :Self = Self::new(b"Hira");
    pub const KANNADA                :Self = Self::new(b"Knda");
    pub const KATAKANA               :Self = Self::new(b"Kana");
    pub const LAO                    :Self = Self::new(b"Laoo");
    pub const LATIN                  :Self = Self::new(b"Latn");
    pub const MALAYALAM              :Self = Self::new(b"Mlym");
    pub const ORIYA                  :Self = Self::new(b"Orya");
    pub const TAMIL                  :Self = Self::new(b"Taml");
    pub const TELUGU                 :Self = Self::new(b"Telu");
    pub const THAI                   :Self = Self::new(b"Thai");
    pub const TIBETAN                :Self = Self::new(b"Tibt");
    pub const BOPOMOFO               :Self = Self::new(b"Bopo");
    pub const BRAILLE                :Self = Self::new(b"Brai");
    pub const CANADIAN_SYLLABICS     :Self = Self::new(b"Cans");
    pub const CHEROKEE               :Self = Self::new(b"Cher");
    pub const ETHIOPIC               :Self = Self::new(b"Ethi");
    pub const KHMER                  :Self = Self::new(b"Khmr");
    pub const MONGOLIAN              :Self = Self::new(b"Mong");
    pub const MYANMAR                :Self = Self::new(b"Mymr");
    pub const OGHAM                  :Self = Self::new(b"Ogam");
    pub const RUNIC                  :Self = Self::new(b"Runr");
    pub const SINHALA                :Self = Self::new(b"Sinh");
    pub const SYRIAC                 :Self = Self::new(b"Syrc");
    pub const THAANA                 :Self = Self::new(b"Thaa");
    pub const YI                     :Self = Self::new(b"Yiii");
    pub const DESERET                :Self = Self::new(b"Dsrt");
    pub const GOTHIC                 :Self = Self::new(b"Goth");
    pub const OLD_ITALIC             :Self = Self::new(b"Ital");
    pub const BUHID                  :Self = Self::new(b"Buhd");
    pub const HANUNOO                :Self = Self::new(b"Hano");
    pub const TAGALOG                :Self = Self::new(b"Tglg");
    pub const TAGBANWA               :Self = Self::new(b"Tagb");
    pub const CYPRIOT                :Self = Self::new(b"Cprt");
    pub const LIMBU                  :Self = Self::new(b"Limb");
    pub const LINEAR_B               :Self = Self::new(b"Linb");
    pub const OSMANYA                :Self = Self::new(b"Osma");
    pub const SHAVIAN                :Self = Self::new(b"Shaw");
    pub const TAI_LE                 :Self = Self::new(b"Tale");
    pub const UGARITIC               :Self = Self::new(b"Ugar");
    pub const BUGINESE               :Self = Self::new(b"Bugi");
    pub const COPTIC                 :Self = Self::new(b"Copt");
    pub const GLAGOLITIC             :Self = Self::new(b"Glag");
    pub const KHAROSHTHI             :Self = Self::new(b"Khar");
    pub const NEW_TAI_LUE            :Self = Self::new(b"Talu");
    pub const OLD_PERSIAN            :Self = Self::new(b"Xpeo");
    pub const SYLOTI_NAGRI           :Self = Self::new(b"Sylo");
    pub const TIFINAGH               :Self = Self::new(b"Tfng");
    pub const BALINESE               :Self = Self::new(b"Bali");
    pub const CUNEIFORM              :Self = Self::new(b"Xsux");
    pub const NKO                    :Self = Self::new(b"Nkoo");
    pub const PHAGS_PA               :Self = Self::new(b"Phag");
    pub const PHOENICIAN             :Self = Self::new(b"Phnx");
    pub const CARIAN                 :Self = Self::new(b"Cari");
    pub const CHAM                   :Self = Self::new(b"Cham");
    pub const KAYAH_LI               :Self = Self::new(b"Kali");
    pub const LEPCHA                 :Self = Self::new(b"Lepc");
    pub const LYCIAN                 :Self = Self::new(b"Lyci");
    pub const LYDIAN                 :Self = Self::new(b"Lydi");
    pub const OL_CHIKI               :Self = Self::new(b"Olck");
    pub const REJANG                 :Self = Self::new(b"Rjng");
    pub const SAURASHTRA             :Self = Self::new(b"Saur");
    pub const SUNDANESE              :Self = Self::new(b"Sund");
    pub const VAI                    :Self = Self::new(b"Vaii");
    pub const AVESTAN                :Self = Self::new(b"Avst");
    pub const BAMUM                  :Self = Self::new(b"Bamu");
    pub const EGYPTIAN_HIEROGLYPHS   :Self = Self::new(b"Egyp");
    pub const IMPERIAL_ARAMAIC       :Self = Self::new(b"Armi");
    pub const INSCRIPTIONAL_PAHLAVI  :Self = Self::new(b"Phli");
    pub const INSCRIPTIONAL_PARTHIAN :Self = Self::new(b"Prti");
    pub const JAVANESE               :Self = Self::new(b"Java");
    pub const KAITHI                 :Self = Self::new(b"Kthi");
    pub const LISU                   :Self = Self::new(b"Lisu");
    pub const MEETEI_MAYEK           :Self = Self::new(b"Mtei");
    pub const OLD_SOUTH_ARABIAN      :Self = Self::new(b"Sarb");
    pub const OLD_TURKIC             :Self = Self::new(b"Orkh");
    pub const SAMARITAN              :Self = Self::new(b"Samr");
    pub const TAI_THAM               :Self = Self::new(b"Lana");
    pub const TAI_VIET               :Self = Self::new(b"Tavt");
    pub const BATAK                  :Self = Self::new(b"Batk");
    pub const BRAHMI                 :Self = Self::new(b"Brah");
    pub const MANDAIC                :Self = Self::new(b"Mand");
    pub const CHAKMA                 :Self = Self::new(b"Cakm");
    pub const MEROITIC_CURSIVE       :Self = Self::new(b"Merc");
    pub const MEROITIC_HIEROGLYPHS   :Self = Self::new(b"Mero");
    pub const MIAO                   :Self = Self::new(b"Plrd");
    pub const SHARADA                :Self = Self::new(b"Shrd");
    pub const SORA_SOMPENG           :Self = Self::new(b"Sora");
    pub const TAKRI                  :Self = Self::new(b"Takr");
    pub const BASSA_VAH              :Self = Self::new(b"Bass");
    pub const CAUCASIAN_ALBANIAN     :Self = Self::new(b"Aghb");
    pub const DUPLOYAN               :Self = Self::new(b"Dupl");
    pub const ELBASAN                :Self = Self::new(b"Elba");
    pub const GRANTHA                :Self = Self::new(b"Gran");
    pub const KHOJKI                 :Self = Self::new(b"Khoj");
    pub const KHUDAWADI              :Self = Self::new(b"Sind");
    pub const LINEAR_A               :Self = Self::new(b"Lina");
    pub const MAHAJANI               :Self = Self::new(b"Mahj");
    pub const MANICHAEAN             :Self = Self::new(b"Mani");
    pub const MENDE_KIKAKUI          :Self = Self::new(b"Mend");
    pub const MODI                   :Self = Self::new(b"Modi");
    pub const MRO                    :Self = Self::new(b"Mroo");
    pub const NABATAEAN              :Self = Self::new(b"Nbat");
    pub const OLD_NORTH_ARABIAN      :Self = Self::new(b"Narb");
    pub const OLD_PERMIC             :Self = Self::new(b"Perm");
    pub const PAHAWH_HMONG           :Self = Self::new(b"Hmng");
    pub const PALMYRENE              :Self = Self::new(b"Palm");
    pub const PAU_CIN_HAU            :Self = Self::new(b"Pauc");
    pub const PSALTER_PAHLAVI        :Self = Self::new(b"Phlp");
    pub const SIDDHAM                :Self = Self::new(b"Sidd");
    pub const TIRHUTA                :Self = Self::new(b"Tirh");
    pub const WARANG_CITI            :Self = Self::new(b"Wara");
    pub const AHOM                   :Self = Self::new(b"Ahom");
    pub const ANATOLIAN_HIEROGLYPHS  :Self = Self::new(b"Hluw");
    pub const HATRAN                 :Self = Self::new(b"Hatr");
    pub const MULTANI                :Self = Self::new(b"Mult");
    pub const OLD_HUNGARIAN          :Self = Self::new(b"Hung");
    pub const SIGNWRITING            :Self = Self::new(b"Sgnw");
    pub const ADLAM                  :Self = Self::new(b"Adlm");
    pub const BHAIKSUKI              :Self = Self::new(b"Bhks");
    pub const MARCHEN                :Self = Self::new(b"Marc");
    pub const OSAGE                  :Self = Self::new(b"Osge");
    pub const TANGUT                 :Self = Self::new(b"Tang");
    pub const NEWA                   :Self = Self::new(b"Newa");
    pub const MASARAM_GONDI          :Self = Self::new(b"Gonm");
    pub const NUSHU                  :Self = Self::new(b"Nshu");
    pub const SOYOMBO                :Self = Self::new(b"Soyo");
    pub const ZANABAZAR_SQUARE       :Self = Self::new(b"Zanb");
    pub const DOGRA                  :Self = Self::new(b"Dogr");
    pub const GUNJALA_GONDI          :Self = Self::new(b"Gong");
    pub const HANIFI_ROHINGYA        :Self = Self::new(b"Rohg");
    pub const MAKASAR                :Self = Self::new(b"Maka");
    pub const MEDEFAIDRIN            :Self = Self::new(b"Medf");
    pub const OLD_SOGDIAN            :Self = Self::new(b"Sogo");
    pub const SOGDIAN                :Self = Self::new(b"Sogd");
    pub const ELYMAIC                :Self = Self::new(b"Elym");
    pub const NANDINAGARI            :Self = Self::new(b"Nand");
    pub const NYIAKENG_PUACHUE_HMONG :Self = Self::new(b"Hmnp");
    pub const WANCHO                 :Self = Self::new(b"Wcho");
    pub const CHORASMIAN             :Self = Self::new(b"Chrs");
    pub const DIVES_AKURU            :Self = Self::new(b"Diak");
    pub const KHITAN_SMALL_SCRIPT    :Self = Self::new(b"Kits");
    pub const YEZIDI                 :Self = Self::new(b"Yezi");
    pub const CYPRO_MINOAN           :Self = Self::new(b"Cpmn");
    pub const OLD_UYGHUR             :Self = Self::new(b"Ougr");
    pub const TANGSA                 :Self = Self::new(b"Tnsa");
    pub const TOTO                   :Self = Self::new(b"Toto");
    pub const VITHKUQI               :Self = Self::new(b"Vith");
    pub const MATH                   :Self = Self::new(b"Zmth");
    pub const KAWI                   :Self = Self::new(b"Kawi");
    pub const NAG_MUNDARI            :Self = Self::new(b"Nagm");
}
