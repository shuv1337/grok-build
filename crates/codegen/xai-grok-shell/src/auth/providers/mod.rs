pub mod registry;
pub mod usage;

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use xai_grok_sampler::WireIdentity;

/// First-class model inference and subscription provider.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionProvider {
    #[default]
    Xai,
    Anthropic,
    #[serde(rename = "openai_codex", alias = "openai-codex")]
    OpenaiCodex,
}

/// Which subscription providers currently hold a usable credential.
///
/// Model visibility is a per-model, per-render question, so it cannot afford
/// to build an [`AuthRegistry`] (three `AuthManager`s, each reading
/// `auth.json`) per check. This resolves the whole answer from **one** read
/// and is then passed down by value.
///
/// Deliberately re-read rather than cached: signing in mid-session must make
/// that provider's models appear without a restart.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LiveProviders {
    anthropic: bool,
    openai_codex: bool,
}

impl LiveProviders {
    /// Read `auth.json` once and record which providers have a credential that
    /// is not hard-expired. A missing or unreadable file means "none", which
    /// fails closed to today's behavior.
    pub fn detect(grok_home: &std::path::Path) -> Self {
        let path = std::env::var("GROK_AUTH_PATH")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| grok_home.join("auth.json"));
        let Ok(store) = crate::auth::read_auth_json(&path) else {
            return Self::default();
        };
        let live = |scope: &str| {
            store
                .get(scope)
                .is_some_and(|auth| !crate::auth::is_expired(auth))
        };
        Self {
            anthropic: live("anthropic::oauth"),
            openai_codex: live("openai-codex::oauth"),
        }
    }

    pub fn has(&self, provider: SubscriptionProvider) -> bool {
        match provider {
            // xAI is governed by the session-token gate, not this set.
            SubscriptionProvider::Xai => true,
            SubscriptionProvider::Anthropic => self.anthropic,
            SubscriptionProvider::OpenaiCodex => self.openai_codex,
        }
    }

    /// Build an explicit set (tests and callers that already know the answer).
    pub fn from_parts(anthropic: bool, openai_codex: bool) -> Self {
        Self {
            anthropic,
            openai_codex,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScopeKey {
    Static(&'static str),
    Dynamic(String),
}

impl ScopeKey {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Static(s) => s,
            Self::Dynamic(s) => s.as_str(),
        }
    }
}

impl fmt::Display for ScopeKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl SubscriptionProvider {
    /// String identifier for CLI flags and config files ("xai" | "anthropic" | "openai-codex").
    pub fn id(&self) -> &'static str {
        match self {
            Self::Xai => "xai",
            Self::Anthropic => "anthropic",
            Self::OpenaiCodex => "openai-codex",
        }
    }

    /// User-visible display name for UI pickers and status banners.
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Xai => "Grok",
            Self::Anthropic => "Claude (Pro/Max)",
            Self::OpenaiCodex => "ChatGPT (Plus/Pro)",
        }
    }

    /// Default auth.json scope key for this provider.
    pub fn default_auth_scope(&self) -> ScopeKey {
        match self {
            Self::Xai => ScopeKey::Dynamic(String::new()),
            Self::Anthropic => ScopeKey::Static("anthropic::oauth"),
            Self::OpenaiCodex => ScopeKey::Static("openai-codex::oauth"),
        }
    }

    pub fn static_auth_scope(&self) -> Option<&'static str> {
        match self {
            Self::Xai => None,
            Self::Anthropic => Some("anthropic::oauth"),
            Self::OpenaiCodex => Some("openai-codex::oauth"),
        }
    }

    /// Wire presentation identity for HTTP inference requests.
    pub fn wire_identity(&self) -> WireIdentity {
        match self {
            Self::Xai => WireIdentity::Grok,
            Self::Anthropic | Self::OpenaiCodex => WireIdentity::Impersonated,
        }
    }

    /// All supported subscription providers.
    pub fn all() -> [SubscriptionProvider; 3] {
        [
            SubscriptionProvider::Xai,
            SubscriptionProvider::Anthropic,
            SubscriptionProvider::OpenaiCodex,
        ]
    }

    /// Providers actually enabled in this build. xAI is always present;
    /// the third-party subscription providers are gated behind
    /// `grok_build_alt_providers` so one flip disables the whole surface.
    pub fn enabled() -> Vec<SubscriptionProvider> {
        #[cfg(feature = "grok_build_alt_providers")]
        {
            Self::all().to_vec()
        }
        #[cfg(not(feature = "grok_build_alt_providers"))]
        {
            vec![SubscriptionProvider::Xai]
        }
    }

    /// Whether this provider is usable in the current build.
    pub fn is_enabled(&self) -> bool {
        match self {
            Self::Xai => true,
            Self::Anthropic | Self::OpenaiCodex => {
                cfg!(feature = "grok_build_alt_providers")
            }
        }
    }

    pub fn from_id(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "xai" | "grok" => Some(Self::Xai),
            "anthropic" | "claude" => Some(Self::Anthropic),
            "openai-codex" | "openai_codex" | "openai" | "codex" | "chatgpt" => {
                Some(Self::OpenaiCodex)
            }
            _ => None,
        }
    }
}

impl fmt::Display for SubscriptionProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

impl FromStr for SubscriptionProvider {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_id(s).ok_or_else(|| {
            format!(
                "unknown provider {s:?}; valid choices are: {}",
                Self::all()
                    .iter()
                    .map(|p| p.id())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
    }
}
