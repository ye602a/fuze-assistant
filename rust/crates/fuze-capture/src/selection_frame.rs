use crate::native::{DiagnosticPixels, Result};
use fuze_contract::{
    selection_pixels::{self, Evidence},
    titles::{self, Atlas, Region},
    CardSlots, Layout,
};

pub(crate) struct CaptureRegions {
    pub areas: [Region; 8],
    three: CardSlots<Region>,
    four: CardSlots<Region>,
    screen: (u32, u32),
}

impl CaptureRegions {
    pub fn new(screen: (u32, u32)) -> Result<Self> {
        let (w, h) = screen;
        let three = titles::regions_for(w, h, Layout::Three)?;
        let four = titles::regions_for(w, h, Layout::Four)?;
        let old = selection_pixels::buttons(w, h)?;
        let new = selection_pixels::buttons_for(w, h, Layout::Four)?;
        // One short band serves either set of title crops. Relatively narrow
        // four-card control probes keep the combined GPU staging width bounded.
        let band = Region {
            width: four[3].x + four[3].width - four[0].x,
            ..four[0]
        };
        Ok(Self {
            areas: [band, old[0], old[1], old[2], new[0], new[1], new[2], new[3]],
            three,
            four,
            screen,
        })
    }

    pub fn bitmap_bounds(&self) -> Region {
        let band = self.areas[0];
        let last = self.areas[4];
        Region {
            height: last.y + last.height - band.y,
            ..band
        }
    }
}

#[derive(Default)]
pub(crate) struct SelectionFrame {
    title_bytes: Vec<u8>,
    button_bytes: Vec<u8>,
    screen: (u32, u32),
    title_size: (u32, u32),
    button_size: (u32, u32),
    layout: Layout,
    pub evidence: Option<Evidence>,
}

impl SelectionFrame {
    pub fn titles(&self) -> Result<Atlas<'_>> {
        Ok(Atlas::with_layout(
            &self.title_bytes,
            self.title_size.0,
            self.title_size.1,
            self.layout,
        )?)
    }

    fn buttons(&self) -> Result<Atlas<'_>> {
        Ok(Atlas::with_layout(
            &self.button_bytes,
            self.button_size.0,
            self.button_size.1,
            self.layout,
        )?)
    }

    pub fn diagnostic_pixels(&self) -> Option<DiagnosticPixels<'_>> {
        Some(DiagnosticPixels {
            buttons: self.buttons().ok()?,
            screen: self.screen,
        })
    }

    pub fn read<'a>(
        &mut self,
        regions: &CaptureRegions,
        active_layout: Layout,
        row: impl Fn(usize, usize) -> &'a [u8],
    ) -> Result<()> {
        self.read_buttons(regions, 4..8, &row);
        let aligned = selection_pixels::aligned_four_buttons(&Atlas::with_layout(
            &self.button_bytes,
            self.button_size.0,
            self.button_size.1,
            Layout::Four,
        )?);
        self.screen = regions.screen;
        let band = regions.areas[0];
        // Fall back to the actual card geometry when a reroll is spent or
        // animating. The sides are already in the title band readback.
        let card_borders = aligned < 4
            && selection_pixels::has_four_card_borders(
                regions.four[0].width as usize,
                regions.four[0].height as usize,
                |slot, x, y| {
                    let offset = ((regions.four[slot].x - band.x) as usize + x) * 4;
                    let pixels = row(0, y);
                    [pixels[offset], pixels[offset + 1], pixels[offset + 2]]
                },
            );
        let use_four = aligned == 4 || card_borders || active_layout == Layout::Four;
        let slots = if use_four {
            &regions.four
        } else {
            &regions.three
        };
        self.layout = slots.layout();
        self.title_size = (slots[0].width, slots[0].height);
        let width = slots[0].width as usize * 4;
        pack_rows(
            &mut self.title_bytes,
            slots.len(),
            slots[0].height,
            width,
            |i, y| {
                let x = (slots[i].x - regions.areas[0].x) as usize * 4;
                &row(0, y)[x..x + width]
            },
        );

        let evidence = if use_four {
            selection_pixels::classify_four(&self.titles()?, aligned, card_borders)
        } else {
            self.read_buttons(regions, 1..4, &row);
            selection_pixels::classify(&self.titles()?, &self.buttons()?)
        };
        self.evidence = Some(evidence);
        Ok(())
    }

    fn read_buttons<'a>(
        &mut self,
        regions: &CaptureRegions,
        areas: std::ops::Range<usize>,
        row: impl Fn(usize, usize) -> &'a [u8],
    ) {
        let first = regions.areas[areas.start];
        self.button_size = (first.width, first.height);
        pack_rows(
            &mut self.button_bytes,
            areas.len(),
            first.height,
            first.width as usize * 4,
            |slot, y| row(areas.start + slot, y),
        );
    }
}

