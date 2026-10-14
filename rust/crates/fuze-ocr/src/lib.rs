use fuze_contract::Slot;
use image::{imageops::FilterType, RgbImage};
use ort::{
    execution_providers::DirectMLExecutionProvider,
    session::Session,
    value::{Tensor, TensorRef},
};
use serde::Serialize;
use std::{collections::HashMap, path::Path, time::Instant};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
pub mod backend;
mod metrics;
use backend::Backend;
pub struct PreparedTitle {
    pub width: usize,
    pub data: Vec<f32>,
}
pub struct Recognizer {
    session: Session,
    dictionary: Vec<String>,
    catalog: HashMap<String, u32>,
}
#[derive(Serialize)]
pub struct FrameResult {
    pub slots: [Slot; 3],
    pub elapsed_ms: f64,
    pub selection_confirmed: bool,
}

// Trim horizontal background inside an already fixed title ROI. This is not a popup detector.
// Ignore card-edge bands; retain generous padding and the original vertical geometry.
fn trim_title_padding(crop: RgbImage) -> RgbImage {
    let (w, h) = crop.dimensions();
    let edge = w * 8 / 100;
    let mut first = w;
    let mut last = 0;
    for x in edge..w - edge {
        let bright = (0..h)
            .filter(|&y| {
                let p = crop.get_pixel(x, y);
                p[0] >= 160 && p[1] >= 160 && p[2] >= 130
            })
            .count() as u32;
        if bright >= 3 && bright < h * 85 / 100 {
            first = first.min(x);
            last = last.max(x);
        }
    }
    if first < last && last - first >= 12 {
        let left = first.saturating_sub(h / 2);
        let right = (last + 1 + h / 2).min(w);
        image::imageops::crop_imm(&crop, left, 0, right - left, h).to_image()
    } else {
        crop
    }
}

