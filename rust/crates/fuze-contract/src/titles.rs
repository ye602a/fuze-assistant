//! One geometry contract shared by raw-frame, GPU-atlas and OCR paths.
use crate::{frame::BgraFrame, CardSlots, Layout};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}
pub fn regions(width: u32, height: u32) -> Result<[Region; 3], &'static str> {
    if !(320..=4096).contains(&width) || !(180..=2160).contains(&height) {
        return Err("invalid-title-frame");
    }
    let regions = std::array::from_fn(|i| Region {
        // v0.6.3 Paddle title envelope. The recognizer locates the title line inside it;
        // clipping to a fixed 3%-high strip loses glyphs during card motion/scaling.
        x: ((0.2305 + i as f64 * 0.1855 + 0.00336) * width as f64).round() as u32,
        y: (0.35 * height as f64).round() as u32,
        width: (0.16128 * width as f64).round() as u32,
        height: (0.095 * height as f64).round().max(1.) as u32,
    });
    if regions
        .iter()
        .any(|r| r.x + r.width > width || r.y + r.height > height)
    {
        return Err("title-outside-frame");
    }
    Ok(regions)
}

/// The Maestro panel uses a different horizontal spacing and center for every card.
pub(crate) fn four_center(i: usize) -> f64 {
    (408. + i as f64 * 368.) / 1920.
}

pub fn regions_for(
    width: u32,
    height: u32,
    layout: Layout,
) -> Result<CardSlots<Region>, &'static str> {
    let ordinary = regions(width, height)?;
    Ok(match layout {
        Layout::Three => ordinary.into(),
        Layout::Four => CardSlots::from_fn(layout, |i| Region {
            x: ((four_center(i) - 0.16128 / 2.) * width as f64).round() as u32,
            ..ordinary[0]
        }),
    })
}

pub fn pick_regions(
    width: u32,
    height: u32,
    layout: Layout,
) -> Result<CardSlots<Region>, &'static str> {
    // Interior of the four-card body (192–720 at 1080p). Reroll starts at y=743.
    let (top, bottom) = match layout {
        Layout::Three => (0.19, 0.61),
        Layout::Four => (198. / 1080., 714. / 1080.),
    };
    let top = (height as f64 * top) as u32;
    let bottom = (height as f64 * bottom) as u32;
    Ok(regions_for(width, height, layout)?.map(|r| Region {
        y: top,
        height: bottom - top,
        ..r
    }))
}

/// Equal-sized title crops packed horizontally in the game's left-to-right order.
pub struct Atlas<'a> {
    frame: BgraFrame<'a>,
    pub layout: Layout,
    pub title_width: u32,
    pub title_height: u32,
}
impl<'a> Atlas<'a> {
    pub fn new(bytes: &'a [u8], title_width: u32, title_height: u32) -> Result<Self, &'static str> {
        Self::with_layout(bytes, title_width, title_height, Layout::Three)
    }
    pub fn with_layout(
        bytes: &'a [u8],
        title_width: u32,
        title_height: u32,
        layout: Layout,
    ) -> Result<Self, &'static str> {
        if title_width == 0 || title_width > 1365 || title_height == 0 || title_height > 256 {
            return Err("invalid-title-atlas");
        }
        let width = title_width as usize * layout.count();
        Ok(Self {
            frame: BgraFrame::new(bytes, width, title_height as usize, width * 4)?,
            layout,
            title_width,
            title_height,
        })
    }
    pub fn rgb_slot(&self, slot: usize) -> Result<Vec<u8>, &'static str> {
        if slot >= self.layout.count() {
            return Err("invalid-slot");
        }
        self.frame.rgb_roi(
            slot * self.title_width as usize,
            0,
            self.title_width as usize,
            self.title_height as usize,
        )
    }
    pub fn has_lit_pixel(&self) -> bool {
        self.bytes()
            .chunks_exact(4)
            .any(|p| u16::from(p[0]) + u16::from(p[1]) + u16::from(p[2]) > 24)
    }
    pub fn bytes(&self) -> &[u8] {
        self.frame.bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn geometry_and_atlas_preserve_positions_and_bounds() {
        for (w, h) in [(1280, 720), (1920, 1080), (2560, 1440), (3840, 2160)] {
            let r = regions(w, h).unwrap();
            assert!(r.windows(2).all(|p| p[0].x + p[0].width < p[1].x));
        }
        let data = [1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255];
        let atlas = Atlas::new(&data, 1, 1).unwrap();
        assert_eq!(atlas.rgb_slot(1).unwrap(), [6, 5, 4]);
        assert!(atlas.rgb_slot(3).is_err());
        assert!(Atlas::new(&data[..8], 1, 1).is_err());
        assert!(regions(u32::MAX, 1080).is_err());
    }
}
