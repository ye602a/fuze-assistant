// Build a standard Windows .res directly from the existing multi-size ICO. No SDK tools,
// downloads or runtime patching. Layout: Microsoft RESOURCEHEADER / RT_GROUP_ICON.
// https://learn.microsoft.com/en-us/windows/win32/menurc/resourceheader
// https://devblogs.microsoft.com/oldnewthing/20120720-00/?p=7083/
use std::{env, fs, path::Path};

fn entry(out: &mut Vec<u8>, kind: u16, id: u16, data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&32_u32.to_le_bytes());
    for word in [
        0xffff,
        kind,
        0xffff,
        id,
        0,
        0,
        if kind == 0 { 0 } else { 0x1030 },
        0,
        0,
        0,
        0,
        0,
    ] {
        out.extend_from_slice(&word.to_le_bytes());
    }
    out.extend_from_slice(data);
    while out.len() % 4 != 0 {
        out.push(0);
    }
}
pub fn embed(source: &str) {
    println!("cargo:rerun-if-changed={source}");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    assert_eq!(
        env::var("CARGO_CFG_TARGET_ENV").as_deref(),
        Ok("msvc"),
        "Windows icon linker requires MSVC"
    );
    let ico = fs::read(source).expect("read FUZE icon");
    assert!(
        ico.len() >= 6 && ico[..4] == [0, 0, 1, 0],
        "invalid FUZE icon"
    );
    let count = u16::from_le_bytes(ico[4..6].try_into().unwrap());
    assert!(count > 0 && count <= 32);
    let mut resources = Vec::new();
    entry(&mut resources, 0, 0, &[]);
    let mut group = ico[..6].to_vec();
    for i in 0..count {
        let at = 6 + i as usize * 16;
        let e = ico.get(at..at + 16).expect("icon directory");
        let length = u32::from_le_bytes(e[8..12].try_into().unwrap()) as usize;
        let offset = u32::from_le_bytes(e[12..16].try_into().unwrap()) as usize;
        let image = ico
            .get(offset..offset.checked_add(length).unwrap())
            .expect("icon pixels");
        group.extend_from_slice(&e[..12]);
        group.extend_from_slice(&(i + 1).to_le_bytes());
        entry(&mut resources, 3, i + 1, image);
    }
    entry(&mut resources, 14, 1, &group);
    let output = Path::new(&env::var_os("OUT_DIR").unwrap()).join("fuze-icon.res");
    fs::write(&output, resources).expect("write FUZE resource");
    println!("cargo:rustc-link-arg-bins={}", output.display());
}
