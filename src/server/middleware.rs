use crate::{
    model::APIKey,
    op::{apikey::get_api_key_by_key, stats::get_stats_api_key, AppState},
    server::{auth::verify_jwt, resp::error},
};
use axum::{
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::Next,
    response::Response,
};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct AuthContext {
    pub api_key: APIKey,
    pub supported_models: Vec<String>,
    pub is_anthropic_header: bool,
}

pub async fn admin_auth_middleware(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Response {
    let mut token_opt = None;

    if let Some(auth_header) = req.headers().get(header::AUTHORIZATION)
        && let Ok(auth_str) = auth_header.to_str()
        && let Some(stripped) = auth_str.strip_prefix("Bearer ")
    {
        token_opt = Some(stripped.trim().to_string());
    }

    let token = match token_opt {
        Some(t) if !t.is_empty() => t,
        _ => return error(StatusCode::UNAUTHORIZED, "Authentication failed"),
    };

    let secret = state.jwt_secret.read().clone();
    match verify_jwt(&token, &secret) {
        Ok(claims) => {
            req.extensions_mut().insert(claims);
            next.run(req).await
        }
        Err(_) => error(StatusCode::UNAUTHORIZED, "Authentication failed"),
    }
}

pub async fn api_key_auth_middleware(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Response {
    let mut is_anthropic_header = false;
    let mut api_key_str_opt = None;

    if let Some(x_key) = req.headers().get("x-api-key")
        && let Ok(key) = x_key.to_str()
        && !key.trim().is_empty()
    {
        api_key_str_opt = Some(key.trim().to_string());
        is_anthropic_header = true;
    }

    if api_key_str_opt.is_none()
        && let Some(auth_header) = req.headers().get(header::AUTHORIZATION)
        && let Ok(auth_str) = auth_header.to_str()
        && let Some(stripped) = auth_str.strip_prefix("Bearer ")
        && !stripped.trim().is_empty()
    {
        api_key_str_opt = Some(stripped.trim().to_string());
    }

    let api_key_str = match api_key_str_opt {
        Some(k) if !k.is_empty() => k,
        _ => return error(StatusCode::UNAUTHORIZED, "Authentication failed"),
    };

    let api_key_obj = match get_api_key_by_key(&state, &api_key_str) {
        Ok(k) => k,
        Err(_) => return error(StatusCode::UNAUTHORIZED, "Authentication failed"),
    };

    if !api_key_obj.enabled {
        return error(StatusCode::UNAUTHORIZED, "API key is disabled");
    }

    let now_sec = chrono::Utc::now().timestamp();
    if api_key_obj.expire_at > 0 && api_key_obj.expire_at < now_sec {
        return error(StatusCode::UNAUTHORIZED, "API key has expired");
    }

    let stats = get_stats_api_key(&state, api_key_obj.id);
    let total_cost = stats.metrics.input_cost + stats.metrics.output_cost;
    if api_key_obj.max_cost > 0.0 && total_cost >= api_key_obj.max_cost {
        return error(StatusCode::UNAUTHORIZED, "API key has reached the max cost");
    }

    let supported_models: Vec<String> = if api_key_obj.supported_models.trim().is_empty() {
        Vec::new()
    } else {
        api_key_obj
            .supported_models
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    };

    let auth_context = AuthContext {
        api_key: api_key_obj,
        supported_models,
        is_anthropic_header,
    };

    req.extensions_mut().insert(Arc::new(auth_context));
    next.run(req).await
}
