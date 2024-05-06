#![allow(unused)]

mod renderer;

//use harfbuzz as hb;
//use freetype as ft;
use std::{
    collections::HashSet, 
    fmt,
    ffi::{CStr,OsStr}
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
    },
}

struct Frame<'a>{
    renderer : &'a renderer::Renderer,
    swap_idx : u32,
    dynamic_state_flags : DynamicStateFlags,
}

impl<'a> Frame<'a> {
    pub fn begin(renderer: &'a renderer::Renderer) -> Self {
        // begin frame
        //unsafe{renderer.device.wait_for_fences(&[renderer.render_finished], true, u64::MAX)};
        //unsafe{renderer.device.reset_fences(&[renderer.render_finished])};
        let (swap_idx,_) = unsafe{renderer.khr_swapchain.acquire_next_image(renderer.swapchain, u64::MAX, renderer.image_available, Fence::null())}.unwrap();

        // begin command buffer
        unsafe{renderer.device.reset_command_buffer(renderer.command_buffer, vk::CommandBufferResetFlags::empty())};
        let begin_info = vk::CommandBufferBeginInfo::default();
        unsafe{renderer.device.begin_command_buffer(renderer.command_buffer, &begin_info)}.unwrap();

        // begin frame
        let image_memory_barriers = [
            vk::ImageMemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .image(renderer.swapchain_images[swap_idx as usize])
                .subresource_range(vk::ImageSubresourceRange::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .base_mip_level(0)
                    .level_count(1)
                    .base_array_layer(0)
                    .layer_count(1))
        ];
        unsafe{renderer.device.cmd_pipeline_barrier(renderer.command_buffer,
            vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT, // (changed from TOP_OF_PIPE)
            vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
            vk::DependencyFlags::empty(),
            &[], &[], &image_memory_barriers)};


        // begin rendering
        let mut clear_color_value = vk::ClearColorValue::default();
        clear_color_value.float32 = [0.0, 0.5, 0.5, 1.0];
        let mut clear_color = vk::ClearValue::default();
        clear_color.color = clear_color_value;
        let color_attachments = [
            vk::RenderingAttachmentInfo::default()
                .image_view(renderer.swapchain_views[swap_idx as usize])
                .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .resolve_image_layout(vk::ImageLayout::PRESENT_SRC_KHR)
                .load_op(vk::AttachmentLoadOp::CLEAR)
                .store_op(vk::AttachmentStoreOp::STORE)
                .clear_value(clear_color),
        ];
        let rendering_info = vk::RenderingInfo::default()
            .render_area(renderer.swapchain_extent.into())
            .layer_count(1)
            .color_attachments(&color_attachments);
        unsafe{renderer.khr_dynamic_rendering.cmd_begin_rendering(renderer.command_buffer, &rendering_info)};

        let dynamic_state_flags = DynamicStateFlags::empty();

        Self{ renderer, swap_idx, dynamic_state_flags}
    }

    pub fn end(&self) {
        let renderer = self.renderer;
        let swap_idx = self.swap_idx;
        // end rendering
        unsafe{renderer.khr_dynamic_rendering.cmd_end_rendering(renderer.command_buffer)};

        // end frame
        let image_memory_barriers = [
            vk::ImageMemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
                .old_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .new_layout(vk::ImageLayout::PRESENT_SRC_KHR)
                .image(renderer.swapchain_images[swap_idx as usize])
                .subresource_range(vk::ImageSubresourceRange::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .base_mip_level(0)
                    .level_count(1)
                    .base_array_layer(0)
                    .layer_count(1))
        ];
        unsafe{renderer.device.cmd_pipeline_barrier(renderer.command_buffer,
            vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
            vk::PipelineStageFlags::BOTTOM_OF_PIPE,
            vk::DependencyFlags::empty(),
            &[], &[], &image_memory_barriers)};


        // end command buffer
        unsafe{renderer.device.end_command_buffer(renderer.command_buffer)}.unwrap();

        // submit queue
        let wait_semaphores = [renderer.image_available];
        let wait_stages = [vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT];
        let command_buffers = [renderer.command_buffer];
        let signal_semaphores = [renderer.render_finished];
        let submit_info = vk::SubmitInfo::default()
            .wait_semaphores(&wait_semaphores)
            .wait_dst_stage_mask(&wait_stages)
            .command_buffers(&command_buffers)
            .signal_semaphores(&signal_semaphores);
        unsafe{renderer.device.queue_submit(renderer.queue, &[submit_info], vk::Fence::null())}.unwrap();

        let swapchains = [renderer.swapchain];
        let image_indices = [swap_idx];
        let present_info = vk::PresentInfoKHR::default()
            .wait_semaphores(&signal_semaphores)
            .swapchains(&swapchains)
            .image_indices(&image_indices);
        unsafe{renderer.khr_swapchain.queue_present(renderer.queue, &present_info)}.unwrap();
    }

