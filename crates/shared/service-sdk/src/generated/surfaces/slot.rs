// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.
#![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]
use crate::generated::surfaces::{IdentifierError, validate_surface_identifier};
use thiserror::Error;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SurfaceSlotDef {
    pub id: &'static str,
    pub multi_entry: bool,
    pub provider_priority_min: i32,
    pub provider_priority_max: i32,
}
impl SurfaceSlotDef {
    pub const fn single_entry(
        id: &'static str,
        provider_priority_min: i32,
        provider_priority_max: i32,
    ) -> Self {
        Self {
            id,
            multi_entry: false,
            provider_priority_min,
            provider_priority_max,
        }
    }
    pub const fn multi_entry(
        id: &'static str,
        provider_priority_min: i32,
        provider_priority_max: i32,
    ) -> Self {
        Self {
            id,
            multi_entry: true,
            provider_priority_min,
            provider_priority_max,
        }
    }
}
pub const SLOT_SETTINGS_TABS: &str = "settings.tabs";
pub const SLOT_SETTINGS_BELOW_GLOBAL: &str = "settings.below.global";
pub const SLOT_SOFTWARE_TABS: &str = "software.tabs";
pub const SLOT_HOST_DETAIL_TABS: &str = "host_detail.tabs";
pub const SLOT_SOFTWARE_ITEM_TABS: &str = "software_item.tabs";
pub const SLOT_SOFTWARE_ITEM_HOST_CONTEXT_MENU: &str = "software_item.host_context_menu";
pub const SLOT_SURFACE_PAGE: &str = "surface.page";
const SURFACE_SLOT_DEFS: [SurfaceSlotDef; 7] = [
    SurfaceSlotDef::multi_entry(SLOT_SETTINGS_TABS, 100, 999),
    SurfaceSlotDef::multi_entry(SLOT_SETTINGS_BELOW_GLOBAL, 100, 999),
    SurfaceSlotDef::multi_entry(SLOT_SOFTWARE_TABS, 100, 999),
    SurfaceSlotDef::multi_entry(SLOT_HOST_DETAIL_TABS, 100, 999),
    SurfaceSlotDef::multi_entry(SLOT_SOFTWARE_ITEM_TABS, 100, 999),
    SurfaceSlotDef::single_entry(SLOT_SOFTWARE_ITEM_HOST_CONTEXT_MENU, 100, 999),
    SurfaceSlotDef::single_entry(SLOT_SURFACE_PAGE, 100, 999),
];
pub fn all_surface_slots() -> &'static [SurfaceSlotDef] {
    &SURFACE_SLOT_DEFS
}
pub fn slot_def(slot_id: &str) -> Option<&'static SurfaceSlotDef> {
    SURFACE_SLOT_DEFS.iter().find(|slot| slot.id == slot_id)
}
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SlotValidationError {
    #[error(transparent)]
    InvalidIdentifier(#[from] IdentifierError),
    #[error("slot `{0}` is not declared in the slot registry")]
    UnknownSlot(String),
}
/// Validates a surface slot identifier and resolves its registered slot definition.
///
/// # Errors
///
/// Returns [`SlotValidationError::InvalidIdentifier`] if `slot_id` is not a valid
/// surface identifier, or [`SlotValidationError::UnknownSlot`] if the identifier
/// is valid but not declared in the slot registry.
pub fn validate_slot_id(slot_id: &str) -> Result<&'static SurfaceSlotDef, SlotValidationError> {
    validate_surface_identifier(slot_id)?;
    slot_def(slot_id).ok_or_else(|| SlotValidationError::UnknownSlot(slot_id.to_owned()))
}
