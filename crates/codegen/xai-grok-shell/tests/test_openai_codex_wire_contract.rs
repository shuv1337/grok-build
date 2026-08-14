//! OpenAI Codex wire contract and device auth integration tests.
//!
//! Validates:
//! - Full Responses API request body shaping (store: false, include, instructions, parallel_tool_calls, clamped prompt_cache_key)
//! - Required header injection (chatgpt-account-id, originator, OpenAI-Beta, session-id, x-client-request-id)
//! - Absence of all `x-grok-*` headers on Impersonated wire
//! - Device code authorization flow (usercode -> poll -> token -> claim extraction)
//! - Token refresh rotation persistence

use std::sync::Arc;

use axum::http::StatusCode;
use chrono::Utc;
use serde_json::json;

use xai_grok_sampler::{ApiBackend, AuthScheme, SamplerConfig, SamplingClient, WireIdentity};
use xai_grok_sampling_types::conversation::{
    ConversationItem, ConversationRequest, SystemItem, ToolSpec, UserItem,
};
use xai_grok_sampling_types::{ContentPart, rs};
use xai_grok_shell::auth::credential_provider::WireValidBearerResolver;
use xai_grok_shell::auth::openai_codex::wire::*;
use xai_grok_shell::auth::providers::SubscriptionProvider;
use xai_grok_shell::auth::{AuthManager, AuthMode, GrokAuth, GrokComConfig};
use xai_grok_test_support::MockInferenceServer;

