use axum::{
    Json,
    extract::{FromRequest, Multipart, Request, State},
    http::{HeaderValue, StatusCode, header},
    response::Response,
};
use bytes::Bytes;

use crate::{
    model::{BackupData, Setting},
    op::{AppState, backup, setting},
    server::resp::{error, success},
};

pub async fn get_setting_list(State(state): State<AppState>) -> Response {
    success(setting::list_settings(&state))
}

pub async fn set_setting(State(state): State<AppState>, Json(payload): Json<Setting>) -> Response {
    match setting::set_setting_string(&state, &payload.key, &payload.value).await {
        Ok(_) => success(payload),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

pub async fn export_db(State(state): State<AppState>) -> Response {
    match backup::export_backup(&state).await {
        Ok(data) => {
            let filename = format!(
                "attachment; filename=\"octopus-export-{}.json\"",
                chrono::Local::now().format("%Y%m%d%H%M%S")
            );

            let json_bytes = match serde_json::to_vec(&data) {
                Ok(b) => b,
                Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
            };

            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::CONTENT_DISPOSITION, HeaderValue::from_str(&filename).unwrap())
                .body(axum::body::Body::from(json_bytes))
                .unwrap()
        }
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

pub async fn import_db(State(state): State<AppState>, req: Request) -> Response {
    let (parts, body) = req.into_parts();
    let content_type = parts
        .headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let json_bytes: Bytes = if content_type.contains("multipart/form-data") {
        let dummy_req = Request::from_parts(parts, body);
        let mut multipart = match Multipart::from_request(dummy_req, &state).await {
            Ok(m) => m,
            Err(e) => return error(StatusCode::BAD_REQUEST, &format!("Invalid multipart: {}", e)),
        };

        let mut file_bytes = None;
        while let Ok(Some(field)) = multipart.next_field().await {
            let name = field.name().unwrap_or("").to_string();
            if name == "file" || field.file_name().is_some() {
                match field.bytes().await {
                    Ok(b) => {
                        file_bytes = Some(b);
                        break;
                    }
                    Err(e) => return error(StatusCode::BAD_REQUEST, &format!("Failed to read upload file: {}", e)),
                }
            }
        }

        match file_bytes {
            Some(b) => b,
            None => return error(StatusCode::BAD_REQUEST, "missing upload file field 'file'"),
        }
    } else {
        match axum::body::to_bytes(body, 50 * 1024 * 1024).await {
            Ok(b) => b,
            Err(e) => return error(StatusCode::BAD_REQUEST, &format!("Failed to read request body: {}", e)),
        }
    };

    let backup_data: BackupData = match serde_json::from_slice(&json_bytes) {
        Ok(data) => data,
        Err(e) => return error(StatusCode::BAD_REQUEST, &format!("Invalid JSON backup file: {}", e)),
    };

    match backup::import_backup(&state, backup_data).await {
        Ok(result) => success(result),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}
