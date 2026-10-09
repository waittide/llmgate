pub mod error;
pub mod finalizer;
pub mod ir;
pub mod pipeline;
pub mod protocol;
pub mod route;
pub mod sse;
pub mod state;
pub mod upstream;

pub(crate) use pipeline::forward_relay;
pub use sse::{SseBuffer, parse_sse_event};

#[cfg(test)]
mod tests {
    use axum::http::{HeaderMap, Uri};
    use bytes::Bytes;
    use serde_json::json;

    use super::*;
    #[allow(unused_imports)]
    use crate::model::{APIKey, Channel, ChannelModel, ChannelProvider, Group, GroupItem, GroupMode};
    use crate::{
        model::RoundLog,
        op::{self, AppState},
        relay::protocol::{ApiFormat, rewrite_request_body_in_json},
        server::middleware::AuthContext,
        test_support,
    };

    #[test]
    fn round_logs_serialization_and_deserialization() {
        let rounds = vec![
            RoundLog {
                round:            1,
                channel_name:     "openai-primary".to_string(),
                model_name:       "gpt-4o".to_string(),
                request_url:      "https://api.openai.com/v1/chat/completions".to_string(),
                request_headers:  "{\"authorization\":\"Bearer key1\"}".to_string(),
                request_body:     "{\"model\":\"gpt-4o\",\"messages\":[]}".to_string(),
                response_url:     "https://api.openai.com/v1/chat/completions".to_string(),
                response_headers: "{\"content-type\":\"application/json\"}".to_string(),
                response_body:    "{\"error\":{\"message\":\"Rate limit reached\"}}".to_string(),
                status:           429,
                duration_ms:      120,
                error:            Some("{\"error\":{\"message\":\"Rate limit reached\"}}".to_string()),
                key_index:        Some(0),
            },
            RoundLog {
                round:            2,
                channel_name:     "openai-backup".to_string(),
                model_name:       "gpt-4o".to_string(),
                request_url:      "https://api.openai-backup.com/v1/chat/completions".to_string(),
                request_headers:  "{\"authorization\":\"Bearer key2\"}".to_string(),
                request_body:     "{\"model\":\"gpt-4o\",\"messages\":[]}".to_string(),
                response_url:     "https://api.openai-backup.com/v1/chat/completions".to_string(),
                response_headers: "{\"content-type\":\"application/json\"}".to_string(),
                response_body:    "{\"choices\":[{\"message\":{\"content\":\"Hello\"}}]}".to_string(),
                status:           200,
                duration_ms:      350,
                error:            None,
                key_index:        Some(0),
            },
        ];

        let json_str = serde_json::to_string(&rounds).expect("序列化轮次日志失败");
        let parsed: Vec<RoundLog> = serde_json::from_str(&json_str).expect("反序列化轮次日志失败");

        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].round, 1);
        assert_eq!(parsed[0].channel_name, "openai-primary");
        assert_eq!(parsed[0].status, 429);
        assert!(parsed[0].error.is_some());

        assert_eq!(parsed[1].round, 2);
        assert_eq!(parsed[1].channel_name, "openai-backup");
        assert_eq!(parsed[1].status, 200);
        assert!(parsed[1].error.is_none());
    }

    #[test]
    fn test_param_override_applied_to_request_body() {
        let original = br#"{"model":"gpt-4o","temperature":0.7,"messages":[]}"#;
        let override_json = r#"{"temperature":0.2,"max_tokens":1024}"#;
        let rewritten = rewrite_request_body_in_json(original, "gpt-4o-custom", Some(override_json));
        let val: serde_json::Value = serde_json::from_slice(&rewritten).unwrap();
        assert_eq!(val.get("model").unwrap().as_str().unwrap(), "gpt-4o-custom");
        assert_eq!(val.get("temperature").unwrap().as_f64().unwrap(), 0.2);
        assert_eq!(val.get("max_tokens").unwrap().as_i64().unwrap(), 1024);
    }

    // ---------- 故障转移集成测试（wiremock 假上游 + 临时 DB） ----------

    fn test_auth() -> std::sync::Arc<AuthContext> {
        std::sync::Arc::new(AuthContext {
            api_key:             crate::model::APIKey {
                id:               1,
                name:             "test".into(),
                api_key:          "sk-test".into(),
                enabled:          true,
                expire_at:        0,
                max_cost:         0.0,
                supported_models: String::new(),
            },
            is_anthropic_header: false,
        })
    }

    async fn chat_body(model: &str) -> Bytes {
        Bytes::from(
            json!({
                "model": model,
                "messages": [{ "role": "user", "content": "hi" }]
            })
            .to_string(),
        )
    }

    const CHAT_OK_BODY: &str = r#"{"id":"x","choices":[{"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;
    /// 捕获上游实际收到的请求体，用于断言"记录开关不影响转发"。
    struct CaptureBody(std::sync::Arc<parking_lot::Mutex<Vec<u8>>>);

    impl wiremock::Respond for CaptureBody {
        fn respond(&self, request: &wiremock::Request) -> wiremock::ResponseTemplate {
            *self.0.lock() = request.body.clone();
            wiremock::ResponseTemplate::new(200).set_body_string(CHAT_OK_BODY)
        }
    }

    async fn wait_finished_log(state: &AppState, id: i64) -> crate::model::RequestLog {
        for _ in 0..100 {
            if let Some(row) = op::log::get_request_log(state, id).await.unwrap()
                && row.status != crate::model::request_status::RUNNING
            {
                return row;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("日志未在期限内完成回写");
    }

    /// 记录开关关闭时：request_body / response_body / rounds_detail 中的报文体一律落库为空，
    /// 但转发给上游的载荷保持完整（开关只影响日志，不影响转发）。
    #[tokio::test]
    async fn body_recording_switches_off_leave_bodies_empty() {
        let (state, db_path) = test_support::setup_test_state("relay_body_off").await;
        op::setting::set_setting_string(&state, crate::op::log::LOG_REQUEST_BODY_ENABLED_KEY, "0")
            .await
            .unwrap();
        op::setting::set_setting_string(&state, crate::op::log::LOG_RESPONSE_BODY_ENABLED_KEY, "0")
            .await
            .unwrap();

        let server = wiremock::MockServer::start().await;
        let captured: std::sync::Arc<parking_lot::Mutex<Vec<u8>>> = Default::default();
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(CaptureBody(captured.clone()))
            .expect(1)
            .mount(&server)
            .await;

        let model = add_channel(&state, "ch", &server.uri(), "k1", "m-bodyoff").await;
        let mut group = crate::model::Group {
            name: "m-bodyoff".into(),
            mode: crate::model::GroupMode::Failover,
            items: vec![crate::model::GroupItem {
                channel_model_id: model.id,
                ..Default::default()
            }],
            ..Default::default()
        };
        op::group::create_group(&state, &mut group).await.unwrap();

        let content = "x".repeat(200 * 1024);
        let req_body = json!({
            "model": "m-bodyoff",
            "messages": [{ "role": "user", "content": content }]
        });
        let resp = forward_relay(
            state.clone(),
            test_auth(),
            ApiFormat::OpenaiCompletion,
            None,
            Uri::from_static("/v1/chat/completions"),
            HeaderMap::new(),
            Bytes::from(req_body.to_string()),
        )
        .await;
        assert_eq!(resp.status(), 200);
        server.verify().await;

        // 上游必须收到完整未截断的载荷
        let upstream: serde_json::Value = serde_json::from_slice(&captured.lock()).unwrap();
        assert_eq!(upstream["messages"][0]["content"].as_str().unwrap().len(), 200 * 1024);

        let log = wait_finished_log(&state, 1).await;
        assert_eq!(log.request_body, "");
        assert_eq!(log.response_body, "");
        let rounds: Vec<crate::model::RoundLog> = serde_json::from_str(&log.rounds_detail).unwrap();
        assert_eq!(rounds.len(), 1);
        assert_eq!(rounds[0].request_body, "");
        assert_eq!(rounds[0].response_body, "");
        let _ = std::fs::remove_file(db_path);
    }

    /// 记录开关默认开启：报文体必须完整落库，不做任何截断。
    #[tokio::test]
    async fn body_recording_defaults_store_full_bodies() {
        let (state, db_path) = test_support::setup_test_state("relay_body_on").await;
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(CHAT_OK_BODY))
            .mount(&server)
            .await;

        let model = add_channel(&state, "ch", &server.uri(), "k1", "m-bodyon").await;
        let mut group = crate::model::Group {
            name: "m-bodyon".into(),
            mode: crate::model::GroupMode::Failover,
            items: vec![crate::model::GroupItem {
                channel_model_id: model.id,
                ..Default::default()
            }],
            ..Default::default()
        };
        op::group::create_group(&state, &mut group).await.unwrap();

        let content = "y".repeat(200 * 1024);
        let req_body = json!({
            "model": "m-bodyon",
            "messages": [{ "role": "user", "content": content.clone() }]
        });
        let raw = req_body.to_string();
        let resp = forward_relay(
            state.clone(),
            test_auth(),
            ApiFormat::OpenaiCompletion,
            None,
            Uri::from_static("/v1/chat/completions"),
            HeaderMap::new(),
            Bytes::from(raw.clone()),
        )
        .await;
        assert_eq!(resp.status(), 200);

        let log = wait_finished_log(&state, 1).await;
        assert_eq!(log.request_body, raw);
        let rounds: Vec<crate::model::RoundLog> = serde_json::from_str(&log.rounds_detail).unwrap();
        assert!(rounds[0].request_body.contains(&content));
        assert!(rounds[0].request_body.len() >= content.len());
        let _ = std::fs::remove_file(db_path);
    }

    /// 上游收到的 (x-opencode-session, x-unlisted-header, user-agent) 列表。
    type CapturedHeaders = std::sync::Arc<parking_lot::Mutex<Vec<(Option<String>, Option<String>, Option<String>)>>>;

    /// 记录上游实际收到的候选头，用于断言透传名单的边界。
    struct CapturePassthrough(CapturedHeaders);

    impl wiremock::Respond for CapturePassthrough {
        fn respond(&self, request: &wiremock::Request) -> wiremock::ResponseTemplate {
            let read = |name: &str| {
                request
                    .headers
                    .get(name)
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_string)
            };
            self.0.lock().push((
                read("x-opencode-session"),
                read("x-unlisted-header"),
                read("user-agent"),
            ));
            wiremock::ResponseTemplate::new(200).set_body_string(CHAT_OK_BODY)
        }
    }

    /// 渠道透传名单：未开启时客户端头全部丢弃；开启后只放行名单内的头且值原样上行。
    #[tokio::test]
    async fn channel_passthrough_forwards_only_listed_client_headers() {
        let (state, db_path) = test_support::setup_test_state("relay_passthrough").await;
        let server = wiremock::MockServer::start().await;
        let seen: CapturedHeaders = Default::default();
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(CapturePassthrough(seen.clone()))
            .mount(&server)
            .await;

        let model = add_channel(&state, "ch", &server.uri(), "k1", "m-passthrough").await;
        let mut group = crate::model::Group {
            name: "m-passthrough".into(),
            mode: crate::model::GroupMode::Failover,
            items: vec![crate::model::GroupItem {
                channel_model_id: model.id,
                ..Default::default()
            }],
            ..Default::default()
        };
        op::group::create_group(&state, &mut group).await.unwrap();

        let body = json!({
            "model": "m-passthrough",
            "messages": [{ "role": "user", "content": "hi" }]
        });

        let mut client_headers = HeaderMap::new();
        client_headers.insert("x-opencode-session", "client-session-abc".parse().unwrap());
        client_headers.insert("x-unlisted-header", "should-be-dropped".parse().unwrap());
        client_headers.insert("user-agent", "omp/18.8.6".parse().unwrap());

        let send = |headers: HeaderMap| {
            let state = state.clone();
            let body = body.clone();
            async move {
                forward_relay(
                    state,
                    test_auth(),
                    ApiFormat::OpenaiCompletion,
                    None,
                    Uri::from_static("/v1/chat/completions"),
                    headers,
                    Bytes::from(body.to_string()),
                )
                .await
                .status()
            }
        };

        // 未开启透传：仅内置白名单头（user-agent）上行
        assert_eq!(send(client_headers.clone()).await, 200);

        // 开启透传：只放行名单内的头
        let req: crate::model::ChannelUpdateRequest = serde_json::from_value(json!({
            "id": model.channel_id,
            "passthrough_header": ["X-Opencode-Session"],
        }))
        .unwrap();
        op::channel::update_channel(&state, &req).await.unwrap();
        assert_eq!(send(client_headers).await, 200);

        server.verify().await;
        let seen = seen.lock().clone();
        assert_eq!(seen.len(), 2);
        assert_eq!(
            seen[0],
            (None, None, Some("omp/18.8.6".to_string())),
            "未开启透传时只有内置白名单头可上行"
        );
        assert_eq!(
            seen[1].0.as_deref(),
            Some("client-session-abc"),
            "名单内客户端头必须原样上行"
        );
        assert_eq!(seen[1].1, None, "名单外客户端头必须丢弃");
        assert_eq!(seen[1].2.as_deref(), Some("omp/18.8.6"), "内置白名单头始终上行");
        let _ = std::fs::remove_file(db_path);
    }

    async fn add_channel(
        state: &AppState,
        name: &str,
        base_url: &str,
        key: &str,
        model_name: &str,
    ) -> crate::model::ChannelModel {
        let mut ch = crate::model::Channel {
            name: name.into(),
            channel_type: crate::model::ChannelProvider::OpenaiCompletion,
            enabled: true,
            base_url: base_url.into(),
            key: key.into(),
            models: vec![crate::model::ChannelModel {
                name: model_name.into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        op::channel::create_channel(state, &mut ch).await.unwrap();
        ch.models[0].clone()
    }

    #[tokio::test]
    async fn key_rotation_on_401_uses_untried_key() {
        let (state, db_path) = test_support::setup_test_state("failover_key").await;
        let server = wiremock::MockServer::start().await;

        wiremock::Mock::given(wiremock::matchers::header("authorization", "Bearer k1"))
            .respond_with(wiremock::ResponseTemplate::new(401).set_body_string("bad key"))
            .up_to_n_times(1)
            .expect(1)
            .mount(&server)
            .await;
        wiremock::Mock::given(wiremock::matchers::header("authorization", "Bearer k2"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(CHAT_OK_BODY))
            .up_to_n_times(1)
            .expect(1)
            .mount(&server)
            .await;

        let model = add_channel(&state, "ch", &server.uri(), "k1\nk2", "m-keyrot").await;
        let mut group = crate::model::Group {
            name: "m-keyrot".into(),
            mode: crate::model::GroupMode::Failover,
            items: vec![crate::model::GroupItem {
                channel_model_id: model.id,
                ..Default::default()
            }],
            ..Default::default()
        };
        op::group::create_group(&state, &mut group).await.unwrap();

        let resp = forward_relay(
            state.clone(),
            test_auth(),
            ApiFormat::OpenaiCompletion,
            None,
            Uri::from_static("/v1/chat/completions"),
            HeaderMap::new(),
            chat_body("m-keyrot").await,
        )
        .await;
        assert_eq!(resp.status(), 200);
        server.verify().await;
        let _ = std::fs::remove_file(db_path);
    }

    /// 上游 200 + 错误信封、且需要跨格式转换时，必须按网关失败返回而不是
    /// 合成一个空应答（回归：parse_* 对缺载荷的响应不报错，误差被静默吞掉）。
    #[tokio::test]
    async fn non_stream_200_error_envelope_is_not_swallowed() {
        let (state, db_path) = test_support::setup_test_state("relay_200_error").await;
        let upstream = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::any())
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(
                r#"{"type":"error","error":{"type":"invalid_request_error","message":"input too long"}}"#,
            ))
            .mount(&upstream)
            .await;

        // 上游是 Anthropic、客户端发的是 Chat：跨格式路径才会走到 parse/emit
        let mut ch = crate::model::Channel {
            name: "ch200".into(),
            channel_type: crate::model::ChannelProvider::Anthropic,
            enabled: true,
            base_url: upstream.uri(),
            key: "k1".into(),
            models: vec![crate::model::ChannelModel {
                name: "m-200err".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        op::channel::create_channel(&state, &mut ch).await.unwrap();
        let mut group = crate::model::Group {
            name: "m-200err".into(),
            mode: crate::model::GroupMode::Failover,
            items: vec![crate::model::GroupItem {
                channel_model_id: ch.models[0].id,
                ..Default::default()
            }],
            ..Default::default()
        };
        op::group::create_group(&state, &mut group).await.unwrap();

        let resp = forward_relay(
            state.clone(),
            test_auth(),
            ApiFormat::OpenaiCompletion,
            None,
            Uri::from_static("/v1/chat/completions"),
            HeaderMap::new(),
            chat_body("m-200err").await,
        )
        .await;
        assert!(!resp.status().is_success(), "不得以 2xx 返回上游错误");
        let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024)
            .await
            .unwrap();
        let body = String::from_utf8_lossy(&bytes);
        assert!(body.contains("input too long"), "错误详情必须透出: {body}");
        assert_eq!(wait_finished_log(&state, 1).await.status, "failed");

        let _ = std::fs::remove_file(db_path);
    }

    /// OpenAI Responses 的成功终态对象带 `"error": null`。流式请求遇到这种
    /// 非 SSE 完整应答时必须按完整应答返回，不能因「存在 error 字段」判成错误。
    #[tokio::test]
    async fn responses_success_with_null_error_is_not_treated_as_failure() {
        let (state, db_path) = test_support::setup_test_state("relay_error_null").await;
        let upstream = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::any())
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_raw(
                r#"{"id":"resp_1","object":"response","status":"completed","model":"m","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"ok"}]}],"error":null,"incomplete_details":null,"usage":{"input_tokens":2,"output_tokens":3,"total_tokens":5}}"#,
                "application/json",
            ))
            .mount(&upstream)
            .await;

        let mut ch = crate::model::Channel {
            name: "ch-respnull".into(),
            channel_type: crate::model::ChannelProvider::OpenaiResponses,
            enabled: true,
            base_url: upstream.uri(),
            key: "k1".into(),
            models: vec![crate::model::ChannelModel {
                name: "m-respnull".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        op::channel::create_channel(&state, &mut ch).await.unwrap();
        let mut group = crate::model::Group {
            name: "m-respnull".into(),
            mode: crate::model::GroupMode::Failover,
            items: vec![crate::model::GroupItem {
                channel_model_id: ch.models[0].id,
                ..Default::default()
            }],
            ..Default::default()
        };
        op::group::create_group(&state, &mut group).await.unwrap();

        let resp = forward_relay(
            state.clone(),
            test_auth(),
            ApiFormat::OpenaiResponse,
            None,
            Uri::from_static("/v1/responses"),
            HeaderMap::new(),
            Bytes::from(
                json!({
                    "model": "m-respnull",
                    "stream": true,
                    "input": [{ "type": "message", "role": "user",
                                "content": [{ "type": "input_text", "text": "hi" }] }]
                })
                .to_string(),
            ),
        )
        .await;

        assert_eq!(resp.status(), 200, "带 error:null 的成功应答不得判为失败");
        let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024)
            .await
            .unwrap();
        assert!(String::from_utf8_lossy(&bytes).contains("\"text\":\"ok\""));
        let log = wait_finished_log(&state, 1).await;
        assert_eq!(log.status, crate::model::request_status::SUCCESS);
        assert_eq!(log.completion_tokens, 3);

        let _ = std::fs::remove_file(db_path);
    }

    /// 非流式请求遇到 200 + 非 JSON 体（HTML 错误页、纯文本）时，不得当作正常应答
    /// 透传：四种协议的成功应答都是 JSON，非 JSON 只可能是上游把错误塞进了 200。
    #[tokio::test]
    async fn non_stream_non_json_success_body_fails_loudly() {
        let (state, db_path) = test_support::setup_test_state("relay_nonjson").await;
        let upstream = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::any())
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_string("<html><body>502 Bad Gateway</body></html>"),
            )
            .mount(&upstream)
            .await;

        let model = add_channel(&state, "ch-nonjson", &upstream.uri(), "k1", "m-nonjson").await;
        let mut group = crate::model::Group {
            name: "m-nonjson".into(),
            mode: crate::model::GroupMode::Failover,
            items: vec![crate::model::GroupItem {
                channel_model_id: model.id,
                ..Default::default()
            }],
            ..Default::default()
        };
        op::group::create_group(&state, &mut group).await.unwrap();

        // 同格式（Chat 上游、Chat 客户端）：旧行为会把 HTML 原样当 200 成功透传
        let resp = forward_relay(
            state.clone(),
            test_auth(),
            ApiFormat::OpenaiCompletion,
            None,
            Uri::from_static("/v1/chat/completions"),
            HeaderMap::new(),
            chat_body("m-nonjson").await,
        )
        .await;
        assert!(!resp.status().is_success(), "非 JSON 的 200 体不得当成功返回");
        let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024)
            .await
            .unwrap();
        let body = String::from_utf8_lossy(&bytes);
        assert!(body.contains("Bad Gateway"), "上游原文应作为错误线索透出: {body}");

        let log = wait_finished_log(&state, 1).await;
        assert_eq!(log.status, crate::model::request_status::FAILED);

        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn failover_to_next_channel_on_500() {
        let (state, db_path) = test_support::setup_test_state("failover_500").await;
        let down = wiremock::MockServer::start().await;
        let up = wiremock::MockServer::start().await;

        wiremock::Mock::given(wiremock::matchers::any())
            .respond_with(wiremock::ResponseTemplate::new(500).set_body_string("boom"))
            .up_to_n_times(1)
            .expect(1)
            .mount(&down)
            .await;
        wiremock::Mock::given(wiremock::matchers::any())
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(CHAT_OK_BODY))
            .expect(1)
            .mount(&up)
            .await;

        let model_a = add_channel(&state, "a", &down.uri(), "k", "m-fail").await;
        let model_b = add_channel(&state, "b", &up.uri(), "k", "m-fail").await;
        let mut group = crate::model::Group {
            name: "m-fail".into(),
            mode: crate::model::GroupMode::Failover,
            items: vec![
                crate::model::GroupItem {
                    channel_model_id: model_a.id,
                    ..Default::default()
                },
                crate::model::GroupItem {
                    channel_model_id: model_b.id,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        op::group::create_group(&state, &mut group).await.unwrap();

        let resp = forward_relay(
            state.clone(),
            test_auth(),
            ApiFormat::OpenaiCompletion,
            None,
            Uri::from_static("/v1/chat/completions"),
            HeaderMap::new(),
            chat_body("m-fail").await,
        )
        .await;
        assert_eq!(resp.status(), 200);
        down.verify().await;
        up.verify().await;
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn client_error_stops_failover() {
        let (state, db_path) = test_support::setup_test_state("failover_400").await;
        let bad = wiremock::MockServer::start().await;
        let healthy = wiremock::MockServer::start().await;

        wiremock::Mock::given(wiremock::matchers::any())
            .respond_with(wiremock::ResponseTemplate::new(400).set_body_string("bad request"))
            .up_to_n_times(1)
            .expect(1)
            .mount(&bad)
            .await;
        // 健康渠道不应被触达：400 属于请求错误，继续重试只会烧钱
        wiremock::Mock::given(wiremock::matchers::any())
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(CHAT_OK_BODY))
            .expect(0)
            .mount(&healthy)
            .await;

        let model_a = add_channel(&state, "a", &bad.uri(), "k", "m-400").await;
        let model_b = add_channel(&state, "b", &healthy.uri(), "k", "m-400").await;
        let mut group = crate::model::Group {
            name: "m-400".into(),
            mode: crate::model::GroupMode::Failover,
            items: vec![
                crate::model::GroupItem {
                    channel_model_id: model_a.id,
                    ..Default::default()
                },
                crate::model::GroupItem {
                    channel_model_id: model_b.id,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        op::group::create_group(&state, &mut group).await.unwrap();

        let resp = forward_relay(
            state.clone(),
            test_auth(),
            ApiFormat::OpenaiCompletion,
            None,
            Uri::from_static("/v1/chat/completions"),
            HeaderMap::new(),
            chat_body("m-400").await,
        )
        .await;
        assert_eq!(resp.status(), 400);
        bad.verify().await;
        healthy.verify().await;
        let _ = std::fs::remove_file(db_path);
    }

    /// 转发以失败告终时，最终日志必须停在 failed。
    ///
    /// 回归：兜底注销的 RelayGuard 在失败路径上未被 defuse，其 Drop 会在同一个
    /// request_id 上再写一条 INTERRUPTED，覆盖正常收尾写下的失败终态（两条异步
    /// 落盘的先后不定，存档状态随机器而异）。
    #[tokio::test]
    async fn failed_relay_log_is_not_overwritten_by_the_guard() {
        let (state, db_path) = test_support::setup_test_state("relay_failed_status").await;
        let upstream = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::any())
            .respond_with(wiremock::ResponseTemplate::new(400).set_body_string("bad request"))
            .mount(&upstream)
            .await;

        let model = add_channel(&state, "bad", &upstream.uri(), "k", "m-status").await;
        let mut group = crate::model::Group {
            name: "m-status".into(),
            mode: crate::model::GroupMode::Failover,
            items: vec![crate::model::GroupItem {
                channel_model_id: model.id,
                ..Default::default()
            }],
            ..Default::default()
        };
        op::group::create_group(&state, &mut group).await.unwrap();

        let resp = forward_relay(
            state.clone(),
            test_auth(),
            ApiFormat::OpenaiCompletion,
            None,
            Uri::from_static("/v1/chat/completions"),
            HeaderMap::new(),
            chat_body("m-status").await,
        )
        .await;
        assert_eq!(resp.status(), 400);

        let log_id = 1;
        assert_eq!(
            wait_finished_log(&state, log_id).await.status,
            crate::model::request_status::FAILED
        );
        // 兜底注销写在另一个任务里：留出落地时间，再确认终态没有被改写
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        let settled = op::log::get_request_log(&state, log_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(settled.status, crate::model::request_status::FAILED);

        let _ = std::fs::remove_file(db_path);
    }

    /// Anthropic 客户端打到 OpenAI 兼容上游：上游的错误体不得原样透传，
    /// 必须改写为 Anthropic 错误信封（否则 SDK 只能报“未知错误”）。
    #[tokio::test]
    async fn anthropic_client_gets_anthropic_error_envelope() {
        let (state, db_path) = test_support::setup_test_state("relay_err_env").await;
        let upstream = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::any())
            .respond_with(wiremock::ResponseTemplate::new(400).set_body_string(
                r#"{"error":{"message":"model not found","type":"invalid_request_error","code":"model_not_found"}}"#,
            ))
            .mount(&upstream)
            .await;

        let model = add_channel(&state, "ch", &upstream.uri(), "k1", "m-env").await;
        let mut group = crate::model::Group {
            name: "m-env".into(),
            mode: crate::model::GroupMode::Failover,
            items: vec![crate::model::GroupItem {
                channel_model_id: model.id,
                ..Default::default()
            }],
            ..Default::default()
        };
        op::group::create_group(&state, &mut group).await.unwrap();

        let resp = forward_relay(
            state.clone(),
            test_auth(),
            ApiFormat::AnthropicMessage,
            None,
            Uri::from_static("/v1/messages"),
            HeaderMap::new(),
            Bytes::from(
                json!({
                    "model": "m-env",
                    "max_tokens": 16,
                    "messages": [{ "role": "user", "content": "hi" }]
                })
                .to_string(),
            ),
        )
        .await;
        assert_eq!(resp.status(), 400);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value.get("type").unwrap(), "error");
        assert_eq!(value.pointer("/error/type").unwrap(), "invalid_request_error");
        assert_eq!(value.pointer("/error/message").unwrap(), "model not found");
        assert!(value.pointer("/error/code").is_none());
        let _ = std::fs::remove_file(db_path);
    }

    /// 流式请求 + 上游 200 却是 JSON 错误体：不得产出“正常但为空”的流，
    /// 必须返回 502 + 客户端协议错误信封（回归：上游错误曾被静默吞掉）。
    #[tokio::test]
    async fn stream_request_with_non_sse_error_body_fails_loudly() {
        let (state, db_path) = test_support::setup_test_state("relay_nonsse_err").await;
        let upstream = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::any())
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_raw(
                r#"{"error":{"message":"upstream boom","type":"server_error"}}"#,
                "application/json",
            ))
            .mount(&upstream)
            .await;

        let model = add_channel(&state, "ch", &upstream.uri(), "k1", "m-nonsse").await;
        let mut group = crate::model::Group {
            name: "m-nonsse".into(),
            mode: crate::model::GroupMode::Failover,
            items: vec![crate::model::GroupItem {
                channel_model_id: model.id,
                ..Default::default()
            }],
            ..Default::default()
        };
        op::group::create_group(&state, &mut group).await.unwrap();

        let resp = forward_relay(
            state.clone(),
            test_auth(),
            ApiFormat::AnthropicMessage,
            None,
            Uri::from_static("/v1/messages"),
            HeaderMap::new(),
            Bytes::from(
                json!({
                    "model": "m-nonsse",
                    "max_tokens": 16,
                    "stream": true,
                    "messages": [{ "role": "user", "content": "hi" }]
                })
                .to_string(),
            ),
        )
        .await;

        assert_eq!(resp.status(), 502);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value.get("type").unwrap(), "error");
        assert_eq!(value.pointer("/error/message").unwrap(), "upstream boom");
        let _ = std::fs::remove_file(db_path);
    }

    /// 上游忽略 stream 参数直接返回完整应答（200 + JSON）：按完整响应转换后返回，
    /// 客户端仍拿到可解析的结果，而不是空流。
    #[tokio::test]
    async fn stream_request_with_json_completion_returns_completion() {
        let (state, db_path) = test_support::setup_test_state("relay_nonsse_ok").await;
        let upstream = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::any())
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_raw(
                r#"{"id":"x","choices":[{"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#,
                "application/json",
            ))
            .mount(&upstream)
            .await;

        let model = add_channel(&state, "ch", &upstream.uri(), "k1", "m-json").await;
        let mut group = crate::model::Group {
            name: "m-json".into(),
            mode: crate::model::GroupMode::Failover,
            items: vec![crate::model::GroupItem {
                channel_model_id: model.id,
                ..Default::default()
            }],
            ..Default::default()
        };
        op::group::create_group(&state, &mut group).await.unwrap();

        let resp = forward_relay(
            state.clone(),
            test_auth(),
            ApiFormat::AnthropicMessage,
            None,
            Uri::from_static("/v1/messages"),
            HeaderMap::new(),
            Bytes::from(
                json!({
                    "model": "m-json",
                    "max_tokens": 16,
                    "stream": true,
                    "messages": [{ "role": "user", "content": "hi" }]
                })
                .to_string(),
            ),
        )
        .await;

        assert_eq!(resp.status(), 200);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value.get("type").unwrap(), "message");
        assert_eq!(value.pointer("/content/0/text").unwrap(), "ok");
        let _ = std::fs::remove_file(db_path);
    }

    /// 跨格式流式端到端：Anthropic 客户端 ← OpenAI 兼容上游 SSE。
    /// 覆盖 SSE 分流（content-type 判定不能被误伤）、翻译泵与用量提取。
    #[tokio::test]
    async fn anthropic_client_streams_from_chat_upstream() {
        let (state, db_path) = test_support::setup_test_state("relay_stream_x").await;
        let upstream = wiremock::MockServer::start().await;
        let sse = concat!(
            "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\",\"content\":\"ok\"},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":1,\"total_tokens\":4}}\n\n",
            "data: [DONE]\n\n",
        );
        wiremock::Mock::given(wiremock::matchers::any())
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_raw(sse, "text/event-stream"))
            .mount(&upstream)
            .await;

        let model = add_channel(&state, "ch", &upstream.uri(), "k1", "m-stream").await;
        let mut group = crate::model::Group {
            name: "m-stream".into(),
            mode: crate::model::GroupMode::Failover,
            items: vec![crate::model::GroupItem {
                channel_model_id: model.id,
                ..Default::default()
            }],
            ..Default::default()
        };
        op::group::create_group(&state, &mut group).await.unwrap();

        let resp = forward_relay(
            state.clone(),
            test_auth(),
            ApiFormat::AnthropicMessage,
            None,
            Uri::from_static("/v1/messages"),
            HeaderMap::new(),
            Bytes::from(
                json!({
                    "model": "m-stream",
                    "max_tokens": 16,
                    "stream": true,
                    "messages": [{ "role": "user", "content": "hi" }]
                })
                .to_string(),
            ),
        )
        .await;

        assert_eq!(resp.status(), 200);
        assert!(
            resp.headers()
                .get("content-type")
                .is_some_and(|v| v.to_str().unwrap().contains("text/event-stream"))
        );
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let body = String::from_utf8_lossy(&body);
        assert!(body.contains("event: message_start"));
        assert!(body.contains("text_delta"));
        assert!(body.contains("\"text\":\"ok\""));
        assert!(body.contains("event: message_stop"));

        let log = wait_finished_log(&state, 1).await;
        assert_eq!(log.total_tokens, 4);
        let _ = std::fs::remove_file(db_path);
    }

    /// 同格式非流式响应必须原样透传上游字节：重新序列化会按 key 排序重排、
    /// 改变数值书写形式，对大响应还多花一次序列化。
    #[tokio::test]
    async fn same_format_json_response_is_byte_identical() {
        let (state, db_path) = test_support::setup_test_state("relay_same_fmt").await;
        // key 顺序刻意非字典序，且带一个浮点值：回炉序列化会被重排
        let upstream_body = r#"{"zzz":1,"choices":[{"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2,"cost":1.5}}"#;
        let upstream = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::any())
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(upstream_body))
            .mount(&upstream)
            .await;

        let model = add_channel(&state, "ch", &upstream.uri(), "k1", "m-same").await;
        let mut group = crate::model::Group {
            name: "m-same".into(),
            mode: crate::model::GroupMode::Failover,
            items: vec![crate::model::GroupItem {
                channel_model_id: model.id,
                ..Default::default()
            }],
            ..Default::default()
        };
        op::group::create_group(&state, &mut group).await.unwrap();

        let resp = forward_relay(
            state.clone(),
            test_auth(),
            ApiFormat::OpenaiCompletion,
            None,
            Uri::from_static("/v1/chat/completions"),
            HeaderMap::new(),
            chat_body("m-same").await,
        )
        .await;
        assert_eq!(resp.status(), 200);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(body.as_ref(), upstream_body.as_bytes());
        let _ = std::fs::remove_file(db_path);
    }

    /// 端到端：上游 200 + SSE 中途报错。客户端必须收到协议错误事件且连接终止，
    /// 日志必须记为 stream_error——不能被当成一次成功的空回复计费。
    #[tokio::test]
    async fn stream_error_mid_flight_aborts_and_logs_stream_error() {
        use futures::StreamExt as _;

        let (state, db_path) = test_support::setup_test_state("relay_stream_err").await;
        let upstream = wiremock::MockServer::start().await;
        let sse = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n",
            "data: {\"error\":{\"message\":\"upstream exploded\",\"type\":\"server_error\"}}\n\n",
        );
        wiremock::Mock::given(wiremock::matchers::any())
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_raw(sse, "text/event-stream"))
            .mount(&upstream)
            .await;

        let model = add_channel(&state, "ch", &upstream.uri(), "k1", "m-ser").await;
        let mut group = crate::model::Group {
            name: "m-ser".into(),
            mode: crate::model::GroupMode::Failover,
            items: vec![crate::model::GroupItem {
                channel_model_id: model.id,
                ..Default::default()
            }],
            ..Default::default()
        };
        op::group::create_group(&state, &mut group).await.unwrap();

        let resp = forward_relay(
            state.clone(),
            test_auth(),
            ApiFormat::AnthropicMessage,
            None,
            Uri::from_static("/v1/messages"),
            HeaderMap::new(),
            Bytes::from(
                json!({
                    "model": "m-ser",
                    "max_tokens": 16,
                    "stream": true,
                    "messages": [{ "role": "user", "content": "hi" }]
                })
                .to_string(),
            ),
        )
        .await;

        let mut stream = resp.into_body().into_data_stream();
        let mut received = Vec::new();
        let mut aborted = false;
        while let Some(item) = stream.next().await {
            match item {
                Ok(bytes) => received.extend_from_slice(&bytes),
                Err(_) => {
                    aborted = true;
                    break;
                }
            }
        }
        let received = String::from_utf8_lossy(&received);
        assert!(received.contains("partial"), "{received}");
        assert!(received.contains("event: error"), "{received}");
        assert!(received.contains("upstream exploded"), "{received}");
        assert!(!received.contains("message_stop"), "错误流不得补正常结束: {received}");
        assert!(aborted, "流内错误必须终止传输");

        let log = wait_finished_log(&state, 1).await;
        assert_eq!(log.status, crate::model::request_status::STREAM_ERROR);
        assert!(log.error.unwrap_or_default().contains("upstream exploded"));
        let _ = std::fs::remove_file(db_path);
    }

    /// key 轮换不得依赖冷却窗口：cooldown_seconds = 0（等于不冷却）时，
    /// pick 每次都会返回 index 0，旧逻辑据此判定「该渠道 key 已试完」而放弃渠道，
    /// 导致第二个 key 永远用不上。
    #[tokio::test]
    async fn key_rotation_works_with_zero_cooldown() {
        let (state, db_path) = test_support::setup_test_state("relay_keyrot0").await;
        let server = wiremock::MockServer::start().await;

        wiremock::Mock::given(wiremock::matchers::header("authorization", "Bearer k1"))
            .respond_with(wiremock::ResponseTemplate::new(401).set_body_string("bad key"))
            .up_to_n_times(1)
            .expect(1)
            .mount(&server)
            .await;
        wiremock::Mock::given(wiremock::matchers::header("authorization", "Bearer k2"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(CHAT_OK_BODY))
            .up_to_n_times(1)
            .expect(1)
            .mount(&server)
            .await;

        let model = add_channel(&state, "ch", &server.uri(), "k1\nk2", "m-keyrot0").await;
        let mut group = crate::model::Group {
            name: "m-keyrot0".into(),
            mode: crate::model::GroupMode::Failover,
            relay_config: crate::model::GroupRelayConfig {
                cooldown_seconds: 0,
                ..Default::default()
            },
            items: vec![crate::model::GroupItem {
                channel_model_id: model.id,
                ..Default::default()
            }],
            ..Default::default()
        };
        op::group::create_group(&state, &mut group).await.unwrap();

        let resp = forward_relay(
            state.clone(),
            test_auth(),
            ApiFormat::OpenaiCompletion,
            None,
            Uri::from_static("/v1/chat/completions"),
            HeaderMap::new(),
            chat_body("m-keyrot0").await,
        )
        .await;
        assert_eq!(resp.status(), 200, "第二个 key 必须被尝试");
        server.verify().await;
        let _ = std::fs::remove_file(db_path);
    }

    /// 连接层失败（渠道不可达）必须把冷却记在**该渠道项**上。
    /// 回归：计数器曾在 `item` 被置空之后才求 item_id，冷却落到不存在的 item 0，
    /// 真正失败的渠道不进冷却、下一次请求立刻再撞一次。
    #[tokio::test]
    async fn connection_failure_cools_the_channel_item() {
        let (state, db_path) = test_support::setup_test_state("relay_conn_cooldown").await;
        // 指向一个必然拒绝连接的端口
        let model = add_channel(&state, "ch-dead", "http://127.0.0.1:1", "k1", "m-dead").await;
        let mut group = crate::model::Group {
            name: "m-dead".into(),
            mode: crate::model::GroupMode::Failover,
            relay_config: crate::model::GroupRelayConfig {
                // 冷却窗口拉长，便于断言；重试间隔归零以免测试空等
                cooldown_seconds: 3600,
                retry_interval_seconds: 0,
                stream_first_event_timeout_seconds: 5,
                ..Default::default()
            },
            items: vec![crate::model::GroupItem {
                channel_model_id: model.id,
                ..Default::default()
            }],
            ..Default::default()
        };
        op::group::create_group(&state, &mut group).await.unwrap();
        let item_id = group.items[0].id;

        let resp = forward_relay(
            state.clone(),
            test_auth(),
            ApiFormat::OpenaiCompletion,
            None,
            Uri::from_static("/v1/chat/completions"),
            HeaderMap::new(),
            chat_body("m-dead").await,
        )
        .await;
        assert!(!resp.status().is_success());

        let cooled: Vec<i64> = state
            .route_manager
            .get_snapshot()
            .into_iter()
            .flat_map(|route| route.cooldowns.into_keys().collect::<Vec<_>>())
            .collect();
        assert!(
            cooled.contains(&item_id),
            "失败渠道项必须进入冷却，实际冷却项: {cooled:?} (item_id={item_id})"
        );
        assert!(!cooled.contains(&0), "不得为不存在的 item 0 记冷却: {cooled:?}");

        let _ = std::fs::remove_file(db_path);
    }
}
