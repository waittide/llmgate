use axum::{Json, extract::State, http::StatusCode, response::Response};

use crate::{
    model::{UserChangePassword, UserChangeUsername, UserLogin},
    op::{AppState, user},
    server::{
        auth::generate_jwt,
        resp::{error, success_msg},
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
                Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
            }
        }
        _ => error(StatusCode::UNAUTHORIZED, "Authentication failed"),
    }
}

pub async fn change_password(State(state): State<AppState>, Json(payload): Json<UserChangePassword>) -> Response {
    match user::change_password(&state, &payload).await {
        Ok(_) => success_msg("password changed successfully"),
        Err(e) => error(StatusCode::BAD_REQUEST, &e.to_string()),
    }
}

pub async fn change_username(State(state): State<AppState>, Json(payload): Json<UserChangeUsername>) -> Response {
    match user::change_username(&state, &payload).await {
        Ok(_) => success_msg("username changed successfully"),
        Err(e) => error(StatusCode::BAD_REQUEST, &e.to_string()),
    }
}

pub async fn status() -> Response {
    success_msg("ok")
}

#[cfg(test)]
mod tests {
    use axum::http::header;

    use super::*;
    use crate::db::init_db;
    #[tokio::test]
    async fn test_login_headers() {
        let rnd = nanoid::nanoid!(12);
        let db_path = std::env::temp_dir().join(format!("octopus_login_test_{}.db", rnd));
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
