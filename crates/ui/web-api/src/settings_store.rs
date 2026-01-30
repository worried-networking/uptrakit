use crate::auth::Result;
use rootcause::prelude::*;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
use time::OffsetDateTime;
use uptrakit_shared_db::entity::{prelude::*, setting};

pub async fn upsert_setting(db: &DatabaseConnection, key: &str, value: &str) -> Result<()> {
    let now = OffsetDateTime::now_utc();
    let existing = Setting::find_by_id(key.to_string())
        .one(db)
        .await
        .context_to()?;

    if let Some(existing) = existing {
        let mut model: setting::ActiveModel = existing.into();
        model.value = Set(value.to_string());
        model.updated_at = Set(now);
        model.update(db).await.context_to()?;
    } else {
        let model = setting::ActiveModel {
            key: Set(key.to_string()),
            value: Set(value.to_string()),
            updated_at: Set(now),
        };
        model.insert(db).await.context_to()?;
    }

    Ok(())
}

pub async fn load_setting(db: &DatabaseConnection, key: &str) -> Result<Option<String>> {
    let setting = Setting::find_by_id(key.to_string())
        .one(db)
        .await
        .context_to()?;
    Ok(setting.map(|s| s.value))
}

pub async fn delete_setting(db: &DatabaseConnection, key: &str) -> Result<()> {
    Setting::delete_many()
        .filter(setting::Column::Key.eq(key))
        .exec(db)
        .await
        .context_to()?;
    Ok(())
}
