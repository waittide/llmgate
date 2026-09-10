use axum::{
    body::Body,
    http::{HeaderValue, Response, StatusCode, Uri, header},
};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "static/out/"]
struct Asset;

pub async fn serve_static(uri: Uri) -> Response<Body> {
    // 全程以 &str 借用路径：静态资源请求量最大，这里不必为每个请求分配一个 String
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    match Asset::get(path) {
        Some(content) => {
            let mime = guess_mime_type(path);
            let mut response = Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, HeaderValue::from_static(mime));
            if path.starts_with("assets/") {
                response = response.header(
                    header::CACHE_CONTROL,
                    HeaderValue::from_static("public, max-age=31536000, immutable"),
                );
            } else {
                response = response.header(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
            }

            response.body(Body::from(content.data)).unwrap()
        }
        None => {
            // API 路由与静态资源路径不得回退到 SPA 的 index.html
            if uri.path().starts_with("/api/")
                || uri.path().starts_with("/v1/")
                || uri.path().starts_with("/v1beta/")
                || path.starts_with("assets/")
            {
                return Response::builder()
                    .status(StatusCode::NOT_FOUND)
                    .header(header::CONTENT_TYPE, HeaderValue::from_static("application/json"))
                    .body(Body::from(
                        r#"{"error":{"message":"Not Found","type":"invalid_request_error","code":404}}"#,
                    ))
                    .unwrap_or_default();
            }

            // SPA 回退：前端客户端路由一律返回 index.html
            if let Some(index_content) = Asset::get("index.html") {
                Response::builder()
                    .status(StatusCode::OK)
                    .header(
                        header::CONTENT_TYPE,
                        HeaderValue::from_static("text/html; charset=utf-8"),
                    )
                    .header(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"))
                    .body(Body::from(index_content.data))
                    .unwrap()
            } else {
                Response::builder()
                    .status(StatusCode::NOT_FOUND)
                    .body(Body::from("404 Not Found"))
                    .unwrap()
            }
        }
    }
}

fn guess_mime_type(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or("");
    match ext {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "webmanifest" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_serve_static_api_routes_return_404_json() {
        let res = serve_static("/v1/chat/invalid_route".parse().unwrap()).await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
        assert_eq!(res.headers().get(header::CONTENT_TYPE).unwrap(), "application/json");
        let bytes = axum::body::to_bytes(res.into_body(), 1024).await.unwrap();
        assert!(String::from_utf8_lossy(&bytes).contains("Not Found"));
        let res_api = serve_static("/api/v1/unknown".parse().unwrap()).await;
        assert_eq!(res_api.status(), StatusCode::NOT_FOUND);
        assert_eq!(res_api.headers().get(header::CONTENT_TYPE).unwrap(), "application/json");
    }

    #[tokio::test]
    async fn test_serve_static_spa_route_returns_html() {
        let res = serve_static("/dashboard/channels".parse().unwrap()).await;
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/html; charset=utf-8"
        );
    }
}
