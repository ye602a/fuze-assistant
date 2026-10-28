//! Optional visual fixture inputs. Invoked only by the explicitly hidden --smoke entry point.
use super::*;
pub(super) fn prepare() -> Result<()> {
    let Some(file) = std::env::var_os("FUZE_OVERLAY_REVIEW") else {
        return Ok(());
    };
    if std::fs::metadata(&file)?.len() > 16 * 1024 {
        return Err("review-limit".into());
    }
    let fixture: serde_json::Value = serde_json::from_slice(&std::fs::read(file)?)?;
    let rows = fixture
        .as_array()
        .filter(|r| r.len() <= 3)
        .ok_or("review-rows")?;
    let mut stats = std::collections::HashMap::new();
    let mut icons = std::collections::HashMap::new();
    for row in rows {
        let id = u32::try_from(row["id"].as_u64().ok_or("review-id")?)?;
        let rate = |key| {
            row[key]
                .as_f64()
                .filter(|v| v.is_finite() && (0.0..=1.0).contains(v))
        };
        stats.insert(
            id,
            fuze_data::recommendations::Recommendation {
                evidence: Default::default(),
                win_rate: rate("win"),
                pick_rate: rate("pick"),
                score: rate("score"),
                origin: "fixture".into(),
                region: "TEST".into(),
                patch: "fixture".into(),
            },
        );
        if let Some(file) = row["image"].as_str() {
            if std::fs::metadata(file)?.len() > 512 * 1024 {
                return Err("review-image-limit".into());
            }
            let bytes = std::fs::read(file)?;
            let mut reader = image::ImageReader::with_format(
                std::io::Cursor::new(bytes),
                image::ImageFormat::Png,
            );
            let mut limits = image::Limits::default();
            limits.max_image_width = Some(512);
            limits.max_image_height = Some(512);
            limits.max_alloc = Some(4 * 1024 * 1024);
            reader.limits(limits);
            let mut rgba = reader
                .decode()?
                .resize_exact(48, 48, image::imageops::FilterType::Triangle)
                .to_rgba8()
                .into_raw();
            for p in rgba.chunks_exact_mut(4) {
                p.swap(0, 2);
            }
            icons.insert(id, std::sync::Arc::new(rgba));
        }
    }
    STATS.with(|s| *s.borrow_mut() = stats);
    ICONS.with(|s| *s.borrow_mut() = icons);
    Ok(())
}
