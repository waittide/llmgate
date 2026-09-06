use axum::{Json, extract::State, http::StatusCode, response::Response};

use crate::{
    model::{UserChangePassword, UserChangeUsername, UserLogin},
    op::{AppState, user},
    server::{
        auth::generate_jwt,
        resp::{error, internal_error, success_msg},
    },
};

pub async fn login(State(state): State<AppState>, Json(payload): Json<UserLogin>) -> Response {
    match user::verify_user(&state, &payload.username, &payload.password).await {
        Ok(true) => {
            let secret = state.jwt_secret.read().clone();
            match generate_jwt(&payload.username, &secret, payload.expire) {
                Ok((token, max_age)) => {
                    let data = serde_json::json!({
                        "token": token,
                        "max_age": max_age,
                    });
                    crate::server::resp::success(data)
                }
                Err(e) => internal_error(e),
            }
        }
        Ok(false) | Err(_) => error(StatusCode::UNAUTHORIZED, "鉴权失败"),
    }
}

pub async fn change_password(State(state): State<AppState>, Json(payload): Json<UserChangePassword>) -> Response {
    match user::change_password(&state, &payload).await {
        Ok(_) => success_msg("密码修改成功"),
        Err(e) => error(StatusCode::BAD_REQUEST, &e.to_string()),
    }
}

pub async fn change_username(State(state): State<AppState>, Json(payload): Json<UserChangeUsername>) -> Response {
    match user::change_username(&state, &payload).await {
        Ok(_) => success_msg("用户名修改成功"),
        Err(e) => error(StatusCode::BAD_REQUEST, &e.to_string()),
    }
}

pub async fn status() -> Response {
    success_msg("服务正常")
}

#[cfg(test)]
mod tests {
    use axum::http::header;

    use super::*;
    use crate::db::init_db;
    #[tokio::test]
    async fn test_login_headers() {
        let rnd = nanoid::nanoid!(12);
        let db_path = std::env::temp_dir().join(format!("llmgate_login_test_{}.db", rnd));
        let pool = init_db(&db_path).await.unwrap();
        let state = AppState::init(pool).await.unwrap();

        let payload = Json(UserLogin {
            username: "admin".to_string(),
            password: "admin".to_string(),
            expire:   3600,
        });

        let resp = login(State(state), payload).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(
            resp.headers().contains_key(header::CONTENT_TYPE),
            "Content-Type must be present"
        );
        assert_eq!(resp.headers().get(header::CONTENT_TYPE).unwrap(), "application/json");
    }
}
