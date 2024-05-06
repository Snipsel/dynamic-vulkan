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
                let mut frame = renderer::Frame::begin(&renderer);
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

