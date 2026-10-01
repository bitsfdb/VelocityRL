/*
 * velocityrl
 * Copyright (c) 2026 bits (https://github.com/bitsfdb/velocityrl)
 * 
 * Licensed under the GNU General Public License v3.0.
 * unauthorized rebranding or stripping of this copyright notice is strictly prohibited.
 */
pub mod crypto;
pub mod parser;
pub mod compression;
pub mod nametable;
pub mod swapper;
pub mod palette;
pub mod tagame_swapper;
pub mod decal_compiler;

pub use swapper::{swap_asset, restore_single, restore_all, SwapOptions, SwapError};
pub use palette::{PaletteStatus, PaletteError};
pub use tagame_swapper::{TagameSwapItem, TagameSwapperStatus, TagameSwapError};
pub use decal_compiler::{CustomDecalConfig, DetectedCarInfo, ParsedDecalPackage};

