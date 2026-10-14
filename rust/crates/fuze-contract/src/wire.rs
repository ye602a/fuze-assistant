//! Private parent/worker pipe. Raw title pixels only; never base64 or encoded screenshots.
use crate::{titles::Atlas, Slot};
use serde::{Deserialize, Serialize};
use std::io::{self, Read, Write};
pub const MAX_PIXELS: usize = 1365 * 256 * 12;
pub const MAX_REPLY: usize = 8192;
fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid-worker-protocol")
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    pub generation: u64,
    pub sequence: u64,
}
pub struct Request {
    pub id: Identity,
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}
impl Request {
    pub fn from_atlas(id: Identity, atlas: &Atlas<'_>) -> Self {
        Self {
            id,
            width: atlas.title_width,
            height: atlas.title_height,
            pixels: atlas.bytes().to_vec(),
        }
    }
    pub fn atlas(&self) -> io::Result<Atlas<'_>> {
        if self.id.generation == 0 || self.id.sequence == 0 {
            return Err(invalid());
        }
        Atlas::new(&self.pixels, self.width, self.height).map_err(|_| invalid())
    }
    pub fn write(&self, out: &mut impl Write) -> io::Result<()> {
        self.atlas()?;
        out.write_all(b"FZA1")?;
        out.write_all(&self.id.generation.to_le_bytes())?;
        out.write_all(&self.id.sequence.to_le_bytes())?;
        out.write_all(&self.width.to_le_bytes())?;
        out.write_all(&self.height.to_le_bytes())?;
        out.write_all(&self.pixels)?;
        out.flush()
    }
    pub fn read(input: &mut impl Read) -> io::Result<Option<Self>> {
        let mut header = [0u8; 28];
        // Clean EOF is shutdown. A partial header is a protocol error.
        loop {
            match input.read(&mut header[..1]) {
                Ok(0) => return Ok(None),
                Ok(_) => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
        input.read_exact(&mut header[1..])?;
        if &header[..4] != b"FZA1" {
            return Err(invalid());
        }
        let id = Identity {
            generation: u64::from_le_bytes(header[4..12].try_into().unwrap()),
            sequence: u64::from_le_bytes(header[12..20].try_into().unwrap()),
        };
        let width = u32::from_le_bytes(header[20..24].try_into().unwrap());
        let height = u32::from_le_bytes(header[24..28].try_into().unwrap());
        if id.generation == 0
            || id.sequence == 0
            || width == 0
            || width > 1365
            || height == 0
            || height > 256
        {
            return Err(invalid());
        }
        let mut pixels = vec![0; width as usize * height as usize * 12];
        input.read_exact(&mut pixels)?;
        Ok(Some(Self {
            id,
            width,
            height,
            pixels,
        }))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActiveBackend {
    Cpu,
    DirectMl,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Reply {
    Ready {
        schema: u16,
        backend: ActiveBackend,
        fallback_count: u8,
    },
    Result {
        id: Identity,
        slots: [Slot; 3],
        ocr_ms: f64,
        backend: ActiveBackend,
        fallback_count: u8,
    },
    Failed {
        id: Identity,
    },
}
pub fn write_reply(out: &mut impl Write, reply: &Reply) -> io::Result<()> {
    let bytes = serde_json::to_vec(reply).map_err(|_| invalid())?;
    if bytes.len() > MAX_REPLY {
        return Err(invalid());
    }
    out.write_all(&(bytes.len() as u32).to_le_bytes())?;
    out.write_all(&bytes)?;
    out.flush()
}
pub fn read_reply(input: &mut impl Read) -> io::Result<Reply> {
    let mut length = [0; 4];
    input.read_exact(&mut length)?;
    let length = u32::from_le_bytes(length) as usize;
    if length == 0 || length > MAX_REPLY {
        return Err(invalid());
    }
    let mut bytes = vec![0; length];
    input.read_exact(&mut bytes)?;
    serde_json::from_slice(&bytes).map_err(|_| invalid())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_binary_transport_preserves_slots_and_rejects_truncation() {
        let bytes = [0u8; 24];
        let atlas = Atlas::new(&bytes, 2, 1).unwrap();
        let r = Request::from_atlas(
            Identity {
                generation: 1,
                sequence: 2,
            },
            &atlas,
        );
        let mut wire = vec![];
        r.write(&mut wire).unwrap();
        assert_eq!(
            Request::read(&mut &wire[..]).unwrap().unwrap().pixels,
            bytes
        );
        assert!(Request::read(&mut &wire[..wire.len() - 1]).is_err());
        wire[20..24].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(Request::read(&mut &wire[..]).is_err());
        assert!(Request::read(&mut &[][..]).unwrap().is_none());
        assert!(read_reply(&mut &(u32::MAX.to_le_bytes())[..]).is_err());
    }
}
