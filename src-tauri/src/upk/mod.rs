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
pub mod legacy_swapper;
pub mod swapper;
pub mod palette;
pub mod decal_compiler;

pub use swapper::{
    TagameSwapItem, TagameSwapperStatus, TagameSwapError, ensure_projectx_patched,
    TagameSwapItem as SwapItem, TagameSwapperStatus as SwapperStatus, TagameSwapError as SwapError,
};
pub use legacy_swapper::{swap_asset, restore_single, restore_all, SwapOptions};
pub use palette::{PaletteStatus, PaletteError};
pub use decal_compiler::{CustomDecalConfig, DetectedCarInfo, ParsedDecalPackage};

