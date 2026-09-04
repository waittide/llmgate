use anyhow::{Result, anyhow};

use crate::{
    model::{User, UserChangePassword, UserChangeUsername},
    op::AppState,
};

/// 刷新用户缓存；副作用：将 JWT 密钥派生为 username_password（改密码/用户名即全员登出）。
pub async fn refresh_user_cache(state: &AppState) -> Result<()> {
    let user: Option<User> = sqlx::query_as("SELECT id, username, password FROM users LIMIT 1")
        .fetch_optional(&state.db)
        .await?;

    // JWT 密钥派生：密钥稳定于当前账号密码，跨重启不变；修改密码即全部 token 失效。
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
    user.ok_or_else(|| anyhow!("用户不存在"))
}

pub async fn verify_user(state: &AppState, username: &str, password: &str) -> Result<bool> {
    let user = get_user(state).await?;
    if user.username != username {
        return Ok(false);
    }
    Ok(user.password == password)
}

pub async fn change_password(state: &AppState, req: &UserChangePassword) -> Result<()> {
    let user = get_user(state).await?;
    if user.password != req.old_password {
        return Err(anyhow!("旧密码错误"));
    }
    sqlx::query("UPDATE users SET password = ? WHERE id = ?")
        .bind(&req.new_password)
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
