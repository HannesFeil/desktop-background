use std::path::PathBuf;

use chrono::Utc;
use clap::Parser;
use image::{open, RgbaImage};
use pixels::{wgpu::RequestAdapterOptions, PixelsBuilder, SurfaceTexture};
use winit::{
    event::{Event, WindowEvent},
    event_loop::EventLoopBuilder,
    platform::wayland::{EventLoopBuilderExtWayland, WindowBuilderExtWayland},
    window::WindowBuilder,
};

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Args {
    /// Desktop resolution width in pixels
    #[arg(value_name = "WIDTH")]
    width: u32,
    /// Desktop resolution height in pixels
    #[arg(value_name = "HEIGHT")]
    height: u32,
    /// Window class name
    #[arg(value_name = "WINDOW_CLASS")]
    window_class: String,
    /// Optional background image
    #[arg(long)]
    background: Option<PathBuf>,
}

fn main() {
    let args = Args::parse();

    let event_loop = EventLoopBuilder::new().with_wayland().build().unwrap();
    let window = WindowBuilder::new()
        .with_name(args.window_class, "")
        .build(&event_loop)
        .unwrap();

    let background_image = args.background.map(|path| {
        image::imageops::resize(
            &open(path).unwrap().into_rgba8(),
            args.width,
            args.height,
            image::imageops::FilterType::Triangle,
        )
    });

    let surface_texture = SurfaceTexture::new(1920, 1080, &window);
    let mut pixels = PixelsBuilder::new(1920, 1080, surface_texture)
        .request_adapter_options(RequestAdapterOptions {
            power_preference: pixels::wgpu::PowerPreference::LowPower,
            force_fallback_adapter: false,
            compatible_surface: None,
        })
        .enable_vsync(true)
        .build()
        .unwrap();

    event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);
    event_loop
        .run(move |event, elwt| match event {
            Event::WindowEvent { event, .. } => match event {
                WindowEvent::CloseRequested => elwt.exit(),
                WindowEvent::RedrawRequested => {
                    let now = Utc::now();
                    let secs = now.timestamp_subsec_micros();

                    render(pixels.frame_mut(), background_image.as_ref());
                    pixels.render().unwrap();
                    window.request_redraw();
                }
                _ => {}
            },
            Event::LoopExiting => println!("bye!"),
            _ => {}
        })
        .unwrap();
}

fn render(buffer: &mut [u8], background: Option<&RgbaImage>) {
    if let Some(image) = background {
        buffer.copy_from_slice(image);
    }
}