    pub fn set_viewports(&mut self, viewports : &[vk::Viewport]){
        self.dynamic_state_flags |= DynamicStateFlags::VIEWPORTS;
        unsafe{self.renderer.ext_shader_object.cmd_set_viewport_with_count(self.renderer.command_buffer, &viewports)};
    }
    pub fn set_scissors(&mut self, scissors : &[vk::Rect2D]){
        self.dynamic_state_flags |= DynamicStateFlags::SCISSORS;
        unsafe{self.renderer.ext_shader_object.cmd_set_scissor_with_count(self.renderer.command_buffer, &scissors)};
    }
    pub fn set_polygon_mode(&mut self, mode : vk::PolygonMode){
        self.dynamic_state_flags |= DynamicStateFlags::POLYGON_MODE;
        unsafe{self.renderer.ext_shader_object.cmd_set_polygon_mode(self.renderer.command_buffer, mode)};
    }
    pub fn set_primitive_topology(&mut self, topology : vk::PrimitiveTopology){
        self.dynamic_state_flags |= DynamicStateFlags::PRIMITIVE_TOPOLOGY;
        unsafe{self.renderer.ext_shader_object.cmd_set_primitive_topology(self.renderer.command_buffer, topology)};
    }
    pub fn set_primitive_restart_enable(&mut self, enabled: bool){
        self.dynamic_state_flags |= DynamicStateFlags::PRIMITIVE_RESTART_ENABLE;
        unsafe{self.renderer.ext_shader_object.cmd_set_primitive_restart_enable(self.renderer.command_buffer, enabled)};
    }
    pub fn set_depth_test_enable(&mut self, enabled: bool){
        self.dynamic_state_flags |= DynamicStateFlags::DEPTH_TEST_ENABLE;
        unsafe{self.renderer.ext_shader_object.cmd_set_depth_test_enable(self.renderer.command_buffer, enabled)};
    }
    pub fn set_depth_write_enable(&mut self, enabled: bool){
        self.dynamic_state_flags |= DynamicStateFlags::DEPTH_WRITE_ENABLE;
        unsafe{self.renderer.ext_shader_object.cmd_set_depth_write_enable(self.renderer.command_buffer, enabled)};
    }
    pub fn set_depth_bias_enable(&mut self, enabled: bool){
        self.dynamic_state_flags |= DynamicStateFlags::DEPTH_BIAS_ENABLE;
        unsafe{self.renderer.ext_shader_object.cmd_set_depth_bias_enable(self.renderer.command_buffer, enabled)};
    }
    pub fn set_stencil_test_enable(&mut self, enabled: bool){
        self.dynamic_state_flags |= DynamicStateFlags::STENCIL_TEST_ENABLE;
        unsafe{self.renderer.ext_shader_object.cmd_set_stencil_test_enable(self.renderer.command_buffer, enabled)};
    }
    pub fn set_rasterizer_discard_enable(&mut self, enabled: bool){
        self.dynamic_state_flags |= DynamicStateFlags::RASTERIZER_DISCARD_ENABLE;
        unsafe{self.renderer.ext_shader_object.cmd_set_rasterizer_discard_enable(self.renderer.command_buffer, enabled)};
    }
    pub fn set_rasterization_samples(&mut self, sample_count_flags: vk::SampleCountFlags){
        self.dynamic_state_flags |= DynamicStateFlags::RASTERIZATION_SAMPLES;
        unsafe{self.renderer.ext_shader_object.cmd_set_rasterization_samples(self.renderer.command_buffer, sample_count_flags)};
    }
    pub fn set_sample_mask(&mut self, samples: vk::SampleCountFlags, sample_mask: &[vk::SampleMask]){
        self.dynamic_state_flags |= DynamicStateFlags::SAMPLE_MASK;
        unsafe{self.renderer.ext_shader_object.cmd_set_sample_mask(self.renderer.command_buffer, samples, sample_mask)};
    }
    pub fn set_alpha_to_coverage_enable(&mut self, enable: bool){
        unsafe{self.renderer.ext_shader_object.cmd_set_alpha_to_coverage_enable(self.renderer.command_buffer, enable)};
    }
    pub fn set_cull_mode(&mut self, cullmode: vk::CullModeFlags){
        unsafe{self.renderer.ext_shader_object.cmd_set_cull_mode(self.renderer.command_buffer, cullmode)};
    }
    pub fn set_color_blend_enable(&mut self, enables: &[u32]){
        unsafe{self.renderer.ext_shader_object.cmd_set_color_blend_enable(self.renderer.command_buffer, 0, &enables)};
    }
    pub fn set_color_blend_equation(&mut self, equations: &[vk::ColorBlendEquationEXT]){
        unsafe{self.renderer.ext_shader_object.cmd_set_color_blend_equation(self.renderer.command_buffer, 0, &equations)};
    }
    pub fn set_color_write_mask(&mut self, write_masks: &[vk::ColorComponentFlags]){
        unsafe{self.renderer.ext_shader_object.cmd_set_color_write_mask(self.renderer.command_buffer, 0, &write_masks)};
    }