pub fn decode_ctc(values: &[f32], classes: usize, dictionary: &[String]) -> Result<(String, f32)> {
    if classes < 2 || classes != dictionary.len() || values.len() % classes != 0 {
        return Err("invalid-output-shape".into());
    }
    let mut text = String::new();
    let mut score = 0.0;
    let mut count = 0;
    let mut last = 0;
    for row in values.chunks_exact(classes) {
        if row.iter().any(|v| !v.is_finite()) {
            return Err("invalid-output-value".into());
        }
        let (index, value) = row
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .unwrap();
        if index != 0 && index != last {
            text.push_str(&dictionary[index]);
            score += value;
            count += 1;
        }
        last = index;
    }
    Ok((text, if count > 0 { score / count as f32 } else { 0.0 }))
}
impl Recognizer {
    pub fn new(
        model: &Path,
        dictionary: Vec<String>,
        catalog: HashMap<String, u32>,
    ) -> Result<Self> {
        Self::with_backend(model, dictionary, catalog, Backend::Cpu)
    }
    pub fn with_backend(
        model: &Path,
        dictionary: Vec<String>,
        catalog: HashMap<String, u32>,
        backend: Backend,
    ) -> Result<Self> {
        if dictionary.len() < 2 || dictionary.len() > 20000 || catalog.len() > 10000 {
            return Err("invalid-dictionary".into());
        }
        let mut builder = Session::builder()?
            .with_intra_threads(1)?
            .with_inter_threads(1)?
            .with_parallel_execution(false)?
            .with_config_entry("session.intra_op.allow_spinning", "0")?
            .with_config_entry("session.inter_op.allow_spinning", "0")?;
        if backend == Backend::DirectMl {
            builder = builder
                .with_memory_pattern(false)?
                .with_config_entry("session.disable_cpu_ep_fallback", "1")?
                .with_execution_providers([DirectMLExecutionProvider::default()
                    .build()
                    .error_on_failure()])?;
        }
        let session = builder.commit_from_file(model)?;
        Ok(Self {
            session,
            dictionary,
            catalog,
        })
    }
    pub fn warm_up(&mut self) -> Result<()> {
        let tensor = Tensor::from_array(([1usize, 3, 48, 256], vec![0f32; 3 * 48 * 256]))?;
        self.session.run(ort::inputs![tensor])?;
        Ok(())
    }
    fn prepare_crop(crop: RgbImage) -> PreparedTitle {
        let width =
            ((crop.width() as f64 * 48.0 / crop.height() as f64).round() as u32).clamp(16, 1024);
        let resized = image::imageops::resize(&crop, width, 48, FilterType::Triangle);
        let plane = (width * 48) as usize;
        let mut input = vec![0f32; 3 * plane];
        for (i, p) in resized.pixels().enumerate() {
            for c in 0..3 {
                input[c * plane + i] = p[c] as f32 / 127.5 - 1.0;
            }
        }
        PreparedTitle {
            width: width as usize,
            data: input,
        }
    }
    pub fn infer_prepared(
        &mut self,
        input: &PreparedTitle,
    ) -> Result<(Option<u32>, f64, Option<f64>)> {
        let cpu = metrics::cpu_ms();
        let started = Instant::now();
        let tensor =
            TensorRef::from_array_view(([1usize, 3, 48, input.width], input.data.as_slice()))?;
        let result = self.session.run(ort::inputs![tensor])?;
        let elapsed = started.elapsed().as_secs_f64() * 1000.0;
        let cpu_elapsed = metrics::cpu_ms().zip(cpu).map(|(end, start)| end - start);
        let (shape, values) = result[0].try_extract_tensor::<f32>()?;
        if shape.len() != 3 || shape[0] != 1 {
            return Err("invalid-output-shape".into());
        }
        let (name, confidence) = decode_ctc(values, shape[2] as usize, &self.dictionary)?;
        let normalized: String = name.chars().filter(|c| !c.is_whitespace()).collect();
        Ok((
            if confidence >= 0.75 {
                self.catalog.get(&normalized).copied()
            } else {
                None
            },
            elapsed,
            cpu_elapsed,
        ))
    }
    fn recognize_crop(&mut self, crop: RgbImage) -> Result<(String, f32)> {
        let input = Self::prepare_crop(crop);
        let tensor =
            TensorRef::from_array_view(([1usize, 3, 48, input.width], input.data.as_slice()))?;
        let result = self.session.run(ort::inputs![tensor])?;
        let (shape, values) = result[0].try_extract_tensor::<f32>()?;
        if shape.len() != 3 || shape[0] != 1 {
            return Err("invalid-output-shape".into());
        }
        decode_ctc(values, shape[2] as usize, &self.dictionary)
    }
    pub fn prepare_frame(image: &RgbImage) -> Result<Vec<PreparedTitle>> {
        let (w, h) = image.dimensions();
        if w < 320 || h < 180 || w > 4096 || h > 2160 {
            return Err("invalid-frame-size".into());
        }
        Ok((0..3)
            .map(|i| {
                let x = ((0.2305 + i as f64 * 0.1855 + 0.01344) * w as f64).round() as u32;
                let y = (0.385 * h as f64).round() as u32;
                let width = (0.14112 * w as f64).round() as u32;
                let height = (0.03 * h as f64).round().max(1.0) as u32;
                Self::prepare_crop(trim_title_padding(
                    image::imageops::crop_imm(image, x, y, width, height).to_image(),
                ))
            })
            .collect())
    }
    pub fn recognize(&mut self, image: &RgbImage) -> Result<FrameResult> {
        let (w, h) = image.dimensions();
        self.recognize_rois(w, h, |x, y, width, height| {
            Ok(image::imageops::crop_imm(image, x, y, width, height).to_image())
        })
    }
    pub fn recognize_bgra(
        &mut self,
        frame: &fuze_contract::frame::BgraFrame<'_>,
    ) -> Result<FrameResult> {
        self.recognize_rois(
            frame.width as u32,
            frame.height as u32,
            |x, y, width, height| {
                let bytes =
                    frame.rgb_roi(x as usize, y as usize, width as usize, height as usize)?;
                RgbImage::from_raw(width, height, bytes).ok_or_else(|| "invalid-roi-buffer".into())
            },
        )
    }
    fn recognize_rois(
        &mut self,
        w: u32,
        h: u32,
        mut crop: impl FnMut(u32, u32, u32, u32) -> Result<RgbImage>,
    ) -> Result<FrameResult> {
        if w < 320 || h < 180 || w > 4096 || h > 2160 {
            return Err("invalid-frame-size".into());
        }
        let started = Instant::now();
        let mut slots = [Slot::Unreadable, Slot::Unreadable, Slot::Unreadable];
        // Prototype fixed single-line title geometry, not a generic text detector.
        for (i, slot) in slots.iter_mut().enumerate() {
            let x = ((0.2305 + i as f64 * 0.1855 + 0.01344) * w as f64).round() as u32;
            let y = (0.385 * h as f64).round() as u32;
            let width = (0.14112 * w as f64).round() as u32;
            let height = (0.03 * h as f64).round().max(1.0) as u32;
            let (name, confidence) =
                self.recognize_crop(trim_title_padding(crop(x, y, width, height)?))?;
            let normalized: String = name.chars().filter(|c| !c.is_whitespace()).collect();
            if confidence >= 0.75 {
                if let Some(&id) = self.catalog.get(&normalized) {
                    *slot = Slot::Known {
                        id,
                        name: normalized,
                        confidence,
                    };
                }
            }
        }
        Ok(FrameResult {
            slots,
            elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
            selection_confirmed: false,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ctc_blank_separates_repeated_characters_and_rejects_bad_shape() {
        let dictionary = vec!["".into(), "甲".into(), "乙".into()];
        let values = [0., 1., 0., 0., 1., 0., 1., 0., 0., 0., 1., 0., 0., 0., 1.];
        assert_eq!(decode_ctc(&values, 3, &dictionary).unwrap().0, "甲甲乙");
        assert!(decode_ctc(&values, 4, &dictionary).is_err());
    }
}
