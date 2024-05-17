use std::{
    collections::HashSet, 
    fmt,
    ffi::{CStr,OsStr}
};
use core::mem::size_of;
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    raw_window_handle::{HasDisplayHandle, HasWindowHandle},
    window::{Window,WindowId}
};
use ash::{
     ext, khr, vk::{self, CommandBuffer, CommandPool, Fence, Handle, Image, ImageView, InstanceCreateInfo, PhysicalDevice, Queue, Semaphore, ShaderEXT, SurfaceCapabilitiesKHR, SurfaceFormatKHR, SurfaceKHR, SwapchainCreateInfoKHR, SwapchainKHR, Extent2D, PhysicalDeviceMemoryProperties}, Device, Entry, Instance
};
use bitflags::bitflags;

const ERR_STR : &'static str = "\x1B[41;97;1m ERROR \x1B[m";
fn pretty_print_path<P:?Sized+AsRef<std::path::Path>>(path:&P) -> String {
    let p = path.as_ref().to_string_lossy();
    let pwd = match std::env::current_dir() {
        Ok(pathbuf) => pathbuf.to_string_lossy().into(),
        Err(e) => "./".to_owned(),
    };
    format!("\x1B[m\x1B[37m{pwd}/\x1B[91;1m{p}\x1B[m")
}


pub struct Renderer{
    pub window:   Window,
    pub entry:    Entry,
    pub instance: Instance,
    pub gpu:      PhysicalDevice,
    pub memory_properties: PhysicalDeviceMemoryProperties,
    pub bar_memory_idx: Option<u32>,
    pub gpu_memory_idx: u32,
    pub surface:  SurfaceKHR,
    pub device:   Device,
    pub queue:    Queue,
    pub fam_idx:  u32,
    pub descriptor_pool: vk::DescriptorPool,
    pub surface_format:   SurfaceFormatKHR,
    pub swapchain:        SwapchainKHR,
    pub swapchain_extent: Extent2D,
    pub swapchain_images: Vec<Image>,
    pub swapchain_views:  Vec<ImageView>,
    pub command_pool:     CommandPool,
    pub command_buffer:   CommandBuffer,
    pub ready_to_submit: Semaphore,
    pub ready_to_record: Fence,
    pub ready_to_present: Semaphore,
    pub khr_display:    khr::display::Instance,
    pub khr_surface:    khr::surface::Instance,
    pub khr_swapchain:  khr::swapchain::Device,
    pub khr_dynamic_rendering: khr::dynamic_rendering::Device,
    pub ext_shader_object: ext::shader_object::Device,
}

impl Renderer {
    // TODO: remove dependencie on winit, use raw window/display handles instead
    fn platform_specific_init(entry: &Entry, window: &Window, mut extensions: Vec<&CStr>) -> (Instance, SurfaceKHR) {
        use winit::raw_window_handle::RawWindowHandle  as raw_win;
        use winit::raw_window_handle::RawDisplayHandle as raw_dpy;
        let layers = [c"VK_LAYER_KHRONOS_validation".as_ptr()];
        let raw_window = window.window_handle().unwrap().as_raw();
        let raw_display = window.display_handle().unwrap().as_raw();
        match (raw_window, raw_display) {
            (raw_win::Xlib(win), raw_dpy::Xlib(dpy)) => {
                extensions.push(khr::xlib_surface::NAME);
                let extensions: Vec<*const i8> = extensions.iter().map(|x| x.as_ptr()).collect();
                let instance_info = InstanceCreateInfo::default()
                    .enabled_layer_names(&layers)
                    .enabled_extension_names(&extensions);
                let instance = unsafe{entry.create_instance(&instance_info, None)}
                .expect("could not create vulkan instance");
                let info = vk::XlibSurfaceCreateInfoKHR::default()
                    .window(win.window)
                    .dpy(dpy.display.unwrap().as_ptr());
                let xlib_surface = khr::xlib_surface::Instance::new(entry, &instance);
                let surface = unsafe{xlib_surface.create_xlib_surface(&info, None)}.unwrap();
                return (instance, surface);
            },
            _ => panic!("unsupported window!"),
        }
    }

