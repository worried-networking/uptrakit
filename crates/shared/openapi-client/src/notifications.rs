use crate::generated::types::notifications::{
    CreateNotificationChannelRequest, CreateNotificationRuleRequest, NotificationChannelResponse,
    NotificationLogResponse, NotificationRuleResponse, TestNotificationResponse,
    UpdateNotificationChannelRequest, UpdateNotificationRuleRequest,
};
use crate::generated::types::pagination::{PaginatedResponse, PaginationParams};
use uuid::Uuid;

use crate::Result;
use crate::paths;

impl crate::UptrakitClient {
    // ── Channels ────────────────────────────────────────────────────

    /// Create a new notification channel.
    pub async fn create_notification_channel(
        &self,
        req: &CreateNotificationChannelRequest,
    ) -> Result<NotificationChannelResponse> {
        self.post_json(paths::notifications::CHANNELS, req).await
    }

    /// List notification channels with pagination.
    pub async fn list_notification_channels(
        &self,
        params: &PaginationParams,
    ) -> Result<PaginatedResponse<NotificationChannelResponse>> {
        self.get_with_query(paths::notifications::CHANNELS, params)
            .await
    }

    /// Get a single notification channel by ID.
    pub async fn get_notification_channel(&self, id: &Uuid) -> Result<NotificationChannelResponse> {
        self.get(&paths::notifications::channel_by_id(id)).await
    }

    /// Update an existing notification channel.
    pub async fn update_notification_channel(
        &self,
        id: &Uuid,
        req: &UpdateNotificationChannelRequest,
    ) -> Result<NotificationChannelResponse> {
        self.put_json(&paths::notifications::channel_by_id(id), req)
            .await
    }

    /// Delete a notification channel.
    pub async fn delete_notification_channel(&self, id: &Uuid) -> Result<()> {
        self.delete(&paths::notifications::channel_by_id(id)).await
    }

    /// Send a test notification through a channel.
    pub async fn test_notification_channel(&self, id: &Uuid) -> Result<TestNotificationResponse> {
        self.post_empty(&paths::notifications::test_channel(id))
            .await
    }

    // ── Rules ───────────────────────────────────────────────────────

    /// Create a new notification rule.
    pub async fn create_notification_rule(
        &self,
        req: &CreateNotificationRuleRequest,
    ) -> Result<NotificationRuleResponse> {
        self.post_json(paths::notifications::RULES, req).await
    }

    /// List notification rules with pagination.
    pub async fn list_notification_rules(
        &self,
        params: &PaginationParams,
    ) -> Result<PaginatedResponse<NotificationRuleResponse>> {
        self.get_with_query(paths::notifications::RULES, params)
            .await
    }

    /// Get a single notification rule by ID.
    pub async fn get_notification_rule(&self, id: &Uuid) -> Result<NotificationRuleResponse> {
        self.get(&paths::notifications::rule_by_id(id)).await
    }

    /// Update an existing notification rule.
    pub async fn update_notification_rule(
        &self,
        id: &Uuid,
        req: &UpdateNotificationRuleRequest,
    ) -> Result<NotificationRuleResponse> {
        self.put_json(&paths::notifications::rule_by_id(id), req)
            .await
    }

    /// Delete a notification rule.
    pub async fn delete_notification_rule(&self, id: &Uuid) -> Result<()> {
        self.delete(&paths::notifications::rule_by_id(id)).await
    }

    // ── Log ─────────────────────────────────────────────────────────

    /// List the notification log with pagination.
    pub async fn list_notification_log(
        &self,
        params: &PaginationParams,
    ) -> Result<PaginatedResponse<NotificationLogResponse>> {
        self.get_with_query(paths::notifications::LOG, params).await
    }
}
