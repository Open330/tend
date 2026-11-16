//! The Tend mark for the notification area, drawn as RGBA: one square
//! split by a canted passage into four rooms, each lit for one state
//! (`assets/brand/tend/mark-rooms.svg`, `STATES.md`, the Mac app's
//! `BrandMark.swift`). Corners are left square at tray size.

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

/// The mark at `size` x `size` pixels, straight RGBA, with `ink` for the
/// working room and the dimmed rooms (light ink for a dark taskbar).
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
                    if let Some((_, (color, alpha))) =
                        layers.iter().find(|(poly, _)| inside(poly, x, y))
                    {
                        for c in 0..3 {
                            acc[c] += f32::from(color[c]) * alpha;
                        }
                        acc[3] += alpha;
                    }
                }
            }
            let n = (SAMPLES * SAMPLES) as f32;
            let i = ((py * size + px) * 4) as usize;
            let alpha = acc[3] / n;
            if alpha > 0.0 {
                for c in 0..3 {
                    out[i + c] = (acc[c] / acc[3]).round().clamp(0.0, 255.0) as u8;
                }
                out[i + 3] = (alpha * 255.0).round().clamp(0.0, 255.0) as u8;
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
    fn lit_rooms_take_their_colours_and_the_passage_stays_clear() {
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
        // Top-right is the waiting room: amber when lit, dim ink when not.
        let waiting = pixel(&busy, size, 28, 3);
        assert_eq!(&waiting[..3], &WAITING);
        assert_eq!(waiting[3], 255);
        let dim = pixel(&quiet, size, 28, 3);
        assert_eq!(&dim[..3], &ink);
        assert!(dim[3] < 80, "dimmed: {dim:?}");
        // Bottom-left is done, top-left error.
        assert_eq!(&pixel(&busy, size, 2, 30)[..3], &DONE);
        assert_eq!(&pixel(&busy, size, 2, 2)[..3], &ERROR);
        // The horizontal passage between the top rooms and the working room.
        assert_eq!(pixel(&busy, size, 28, 9)[3], 0, "passage is transparent");
    }
}