    fn create_swapchain(window:&Window, gpu:&PhysicalDevice, device:&Device, khr_swapchain: &khr::swapchain::Device, khr_surface: &khr::surface::Instance, surface: SurfaceKHR, surface_format:SurfaceFormatKHR)
            -> (SwapchainKHR, Vec<Image>, Vec<ImageView>, Extent2D) {
        let capabilities = unsafe{khr_surface.get_physical_device_surface_capabilities(*gpu, surface)}.unwrap();
        let swapchain_extent = match capabilities.current_extent {
            Extent2D{width:u32::MAX, height:u32::MAX} => {
                let size = window.inner_size();
                let min = capabilities.max_image_extent;
                let max = capabilities.max_image_extent;
                vk::Extent2D{
                    width:  size.width.clamp(min.width, max.width),
                    height: size.height.clamp(min.height, max.height),
                }
            },
            x => x,
        };
        let swapchain_info = SwapchainCreateInfoKHR::default()
            .surface(surface)
            .min_image_count(capabilities.max_image_count.min(capabilities.min_image_count+1))
            .image_format(surface_format.format)
            .image_color_space(surface_format.color_space)
            .image_extent(swapchain_extent)
            .image_array_layers(1)
            .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
            .pre_transform(vk::SurfaceTransformFlagsKHR::IDENTITY)
            .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE);
        let swapchain = unsafe{khr_swapchain.create_swapchain(&swapchain_info, None)}.unwrap();
        let swapchain_images = unsafe{khr_swapchain.get_swapchain_images(swapchain)}.unwrap();
        let swapchain_views : Vec<_> = swapchain_images.iter().map(|img|{
            let info = vk::ImageViewCreateInfo::default()
                .image(*img)
                .view_type(vk::ImageViewType::TYPE_2D)
                .format(surface_format.format)
                .components(vk::ComponentMapping::default())
                .subresource_range(vk::ImageSubresourceRange::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .base_mip_level(0)
                    .level_count(1)
                    .base_array_layer(0)
                    .layer_count(1));
            unsafe{ device.create_image_view(&info, None) }.unwrap()
        }).collect();

