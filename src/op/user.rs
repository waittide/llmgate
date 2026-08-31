use crate::{
    model::{User, UserChangePassword, UserChangeUsername},
    op::AppState,
    utils::{hash_password, verify_password},
};
use anyhow::{anyhow, Result};

pub async fn refresh_user_cache(state: &AppState) -> Result<()> {
    let user: Option<User> = sqlx::query_as("SELECT id, username, password FROM users LIMIT 1")
        .fetch_optional(&state.db)
        .await?;

    if let Some(user) = user {
        let mut secret = state.jwt_secret.write();
        *secret = format!("{}_{}", user.username, user.password);
    }
    Ok(())
}

pub async fn get_user(state: &AppState) -> Result<User> {
    let user: Option<User> = sqlx::query_as("SELECT id, username, password FROM users LIMIT 1")
        .fetch_optional(&state.db)
        .await?;
    user.ok_or_else(|| anyhow!("User not found"))
}

pub async fn verify_user(state: &AppState, username: &str, password: &str) -> Result<bool> {
    let user = get_user(state).await?;
    if user.username != username {
        return Ok(false);
    }
    Ok(verify_password(password, &user.password))
}

pub async fn change_password(state: &AppState, req: &UserChangePassword) -> Result<()> {
    let user = get_user(state).await?;
    if !verify_password(&req.old_password, &user.password) {
        return Err(anyhow!("Incorrect old password"));
    }
    let new_hash = hash_password(&req.new_password);
    sqlx::query("UPDATE users SET password = ? WHERE id = ?")
        .bind(&new_hash)
        .bind(user.id)
        .execute(&state.db)
        .await?;

    refresh_user_cache(state).await?;
    Ok(())
}

pub async fn change_username(state: &AppState, req: &UserChangeUsername) -> Result<()> {
    let user = get_user(state).await?;
    sqlx::query("UPDATE users SET username = ? WHERE id = ?")
        .bind(&req.new_username)
        .bind(user.id)
        .execute(&state.db)
        .await?;

    refresh_user_cache(state).await?;
    Ok(())
}
