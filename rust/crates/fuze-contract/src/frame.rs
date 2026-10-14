/// Borrowed top-down BGRA8. Padding is allowed; no encoded image or full-frame conversion.
pub struct BgraFrame<'a> {
    bytes: &'a [u8],
    pub width: usize,
    pub height: usize,
    stride: usize,
}
impl<'a> BgraFrame<'a> {
    pub fn bytes(&self) -> &[u8] {
        self.bytes
    }
    pub fn new(
        bytes: &'a [u8],
        width: usize,
        height: usize,
        stride: usize,
    ) -> Result<Self, &'static str> {
        if width == 0
            || height == 0
            || width > 4096
            || height > 2160
            || stride < width * 4
            || stride > 65536
        {
            return Err("invalid-frame-layout");
        }
        let size = stride.checked_mul(height).ok_or("frame-overflow")?;
        if size > 64 * 1024 * 1024 || bytes.len() != size {
            return Err("invalid-frame-length");
        }
        Ok(Self {
            bytes,
            width,
            height,
            stride,
        })
    }
    pub fn rgb_roi(
        &self,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
    ) -> Result<Vec<u8>, &'static str> {
        if width == 0
            || height == 0
            || x.checked_add(width).is_none_or(|r| r > self.width)
            || y.checked_add(height).is_none_or(|b| b > self.height)
        {
            return Err("invalid-roi");
        }
        let mut rgb = Vec::with_capacity(width * height * 3);
        for row in y..y + height {
            let start = row * self.stride + x * 4;
            for pixel in self.bytes[start..start + width * 4].chunks_exact(4) {
                rgb.extend_from_slice(&[pixel[2], pixel[1], pixel[0]]);
            }
        }
        Ok(rgb)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn honors_padded_rows_and_rejects_truncated_or_overflowing_rois() {
        let bytes = [
            1, 2, 3, 255, 4, 5, 6, 255, 99, 99, 99, 99, 7, 8, 9, 255, 10, 11, 12, 255, 99, 99, 99,
            99,
        ];
        let frame = BgraFrame::new(&bytes, 2, 2, 12).unwrap();
        assert_eq!(frame.rgb_roi(1, 0, 1, 2).unwrap(), [6, 5, 4, 12, 11, 10]);
        assert!(frame.rgb_roi(usize::MAX, 0, 2, 1).is_err());
        assert!(BgraFrame::new(&bytes[..20], 2, 2, 12).is_err());
    }
}
