use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use serde::{Deserialize, Serialize};

use super::wire::*;
use crate::auth::model::{AuthMode, GrokAuth};
use crate::auth::providers::SubscriptionProvider;
use crate::auth::{AuthChannels, AuthManager, AuthUrlInfo, AuthUrlMode};

#[derive(Serialize)]
struct DeviceUserCodeRequest<'a> {
    client_id: &'a str,
}

#[derive(Deserialize)]
struct DeviceUserCodeResponse {
    device_auth_id: String,
    user_code: String,
    #[serde(default = "default_interval")]
    interval: u64,
}

fn default_interval() -> u64 {
    5
}

#[derive(Serialize)]
struct DeviceTokenRequest<'a> {
    device_auth_id: &'a str,
    user_code: &'a str,
}

#[derive(Deserialize)]
struct DeviceTokenSuccess {
    authorization_code: String,
    code_verifier: String,
}

#[derive(Deserialize)]
struct DeviceTokenError {
    error: Option<String>,
    error_description: Option<String>,
}

#[derive(Deserialize)]
struct OpenAiTokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
}

pub async fn run_openai_codex_device_login(
    auth_manager: &Arc<AuthManager>,
    channels: Option<AuthChannels>,
) -> anyhow::Result<(GrokAuth, bool)> {
    tracing::info!("OpenAI Codex: starting device OAuth login flow");

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;

    let usercode_req = DeviceUserCodeRequest {
        client_id: CLIENT_ID,
    };
    let usercode_res = client
        .post(device_usercode_url())
        .json(&usercode_req)
        .send()
        .await?;

    if !usercode_res.status().is_success() {
        let status = usercode_res.status();
        let body = usercode_res.text().await.unwrap_or_default();
        anyhow::bail!("Failed to request device authorization code ({status}): {body}");
    }

    let usercode_data: DeviceUserCodeResponse = usercode_res.json().await?;
    let mut interval = Duration::from_secs(usercode_data.interval.max(2));

    let (url_tx, _code_rx) = match channels {
        Some(ch) => (ch.url_tx, Some(ch.code_rx)),
        None => (None, None),
    };

    let verify_url = format!("{DEVICE_USER_URL}?user_code={}", usercode_data.user_code);

    if let Some(tx) = url_tx {
        let _ = tx.send(AuthUrlInfo {
            url: verify_url.clone(),
            mode: AuthUrlMode::Device,
            provider: Some(SubscriptionProvider::OpenaiCodex),
        });
    } else {
        eprintln!();
        eprintln!("To sign in with ChatGPT, open this URL in your browser:");
        eprintln!("  {}", verify_url);
        eprintln!();
        eprintln!("Your confirmation code is: {}", usercode_data.user_code);
        eprintln!("Waiting for authentication...");
        eprintln!();
        let _ = webbrowser::open(&verify_url);
    }

    let deadline = tokio::time::Instant::now() + Duration::from_secs(600);
    let mut authorization_code = String::new();
    let mut code_verifier = String::new();

    while tokio::time::Instant::now() < deadline {
        tokio::time::sleep(interval).await;

        let poll_req = DeviceTokenRequest {
            device_auth_id: &usercode_data.device_auth_id,
            user_code: &usercode_data.user_code,
        };

        let poll_res = client.post(device_token_url()).json(&poll_req).send().await;
        let res = match poll_res {
            Ok(r) => r,
            Err(e) => {
                tracing::debug!(error = %e, "Device poll network error, retrying");
                continue;
            }
        };

        let status = res.status();
        if status.is_success() {
            let success: DeviceTokenSuccess = res.json().await?;
            authorization_code = success.authorization_code;
            code_verifier = success.code_verifier;
            break;
        }

        let body = res.text().await.unwrap_or_default();
        if let Ok(err_payload) = serde_json::from_str::<DeviceTokenError>(&body) {
            let code = err_payload.error.as_deref().unwrap_or_default();
            if code == "authorization_pending"
                || code == "deviceauth_authorization_pending"
                || status == reqwest::StatusCode::FORBIDDEN
            {
                continue;
            }
            if code == "slow_down" {
                interval += Duration::from_secs(5);
                continue;
            }
            if code == "expired_token" || code == "access_denied" {
                anyhow::bail!("Device login failed: {code}");
            }
        }
    }

    if authorization_code.is_empty() {
        anyhow::bail!("Timed out waiting for device authorization");
    }

    let token_params = [
        ("grant_type", "authorization_code"),
        ("client_id", CLIENT_ID),
        ("code", authorization_code.as_str()),
        ("code_verifier", code_verifier.as_str()),
        ("redirect_uri", DEVICE_REDIRECT_URI),
    ];

    let token_res = client.post(token_url()).form(&token_params).send().await?;

    if !token_res.status().is_success() {
        let status = token_res.status();
        let body = token_res.text().await.unwrap_or_default();
        anyhow::bail!("Device token exchange failed ({status}): {body}");
    }

    let tokens: OpenAiTokenResponse = token_res.json().await?;

    let account_id = extract_chatgpt_account_id(&tokens.access_token).ok_or_else(|| {
        anyhow::anyhow!(
            "Missing chatgpt_account_id in OpenAI access token. Ensure your account has ChatGPT subscription access."
        )
    })?;

    let now = Utc::now();
    let expires_in_secs = tokens.expires_in.unwrap_or(3600);
    let expires_at = now + chrono::Duration::seconds((expires_in_secs as i64) - EXPIRY_MARGIN_SECS);

    let auth = GrokAuth {
        provider: SubscriptionProvider::OpenaiCodex,
        key: tokens.access_token,
        auth_mode: AuthMode::SubscriptionOauth,
        create_time: now,
        user_id: account_id.clone(),
        account_id: Some(account_id),
        refresh_token: tokens.refresh_token,
        expires_at: Some(expires_at),
        ..Default::default()
    };

    auth_manager
        .update(auth.clone())
        .await
        .map_err(|e| anyhow::anyhow!("Failed to save OpenAI Codex credentials: {e}"))?;

    tracing::info!("OpenAI Codex: device login complete, credentials saved under {AUTH_SCOPE}");
    Ok((auth, true))
}
