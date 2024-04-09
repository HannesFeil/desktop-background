use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
};

use anyhow::bail;
use chrono::{Local, Timelike};
use clap::{Parser, Subcommand};
use color::{color_space::Srgb, Deg, Hsv, ToRgb};
use image::RgbaImage;
use pixels::{wgpu::RequestAdapterOptions, Pixels, PixelsBuilder, SurfaceTexture};
use winit::{
    event::{Event, WindowEvent},
    event_loop::EventLoopBuilder,
    platform::wayland::{EventLoopBuilderExtWayland, WindowBuilderExtWayland},
    window::WindowBuilder,
};

const PRE_BUFFERED_IMAGES: usize = 10;
const MILLIS_PER_SECOND: u32 = 1000;
const MILLIS_PER_MINUTE: u32 = 60 * MILLIS_PER_SECOND;
const MILLIS_PER_HOUR: u32 = 60 * MILLIS_PER_MINUTE;
const MILLIS_TOTAL: u32 = 12 * MILLIS_PER_HOUR;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Args {
    /// Desktop resolution width in pixels
    #[arg()]
    width: u32,
    /// Desktop resolution height in pixels
    #[arg()]
    height: u32,
    /// Window class name
    #[arg()]
    window_class: String,
    /// Background
    #[command(subcommand)]
    background: Background,
}

#[derive(Debug, Clone, Subcommand)]
enum Background {
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

impl Background {
    pub fn into_renderer(self, width: u32, height: u32) -> anyhow::Result<BackgroundRenderer> {
        match self {
            Background::StaticImage { path } => {
                let image = image::imageops::resize(
                    &image::open(path)?,
                    width,
                    height,
                    image::imageops::FilterType::Triangle,
                );
                Ok(BackgroundRenderer::StaticImage { image })
            }
            Background::ClockImage {
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

                let millis = clock_millis(clock_step);
                let image = load_clock_image(&dir, &file_template, millis, width, height)?;
                let mut buffered_images = VecDeque::with_capacity(PRE_BUFFERED_IMAGES);
                for _ in 0..PRE_BUFFERED_IMAGES {
                    buffered_images.push_front((millis, image.clone()));
                }

                Ok(BackgroundRenderer::ClockImage {
                    dir,
                    file_template,
                    clock_step,
                    buffered_images,
                    rainbow,
                    color,
                })
            }
        }
    }
}

enum BackgroundRenderer {
    StaticImage {
        image: RgbaImage,
    },
    ClockImage {
        dir: PathBuf,
        file_template: String,
        clock_step: u32,
        buffered_images: VecDeque<(u32, RgbaImage)>,
        rainbow: bool,
        color: Option<[f32; 3]>,
    },
}

impl BackgroundRenderer {
    pub fn render(&mut self, pixels: &mut Pixels, width: u32, height: u32) -> anyhow::Result<()> {
        match self {
            BackgroundRenderer::StaticImage { image } => {
                pixels.frame_mut().copy_from_slice(image);
            }
            BackgroundRenderer::ClockImage {
                dir,
                file_template,
                clock_step,
                buffered_images,
                rainbow,
                color,
            } => {
                let mut current_millis = clock_millis(*clock_step);
                current_millis = (current_millis + *clock_step * (PRE_BUFFERED_IMAGES as u32 - 1))
                    % MILLIS_TOTAL;

                assert!(!buffered_images.is_empty());

                if current_millis - buffered_images.front().unwrap().0 >= *clock_step {
                    if buffered_images.len() >= PRE_BUFFERED_IMAGES {
                        buffered_images.pop_back();
                    }

                    let image =
                        load_clock_image(dir, file_template, current_millis, width, height)?;
                    buffered_images.push_front((current_millis, image));
                }

                let color = if *rainbow {
                    Some(
                        *Hsv::<f32, Srgb>::new(
                            Deg(current_millis as f32 / MILLIS_TOTAL as f32 * 360.0),
                            1.0,
                            1.0,
                        )
                        .to_rgb::<f32>()
                        .as_ref(),
                    )
                } else {
                    color.map(|c| c)
                };

                if let Some(color) = color {
                    pixels
                        .frame_mut()
                        .iter_mut()
                        .zip(buffered_images.back().unwrap().1.iter())
                        .enumerate()
                        .for_each(|(idx, (dst, src))| {
                            if (idx + 1) % 4 == 0 {
                                *dst = 255;
                            } else {
                                *dst = (*src as f32 * color[idx % 4]) as u8;
                            }
                        });
                } else {
                    pixels
                        .frame_mut()
                        .copy_from_slice(&buffered_images.back().unwrap().1)
                }
            }
        }
        Ok(())
    }
}

fn clock_millis(clock_step: u32) -> u32 {
    let now = Local::now();
    let time = now.time();
    (((time.hour() % 12) * MILLIS_PER_HOUR
        + time.minute() * MILLIS_PER_MINUTE
        + time.second() * MILLIS_PER_SECOND
        + now.timestamp_subsec_millis())
        / clock_step)
        * clock_step
}

fn load_clock_image(
    dir: &Path,
    file_template: &str,
    millis: u32,
    width: u32,
    height: u32,
) -> anyhow::Result<RgbaImage> {
    let mut path = dir.to_path_buf();
    path.push(format!(
        "{hour}/{file}",
        hour = millis / MILLIS_PER_HOUR,
        file = file_template.replace("%m", &format!("{millis:08}")),
    ));
    Ok(image::imageops::resize(
        &image::open(path)?,
        width,
        height,
        image::imageops::FilterType::Triangle,
    ))
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let event_loop = EventLoopBuilder::new().with_wayland().build().unwrap();
    let window = WindowBuilder::new()
        .with_name(&args.window_class, &args.window_class)
        .build(&event_loop)
        .unwrap();

    let surface_texture = SurfaceTexture::new(args.width, args.height, &window);
    let mut pixels = PixelsBuilder::new(args.width, args.height, surface_texture)
        .request_adapter_options(RequestAdapterOptions {
            power_preference: pixels::wgpu::PowerPreference::LowPower,
            force_fallback_adapter: false,
            compatible_surface: None,
        })
        .enable_vsync(true)
        .build()
        .unwrap();

    let mut renderer = args.background.into_renderer(args.width, args.height)?;

    event_loop
        .run(move |event, elwt| match event {
            Event::WindowEvent { event, .. } => match event {
                WindowEvent::CloseRequested => elwt.exit(),
                WindowEvent::RedrawRequested => {
                    renderer
                        .render(&mut pixels, args.width, args.height)
                        .unwrap_or_else(|e| {
                            eprintln!("{e}");
                            elwt.exit();
                        });
                    pixels.render().unwrap();
                    window.request_redraw();
                }
                _ => {}
            },
            Event::LoopExiting => println!("bye!"),
            _ => {}
        })
        .unwrap();

    Ok(())
}