fn create_mock_jwt(account_id: &str) -> String {
    use base64::Engine;
    let enc = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let header = enc.encode(r#"{"alg":"RS256","typ":"JWT"}"#);
    let payload = enc.encode(
        json!({
            "https://api.openai.com/auth": {
                "chatgpt_account_id": account_id
            },
            "exp": (Utc::now() + chrono::Duration::hours(1)).timestamp()
        })
        .to_string(),
    );
    format!("{header}.{payload}.fake-sig")
}

#[tokio::test]
async fn openai_codex_wire_contract_and_header_suppression() {
    let mock = MockInferenceServer::start().await.unwrap();
    let url = mock.origin();

    let temp = tempfile::tempdir().unwrap();
    let auth_manager = Arc::new(AuthManager::new_for_scope(
        temp.path(),
        AUTH_SCOPE.to_string(),
        GrokComConfig::default(),
    ));

    let mock_jwt = create_mock_jwt("org-chatgpt-account-999");
    let codex_auth = GrokAuth {
        provider: SubscriptionProvider::OpenaiCodex,
        key: mock_jwt.clone(),
        auth_mode: AuthMode::SubscriptionOauth,
        create_time: Utc::now(),
        user_id: "org-chatgpt-account-999".into(),
        account_id: Some("org-chatgpt-account-999".into()),
        refresh_token: Some("codex-rt".into()),
        expires_at: Some(Utc::now() + chrono::Duration::hours(1)),
        ..Default::default()
    };
    auth_manager.update(codex_auth).await.unwrap();

    let resolver = WireValidBearerResolver::shared(auth_manager.clone());

    #[derive(Debug)]
    struct CodexTestInjector {
        auth_mgr: Arc<AuthManager>,
        session_id: String,
    }
    impl xai_grok_sampler::HeaderInjector for CodexTestInjector {
        fn inject(&self, headers: &mut reqwest::header::HeaderMap) {
            let auth = self.auth_mgr.current_wire_valid();
            if let Some(account_id) = auth.as_ref().and_then(|a| a.account_id.as_deref()) {
                headers.insert(
                    reqwest::header::HeaderName::from_static("chatgpt-account-id"),
                    reqwest::header::HeaderValue::from_str(account_id).unwrap(),
                );
            }
            headers.insert(
                reqwest::header::HeaderName::from_static("originator"),
                reqwest::header::HeaderValue::from_static(ORIGINATOR),
            );
            headers.insert(
                reqwest::header::HeaderName::from_static("openai-beta"),
                reqwest::header::HeaderValue::from_static(OPENAI_BETA),
            );
            headers.insert(
                reqwest::header::HeaderName::from_static("session-id"),
                reqwest::header::HeaderValue::from_str(&self.session_id).unwrap(),
            );
            headers.insert(
                reqwest::header::HeaderName::from_static("x-client-request-id"),
                reqwest::header::HeaderValue::from_str("req-test-uuid-456").unwrap(),
            );
        }
    }

    let config = SamplerConfig {
        api_key: Some(mock_jwt),
        base_url: format!("{url}/v1"),
        model: "gpt-5.6-luna".to_string(),
        api_backend: ApiBackend::Responses,
        auth_scheme: AuthScheme::Bearer,
        wire_identity: WireIdentity::Impersonated,
        system_prompt_as_instructions: true,
        bearer_resolver: Some(resolver),
        header_injector: Some(Arc::new(CodexTestInjector {
            auth_mgr: auth_manager.clone(),
            session_id: "sess-test-uuid-123".to_string(),
        })),
        client_identifier: Some("suppressed-id".to_string()),
        deployment_id: Some("suppressed-dep".to_string()),
        user_id: Some("suppressed-user".to_string()),
        context_window: 272_000,
        ..Default::default()
    };

    let client = SamplingClient::new(config).expect("SamplingClient should build");

    let long_prompt_cache_key = "a".repeat(128); // Exceeds 64 chars

    let req = ConversationRequest {
        items: vec![
            ConversationItem::System(SystemItem {
                content: "System instructions for codex".into(),
            }),
            ConversationItem::User(UserItem {
                content: vec![ContentPart::Text {
                    text: "Hello codex".into(),
                }],
                synthetic_reason: None,
                ..Default::default()
            }),
        ],
        tools: vec![ToolSpec {
            name: "bash".to_string(),
            description: Some("Run command".to_string()),
            parameters: json!({"type": "object", "properties": {"command": {"type": "string"}}}),
        }],
        model: Some("gpt-5.6-luna".to_string()),
        prompt_cache_key: Some(long_prompt_cache_key),
        parallel_tool_calls: Some(true),
        x_grok_conv_id: Some("suppressed-conv".to_string()),
        x_grok_req_id: Some("suppressed-req".to_string()),
        ..Default::default()
    };

    let create_resp: rs::CreateResponse = (&req).into();
    let wrapper = xai_grok_sampling_types::CreateResponseWrapper::new(create_resp);

    let _ = client.create_response(wrapper).await;

    let entries = mock.requests();
    assert_eq!(entries.len(), 1);
    let entry = &entries[0];

    // Verify method & path
    assert_eq!(entry.method, "POST");
    assert_eq!(entry.path, "/v1/responses");

    // Verify headers
    assert_eq!(
        entry.header("chatgpt-account-id"),
        Some("org-chatgpt-account-999")
    );
    assert_eq!(entry.header("originator"), Some("grok"));
    assert_eq!(entry.header("openai-beta"), Some("responses=experimental"));
    assert_eq!(entry.header("session-id"), Some("sess-test-uuid-123"));
    assert_eq!(
        entry.header("x-client-request-id"),
        Some("req-test-uuid-456")
    );

    // Verify absence of grok headers
    for (name, val) in &entry.headers {
        let name_str: &str = name.as_str();
        assert!(
            !name_str.starts_with("x-grok-"),
            "Disallowed grok header {name}: {val} found in request"
        );
    }

    // Verify body parameters
    let body = entry.body.as_ref().expect("body logged");
    assert_eq!(
        body.get("store")
            .and_then(|v: &serde_json::Value| v.as_bool()),
        Some(false)
    );
    assert_eq!(
        body.get("parallel_tool_calls")
            .and_then(|v: &serde_json::Value| v.as_bool()),
        Some(true)
    );
    assert_eq!(
        body.get("instructions")
            .and_then(|v: &serde_json::Value| v.as_str()),
        Some("System instructions for codex")
    );

    // Clamped prompt_cache_key to exactly 64 chars
    let cache_key = body
        .get("prompt_cache_key")
        .and_then(|v: &serde_json::Value| v.as_str())
        .unwrap();
    assert_eq!(cache_key.len(), 64);
    assert_eq!(cache_key, &"a".repeat(64));

    // Input items contains ONLY user messages, not system messages
    let input = body
        .get("input")
        .and_then(|v: &serde_json::Value| v.as_array())
        .expect("input array");
    assert_eq!(input.len(), 1);
    assert_eq!(
        input[0]
            .get("role")
            .and_then(|v: &serde_json::Value| v.as_str()),
        Some("user")
    );
}

#[tokio::test]
#[serial_test::serial]
async fn openai_codex_device_flow_and_claim_extraction() {
    let poll_count = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let poll_count_clone = poll_count.clone();

    let app = axum::Router::new()
        .route(
            "/api/accounts/deviceauth/usercode",
            axum::routing::post(|| async {
                axum::response::Response::builder()
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(
                        json!({
                            "device_auth_id": "da_12345",
                            "user_code": "WDJ8-4921",
                            "interval": 1
                        })
                        .to_string(),
                    ))
                    .unwrap()
            }),
        )
        .route(
            "/api/accounts/deviceauth/token",
            axum::routing::post(move || {
                let count = poll_count_clone.clone();
                async move {
                    let iter = count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    if iter == 0 {
                        axum::response::Response::builder()
                            .status(StatusCode::FORBIDDEN)
                            .header("content-type", "application/json")
                            .body(axum::body::Body::from(
                                json!({
                                    "error": "authorization_pending"
                                })
                                .to_string(),
                            ))
                            .unwrap()
                    } else {
                        axum::response::Response::builder()
                            .status(StatusCode::OK)
                            .header("content-type", "application/json")
                            .body(axum::body::Body::from(
                                json!({
                                    "authorization_code": "ac_test_auth_code",
                                    "code_verifier": "cv_test_verifier"
                                })
                                .to_string(),
                            ))
                            .unwrap()
                    }
                }
            }),
        )
        .route(
            "/oauth/token",
            axum::routing::post(|body: String| async move {
                assert!(body.contains("grant_type=authorization_code"));
                assert!(body.contains("code=ac_test_auth_code"));
                assert!(body.contains("code_verifier=cv_test_verifier"));

                let jwt = create_mock_jwt("org-device-chatgpt-account");
                axum::response::Response::builder()
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(
                        json!({
                            "access_token": jwt,
                            "refresh_token": "device-rt-123",
                            "expires_in": 3600
                        })
                        .to_string(),
                    ))
                    .unwrap()
            }),
        );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    unsafe {
        std::env::set_var(
            "OPENAI_CODEX_DEVICE_USERCODE_URL_OVERRIDE",
            format!("http://{addr}/api/accounts/deviceauth/usercode"),
        );
        std::env::set_var(
            "OPENAI_CODEX_DEVICE_TOKEN_URL_OVERRIDE",
            format!("http://{addr}/api/accounts/deviceauth/token"),
        );
        std::env::set_var(
            "OPENAI_CODEX_TOKEN_URL_OVERRIDE",
            format!("http://{addr}/oauth/token"),
        );
    }

    let temp = tempfile::tempdir().unwrap();
    let auth_manager = Arc::new(AuthManager::new_for_scope(
        temp.path(),
        AUTH_SCOPE.to_string(),
        GrokComConfig::default(),
    ));

    let (auth, success) =
        xai_grok_shell::auth::openai_codex::device::run_openai_codex_device_login(
            &auth_manager,
            None,
        )
        .await
        .expect("device login ok");

    server.abort();

    assert!(success);
    assert_eq!(auth.provider, SubscriptionProvider::OpenaiCodex);
    assert_eq!(
        auth.account_id.as_deref(),
        Some("org-device-chatgpt-account")
    );
    assert_eq!(auth.user_id, "org-device-chatgpt-account");
    assert_eq!(auth.refresh_token.as_deref(), Some("device-rt-123"));
}