        (swapchain, swapchain_images, swapchain_views, swapchain_extent)
    }
    fn destroy_swapchain(&self){
        // Note: swapchain images are owned by the the swapchain, so we only have to free the views
        for view in self.swapchain_views.iter() {
            unsafe{self.device.destroy_image_view(*view, None)};
        }
        unsafe{self.khr_swapchain.destroy_swapchain(self.swapchain, None)};
    }
    fn recreate_swapchain(&mut self){
        self.destroy_swapchain();
        let (swapchain, swapchain_images, swapchain_views, swapchain_extent) = Self::create_swapchain(&self.window, &self.gpu, &self.device, &self.khr_swapchain, &self.khr_surface, self.surface, self.surface_format);
        self.swapchain = swapchain;
        self.swapchain_images = swapchain_images;
        self.swapchain_views = swapchain_views;
        self.swapchain_extent = swapchain_extent;
    }

    pub fn new(event_loop: &ActiveEventLoop) -> Self {
        let entry = unsafe{Entry::load()}.expect("could not find Vulkan");
        let window = event_loop.create_window(Window::default_attributes()).expect("could not open Vulkan");

        let (instance, surface) = Self::platform_specific_init(&entry, &window, vec![
            khr::surface::NAME,
            khr::display::NAME,
            khr::get_physical_device_properties2::NAME, // required for shader_object
        ]);
        let khr_display = khr::display::Instance::new(&entry, &instance);
        let khr_surface = khr::surface::Instance::new(&entry, &instance);

        let required_device_extensions = [
            khr::swapchain::NAME, 

            // these are all required for shader_object
            ext::shader_object::NAME,
            khr::dynamic_rendering::NAME,
            khr::depth_stencil_resolve::NAME,
            khr::create_renderpass2::NAME,
            khr::multiview::NAME,
            khr::maintenance2::NAME,

            // required for descriptor_indexing
            ext::descriptor_indexing::NAME,
            khr::maintenance3::NAME,
        ];

        let required_device_extensions_set : HashSet<_> = required_device_extensions.into();
        let gpus = unsafe{instance.enumerate_physical_devices()}.unwrap();
        let (gpu, fam_idx, surface_format) = gpus.iter().filter_map(|gpu| {
            // check whether gpu supports our required extensions
            let extensions = unsafe{instance.enumerate_device_extension_properties(*gpu)}.unwrap();
            let extensions : HashSet::<_> = extensions.iter().map(|x|x.extension_name_as_c_str().unwrap()).collect();
            let missing : Vec<_> = required_device_extensions_set.difference(&extensions).collect();
            if missing.len()!=0 { return None }

            let queueprop = unsafe{ instance.get_physical_device_queue_family_properties(*gpu) };
            let Some(fam_idx) = queueprop.iter().enumerate().filter_map(|(fam_idx,queue)|{
                let fam_idx = fam_idx as u32;
                println!("{fam_idx} {queue:?}");
                if !queue.queue_flags.contains(vk::QueueFlags::GRAPHICS|vk::QueueFlags::TRANSFER) { 
                    return None
                };
                if !unsafe{khr_surface.get_physical_device_surface_support(*gpu, fam_idx, surface)}.unwrap_or(false) {
                    return None
                };
                Some(fam_idx)
            }).next() else { return None };

            let surface_formats = unsafe{khr_surface.get_physical_device_surface_formats(*gpu, surface)}.unwrap();
            let Some(surface_format) = surface_formats.iter().filter_map(|format| {
                match format.format {
                    vk::Format::B8G8R8A8_SRGB => Some(format.clone()),
                    vk::Format::R8G8B8A8_SRGB => Some(format.clone()),
                    _ => None
                }
            }).next() else { return None };

            Some((*gpu, fam_idx, surface_format))
        }).next().expect("no suitable gpu's found.");

        let queue_priorities = [1.0];
        let queue_infos = [
            vk::DeviceQueueCreateInfo::default()
                .queue_family_index(fam_idx)
                .queue_priorities(&queue_priorities)
        ];

        let required_device_extensions = required_device_extensions.map(|x|x.as_ptr());

        let mut feature_descriptor_indexing = vk::PhysicalDeviceDescriptorIndexingFeaturesEXT::default()
            .descriptor_binding_storage_buffer_update_after_bind(true);
        let mut feature_shader_object     = vk::PhysicalDeviceShaderObjectFeaturesEXT::default().shader_object(true);
        let mut feature_dynamic_rendering = vk::PhysicalDeviceDynamicRenderingFeatures::default().dynamic_rendering(true);
        let device_info = vk::DeviceCreateInfo::default()
            .queue_create_infos(&queue_infos)
            .enabled_extension_names(&required_device_extensions)
            .push_next(&mut feature_shader_object)
            .push_next(&mut feature_dynamic_rendering)
            .push_next(&mut feature_descriptor_indexing);
        let device = unsafe{instance.create_device(gpu, &device_info, None)}.expect("unable to create vkdevice");
        let queue = unsafe{device.get_device_queue(fam_idx, 0)};
        let khr_dynamic_rendering = khr::dynamic_rendering::Device::new(&instance, &device);
        let ext_shader_object     = ext::shader_object::Device::new(&instance, &device);
        let khr_swapchain = khr::swapchain::Device::new(&instance, &device);
        println!("device ready");

        fn fmt_size(n:u64) -> String{
            if n<1_000_000 {
                format!("{:>3} B", n)
            }else if n<1_000_000 {
                format!("{:>3} kB", n>>10)
            }else if n<1_000_000_000 {
                format!("{:>3} MB", n>>20)
            }else{
                format!("{:>3} GB", n>>30)
            }
        }

        // identify memories
        let memory_properties = unsafe{ instance.get_physical_device_memory_properties(gpu) };
        // for i in 0..memory_properties.memory_heap_count {
        //     let heap = memory_properties.memory_heaps[i as usize];
        //     let bar = if heap.size <= 256<<20 && heap.flags.contains(vk::MemoryHeapFlags::DEVICE_LOCAL) { "<- likely BAR heap" } else {""};
        //     println!("{i:>2}: {size} {flags:?} {bar}", flags=heap.flags, size=fmt_size(heap.size));
        // }
        let mut bar_memory_idx = None;
        let mut gpu_memory_idx = None;
        for (i,memtype) in memory_properties.memory_types_as_slice().iter().enumerate() {
            let heap = memory_properties.memory_heaps[memtype.heap_index as usize];
            let host_visible = memtype.property_flags.contains(vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT);
            let device_local = heap.flags.contains(vk::MemoryHeapFlags::DEVICE_LOCAL);
            let mut select = "";
            if host_visible && device_local {
                let rebar = if heap.size<(256<<20) { select="BAR ->" } else { select="reBAR ->" };
                if bar_memory_idx.is_none() { 
                    bar_memory_idx = Some(i as u32);
                };
            } else if device_local {
                if gpu_memory_idx.is_none() {
                    gpu_memory_idx = Some(i as u32);
                    select = "gpu ->";
                };
            } 
            println!("{select:>8}{i:>2}:{heap} {size} {location:<4} {flags:?}", heap=memtype.heap_index, size=fmt_size(heap.size), location=if device_local {"gpu"} else {"host"}, flags=memtype.property_flags );
        }
        let Some(gpu_memory_idx) = gpu_memory_idx else {panic!("no device memory")};
        println!("gpu: {gpu_memory_idx:?}");
        println!("bar: {bar_memory_idx:?}");

        let (swapchain, swapchain_images, swapchain_views, swapchain_extent) = Self::create_swapchain(&window, &gpu, &device, &khr_swapchain, &khr_surface, surface, surface_format);
        println!("swapchain created");

        let command_pool_info = vk::CommandPoolCreateInfo::default()
            .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER)
            .queue_family_index(fam_idx);
        let command_pool = unsafe{device.create_command_pool(&command_pool_info, None)}.unwrap();

        let alloc_info = vk::CommandBufferAllocateInfo::default()
            .command_pool(command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        let [command_buffer] = unsafe{device.allocate_command_buffers(&alloc_info)}.unwrap()[..] else {panic!("got more buffers than expected")};
        println!("command buffer created");

        let semaphore_info = vk::SemaphoreCreateInfo::default();
        let ready_to_submit = unsafe{device.create_semaphore(&semaphore_info, None)}.unwrap();
        let ready_to_present = unsafe{device.create_semaphore(&semaphore_info, None)}.unwrap();
        let fence_info = vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED);
        let ready_to_record = unsafe{device.create_fence(&fence_info, None)}.unwrap();

        let descriptor_pool_sizes = [
            vk::DescriptorPoolSize::default().ty(vk::DescriptorType::COMBINED_IMAGE_SAMPLER).descriptor_count(1),
        ];
        let descriptor_pool_info = vk::DescriptorPoolCreateInfo::default()
            .pool_sizes(&descriptor_pool_sizes)
            //.flags(vk::DescriptorPoolCreateFlags::UPDATE_AFTER_BIND_EXT)
            .max_sets(1);
        let descriptor_pool = unsafe{device.create_descriptor_pool(&descriptor_pool_info, None)}.unwrap();



        Self{ window, entry, instance, gpu, memory_properties, bar_memory_idx, gpu_memory_idx, surface, device, queue, fam_idx, descriptor_pool, surface_format, swapchain, swapchain_extent, swapchain_images, swapchain_views, command_pool, command_buffer, ready_to_submit, ready_to_present, ready_to_record, khr_display, khr_surface,  khr_swapchain, khr_dynamic_rendering, ext_shader_object }
    }

    pub fn alloc_buffer(&self, size:u64,usage: vk::BufferUsageFlags, mem_idx:u32) -> (vk::Buffer,vk::DeviceMemory) {
        let buffer_info = vk::BufferCreateInfo::default()
            .size(size)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .usage(usage);
        let buffer = unsafe{self.device.create_buffer(&buffer_info, None)}.expect("could not create buffer");
        let req    = unsafe{self.device.get_buffer_memory_requirements(buffer)};
        let alloc_info = vk::MemoryAllocateInfo::default()
            .allocation_size(req.size)
            .memory_type_index(mem_idx);
        let memory = unsafe{ self.device.allocate_memory(&alloc_info, None) }.expect("could not alloc memory");
        unsafe{ self.device.bind_buffer_memory(buffer, memory, 0); }
        (buffer, memory)
    }

    pub fn map_bar_buffer(&self, size:u64, usage: vk::BufferUsageFlags) -> Option<(vk::Buffer,*mut core::ffi::c_void)> {
        let mem_idx = match self.bar_memory_idx {
            None => return None,
            Some(idx) => idx,
        };
        let (buffer, memory) = self.alloc_buffer(size, usage, mem_idx);

        //let req = unsafe{self.device.get_buffer_memory_requirements(buf)};
        let ptr = unsafe{ self.device.map_memory(memory, 0, size, vk::MemoryMapFlags::empty()) }.expect("memory map failed");
        //let ptr = core::ptr::slice_from_raw_parts_mut(unsafe{core::mem::transmute(ptr)}, size as usize);
        Some((buffer,ptr))
    }

    // TODO: #[cfg(feature="glslc")]
    pub fn load_glsl_vs_fs<P:?Sized+AsRef<std::path::Path>> (&self,
            vs_glsl_path: &P, 
            fs_glsl_path: &P, 
            push_constant_ranges : &[vk::PushConstantRange],
            descriptor_set_layout : &[vk::DescriptorSetLayout]) -> (ShaderEXT,ShaderEXT) {
        use shaderc;
        let mut compiler = shaderc::Compiler::new().unwrap();
        let mut options  = shaderc::CompileOptions::new().unwrap();

        let vert_src = std::fs::read_to_string(vs_glsl_path).expect("could not read vertex shader");
        let vert = compiler.compile_into_spirv(
            &vert_src, 
            shaderc::ShaderKind::Vertex,
            &vs_glsl_path.as_ref().to_string_lossy(),
            "main",
            Some(&options)
        ).expect("vert shader failed to compile");

        let frag_src = std::fs::read_to_string(fs_glsl_path).expect("could not read fragment shader");
        let frag = compiler.compile_into_spirv(
            &frag_src, 
            shaderc::ShaderKind::Fragment,
            &fs_glsl_path.as_ref().to_string_lossy(),
            "main",
            Some(&options)
        ).expect("vert shader failed to compile");
        self.load_spirv_vs_fs(vert.as_binary_u8(), frag.as_binary_u8(), push_constant_ranges, descriptor_set_layout)
    }

    pub fn load_spirv_vs_fs (&self, 
            vs_spv : &[u8],
            fs_spv : &[u8],
            push_constant_ranges : &[vk::PushConstantRange],
            descriptor_set_layout : &[vk::DescriptorSetLayout]) -> (ShaderEXT,ShaderEXT) {

        let shader_infos = [
            vk::ShaderCreateInfoEXT::default()
                .flags(vk::ShaderCreateFlagsEXT::LINK_STAGE)
                .stage(vk::ShaderStageFlags::VERTEX)
                .next_stage(vk::ShaderStageFlags::FRAGMENT)
                .code_type(vk::ShaderCodeTypeEXT::SPIRV)
                .code(vs_spv)
                .name(c"main")
                .push_constant_ranges(&push_constant_ranges)
                .set_layouts(&descriptor_set_layout),
            vk::ShaderCreateInfoEXT::default()
                .flags(vk::ShaderCreateFlagsEXT::LINK_STAGE)
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .next_stage(vk::ShaderStageFlags::empty())
                .code_type(vk::ShaderCodeTypeEXT::SPIRV)
                .code(fs_spv)
                .name(c"main")
                .push_constant_ranges(&push_constant_ranges)
                .set_layouts(&descriptor_set_layout),
        ];
        match unsafe{ self.ext_shader_object.create_shaders(&shader_infos, None) } {
            Ok(ret) => (ret[0], ret[1]),
            Err((ret,err)) => {
                let vs = ret[0];
                let fs = ret[1];
                if ret[0].is_null() {
                    panic!("\n{ERR_STR} vertex shader failed to compile\n{err}\n")
                }else if ret[1].is_null() {
                    panic!("\n{ERR_STR} fragment shader failed to compile\n{err}\n")
                }else {
                    panic!("\n{ERR_STR} shader compilation failed\n{err}\n")
                }
            }
        }
    }

    pub fn wait_for_frame(&mut self) -> Frame { Frame::new(self) }

    pub fn debug_print(&self){
        let properties = unsafe{self.instance.get_physical_device_properties(self.gpu)};
        let name = properties.device_name_as_c_str().unwrap().to_str().unwrap();
        println!("gpu: {name}");

        let displays = unsafe{self.khr_display.get_physical_device_display_properties(self.gpu)}.unwrap();
        for display_properties in displays {
            let name = unsafe{display_properties.display_name_as_c_str()}.unwrap().to_str().unwrap();
            let mm = display_properties.physical_dimensions;
            let px = display_properties.physical_resolution;
            let dpi_w = 25.4*px.width  as f32/mm.width  as f32;
            let dpi_h = 25.4*px.height as f32/mm.height as f32;
            println!("-> {:>4}x{:>4}px  {:3>}x{:3>}mm  {dpi_w:>3.0}x{dpi_h:>3.0}dpi  {name}", 
                px.width, px.height, mm.width, mm.height);
        }
    }
}

