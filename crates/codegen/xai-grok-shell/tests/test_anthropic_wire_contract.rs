//! Anthropic wire contract integration tests.
//!
//! Validates:
//! - Full Anthropic HTTP request headers & query params (?beta=true, anthropic-version, claude-cli UA, etc.)
//! - Total absence of all `x-grok-*` headers on Impersonated wire
//! - System[0] identity injection and system prompt de-branding
//! - Tool definitions and history tool-use casing to Claude Code 17 tool names
//! - Tool-use ID normalization
//! - Token refresh rotation persistence and 401 retry behavior

use std::sync::Arc;

use axum::Router;
use axum::http::HeaderMap;
use axum::routing::post;
use chrono::Utc;
use indexmap::IndexMap;
use serde_json::{Value, json};
use tokio::net::TcpListener;

use xai_grok_sampler::{ApiBackend, AuthScheme, SamplerConfig, SamplingClient, WireIdentity};
use xai_grok_sampling_types::conversation::{
    AssistantItem, ConversationItem, ConversationRequest, MessagesShaping, SystemItem, ToolCall,
    ToolResultItem, ToolSpec, UserItem,
};
use xai_grok_sampling_types::{ContentPart, build_messages_request};
use xai_grok_shell::auth::anthropic::AnthropicRefresher;
use xai_grok_shell::auth::anthropic::login::exchange_code;
use xai_grok_shell::auth::anthropic::wire::*;
use xai_grok_shell::auth::providers::SubscriptionProvider;
use xai_grok_shell::auth::{AuthManager, AuthMode, GrokAuth, GrokComConfig};
use xai_grok_test_support::MockInferenceServer;

