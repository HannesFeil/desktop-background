use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
};

use chrono::{Local, Timelike};
use color::{color_space::Srgb, Deg, Hsv, ToRgb};
use image::RgbaImage;
use pixels::Pixels;

const PRE_BUFFERED_IMAGES: usize = 10;
const MILLIS_PER_SECOND: u32 = 1000;
const MILLIS_PER_MINUTE: u32 = 60 * MILLIS_PER_SECOND;
const MILLIS_PER_HOUR: u32 = 60 * MILLIS_PER_MINUTE;
const MILLIS_TOTAL: u32 = 12 * MILLIS_PER_HOUR;

pub enum BackgroundRenderer {
    None,
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
            BackgroundRenderer::None => {}
            BackgroundRenderer::ClockImage {
                dir,
                file_template,
                clock_step,
                buffered_images,
                rainbow,
                color,
            } => {
                let current_millis = clock_millis(*clock_step);
                let mut redraw = false;

                while buffered_images
                    .back()
                    .is_some_and(|(time, _)| time.abs_diff(current_millis) >= *clock_step)
                {
                    buffered_images.pop_back();
                }

                while buffered_images.len() < PRE_BUFFERED_IMAGES {
                    redraw = true;

                    let image_millis = buffered_images
                        .front()
                        .map(|t| (t.0 + *clock_step) % MILLIS_TOTAL)
                        .unwrap_or(current_millis);

                    let image = load_clock_image(dir, file_template, image_millis, width, height)?;

                    buffered_images.push_front((image_millis, image));
                }

                if redraw {
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
