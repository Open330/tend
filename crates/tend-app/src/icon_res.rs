//! Turns a `.ico` file into a compiled Windows resource (`.res`) holding it
//! as the program's icon: one `RT_ICON` per image and one `RT_GROUP_ICON`
//! that lists them. MSVC's `link.exe` (and `lld-link`) accept a `.res` on the
//! command line, so no resource compiler is needed. Shared by `build.rs` and
//! the tests in `main.rs`.

const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
/// `LANG_ENGLISH`, `SUBLANG_ENGLISH_US`, what `rc.exe` writes by default.
const LANGUAGE: u16 = 0x0409;
/// `MOVEABLE | DISCARDABLE` for images, plus `PURE` for the group, as `rc.exe`.
const ICON_FLAGS: u16 = 0x1010;
const GROUP_FLAGS: u16 = 0x1030;

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

/// One resource entry: a 32-byte header with numeric type and name, the
/// data, padded to four bytes.
fn entry(out: &mut Vec<u8>, kind: u16, name: u16, flags: u16, data: &[u8]) {
    let size = u32::try_from(data.len()).expect("resource under 4 GiB");
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&32u32.to_le_bytes());
    out.extend_from_slice(&[0xFF, 0xFF]);
    out.extend_from_slice(&kind.to_le_bytes());
    out.extend_from_slice(&[0xFF, 0xFF]);
    out.extend_from_slice(&name.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // DataVersion
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&LANGUAGE.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // Version
    out.extend_from_slice(&0u32.to_le_bytes()); // Characteristics
    out.extend_from_slice(data);
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
}

/// The `.res` bytes for `ico`, or `None` if it is not a well-formed icon file.
pub fn res_from_ico(ico: &[u8]) -> Option<Vec<u8>> {
    if u16_at(ico, 0)? != 0 || u16_at(ico, 2)? != 1 {
        return None;
    }
    let count = usize::from(u16_at(ico, 4)?);
    if count == 0 {
        return None;
    }
    let mut out = Vec::new();
    // A .res file starts with an empty entry (type 0, name 0).
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&32u32.to_le_bytes());
    out.extend_from_slice(&[0xFF, 0xFF, 0, 0, 0xFF, 0xFF, 0, 0]);
    out.extend_from_slice(&[0; 16]);
    let mut group = Vec::new();
    group.extend_from_slice(&0u16.to_le_bytes());
    group.extend_from_slice(&1u16.to_le_bytes());
    group.extend_from_slice(&u16::try_from(count).ok()?.to_le_bytes());
    for i in 0..count {
        let at = 6 + 16 * i;
        let dir = ico.get(at..at + 16)?;
        let len = u32_at(ico, at + 8)?;
        let offset = usize::try_from(u32_at(ico, at + 12)?).ok()?;
        let image = ico.get(offset..offset + usize::try_from(len).ok()?)?;
        let id = u16::try_from(i + 1).ok()?;
        entry(&mut out, RT_ICON, id, ICON_FLAGS, image);
        // GRPICONDIRENTRY: the ICONDIRENTRY's first 12 bytes, then the id.
        group.extend_from_slice(&dir[..12]);
        group.extend_from_slice(&id.to_le_bytes());
    }
    entry(&mut out, RT_GROUP_ICON, 1, GROUP_FLAGS, &group);
    Some(out)
}

/// The largest PNG-encoded image in a `.ico`: what a toast's `IconUri` can
/// point at (Windows reads PNG there, not `.ico`). `None` when the file has
/// no PNG image or is malformed.
#[allow(dead_code)] // build.rs includes this file too and uses only res_from_ico
pub fn largest_png(ico: &[u8]) -> Option<&[u8]> {
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";
    if u16_at(ico, 0)? != 0 || u16_at(ico, 2)? != 1 {
        return None;
    }
    let count = usize::from(u16_at(ico, 4)?);
    let mut best: Option<(u32, &[u8])> = None;
    for index in 0..count {
        let at = 6 + 16 * index;
        let width = match *ico.get(at)? {
            0 => 256,
            w => u32::from(w),
        };
        let size = usize::try_from(u32_at(ico, at + 8)?).ok()?;
        let offset = usize::try_from(u32_at(ico, at + 12)?).ok()?;
        let image = ico.get(offset..offset.checked_add(size)?)?;
        if image.starts_with(PNG) && best.is_none_or(|(w, _)| width > w) {
            best = Some((width, image));
        }
    }
    best.map(|(_, image)| image)
}
