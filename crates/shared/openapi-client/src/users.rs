use crate::Result;
use crate::UptrakitClient;
use crate::generated::types::users::{
    ApplyPresetRequest, UpdateUserActiveRequest, UpdateUserRolesRequest, UserWithRolesResponse,
};
use uuid::Uuid;

impl UptrakitClient {
    /// List all users with their roles and resolved permissions.
    pub async fn list_users(&self) -> Result<Vec<UserWithRolesResponse>> {
        self.get(crate::paths::users::BASE).await
    }

    /// Get a single user by ID with roles and permissions.
    pub async fn get_user(&self, id: &Uuid) -> Result<UserWithRolesResponse> {
        self.get(&crate::paths::users::by_id(id)).await
    }

    /// Replace a user's roles.
    pub async fn update_user_roles(
        &self,
        id: &Uuid,
        req: &UpdateUserRolesRequest,
    ) -> Result<UserWithRolesResponse> {
        self.put_json(&crate::paths::users::roles(id), req).await
    }

    /// Activate or deactivate a user.
    pub async fn update_user_active(
        &self,
        id: &Uuid,
        req: &UpdateUserActiveRequest,
    ) -> Result<UserWithRolesResponse> {
        self.put_json(&crate::paths::users::active(id), req).await
    }

    /// Apply an access preset to a user, replacing their roles.
    pub async fn apply_preset(
        &self,
        id: &Uuid,
        req: &ApplyPresetRequest,
    ) -> Result<UserWithRolesResponse> {
        self.post_json(&crate::paths::users::apply_preset(id), req)
            .await
    }
}

#[cfg(test)]
mod tests {
    use crate::generated::types::users::{
        ApplyPresetRequest, UpdateUserActiveRequest, UpdateUserRolesRequest,
    };

    #[test]
    fn update_user_roles_request_serialization() {
        let id: uuid::Uuid = "550e8400-e29b-41d4-a716-446655440000".parse().unwrap();
        let req = UpdateUserRolesRequest { role_ids: vec![id] };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["role_ids"][0], "550e8400-e29b-41d4-a716-446655440000");
    }

    #[test]
    fn update_user_active_request_serialization() {
        let req = UpdateUserActiveRequest { is_active: false };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["is_active"], false);
    }

    #[test]
    fn apply_preset_request_serialization() {
        let req = ApplyPresetRequest {
            preset: "admin".to_string(),
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["preset"], "admin");
    }
}
