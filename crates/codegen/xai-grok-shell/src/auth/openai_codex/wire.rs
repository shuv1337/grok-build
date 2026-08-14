pub const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
pub const AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
pub const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
pub const SCOPES: &str = "openid profile email offline_access";
pub const CALLBACK_PORT: u16 = 1455;
pub const CALLBACK_PATH: &str = "/auth/callback";
pub const CALLBACK_URL: &str = "http://localhost:1455/auth/callback";
pub const DEVICE_USERCODE_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/usercode";
pub const DEVICE_TOKEN_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/token";
pub const DEVICE_REDIRECT_URI: &str = "https://auth.openai.com/deviceauth/callback";
pub const DEVICE_USER_URL: &str = "https://auth.openai.com/codex/device";
pub const RESPONSES_BASE_URL: &str = "https://chatgpt.com/backend-api/codex";
pub const ORIGINATOR: &str = "grok";
/// Subscription-quota endpoint, the same one the Codex CLI reads. Verified
/// live to accept our own [`ORIGINATOR`], so no separate client identity is
/// needed for it.
pub const USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
pub const OPENAI_BETA: &str = "responses=experimental";
pub const AUTH_SCOPE: &str = "openai-codex::oauth";
pub const EXPIRY_MARGIN_SECS: i64 = 300;

pub fn token_url() -> String {
    std::env::var("OPENAI_CODEX_TOKEN_URL_OVERRIDE").unwrap_or_else(|_| TOKEN_URL.to_string())
}

pub fn usage_url() -> String {
    std::env::var("OPENAI_CODEX_USAGE_URL_OVERRIDE").unwrap_or_else(|_| USAGE_URL.to_string())
}

pub fn device_usercode_url() -> String {
    std::env::var("OPENAI_CODEX_DEVICE_USERCODE_URL_OVERRIDE")
        .unwrap_or_else(|_| DEVICE_USERCODE_URL.to_string())
}

pub fn device_token_url() -> String {
    std::env::var("OPENAI_CODEX_DEVICE_TOKEN_URL_OVERRIDE")
        .unwrap_or_else(|_| DEVICE_TOKEN_URL.to_string())
}

/// Extract `chatgpt_account_id` from the decoded JWT claims of the access token.
pub fn extract_chatgpt_account_id(token: &str) -> Option<String> {
    let data = jsonwebtoken::dangerous::insecure_decode::<serde_json::Value>(token).ok()?;
    // The namespaced claim is the documented location; the bare key is a
    // fallback for token shapes that hoist it to the top level.
    [
        data.claims
            .get("https://api.openai.com/auth")
            .and_then(|v| v.get("chatgpt_account_id")),
        data.claims.get("chatgpt_account_id"),
    ]
    .into_iter()
    .flatten()
    .filter_map(|v| v.as_str())
    .find(|acc| !acc.is_empty())
    .map(str::to_owned)
}
