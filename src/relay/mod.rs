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
            supported_models:    Vec::new(),
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
}
