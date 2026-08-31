use crate::{
    model::{UserChangePassword, UserChangeUsername, UserLogin},
    op::{user, AppState},
    server::{
        auth::generate_jwt,
        resp::{error, success_msg},
    },
};
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};

pub async fn login(
    State(state): State<AppState>,
    jar: CookieJar,
    Json(payload): Json<UserLogin>,
) -> Response {
    match user::verify_user(&state, &payload.username, &payload.password).await {
        Ok(true) => {
            let secret = state.jwt_secret.read().clone();
            match generate_jwt(&payload.username, &secret, payload.expire) {
                Ok((token, max_age)) => {
                    let cookie = Cookie::build(("auth", token))
                        .path("/")
                        .http_only(true)
                        .same_site(SameSite::Lax)
                        .max_age(time::Duration::seconds(max_age as i64))
                        .build();

                    let updated_jar = jar.add(cookie);
                    (updated_jar, success_msg("login successfully")).into_response()
                }
                Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
            }
        }
        _ => error(StatusCode::UNAUTHORIZED, "Authentication failed"),
    }
}

pub async fn change_password(
    State(state): State<AppState>,
    Json(payload): Json<UserChangePassword>,
) -> Response {
    match user::change_password(&state, &payload).await {
        Ok(_) => success_msg("password changed successfully"),
        Err(e) => error(StatusCode::BAD_REQUEST, &e.to_string()),
    }
}

pub async fn change_username(
    State(state): State<AppState>,
    Json(payload): Json<UserChangeUsername>,
) -> Response {
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
    use super::*;
    use crate::db::init_db;
    use axum::http::header;

    #[tokio::test]
    async fn test_login_headers() {
        let rnd: u64 = rand::random();
        let db_path = std::env::temp_dir().join(format!("octopus_login_test_{}.db", rnd));
        let pool = init_db(&db_path).await.unwrap();
        let state = AppState::init(pool).await.unwrap();

        let jar = CookieJar::new();
        let payload = Json(UserLogin {
            username: "admin".to_string(),
            password: "admin".to_string(),
            expire: 3600,
        });

        let resp = login(State(state), jar, payload).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(resp.headers().contains_key(header::CONTENT_TYPE), "Content-Type must be present");
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/json"
        );
        assert!(resp.headers().contains_key(header::SET_COOKIE), "Set-Cookie must be present");
    }
}
