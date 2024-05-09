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
     Device, Entry, Instance,
    ext, khr,
    vk::{self, Handle, Image, ImageView, InstanceCreateInfo, CommandPool, CommandBuffer, PhysicalDevice, Queue, ShaderEXT, SurfaceFormatKHR, SurfaceKHR, SwapchainCreateInfoKHR, SwapchainKHR, Semaphore, Fence},
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
    },
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        match self {
            App::Resumed{..}    => todo!("handle re-resuming"),
            App::Uninitialized => {
                let renderer = renderer::Renderer::new(event_loop);
                renderer.debug_print();
                let push_constant_ranges = [
                    vk::PushConstantRange::default()
                        .stage_flags(vk::ShaderStageFlags::VERTEX)
                        .size(core::mem::size_of::<[f32;4]>() as u32) ];
                let binding_flag_bits = [vk::DescriptorBindingFlagsEXT::UPDATE_AFTER_BIND];
                let mut binding_flags = vk::DescriptorSetLayoutBindingFlagsCreateInfoEXT::default()
                    .binding_flags(&binding_flag_bits);

                // create descriptor set layout
                let set_layout_bindings = [
                    vk::DescriptorSetLayoutBinding::default()
                        .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                        .descriptor_count(1)
                        .stage_flags(vk::ShaderStageFlags::VERTEX) ];
                let set_layout_info = vk::DescriptorSetLayoutCreateInfo::default()
                    .bindings(&set_layout_bindings)
                    .flags(vk::DescriptorSetLayoutCreateFlags::UPDATE_AFTER_BIND_POOL_EXT)
                    .push_next(&mut binding_flags);
                let set_layouts = [unsafe{renderer.device.create_descriptor_set_layout(&set_layout_info, None)}.unwrap()];
                let descriptor_alloc_info = vk::DescriptorSetAllocateInfo::default()
                    .descriptor_pool(renderer.descriptor_pool)
                    .set_layouts(&set_layouts);
                let descriptor_set = unsafe{renderer.device.allocate_descriptor_sets(&descriptor_alloc_info)}.unwrap()[0];
                

                // create pipeline layout
                let pipeline_layout_info = vk::PipelineLayoutCreateInfo::default()
                    //.set_layouts(&set_layouts)
                    .push_constant_ranges(&push_constant_ranges);
                let pipeline_layout = unsafe{ renderer.device.create_pipeline_layout(&pipeline_layout_info, None) }.unwrap();
                println!("pipeline layout: {pipeline_layout:?}");

                let (vs,fs) = renderer.load_shader_vs_fs("in_triangle.vert.spv", "triangle.frag.spv", &push_constant_ranges, &[]);
                let Some((bar_buffer, bar_memory)) = renderer.map_bar_buffer(128<<20, vk::BufferUsageFlags::VERTEX_BUFFER|vk::BufferUsageFlags::INDEX_BUFFER) else {panic!(":(")};
                println!("mem ptr {bar_memory:?}");

                println!("initialized!!");
                *self = App::Resumed{ renderer, vs, fs, bar_buffer, bar_memory, pipeline_layout, descriptor_set};
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
                let App::Resumed{renderer,vs,fs,bar_buffer,bar_memory, pipeline_layout, descriptor_set} = self else { panic!("not active!") };
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

                let mut frame = renderer.new_frame();

                let vertex_memory = unsafe{core::mem::transmute::<*mut core::ffi::c_void, *mut _>(*bar_memory)};
                unsafe{core::ptr::write_volatile(vertex_memory, [
                    Vertex{x:10,   y:10,  u:0, v:0, r:0xFF, g:0x00, b:0x00, a:0xFF},
                    Vertex{x:10,   y:266, u:0, v:0, r:0x00, g:0xFF, b:0x00, a:0xFF},
                    Vertex{x:266,  y:266, u:0, v:0, r:0x00, g:0x00, b:0xFF, a:0xFF},

                    Vertex{x:266,  y:10,  u:0, v:0, r:0xFF, g:0xFF, b:0x00, a:0xFF},
                    Vertex{x:266,  y:266, u:0, v:0, r:0x00, g:0x00, b:0xFF, a:0xFF},
                    Vertex{x:10,   y:10,  u:0, v:0, r:0xFF, g:0x00, b:0x00, a:0xFF},
                ])};

                //let idx_memory = unsafe{core::mem::transmute::<*mut core::ffi::c_void, *mut _>((*bar_memory).byte_offset(4*12))};
                //unsafe{core::ptr::write_volatile(idx_memory, [
                //    0u16, 1, 2, 2, 1, 0,
                //])};

                frame.bind_vs_fs(*vs, *fs);
                frame.bind_vertex_buffer(*bar_buffer);
                //frame.bind_index_buffer(*bar_buffer, 4*12);
                frame.set_vertex_input(core::mem::size_of::<Vertex>() as u32, &[
                    (0, vk::Format::R16G16_UINT),
                    (4, vk::Format::R16G16_UINT),
                    (8, vk::Format::R8G8B8A8_UNORM),
                ]);

                //frame.bind_descriptor_set(*descriptor_set, *pipeline_layout);
                frame.push_constant(*pipeline_layout, &[2.0/win_w, 2.0/win_h, win_w/2.0, win_h/2.0]);
                frame.draw(6,0);
                //frame.draw_indexed(6, 2*12, -4*12);


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

