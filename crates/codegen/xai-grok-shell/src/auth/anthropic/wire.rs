pub const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
pub const AUTHORIZE_URL: &str = "https://claude.ai/oauth/authorize";
pub const TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
pub const SCOPES: &str =
    "user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload";
pub const CALLBACK_PORT: u16 = 53692;
pub const CALLBACK_PATH: &str = "/callback";
pub const CALLBACK_URL: &str = "http://localhost:53692/callback";
pub const CODE_PASTE_URL: &str = "https://platform.claude.com/oauth/code/callback";
pub const USER_AGENT: &str = "claude-cli/0.2.149 (external, cli)";
pub const ANTHROPIC_VERSION: &str = "2023-06-01";
pub const ANTHROPIC_BETAS: &str = "claude-code-20250219,oauth-2025-04-20,interleaved-thinking-2025-05-14,prompt-caching-scope-2026-01-05";
pub const BASE_URL: &str = "https://api.anthropic.com/v1";
/// Subscription-quota endpoint, the same one Claude Code reads. Note it sits
/// at `/api/`, not under the `/v1` inference [`BASE_URL`].
pub const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
/// The quota endpoint accepts only the OAuth beta, not the full inference
/// [`ANTHROPIC_BETAS`] list.
pub const OAUTH_BETA: &str = "oauth-2025-04-20";
pub const AUTH_SCOPE: &str = "anthropic::oauth";
pub const EXPIRY_MARGIN_SECS: i64 = 300;
pub const SYSTEM_IDENTITY: &str = "You are Claude Code, Anthropic's official CLI for Claude.";

pub fn token_url() -> String {
    std::env::var("ANTHROPIC_TOKEN_URL_OVERRIDE").unwrap_or_else(|_| TOKEN_URL.to_string())
}

pub fn usage_url() -> String {
    std::env::var("ANTHROPIC_USAGE_URL_OVERRIDE").unwrap_or_else(|_| USAGE_URL.to_string())
}

/// Scrub Grok branding from the rendered system prompt before sending on third-party wire.
pub fn debrand_system_prompt(prompt: &str) -> String {
    let mut s = prompt.to_string();
    s = s.replace("You are Grok Code", "You are Claude Code");
    s = s.replace("You are Grok", "You are Claude");
    s = s.replace("Grok Code", "Claude Code");
    s = s.replace("Grok", "Claude");
    s = s.replace("grok", "claude");
    s
}
