//! v0.6.3 title + reroll-button screen gate. Bounded pixel sampling, no OCR/encoding.
use crate::titles::{regions, Atlas, Region};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Evidence {
    Visible,
    Partial,
    Absent,
    Uncertain,
}

pub fn buttons(width: u32, height: u32) -> Result<[Region; 3], &'static str> {
    regions(width, height)?;
    Ok(std::array::from_fn(|i| Region {
        x: ((0.2305 + i as f64 * 0.1855 + 0.168 * 0.56) * width as f64).round() as u32,
        y: (0.63 * height as f64).round() as u32,
        width: (0.168 * 0.34 * width as f64).round().max(1.) as u32,
        height: (0.045 * height as f64).round().max(1.) as u32,
    }))
}

fn sample(atlas: &Atlas<'_>, slot: usize, mut take: impl FnMut(u8, u8, u8)) -> usize {
    let w = atlas.title_width as usize;
    let h = atlas.title_height as usize;
    let mut count = 0;
    for y in (0..h).step_by((h / 48).max(1)) {
        for x in (0..w).step_by((w / 120).max(1)) {
            let i = (y * w * 3 + slot * w + x) * 4;
            let p = &atlas.bytes()[i..i + 3];
            take(p[2], p[1], p[0]);
            count += 1;
        }
    }
    count
}

pub fn classify(titles: &Atlas<'_>, buttons: &Atlas<'_>) -> Evidence {
    let mut title_slots = 0;
    let mut button_slots = 0;
    for slot in 0..3 {
        let (mut bright, mut dark) = (0, 0);
        let n = sample(titles, slot, |r, g, b| {
            bright += usize::from(r > 180 && g > 180 && b > 170);
            dark += usize::from(r.max(g).max(b) < 55);
        }) as f64;
        title_slots += usize::from(bright as f64 / n >= 0.004 && dark as f64 / n >= 0.72);
        let mut button = 0;
        let n = sample(buttons, slot, |r, g, b| {
            let max = r.max(g).max(b);
            let min = r.min(g).min(b);
            // Preserve v0.6.3's muted/disabled button and bright-border evidence.
            // Stricter replacement thresholds lost valid partially occluded cards
            // after downscaling, even though their title was readable by OCR.
            let gray = (65..=190).contains(&max) && max - min <= 60;
            let gold = r >= 100 && (70..=190).contains(&g) && b <= 135;
            let cyan = g >= 105 && b >= 105 && r <= 120;
            let border = max >= 145 && min >= 105 && max - min <= 85;
            button += usize::from(gray || gold || cyan || border);
        }) as f64;
        button_slots += usize::from(button as f64 / n >= 0.08);
    }
    if button_slots >= 2 && title_slots >= 1 {
        Evidence::Visible
    } else if button_slots >= 1 && title_slots >= 1 {
        Evidence::Partial
    } else if button_slots == 0 && title_slots == 0 {
        Evidence::Absent
    } else {
        Evidence::Uncertain
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn requires_titles_and_multiple_buttons_and_handles_dark_or_busy_scenes() {
        let mut t = vec![0u8; 120 * 48 * 12];
        let mut b = vec![0u8; t.len()];
        let evidence = |t: &[u8], b: &[u8]| {
            classify(
                &Atlas::new(t, 120, 48).unwrap(),
                &Atlas::new(b, 120, 48).unwrap(),
            )
        };
        assert_eq!(evidence(&t, &b), Evidence::Absent);
        for i in 0..300 {
            t[i * 4..i * 4 + 3].fill(220);
        }
        assert_eq!(evidence(&t, &b), Evidence::Uncertain);
        b.fill(125);
        assert_eq!(evidence(&t, &b), Evidence::Visible);
        t.fill(220);
        assert_eq!(evidence(&t, &b), Evidence::Uncertain);
        for (w, h) in [(1280, 720), (1920, 1080), (3840, 2160)] {
            for r in buttons(w, h).unwrap() {
                assert!(r.x + r.width <= w && r.y + r.height <= h);
            }
        }
    }
}