impl Drop for Renderer {
    fn drop(&mut self){
        println!("todo! implement drop for renderer");
    }
}


pub struct Frame<'a>{
    renderer : &'a mut Renderer,
    swap_idx : u32,
    dynamic_state_flags : DynamicStateFlags,
}
impl<'a> Frame<'a> {

    pub fn new(renderer: &'a mut Renderer) -> Self {
        // Synchronisation
        // three primitives:
        //  - ready_to_record:  signaled by VkQueueSubmit, awaited by host before vkAcquireNextImageKHR
        //  - ready_to_submit:  signaled by vkAcquireNextImageKHR, awaited by vkQueueSubmit
        //  - ready_to_present: signaled by vkQueueSubmit, awaited by vkQueuePresentKHR
        unsafe{renderer.device.wait_for_fences(&[renderer.ready_to_record], true, u64::MAX)};
        unsafe{renderer.device.reset_fences(&[renderer.ready_to_record])};

        let swap_idx = loop{
            let swap_idx = match unsafe{renderer.khr_swapchain.acquire_next_image(renderer.swapchain, u64::MAX, renderer.ready_to_submit, Fence::null())} {
                Ok((swap_idx, false)) => break swap_idx,
                Ok((swap_idx, true)) => {
                    println!("resize! (aquire_next_image suboptimal)");
                    renderer.recreate_swapchain();
                },
                Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                    println!("resize! (aquire_next_image out of date)");
                    renderer.recreate_swapchain();
                },
                Err(e) => {
                    panic!("error: {e}\n");
                }
            };
        };