#[tokio::test]
async fn anthropic_wire_contract_and_header_suppression() {
    let mock = MockInferenceServer::start().await.unwrap();
    let url = mock.origin();

    let mut extra_headers = IndexMap::new();
    extra_headers.insert(
        "anthropic-version".to_string(),
        ANTHROPIC_VERSION.to_string(),
    );
    extra_headers.insert("anthropic-beta".to_string(), ANTHROPIC_BETAS.to_string());
    extra_headers.insert("x-app".to_string(), "cli".to_string());
    extra_headers.insert(
        "anthropic-dangerous-direct-browser-access".to_string(),
        "true".to_string(),
    );
    extra_headers.insert("User-Agent".to_string(), USER_AGENT.to_string());

    let mut query_params = IndexMap::new();
    query_params.insert("beta".to_string(), "true".to_string());

    let config = SamplerConfig {
        api_key: Some("sk-ant-oat-test-access-token".to_string()),
        base_url: format!("{url}/v1"),
        model: "claude-sonnet-5".to_string(),
        api_backend: ApiBackend::Messages,
        auth_scheme: AuthScheme::Bearer,
        wire_identity: WireIdentity::Impersonated,
        extra_headers,
        query_params,
        client_identifier: Some("suppressed-identifier".to_string()),
        client_version: Some("1.0.0".to_string()),
        deployment_id: Some("dep-suppressed".to_string()),
        user_id: Some("usr-suppressed".to_string()),
        context_window: 200_000,
        ..Default::default()
    };

    let client = SamplingClient::new(config).expect("SamplingClient should build");

    let system_prompt = "You are Grok Code, an interactive CLI agent from xAI. Grok is helpful.";
    let debranded = debrand_system_prompt(system_prompt);

    let req = ConversationRequest {
        items: vec![
            ConversationItem::System(SystemItem {
                content: debranded.into(),
            }),
            ConversationItem::User(UserItem {
                content: vec![ContentPart::Text {
                    text: "Run tests".into(),
                }],
                synthetic_reason: None,
                ..Default::default()
            }),
            ConversationItem::Assistant(AssistantItem {
                content: "".into(),
                tool_calls: vec![ToolCall {
                    id: "call_123:abc#456".into(),
                    name: "read_file".to_string(),
                    arguments: r#"{"path":"test.rs"}"#.into(),
                }],
                model_id: None,
                model_fingerprint: None,
                reasoning_effort: None,
            }),
            ConversationItem::ToolResult(ToolResultItem {
                tool_call_id: "call_123:abc#456".to_string(),
                content: "file content".into(),
                images: vec![],
            }),
            ConversationItem::User(UserItem {
                content: vec![ContentPart::Text {
                    text: "Next turn".into(),
                }],
                synthetic_reason: None,
                ..Default::default()
            }),
        ],
        tools: vec![
            ToolSpec {
                name: "read_file".to_string(),
                description: Some("Read a file".to_string()),
                parameters: json!({"type": "object", "properties": {"path": {"type": "string"}}}),
            },
            ToolSpec {
                name: "bash".to_string(),
                description: Some("Run command".to_string()),
                parameters: json!({"type": "object", "properties": {"command": {"type": "string"}}}),
            },
        ],
        model: Some("claude-sonnet-5".to_string()),
        messages_shaping: Some(MessagesShaping {
            inject_claude_identity: true,
            claude_tool_casing: true,
        }),
        x_grok_conv_id: Some("conv-should-be-suppressed".to_string()),
        x_grok_req_id: Some("req-should-be-suppressed".to_string()),
        ..Default::default()
    };

    let messages_req = build_messages_request(&req);
    let wrapper = xai_grok_sampling_types::MessagesRequestWrapper::new(messages_req);

    let _ = client.create_message(wrapper).await;

    let entries = mock.requests();
    assert_eq!(entries.len(), 1);
    let entry = &entries[0];

    // Verify method & path & query
    assert_eq!(entry.method, "POST");
    assert!(entry.path.contains("/v1/messages"));

    // Verify headers
    assert_eq!(entry.header("anthropic-version"), Some("2023-06-01"));
    assert_eq!(entry.header("x-app"), Some("cli"));
    assert_eq!(
        entry.header("anthropic-dangerous-direct-browser-access"),
        Some("true")
    );
    assert_eq!(entry.header("user-agent"), Some(USER_AGENT));
    assert_eq!(
        entry.header("authorization"),
        Some("Bearer sk-ant-oat-test-access-token")
    );
    assert_eq!(entry.header("x-api-key"), None);

    // Verify NO x-grok-* headers exist in headers
    for (name, val) in &entry.headers {
        let name_str: &str = name.as_str();
        assert!(
            !name_str.starts_with("x-grok-"),
            "Disallowed grok header {name}: {val} found in request"
        );
    }

    // Verify body structure
    let body = entry.body.as_ref().expect("body logged");
    let system = body
        .get("system")
        .and_then(|v: &Value| v.as_array())
        .expect("system blocks array");
    assert_eq!(system.len(), 2);
    assert_eq!(
        system[0].get("text").and_then(|v: &Value| v.as_str()),
        Some(SYSTEM_IDENTITY)
    );

    let sys1_text = system[1]
        .get("text")
        .and_then(|v: &Value| v.as_str())
        .unwrap();
    assert!(!sys1_text.to_ascii_lowercase().contains("grok"));
    assert!(sys1_text.contains("Claude"));

    // Verify tools casing in definitions
    let tools = body
        .get("tools")
        .and_then(|v: &Value| v.as_array())
        .expect("tools array");
    assert_eq!(
        tools[0].get("name").and_then(|v: &Value| v.as_str()),
        Some("Read")
    );
    assert_eq!(
        tools[1].get("name").and_then(|v: &Value| v.as_str()),
        Some("Bash")
    );

    // Verify assistant tool_use casing and ID normalization in messages
    let msgs = body
        .get("messages")
        .and_then(|v: &Value| v.as_array())
        .expect("messages array");
    let assistant_msg = &msgs[1];
    let content_blocks = assistant_msg
        .get("content")
        .and_then(|v: &Value| v.as_array())
        .unwrap();
    let tool_use = &content_blocks[0];
    assert_eq!(
        tool_use.get("type").and_then(|v: &Value| v.as_str()),
        Some("tool_use")
    );
    assert_eq!(
        tool_use.get("name").and_then(|v: &Value| v.as_str()),
        Some("Read")
    );
    // ID normalized: "call_123:abc#456" -> "call_123_abc_456"
    assert_eq!(
        tool_use.get("id").and_then(|v: &Value| v.as_str()),
        Some("call_123_abc_456")
    );
}

