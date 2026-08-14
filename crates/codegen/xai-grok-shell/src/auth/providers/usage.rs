//! Subscription-quota lookups for the alternative providers.
//!
//! Both Anthropic and OpenAI expose the same quota their own first-party CLIs
//! render, on undocumented-but-stable OAuth endpoints:
//!
//! | Provider | Endpoint |
//! |----------|----------|
//! | Anthropic | `GET https://api.anthropic.com/api/oauth/usage` |
//! | OpenAI Codex | `GET https://chatgpt.com/backend-api/wham/usage` |
//!
//! Both authenticate with the subscription access token we already hold, so no
//! extra scope or consent is needed.
//!
//! Everything here fails **soft**. A quota panel is a convenience; a provider
//! outage, a shape change, or an expired token must degrade to one row saying
//! so, never break `/usage` or block a turn.

use serde::{Deserialize, Serialize};

use super::SubscriptionProvider;

/// One quota window (a rolling 5-hour session, a weekly cap, a per-model cap).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UsageWindow {
    /// Row label, e.g. `"Session"`, `"Weekly"`, `"Opus weekly"`.
    pub label: String,
    /// Percent of the allowance **consumed**, 0-100.
    pub used_percent: f64,
    /// RFC3339 instant the window rolls over. Formatted by the client so the
    /// timestamp lands in the viewer's timezone.
    #[serde(default)]
    pub resets_at: Option<String>,
}

/// Quota for one provider, or the reason it is unavailable.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderUsage {
    /// Canonical provider id (`anthropic`, `openai-codex`).
    pub id: String,
    pub display_name: String,
    /// Whether a credential exists at all.
    pub connected: bool,
    /// Plan name the endpoint reports (`"pro"`, `"max"`), when it reports one.
    #[serde(default)]
    pub plan: Option<String>,
    #[serde(default)]
    pub windows: Vec<UsageWindow>,
    /// Present when the lookup failed. Rendered instead of the bars, so a
    /// failure is visible rather than silently showing "no limits".
    #[serde(default)]
    pub error: Option<String>,
}

impl ProviderUsage {
    fn failed(provider: SubscriptionProvider, connected: bool, error: impl Into<String>) -> Self {
        Self {
            id: provider.id().to_string(),
            display_name: provider.display_name().to_string(),
            connected,
            plan: None,
            windows: Vec::new(),
            error: Some(error.into()),
        }
    }
}

/// Wire response for `x.ai/auth/subscription_usage`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubscriptionUsageResponse {
    pub providers: Vec<ProviderUsage>,
}

/// Cap on rows per provider, so a provider that suddenly reports a long tail of
/// per-model limits cannot push the rest of `/usage` off screen.
const MAX_WINDOWS: usize = 6;

/// Seconds in the windows we can name from their length alone.
const FIVE_HOURS: u64 = 5 * 60 * 60;
const SEVEN_DAYS: u64 = 7 * 24 * 60 * 60;

