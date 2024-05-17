#![allow(unused)]
mod renderer;

use std::{
    collections::HashMap, ffi::{CStr,OsStr}, fmt, mem::size_of, ptr
};
use freetype as ft;
use harfbuzz_sys as hb;
use hb::{freetype::hb_ft_font_create_referenced, hb_face_create, hb_font_get_face, hb_font_set_variations, hb_language_t};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    raw_window_handle::{HasDisplayHandle, HasWindowHandle},
    window::{Window,WindowId}
};
use ash::{
     ext, khr, vk::{self, CommandBuffer, CommandPool, Fence, Handle, Image, ImageView, InstanceCreateInfo, PhysicalDevice, Queue, Semaphore, ShaderEXT, SurfaceFormatKHR, SurfaceKHR, SwapchainCreateInfoKHR, SwapchainKHR, WriteDescriptorSet}, Device, Entry, Instance
};
use bitflags::bitflags;

mod math{
    #![allow(non_camel_case_types)]
    use core::ops::*;

    // pub const fn q6(x:i32) -> i32q6 { i32q6(x<<6) }

    // #[repr(transparent)]
    // #[derive(Clone,Copy)]
    // pub struct i32q6(pub(super)i32);
    // impl From<i32> for i32q6 { fn from(value: i32) -> Self { Self(value<<6) } }
    // impl Add for i32q6 { type Output=Self; fn add(self, rhs: Self) -> Self { Self(self.0+rhs.0) } }
    // impl Sub for i32q6 { type Output=Self; fn sub(self, rhs: Self) -> Self { Self(self.0-rhs.0) } }
    // impl Mul for i32q6 { type Output=Self; fn mul(self, rhs: Self) -> Self { Self((self.0*rhs.0)>>6) } }
    // impl AddAssign      for i32q6 { fn add_assign(&mut self, rhs: Self) { self.0 += rhs.0;  } }
    // impl SubAssign      for i32q6 { fn sub_assign(&mut self, rhs: Self) { self.0 -= rhs.0;  } }
    // impl AddAssign<i32> for i32q6 { fn add_assign(&mut self, rhs: i32)  { self.0 += rhs<<6; } }
    // impl SubAssign<i32> for i32q6 { fn sub_assign(&mut self, rhs: i32)  { self.0 -= rhs<<6; } }

    #[derive(Clone,Copy)]
    pub struct vec2<T>(pub T,pub T) where T:Clone+Copy;
    impl<T> Add for vec2<T> where T:Clone+Copy+Add<Output=T> { type Output=Self; fn add(self, rhs: Self) -> Self { Self(self.0+rhs.0, self.1+rhs.1) } }
    impl<T> Sub for vec2<T> where T:Clone+Copy+Sub<Output=T> { type Output=Self; fn sub(self, rhs: Self) -> Self { Self(self.0-rhs.0, self.1-rhs.1) } }
    impl<T> Mul<T> for vec2<T> where T:Clone+Copy+Mul<Output=T> { type Output=Self; fn mul(self, rhs: T) -> Self { Self(self.0*rhs, self.1*rhs) } }
    impl<T> Div<T> for vec2<T> where T:Clone+Copy+Div<Output=T> { type Output=Self; fn div(self, rhs: T) -> Self { Self(self.0/rhs, self.1/rhs) } }
    impl<T,U> AddAssign<vec2<U>> for vec2<T> where T:Clone+Copy+AddAssign<U>, U:Clone+Copy { fn add_assign(&mut self, rhs: vec2<U>) { self.0 += rhs.0; self.1 += rhs.1; } }
    impl<T,U> SubAssign<vec2<U>> for vec2<T> where T:Clone+Copy+SubAssign<U>, U:Clone+Copy { fn sub_assign(&mut self, rhs: vec2<U>) { self.0 -= rhs.0; self.1 -= rhs.1; } }
}
use math::{vec2};

