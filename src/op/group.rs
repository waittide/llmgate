use anyhow::{Result, anyhow};
use sqlx::Row;

use crate::{
    model::{Group, GroupActiveItemUpdateRequest, GroupItem, GroupMode, GroupRelayConfig, GroupUpdateRequest},
    op::AppState,
};

pub async fn refresh_group_cache(state: &AppState) -> Result<()> {
    let rows = sqlx::query("SELECT id, name, mode, active_item_id, relay_config FROM groups ORDER BY id ASC")
        .fetch_all(&state.db)
        .await?;

    let mut local_groups = std::collections::HashMap::new();
    let mut local_name_index = std::collections::HashMap::new();
    for row in rows {
        let id: i64 = row.get("id");
        let name: String = row.get("name");
        let mode_str: String = row.get("mode");
        let active_item_id: i64 = row.get("active_item_id");
        let relay_config_json: String = row.get("relay_config");
        let relay_config: GroupRelayConfig = serde_json::from_str(&relay_config_json).unwrap_or_default();

        let group = Group {
            id,
            name: name.clone(),
            mode: GroupMode::from_str(&mode_str),
            active_item_id,
            relay_config,
            items: Vec::new(),
        };

        local_name_index.insert(name, id);
        local_groups.insert(id, group);
    }

    // Fetch all group items
    let item_rows = sqlx::query(
        r#"
        SELECT gi.id, gi.group_id, gi.channel_model_id, gi.priority
        FROM group_items gi
        ORDER BY gi.priority ASC, gi.id ASC
        "#,
    )
    .fetch_all(&state.db)
    .await?;

    for row in item_rows {
        let id: i64 = row.get("id");
        let group_id: i64 = row.get("group_id");
        let channel_model_id: i64 = row.get("channel_model_id");
        let priority: i64 = row.get("priority");

        let channel_model = state
            .channel_models
            .get(&channel_model_id)
            .map(|cm| cm.clone());

        let item = GroupItem {
            id,
            group_id,
            channel_model_id,
            channel_model,
            priority,
        };

        if let Some(group) = local_groups.get_mut(&group_id) {
            group.items.push(item);
        }
    }

    *state.groups.write() = crate::op::GroupCache {
        by_id:   local_groups,
        by_name: local_name_index,
    };
    Ok(())
}

pub fn list_groups(state: &AppState) -> Vec<Group> {
    let mut list: Vec<Group> = state.groups.read().by_id.values().cloned().collect();
    list.sort_by_key(|g| g.id);
    list
}

pub fn list_model_names(state: &AppState) -> Vec<String> {
    let mut names: Vec<String> = state.groups.read().by_name.keys().cloned().collect();
    names.sort();
    names
}

pub fn get_group_by_name(state: &AppState, name: &str) -> Result<Group> {
    let cache = state.groups.read();
    let group_id = cache
        .by_name
        .get(name)
        .ok_or_else(|| anyhow!("Group not found for model: {}", name))?;
    cache
        .by_id
        .get(group_id)
        .cloned()
        .ok_or_else(|| anyhow!("Group not found for model: {}", name))
}

pub fn get_group_by_id(state: &AppState, id: i64) -> Result<Group> {
    state
        .groups
        .read()
        .by_id
        .get(&id)
        .cloned()
        .ok_or_else(|| anyhow!("Group not found with id: {}", id))
}

pub async fn create_group(state: &AppState, group: &mut Group) -> Result<()> {
    let relay_config_json = serde_json::to_string(&group.relay_config)?;
    let mut tx = state.db.begin().await?;
    let result = sqlx::query("INSERT INTO groups (name, mode, active_item_id, relay_config) VALUES (?, ?, ?, ?)")
        .bind(&group.name)
        .bind(group.mode.as_str())
        .bind(group.active_item_id)
        .bind(&relay_config_json)
        .execute(&mut *tx)
        .await?;

    let group_id = result.last_insert_rowid();
    group.id = group_id;

    for item in &mut group.items {
        item.group_id = group_id;
        let item_res = sqlx::query("INSERT INTO group_items (group_id, channel_model_id, priority) VALUES (?, ?, ?)")
            .bind(group_id)
            .bind(item.channel_model_id)
            .bind(item.priority)
            .execute(&mut *tx)
            .await?;
        item.id = item_res.last_insert_rowid();
    }

    tx.commit().await?;
    refresh_group_cache(state).await?;
    Ok(())
}

pub async fn update_group(state: &AppState, req: &GroupUpdateRequest) -> Result<Group> {
    let current = get_group_by_id(state, req.id)?;

    let name = req.name.as_ref().unwrap_or(&current.name);
    let mode = req.mode.as_ref().unwrap_or(&current.mode);
    let relay_config = req.relay_config.as_ref().unwrap_or(&current.relay_config);
    let relay_config_json = serde_json::to_string(relay_config)?;
    let mut tx = state.db.begin().await?;

    sqlx::query("UPDATE groups SET name = ?, mode = ?, relay_config = ? WHERE id = ?")
        .bind(name)
        .bind(mode.as_str())
        .bind(&relay_config_json)
        .bind(req.id)
        .execute(&mut *tx)
        .await?;

    for item in &req.items_to_add {
        sqlx::query("INSERT OR IGNORE INTO group_items (group_id, channel_model_id, priority) VALUES (?, ?, ?)")
            .bind(req.id)
            .bind(item.channel_model_id)
            .bind(item.priority)
            .execute(&mut *tx)
            .await?;
    }

    for item in &req.items_to_update {
        sqlx::query("UPDATE group_items SET priority = ? WHERE id = ? AND group_id = ?")
            .bind(item.priority)
            .bind(item.id)
            .bind(req.id)
            .execute(&mut *tx)
            .await?;
    }

    for item_id in &req.items_to_delete {
        sqlx::query("DELETE FROM group_items WHERE id = ? AND group_id = ?")
            .bind(item_id)
            .bind(req.id)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await?;
    refresh_group_cache(state).await?;
    get_group_by_id(state, req.id)
}

pub async fn set_group_active_item(
    state: &AppState,
    group_id: i64,
    req: &GroupActiveItemUpdateRequest,
) -> Result<Group> {
    let active_item_id = req.item_id.unwrap_or(0);
    sqlx::query("UPDATE groups SET active_item_id = ? WHERE id = ?")
        .bind(active_item_id)
        .bind(group_id)
        .execute(&state.db)
        .await?;

    refresh_group_cache(state).await?;
    get_group_by_id(state, group_id)
}

pub async fn delete_group(state: &AppState, id: i64) -> Result<()> {
    sqlx::query("DELETE FROM groups WHERE id = ?")
        .bind(id)
        .execute(&state.db)
        .await?;

    refresh_group_cache(state).await?;
    Ok(())
}