        // begin command buffer
        unsafe{renderer.device.reset_command_buffer(renderer.command_buffer, vk::CommandBufferResetFlags::empty())};
        let begin_info = vk::CommandBufferBeginInfo::default();
        unsafe{renderer.device.begin_command_buffer(renderer.command_buffer, &begin_info)}.unwrap();

        // transition swapchain image from present-optimal to render-optimal
        let range = vk::ImageSubresourceRange::default()
            .aspect_mask(vk::ImageAspectFlags::COLOR)
            .base_mip_level(0)
            .level_count(1)
            .base_array_layer(0)
            .layer_count(1);
        let image_memory_barriers = [
            vk::ImageMemoryBarrier::default()
                .image(renderer.swapchain_images[swap_idx as usize])
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
                .subresource_range(range),
        ];
        unsafe{renderer.device.cmd_pipeline_barrier(renderer.command_buffer,
            vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT, // (changed from TOP_OF_PIPE)
            vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
            vk::DependencyFlags::empty(),
            &[], &[], &image_memory_barriers)};

        let dynamic_state_flags = DynamicStateFlags::empty();
        Self{ renderer, swap_idx, dynamic_state_flags}
    }

    pub fn buffer_to_image(&self, buffer: vk::Buffer, image: vk::Image, regions: &[vk::BufferImageCopy]){
        if regions.len() == 0 { return; }
        let subrange = vk::ImageSubresourceRange{ aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level:0, level_count:1, base_array_layer:0, layer_count:1 };
        let to_write_optimal = [vk::ImageMemoryBarrier::default()
            .image(image)
            .old_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
            .src_access_mask(vk::AccessFlags::NONE)
            .dst_access_mask(vk::AccessFlags::TRANSFER_READ)
            .subresource_range(subrange)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        ];
        unsafe{self.renderer.device.cmd_pipeline_barrier(
            self.renderer.command_buffer,
            vk::PipelineStageFlags::TOP_OF_PIPE,
            vk::PipelineStageFlags::TRANSFER,
            vk::DependencyFlags::BY_REGION,
            &[], &[], &to_write_optimal)};

        unsafe{self.renderer.device.cmd_copy_buffer_to_image(self.renderer.command_buffer,
            buffer,
            image,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            regions)};

        let to_display_optimal = [vk::ImageMemoryBarrier::default()
            .image(image)
            .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
            .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            .src_access_mask(vk::AccessFlags::NONE)
            .dst_access_mask(vk::AccessFlags::TRANSFER_READ)
            .subresource_range(subrange)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        ];
        unsafe{self.renderer.device.cmd_pipeline_barrier(
            self.renderer.command_buffer,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::FRAGMENT_SHADER,
            vk::DependencyFlags::BY_REGION,
            &[], &[], &to_display_optimal)};
    }


    pub fn begin_rendering(&self, color: [f32;4]) {
        // begin rendering
        let mut clear_color_value = vk::ClearColorValue::default();
        clear_color_value.float32 = color; 
        let mut clear_color = vk::ClearValue::default();
        clear_color.color = clear_color_value;
        let color_attachments = [
            vk::RenderingAttachmentInfo::default()
                .image_view(self.renderer.swapchain_views[self.swap_idx as usize])
                .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .resolve_image_layout(vk::ImageLayout::PRESENT_SRC_KHR)
                .load_op(vk::AttachmentLoadOp::CLEAR)
                .store_op(vk::AttachmentStoreOp::STORE)
                .clear_value(clear_color),
        ];
        let rendering_info = vk::RenderingInfo::default()
            .render_area(self.renderer.swapchain_extent.into())
            .layer_count(1)
            .color_attachments(&color_attachments);
        unsafe{self.renderer.khr_dynamic_rendering.cmd_begin_rendering(self.renderer.command_buffer, &rendering_info)};
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
        self.dynamic_state_flags |= DynamicStateFlags::ALPHA_TO_COVERAGE_ENABLE ;
        unsafe{self.renderer.ext_shader_object.cmd_set_alpha_to_coverage_enable(self.renderer.command_buffer, enable)};
    }
    pub fn set_cull_mode(&mut self, cullmode: vk::CullModeFlags){
        self.dynamic_state_flags |= DynamicStateFlags::SET_CULL_MODE;
        unsafe{self.renderer.ext_shader_object.cmd_set_cull_mode(self.renderer.command_buffer, cullmode)};
    }
    pub fn set_color_blend_enable(&mut self, enables: &[u32]){
        self.dynamic_state_flags |= DynamicStateFlags::COLOR_BLEND_ENABLE;
        unsafe{self.renderer.ext_shader_object.cmd_set_color_blend_enable(self.renderer.command_buffer, 0, &enables)};
    }
    pub fn set_color_blend_equation(&mut self, equations: &[vk::ColorBlendEquationEXT]){
        self.dynamic_state_flags |= DynamicStateFlags::COLOR_BLEND_EQUATION;
        unsafe{self.renderer.ext_shader_object.cmd_set_color_blend_equation(self.renderer.command_buffer, 0, &equations)};
    }
    pub fn set_color_write_mask(&mut self, write_masks: &[vk::ColorComponentFlags]){
        self.dynamic_state_flags |= DynamicStateFlags::COLOR_WRITE_MASK;
        unsafe{self.renderer.ext_shader_object.cmd_set_color_write_mask(self.renderer.command_buffer, 0, &write_masks)};
    }

    // vulkan requires us to set these things before rendering
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

    pub fn draw(&mut self, vertex_count:u32, first_vertex:u32){
        self.apply_unset_defaults();
        unsafe{self.renderer.device.cmd_draw(self.renderer.command_buffer, vertex_count, 1, first_vertex, 0)};
    }

    pub fn draw_indexed(&mut self, index_count:u32, first_index:u32, vertex_offset:i32){
        self.apply_unset_defaults();
        unsafe{self.renderer.device.cmd_draw_indexed(self.renderer.command_buffer, index_count, 1, first_index, vertex_offset, 0)};
    }

    pub fn bind_vs_fs(&self, vs: ShaderEXT, fs: ShaderEXT){
        let stages  = [vk::ShaderStageFlags::VERTEX, vk::ShaderStageFlags::FRAGMENT];
        let shaders = [vs, fs];
        unsafe{self.renderer.ext_shader_object.cmd_bind_shaders(self.renderer.command_buffer, &stages, &shaders)};
    }

    pub fn set_vertex_input(&self, vertex_stride:u32, offsets:&[(u32,vk::Format)]){
        let binding = [vk::VertexInputBindingDescription2EXT::default()
            .binding(0)
            .stride(vertex_stride)
            .input_rate(vk::VertexInputRate::VERTEX)
            .divisor(1) ];
        let attribute : Vec<_> = offsets.into_iter().enumerate().map(|(i,(off,fmt))|
            vk::VertexInputAttributeDescription2EXT::default()
            .location(i as u32)
            .binding(0)
            .format(*fmt)
            .offset(*off) ).collect();
        unsafe{self.renderer.ext_shader_object.cmd_set_vertex_input(self.renderer.command_buffer,
            &binding, &attribute)};
    }

    pub fn bind_index_buffer(&self, buffer: vk::Buffer, offset:u64){
        unsafe{self.renderer.device.cmd_bind_index_buffer(self.renderer.command_buffer, buffer, offset, vk::IndexType::UINT16)};
    }

    pub fn bind_vertex_buffer(&self, buffer: vk::Buffer){
        let buffers = [buffer];
        let offsets = [0];
        unsafe{self.renderer.device.cmd_bind_vertex_buffers(self.renderer.command_buffer, 0, &buffers, &offsets)};//, Some(&sizes), Some(&strides))};
    }

    pub fn bind_descriptor_set(&self, descriptor_set:vk::DescriptorSet, pipeline_layout:vk::PipelineLayout){
        let descriptor_set = [descriptor_set];
        unsafe{self.renderer.device.cmd_bind_descriptor_sets(
            self.renderer.command_buffer, vk::PipelineBindPoint::GRAPHICS,
            pipeline_layout,
            0,
            &descriptor_set,
            &[])};
    }

    pub fn push_constant<T>(&self, pipeline_layout: vk::PipelineLayout, data:&T){
        let ptr = core::ptr::from_ref(data);
        let byte_ptr = unsafe{core::mem::transmute::<*const T,*const u8>(ptr)};
        let bytes = unsafe{core::slice::from_raw_parts(byte_ptr, size_of::<T>())};
        unsafe{self.renderer.device.cmd_push_constants(
            self.renderer.command_buffer,
            pipeline_layout,
            vk::ShaderStageFlags::VERTEX, 0, bytes)};
    }
}

