use fuze_contract::{selection_pixels::Evidence, titles::Region};

#[derive(Clone, Copy, Default)]
pub enum PickProbe {
    #[default]
    Off,
    Offer,
    Click,
}

impl PickProbe {
    pub(crate) fn regions(
        self,
        evidence: Option<Evidence>,
        (width, height): (u32, u32),
    ) -> Option<[Region; 2]> {
        if matches!(self, Self::Off)
            || (matches!(self, Self::Offer) && evidence != Some(Evidence::Visible))
        {
            return None;
        }
        let center = width / 2;
        let hud_left = center.saturating_sub(height * 55 / 100);
        let hud_right = center.saturating_sub(height * 12 / 100);
        let hud_top = height * 884 / 1000;
        let cards_left = (width * 212 / 1000).saturating_sub(height * 61 / 1000);
        let cards_right = width - cards_left;
        if hud_right <= hud_left {
            return None;
        }
        Some([
            Region {
                x: hud_left,
                y: hud_top,
                width: hud_right - hud_left,
                height: height - hud_top,
            },
            Region {
                x: cards_left,
                y: height * 227 / 1000,
                width: cards_right - cards_left,
                height: height * 118 / 1000,
            },
        ])
    }
}

/// Two short strips of a sampled frame. Their coordinates are in physical screen pixels.
#[derive(Clone, Copy)]
pub struct PixelStrip<'a> {
    bytes: &'a [u8],
    stride: usize,
    bounds: Region,
}

impl<'a> PixelStrip<'a> {
    pub fn new(bytes: &'a [u8], stride: usize, bounds: Region) -> Option<Self> {
        if bounds.width == 0
            || bounds.height == 0
            || stride < bounds.width as usize * 4
            || bytes.len()
                < (bounds.height as usize - 1)
                    .checked_mul(stride)?
                    .checked_add(bounds.width as usize * 4)?
        {
            return None;
        }
        Some(Self {
            bytes,
            stride,
            bounds,
        })
    }

    #[inline]
    pub fn luminance(&self, x: i32, y: i32) -> f32 {
        let x = (x as u32).wrapping_sub(self.bounds.x);
        let y = (y as u32).wrapping_sub(self.bounds.y);
        if x >= self.bounds.width || y >= self.bounds.height {
            return 0.;
        }
        let i = y as usize * self.stride + x as usize * 4;
        let b = u16::from(self.bytes[i]);
        let g = u16::from(self.bytes[i + 1]);
        let r = u16::from(self.bytes[i + 2]);
        f32::from(b + 5 * g + 2 * r) / 8.
    }
}

#[derive(Clone, Copy)]
pub struct PickPixels<'a> {
    pub screen: (u32, u32),
    pub hud: PixelStrip<'a>,
    pub cards: PixelStrip<'a>,
}

impl<'a> PickPixels<'a> {
    pub(crate) fn packed(
        bytes: &'a [u8],
        [hud, cards]: [Region; 2],
        screen: (u32, u32),
    ) -> Option<Self> {
        let stride = (hud.width + cards.width) as usize * 4;
        Some(Self {
            screen,
            hud: PixelStrip::new(bytes, stride, hud)?,
            cards: PixelStrip::new(bytes.get(hud.width as usize * 4..)?, stride, cards)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supplementation_fits_the_roi_budget_and_stays_off_between_offers() {
        for screen in [(852, 480), (1920, 1080), (2560, 1440), (4096, 2160)] {
            assert!(PickProbe::Off
                .regions(Some(Evidence::Visible), screen)
                .is_none());
            assert!(PickProbe::Offer
                .regions(Some(Evidence::Absent), screen)
                .is_none());
            assert!(PickProbe::Click
                .regions(Some(Evidence::Absent), screen)
                .is_some());
            let areas = PickProbe::Offer
                .regions(Some(Evidence::Visible), screen)
                .unwrap();
            assert!(areas.iter().map(|r| r.width).sum::<u32>() <= 4096);
            assert!(areas.iter().all(|r| {
                r.height <= 256 && r.x + r.width <= screen.0 && r.y + r.height <= screen.1
            }));
        }
    }
}
