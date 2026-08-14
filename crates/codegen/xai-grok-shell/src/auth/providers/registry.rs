use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

use super::SubscriptionProvider;
use crate::auth::{AuthManager, GrokAuth, GrokComConfig};

/// Registry of per-provider AuthManager instances.
#[derive(Clone)]
pub struct AuthRegistry {
    managers: HashMap<SubscriptionProvider, Arc<AuthManager>>,
}

impl AuthRegistry {
    pub fn new(grok_home: &Path, grok_com_config: &GrokComConfig) -> Self {
        let mut managers = HashMap::new();
        for p in SubscriptionProvider::enabled() {
            let mgr = Arc::new(AuthManager::for_provider(grok_home, p, grok_com_config));
            mgr.configure_refresher(None, None);
            managers.insert(p, mgr);
        }
        Self { managers }
    }

    /// Build a registry that reuses the process's existing xAI manager (so its
    /// single-flight and cached credential are shared), adding one manager per
    /// enabled third-party provider.
    pub fn from_xai_manager(xai_manager: Arc<AuthManager>, grok_home: &Path) -> Self {
        let grok_com_config = xai_manager.grok_com_config();
        let mut managers = HashMap::new();

        for p in SubscriptionProvider::enabled() {
            if p == SubscriptionProvider::Xai {
                continue;
            }
            let mgr = Arc::new(AuthManager::for_provider(grok_home, p, grok_com_config));
            mgr.configure_refresher(None, None);
            managers.insert(p, mgr);
        }

        managers.insert(SubscriptionProvider::Xai, xai_manager);
        Self { managers }
    }

    pub fn manager(&self, p: SubscriptionProvider) -> Option<Arc<AuthManager>> {
        self.managers.get(&p).cloned()
    }

    pub fn credential_for(&self, p: SubscriptionProvider) -> Option<GrokAuth> {
        self.managers.get(&p).and_then(|m| m.current_wire_valid())
    }

    pub fn has_live_subscription(&self, p: SubscriptionProvider) -> bool {
        self.managers
            .get(&p)
            .is_some_and(|m| m.current_wire_valid().is_some())
    }

    pub fn start_proactive_refresh_all(&self, cancel: CancellationToken) {
        for manager in self.managers.values() {
            manager.start_proactive_refresh(cancel.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::model::AuthMode;
    use chrono::Utc;

    #[tokio::test]
    async fn auth_registry_scopes_and_managers_are_independent() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();

        let cfg = GrokComConfig::default();
        let reg = AuthRegistry::new(home, &cfg);

        let xai_mgr = reg.manager(SubscriptionProvider::Xai).unwrap();
        let ant_mgr = reg.manager(SubscriptionProvider::Anthropic).unwrap();
        let codex_mgr = reg.manager(SubscriptionProvider::OpenaiCodex).unwrap();

        assert_eq!(ant_mgr.auth_scope(), "anthropic::oauth");
        assert_eq!(codex_mgr.auth_scope(), "openai-codex::oauth");
        assert_ne!(xai_mgr.auth_scope(), ant_mgr.auth_scope());

        // Update Anthropic credential
        let ant_auth = GrokAuth {
            provider: SubscriptionProvider::Anthropic,
            key: "sk-ant-oat-test-123".into(),
            auth_mode: AuthMode::SubscriptionOauth,
            create_time: Utc::now(),
            user_id: "".into(),
            refresh_token: Some("ant-rt".into()),
            ..Default::default()
        };
        ant_mgr.update(ant_auth).await.unwrap();

        assert!(reg.has_live_subscription(SubscriptionProvider::Anthropic));
        assert!(!reg.has_live_subscription(SubscriptionProvider::OpenaiCodex));

        let cred = reg.credential_for(SubscriptionProvider::Anthropic).unwrap();
        assert_eq!(cred.key, "sk-ant-oat-test-123");
        assert_eq!(cred.provider, SubscriptionProvider::Anthropic);

        // Update Codex credential
        let codex_auth = GrokAuth {
            provider: SubscriptionProvider::OpenaiCodex,
            key: "codex-jwt-test".into(),
            auth_mode: AuthMode::SubscriptionOauth,
            create_time: Utc::now(),
            user_id: "chatgpt-acc-1".into(),
            account_id: Some("chatgpt-acc-1".into()),
            refresh_token: Some("codex-rt".into()),
            ..Default::default()
        };
        codex_mgr.update(codex_auth).await.unwrap();

        assert!(reg.has_live_subscription(SubscriptionProvider::OpenaiCodex));
        let codex_cred = reg
            .credential_for(SubscriptionProvider::OpenaiCodex)
            .unwrap();
        assert_eq!(codex_cred.key, "codex-jwt-test");
        assert_eq!(codex_cred.account_id.as_deref(), Some("chatgpt-acc-1"));
    }
}