    fn apply_unset_defaults(&mut self){
        let default_viewport : vk::Viewport = 
            vk::Viewport::default()
                .x(0.0).y(0.0).min_depth(0.0).max_depth(1.0)
                .width(self.renderer.swapchain_extent.width as f32)
                .height(self.renderer.swapchain_extent.height as f32);
        
        if !self.dynamic_state_flags.contains(DynamicStateFlags::VIEWPORTS                ){ self.set_viewports(&[default_viewport]); }
        if !self.dynamic_state_flags.contains(DynamicStateFlags::SCISSORS                 ){ self.set_scissors(&[self.renderer.swapchain_extent.into()]); }
        if !self.dynamic_state_flags.contains(DynamicStateFlags::POLYGON_MODE             ){ self.set_polygon_mode(vk::PolygonMode::FILL); }
        if !self.dynamic_state_flags.contains(DynamicStateFlags::PRIMITIVE_TOPOLOGY       ){ self.set_primitive_topology(vk::PrimitiveTopology::TRIANGLE_LIST); }
        if !self.dynamic_state_flags.contains(DynamicStateFlags::PRIMITIVE_RESTART_ENABLE ){ self.set_primitive_restart_enable(false); }
        if !self.dynamic_state_flags.contains(DynamicStateFlags::DEPTH_TEST_ENABLE        ){ self.set_depth_test_enable(false); }
        if !self.dynamic_state_flags.contains(DynamicStateFlags::DEPTH_WRITE_ENABLE       ){ self.set_depth_write_enable(false); }
        if !self.dynamic_state_flags.contains(DynamicStateFlags::DEPTH_BIAS_ENABLE        ){ self.set_depth_bias_enable(false); }
        if !self.dynamic_state_flags.contains(DynamicStateFlags::STENCIL_TEST_ENABLE      ){ self.set_stencil_test_enable(false); }
        if !self.dynamic_state_flags.contains(DynamicStateFlags::RASTERIZER_DISCARD_ENABLE){ self.set_rasterizer_discard_enable(false); }
        if !self.dynamic_state_flags.contains(DynamicStateFlags::RASTERIZATION_SAMPLES    ){ self.set_rasterization_samples(vk::SampleCountFlags::TYPE_1); }
        if !self.dynamic_state_flags.contains(DynamicStateFlags::SAMPLE_MASK              ){ self.set_sample_mask(vk::SampleCountFlags::TYPE_1, &[vk::SampleMask::max_value()]); }
        if !self.dynamic_state_flags.contains(DynamicStateFlags::ALPHA_TO_COVERAGE_ENABLE ){ self.set_alpha_to_coverage_enable(false); }
        if !self.dynamic_state_flags.contains(DynamicStateFlags::SET_CULL_MODE            ){ self.set_cull_mode(vk::CullModeFlags::NONE); }
        if !self.dynamic_state_flags.contains(DynamicStateFlags::COLOR_BLEND_ENABLE       ){ self.set_color_blend_enable(&[0]); }
        if !self.dynamic_state_flags.contains(DynamicStateFlags::COLOR_BLEND_EQUATION     ){ self.set_color_blend_equation(&[vk::ColorBlendEquationEXT::default()]); }
        if !self.dynamic_state_flags.contains(DynamicStateFlags::COLOR_WRITE_MASK         ){ self.set_color_write_mask(&[vk::ColorComponentFlags::RGBA]);}
        // TODO: match this with the exact rules when things should be defined
    }

