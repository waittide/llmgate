pub mod auth;
pub mod handlers;
pub mod middleware;
pub mod resp;

use axum::{
    Router,
    extract::DefaultBodyLimit,
    middleware::from_fn_with_state,
    routing::{delete, get, post},
};
use tower_http::{
    compression::CompressionLayer,
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};

use crate::{
    op::AppState,
    server::{
        handlers::{apikey, channel, group, log, model, relay, setting, static_files, stats, user},
        middleware::{admin_auth_middleware, api_key_auth_middleware},
    },
};

pub fn build_router(state: AppState) -> Router {
    // 1. 管理后台 API 路由（受 admin_auth_middleware 保护）
    let admin_router = Router::new()
        // 用户管理
        .route("/user/change-password", post(user::change_password))
        .route("/user/change-username", post(user::change_username))
        .route("/user/status", get(user::status))
        // 渠道管理
        .route("/channel/list", get(channel::list_channel))
        .route("/channel/create", post(channel::create_channel))
        .route("/channel/update", post(channel::update_channel))
        .route("/channel/enable", post(channel::enable_channel))
        .route("/channel/delete/{id}", delete(channel::delete_channel))
        .route("/channel/fetch-model", post(channel::fetch_model))
        .route("/channel/sync", post(channel::sync_channel))
        .route("/channel/last-sync-time", get(channel::get_last_sync_time))
        // 分组管理
        .route("/group/list", get(group::list_groups))
        .route("/group/runtime/stream", get(group::stream_group_runtime))
        .route("/group/create", post(group::create_group))
        .route("/group/update", post(group::update_group))
        .route("/group/active/{id}", post(group::update_group_active_item))
        .route("/group/delete/{id}", delete(group::delete_group))
        // API 密钥管理
        .route("/apikey/list", get(apikey::list_api_keys))
        .route("/apikey/create", post(apikey::create_api_key))
        .route("/apikey/update", post(apikey::update_api_key))
        .route("/apikey/delete/{id}", delete(apikey::delete_api_key))
        // 模型管理
        .route("/model/list", get(model::list_model))
        .route("/model/create", post(model::create_model))
        .route("/model/update", post(model::update_model))
        .route("/model/delete", post(model::delete_model))
        .route("/model/update-price", post(model::update_model_price))
        .route("/model/rebuild-price", post(model::rebuild_model_price))
        .route("/model/last-update-time", get(model::get_last_update_time))
        // 系统设置
        .route("/setting/list", get(setting::get_setting_list))
        .route("/setting/set", post(setting::set_setting))
        .route("/setting/export", get(setting::export_db))
        .route("/setting/import", post(setting::import_db))
        // 统计数据
        .route("/stats/today", get(stats::get_stats_today))
        .route("/stats/daily", get(stats::get_stats_daily))
        .route("/stats/hourly", get(stats::get_stats_hourly))
        .route("/stats/total", get(stats::get_stats_total))
        .route("/stats/apikey", get(stats::get_stats_apikey))
        // 请求日志
        .route("/log/list", get(log::list_logs))
        .route("/log/{id}", get(log::get_log_detail))
        .route("/log/overview/stream", get(log::stream_overview))
        .route("/log/{id}/request-body", get(log::get_request_body))
        .route("/log/{id}/response-body", get(log::get_response_body))
        .route("/log/{id}/{round}/stop", post(log::interrupt_round))
        .route("/log/clear", delete(log::clear_log))
        .layer(from_fn_with_state(state.clone(), admin_auth_middleware));

    // 2. API Key 控制台路由（受 api_key_auth_middleware 保护）
    let apikey_dashboard_router = Router::new()
        .route("/apikey/stats", get(apikey::get_api_key_stats_by_id))
        .route("/apikey/login", get(apikey::login_api_key))
        .layer(from_fn_with_state(state.clone(), api_key_auth_middleware));

    // 3. 公开管理员 API 路由（用户登录）
    let public_api_router = Router::new().route("/user/login", post(user::login));

    // 4. 合并 /api/v1 路由（trace 仅挂管理 API：终端保留后台请求记录；
    //    /v1 中继的请求与错误详情已完整写入请求日志页，不再输出到终端）
    let api_v1 = Router::new()
        .merge(public_api_router)
        .merge(admin_router)
        .merge(apikey_dashboard_router)
        .layer(TraceLayer::new_for_http());

    // 5. 客户端模型代理中继路由 /v1（受 api_key_auth_middleware 保护）
    let client_v1 = Router::new()
        .route("/models", get(model::get_model_list))
        .route("/chat/completions", post(relay::chat_completions))
        .route("/responses", post(relay::responses))
        .route("/messages", post(relay::messages))
        .layer(from_fn_with_state(state.clone(), api_key_auth_middleware));

    // 6. CORS 跨域配置
    let cors = {
        let origins_setting = state
            .settings
            .get("cors_allow_origins")
            .map(|e| e.value().clone())
            .unwrap_or_default();
        let trimmed = origins_setting.trim();
        if !trimmed.is_empty() {
            let origins: Vec<axum::http::HeaderValue> = trimmed
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .filter_map(|s| s.parse().ok())
                .collect();
            if !origins.is_empty() {
                CorsLayer::new()
                    .allow_origin(origins)
                    .allow_methods(Any)
                    .allow_headers(Any)
            } else {
                CorsLayer::new()
                    .allow_origin(Any)
                    .allow_methods(Any)
                    .allow_headers(Any)
            }
        } else {
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any)
        }
    };

    // 7. 根路由，静态前端 SPA 单页文件回退
    Router::new()
        .nest("/api/v1", api_v1)
        .nest("/v1", client_v1)
        .fallback(static_files::serve_static)
        .layer(DefaultBodyLimit::max(64 * 1024 * 1024))
        .layer(cors)
        .layer(CompressionLayer::new())
        .with_state(state)
}
