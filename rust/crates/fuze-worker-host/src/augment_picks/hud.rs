//! Match the visible art on a clicked card against a new glyph in the game's own HUD.
//! Geometry is anchored by the game's pending-choice marker, which scales with the HUD.
use fuze_capture::pick_pixels::{PickPixels, PixelStrip};
use fuze_contract::{titles, CardSlots, Layout};

const SIDE: usize = 12;
const CELLS: usize = SIDE * SIDE;
const SLOTS: usize = 6;
// Luminance above background for the generic pending-choice marker, normalized
// to the width of one slot. The marker has the same shape while dimmed.
#[rustfmt::skip]
pub(super) const MARKER: [i8; CELLS] = [
    -5, -1,  4,  3,  1,  2,  7, 12,  9,  3,  0,  2,
     0, 11, 33, 31, 23, 27, 34, 38, 26,  6,  1,  2,
     4, 15, 36, 25,  4,  8, 23, 36, 30,  9,  0,  1,
     1, 14, 33, 23,  1, -3, 17, 28, 29,  8,  0,  1,
     0, 13, 30, 22, 19, 13, 20, 21, 27,  6, -3,  0,
     0, 14, 23, 12, 33, 30, 16,  7, 24,  5, -3,  0,
    -1, 24, 19,  3, 41, 41, 12, -1, 22,  3, -3,  1,
    -2, 25, 21, -1, 37, 41, 10,  2, 26,  2, -3,  1,
    -4, 21, 25,  0, 24, 40,  7, -1, 31,  3,  0,  0,
    -2, 16, 31,  7,  8, 20,  3,  9, 37,  4,  8, -2,
     0, 14, 28, 20, 17, 17, 15, 22, 23,  3,  4, -2,
     2,  5,  7,  9, 12, 11, 10,  6,  3,  0, -1, -1,
];

type Shape = [f32; CELLS];
const CARD_MATCH: f32 = 0.74;
const SAME_ICON: f32 = 0.78;
const MARKER_MATCH: f32 = 0.82;

pub(super) struct Offer {
    inventory: Inventory,
    cards: CardSlots<Option<Shape>>,
}

impl Offer {
    pub fn read(pixels: PickPixels<'_>, layout: Layout, previous: Option<&Self>) -> Option<Self> {
        let inventory = previous
            .and_then(|previous| previous.inventory.grid.read_pending(pixels.hud))
            .or_else(|| Inventory::read(pixels))?;
        let (w, h) = pixels.screen;
        let regions = titles::regions_for(w, h, layout).ok()?;
        let cards = CardSlots::from_fn(layout, |i| {
            let center = match layout {
                Layout::Three => w as f32 * (0.309 + 0.191 * i as f32),
                Layout::Four => regions[i].x as f32 + regions[i].width as f32 / 2.,
            };
            glyph(
                pixels.cards,
                center,
                h as f32 * 0.286,
                h as f32 * 0.116,
                true,
            )
        });
        Some(Self { inventory, cards })
    }

    pub fn can_follow(&self, slot: usize, transforms: bool) -> bool {
        if transforms {
            !self.inventory.icons.is_empty()
        } else {
            self.cards[slot].is_some()
        }
    }

    pub fn confirms(&self, slot: usize, transforms: bool, hud: PixelStrip<'_>) -> bool {
        if transforms {
            return self.inventory.transformed(hud);
        }
        self.cards[slot]
            .as_ref()
            .is_some_and(|card| self.inventory.added(hud, card))
    }
}

#[derive(Clone, Copy, Debug)]
struct Grid {
    x: f32,
    y: f32,
    pitch: f32,
    marker_scale: f32,
}

impl Grid {
    fn center(self, slot: usize) -> (f32, f32) {
        (
            self.x + (slot % 3) as f32 * self.pitch,
            self.y + (slot / 3) as f32 * self.pitch,
        )
    }

    fn icon(self, hud: PixelStrip<'_>, slot: usize) -> Option<Shape> {
        let (x, y) = self.center(slot);
        if self.marker(hud, slot) >= MARKER_MATCH {
            return None;
        }
        let side = self.pitch * 0.70;
        let sample = sample(hud, x, y, side);
        glyph_from_sample(hud, (x, y), side, sample, false)
    }

    fn marker(self, hud: PixelStrip<'_>, slot: usize) -> f32 {
        let (x, y) = self.center(slot);
        marker_score(&sample(hud, x, y, self.pitch * self.marker_scale))
    }

    fn readable(self, hud: PixelStrip<'_>) -> bool {
        // The alternate stats view puts bright numbers between the icon
        // columns, where the augment grid has dark gutters.
        for col in 0..2 {
            for row in 0..2 {
                let (x, y) = self.center(row * 3 + col);
                let bright = [-0.12, -0.06, 0., 0.06, 0.12]
                    .into_iter()
                    .flat_map(|dx| {
                        (-4..=4).map(move |dy| {
                            let x = x + (0.5 + dx) * self.pitch;
                            let y = y + dy as f32 * 0.065 * self.pitch;
                            hud.luminance(x.round() as i32, y.round() as i32)
                        })
                    })
                    .filter(|&value| value >= 90.)
                    .count();
                if bright >= 6 {
                    return false;
                }
            }
        }
        true
    }

