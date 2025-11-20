use glutin::config::ConfigTemplateBuilder;
use glutin::context::{ContextAttributesBuilder, NotCurrentGlContext};
use glutin::display::GetGlDisplay;
use glutin::prelude::*;
use glutin::surface::{SurfaceAttributesBuilder, WindowSurface};
use glutin_winit::DisplayBuilder;
use parking_lot::Mutex;
use glow::HasContext;
use raw_window_handle::HasRawWindowHandle;
use std::num::NonZeroU32;
use std::sync::Arc;
use tokio::runtime::Runtime;
use winit::event::{Event, WindowEvent};
use winit::event_loop::{ControlFlow, EventLoop};
use winit::window::WindowBuilder;

mod grpc_client;
mod ui;

pub mod kaonic {
    tonic::include_proto!("kaonic");
}

use grpc_client::GrpcClient;
use ui::{AppState, RadioGuiApp};

fn main() {
    env_logger::init();

    // Create tokio runtime for async gRPC operations
    let runtime = Arc::new(Runtime::new().expect("Failed to create tokio runtime"));

    // Create gRPC client
    let client = Arc::new(Mutex::new(GrpcClient::new(runtime.clone())));

    // Create app state
    let state = Arc::new(Mutex::new(AppState::new()));

    // Create event loop and window
    let event_loop = EventLoop::new().expect("Failed to create event loop");
    let window_builder = WindowBuilder::new()
        .with_title("Kaonic Radio Control")
        .with_inner_size(winit::dpi::LogicalSize::new(1200.0, 800.0))
        .with_resizable(false);

    // Create OpenGL context
    let template = ConfigTemplateBuilder::new()
        .with_alpha_size(8)
        .with_transparency(false);

    let display_builder = DisplayBuilder::new().with_window_builder(Some(window_builder));

    let (window, gl_config) = display_builder
        .build(&event_loop, template, |configs| {
            configs
                .reduce(|accum, config| {
                    if config.num_samples() > accum.num_samples() {
                        config
                    } else {
                        accum
                    }
                })
                .unwrap()
        })
        .expect("Failed to create window");

    let window = window.expect("Failed to create window");

    let raw_window_handle = window.raw_window_handle();
    let gl_display = gl_config.display();

    let context_attributes = ContextAttributesBuilder::new().build(Some(raw_window_handle));

    let context = unsafe {
        gl_display
            .create_context(&gl_config, &context_attributes)
            .expect("Failed to create context")
    };

    let (width, height): (u32, u32) = window.inner_size().into();
    let attrs = SurfaceAttributesBuilder::<WindowSurface>::new().build(
        raw_window_handle,
        NonZeroU32::new(width).unwrap(),
        NonZeroU32::new(height).unwrap(),
    );

    let surface = unsafe {
        gl_display
            .create_window_surface(&gl_config, &attrs)
            .expect("Failed to create surface")
    };

    let context = context
        .make_current(&surface)
        .expect("Failed to make context current");

    let gl = unsafe {
        glow::Context::from_loader_function(|s| {
            gl_display.get_proc_address(&std::ffi::CString::new(s).unwrap())
        })
    };

    // Setup ImGui
    let mut imgui = imgui::Context::create();
    imgui.set_ini_filename(None);

    let mut platform = imgui_winit_support::WinitPlatform::init(&mut imgui);
    platform.attach_window(imgui.io_mut(), &window, imgui_winit_support::HiDpiMode::Default);

    let mut renderer = imgui_glow_renderer::AutoRenderer::initialize(gl, &mut imgui)
        .expect("Failed to initialize renderer");

    // Create app
    let mut app = RadioGuiApp::new(client, state, runtime);

    // Main loop
    event_loop
        .run(move |event, window_target| {
            window_target.set_control_flow(ControlFlow::Poll);

            match event {
                Event::NewEvents(_) => {
                    let now = std::time::Instant::now();
                    imgui.io_mut().update_delta_time(now - app.last_frame);
                    app.last_frame = now;
                }
                Event::AboutToWait => {
                    platform
                        .prepare_frame(imgui.io_mut(), &window)
                        .expect("Failed to prepare frame");
                    window.request_redraw();
                }
                Event::WindowEvent {
                    event: WindowEvent::RedrawRequested,
                    ..
                } => {
                    let ui = imgui.frame();
                    app.render(ui);

                    platform.prepare_render(ui, &window);
                    let draw_data = imgui.render();

                    unsafe {
                        renderer.gl_context().clear(glow::COLOR_BUFFER_BIT);
                    }

                    renderer
                        .render(draw_data)
                        .expect("Failed to render");

                    surface
                        .swap_buffers(&context)
                        .expect("Failed to swap buffers");
                }
                Event::WindowEvent {
                    event: WindowEvent::CloseRequested,
                    ..
                } => {
                    window_target.exit();
                }
                Event::WindowEvent {
                    event: WindowEvent::Resized(new_size),
                    ..
                } => {
                    if new_size.width > 0 && new_size.height > 0 {
                        surface.resize(
                            &context,
                            NonZeroU32::new(new_size.width).unwrap(),
                            NonZeroU32::new(new_size.height).unwrap(),
                        );
                    }
                }
                event => {
                    platform.handle_event(imgui.io_mut(), &window, &event);
                }
            }
        })
        .expect("Event loop error");
}