    pub fn draw(&mut self, vertex_count:u32, instance_count:u32, first_vertex:u32, first_instance:u32){
        self.apply_unset_defaults();
        unsafe{self.renderer.device.cmd_draw(self.renderer.command_buffer, 3, 1, 0, 0)};
    }

    pub fn bind_vs_fs(&self, vs: ShaderEXT, fs: ShaderEXT){
        let stages  = [vk::ShaderStageFlags::VERTEX, vk::ShaderStageFlags::FRAGMENT];
        let shaders = [vs, fs];
        unsafe{self.renderer.ext_shader_object.cmd_bind_shaders(self.renderer.command_buffer, &stages, &shaders)};
    }
}

bitflags!{
    pub struct DynamicStateFlags: u32 {
        const VIEWPORTS                 = 1<< 0;
        const SCISSORS                  = 1<< 1;
        const POLYGON_MODE              = 1<< 2;
        const PRIMITIVE_TOPOLOGY        = 1<< 3;
        const PRIMITIVE_RESTART_ENABLE  = 1<< 4;
        const DEPTH_TEST_ENABLE         = 1<< 5;
        const DEPTH_WRITE_ENABLE        = 1<< 6;
        const DEPTH_BIAS_ENABLE         = 1<< 7;
        const STENCIL_TEST_ENABLE       = 1<< 8;
        const RASTERIZER_DISCARD_ENABLE = 1<< 9;
        const RASTERIZATION_SAMPLES     = 1<<10;
        const SAMPLE_MASK               = 1<<11;
        const ALPHA_TO_COVERAGE_ENABLE  = 1<<12;
        const SET_CULL_MODE             = 1<<13;
        const COLOR_BLEND_ENABLE        = 1<<14;
        const COLOR_BLEND_EQUATION      = 1<<15;
        const COLOR_WRITE_MASK          = 1<<16;
    }
}


impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        match self {
            App::Resumed{..}    => todo!("handle re-resuming"),
            App::Uninitialized => {
                let renderer = renderer::Renderer::new(event_loop);
                renderer.debug_print();
                let [vs,fs] = renderer.load_shader_vs_fs("vert.spv", "frag.spv");
                println!("initialized!!");
                *self = App::Resumed{ renderer, vs, fs };
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
                let App::Resumed{renderer,vs,fs} = self else { panic!("not active!") };
                println!("================================================================================");
                let mut frame = Frame::begin(&renderer);
                frame.bind_vs_fs(*vs, *fs);
                frame.draw(3,1,0,0);
                frame.end();
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