fn pack_rows<'a>(
    target: &mut Vec<u8>,
    count: usize,
    height: u32,
    width: usize,
    row: impl Fn(usize, usize) -> &'a [u8],
) {
    let stride = count * width;
    target.resize(stride * height as usize, 0);
    for (y, out) in target.chunks_exact_mut(stride).enumerate() {
        for (slot, crop) in out.chunks_exact_mut(width).enumerate() {
            crop.copy_from_slice(row(slot, y));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(
        frame: &mut SelectionFrame,
        plan: &CaptureRegions,
        tiles: &[Vec<u8>; 8],
        active_layout: Layout,
    ) {
        frame
            .read(plan, active_layout, |i, y| {
                let width = plan.areas[i].width as usize * 4;
                &tiles[i][y * width..(y + 1) * width]
            })
            .unwrap();
    }

    #[test]
    fn both_layouts_fit_the_existing_gpu_readback_budget() {
        for width in 320..=4096 {
            let plan = CaptureRegions::new((width, 2160)).unwrap();
            assert!(
                plan.areas.iter().map(|r| r.width).sum::<u32>() <= 4096,
                "{width}"
            );
            assert!(plan.areas.iter().all(|r| r.height <= 256));
            let bounds = plan.bitmap_bounds();
            assert!(plan.areas.iter().all(|r| {
                r.x >= bounds.x
                    && r.x + r.width <= bounds.x + bounds.width
                    && r.y >= bounds.y
                    && r.y + r.height <= bounds.y + bounds.height
            }));
        }
    }

    #[test]
    fn four_card_layout_holds_during_a_reroll_without_four_visible_buttons() {
        let plan = CaptureRegions::new((1920, 1080)).unwrap();
        let mut tiles = plan
            .areas
            .map(|r| vec![0; (r.width * r.height * 4) as usize]);
        let band = plan.areas[0];
        let title = plan.four[0];
        for y in 48..65 {
            for x in 110..200 {
                let offset = (y * band.width + title.x - band.x + x) as usize * 4;
                tiles[0][offset..offset + 3].fill(235);
            }
        }
        for (i, area) in plan.areas.iter().enumerate().skip(4) {
            let row = area.width as usize * 4;
            let color = if i == 7 { 160 } else { 125 }; // enabled and muted edges
            tiles[i][16 * row..17 * row].fill(color);
        }
        let mut frame = SelectionFrame::default();
        feed(&mut frame, &plan, &tiles, Layout::Three);
        assert_eq!(frame.titles().unwrap().layout, Layout::Four);
        assert_eq!(frame.evidence, Some(Evidence::Visible));
        tiles[5].fill(0);
        feed(&mut frame, &plan, &tiles, Layout::Four);
        assert_eq!(frame.titles().unwrap().layout, Layout::Four);
        assert_eq!(frame.evidence, Some(Evidence::Visible));
        tiles[6].fill(0);
        tiles[7].fill(0);
        feed(&mut frame, &plan, &tiles, Layout::Four);
        assert_eq!(frame.titles().unwrap().layout, Layout::Four);
        assert_eq!(frame.evidence, Some(Evidence::Partial));
        tiles[4].fill(0);
        feed(&mut frame, &plan, &tiles, Layout::Four);
        assert_eq!(frame.evidence, Some(Evidence::Uncertain));
        feed(&mut frame, &plan, &tiles, Layout::Three);
        assert_eq!(frame.titles().unwrap().layout, Layout::Three);
    }

    #[test]
    fn four_cards_survive_any_reroll_and_reopen_without_button_borders() {
        for size in [(1280, 720), (1920, 1080), (3840, 2160)] {
            let plan = CaptureRegions::new(size).unwrap();
            let blank = plan
                .areas
                .map(|r| vec![16; (r.width * r.height * 4) as usize]);
            let band = plan.areas[0];
            let with_frames = |omitted: Option<usize>, titles: bool| {
                let mut tiles = blank.clone();
                for (slot, r) in plan.four.iter().enumerate() {
                    if omitted == Some(slot) {
                        continue;
                    }
                    let left = r.x - band.x;
                    for y in 0..r.height {
                        for x in [left + 2, left + r.width - 3] {
                            let offset = ((y * band.width + x) * 4) as usize;
                            tiles[0][offset..offset + 3].fill(125);
                        }
                    }
                    if titles {
                        for y in r.height / 2..r.height * 7 / 12 {
                            for x in left + r.width / 3..left + r.width * 2 / 3 {
                                let offset = ((y * band.width + x) * 4) as usize;
                                tiles[0][offset..offset + 3].fill(235);
                            }
                        }
                    }
                }
                tiles
            };
            let mut frame = SelectionFrame::default();
            let full = with_frames(None, true);
            feed(&mut frame, &plan, &full, Layout::Three);
            assert_eq!(frame.titles().unwrap().layout, Layout::Four);
            assert_eq!(frame.evidence, Some(Evidence::Visible));
            for slot in 0..4 {
                // The refreshed card and all button borders can vanish while
                // the other three card positions remain on screen.
                feed(
                    &mut frame,
                    &plan,
                    &with_frames(Some(slot), true),
                    Layout::Four,
                );
                assert_eq!(frame.evidence, Some(Evidence::Visible));
            }
            feed(&mut frame, &plan, &with_frames(None, false), Layout::Four);
            assert_eq!(frame.evidence, Some(Evidence::Partial));
            feed(&mut frame, &plan, &blank, Layout::Four);
            assert_eq!(frame.evidence, Some(Evidence::Absent));
            feed(&mut frame, &plan, &full, Layout::Three);
            assert_eq!(frame.titles().unwrap().layout, Layout::Four);
            assert_eq!(frame.evidence, Some(Evidence::Visible));
        }
    }
}