impl Drop for Frame<'_> {
    fn drop(&mut self){
        let renderer = &self.renderer;
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
        let wait_semaphores = [renderer.ready_to_submit];
        let wait_stages = [vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT];
        let command_buffers = [renderer.command_buffer];
        let signal_semaphores = [renderer.ready_to_present];
        let submit_info = vk::SubmitInfo::default()
            .wait_semaphores(&wait_semaphores)
            .wait_dst_stage_mask(&wait_stages)
            .command_buffers(&command_buffers)
            .signal_semaphores(&signal_semaphores);
        unsafe{renderer.device.queue_submit(renderer.queue, &[submit_info], renderer.ready_to_record)}.unwrap();

        let swapchains = [renderer.swapchain];
        let image_indices = [swap_idx];
        let present_info = vk::PresentInfoKHR::default()
            .wait_semaphores(&signal_semaphores)
            .swapchains(&swapchains)
            .image_indices(&image_indices);
        match unsafe{renderer.khr_swapchain.queue_present(renderer.queue, &present_info)} {
            Ok(false) => (),
            Ok(true) => {
                println!("resize! (queue present suboptimal)");
                self.renderer.recreate_swapchain();
                self.renderer.window.request_redraw();
            },
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                println!("resize! (queue present out of date)");
                self.renderer.recreate_swapchain();
                self.renderer.window.request_redraw();
            },
            Err(e) => panic!("queue present error: {e}"),
        }
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

