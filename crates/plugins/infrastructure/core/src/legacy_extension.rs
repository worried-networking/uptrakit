//! Internal compatibility seam for legacy extension-framework types.
//!
//! This module intentionally re-exports the existing
//! `uptrakit_extension_framework` types without changing type identity.

pub use uptrakit_extension_framework::{
    ActionDef, ActionUi, ApiSubmitDef, ContextSelectorDef, ContextSelectorSource,
    ExtensionManifest, ExtensionPlacement, ExtensionRequestPayload, ExtensionResponsePayload,
    ExtensionTargeting, ExtensionUi, FieldDef, FieldType, FormDef, PanelPosition, RowCondition,
    RowVisibleWhen, SelectOption, SelectSource, TableColumn, WizardStep,
};
