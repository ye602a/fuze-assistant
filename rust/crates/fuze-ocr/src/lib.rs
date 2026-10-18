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
pub mod reuse;
pub mod selection;
mod title_matcher;
use backend::Backend;
pub struct PreparedTitle {
    pub width: usize,
    pub data: Vec<f32>,
}
pub struct Recognizer {
    session: Session,
    dictionary: Vec<String>,
    catalog: title_matcher::Matcher,
    fixed_width: Option<usize>,
    warmed: bool,
    #[cfg(windows)]
    pub adapter: Option<fuze_capture::adapter::Adapter>,
}
#[derive(Serialize)]
pub struct FrameResult {
    pub slots: [Slot; 3],
    pub title_kind: fuze_contract::wire::TitleKind,
    pub title_reads: [fuze_contract::wire::TitleRead; 3],
    pub elapsed_ms: f64,
    pub selection_confirmed: bool,
}

// Locate one title line inside the legacy title envelope, without another inference or
// full-screen text fallback. The larger glyph band wins over the small category label.
// Only the three title crops are scanned; preserve card order and glyph aspect ratio.
fn erase_persistent_frame_columns(mut crop: RgbImage) -> RgbImage {
    let (w, h) = crop.dimensions();
    if w < 16 || h < 8 {
        return crop;
    }
    let edge = w * 8 / 100;
    for x in edge..w - edge {
        let bright = (0..h)
            .filter(|&y| {
                let p = crop.get_pixel(x, y);
                p[0] >= 160 && p[1] >= 160 && p[2] >= 130
            })
            .count() as u32;
        if bright >= h * 4 / 5 {
            for y in 0..h {
                crop.put_pixel(x, y, image::Rgb([0, 0, 0]));
            }
        }
    }
    crop
}