/// Regression for a live 400: "adaptive thinking is not supported on this
/// model". `thinking: {type: "adaptive"}` and `output_config.effort` are xAI
/// extensions to the Messages dialect; upstream Anthropic only understands
/// `{"type":"enabled","budget_tokens":N}`. The impersonated path must emit the
/// upstream shape, while the xAI path keeps sending adaptive.
#[test]
fn anthropic_impersonated_path_uses_native_thinking_not_adaptive() {
    let base = |shaping: Option<MessagesShaping>| ConversationRequest {
        items: vec![ConversationItem::User(UserItem {
            content: vec![ContentPart::Text { text: "hi".into() }],
            synthetic_reason: None,
            ..Default::default()
        })],
        model: Some("claude-sonnet-4-5".to_string()),
        reasoning_effort: Some(xai_grok_sampling_types::ReasoningEffort::High),
        messages_shaping: shaping,
        ..Default::default()
    };

    // Impersonated: upstream Anthropic shape, and no xAI-only output_config.
    let impersonated = build_messages_request(&base(Some(MessagesShaping {
        inject_claude_identity: true,
        claude_tool_casing: true,
    })));
    let wire = serde_json::to_value(&impersonated).unwrap();
    assert_eq!(wire["thinking"]["type"], "enabled");
    assert!(
        wire["thinking"]["budget_tokens"].as_u64().unwrap() > 0,
        "extended thinking needs a positive budget"
    );
    assert!(
        wire.get("output_config").is_none_or(|v| v.is_null()),
        "output_config.effort is an xAI extension and must not reach Anthropic"
    );
    // budget_tokens must stay below the model's max_tokens.
    let budget = wire["thinking"]["budget_tokens"].as_u64().unwrap();
    assert!(budget < 64_000, "budget must fit the smallest catalog cap");

    // xAI path is unchanged: still adaptive.
    let native = build_messages_request(&base(None));
    let wire = serde_json::to_value(&native).unwrap();
    assert_eq!(wire["thinking"]["type"], "adaptive");
}

