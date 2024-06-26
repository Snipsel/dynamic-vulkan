use common::*;
use text_engine::*;

use core::{ mem::{transmute,size_of}, ptr::write_volatile, ffi::c_void };
use ash::vk;
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    raw_window_handle::{HasDisplayHandle, HasWindowHandle},
    window::{Window,WindowId}
};

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

unsafe fn push_type<T>(ptr:*mut c_void, object:T) -> *mut c_void {
    let vertex_memory = unsafe{transmute::<*mut c_void, *mut T>(ptr)};
    unsafe{write_volatile(vertex_memory, object)};
    let t_size = size_of::<T>() as isize;
    unsafe{ptr.byte_offset(t_size)}
}

fn push_quad_indices(ptr:*mut c_void, i:u16) -> *mut c_void {
    let indices = [ i+0, i+1, i+2, i+2, i+1, i+3 ];
    unsafe{push_type::<[u16;6]>(ptr, indices)}
}

#[derive(Default)]
enum App{
    #[default] Uninitialized,
    Resumed{
        window: Window,
        renderer: renderer::Renderer,
        vs : vk::ShaderEXT,
        fs : vk::ShaderEXT,
        bar_buffer : vk::Buffer,
        bar_memory : *mut c_void,
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
                use std::time::Instant;
                let init_start = Instant::now();

                let glyph_cache_size = 1<<10;
                let glyph_cache_format = vk::Format::R8G8B8A8_UNORM;

                let text_engine = TextEngine::new(glyph_cache_size, &[
                    "./fonts/source-sans/upright.ttf",
                    "./fonts/source-sans/italic.ttf",
                    "./fonts/crimson-pro/upright.ttf",
                    "./fonts/crimson-pro/italic.ttf",
                ]);
                let init_text_engine = Instant::now();

                let window = event_loop.create_window(Window::default_attributes()).expect("could not create window");
                let raw_window  = window.window_handle().unwrap().as_raw();
                let raw_display = window.display_handle().unwrap().as_raw();
                let renderer = renderer::Renderer::new(raw_window, raw_display);
                let init_render = Instant::now();

                renderer.debug_print();
                let push_constant_ranges = [
                    vk::PushConstantRange::default()
                        .stage_flags(vk::ShaderStageFlags::VERTEX)
                        .size(size_of::<[f32;4]>() as u32) ];
                //let binding_flag_bits = [vk::DescriptorBindingFlagsEXT::UPDATE_AFTER_BIND];
                //let mut binding_flags = vk::DescriptorSetLayoutBindingFlagsCreateInfoEXT::default()
                //    .binding_flags(&binding_flag_bits);

                // create texture image
                let (image,view) = renderer.alloc_image_and_view(glyph_cache_size as u32, glyph_cache_size as u32, glyph_cache_format);
                {
                    let cmd = renderer.begin_oneshot_cmd();
                    renderer.transition_image(cmd, image, vk::ImageLayout::UNDEFINED, vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);
                    renderer.end_oneshot_cmd(cmd);
                }
                let sampler = renderer.new_sampler_nearest();

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

                let init_end = Instant::now();
                println!("{:>13?} text engine",   init_text_engine-init_start);
                println!("{:>13?} renderer new",  init_render-init_text_engine);
                println!("{:>13?} post renderer", init_end-init_render);
                println!("{:>13?} total init",    init_end-init_start);
                *self = App::Resumed{ window, renderer, vs, fs, bar_buffer, bar_memory, pipeline_layout, descriptor_set, image, text_engine};
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
                let App::Resumed{window, renderer,vs,fs,bar_buffer,bar_memory, pipeline_layout, descriptor_set, image, text_engine} = self else { panic!("not active!") };
                println!("================================================================================");
                let winsize = window.inner_size();
                let win_w = winsize.width as f32;
                let win_h = winsize.height as f32;

                let english = Locale::new("en", Script::LATIN, Direction::LeftToRight);

                let gb_light = Color::srgb8(0xF2, 0xe5, 0xbc, 0xFF);
                let gb_aqua  = Color::srgb8(0x8e, 0xc0, 0x7c, 0xFF);
                let gb_red   = Color::srgb8(0xfb, 0x49, 0x34, 0xFF);
                let gb_yellow= Color::srgb8(0xfa, 0xbd, 0x2f, 0xFF);
                let color = gb_aqua;
                let features = &[];
                let subpixel = 4;

                let style_h1  = Style{ features, color:gb_light,  subpixel,   autohint: false, font_idx: 0, size: 32, weight: 600 };
                //let style_h2  = Style{ features, color:gb_light,  subpixel,   autohint: false, font_idx: 0, size: 18, weight: 300 };

                let style_s0  = Style{ features, color,           subpixel,   autohint: false, font_idx: 0, size: 21, weight: 400 };
                let style_s1  = Style{ features, color,           subpixel,   autohint: false, font_idx: 1, size: 21, weight: 400 };
                let style_s2  = Style{ features, color:gb_red,    subpixel,   autohint: false, font_idx: 0, size: 12, weight: 400 };
                let style_s3  = Style{ features, color:gb_yellow, subpixel,   autohint: false, font_idx: 2, size: 21, weight: 300 };
                let style_s3b = Style{ features, color:gb_yellow, subpixel,   autohint: false, font_idx: 2, size: 21, weight: 700 };
                let style_s4  = Style{ features, color:gb_light,  subpixel,   autohint: false, font_idx: 3, size: 18, weight: 250 };
                let style_s5  = Style{ features, color:gb_light,  subpixel,   autohint: false, font_idx: 0, size: 18, weight: 400 };

                let line_width = 600.0;
                let mut cursor = vec2(50,100);
                let cursor_s = cursor;

                let mut h1 = text_engine::StyledParagraph::default();
                h1.add(&english, &style_h1, "How do you spell Пётр Кропоткин?");
                let mut text  = text_engine.render_paragraph(&mut cursor, line_width, 0.5, &h1);

                let mut p1 = text_engine::StyledParagraph::default();
                p1.add(&english, &style_s0,  "Hållo, World! ");
                p1.add(&english, &style_s2,  "There was no cleavage between the man and his world. He spoke and acted in all things as he felt and believed and wrote. Kropotkin was a whole man. ");
                p1.add(&english, &style_s1,  "The economic change which will result from the Social Revolution will be so immense and so profound, it must so change all the relations based today on property and exchange, that it is impossible for one or any individual to elaborate the different social forms, which must spring up in the society of the future. ");
                p1.add(&english, &style_s3,  "Here's a serif font. ");
                p1.add(&english, &style_s4,  "wololo");
                text.append(text_engine.render_paragraph(&mut cursor, line_width, 1.5, &p1));

                let mut p2 = text_engine::StyledParagraph::default();
                p2.add(&english, &style_s3,  "A printable character results in output when rendered, but a whitespace character does not. Instead, whitespace characters define the layout of text to a limited degree – interrupting the normal sequence of ");
                p2.add(&english, &style_s3b, "rendering characters");
                p2.add(&english, &style_s3, " next to each other. The output of subsequent characters is typically shifted to the right (or to the left for right-to-left script) or to the start of the next line. The effect of multiple sequential whitespace characters is cumulative such that the next printable character is rendered in a location based on the accumulated effect of preceding whitespace characters. ");
                text.append(text_engine.render_paragraph(&mut cursor, line_width, 1.5, &p2));

                let mut p3 = text_engine::StyledParagraph::default();
                p3.add(&english, &style_s5,  "Whenever we make a new library project with Cargo, a test module with a test function in it is automatically generated for us. This module gives you a template for writing your tests so you don’t have to look up the exact structure and syntax every time you start a new project. You can add as many additional test functions and as many test modules as you want!");
                text.append(text_engine.render_paragraph(&mut cursor, line_width, 1.5, &p3));


                println!("{cursor_s} -> {cursor}");
                //text.quads.insert(0, gen_rect(cursor_s, vec2(line_width, cursor.y-cursor_s.y), Color::srgb8(16, 16, 16, 0xFF)) );

                // top left
                text.draw_hook_top_left    (cursor_s,                    Color::srgb8(0xFF, 0xFF, 0, 0xFF));
                text.draw_hook_top_right   (cursor_s+vec2(line_width,0), Color::srgb8(0xFF, 0xFF, 0, 0xFF));
                text.draw_hook_bottom_left (cursor,                      Color::srgb8(0xFF, 0xFF, 0, 0xFF));
                text.draw_hook_bottom_right(cursor  +vec2(line_width,0), Color::srgb8(0xFF, 0xFF, 0, 0xFF));

                let mut frame = renderer.wait_and_begin_frame();

                // copy text into bar memory
                let mut bar_ptr = *bar_memory;
                let _vertex_start = bar_ptr;
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
                    unsafe{transmute::<*mut c_void, *mut u8>(bar_ptr).write_volatile(*b);}
                    bar_ptr = unsafe{bar_ptr.byte_add(1)};
                }
                let _buffer_end = bar_ptr;

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
                frame.set_vertex_input(size_of::<Vertex>() as u32, &[
                    (0, vk::Format::R16G16_SINT),
                    (4, vk::Format::R16G16_UINT),
                    (8, vk::Format::R8G8B8A8_UNORM),
                ]);

                frame.set_color_blend_enable(&[1]);
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

                frame.end_rendering();
                if !frame.end_frame() {
                    window.request_redraw();
                }
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