    fn read_icons(self, hud: PixelStrip<'_>) -> Option<Vec<Shape>> {
        self.readable(hud)
            .then(|| (0..SLOTS).map_while(|slot| self.icon(hud, slot)).collect())
    }

    fn read_pending(self, hud: PixelStrip<'_>) -> Option<Inventory> {
        let icons = self.read_icons(hud)?;
        (icons.len() < SLOTS && self.marker(hud, icons.len()) >= MARKER_MATCH)
            .then_some(Inventory { grid: self, icons })
    }
}

struct Inventory {
    grid: Grid,
    icons: Vec<Shape>,
}

impl Inventory {
    fn read(pixels: PickPixels<'_>) -> Option<Self> {
        let (w, h) = (pixels.screen.0 as f32, pixels.screen.1 as f32);
        let mut best = (MARKER_MATCH, None);
        // The panel is tied to the centered HUD. Search its pitch rather than
        // using a screen percentage for the first slot.
        for pitch in (h * 0.026).round().max(7.) as u32..=(h * 0.055).round() as u32 {
            let pitch = pitch as f32;
            let grid = Grid {
                x: w / 2. - 9.1 * pitch,
                y: h - 1.66 * pitch,
                pitch,
                marker_scale: 0.70,
            };
            for slot in 0..SLOTS {
                for dy in [-0.08, -0.04, 0., 0.04, 0.08] {
                    for dx in [-0.18, -0.09, 0., 0.09, 0.18] {
                        let coarse = Grid {
                            x: grid.x + dx * pitch,
                            y: grid.y + dy * pitch,
                            ..grid
                        };
                        let score = coarse.marker(pixels.hud, slot);
                        if score > best.0 {
                            best = (score, Some(coarse));
                        }
                        if score < 0.74 || score + 0.08 < best.0 {
                            continue;
                        }
                        // Compressed footage and small HUD scales can shift
                        // the fit by a fraction of a pixel.
                        for scale in [0.66, 0.70, 0.74] {
                            for nudge in [-0.02, 0., 0.02] {
                                let fine = Grid {
                                    x: coarse.x + nudge * pitch,
                                    marker_scale: scale,
                                    ..coarse
                                };
                                let score = fine.marker(pixels.hud, slot);
                                if score > best.0 {
                                    best = (score, Some(fine));
                                }
                            }
                        }
                    }
                }
            }
        }
        best.1?.read_pending(pixels.hud)
    }

    fn added(&self, hud: PixelStrip<'_>, card: &Shape) -> bool {
        let Some(after) = self.grid.read_icons(hud) else {
            return false;
        };
        if after.len() <= self.icons.len() {
            return false;
        }
        // Try the new glyph as the unmatched element. A pre-existing glyph
        // moving to another slot cannot satisfy this check on its own.
        after.iter().enumerate().any(|(i, new)| {
            similarity(card, new) >= CARD_MATCH && keeps_existing(&self.icons, &after, 1 << i)
        })
    }

    fn transformed(&self, hud: PixelStrip<'_>) -> bool {
        // Pandora's Box consumes the choice itself. Only visible replacement
        // of the previous glyphs can invalidate the saved combination inputs.
        let Some(after) = self.grid.read_icons(hud) else {
            return false;
        };
        !self.icons.is_empty()
            && after.len() >= self.icons.len()
            && self
                .icons
                .iter()
                .all(|old| after.iter().all(|new| similarity(old, new) < SAME_ICON))
    }
}

fn keeps_existing(old: &[Shape], new: &[Shape], used: u8) -> bool {
    let Some((first, rest)) = old.split_first() else {
        return true;
    };
    new.iter().enumerate().any(|(i, icon)| {
        used & (1 << i) == 0
            && similarity(first, icon) >= SAME_ICON
            && keeps_existing(rest, new, used | (1 << i))
    })
}

fn sample(pixels: PixelStrip<'_>, cx: f32, cy: f32, side: f32) -> Shape {
    let step = side / SIDE as f32;
    let samples = if step >= 2.4 { 2 } else { 1 };
    std::array::from_fn(|i| {
        let x = cx + ((i % SIDE) as f32 - SIDE as f32 / 2.) * step;
        let y = cy + ((i / SIDE) as f32 - SIDE as f32 / 2.) * step;
        let mut value = 0.;
        for iy in 0..samples {
            for ix in 0..samples {
                value += interpolate(
                    pixels,
                    x + (ix as f32 + 0.5) * step / samples as f32,
                    y + (iy as f32 + 0.5) * step / samples as f32,
                );
            }
        }
        value / (samples * samples) as f32
    })
}

