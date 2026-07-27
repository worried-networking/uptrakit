//! Agent-side interaction authoring (ADR-0028, spec D4). One declaration
//! carries the wire-descriptor inputs, placement metadata the wire
//! `InteractionDescriptor` cannot express, and the agent handler.

use crate::descriptor::{SurfaceActionUi, SurfaceRowCondition, SurfaceRowVisibleWhen};

/// Where the interaction appears on the owning agent surface.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentInteractionPlacement {
    /// Not placed in the action bar or rows (wizard steps, data loads,
    /// select-source feeders).
    Internal,
    /// Surface action bar (primary actions).
    Primary,
    /// Per-row table action.
    Row,
}

/// Handler an infrastructure plugin attaches to its agent interaction;
/// dispatched by the plugin's `GuestExec::handle_service_extension_action`
/// table lookup.
#[cfg(feature = "agent-infra")]
pub type AgentInteractionHandler = for<'a> fn(
    &'a crate::agent_infra::InfraPluginContext<'a>,
    &'a crate::surfaces::SurfaceActionRequest,
) -> std::pin::Pin<
    Box<dyn std::future::Future<Output = crate::surfaces::SurfaceActionResponse> + Send + 'a>,
>;

/// Single-source agent interaction declaration.
// Derives mirror the legacy per-surface action descriptor minus
// Serialize/Deserialize (the handler is not serializable — this type never
// crosses the wire) and minus PartialEq/Eq (fn-pointer comparison is
// unpredictable; nothing compares authoring declarations).
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct AgentInteraction {
    /// Interaction id (wire `InteractionId` string).
    pub action_id: String,
    /// Human label.
    pub label: String,
    /// Optional lucide icon name.
    pub icon: Option<String>,
    /// Optional form/wizard UI.
    pub ui: Option<SurfaceActionUi>,
    /// Required permission ("" = none, mirrors the legacy descriptor).
    pub permission: String,
    /// Destructive marker (with `confirm_entity_field` derives
    /// `ConfirmableAction`).
    pub destructive: bool,
    /// Timeout in seconds (clamped 1–300 at derivation).
    pub timeout_seconds: Option<u32>,
    /// Entity field named in the confirmation prompt.
    pub confirm_entity_field: Option<String>,
    /// Row-visibility condition (row placements).
    pub row_visible_when: Option<SurfaceRowVisibleWhen>,
    /// Placement on the owning surface.
    pub placement: AgentInteractionPlacement,
    /// Agent-side handler (infrastructure plugins; the runtime's built-ins
    /// dispatch inline and leave this `None`).
    #[cfg(feature = "agent-infra")]
    pub agent_handler: Option<AgentInteractionHandler>,
    /// HTTP method for the derived wire descriptor. `None` keeps the wire
    /// default (POST; DataLoads normalize to GET at admission).
    pub http_method: Option<crate::surfaces::InteractionHttpMethod>,
}

impl AgentInteraction {
    /// Starts a declaration with defaults mirroring the legacy action
    /// descriptor's constructor.
    pub fn new(action_id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            action_id: action_id.into(),
            label: label.into(),
            icon: None,
            ui: None,
            permission: String::new(),
            destructive: false,
            timeout_seconds: None,
            confirm_entity_field: None,
            row_visible_when: None,
            placement: AgentInteractionPlacement::Internal,
            #[cfg(feature = "agent-infra")]
            agent_handler: None,
            http_method: None,
        }
    }

    /// Set the lucide-canonical kebab-case icon name (e.g. `"refresh-cw"`).
    /// Validation lives in the wire layer; this builder accepts the value verbatim.
    #[must_use]
    pub fn with_icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// Set the action UI (form or wizard shown before invocation).
    pub fn with_ui(mut self, ui: SurfaceActionUi) -> Self {
        self.ui = Some(ui);
        self
    }

    /// Set the required permission.
    pub fn with_permission(mut self, permission: impl Into<String>) -> Self {
        self.permission = permission.into();
        self
    }

    /// Mark this action as destructive.
    pub fn destructive(mut self) -> Self {
        self.destructive = true;
        self
    }

    /// Set the timeout in seconds.
    pub fn with_timeout(mut self, seconds: u32) -> Self {
        self.timeout_seconds = Some(seconds);
        self
    }

    /// Set conditional visibility for row actions in a `DataTable`.
    pub fn with_row_visible_when(
        mut self,
        field: impl Into<String>,
        condition: SurfaceRowCondition,
    ) -> Self {
        self.row_visible_when = Some(SurfaceRowVisibleWhen {
            field: field.into(),
            condition,
        });
        self
    }

    /// Set the row data field used as the entity name in confirmation dialogs.
    pub fn with_confirm_entity_field(mut self, field: impl Into<String>) -> Self {
        self.confirm_entity_field = Some(field.into());
        self
    }

    /// Sets the surface placement.
    pub fn placement(mut self, placement: AgentInteractionPlacement) -> Self {
        self.placement = placement;
        self
    }

    /// Attaches the agent-side handler.
    #[cfg(feature = "agent-infra")]
    pub fn with_agent_handler(mut self, handler: AgentInteractionHandler) -> Self {
        self.agent_handler = Some(handler);
        self
    }

    /// Declare the HTTP method the derived wire descriptor dispatches under.
    #[must_use]
    pub fn with_http_method(mut self, method: crate::surfaces::InteractionHttpMethod) -> Self {
        self.http_method = Some(method);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_populates_fields_and_defaults() {
        let interaction = AgentInteraction::new("sync-host", "Sync")
            .with_icon("refresh-cw")
            .with_timeout(30)
            .destructive()
            .with_confirm_entity_field("name")
            .placement(AgentInteractionPlacement::Row);
        assert_eq!(interaction.action_id, "sync-host");
        assert_eq!(interaction.label, "Sync");
        assert_eq!(interaction.icon.as_deref(), Some("refresh-cw"));
        assert_eq!(interaction.timeout_seconds, Some(30));
        assert!(interaction.destructive);
        assert_eq!(interaction.confirm_entity_field.as_deref(), Some("name"));
        assert!(matches!(
            interaction.placement,
            AgentInteractionPlacement::Row
        ));

        let default = AgentInteraction::new("list-hosts", "List Hosts");
        assert!(matches!(
            default.placement,
            AgentInteractionPlacement::Internal
        ));
        assert!(!default.destructive);
        assert!(default.ui.is_none());
    }
}