fn trim_title_padding(crop: RgbImage) -> RgbImage {
    // Card borders can stay bright for every row and masquerade as a full-height title band.
    let crop = trim_title_rows(erase_persistent_frame_columns(crop));
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

fn trim_title_rows(crop: RgbImage) -> RgbImage {
    let (w, h) = crop.dimensions();
    let edge = w * 8 / 100;
    let inner = w - edge * 2;
    let gap = (h / 50).max(1);
    let mut bands = Vec::new();
    let mut start = None;
    let mut last = 0;
    for y in 0..h {
        let count = (edge..w - edge)
            .filter(|&x| {
                let p = crop.get_pixel(x, y);
                p[0] >= 160 && p[1] >= 160 && p[2] >= 130
            })
            .count() as u32;
        let active = count >= (w / 100).max(3) && count < inner * 70 / 100;
        if active {
            start.get_or_insert(y);
            last = y;
        } else if start.is_some() && y > last + gap {
            bands.push((start.take().unwrap(), last + 1));
        }
    }
    if let Some(start) = start {
        bands.push((start, last + 1));
    }
    let band = bands
        .into_iter()
        .filter(|(top, bottom)| bottom - top >= (h / 30).max(3) && bottom - top <= h * 3 / 4)
        .max_by_key(|(top, bottom)| {
            (bottom - top) as i64 * 100 - ((top + bottom) as i64 - h as i64).abs()
        });
    if let Some((top, bottom)) = band {
        let padding = ((bottom - top) / 3).max(2);
        let top = top.saturating_sub(padding);
        let bottom = (bottom + padding).min(h);
        image::imageops::crop_imm(&crop, 0, top, w, bottom - top).to_image()
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
        Self::build(model, dictionary, catalog, backend, None)
    }
    fn build(
        model: &Path,
        dictionary: Vec<String>,
        catalog: HashMap<String, u32>,
        backend: Backend,
        profile: Option<&Path>,
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
        // Offline A/B switch; stable application does not import this prototype.
        let sizing = std::env::var("FUZE_RUST_DML_SHAPE").unwrap_or_else(|_| "fixed512".into());
        if !["dynamic", "padded512", "fixed512"].contains(&sizing.as_str()) {
            return Err("invalid-shape-mode".into());
        }
        let fixed_width = (backend != Backend::Cpu && sizing != "dynamic").then_some(512);
        #[cfg(windows)]
        let adapter = if backend != Backend::Cpu {
            Some(
                fuze_capture::adapter::enumerate()?
                    .into_iter()
                    .find(|a| a.index == 0 && !a.software)
                    .ok_or("hardware-adapter-unavailable")?,
            )
        } else {
            None
        };
        if backend != Backend::Cpu {
            builder = builder
                .with_memory_pattern(false)?
                .with_execution_providers([DirectMLExecutionProvider::default()
                    .with_device_id(0)
                    .build()
                    .error_on_failure()])?;
            if sizing == "fixed512" {
                builder = builder
                    .with_dimension_override("DynamicDimension.0", 1)?
                    .with_dimension_override("DynamicDimension.1", 512)?;
            }
        }
        if backend == Backend::DirectMlStrict {
            builder = builder.with_config_entry("session.disable_cpu_ep_fallback", "1")?;
        }
        if let Some(file) = profile {
            builder = builder.with_profiling(file)?;
        }
        let session = builder.commit_from_file(model)?;
        if backend != Backend::Cpu
            && sizing == "fixed512"
            && !session
                .inputs
                .first()
                .and_then(|v| v.input_type.tensor_shape())
                .is_some_and(|shape| **shape == [1, 3, 48, 512])
        {
            return Err("static-input-shape-not-applied".into());
        }
        Ok(Self {
            session,
            dictionary,
            catalog: title_matcher::Matcher::new(catalog),
            fixed_width,
            warmed: false,
            #[cfg(windows)]
            adapter,
        })
    }
    // Explicit offline audit only, separate from all timing sessions. No per-frame profiling.
    pub fn audit_provider(
        model: &Path,
        dictionary: Vec<String>,
        catalog: HashMap<String, u32>,
        directory: &Path,
    ) -> Result<serde_json::Value> {
        std::fs::create_dir_all(directory)?;
        let prefix = directory.join(format!("provider-audit-{}", std::process::id()));
        let mut engine = Self::build(model, dictionary, catalog, Backend::DirectMl, Some(&prefix))?;
        engine.warm_up()?;
        let file = engine.session.end_profiling()?;
        if std::fs::metadata(&file)?.len() > 8 * 1024 * 1024 {
            return Err("profile-too-large".into());
        }
        let events: Vec<serde_json::Value> = serde_json::from_slice(&std::fs::read(file)?)?;
        let (mut dml, mut cpu, mut other) = (0u64, 0u64, 0u64);
        let (mut dml_us, mut cpu_us) = (0.0, 0.0);
        for event in events {
            let duration = event["dur"].as_f64().unwrap_or(0.0);
            match event["args"]["provider"].as_str() {
                Some("DmlExecutionProvider") => {
                    dml += 1;
                    dml_us += duration;
                }
                Some("CPUExecutionProvider") => {
                    cpu += 1;
                    cpu_us += duration;
                }
                Some(_) => other += 1,
                None => {}
            }
        }
        Ok(
            serde_json::json!({"dml_events":dml,"cpu_events":cpu,"other_provider_events":other,"dml_event_us":dml_us,"cpu_event_us":cpu_us,"dml_execution_observed":dml>0}),
        )
    }
    pub fn warm_up(&mut self) -> Result<()> {
        if self.warmed {
            return Ok(());
        }
        let width = self.fixed_width.unwrap_or(256);
        // The second run also completes deferred GPU preparation before the first real frame.
        for _ in 0..if self.fixed_width.is_some() { 2 } else { 1 } {
            let tensor = Tensor::from_array(([1usize, 3, 48, width], vec![0f32; 3 * 48 * width]))?;
            self.session.run(ort::inputs![tensor])?;
        }
        self.warmed = true;
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
        let padded = pad_input(input, self.fixed_width)?;
        let input = padded.as_ref().unwrap_or(input);
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
        Ok((
            match self.catalog.read(&name, confidence).0 {
                Slot::Known { id, .. } => Some(id),
                Slot::Unreadable => None,
            },
            elapsed,
            cpu_elapsed,
        ))
    }
    fn recognize_crop(&mut self, crop: RgbImage) -> Result<(String, f32)> {
        let input = Self::prepare_crop(crop);
        let padded = pad_input(&input, self.fixed_width)?;
        let input = padded.as_ref().unwrap_or(&input);
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
        Ok(fuze_contract::titles::regions(w, h)?
            .into_iter()
            .map(|r| {
                Self::prepare_crop(trim_title_padding(
                    image::imageops::crop_imm(image, r.x, r.y, r.width, r.height).to_image(),
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
    pub fn recognize_atlas(
        &mut self,
        atlas: &fuze_contract::titles::Atlas<'_>,
    ) -> Result<FrameResult> {
        self.recognize_crops(|i| {
            RgbImage::from_raw(atlas.title_width, atlas.title_height, atlas.rgb_slot(i)?)
                .ok_or_else(|| "invalid-title-buffer".into())
        })
    }
    fn recognize_rois(
        &mut self,
        w: u32,
        h: u32,
        mut crop: impl FnMut(u32, u32, u32, u32) -> Result<RgbImage>,
    ) -> Result<FrameResult> {
        let regions = fuze_contract::titles::regions(w, h)?;
        self.recognize_crops(|i| {
            let r = regions[i];
            crop(r.x, r.y, r.width, r.height)
        })
    }
    fn recognize_crops(
        &mut self,
        mut crop: impl FnMut(usize) -> Result<RgbImage>,
    ) -> Result<FrameResult> {
        let started = Instant::now();
        let mut slots = [Slot::Unreadable, Slot::Unreadable, Slot::Unreadable];
        let mut shards = [false; 3];
        let mut title_reads = [fuze_contract::wire::TitleRead::Unknown; 3];
        // One recognition pass per card; locate its title inside the bounded envelope.
        for (i, slot) in slots.iter_mut().enumerate() {
            let (name, confidence) = self.recognize_crop(trim_title_padding(crop(i)?))?;
            let shard_name = selection::shard_title(&name, 1.0);
            shards[i] = shard_name && confidence.is_finite() && confidence >= 0.9;
            if shard_name {
                title_reads[i] = fuze_contract::wire::TitleRead::StatShard;
            } else {
                (*slot, title_reads[i]) = self.catalog.read(&name, confidence);
            }
        }
        Ok(FrameResult {
            title_kind: selection::classify(&slots, shards),
            title_reads,
            slots,
            elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
            selection_confirmed: false,
        })
    }
}
// Preserve glyph aspect ratio and vertical geometry. Never squeeze/crop an oversized title.
// Auto mode can drop the GPU session and retry on CPU through the existing one-shot fallback.
fn pad_input(input: &PreparedTitle, fixed: Option<usize>) -> Result<Option<PreparedTitle>> {
    if !(16..=1024).contains(&input.width) || input.data.len() != 3 * 48 * input.width {
        return Err("invalid-prepared-input".into());
    }
    let Some(width) = fixed else { return Ok(None) };
    if input.width > width {
        return Err("title-exceeds-fixed-width".into());
    }
    if input.width == width {
        return Ok(None);
    }
    let mut data = vec![0.; 3 * 48 * width];
    for (src, dst) in input
        .data
        .chunks_exact(input.width)
        .zip(data.chunks_exact_mut(width))
    {
        dst[..src.len()].copy_from_slice(src);
    }
    Ok(Some(PreparedTitle { width, data }))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn persistent_card_edge_does_not_hide_a_short_title_row() {
        let mut crop = RgbImage::from_pixel(165, 55, image::Rgb([0, 0, 0]));
        for y in 0..55 {
            for x in 13..=15 {
                crop.put_pixel(x, y, image::Rgb([230, 230, 210]));
            }
        }
        for y in 30..40 {
            for x in 58..89 {
                crop.put_pixel(x, y, image::Rgb([220, 220, 200]));
            }
        }
        let title = trim_title_padding(crop);
        assert!(title.height() < 25);
        assert!(title.width() < 100);
        assert!(title.pixels().any(|p| p[0] >= 200));
    }
    #[test]
    fn padding_preserves_channels_and_rejects_overflow() {
        let input = PreparedTitle {
            width: 32,
            data: (0..3 * 48 * 32).map(|v| v as f32).collect(),
        };
        let padded = pad_input(&input, Some(512)).unwrap().unwrap();
        for row in 0..3 * 48 {
            assert_eq!(
                &padded.data[row * 512..row * 512 + 32],
                &input.data[row * 32..(row + 1) * 32]
            );
            assert!(padded.data[row * 512 + 32..(row + 1) * 512]
                .iter()
                .all(|v| *v == 0.));
        }
        assert!(pad_input(&input, Some(16)).is_err());
        assert!(pad_input(&input, None).unwrap().is_none());
    }
    #[test]
    fn ctc_blank_separates_repeated_characters_and_rejects_bad_shape() {
        let dictionary = vec!["".into(), "甲".into(), "乙".into()];
        let values = [0., 1., 0., 0., 1., 0., 1., 0., 0., 0., 1., 0., 0., 0., 1.];
        assert_eq!(decode_ctc(&values, 3, &dictionary).unwrap().0, "甲甲乙");
        assert!(decode_ctc(&values, 4, &dictionary).is_err());
    }
}
