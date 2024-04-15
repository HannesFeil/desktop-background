mod render;

use anyhow::bail;
use clap::{Parser, Subcommand};
use interprocess::local_socket::{LocalSocketListener, LocalSocketStream};
use pixels::{wgpu::RequestAdapterOptions, Pixels, PixelsBuilder, SurfaceTexture};
use render::BackgroundRenderer;
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    io::Write,
    path::PathBuf,
    time::{Duration, Instant},
};
use winit::{
    event::{Event, WindowEvent},
    event_loop::EventLoopBuilder,
    platform::wayland::{EventLoopBuilderExtWayland, WindowBuilderExtWayland},
    window::WindowBuilder,
};

const TICK_RATE: u64 = 50;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Args {
    /// The socket name
    #[arg()]
    socket_name: String,
    /// Command
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Clone, Subcommand, Serialize, Deserialize)]
enum Command {
    /// Start the desktop program
    Start {
        /// Desktop resolution width in pixels
        #[arg()]
        width: u32,
        /// Desktop resolution height in pixels
        #[arg()]
        height: u32,
        /// Window class name
        #[arg()]
        window_class: String,
    },
    /// Close the running desktop program
    Stop,
    /// A static image background
    StaticImage {
        /// The image file to use
        #[arg()]
        path: PathBuf,
    },
    /// A dynamically changing background image according to the time of day the
    ClockImage {
        /// The directory which contains the clock images by hour in sub folders "0" to "11"
        #[arg()]
        dir: PathBuf,
        /// The template file name where %m will get replaced by the current time in milliseconds
        /// padded to 8 digits with 0's eg in the range of 0000000 (inclusive) - 43200000 (exclusive).
        ///
        /// # Example
        /// `"clock_frame_%m.png"`
        #[arg()]
        file_template: String,
        /// The clock step in milli seconds
        #[arg(default_value_t = 100)]
        clock_step: u32,
        /// The clock color: < RAINBOW | ###### (rgb hex) >
        #[arg(long, short)]
        clock_color: Option<String>,
    },
}

impl Command {
    pub fn into_renderer(
        self,
        pixels: &mut Pixels,
        width: u32,
        height: u32,
    ) -> anyhow::Result<render::BackgroundRenderer> {
        match self {
            Command::StaticImage { path } => {
                let image = image::imageops::resize(
                    &image::open(path)?,
                    width,
                    height,
                    image::imageops::FilterType::Triangle,
                );
                pixels.frame_mut().copy_from_slice(&image);

                Ok(BackgroundRenderer::None)
            }
            Command::ClockImage {
                dir,
                file_template,
                clock_step,
                clock_color,
            } => {
                let (rainbow, color) = match clock_color {
                    Some(string) => {
                        if string.to_uppercase() == "RAINBOW" {
                            (true, None)
                        } else {
                            if string.len() > 6 {
                                bail!(
                                    "clock-color should be of the format < RAINBOW | ###### (rgb hex) >"
                                )
                            }

                            let parsed = u32::from_str_radix(&string, 16)?;
                            (
                                false,
                                Some([
                                    ((parsed >> 16) & 0xFF) as f32 / 255.0,
                                    ((parsed >> 8) & 0xFF) as f32 / 255.0,
                                    (parsed & 0xFF) as f32 / 255.0,
                                ]),
                            )
                        }
                    }
                    None => (false, None),
                };

                Ok(BackgroundRenderer::ClockImage {
                    dir,
                    file_template,
                    clock_step,
                    buffered_images: VecDeque::new(),
                    rainbow,
                    color,
                })
            }
            _ => Ok(BackgroundRenderer::None),
        }
    }
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    match args.command {
        Command::Start {
            width,
            height,
            window_class,
        } => {
            let socket = LocalSocketListener::bind(args.socket_name)?;
            socket.set_nonblocking(true)?;

            run(
                &window_class,
                width,
                height,
                BackgroundRenderer::None,
                socket,
            )?;
        }
        command => {
            let mut socket = LocalSocketStream::connect(args.socket_name)?;
            bincode::serialize_into(&mut socket, &command)?;
            socket.flush()?;
            drop(socket);
        }
    }

    Ok(())
}

fn run(
    window_class: &str,
    width: u32,
    height: u32,
    mut renderer: BackgroundRenderer,
    socket: LocalSocketListener,
) -> anyhow::Result<()> {
    let event_loop = EventLoopBuilder::new().with_wayland().build().unwrap();
    let window = WindowBuilder::new()
        .with_name(window_class, window_class)
        .build(&event_loop)
        .unwrap();

    let surface_texture = SurfaceTexture::new(width, height, &window);
    let mut pixels = PixelsBuilder::new(width, height, surface_texture)
        .request_adapter_options(RequestAdapterOptions {
            power_preference: pixels::wgpu::PowerPreference::LowPower,
            force_fallback_adapter: false,
            compatible_surface: None,
        })
        .enable_vsync(true)
        .build()
        .unwrap();

    event_loop
        .run(move |event, elwt| match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => elwt.exit(),
            Event::AboutToWait => {
                match socket.accept() {
                    Ok(stream) => {
                        match bincode::deserialize_from::<_, Command>(stream) {
                            Ok(Command::Stop) => {
                                elwt.exit();
                            }
                            Ok(command) => {
                                renderer = command
                                    .into_renderer(&mut pixels, width, height)
                                    .unwrap_or_else(|e| {
                                        eprintln!("{e}");
                                        elwt.exit();
                                        BackgroundRenderer::None
                                    });
                            }
                            Err(error) => {
                                eprintln!("{error}");
                                elwt.exit();
                            }
                        };
                    }
                    Err(error) => match error.kind() {
                        std::io::ErrorKind::WouldBlock => {}
                        _ => {
                            eprintln!("{error}");
                            elwt.exit();
                        }
                    },
                }

                renderer
                    .render(&mut pixels, width, height)
                    .unwrap_or_else(|e| {
                        eprintln!("{e}");
                        elwt.exit();
                    });
                pixels.render().unwrap();
                elwt.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(
                    Instant::now() + Duration::from_millis(TICK_RATE),
                ));
            }
            _ => {}
        })
        .unwrap();

    Ok(())
}
