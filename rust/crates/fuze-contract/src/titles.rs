//! One geometry contract shared by raw-frame, GPU-atlas and OCR paths.
use crate::frame::BgraFrame;

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
        x: ((0.2305 + i as f64 * 0.1855 + 0.01344) * width as f64).round() as u32,
        y: (0.385 * height as f64).round() as u32,
        width: (0.14112 * width as f64).round() as u32,
        height: (0.03 * height as f64).round().max(1.) as u32,
    });
    if regions
        .iter()
        .any(|r| r.x + r.width > width || r.y + r.height > height)
    {
        return Err("title-outside-frame");
    }
    Ok(regions)
}

/// Three equal-sized title crops packed horizontally in original left/center/right order.
pub struct Atlas<'a> {
    frame: BgraFrame<'a>,
    pub title_width: u32,
    pub title_height: u32,
}
impl<'a> Atlas<'a> {
    pub fn new(bytes: &'a [u8], title_width: u32, title_height: u32) -> Result<Self, &'static str> {
        if title_width == 0 || title_width > 1365 || title_height == 0 || title_height > 256 {
            return Err("invalid-title-atlas");
        }
        let width = title_width as usize * 3;
        Ok(Self {
            frame: BgraFrame::new(bytes, width, title_height as usize, width * 4)?,
            title_width,
            title_height,
        })
    }
    pub fn rgb_slot(&self, slot: usize) -> Result<Vec<u8>, &'static str> {
        if slot >= 3 {
            return Err("invalid-slot");
        }
        self.frame.rgb_roi(
            slot * self.title_width as usize,
            0,
            self.title_width as usize,
            self.title_height as usize,
        )
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
