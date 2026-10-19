//! Units and world settings shared by the project document and the terrain crates.

use serde::{Deserialize, Serialize};

/// Which axis points up in the canonical scene frame.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, Default,
)]
pub enum UpAxis {
    /// glTF convention (canonical).
    #[default]
    Y,
    /// Blender/Unreal convention.
    Z,
}

/// Coordinate-system handedness.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, Default,
)]
#[serde(rename_all = "snake_case")]
pub enum Handedness {
    /// glTF/USD/Blender (canonical).
    #[default]
    Right,
    /// Unity/Unreal.
    Left,
}

/// Scene units. Canonical frame is Y-up, right-handed, metres (glTF).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Units {
    /// Metres per scene unit (1.0 = metres).
    #[serde(rename = "metersPerUnit")]
    pub meters_per_unit: f64,
    /// Up axis.
    #[serde(rename = "upAxis")]
    pub up_axis: UpAxis,
    /// Handedness.
    pub handedness: Handedness,
}

impl Default for Units {
    fn default() -> Self {
        Self {
            meters_per_unit: 1.0,
            up_axis: UpAxis::Y,
            handedness: Handedness::Right,
        }
    }
}

/// World extent, resolution and tiling (`docs/spec/project-format.md`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct WorldSettings {
    /// Extent in metres, [x, y].
    pub extent_m: [f64; 2],
    /// Heightfield resolution in pixels, [x, y].
    pub resolution_px: [u32; 2],
    /// Tile edge length in pixels (power of two).
    pub tile_size: u32,
    /// Halo width in pixels around each tile.
    pub halo: u32,
    /// World-space origin of pixel (0,0), metres.
    pub origin: [f64; 2],
    /// Optional CRS identifier (e.g. "EPSG:32637") when georeferenced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crs: Option<String>,
    /// Master seed for all procedural randomness.
    pub seed: u64,
}

impl Default for WorldSettings {
    fn default() -> Self {
        Self {
            extent_m: [4096.0, 4096.0],
            resolution_px: [4096, 4096],
            tile_size: 512,
            halo: 4,
            origin: [0.0, 0.0],
            crs: None,
            seed: 42,
        }
    }
}

impl WorldSettings {
    /// Validate invariants that every consumer relies on.
    pub fn validate(&self) -> Result<(), String> {
        if !self.tile_size.is_power_of_two() || self.tile_size < 64 {
            return Err(format!(
                "tile_size must be a power of two ≥ 64, got {}",
                self.tile_size
            ));
        }
        if self.resolution_px.iter().any(|&r| r == 0 || r > 65_536) {
            return Err(format!(
                "resolution_px must be within 1..=65536, got {:?}",
                self.resolution_px
            ));
        }
        if self.extent_m.iter().any(|&e| e <= 0.0 || !e.is_finite()) {
            return Err(format!(
                "extent_m must be finite and positive, got {:?}",
                self.extent_m
            ));
        }
        if self.halo > self.tile_size / 2 {
            return Err(format!(
                "halo {} exceeds half the tile size {}",
                self.halo, self.tile_size
            ));
        }
        Ok(())
    }

    /// Metres per pixel along x and y.
    pub fn spacing_m(&self) -> [f64; 2] {
        [
            self.extent_m[0] / self.resolution_px[0] as f64,
            self.extent_m[1] / self.resolution_px[1] as f64,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn default_world_is_valid() {
        WorldSettings::default().validate().expect("default valid");
        assert_eq!(WorldSettings::default().spacing_m(), [1.0, 1.0]);
    }

    proptest! {
        #[test]
        fn validate_rejects_non_power_of_two_tiles(tile in 65u32..4096) {
            prop_assume!(!tile.is_power_of_two());
            let w = WorldSettings { tile_size: tile, ..Default::default() };
            prop_assert!(w.validate().is_err());
        }

        #[test]
        fn validate_rejects_non_finite_extent(x in prop::num::f64::ANY) {
            prop_assume!(!(x > 0.0 && x.is_finite()));
            let w = WorldSettings { extent_m: [x, 1.0], ..Default::default() };
            prop_assert!(w.validate().is_err());
        }
    }
}
