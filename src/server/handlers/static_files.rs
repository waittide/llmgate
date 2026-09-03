use axum::{
    body::Body,
    http::{HeaderValue, Response, StatusCode, Uri, header},
};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "static/out/"]
struct Asset;

pub async fn serve_static(uri: Uri) -> Response<Body> {
    let mut path = uri.path().trim_start_matches('/').to_string();
    if path.is_empty() {
        path = "index.html".to_string();
    }

    match Asset::get(&path) {
        Some(content) => {
            let mime = mime_guess::from_path(&path).first_or_octet_stream();
            let mut response = Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, HeaderValue::from_str(mime.as_ref()).unwrap());

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
            // SPA Fallback: serve index.html for any frontend client-side route
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
