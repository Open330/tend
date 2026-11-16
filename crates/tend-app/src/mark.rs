//! The Tend mark for the notification area, drawn as RGBA: one square
//! split by a canted passage into four rooms, each lit for one state
//! (`assets/brand/tend/mark-rooms.svg`, `STATES.md`, the Mac app's
//! `BrandMark.swift`). As the app icon (variant D of
//! `docs/proposals/ICON.md`) the rooms fill a rounded graphite plate, which
//! shows in the corridors between them.

/// Which rooms are lit: one flag per room, as the Mac app's `TendBrandMark`.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Rooms {
    pub waiting: bool,
    pub error: bool,
    pub working: bool,
    pub done: bool,
}

const WAITING: [u8; 3] = [0xFF, 0xAA, 0x2B];
const ERROR: [u8; 3] = [0xFF, 0x5A, 0x3C];
const DONE: [u8; 3] = [0x3F, 0xBF, 0x84];
/// Working lights in the ink; dimmed rooms are the ink at this alpha.
const DIM_ALPHA: f32 = 0.26;

// The rooms' polygons in the SVG's 4..60 viewBox.
const ROOM_WAITING: &[(f32, f32)] = &[(26.266, 4.0), (60.0, 4.0), (60.0, 17.25), (27.685, 17.25)];
const ROOM_ERROR: &[(f32, f32)] = &[(4.0, 4.0), (20.734, 4.0), (24.082, 35.25), (4.0, 35.25)];
const ROOM_WORKING: &[(f32, f32)] = &[(28.275, 22.75), (60.0, 22.75), (60.0, 60.0), (32.266, 60.0)];
const ROOM_DONE: &[(f32, f32)] = &[(4.0, 40.75), (24.672, 40.75), (26.734, 60.0), (4.0, 60.0)];

fn inside(polygon: &[(f32, f32)], x: f32, y: f32) -> bool {
    let mut odd = false;
    let mut j = polygon.len() - 1;
    for i in 0..polygon.len() {
        let (xi, yi) = polygon[i];
        let (xj, yj) = polygon[j];
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            odd = !odd;
        }
        j = i;
    }
    odd
}

/// The plate's corner radius in design units (22.5% of the 56-unit side), the
/// shape of `crates/tend-app/assets/tend.ico` (`scripts/brand/tend_icons.py`).
const CORNER: f32 = 56.0 * 0.225;
/// The graphite plate, top and bottom: it shows in the corridors between the
/// rooms (variant D of `docs/proposals/ICON.md`).
const PLATE_TOP: [f32; 3] = [60.0, 65.0, 75.0];
const PLATE_BOTTOM: [f32; 3] = [27.0, 31.0, 38.0];

fn on_plate(x: f32, y: f32) -> bool {
    let (lo, hi) = (4.0 + CORNER, 60.0 - CORNER);
    let dx = (lo - x).max(x - hi).max(0.0);
    let dy = (lo - y).max(y - hi).max(0.0);
    (4.0..=60.0).contains(&x) && (4.0..=60.0).contains(&y) && dx * dx + dy * dy <= CORNER * CORNER
}

/// The icon at `size` x `size` pixels, straight RGBA: the rooms fill a
/// rounded graphite plate that shows in the corridors between them. Lit
/// rooms take their state colour; the working room lights in `ink`; dimmed
/// rooms are `ink` at a low alpha over the plate. The plate is the same on
/// light and dark taskbars, so the icon does not depend on the theme.
#[must_use]
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
pub fn rgba(size: u32, rooms: Rooms, ink: [u8; 3]) -> Vec<u8> {
    const SAMPLES: u32 = 4;
    let lit = |on: bool, color: [u8; 3]| if on { (color, 1.0) } else { (ink, DIM_ALPHA) };
    let layers = [
        (ROOM_WAITING, lit(rooms.waiting, WAITING)),
        (ROOM_ERROR, lit(rooms.error, ERROR)),
        (ROOM_WORKING, lit(rooms.working, ink)),
        (ROOM_DONE, lit(rooms.done, DONE)),
    ];
    let mut out = vec![0u8; (size * size * 4) as usize];
    let scale = 56.0 / size as f32;
    for py in 0..size {
        for px in 0..size {
            let mut acc = [0.0f32; 4];
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    let x = 4.0 + (px as f32 + (sx as f32 + 0.5) / SAMPLES as f32) * scale;
                    let y = 4.0 + (py as f32 + (sy as f32 + 0.5) / SAMPLES as f32) * scale;
                    if !on_plate(x, y) {
                        continue;
                    }
                    let t = (y - 4.0) / 56.0;
                    let mut color = [0.0f32; 3];
                    for c in 0..3 {
                        color[c] = PLATE_TOP[c] + (PLATE_BOTTOM[c] - PLATE_TOP[c]) * t;
                    }
                    if let Some((_, (room, alpha))) =
                        layers.iter().find(|(poly, _)| inside(poly, x, y))
                    {
                        for c in 0..3 {
                            color[c] += (f32::from(room[c]) - color[c]) * alpha;
                        }
                    }
                    for c in 0..3 {
                        acc[c] += color[c];
                    }
                    acc[3] += 1.0;
                }
            }
            let i = ((py * size + px) * 4) as usize;
            if acc[3] > 0.0 {
                for c in 0..3 {
                    out[i + c] = (acc[c] / acc[3]).round().clamp(0.0, 255.0) as u8;
                }
                let n = (SAMPLES * SAMPLES) as f32;
                out[i + 3] = (acc[3] / n * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(buf: &[u8], size: u32, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * size + x) * 4) as usize;
        [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
    }

    #[test]
    fn lit_rooms_take_their_colours_on_a_graphite_plate() {
        let size = 32;
        let ink = [0xE8, 0xE6, 0xE3];
        let quiet = rgba(size, Rooms::default(), ink);
        let busy = rgba(
            size,
            Rooms {
                waiting: true,
                error: true,
                working: false,
                done: true,
            },
            ink,
        );
        assert_eq!(quiet.len(), 32 * 32 * 4);
        // Top-right is the waiting room: amber when lit, dim ink over the
        // plate when not.
        let waiting = pixel(&busy, size, 24, 3);
        assert_eq!(&waiting[..3], &WAITING);
        assert_eq!(waiting[3], 255);
        let dim = pixel(&quiet, size, 24, 3);
        assert_eq!(dim[3], 255, "the plate is opaque: {dim:?}");
        assert!(dim[0] > 60 && dim[0] < 150, "dimmed over graphite: {dim:?}");
        // Bottom-left is done, top-left error (inside the rounded corner).
        assert_eq!(&pixel(&busy, size, 6, 28)[..3], &DONE);
        assert_eq!(&pixel(&busy, size, 6, 4)[..3], &ERROR);
        // The corridor between the top rooms and the working room shows the
        // graphite plate.
        let corridor = pixel(&busy, size, 24, 9);
        assert_eq!(corridor[3], 255);
        assert!(
            corridor[..3].iter().all(|&c| c < 80),
            "graphite: {corridor:?}"
        );
        // Outside the rounded corner the icon is transparent.
        assert_eq!(pixel(&busy, size, 0, 0)[3], 0);
        assert_eq!(pixel(&busy, size, 31, 31)[3], 0);
    }
}
