//! Numeric conversion helpers used during the migration to common patch types.
//!
//! These functions are temporary shims to bridge `usize` values from legacy
//! code with the `u32` indices used in the shared patch structures.

use crate::core::errors::DevItError;

/// Convert `usize` to `u32`, saturating at `u32::MAX` on overflow.
pub fn u32_sat(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Convert `usize` to `u32`, returning an internal error if the value does not fit.
pub fn u32_checked(n: usize) -> Result<u32, DevItError> {
    u32::try_from(n).map_err(|_| DevItError::internal(format!("usize→u32 overflow: {n}")))
}
