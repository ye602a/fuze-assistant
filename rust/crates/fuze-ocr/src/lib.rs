use fuze_contract::Slot;
use image::{imageops::FilterType, RgbImage};
use ort::{session::Session, value::Tensor};
use serde::Serialize;
use std::{collections::HashMap, path::Path, time::Instant};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
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
        if dictionary.len() < 2 || dictionary.len() > 20000 || catalog.len() > 10000 {
            return Err("invalid-dictionary".into());
        }
        let session = Session::builder()?
            .with_intra_threads(1)?
            .with_inter_threads(1)?
            .with_parallel_execution(false)?
            .with_config_entry("session.intra_op.allow_spinning", "0")?
            .with_config_entry("session.inter_op.allow_spinning", "0")?
            .commit_from_file(model)?;
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
    fn recognize_crop(&mut self, crop: RgbImage) -> Result<(String, f32)> {
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
        let tensor = Tensor::from_array(([1usize, 3, 48, width as usize], input))?;
        let result = self.session.run(ort::inputs![tensor])?;
        let (shape, values) = result[0].try_extract_tensor::<f32>()?;
        if shape.len() != 3 || shape[0] != 1 {
            return Err("invalid-output-shape".into());
        }
        decode_ctc(values, shape[2] as usize, &self.dictionary)
    }
    pub fn recognize(&mut self, image: &RgbImage) -> Result<FrameResult> {
        let (w, h) = image.dimensions();
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
            let crop = image::imageops::crop_imm(image, x, y, width, height).to_image();
            let (name, confidence) = self.recognize_crop(crop)?;
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
