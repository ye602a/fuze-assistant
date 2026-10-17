use super::*;
use fuze_contract::titles::{regions, Atlas};

/// Source created exclusively from supplied offline BGRA bytes. Not a capture handle.
pub struct GpuFixture {
    texture: ID3D11Texture2D,
    width: u32,
    height: u32,
}
pub struct GpuFixtureReader {
    reader: SelectionReader,
    device: Device,
}
impl GpuFixtureReader {
    pub fn new() -> Result<Self> {
        let device = Device::new()?;
        Ok(Self {
            reader: SelectionReader::new(&device),
            device,
        })
    }
    pub fn upload(&self, bytes: &[u8], width: u32, height: u32) -> Result<GpuFixture> {
        fuze_contract::frame::BgraFrame::new(
            bytes,
            width as usize,
            height as usize,
            width as usize * 4,
        )?;
        regions(width, height)?;
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            ..Default::default()
        };
        let initial = D3D11_SUBRESOURCE_DATA {
            pSysMem: bytes.as_ptr().cast(),
            SysMemPitch: width * 4,
            ..Default::default()
        };
        let mut texture = None;
        unsafe {
            self.device
                .device
                .CreateTexture2D(&desc, Some(&initial), Some(&mut texture))?;
        }
        Ok(GpuFixture {
            texture: texture.ok_or("fixture-texture")?,
            width,
            height,
        })
    }
    pub fn selection(&self) -> Option<fuze_contract::selection_pixels::Evidence> {
        self.reader.evidence
    }
    pub fn read<'a>(&'a mut self, source: &GpuFixture) -> Result<Atlas<'a>> {
        // Fixtures must belong to this D3D device; never submit a foreign-device texture.
        let source_device = unsafe { source.texture.GetDevice()? };
        if source_device != self.device.device {
            return Err("fixture-device-mismatch".into());
        }
        let rects = regions(source.width, source.height)?;
        self.reader
            .read(&source.texture, &rects, (source.width, source.height))?;
        Ok(Atlas::new(
            &self.reader.bytes,
            rects[0].width,
            rects[0].height,
        )?)
    }
}
