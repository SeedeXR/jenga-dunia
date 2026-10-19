//! Compact previews for the viewport: a heightfield downsampled by `2^lod` and quantised to u16.
//!
//! Wire format (little-endian): `u32 width, u32 height, f32 min, f32 max, u16[width*height]`.
//! Row 0 is the south edge (field convention); the viewport maps y→z accordingly.

use crate::heightfield::Heightfield;

/// A downsampled, quantised heightfield.
#[derive(Debug, Clone, PartialEq)]
pub struct Preview {
    /// Width in samples.
    pub width: u32,
    /// Height in samples.
    pub height: u32,
    /// Height of value 0.
    pub min: f32,
    /// Height of value 65535.
    pub max: f32,
    /// Row-major samples.
    pub data: Vec<u16>,
}

/// Size of the fixed header in [`Preview::to_bytes`].
pub const HEADER_BYTES: usize = 16;

/// Build a preview at `lod` (0 = full resolution, 1 = half, …). Samples are nearest-neighbour so
/// a preview never invents heights outside the field's range.
pub fn preview_u16(hf: &Heightfield, lod: u8) -> Preview {
    let step = 1u32 << lod.min(12);
    let width = hf.width().div_ceil(step).max(1);
    let height = hf.height().div_ceil(step).max(1);
    let stats = hf.stats();
    let (min, max) = if stats.max > stats.min {
        (stats.min, stats.max)
    } else {
        (stats.min, stats.min + 1.0)
    };
    let scale = 65535.0 / (max - min);
    let mut data = Vec::with_capacity((width * height) as usize);
    for y in 0..height {
        for x in 0..width {
            let v = hf.get(x * step, y * step);
            data.push(((v - min) * scale).round().clamp(0.0, 65535.0) as u16);
        }
    }
    Preview {
        width,
        height,
        min,
        max,
        data,
    }
}

impl Preview {
    /// Serialise to the wire format.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER_BYTES + self.data.len() * 2);
        out.extend_from_slice(&self.width.to_le_bytes());
        out.extend_from_slice(&self.height.to_le_bytes());
        out.extend_from_slice(&self.min.to_le_bytes());
        out.extend_from_slice(&self.max.to_le_bytes());
        for v in &self.data {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out
    }

    /// Parse the wire format (used by tests and the CLI; the frontend has its own decoder).
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < HEADER_BYTES {
            return None;
        }
        let u32_at = |i: usize| u32::from_le_bytes(bytes[i..i + 4].try_into().ok()?).into();
        let f32_at = |i: usize| f32::from_le_bytes(bytes[i..i + 4].try_into().ok()?).into();
        let (width, height): (Option<u32>, Option<u32>) = (u32_at(0), u32_at(4));
        let (min, max): (Option<f32>, Option<f32>) = (f32_at(8), f32_at(12));
        let (width, height, min, max) = (width?, height?, min?, max?);
        let n = (width as usize).checked_mul(height as usize)?;
        let body = bytes.get(HEADER_BYTES..HEADER_BYTES + n * 2)?;
        let data = body
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect();
        Some(Self {
            width,
            height,
            min,
            max,
            data,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_downsamples_and_round_trips() {
        let mut hf = Heightfield::new(100, 60, 64, 2).expect("hf");
        hf.fill_with(|x, y| (x + y) as f32);
        let p = preview_u16(&hf, 2);
        assert_eq!((p.width, p.height), (25, 15));
        assert_eq!((p.min, p.max), (0.0, 158.0));
        assert_eq!(p.data[0], 0);
        assert_eq!(
            *p.data.last().expect("last"),
            ((96 + 56) as f32 / 158.0 * 65535.0).round() as u16
        );
        let bytes = p.to_bytes();
        assert_eq!(bytes.len(), HEADER_BYTES + 25 * 15 * 2);
        assert_eq!(Preview::from_bytes(&bytes).expect("parse"), p);
        assert!(Preview::from_bytes(&bytes[..HEADER_BYTES + 3]).is_none());
    }

    #[test]
    fn flat_field_does_not_divide_by_zero() {
        let hf = Heightfield::new(8, 8, 64, 0).expect("hf");
        let p = preview_u16(&hf, 0);
        assert!(p.data.iter().all(|&v| v == 0));
        assert_eq!(p.max - p.min, 1.0);
    }
}
