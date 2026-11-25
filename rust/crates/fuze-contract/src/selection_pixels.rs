//! v0.6.3 title + reroll-button screen gate. Bounded pixel sampling, no OCR/encoding.
use crate::{
    titles::{four_center, regions, Atlas, Region},
    CardSlots, Layout,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Evidence {
    Visible,
    /// Three title-shaped regions without reroll evidence: only skill OCR may admit this.
    TitlesOnly,
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

pub fn buttons_for(
    width: u32,
    height: u32,
    layout: Layout,
) -> Result<CardSlots<Region>, &'static str> {
    let ordinary = buttons(width, height)?;
    Ok(match layout {
        Layout::Three => ordinary.into(),
        Layout::Four => CardSlots::from_fn(layout, |i| Region {
            // The center of each reroll is four pixels right of its card at 1080p.
            // Sample the straight middle of the border, away from rounded corners.
            x: ((four_center(i) + (4. - 21.) / 1920.) * width as f64).round() as u32,
            y: (728. / 1080. * height as f64).round() as u32,
            width: (42. / 1920. * width as f64).round() as u32,
            height: (60. / 1080. * height as f64).round() as u32,
        }),
    })
}

fn sample(atlas: &Atlas<'_>, slot: usize, mut take: impl FnMut(u8, u8, u8)) -> usize {
    let w = atlas.title_width as usize;
    let h = atlas.title_height as usize;
    let mut count = 0;
    for y in (0..h).step_by((h / 48).max(1)) {
        for x in (0..w).step_by((w / 120).max(1)) {
            let i = (y * w * atlas.layout.count() + slot * w + x) * 4;
            let p = &atlas.bytes()[i..i + 3];
            take(p[2], p[1], p[0]);
            count += 1;
        }
    }
    count
}

fn title_visible(atlas: &Atlas<'_>, slot: usize) -> bool {
    let (mut bright, mut dark) = (0, 0);
    let n = sample(atlas, slot, |r, g, b| {
        bright += usize::from(r > 180 && g > 180 && b > 170);
        dark += usize::from(r.max(g).max(b) < 55);
    }) as f64;
    bright as f64 / n >= 0.004 && dark as f64 / n >= 0.72
}

/// Count aligned horizontal button borders, independent of gold/gray button color.
/// The older three-slot color probe can see the four-card frames as button pixels.
pub fn aligned_four_buttons(atlas: &Atlas<'_>) -> usize {
    debug_assert_eq!(atlas.layout, Layout::Four);
    let w = atlas.title_width as usize;
    let h = atlas.title_height as usize;
    let row_step = (h / 28).max(1);
    let brightness = |slot, x, y| {
        let offset: usize = (y * w * 4 + slot * w + x) * 4;
        atlas.bytes()[offset..offset + 3]
            .iter()
            .map(|&v| i16::from(v))
            .sum::<i16>()
    };
    let tops: [Option<usize>; 4] = std::array::from_fn(|slot| {
        (row_step..h / 2)
            .map(|y| {
                let strong = (0..7)
                    .filter(|&i| {
                        let x = (w - 1) * i / 6;
                        brightness(slot, x, y) - brightness(slot, x, y - row_step) >= 90
                    })
                    .count();
                (y, strong)
            })
            .filter(|&(_, strong)| strong >= 5)
            .max_by_key(|&(y, strong)| (strong, std::cmp::Reverse(y)))
            .map(|(y, _)| y)
    });
    tops.iter()
        .flatten()
        .map(|&y| {
            tops.iter()
                .flatten()
                .filter(|&&r| y.abs_diff(r) <= row_step)
                .count()
        })
        .max()
        .unwrap_or(0)
}

pub fn classify_four(titles: &Atlas<'_>, aligned_buttons: usize) -> Evidence {
    let has_title = (0..4).any(|slot| title_visible(titles, slot));
    match (has_title, aligned_buttons) {
        (true, 2..) => Evidence::Visible,
        (true, 1) => Evidence::Partial,
        (false, 0) => Evidence::Absent,
        _ => Evidence::Uncertain,
    }
}

pub fn classify(titles: &Atlas<'_>, buttons: &Atlas<'_>) -> Evidence {
    let mut title_slots = 0;
    let mut button_slots = 0;
    for slot in 0..3 {
        title_slots += usize::from(title_visible(titles, slot));
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
    } else if title_slots == 3 && button_slots == 0 {
        Evidence::TitlesOnly
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
    fn three_titles_without_buttons_only_admit_the_skill_probe() {
        let (w, h) = (120, 48);
        let mut t = vec![0u8; w * h * 12];
        let mut b = vec![0u8; t.len()];
        for slot in 0..3 {
            for x in 20..60 {
                let i = (24 * w * 3 + slot * w + x) * 4;
                t[i..i + 3].fill(220);
            }
        }
        let evidence = |b: &[u8]| {
            classify(
                &Atlas::new(&t, w as u32, h as u32).unwrap(),
                &Atlas::new(b, w as u32, h as u32).unwrap(),
            )
        };
        assert_eq!(evidence(&b), Evidence::TitlesOnly);
        // One remaining reroll button retains the existing partial-screen path.
        for y in 0..h {
            for x in 0..w {
                let i = (y * w * 3 + x) * 4;
                b[i..i + 3].fill(125);
            }
        }
        assert_eq!(evidence(&b), Evidence::Partial);
        b.fill(125);
        assert_eq!(evidence(&b), Evidence::Visible);
    }
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
        assert_eq!(evidence(&t, &b), Evidence::TitlesOnly);
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
