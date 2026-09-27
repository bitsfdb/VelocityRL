pub mod crypto;
pub mod parser;
pub mod compression;
pub mod nametable;
pub mod swapper;
pub mod palette;
pub mod tagame_swapper;
pub mod decal_compiler;
pub mod ini_swapper;

pub use swapper::{swap_asset, restore_single, restore_all, SwapOptions, SwapError};
pub use palette::{PaletteStatus, PaletteError};
pub use tagame_swapper::{TagameSwapItem, TagameSwapperStatus, TagameSwapError, apply_tagame_ini_hook};
pub use decal_compiler::{CustomDecalConfig, DetectedCarInfo, ParsedDecalPackage};
pub use ini_swapper::{write_swaps_ini, read_swaps_ini, write_decals_ini, read_decals_ini, SwapsIniConfig, DecalsIniConfig};

