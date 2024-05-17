#![allow(unused)]
mod common;
use common::{Color,Vertex,vec2,div_round};
mod renderer;
mod text_engine;
use text_engine::*;

use std::{
    collections::HashMap, ffi::{CStr,OsStr}, fmt, mem::size_of, ptr
};
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

fn gen_buffer_image_copy(ptr_offset:u64, buffer_image_copy: BufferImageCopy) -> vk::BufferImageCopy {
    let BufferImageCopy { buffer_offset, width, height, u, v } = buffer_image_copy;
    vk::BufferImageCopy{
        buffer_offset: buffer_offset+ptr_offset,
        buffer_row_length: 0,
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
        text_engine : TextEngine,
    },
}
impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        match self {
            App::Resumed{..}    => todo!("handle re-resuming"),
            App::Uninitialized => {
                let glyph_cache_size = 1<<10;
                let glyph_cache_format = vk::Format::R8G8B8A8_UNORM;

                let text_engine = TextEngine::new(glyph_cache_size, &[
                    "./fonts/source-sans/upright.ttf",
                    "./fonts/source-sans/italic.ttf",
                    "./fonts/crimson-pro/upright.ttf",
                    "./fonts/crimson-pro/italic.ttf",
                ]);

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
                    .extent(vk::Extent3D{width:glyph_cache_size as u32, height:glyph_cache_size as u32, depth:1})
                    .mip_levels(1)
                    .array_layers(1)
                    .format(glyph_cache_format)
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
                println!("allocated {} MB GPU memory for {glyph_cache_size}x{glyph_cache_size} glyph cache image", req.size>>20);
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
                    .format(glyph_cache_format)
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

                //let (vs,fs) = renderer.load_glsl_vs_fs("shaders/text-renderer.vert.glsl", "shaders/text-renderer.frag.glsl", &push_constant_ranges, &set_layouts);
                let (vs,fs) = renderer.load_glsl_vs_fs("shaders/text-renderer.vert.glsl", "shaders/subpixel.frag.glsl", &push_constant_ranges, &set_layouts);
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
                *self = App::Resumed{ renderer, vs, fs, bar_buffer, bar_memory, pipeline_layout, descriptor_set, image, text_engine};
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
                let App::Resumed{renderer,vs,fs,bar_buffer,bar_memory, pipeline_layout, descriptor_set, image, text_engine} = self else { panic!("not active!") };
                println!("================================================================================");
                let winsize = renderer.window.inner_size();
                let win_w = winsize.width as f32;
                let win_h = winsize.height as f32;


                let mut frame = renderer.wait_for_frame();


                let english = Locale::new("en", Script::LATIN, Direction::LeftToRight);
                let mut text = Text::default();

                let gb_light = Color::srgb8(0xF2, 0xe5, 0xbc, 0xFF);
                let gb_aqua  = Color::srgb8(0x8e, 0xc0, 0x7c, 0xFF);
                let gb_red   = Color::srgb8(0xfb, 0x49, 0x34, 0xFF);
                let gb_yellow= Color::srgb8(0xfa, 0xbd, 0x2f, 0xFF);
                let color = gb_aqua;
                let features = &[];
                let subpixel = 8;
                let style_h1  = Style{ features, color:gb_light, subpixel,   autohint: false, font_idx: 0, size: 48, weight: 300 };
                let style_s1  = Style{ features, color, subpixel,   autohint: false, font_idx: 1, size: 21, weight: 400 };

                let style_s2  = Style{ features, color:gb_red, subpixel,   autohint: false, font_idx: 0, size: 12, weight: 400 };
                let style_s2s = Style{ features, color:gb_red, subpixel:1, autohint: false, font_idx: 0, size: 12, weight: 400 };
                let style_s2h = Style{ features, color:gb_red, subpixel,   autohint: true,  font_idx: 0, size: 12, weight: 400 };

                let style_s3  = Style{ features, color:gb_yellow, subpixel,   autohint: false, font_idx: 2, size: 21, weight: 300 };
                let style_h2  = Style{ features, color:gb_light, subpixel,   autohint: false, font_idx: 3, size: 48, weight: 250 };

                let mut cursor = vec2(50,50)*64;
                text_engine.render_line_of_text(&mut text, &english, &style_h1, cursor, "Hello, World! 48pt");
                cursor.1 += 30*64;
                text_engine.render_line_of_text(&mut text, &english, &style_s1, cursor, "This is an example of an italic sentence. This is set at 21pts");
                cursor.1 += 30*64;

                text_engine.render_line_of_text(&mut text, &english, &style_s2h,cursor, "Text rendering fidelity is bad at small sizes without sub-pixel positioning. This is 12pts. A");
                cursor.1 += 20*64;
                text_engine.render_line_of_text(&mut text, &english, &style_s2, cursor, "Text rendering fidelity is bad at small sizes without sub-pixel positioning. This is 12pts. B");
                cursor.1 += 20*64;
                text_engine.render_line_of_text(&mut text, &english, &style_s2s,cursor, "Text rendering fidelity is bad at small sizes without sub-pixel positioning. This is 12pts. C");
                cursor.1 += 30*64;

                text_engine.render_line_of_text(&mut text, &english, &style_s3, cursor, "Here's a serif font at 21px. I love Crimson Pro, it's a good-looking font.");
                cursor.1 += 50*64;
                text_engine.render_line_of_text(&mut text, &english, &style_h2, cursor, "And it has absolutely kick-ass italics.");
                cursor.1 += 20*64;
                //text.quads.push(gen_quad(50, (cursor.1/64) as i16, text_engine.glyph_cache.current_x as i16, 50, 0, 0, gb_yellow)); // debug: visualize glyph_cache

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

                // align pixel buffer
                bar_ptr = unsafe{bar_ptr.byte_offset(bar_ptr.byte_offset_from(*bar_memory)%4)};

                let pixel_buffer_offset = unsafe{bar_ptr.byte_offset_from(*bar_memory)} as u64;
                println!("pixel_buffer_offset: {pixel_buffer_offset}");
                assert_eq!(pixel_buffer_offset%4,0);
                for b in text.pixels.iter() {
                    // inefficient?
                    unsafe{core::mem::transmute::<*mut core::ffi::c_void, *mut u8>(bar_ptr).write_volatile(*b);}
                    bar_ptr = unsafe{bar_ptr.byte_add(1)};
                }
                let buffer_end = bar_ptr;

                // add pixel offset to the buffers
                let buffer_updates :Vec<vk::BufferImageCopy> = text.buffer_updates.into_iter().map(move|buffer_image_copy|gen_buffer_image_copy(pixel_buffer_offset,buffer_image_copy)).collect();

                frame.buffer_to_image(*bar_buffer, *image, &buffer_updates);

                frame.begin_rendering([(0x32 as f32/0xFF as f32).powf(2.2),
                                       (0x30 as f32/0xFF as f32).powf(2.2),
                                       (0x2f as f32/0xFF as f32).powf(2.2),
                                       1.0]);
                frame.bind_vs_fs(*vs, *fs);
                frame.bind_vertex_buffer(*bar_buffer);
                frame.bind_index_buffer(*bar_buffer, index_buffer_offset);
                frame.set_vertex_input(core::mem::size_of::<Vertex>() as u32, &[
                    (0, vk::Format::R16G16_SINT),
                    (4, vk::Format::R16G16_UINT),
                    (8, vk::Format::R8G8B8A8_UNORM),
                ]);

                frame.set_color_blend_enable(&[1]);
                /* frame.set_color_blend_equation(&[
                    vk::ColorBlendEquationEXT::default()
                        .src_color_blend_factor(vk::BlendFactor::SRC_ALPHA)
                        .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
                        .color_blend_op(vk::BlendOp::ADD)
                        .src_alpha_blend_factor(vk::BlendFactor::ONE)
                        .dst_alpha_blend_factor(vk::BlendFactor::ZERO)
                        .alpha_blend_op(vk::BlendOp::ADD)
                ]); */

                // component-alpha blending
                frame.set_color_blend_equation(&[
                    vk::ColorBlendEquationEXT::default()
                        .src_color_blend_factor(vk::BlendFactor::SRC1_COLOR)
                        .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC1_COLOR)
                        .color_blend_op(vk::BlendOp::ADD)
                        // ignore alpha component
                        .src_alpha_blend_factor(vk::BlendFactor::ZERO)
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

