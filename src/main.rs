#![allow(unused)]
mod renderer;

//use harfbuzz as hb;
//use freetype as ft;
use std::{
    collections::HashSet, ffi::{CStr,OsStr}, fmt, mem::size_of, ptr
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
        image : vk::Image
    },
}

// render strategy
// - single queue
// - one BAR memory chunk
//   - buffer_to_img
//   - vertices
//   - indices
// - one gpu memory texture

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

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        match self {
            App::Resumed{..}    => todo!("handle re-resuming"),
            App::Uninitialized => {
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

                let (vs,fs) = renderer.load_shader_vs_fs("in_triangle.vert.spv", "image_triangle.frag.spv", &push_constant_ranges, &set_layouts);
                let Some((bar_buffer, bar_memory)) = renderer.map_bar_buffer(128<<20,
                    vk::BufferUsageFlags::VERTEX_BUFFER
                  | vk::BufferUsageFlags::INDEX_BUFFER
                  | vk::BufferUsageFlags::TRANSFER_SRC) else {panic!(":(")};
                println!("mem ptr {bar_memory:?}");

                println!("initialized!!");

                let init_end = std::time::Instant::now();
                println!("{:>13?} renderer new", init_render-init_start);
                println!("{:>13?} post renderer", init_end-init_render);
                println!("{:>13?} total init", init_end-init_start);
                *self = App::Resumed{ renderer, vs, fs, bar_buffer, bar_memory, pipeline_layout, descriptor_set, image};
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
                let App::Resumed{renderer,vs,fs,bar_buffer,bar_memory, pipeline_layout, descriptor_set, image} = self else { panic!("not active!") };
                println!("================================================================================");
                let winsize = renderer.window.inner_size();
                let win_w = winsize.width as f32;
                let win_h = winsize.height as f32;

                #[repr(packed)]
                struct Vertex{
                    x:u16, y:u16,
                    u:u16, v:u16,
                    r:u8, g:u8, b:u8, a:u8
                }

                let mut frame = renderer.wait_for_frame();

                let vertex_memory = unsafe{core::mem::transmute::<*mut core::ffi::c_void, *mut _>(*bar_memory)};
                unsafe{core::ptr::write_volatile(vertex_memory, [
                    // top left
                    Vertex{x:10,   y:10,  u:0, v:0, r:0xFF, g:0x00, b:0x00, a:0xFF},

                    // bottom left
                    Vertex{x:10,   y:265, u:0, v:4, r:0x00, g:0xFF, b:0x00, a:0xFF},

                    // top right
                    Vertex{x:265,  y:10,  u:4, v:0, r:0xFF, g:0xFF, b:0x00, a:0xFF},

                    // bottom right
                    Vertex{x:265,  y:265, u:4, v:4, r:0x00, g:0x00, b:0xFF, a:0xFF},
                ])};

                let vertex_count = 4 as u32;
                let vertex_size  = size_of::<Vertex>() as u32;

                let idx_memory = unsafe{core::mem::transmute::<*mut core::ffi::c_void, *mut [u16;6]>((*bar_memory).byte_offset((vertex_count*vertex_size) as isize))};
                unsafe{core::ptr::write_volatile(idx_memory, [
                    0, 1, 2,
                    2, 1, 3,
                ])};

                let image_memory = unsafe{core::mem::transmute::<*mut core::ffi::c_void, *mut [u8;16]>((*bar_memory).byte_offset((vertex_count*vertex_size+6*2) as isize))};
                unsafe{core::ptr::write_volatile(image_memory, [
                    0x88, 0x88, 0xFF, 0xFF,
                    0x88, 0x88, 0xFF, 0xFF,
                    0x00, 0x00, 0xFF, 0xFF,
                    0x00, 0x00, 0xFF, 0xFF,
                ])};
                frame.buffer_to_image(*bar_buffer, *image, &[
                    vk::BufferImageCopy{
                        buffer_offset: (vertex_count*vertex_size+6*2) as u64,
                        buffer_row_length: 0,
                        buffer_image_height: 0,
                        image_offset: vk::Offset3D{x:0,y:0,z:0},
                        image_extent: vk::Extent3D{width:4,height:4,depth:1},
                        image_subresource: vk::ImageSubresourceLayers{
                            layer_count: 1,
                            aspect_mask: vk::ImageAspectFlags::COLOR,
                            base_array_layer: 0,
                            mip_level: 0
                        }
                    }
                ]);

                frame.begin_rendering();
                frame.bind_vs_fs(*vs, *fs);
                frame.bind_vertex_buffer(*bar_buffer);
                frame.bind_index_buffer(*bar_buffer, 4*12);
                frame.set_vertex_input(core::mem::size_of::<Vertex>() as u32, &[
                    (0, vk::Format::R16G16_UINT),
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
                //frame.draw(6,0);
                frame.draw_indexed(6, 0, 0);
            },
            _ => (),
        }
    }
}

fn main() {
    //let mut buf = hb::Buffer::with("Hello World!");
    //buf.set_direction(hb::Direction::LTR);
    //buf.set_script(hb::sys::HB_SCRIPT_LATIN);
    //let lib = ft::Library::init().expect("failed to initialize freetype");
    //let face = lib.new_face("./source-sans/SourceSans3-Regular.ttf", 0).expect("could not find font");

    let event_loop = EventLoop::new().unwrap();
    event_loop.set_control_flow(ControlFlow::Wait);

    let mut app = App::default();
    event_loop.run_app(&mut app).unwrap();
}

