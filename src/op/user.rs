use anyhow::{Result, anyhow};

use crate::{
    model::{User, UserChangePassword, UserChangeUsername},
    op::AppState,
};

/// 确保 JWT 签名密钥存在：首次生成高熵随机值并持久化到 settings，跨重启稳定。
/// 不由账号密码派生；修改密码不再使已签发 token 失效（个人部署可接受）。
pub async fn ensure_jwt_secret(state: &AppState) -> Result<()> {
    if let Some(secret) = state.settings.get("jwt_secret") {
        *state.jwt_secret.write() = secret.value().clone();
        return Ok(());
    }
    let secret = nanoid::nanoid!(64);
    crate::op::setting::set_setting_string(state, "jwt_secret", &secret).await?;
    *state.jwt_secret.write() = secret;
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
    Ok(())
}

pub async fn change_username(state: &AppState, req: &UserChangeUsername) -> Result<()> {
    let user = get_user(state).await?;
    sqlx::query("UPDATE users SET username = ? WHERE id = ?")
        .bind(&req.new_username)
        .bind(user.id)
        .execute(&state.db)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::setup_test_state;

    #[tokio::test]
    async fn change_password_verifies_old_password() {
        let (state, db_path) = setup_test_state("user_pw").await;

        // 旧密码错误时拒绝修改
        let bad = UserChangePassword {
            old_password: "nope".into(),
            new_password: "new-pass".into(),
        };
        assert!(change_password(&state, &bad).await.is_err());
        assert!(verify_user(&state, "admin", "admin").await.unwrap());

        // 旧密码正确时更新为新密码
        let good = UserChangePassword {
            old_password: "admin".into(),
            new_password: "new-pass".into(),
        };
        change_password(&state, &good).await.unwrap();
        assert!(verify_user(&state, "admin", "new-pass").await.unwrap());
        assert!(!verify_user(&state, "admin", "admin").await.unwrap());
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn jwt_secret_is_random_and_persistent() {
        let (state, db_path) = setup_test_state("user_jwt").await;
        let secret = state.jwt_secret.read().clone();
        assert!(!secret.is_empty());
        assert_ne!(secret, "llmgate-jwt-secret-default");
        // 密钥持久化在 settings 中，与内存缓存一致
        assert_eq!(
            state.settings.get("jwt_secret").map(|e| e.value().clone()),
            Some(secret.clone())
        );
        // 模拟重启：同一数据库重建 AppState，密钥保持稳定
        let state2 = AppState::init(state.db.clone()).await.unwrap();
        assert_eq!(*state2.jwt_secret.read(), secret);
        let _ = std::fs::remove_file(db_path);
    }
}
