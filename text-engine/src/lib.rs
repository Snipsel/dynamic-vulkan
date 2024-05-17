use freetype as ft;
use harfbuzz_sys as hb;
use std::collections::HashMap;
use common::{Color,Vertex,vec2,div_round,gen_quad};

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
    pub buffer_updates : Vec<BufferImageCopy>,
    pub pixels         : Vec<u8>,
}

pub struct TextEngine{
    _freetype_lib: ft::Library,
    pub glyph_cache: GlyphCache,
    buffer:      *mut hb::hb_buffer_t,
    ft_faces:    Vec<ft::Face>,
    hb_fonts:    Vec<*mut hb::hb_font_t>,
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

impl TextEngine {
    pub fn new(glyph_texture_size:u16, fontfiles: &[&str]) -> Self {
        let freetype_lib = ft::Library::init().expect("failed to initialize freetype");
        let mut hb_fonts = Vec::new();
        let mut ft_faces = Vec::new();
        for file in fontfiles {
            ft_faces.push(freetype_lib.new_face(file, 0).expect("could not find font"));
        }
        for ft_face in &mut ft_faces {
            let hb_font = unsafe{hb::freetype::hb_ft_font_create_referenced(ft_face.raw_mut())};
            hb_fonts.push(hb_font);
        }
        TextEngine{
            _freetype_lib: freetype_lib,
            buffer: unsafe{hb::hb_buffer_create()},
            ft_faces, hb_fonts,
            glyph_cache: GlyphCache::new(glyph_texture_size),
        }
    }


    // Text rendering can fundamentally not be cleanly separated into parts. Everything affects
    // everything else. This means the easiest thing to do is have a monolithic function that does
    // everything. It's better to have a monolithic function as API that can hide language
    // complexities, than have a complex API.
    //
    // next features: subpixel positioning and line-breaking
    pub fn render_line_of_text(&mut self,
            ret:            &mut Text,
            locale:         &Locale,
            style:          &Style,
            start_position: vec2<i32>,
            text:           &str){
        use hb::*;
        assert!(style.subpixel>=1);
        assert!(style.subpixel<=64);

        let hb_font = self.hb_fonts[style.font_idx as usize];
        let ft_face = &mut self.ft_faces[style.font_idx as usize];
        ft_face.set_char_size(0, (style.size as isize)*64, 0, 0).unwrap();


        // TODO: assert that exactly 1 variable axis exists, and that it corresponds to font-weight
        //let mut amaster : *mut ft::ffi::FT_MM_Var = core::ptr::null_mut();
        let var = (style.weight as i64) <<16;
        unsafe{ft::ffi::FT_Set_Var_Design_Coordinates(ft_face.raw_mut(), 1, &var)};

        unsafe{hb_ft_font_changed(hb_font)};
        let load_flags = if style.autohint { ft::face::LoadFlag::FORCE_AUTOHINT } else { ft::face::LoadFlag::NO_AUTOHINT };
        unsafe{hb_ft_font_set_load_flags(hb_font, load_flags.bits())};

        unsafe{
            hb_buffer_reset(self.buffer);
            hb_buffer_add_utf8(self.buffer, text.as_ptr() as *const i8, text.len() as i32, 0, -1);
            hb_buffer_set_segment_properties(self.buffer, &locale.segment_properties);
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
            let y = div_round( cursor.1 + pos.y_offset , 64);
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
                ft_face.load_glyph(id, load_flags).unwrap();
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
                        BufferImageCopy{
                            buffer_offset,
                            width:  width as u32,
                            height: height as u32,
                            u: uv.0 as i32,
                            v: uv.1 as i32 });
                }
            }

            cursor.0 += pos.x_advance;
            cursor.1 += pos.y_advance;
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