fn interpolate(pixels: PixelStrip<'_>, x: f32, y: f32) -> f32 {
    let (ix, iy) = (x.floor() as i32, y.floor() as i32);
    let (dx, dy) = (x - ix as f32, y - iy as f32);
    let top = pixels.luminance(ix, iy) * (1. - dx) + pixels.luminance(ix + 1, iy) * dx;
    let bottom = pixels.luminance(ix, iy + 1) * (1. - dx) + pixels.luminance(ix + 1, iy + 1) * dx;
    top * (1. - dy) + bottom * dy
}

fn marker_score(sample: &Shape) -> f32 {
    // Fast rejection of empty cells and of most bright art before correlation.
    let background = sample[0].min(sample[SIDE - 1]).min(sample[CELLS - SIDE]);
    let peak = sample.iter().copied().fold(0_f32, f32::max);
    if !(9. ..=100.).contains(&(peak - background)) {
        return 0.;
    }
    let mut sorted = *sample;
    sorted.select_nth_unstable_by(CELLS / 6, f32::total_cmp);
    let background = sorted[CELLS / 6];
    let mut dot = 0.;
    let mut norm = 0.;
    let mut marker_norm = 0.;
    for (value, template) in sample.iter().zip(MARKER) {
        let value = value - background;
        let template = f32::from(template);
        dot += value * template;
        norm += value * value;
        marker_norm += template * template;
    }
    dot / (norm * marker_norm).sqrt().max(1.)
}

fn foreground(mut samples: Shape, cutoff: f32, min_contrast: f32) -> Option<Shape> {
    let mut sorted = samples;
    sorted.select_nth_unstable_by(CELLS / 5, f32::total_cmp);
    let low = sorted[CELLS / 5];
    let high_index = CELLS * 94 / 100;
    sorted[CELLS / 5..].select_nth_unstable_by(high_index - CELLS / 5, f32::total_cmp);
    let contrast = sorted[high_index] - low;
    if contrast < min_contrast {
        return None;
    }
    let floor = low + cutoff * contrast;
    for value in &mut samples {
        *value = ((*value - floor) / ((1. - cutoff) * contrast)).clamp(0., 1.);
    }
    Some(samples)
}

fn glyph(pixels: PixelStrip<'_>, x: f32, y: f32, side: f32, card: bool) -> Option<Shape> {
    glyph_from_sample(pixels, (x, y), side, sample(pixels, x, y, side), card)
}

fn glyph_from_sample(
    pixels: PixelStrip<'_>,
    (cx, cy): (f32, f32),
    side: f32,
    mut sample_values: Shape,
    card: bool,
) -> Option<Shape> {
    if card {
        // The outer decoration/coin indicator does not transfer to the HUD.
        for (i, value) in sample_values.iter_mut().enumerate() {
            let dx = (i % SIDE) as f32 + 0.5 - SIDE as f32 / 2.;
            let dy = (i / SIDE) as f32 + 0.5 - SIDE as f32 / 2.;
            if dx * dx + dy * dy > (SIDE as f32 * 0.40).powi(2) {
                *value = 0.;
            }
        }
    }
    let inner = foreground(sample_values, 0.50, if card { 32. } else { 24. })?;
    let mut rows = [0.; SIDE];
    let mut cols = [0.; SIDE];
    for (i, v) in inner.iter().enumerate() {
        rows[i / SIDE] += v;
        cols[i % SIDE] += v;
    }
    let (mut left, mut top, mut right, mut bottom, mut count) = (SIDE, SIDE, 0, 0, 0);
    for (i, &v) in inner.iter().enumerate() {
        let (x, y) = (i % SIDE, i / SIDE);
        if v > 0.28 && rows[y] >= 1.1 && cols[x] >= 1.1 {
            left = left.min(x);
            top = top.min(y);
            right = right.max(x);
            bottom = bottom.max(y);
            count += 1;
        }
    }
    if count < 8 || right - left < 2 || bottom - top < 2 {
        return None;
    }
    let span = (right - left + 1).max(bottom - top + 1);
    if span >= SIDE {
        return None;
    }
    let step = side / SIDE as f32;
    let x = cx + ((left + right + 1) as f32 / 2. - SIDE as f32 / 2.) * step;
    let y = cy + ((top + bottom + 1) as f32 / 2. - SIDE as f32 / 2.) * step;
    foreground(sample(pixels, x, y, (span + 1) as f32 * step), 0.46, 20.)
}

fn similarity(a: &Shape, b: &Shape) -> f32 {
    let norm = (a.iter().map(|x| x * x).sum::<f32>() * b.iter().map(|x| x * x).sum::<f32>()).sqrt();
    if norm == 0. {
        return 0.;
    }
    let mut best = 0_f32;
    // Align by up to one normalized sample to tolerate scaling/anti-aliasing.
    for dy in -1..=1 {
        for dx in -1..=1 {
            let mut dot = 0.;
            for y in usize::from(dy < 0)..SIDE - usize::from(dy > 0) {
                for x in usize::from(dx < 0)..SIDE - usize::from(dx > 0) {
                    let other = (y as isize + dy) as usize * SIDE + (x as isize + dx) as usize;
                    dot += a[y * SIDE + x] * b[other];
                }
            }
            best = best.max(dot);
        }
    }
    best / norm
}