#[tokio::test]
#[serial_test::serial]
async fn openai_codex_refresh_rotation_and_persistence() {
    let app = axum::Router::new().route(
        "/oauth/token",
        axum::routing::post(|body: String| async move {
            assert!(body.contains("grant_type=refresh_token"));
            assert!(body.contains("refresh_token=old-codex-rt"));

            let jwt = create_mock_jwt("org-refreshed-chatgpt-account");
            axum::response::Response::builder()
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    json!({
                        "access_token": jwt,
                        "refresh_token": "new-codex-rt-789",
                        "expires_in": 3600
                    })
                    .to_string(),
                ))
                .unwrap()
        }),
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    unsafe {
        std::env::set_var(
            "OPENAI_CODEX_TOKEN_URL_OVERRIDE",
            format!("http://{addr}/oauth/token"),
        );
    }

    let temp = tempfile::tempdir().unwrap();
    let auth_manager = Arc::new(AuthManager::new_for_scope(
        temp.path(),
        AUTH_SCOPE.to_string(),
        GrokComConfig::default(),
    ));

    let old_jwt = create_mock_jwt("org-refreshed-chatgpt-account");
    let initial_auth = GrokAuth {
        provider: SubscriptionProvider::OpenaiCodex,
        key: old_jwt,
        auth_mode: AuthMode::SubscriptionOauth,
        create_time: Utc::now() - chrono::Duration::hours(2),
        user_id: "org-refreshed-chatgpt-account".into(),
        account_id: Some("org-refreshed-chatgpt-account".into()),
        refresh_token: Some("old-codex-rt".into()),
        expires_at: Some(Utc::now() - chrono::Duration::minutes(10)),
        ..Default::default()
    };
    auth_manager.update(initial_auth).await.unwrap();

    let refresher =
        xai_grok_shell::auth::openai_codex::CodexRefresher::for_manager(auth_manager.clone());
    let refreshed = refresher
        .refresh_credential()
        .await
        .expect("refresh success");

    server.abort();

    assert_eq!(refreshed.provider, SubscriptionProvider::OpenaiCodex);
    assert_eq!(refreshed.refresh_token.as_deref(), Some("new-codex-rt-789"));
    assert_eq!(
        refreshed.account_id.as_deref(),
        Some("org-refreshed-chatgpt-account")
    );
    assert!(refreshed.expires_at.unwrap() > Utc::now());
}