// ── Anthropic ───────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct AnthropicWindow {
    #[serde(default)]
    utilization: Option<f64>,
    #[serde(default)]
    resets_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AnthropicScopeModel {
    #[serde(default)]
    display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AnthropicScope {
    #[serde(default)]
    model: Option<AnthropicScopeModel>,
}

#[derive(Debug, Deserialize)]
struct AnthropicLimit {
    #[serde(default)]
    percent: Option<f64>,
    #[serde(default)]
    resets_at: Option<String>,
    #[serde(default)]
    scope: Option<AnthropicScope>,
}

#[derive(Debug, Deserialize)]
struct AnthropicUsageResponse {
    #[serde(default)]
    five_hour: Option<AnthropicWindow>,
    #[serde(default)]
    seven_day: Option<AnthropicWindow>,
    #[serde(default)]
    seven_day_opus: Option<AnthropicWindow>,
    #[serde(default)]
    seven_day_sonnet: Option<AnthropicWindow>,
    /// Richer per-model breakdown. Only the model-scoped rows are used; the
    /// unscoped ones duplicate `five_hour`/`seven_day`.
    #[serde(default)]
    limits: Vec<AnthropicLimit>,
}

/// Shape the Anthropic payload into rows.
///
/// `utilization` is already a percentage (0-100), not a fraction — an account
/// at 34% of its weekly cap reports `34.0`.
pub(crate) fn parse_anthropic_usage(body: &str) -> Result<Vec<UsageWindow>, String> {
    let parsed: AnthropicUsageResponse =
        serde_json::from_str(body).map_err(|e| format!("unexpected response shape: {e}"))?;

    let mut windows = Vec::new();
    let mut push = |label: &str, w: Option<AnthropicWindow>| {
        if let Some(w) = w
            && let Some(used) = w.utilization
        {
            windows.push(UsageWindow {
                label: label.to_string(),
                used_percent: used.clamp(0.0, 100.0),
                resets_at: w.resets_at,
            });
        }
    };

    push("Session", parsed.five_hour);
    push("Weekly", parsed.seven_day);
    push("Opus weekly", parsed.seven_day_opus);
    push("Sonnet weekly", parsed.seven_day_sonnet);

    // Model-scoped caps (e.g. a per-model weekly limit) are the ones most
    // likely to bite first and are absent from the top-level fields.
    for limit in parsed.limits {
        let Some(name) = limit
            .scope
            .and_then(|s| s.model)
            .and_then(|m| m.display_name)
        else {
            continue;
        };
        let Some(pct) = limit.percent else { continue };
        let label = format!("{name} weekly");
        if windows.iter().any(|w| w.label == label) {
            continue;
        }
        windows.push(UsageWindow {
            label,
            used_percent: pct.clamp(0.0, 100.0),
            resets_at: limit.resets_at,
        });
    }

    windows.truncate(MAX_WINDOWS);
    Ok(windows)
}

// ── OpenAI Codex ────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct CodexWindow {
    #[serde(default)]
    used_percent: Option<f64>,
    #[serde(default)]
    limit_window_seconds: Option<u64>,
    #[serde(default)]
    reset_at: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct CodexRateLimit {
    #[serde(default)]
    primary_window: Option<CodexWindow>,
    #[serde(default)]
    secondary_window: Option<CodexWindow>,
}

#[derive(Debug, Deserialize)]
struct CodexUsageResponse {
    #[serde(default)]
    plan_type: Option<String>,
    #[serde(default)]
    rate_limit: Option<CodexRateLimit>,
}

/// Name a Codex window from its length: the payload labels windows only by
/// duration, so a 604800s window is the weekly cap and an 18000s one is the
/// rolling session cap.
fn codex_window_label(seconds: Option<u64>, fallback: &str) -> String {
    match seconds {
        Some(SEVEN_DAYS) => "Weekly".to_string(),
        Some(FIVE_HOURS) => "Session".to_string(),
        Some(s) if s % 86_400 == 0 => format!("{}-day", s / 86_400),
        Some(s) if s % 3_600 == 0 => format!("{}h", s / 3_600),
        _ => fallback.to_string(),
    }
}

/// Shape the Codex payload into rows. Returns the plan alongside the windows.
pub(crate) fn parse_codex_usage(body: &str) -> Result<(Option<String>, Vec<UsageWindow>), String> {
    let parsed: CodexUsageResponse =
        serde_json::from_str(body).map_err(|e| format!("unexpected response shape: {e}"))?;

    let mut windows = Vec::new();
    if let Some(rate) = parsed.rate_limit {
        for (w, fallback) in [
            (rate.primary_window, "Primary"),
            (rate.secondary_window, "Secondary"),
        ] {
            if let Some(w) = w
                && let Some(used) = w.used_percent
            {
                windows.push(UsageWindow {
                    label: codex_window_label(w.limit_window_seconds, fallback),
                    used_percent: used.clamp(0.0, 100.0),
                    resets_at: w.reset_at.and_then(unix_to_rfc3339),
                });
            }
        }
    }
    windows.truncate(MAX_WINDOWS);
    Ok((parsed.plan_type, windows))
}

/// Codex reports resets as epoch seconds; the panel speaks RFC3339 so the
/// client can localize one way for every provider.
fn unix_to_rfc3339(secs: i64) -> Option<String> {
    chrono::DateTime::from_timestamp(secs, 0).map(|dt| dt.to_rfc3339())
}

// ── Fetch ───────────────────────────────────────────────────────────────────

/// Quota for every enabled provider, in catalog order.
///
/// Providers are queried concurrently: this runs while a modal is open, so the
/// panel should cost one round trip, not one per provider in series.
pub async fn fetch_all(registry: &super::registry::AuthRegistry) -> Vec<ProviderUsage> {
    let providers: Vec<_> = SubscriptionProvider::enabled()
        .into_iter()
        .filter(|p| *p != SubscriptionProvider::Xai)
        .collect();

    let futures = providers.into_iter().map(|p| fetch_one(registry, p));
    futures::future::join_all(futures).await
}

async fn fetch_one(
    registry: &super::registry::AuthRegistry,
    provider: SubscriptionProvider,
) -> ProviderUsage {
    let Some(manager) = registry.manager(provider) else {
        return ProviderUsage::failed(provider, false, "not signed in");
    };
    // `current_wire_valid` deliberately ignores the early-invalidation buffer:
    // a token the API still accepts should still answer a read-only query.
    let Some(auth) = manager.current_wire_valid() else {
        let connected = registry.credential_for(provider).is_some();
        return ProviderUsage::failed(
            provider,
            connected,
            if connected {
                "sign-in expired"
            } else {
                "not signed in"
            },
        );
    };

    match provider {
        SubscriptionProvider::Anthropic => fetch_anthropic(&auth.key).await,
        SubscriptionProvider::OpenaiCodex => {
            fetch_codex(&auth.key, auth.account_id.as_deref()).await
        }
        SubscriptionProvider::Xai => ProviderUsage::failed(provider, true, "not applicable"),
    }
}

async fn fetch_anthropic(token: &str) -> ProviderUsage {
    let provider = SubscriptionProvider::Anthropic;
    let url = crate::auth::anthropic::wire::usage_url();
    let request = crate::http::shared_client()
        .get(url)
        .bearer_auth(token)
        .header(
            "anthropic-version",
            crate::auth::anthropic::wire::ANTHROPIC_VERSION,
        )
        .header("anthropic-beta", crate::auth::anthropic::wire::OAUTH_BETA)
        .timeout(REQUEST_TIMEOUT);

    match send_and_read(request).await {
        Ok(body) => match parse_anthropic_usage(&body) {
            Ok(windows) => ProviderUsage {
                id: provider.id().to_string(),
                display_name: provider.display_name().to_string(),
                connected: true,
                plan: None,
                windows,
                error: None,
            },
            Err(e) => ProviderUsage::failed(provider, true, e),
        },
        Err(e) => ProviderUsage::failed(provider, true, e),
    }
}

async fn fetch_codex(token: &str, account_id: Option<&str>) -> ProviderUsage {
    let provider = SubscriptionProvider::OpenaiCodex;
    let mut request = crate::http::shared_client()
        .get(crate::auth::openai_codex::wire::usage_url())
        .bearer_auth(token)
        .header("accept", "application/json")
        // The backend gates this endpoint on a known client.
        .header("originator", crate::auth::openai_codex::wire::ORIGINATOR)
        .timeout(REQUEST_TIMEOUT);
    if let Some(id) = account_id {
        request = request.header("chatgpt-account-id", id);
    }

    match send_and_read(request).await {
        Ok(body) => match parse_codex_usage(&body) {
            Ok((plan, windows)) => ProviderUsage {
                id: provider.id().to_string(),
                display_name: provider.display_name().to_string(),
                connected: true,
                plan,
                windows,
                error: None,
            },
            Err(e) => ProviderUsage::failed(provider, true, e),
        },
        Err(e) => ProviderUsage::failed(provider, true, e),
    }
}

/// A quota panel must never hang a modal open waiting on a slow provider.
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Send and return the body, mapping transport and HTTP status into short
/// user-facing strings. Response bodies are **not** surfaced: they can echo
/// token material on auth failures.
async fn send_and_read(request: reqwest::RequestBuilder) -> Result<String, String> {
    let response = request.send().await.map_err(|e| {
        if e.is_timeout() {
            "timed out".to_string()
        } else {
            "network error".to_string()
        }
    })?;

    let status = response.status();
    if !status.is_success() {
        return Err(match status.as_u16() {
            401 | 403 => "sign-in expired".to_string(),
            429 => "rate limited".to_string(),
            s => format!("HTTP {s}"),
        });
    }
    response
        .text()
        .await
        .map_err(|_| "could not read response".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Captured from a live `GET /api/oauth/usage`, trimmed to the fields we
    /// read. Pins the two facts most likely to drift: `utilization` is a
    /// percentage (not a 0-1 fraction), and model-scoped caps live only in
    /// `limits`, never in the top-level windows.
    const ANTHROPIC_LIVE: &str = r#"{
      "five_hour": { "utilization": 2.0, "resets_at": "2026-08-15T12:50:00.224042+00:00" },
      "seven_day": { "utilization": 34.0, "resets_at": "2026-08-16T18:00:00.224067+00:00" },
      "seven_day_opus": null,
      "seven_day_sonnet": null,
      "limits": [
        { "kind": "session", "percent": 2, "resets_at": "2026-08-15T12:50:00.224042+00:00", "scope": null },
        { "kind": "weekly_all", "percent": 34, "resets_at": "2026-08-16T18:00:00.224067+00:00", "scope": null },
        { "kind": "weekly_scoped", "percent": 41, "resets_at": "2026-08-16T18:00:00.224334+00:00",
          "scope": { "model": { "id": null, "display_name": "Fable" }, "surface": null } }
      ]
    }"#;

    #[test]
    fn parses_live_anthropic_payload() {
        let windows = parse_anthropic_usage(ANTHROPIC_LIVE).expect("parses");
        let labels: Vec<_> = windows.iter().map(|w| w.label.as_str()).collect();
        assert_eq!(labels, ["Session", "Weekly", "Fable weekly"]);

        assert_eq!(windows[1].used_percent, 34.0, "utilization is a percentage");
        assert_eq!(
            windows[1].resets_at.as_deref(),
            Some("2026-08-16T18:00:00.224067+00:00")
        );
        // Null sub-windows must not become 0% rows: "no Opus cap reported" and
        // "Opus cap untouched" are different claims.
        assert!(!labels.contains(&"Opus weekly"));
        // The scoped cap is the highest here, so dropping it would under-report.
        assert_eq!(windows[2].used_percent, 41.0);
    }

    /// Captured from a live `GET /backend-api/wham/usage`.
    const CODEX_LIVE: &str = r#"{
      "plan_type": "pro",
      "rate_limit": {
        "allowed": true,
        "primary_window": {
          "used_percent": 13, "limit_window_seconds": 604800,
          "reset_after_seconds": 417111, "reset_at": 1787197860
        },
        "secondary_window": null
      }
    }"#;

    #[test]
    fn parses_live_codex_payload() {
        let (plan, windows) = parse_codex_usage(CODEX_LIVE).expect("parses");
        assert_eq!(plan.as_deref(), Some("pro"));
        assert_eq!(windows.len(), 1, "a null secondary window is not a row");
        assert_eq!(windows[0].label, "Weekly", "604800s is the weekly cap");
        assert_eq!(windows[0].used_percent, 13.0);
        assert!(
            windows[0]
                .resets_at
                .as_deref()
                .is_some_and(|s| s.starts_with("2026-")),
            "epoch seconds must become RFC3339: {:?}",
            windows[0].resets_at
        );
    }

    #[test]
    fn window_labels_derive_from_duration() {
        assert_eq!(codex_window_label(Some(SEVEN_DAYS), "x"), "Weekly");
        assert_eq!(codex_window_label(Some(FIVE_HOURS), "x"), "Session");
        assert_eq!(codex_window_label(Some(3 * 86_400), "x"), "3-day");
        assert_eq!(codex_window_label(None, "Primary"), "Primary");
    }

    /// A shape change must degrade to a visible error, not a panic and not a
    /// confident "0%".
    #[test]
    fn malformed_payloads_report_rather_than_panic() {
        assert!(parse_anthropic_usage("not json").is_err());
        assert!(parse_codex_usage("{{").is_err());
        // Valid JSON with nothing we recognize is an empty panel, not an error:
        // the provider answered, it just reports no caps.
        assert_eq!(parse_anthropic_usage("{}").unwrap(), Vec::new());
        assert_eq!(parse_codex_usage("{}").unwrap(), (None, Vec::new()));
    }
}