/// Regression for a live 400 `invalid_request_error`. The loopback token
/// endpoint requires a **JSON** body and a **`state`** field (for Anthropic the
/// state is the PKCE verifier). A form-encoded body, or JSON missing `state`,
/// is rejected — and neither is visible without exercising the real exchange.
#[tokio::test]
#[serial_test::serial]
async fn anthropic_code_exchange_sends_json_with_state() {
    let app = Router::new().route(
        "/v1/oauth/token",
        post(|headers: HeaderMap, body: String| async move {
            assert_eq!(
                headers
                    .get(reqwest::header::CONTENT_TYPE)
                    .unwrap()
                    .to_str()
                    .unwrap(),
                "application/json",
                "exchange must be JSON, not form-encoded"
            );
            let parsed: Value = serde_json::from_str(&body).expect("exchange body must be JSON");
            assert_eq!(parsed["grant_type"], "authorization_code");
            assert_eq!(parsed["client_id"], CLIENT_ID);
            assert_eq!(parsed["code"], "the-auth-code");
            assert_eq!(parsed["code_verifier"], "the-verifier");
            // Load-bearing: omitting `state` yields 400 invalid_request_error.
            assert_eq!(parsed["state"], "the-state");
            // Must be the registered redirect verbatim.
            assert_eq!(parsed["redirect_uri"], CALLBACK_URL);

            axum::response::Response::builder()
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    json!({
                        "access_token": "sk-ant-oat-fresh",
                        "refresh_token": "rt-fresh",
                        "expires_in": 3600
                    })
                    .to_string(),
                ))
                .unwrap()
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    unsafe {
        std::env::set_var(
            "ANTHROPIC_TOKEN_URL_OVERRIDE",
            format!("http://{addr}/v1/oauth/token"),
        );
    }

    let tokens = exchange_code("the-auth-code", "the-verifier", "the-state", CALLBACK_URL)
        .await
        .expect("exchange succeeds");
    server.abort();
    unsafe { std::env::remove_var("ANTHROPIC_TOKEN_URL_OVERRIDE") };

    assert_eq!(tokens.access_token, "sk-ant-oat-fresh");
    assert_eq!(tokens.refresh_token.as_deref(), Some("rt-fresh"));
}

/// The callback can render the code as `code#state`; only the code may be sent.
#[tokio::test]
#[serial_test::serial]
async fn anthropic_code_exchange_strips_state_suffix_from_code() {
    let app = Router::new().route(
        "/v1/oauth/token",
        post(|body: String| async move {
            let parsed: Value = serde_json::from_str(&body).unwrap();
            assert_eq!(
                parsed["code"], "bare-code",
                "the #state suffix must be stripped from the code"
            );
            axum::response::Response::builder()
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    json!({"access_token": "a", "refresh_token": "r", "expires_in": 3600})
                        .to_string(),
                ))
                .unwrap()
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    unsafe {
        std::env::set_var(
            "ANTHROPIC_TOKEN_URL_OVERRIDE",
            format!("http://{addr}/v1/oauth/token"),
        );
    }

    exchange_code("bare-code#some-state", "v", "s", CALLBACK_URL)
        .await
        .expect("exchange succeeds");
    server.abort();
    unsafe { std::env::remove_var("ANTHROPIC_TOKEN_URL_OVERRIDE") };
}

#[tokio::test]
#[serial_test::serial]
async fn anthropic_refresh_rotation_and_persistence() {
    let app = Router::new().route(
        "/v1/oauth/token",
        post(|headers: HeaderMap, body: String| async move {
            // JSON, not form encoding: this endpoint answers 400
            // `invalid_request_error` to a form-encoded body.
            assert_eq!(
                headers
                    .get(reqwest::header::CONTENT_TYPE)
                    .unwrap()
                    .to_str()
                    .unwrap(),
                "application/json"
            );
            let parsed: Value = serde_json::from_str(&body).expect("refresh body must be JSON");
            assert_eq!(parsed["grant_type"], "refresh_token");
            assert_eq!(parsed["client_id"], CLIENT_ID);
            assert_eq!(parsed["refresh_token"], "old-rt-123");

            axum::response::Response::builder()
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    json!({
                        "access_token": "sk-ant-oat-rotated-new-token",
                        "refresh_token": "new-rotated-rt-456",
                        "expires_in": 3600
                    })
                    .to_string(),
                ))
                .unwrap()
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    unsafe {
        std::env::set_var(
            "ANTHROPIC_TOKEN_URL_OVERRIDE",
            format!("http://{addr}/v1/oauth/token"),
        );
    }

    let temp = tempfile::tempdir().unwrap();
    let auth_manager = Arc::new(AuthManager::new_for_scope(
        temp.path(),
        AUTH_SCOPE.to_string(),
        GrokComConfig::default(),
    ));

    // Seed with initial expired credential
    let initial_auth = GrokAuth {
        provider: SubscriptionProvider::Anthropic,
        key: "sk-ant-oat-old-stale".into(),
        auth_mode: AuthMode::SubscriptionOauth,
        create_time: Utc::now() - chrono::Duration::hours(2),
        user_id: "".into(),
        refresh_token: Some("old-rt-123".into()),
        expires_at: Some(Utc::now() - chrono::Duration::minutes(10)),
        ..Default::default()
    };
    auth_manager.update(initial_auth).await.unwrap();

    let refresher = AnthropicRefresher::for_manager(auth_manager.clone());
    let refreshed = refresher
        .refresh_credential()
        .await
        .expect("refresh success");

    server.abort();

    assert_eq!(refreshed.key, "sk-ant-oat-rotated-new-token");
    assert_eq!(
        refreshed.refresh_token.as_deref(),
        Some("new-rotated-rt-456")
    );
    assert_eq!(refreshed.provider, SubscriptionProvider::Anthropic);
    assert!(refreshed.expires_at.unwrap() > Utc::now());
}
