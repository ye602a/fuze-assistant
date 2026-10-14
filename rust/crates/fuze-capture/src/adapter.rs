use serde::Serialize;
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE, DXGI_ERROR_NOT_FOUND,
};

#[derive(Debug, Clone, Serialize)]
pub struct Adapter {
    pub index: u32,
    pub name: String,
    pub software: bool,
    pub dedicated_mib: usize,
}

/// Match ORT DirectML's documented EnumAdapters order, not CIM or Task Manager ordering.
pub fn enumerate() -> windows::core::Result<Vec<Adapter>> {
    unsafe {
        let factory: IDXGIFactory1 = CreateDXGIFactory1()?;
        let mut adapters = Vec::new();
        for index in 0..32 {
            let adapter = match factory.EnumAdapters1(index) {
                Ok(adapter) => adapter,
                Err(error) if error.code() == DXGI_ERROR_NOT_FOUND => break,
                Err(error) => return Err(error),
            };
            let desc = adapter.GetDesc1()?;
            let end = desc
                .Description
                .iter()
                .position(|c| *c == 0)
                .unwrap_or(desc.Description.len());
            adapters.push(Adapter {
                index,
                name: String::from_utf16_lossy(&desc.Description[..end]),
                software: desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0,
                dedicated_mib: desc.DedicatedVideoMemory / (1024 * 1024),
            });
        }
        Ok(adapters)
    }
}