fn begin_oneshot_cmd(renderer: &renderer::Renderer) -> vk::CommandBuffer {
    let alloc_info = vk::CommandBufferAllocateInfo::default()
        .level(vk::CommandBufferLevel::PRIMARY)
        .command_pool(renderer.command_pool)
        .command_buffer_count(1);
    let cmdbuf = unsafe{renderer.device.allocate_command_buffers(&alloc_info)}.unwrap()[0];
    let begin_info = vk::CommandBufferBeginInfo::default()
        .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
    unsafe{renderer.device.begin_command_buffer(cmdbuf, &begin_info)};
    cmdbuf
}

fn end_oneshot_cmd(renderer: &renderer::Renderer, cmdbuf : vk::CommandBuffer){
    unsafe{renderer.device.end_command_buffer(cmdbuf)};
    let cmdbuf = [cmdbuf];
    let info = [vk::SubmitInfo::default()
        .command_buffers(&cmdbuf)];
    unsafe{renderer.device.queue_submit(renderer.queue, &info, vk::Fence::null())};
    unsafe{renderer.device.queue_wait_idle(renderer.queue)}; // TODO: remove blocking wait
    unsafe{renderer.device.free_command_buffers(renderer.command_pool, &cmdbuf)};
}

// TODO: think about colors. Right now Color means sRGBA8
// maybe have oklab colors internally and use .srgba8() to convert
#[derive(Debug, Clone, Copy)]
#[repr(C)]
struct Color{
    r:u8, g:u8, b:u8, a:u8
}
impl Color{
    const CLEAR:Color = Color{r:0x00, g:0x00, b:0x00, a:0x00};
    const WHITE:Color = Color{r:0xFF, g:0xFF, b:0xFF, a:0xFF};
    const BLACK:Color = Color{r:0x00, g:0x00, b:0x00, a:0xFF};
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
struct Vertex{
    x:i16, y:i16,
    u:u16, v:u16,
    color: Color
}

fn gen_quad(x: i16, y: i16, w: i16, h: i16, u:u16, v:u16, color: Color) -> [Vertex;4] {
    assert!(w > 0);
    assert!(h > 0);
    let w_ = w as u16;
    let h_ = h as u16;
    [
        Vertex{x:x+0,  y:y+0, u:u+0,  v:v+0,  color}, // top left
        Vertex{x:x+0,  y:y+h, u:u+0,  v:v+h_, color}, // bottom left
        Vertex{x:x+w,  y:y+0, u:u+w_, v:v+0,  color}, // top right
        Vertex{x:x+w,  y:y+h, u:u+w_, v:v+h_, color}, // bottom right
    ]
}

unsafe fn push_type<T>(ptr:*mut core::ffi::c_void, object:T) -> *mut core::ffi::c_void {
    let vertex_memory = unsafe{core::mem::transmute::<*mut core::ffi::c_void, *mut T>(ptr)};
    unsafe{core::ptr::write_volatile(vertex_memory, object)};
    let t_size = std::mem::size_of::<T>() as isize;
    unsafe{ptr.byte_offset(t_size)}
}

fn push_quad_verts(ptr:*mut core::ffi::c_void, verts: [Vertex;4]) -> *mut core::ffi::c_void {
    unsafe{push_type::<[Vertex;4]>(ptr, verts)}
}

fn push_quad_indices(ptr:*mut core::ffi::c_void, i:u16) -> *mut core::ffi::c_void {
    let indices = [ i+0, i+1, i+2, i+2, i+1, i+3 ];
    unsafe{push_type::<[u16;6]>(ptr, indices)}
}

fn new_locale(lang: &str, script: hb::hb_script_t, direction: hb::hb_direction_t) -> hb::hb_segment_properties_t {
    // zero initialize struct (due to reserved fields in the c-struct)
    let mut ret = unsafe{core::mem::MaybeUninit::<hb::hb_segment_properties_t>::zeroed().assume_init()};
    ret.language = unsafe{hb::hb_language_from_string(lang.as_ptr() as *const i8, lang.len() as i32)};
    ret.direction = direction;
    ret.script = script;
    return ret;
}

// freetype integration of harfbuzz_sys 0.6.1 is missing these bindings
#[link(name="harfbuzz")]
extern{
    //fn hb_ft_font_get_face(hb_font: *mut hb::hb_font_t) -> ft::ffi::FT_Face;
    fn hb_ft_font_set_funcs(hb_font: *mut hb::hb_font_t);
    fn hb_ft_font_set_load_flags(hb_font: *mut hb::hb_font_t, load_flags: i32);
    fn hb_ft_face_create(ft_face : ft::ffi::FT_Face, destroy : hb::hb_destroy_func_t) -> *mut hb::hb_face_t;
    fn hb_ft_font_changed(font : *mut hb::hb_font_t);
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
    current_x: u16,
}
impl GlyphCache {
    fn new() -> Self { Self { map: HashMap::new(), current_x:0 } }
    fn get(&self, key: &GlyphCacheKey) -> Option<GlyphCacheEntry> {
        self.map.get(key).copied()
    }
    fn insert(&mut self, key:GlyphCacheKey, width: u16, height:u16, left: i16, top: i16) -> (u16,u16) {
        let ret = (self.current_x, 0);
        self.map.insert(key, GlyphCacheEntry{
            u: ret.0, v: ret.1,
            width, height, left, top
        });
        self.current_x += width;
        ret
    }
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
        buffer_image_height: height, // 0 should also be fine
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

#[derive(Default)]
struct Text{
    quads          : Vec<[Vertex;4]>,
    buffer_updates : Vec<vk::BufferImageCopy>,
    pixels         : Vec<u8>,
}

fn div_round(a:i32, b:i32) -> i32 { (a+(b/2))/b }

#[derive(Clone)]
struct Style<'a>{
    font_idx: u32,
    size:     u32,
    weight:   u32,
    color:    Color,
    autohint: bool,
    features: &'a[hb::hb_feature_t],
}

struct TextRenderer{
    glyph_cache: GlyphCache,
    buffer:      *mut hb::hb_buffer_t,
    ft_faces:    Vec<ft::Face>,
    hb_fonts:    Vec<*mut hb::hb_font_t>,
}

// Text rendering can fundamentally not be cleanly separated into parts. Everything affects
// everything else. This means the easiest thing to do is have a monolithic function that does
// everything. It's better to have a monolithic function as API that can hide language
// complexities, than have a complex API.
//
// next features: subpixel positioning and line-breaking
fn render_line_of_text(
        ret:         &mut Text,
        renderer:    &mut TextRenderer,
        locale:      &hb::hb_segment_properties_t,
        style:       &Style,
        start_position: vec2<i32>,
        text:        &str){
    use hb::*;

    const SUBPIXEL_BITS :i32 = 3;
    const SUBPIXEL_MASK :i32 = ((1<<SUBPIXEL_BITS)-1);
    const SUBPIXEL_INV  :i32 = 6-SUBPIXEL_BITS;
    
    let hb_font =  renderer.hb_fonts[style.font_idx as usize];
    let mut ft_face = &mut renderer.ft_faces[style.font_idx as usize];
    ft_face.set_char_size(0, (style.size as isize)*64, 0, 0);


    // TODO: assert that exactly 1 variable axis exists, and that it corresponds to font-weight
    let mut amaster : *mut ft::ffi::FT_MM_Var = core::ptr::null_mut();
    let var = (style.weight as i64) <<16;
    unsafe{ft::ffi::FT_Set_Var_Design_Coordinates(ft_face.raw_mut(), 1, &var)};

    unsafe{hb_ft_font_changed(hb_font)};
    let load_flags = if style.autohint { ft::face::LoadFlag::FORCE_AUTOHINT } else { ft::face::LoadFlag::NO_AUTOHINT };
    unsafe{hb_ft_font_set_load_flags(hb_font, load_flags.bits())};

    unsafe{
        hb_buffer_reset(renderer.buffer);
        hb_buffer_add_utf8(renderer.buffer, text.as_ptr() as *const i8, text.len() as i32, 0, -1);
        hb_buffer_set_segment_properties(renderer.buffer, locale);
        hb_shape(hb_font, renderer.buffer, if style.features.len()==0 {core::ptr::null()} else {style.features.as_ptr()}, style.features.len() as u32);
    };

    let mut glyph_info_count = 0;
    let glyph_info_ptr = unsafe{hb_buffer_get_glyph_infos(renderer.buffer, &mut glyph_info_count)};
    let glyph_infos = unsafe{core::slice::from_raw_parts_mut(glyph_info_ptr, glyph_info_count as usize)};

    let mut glyph_pos_count = 0;
    let glyph_pos_ptr = unsafe{hb_buffer_get_glyph_positions(renderer.buffer, &mut glyph_pos_count)};
    let glyph_positons = unsafe{core::slice::from_raw_parts_mut(glyph_pos_ptr, glyph_pos_count as usize)};

    assert_eq!(glyph_info_count, glyph_pos_count);

    let mut cursor = start_position;
    for (info,pos) in std::iter::zip(glyph_infos, glyph_positons) {
        let id = info.codepoint; // actually glyph index, not codepoint
        let x = div_round(cursor.0 + pos.x_offset, 64>>SUBPIXEL_BITS);
        let y = div_round(cursor.1 + pos.y_offset, 64);
        let x_frac = x&SUBPIXEL_MASK;
        let x = x>>SUBPIXEL_BITS;
        println!("{x:4}.{x_frac:01}");

        if let Some(entry) = renderer.glyph_cache.get(&GlyphCacheKey{font_idx:style.font_idx, glyph_idx:id, font_size:style.size, autohint:style.autohint, subpixel:x_frac as u32}) {
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
            let subpixel_offset = Some(ft::Vector{x:(x_frac<<SUBPIXEL_INV)as i64, y:0});
            let glyph  = ft_face.glyph().get_glyph().unwrap().to_bitmap(ft::render_mode::RenderMode::Normal, subpixel_offset).unwrap();
            let bitmap = glyph.bitmap();
            let width  = bitmap.width();
            let height = bitmap.rows();
            let pitch  = bitmap.pitch();
            let left   = glyph.left();
            let top    = glyph.top();
            if !(width<=0 || height<=0) { 
                let uv = renderer.glyph_cache.insert( GlyphCacheKey{ font_idx:style.font_idx, glyph_idx:id, font_size:style.size, autohint:style.autohint, subpixel:x_frac as u32}, 
                                                      width as u16, height as u16, left as i16, top as i16);
                ret.quads.push(
                    gen_quad((x+left) as i16,
                             (y-top)  as i16,
                             width as i16, height as i16,
                             uv.0, uv.1,
                             style.color));
                ret.buffer_updates.push(
                    gen_buffer_image_copy(
                        ret.pixels.len() as u64,
                        pitch as u32, width as u32, height as u32,
                        uv.0 as i32,
                        uv.1 as i32));
                for b in bitmap.buffer() {
                    ret.pixels.push(*b);
                }
            }
        }

        cursor.0 += pos.x_advance;
        cursor.1 += pos.y_advance;
    }
}

#[derive(Default)]
enum App{
    #[default] Uninitialized,
    Resumed{
        renderer: renderer::Renderer,
        vs : ShaderEXT,
        fs : ShaderEXT,
        bar_buffer : vk::Buffer,
        bar_memory : *mut core::ffi::c_void,
        pipeline_layout : vk::PipelineLayout,
        descriptor_set : vk::DescriptorSet,
        image : vk::Image,
        text_renderer : TextRenderer,
    },
}
impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        match self {
            App::Resumed{..}    => todo!("handle re-resuming"),
            App::Uninitialized => {
                let freetype_lib  = ft::Library::init().expect("failed to initialize freetype");

                // let mut ft_face = freetype_lib.new_face("./fonts/source-sans/upright.otf", 0).expect("could not find font");
                // let mut hb_face = unsafe{ hb_ft_face_create(ft_face.raw_mut(), None) };
                // let hb_font = unsafe{hb::hb_font_create(hb_face)};

                let mut hb_fonts = vec![];
                let mut ft_faces = vec![
                    freetype_lib.new_face("./fonts/source-sans/upright.ttf", 0).expect("could not find font"),
                    freetype_lib.new_face("./fonts/source-sans/italic.ttf",  0).expect("could not find font"),
                    freetype_lib.new_face("./fonts/crimson-pro/upright.ttf", 0).expect("could not find font"),
                    freetype_lib.new_face("./fonts/crimson-pro/italic.ttf",  0).expect("could not find font"),
                ];
                for ft_face in &mut ft_faces {
                    let hb_font = unsafe{hb::freetype::hb_ft_font_create_referenced(ft_face.raw_mut())};
                    //unsafe{hb_ft_font_set_funcs(hb_font)};
                    //unsafe{hb_ft_font_set_load_flags(hb_font, (ft::face::LoadFlag::FORCE_AUTOHINT).bits())};
                    hb_fonts.push(hb_font);
                }

                let mut text_renderer = TextRenderer{
                    buffer: unsafe{hb::hb_buffer_create()},
                    ft_faces, hb_fonts,
                    glyph_cache: GlyphCache::new(),
                };

                // let mut ft_face2 = ft_face.clone();
                // ft_face2.set_char_size(0, 24*64, 0, 0);
                // let hb_font = unsafe{hb::freetype::hb_ft_font_create_referenced(ft_face2.raw_mut())};
                // unsafe{hb_ft_font_set_funcs(hb_font)};
                // unsafe{hb_ft_font_set_load_flags(hb_font, (ft::face::LoadFlag::FORCE_AUTOHINT).bits())};

                // let hb_face = unsafe{hb_font_get_face(hb_font)};
                // let count = unsafe{hb::hb_ot_var_get_axis_count(hb_face)};
                // let tag_wght = unsafe{ hb::hb_tag_from_string("wght".as_ptr() as *const i8, 4)};
                // let mut axis_info = unsafe{core::mem::MaybeUninit::<hb::hb_ot_var_axis_info_t>::zeroed().assume_init()};
                // if unsafe{hb::hb_ot_var_find_axis_info(hb_face, tag_wght, &mut axis_info)} >= 0 {
                //     let weight =  500.0f32.clamp(axis_info.min_value, axis_info.max_value);
                //     println!("has weight! {}<={}<={}", axis_info.min_value, weight, axis_info.max_value );
                //     let weight_data = hb::hb_variation_t{ tag: tag_wght, value: weight };
                //     unsafe{hb_font_set_variations(hb_font, &weight_data, 1)};
                // }

                let init_start = std::time::Instant::now();
                let renderer = renderer::Renderer::new(event_loop);
                let init_render = std::time::Instant::now();

                renderer.debug_print();
                let push_constant_ranges = [
                    vk::PushConstantRange::default()
                        .stage_flags(vk::ShaderStageFlags::VERTEX)
                        .size(core::mem::size_of::<[f32;4]>() as u32) ];
                //let binding_flag_bits = [vk::DescriptorBindingFlagsEXT::UPDATE_AFTER_BIND];
                //let mut binding_flags = vk::DescriptorSetLayoutBindingFlagsCreateInfoEXT::default()
                //    .binding_flags(&binding_flag_bits);

                // create texture image
                let img_info = vk::ImageCreateInfo::default()
                    .image_type(vk::ImageType::TYPE_2D)
                    .extent(vk::Extent3D{width:1<<14, height:1<<14, depth:1})
                    .mip_levels(1)
                    .array_layers(1)
                    .format(vk::Format::R8_UNORM)
                    .tiling(vk::ImageTiling::LINEAR)
                    .initial_layout(vk::ImageLayout::UNDEFINED)
                    .usage(vk::ImageUsageFlags::TRANSFER_DST
                          |vk::ImageUsageFlags::SAMPLED)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE)
                    .samples(vk::SampleCountFlags::TYPE_1);
                let image = unsafe{renderer.device.create_image(&img_info, None)}.unwrap();
                let req = unsafe{renderer.device.get_image_memory_requirements(image)};
                let alloc = vk::MemoryAllocateInfo::default()
                    .allocation_size(req.size)
                    .memory_type_index(renderer.gpu_memory_idx);
                let mem = unsafe{renderer.device.allocate_memory(&alloc, None)}.unwrap();
                unsafe{renderer.device.bind_image_memory(image, mem, 0)}.unwrap();
                //let img_ptr = unsafe{renderer.device.map_memory(mem, 0, req.size, vk::MemoryMapFlags::empty())}.unwrap();
                println!("allocated {} MB GPU memory for image", req.size>>20);
                let subrange = vk::ImageSubresourceRange{ aspect_mask: vk::ImageAspectFlags::COLOR,
                        base_mip_level:0, level_count:1, base_array_layer:0, layer_count:1 };
                let cmd = begin_oneshot_cmd(&renderer);
                {
                    let barrier = [vk::ImageMemoryBarrier::default()
                        .image(image)
                        .old_layout(vk::ImageLayout::UNDEFINED)
                        .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                        .src_access_mask(vk::AccessFlags::NONE)
                        .dst_access_mask(vk::AccessFlags::TRANSFER_READ)
                        .subresource_range(subrange)
                        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    ];
                    unsafe{renderer.device.cmd_pipeline_barrier(cmd,
                        vk::PipelineStageFlags::TOP_OF_PIPE,
                        vk::PipelineStageFlags::TRANSFER,
                        vk::DependencyFlags::BY_REGION,
                        &[], &[], &barrier)};
                }
                end_oneshot_cmd(&renderer, cmd);

                let view_info = vk::ImageViewCreateInfo::default()
                    .image(image)
                    .view_type(vk::ImageViewType::TYPE_2D)
                    .format(vk::Format::R8_UNORM)
                    .subresource_range(subrange);
                let view = unsafe{renderer.device.create_image_view(&view_info, None)}.unwrap();

                let sampler_info = vk::SamplerCreateInfo::default()
                    .mag_filter(vk::Filter::NEAREST)
                    .min_filter(vk::Filter::NEAREST)
                    .border_color(vk::BorderColor::INT_TRANSPARENT_BLACK)
                    .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_BORDER)
                    .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_BORDER)
                    .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_BORDER)
                    .anisotropy_enable(false)
                    .unnormalized_coordinates(true)
                    .compare_enable(false)
                    .compare_op(vk::CompareOp::ALWAYS)
                    .mipmap_mode(vk::SamplerMipmapMode::NEAREST)
                    .mip_lod_bias(0.0)
                    .min_lod(0.0)
                    .max_lod(0.0);
                let sampler = unsafe{renderer.device.create_sampler(&sampler_info, None)}.unwrap();

                // create descriptor set layout
                let set_layout_bindings = [
                    vk::DescriptorSetLayoutBinding::default()
                        .binding(0)
                        .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                        .descriptor_count(1)
                        .stage_flags(vk::ShaderStageFlags::FRAGMENT) ];
                let set_layout_info = vk::DescriptorSetLayoutCreateInfo::default()
                    .bindings(&set_layout_bindings);
                    //.flags(vk::DescriptorSetLayoutCreateFlags::UPDATE_AFTER_BIND_POOL_EXT)
                    //.push_next(&mut binding_flags);
                let set_layouts = [unsafe{renderer.device.create_descriptor_set_layout(&set_layout_info, None)}.unwrap()];
                let descriptor_alloc_info = vk::DescriptorSetAllocateInfo::default()
                    .descriptor_pool(renderer.descriptor_pool)
                    .set_layouts(&set_layouts);
                let descriptor_set = unsafe{renderer.device.allocate_descriptor_sets(&descriptor_alloc_info)}.unwrap()[0];

                let desc_img_info = [
                    vk::DescriptorImageInfo::default()
                    .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                    .image_view(view)
                    .sampler(sampler)
                ];
                let descriptor_writes = [
                    vk::WriteDescriptorSet::default()
                        .dst_set(descriptor_set)
                        .dst_binding(0)
                        .dst_array_element(0)
                        .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                        .descriptor_count(1)
                        .image_info(&desc_img_info)
                ];
                unsafe{renderer.device.update_descriptor_sets(&descriptor_writes, &[])};

                // create pipeline layout
                let pipeline_layout_info = vk::PipelineLayoutCreateInfo::default()
                    .set_layouts(&set_layouts)
                    .push_constant_ranges(&push_constant_ranges);
                let pipeline_layout = unsafe{ renderer.device.create_pipeline_layout(&pipeline_layout_info, None) }.unwrap();
                println!("pipeline layout: {pipeline_layout:?}");

                let (vs,fs) = renderer.load_glsl_vs_fs("shaders/text-renderer.vert.glsl", "shaders/text-renderer.frag.glsl", &push_constant_ranges, &set_layouts);
                let Some((bar_buffer, bar_memory)) = renderer.map_bar_buffer(64<<20,
                    vk::BufferUsageFlags::VERTEX_BUFFER
                  | vk::BufferUsageFlags::INDEX_BUFFER
                  | vk::BufferUsageFlags::TRANSFER_SRC) else {panic!(":(")};
                println!("mem ptr {bar_memory:?}");

                println!("initialized!!");

                let init_end = std::time::Instant::now();
                println!("{:>13?} renderer new", init_render-init_start);
                println!("{:>13?} post renderer", init_end-init_render);
                println!("{:>13?} total init", init_end-init_start);
                *self = App::Resumed{ renderer, vs, fs, bar_buffer, bar_memory, pipeline_layout, descriptor_set, image, text_renderer};
            },
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent){
        match event {
            WindowEvent::CloseRequested => {
                println!("Window closed");
                event_loop.exit()
            },
            WindowEvent::RedrawRequested => {
                let App::Resumed{renderer,vs,fs,bar_buffer,bar_memory, pipeline_layout, descriptor_set, image, text_renderer} = self else { panic!("not active!") };
                println!("================================================================================");
                let winsize = renderer.window.inner_size();
                let win_w = winsize.width as f32;
                let win_h = winsize.height as f32;


                let mut frame = renderer.wait_for_frame();


                let english = new_locale("en", hb::HB_SCRIPT_LATIN, hb::HB_DIRECTION_LTR);
                let mut text = Text::default();

                let style_h1  = Style{ color: Color::WHITE, autohint: false, features: &[], font_idx: 0, size: 48, weight: 300 };
                let style_s1  = Style{ color: Color::WHITE, autohint: false, features: &[], font_idx: 1, size: 21, weight: 400 };
                let style_s2  = Style{ color: Color::WHITE, autohint: false, features: &[], font_idx: 0, size: 16, weight: 400 };
                let style_s2h = Style{ color: Color::WHITE, autohint: true,  features: &[], font_idx: 0, size: 16, weight: 400 };
                let style_s3  = Style{ color: Color::WHITE, autohint: false, features: &[], font_idx: 2, size: 21, weight: 300 };
                let style_h2  = Style{ color: Color::WHITE, autohint: false, features: &[], font_idx: 3, size: 48, weight: 250 };

                let mut cursor = vec2(50,50)*64;
                render_line_of_text(&mut text, text_renderer, &english, &style_h1, cursor, "Hello, World! 48pt");
                cursor.1 += 30*64;
                render_line_of_text(&mut text, text_renderer, &english, &style_s1, cursor, "This is an example of an italic sentence. This is set at 21pts");
                cursor.1 += 30*64;
                render_line_of_text(&mut text, text_renderer, &english, &style_s2, cursor, "Text rendering fidelity is bad at small sizes without sub-pixel positioning. This is 16pts.");
                cursor.1 += 20*64;
                render_line_of_text(&mut text, text_renderer, &english, &style_s2h,cursor, "Text rendering fidelity is bad at small sizes without sub-pixel positioning. This is 16pts.");
                cursor.1 += 30*64;
                render_line_of_text(&mut text, text_renderer, &english, &style_s3, cursor, "Here's a serif font at 21px. I love Crimson Pro, it's a good-looking font.");
                cursor.1 += 50*64;
                render_line_of_text(&mut text, text_renderer, &english, &style_h2, cursor, "And it has absolutely kick-ass italics.");
                text.quads.push(gen_quad(50, 300, text_renderer.glyph_cache.current_x as i16, 50, 0, 0, Color{r:0xFF,g:0xFF,b:0x00,a:0xFF})); // debug: visualize glyph_cache

                // copy text into bar memory
                let mut bar_ptr = *bar_memory;
                let vertex_start = bar_ptr;
                let quad_count = text.quads.len();
                for quads in text.quads {
                    bar_ptr = unsafe{push_type::<[Vertex;4]>(bar_ptr, quads)};
                }
                let index_buffer_offset   = unsafe{bar_ptr.byte_offset_from(*bar_memory)} as u64;
                for i in 0..quad_count {
                    bar_ptr = push_quad_indices(bar_ptr, (i*4) as u16);
                }

                let pixel_buffer_offset   = unsafe{bar_ptr.byte_offset_from(*bar_memory)} as u64;
                for b in text.pixels.iter() {
                    // inefficient?
                    unsafe{core::mem::transmute::<*mut core::ffi::c_void, *mut u8>(bar_ptr).write_volatile(*b);}
                    bar_ptr = unsafe{bar_ptr.byte_add(1)};
                }
                let buffer_end = bar_ptr;

                // add pixel offset to the buffers
                for mut update in &mut text.buffer_updates {
                    update.buffer_offset += pixel_buffer_offset;
                }

                frame.buffer_to_image(*bar_buffer, *image, &text.buffer_updates);

                frame.begin_rendering();
                frame.bind_vs_fs(*vs, *fs);
                frame.bind_vertex_buffer(*bar_buffer);
                frame.bind_index_buffer(*bar_buffer, index_buffer_offset);
                frame.set_vertex_input(core::mem::size_of::<Vertex>() as u32, &[
                    (0, vk::Format::R16G16_SINT),
                    (4, vk::Format::R16G16_UINT),
                    (8, vk::Format::R8G8B8A8_UNORM),
                ]);

                frame.set_color_blend_enable(&[1]);
                frame.set_color_blend_equation(&[
                    vk::ColorBlendEquationEXT::default()
                        .src_color_blend_factor(vk::BlendFactor::SRC_ALPHA)
                        .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
                        .color_blend_op(vk::BlendOp::ADD)
                        .src_alpha_blend_factor(vk::BlendFactor::ONE)
                        .dst_alpha_blend_factor(vk::BlendFactor::ZERO)
                        .alpha_blend_op(vk::BlendOp::ADD)
                ]);
                frame.bind_descriptor_set(*descriptor_set, *pipeline_layout);
                frame.push_constant(*pipeline_layout, &[2.0/win_w, 2.0/win_h, win_w/2.0, win_h/2.0]);
                frame.draw_indexed((quad_count*6) as u32, 0, 0);
            },
            _ => (),
        }
    }
}

fn main() {
    let event_loop = EventLoop::new().unwrap();
    event_loop.set_control_flow(ControlFlow::Wait);

    let mut app = App::default();
    event_loop.run_app(&mut app).unwrap();
}

