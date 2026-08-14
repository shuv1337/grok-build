# Plan: Alternative first-class providers with OAuth subscription login

**Status:** revised after repo-grounded review · **Owner:** shuv · **References:** `~/repos/shuvpi`, `~/repos/shuvcode`

**Goal.** Anthropic (Claude Pro/Max) and OpenAI (ChatGPT Plus/Pro via Codex) become
first-class providers in `grok`: `grok login` offers a provider picker, OAuth
subscription credentials live beside the xAI session credential in `auth.json`,
tokens refresh transparently (including Anthropic's rotating refresh tokens), and
each provider ships its own model catalog and wire shaping.

**Non-goals (v1).** Google/Antigravity; gateway-hosted credentials (opencode-console
model); WS transport for Codex (SSE only); tier-based model allowlisting beyond
login/no-login gating; per-provider proxy processes (shuvcode's loopback proxy
pattern — we shape in-process instead).

**Revision note.** All file:line citations below were verified against the working
tree. Changes from the first draft: first-party header suppression is now a
tracked blocker (§3.5), the credential layer is a per-provider `AuthManager`
registry rather than a parameterized single manager (§3.3.7), the Codex
`instructions` move lands in the sampler rather than the `From` impl (§3.6.3),
the Anthropic identity block is injected at wire-shaping time rather than into
stored conversation state (§3.6.1), fail-closed integration now names its three
real enforcement sites (§3.4), and the catalog loader accounts for
`DefaultModelJson` (§3.7).

---

## 1. Verified reference contracts

Confirmed against both reference repos; per-item attribution noted where they differ.

### 1.1 Anthropic (Claude Pro/Max)

| Item | Value |
|---|---|
| client_id | `9d1c250a-e61b-44d9-88ed-5944d1962f5e` (official Claude Code CLI id) |
| authorize | `https://claude.ai/oauth/authorize` |
| token | `POST https://platform.claude.com/v1/oauth/token` |
| scopes | `user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload` — **exactly shuvcode's working set** (`shuvcode/packages/core/src/plugin/provider/anthropic-claude-code.ts:39`). shuvpi additionally sends `org:create_api_key` (`shuvpi/packages/ai/src/auth/oauth/anthropic.ts:36-37`); omitting it is reference-verified, not a gamble |
| PKCE | S256; **`state` = the PKCE verifier itself** (shuvpi `anthropic.ts:255`, shuvcode `anthropic-claude-code.ts:128`; callback validates `state === verifier`, shuvpi `:287,297`) |
| extra authorize params | `code=true`, `response_type=code` |
| redirect | loopback `http://localhost:53692/callback` (shuvpi `anthropic.ts:32-35`). **Fixed port — not negotiable**, it is pre-registered with the provider. (Official/shuvcode alt: `https://platform.claude.com/oauth/code/callback` + paste displayed code; keep as fallback capture path) |
| token body | form-urlencoded; `grant_type=authorization_code` + `code_verifier` + `client_id` + `redirect_uri`; refresh = `grant_type=refresh_token` + `client_id` (no `scope`) |
| **refresh rotation** | response carries a **new `refresh_token`** every refresh — must persist before release; concurrent refresh invalidates the loser |
| expiry margin | store `expires_at = now + expires_in − 5 min`. **shuvpi only** (`anthropic.ts:230,351`); shuvcode stores raw `expires_in` (`anthropic-claude-code.ts:116`). We adopt shuvpi's margin |
| token shape | access tokens start `sk-ant-oat` (useful for detection/logging redaction) |

Request shaping against `https://api.anthropic.com/v1/messages`. All of this is
required for subscription-plan treatment: shuvcode documents that a request which
authenticates correctly but *presents* differently is accepted and then billed as
pay-as-you-go "extra usage" instead of against the subscription
(`anthropic-claude-code.ts:8-9`).

```
POST /v1/messages?beta=true
Authorization: Bearer sk-ant-oat…          (NEVER x-api-key)
anthropic-version: 2023-06-01
anthropic-beta: claude-code-20250219,oauth-2025-04-20[,<feature betas>]
user-agent: claude-cli/<pinned-ver> (external, cli)
x-app: cli
anthropic-dangerous-direct-browser-access: true
accept: application/json
X-Claude-Code-Session-Id: <session uuid>
```
- **`anthropic-version: 2023-06-01` is required and is currently sent by nothing
  in this repo** — `grep -rn "anthropic-version" crates/` returns zero hits. Both
  references get it for free from the official SDK, which injects it at
  `@anthropic-ai/sdk/src/client.ts:841`; grok's Messages client is hand-rolled, so
  it must set the header explicitly. Add to the Anthropic provider `extra_headers`
  and assert it in T14.
- `X-Claude-Code-Session-Id` is confirmed in shuvpi's OAuth branch
  (`anthropic-messages.ts:915`, conditional on a client session id). shuvcode's
  `headers()` omits it (`anthropic-claude-code.ts:299-304`) — send it, matching
  shuvpi and real Claude Code.
- Current shuvcode beta set adds two feature betas
  (`interleaved-thinking-2025-05-14,prompt-caching-scope-2026-01-05`,
  `anthropic-claude-code.ts:43-44`) — pin the exact string in `wire.rs` at
  implementation time.
- **No other headers may ride along.** In particular every `x-grok-*` header and
  the grok User-Agent must be suppressed — see §3.5, which is a blocking
  precondition for this whole contract.
- `system[0]` forced to `"You are Claude Code, Anthropic's official CLI for Claude."`;
  the real system prompt follows as the next system block.
- **The trailing system prompt must be de-branded.** shuvpi does not merely
  prepend an identity block — under OAuth it rewrites the harness prompt before
  sending (`anthropic-messages.ts`, OAuth branch of `buildParams`):
  `.replace(shuvpiSystemIdentity, claudeCodeSystemIdentity).replace(/shuvpi/gi, "Claude")`.
  A prompt that opens with the Claude Code identity and then says "You are Grok
  Code…" is self-contradicting and an obvious fingerprint. See §3.6.2.
- Both system blocks carry `cache_control` when caching is enabled (shuvpi sets it
  on each block). Note this interacts with the `SystemParam::Text` shortcut —
  see §3.6.1.
- Tool `input_schema` is normalized to the legacy three-key shape
  (`{type:"object", properties, required}`) and tools carry
  `eager_input_streaming: true` (`convertTools`). Tool-use ids are normalized to
  `[^a-zA-Z0-9_-] → "_"` truncated to 64 chars (`normalizeToolCallId`) — Anthropic
  rejects ids outside that pattern.
- Tool names canonicalized to Claude Code casing. The exact list is
  `claudeCodeTools` at `shuvpi/packages/ai/src/api/anthropic-messages.ts:84-101`
  (17 names): `Read, Write, Edit, Bash, Grep, Glob, AskUserQuestion,
  EnterPlanMode, ExitPlanMode, KillShell, NotebookEdit, Skill, Task, TaskOutput,
  TodoWrite, WebFetch, WebSearch`. **There is no `LS`.** Forward map is
  case-insensitive lookup (`toClaudeCodeName`, `:106`); reverse map matches
  case-insensitively against the *actual* session tool list
  (`fromClaudeCodeName`, `:107-115`) — copy that shape, not a static inverse.
- No account-id header needed. No DPoP.

### 1.2 OpenAI (ChatGPT Plus/Pro → Codex backend)

| Item | Value |
|---|---|
| client_id | `app_EMoamEEZ73f0CkXaXp7hrann` (official Codex CLI id) |
| authorize | `https://auth.openai.com/oauth/authorize` |
| token | `POST https://auth.openai.com/oauth/token` (form-urlencoded) |
| scope | `openid profile email offline_access` |
| extra authorize params | `id_token_add_organizations=true`, `codex_cli_simplified_flow=true`, `originator=<app>` (`shuvpi/packages/ai/src/auth/oauth/openai-codex.ts:310-316`; shuvpi defaults `originator` to `"shuvpi"` at `:301` — it is a parameter, so `grok` is structurally fine but unverified against OpenAI acceptance; see D4) |
| redirect (browser) | loopback `http://localhost:1455/auth/callback` (`openai-codex.ts:30`, listener at `:377`). **Fixed port — not negotiable** |
| device flow | `POST /api/accounts/deviceauth/usercode` `{client_id}` → `{device_auth_id, user_code, interval}`; user visits `https://auth.openai.com/codex/device`; poll `POST /api/accounts/deviceauth/token` `{device_auth_id, user_code}` → on success returns `{authorization_code, code_verifier}` (**server supplies the PKCE verifier**, `openai-codex.ts:260-269`) → normal token exchange with `redirect_uri=https://auth.openai.com/deviceauth/callback` (`:34`). Pending state is `deviceauth_authorization_pending` (`:285`) |
| **account id** | decode access-token JWT, claim `https://api.openai.com/auth` → `chatgpt_account_id` (`openai-codex.ts:61,406`); send as `chatgpt-account-id` header — mandatory, fail login if missing |
| transport | plain Bearer, no DPoP |

Request shaping against `https://chatgpt.com/backend-api/codex/responses`
(Responses API; SSE):

```
Authorization: Bearer <access>
chatgpt-account-id: <from JWT claim>
originator: grok
OpenAI-Beta: responses=experimental      (shuvpi openai-codex-responses.ts:1666)
session-id: <clamped uuid>
x-client-request-id: <same>
accept: text/event-stream
```
Body: `model`, `stream:true`, **`store:false`** (backend rejects true — "Store must
be set to false", `openai-codex-responses.ts:1481`, set at `:556`),
`include:["reasoning.encrypted_content"]` (`:561`), `prompt_cache_key` (`:562`;
max length 64, `shuvpi/packages/ai/src/api/openai-prompt-cache.ts:1`
`OPENAI_PROMPT_CACHE_KEY_MAX_LENGTH = 64`), `instructions: <system prompt>`
(Codex clients put system in `instructions`, not an input system message),
`parallel_tool_calls:true`, optional `reasoning:{effort,summary:"auto"}`,
`text:{verbosity:"low"}`.

Same suppression rule as Anthropic: no `x-grok-*`, no grok UA (§3.5).

### 1.3 Model catalogs (from shuvpi, current)

- **anthropic** (`providers/data/anthropic.json`): claude-opus-5, claude-opus-4-8/4-7/4-6,
  claude-sonnet-5/4-6, claude-sonnet-4-5, claude-haiku-4-5, claude-fable-5 (+ dated
  snapshots). Context 200k–1M, max_tokens 64k–128k, all reasoning-capable.
- **openai-codex** (`providers/data/openai-codex.json`): gpt-5.6-luna/sol/terra,
  gpt-5.5, gpt-5.4, gpt-5.4-mini, gpt-5.3-codex-spark, gpt-daybreak-blue-latest.
  Context 128k–272k, max_tokens 128k.

These move fast; treat the JSON as a snapshot to refresh per release.

---

## 2. Current-state seams (verified file:line)

Paths are relative to `crates/codegen/`.

| Concern | Seam | Status |
|---|---|---|
| Wire dialects | `ApiBackend::{ChatCompletions,Responses,Messages}` — `xai-grok-sampling-types/src/types.rs:1013` (+ `impl` helpers `:1023-1042`) | ✅ all three exist |
| Auth scheme | `AuthScheme::{Bearer,XApiKey}` — `xai-grok-sampler/src/config.rs:20-24`; chosen in `SamplingClient::new` `client.rs:575-603`, re-stamped per request in `post()` `client.rs:738-757` (only when a `bearer_resolver` is wired) | ✅ |
| Provider config data | `ModelProviderConfig` — `xai-grok-shell/src/agent/model_providers.rs:9-23` (base_url, api_base_url, env_key, api_key, api_backend, extra_headers, query_params, env_http_headers, auth_provider, auth, context_window) | ✅ BYOK works today |
| Static headers/query | `extra_headers` folded at `client.rs:608-614`; `query_params` → `EndpointTemplate` `client.rs:711`, `impl` `:415-459` | ✅ enough for Anthropic `?beta=true` + beta headers |
| **First-party header stamping** | `GrokRequestHeaders::apply` — `xai-grok-sampler/src/client.rs:60-77` (`x-grok-conv-id`, `-req-id`, `-model-override`, `-session-id`, `-agent-id`, `-turn-idx`, `-deployment-id`, `-user-id`), applied on all three send paths `client.rs:978,1038,1249`; `x-grok-client-version` `:624-631`; `x-grok-deployment-id` `:633-640`; `x-grok-user-id` `:642-647`; `x-grok-client-identifier` (always, with default) `:649-660`; **UA always inserted at `:663-673`, after `extra_headers`** | 🚨 **blocker** — leaks grok identity to third parties and clobbers the impersonated UA (§3.5) |
| **Live per-request headers** | `HeaderInjector` trait — `xai-grok-sampler/src/config.rs:186-188`, invoked in `post()` at `client.rs:782-784` **after** bearer re-stamp and after `sent_bearer` capture `:781` | ✅ the seam for `chatgpt-account-id` (rotates with token) |
| **Live bearer** | `BearerResolver` — sampler `config.rs:179-183`; `WireValidBearerResolver(pub Arc<AuthManager>)` — `xai-grok-shell/src/auth/credential_provider.rs:18`, ctor `shared()` `:22-24`, impl `:31-35`; construction sites `credential_provider.rs:22`, `agent/subagent/mod.rs:748`, `session/acp_session_impl/sampler_turn.rs:520` (+ test `:516`) | ⚠️ wraps one `AuthManager`; needs the right instance per provider (§3.3.7) |
| Credential resolution | `resolve_credentials` — `xai-grok-shell/src/agent/config.rs:4787-4846` (arms: own_credential `:4792-4797`, auth_provider `:4798-4804`, session_key `:4805-4810`, `XAI_API_KEY` `:4811-4816`, fallthrough warn `:4817-4833`); `sampling_config_for_model` `:5159-5216` (leaves `bearer_resolver: None` `:5209`, `header_injector: None` `:5214`); `ModelsManager::sampling_config` `agent/models.rs:1017-1050` | ⚠️ needs a subscription arm |
| **Fail-closed (foreign URL) enforcement** | *Not* in `resolve_credentials`. Real sites: catalog stamp `AuthProviderRef::fail_closed` for provider-backed models on non-xAI URLs — `agent/config.rs:3568-3580`; `session_token_auth_gate` — `agent/auth_method.rs:390-401`; session bearer-resolver stamping gated on `is_xai_api_bearer_url` — `config.rs:5094-5096`; kill-switch scoping `enforce_disable_api_key_auth` `:4855-4860`. Tests: `model_providers.rs:335`, `config_tests.rs:826`, `config_tests.rs:468` | ⚠️ new provider models are classified BYOK fail-closed today (§3.4) |
| Messages body | `build_messages_request` — `sampling-types/src/conversation/messages.rs:74-355`; System items hoisted to top-level `system_blocks` `:168-176`; collapse `:281-288` — `SystemParam::Text` only when **len==1 AND `cache_control.is_none()`** `:283-285`, else `SystemParam::Blocks` `:287`; tool name `:297`; Anthropic max_tokens default `sampler/client.rs:46` (`ANTHROPIC_DEFAULT_MAX_TOKENS = 128_000`) applied `:1572-1577` | ✅ good injection point for identity + tool casing |
| Responses body | `From<&ConversationRequest> for rs::CreateResponse` — `sampling-types/src/conversation/responses.rs:97-164`; `instructions: None` hardcoded `:133`; System items become `EasyInputMessage{role:System}` in `input` `:203-209`; `parallel_tool_calls: None` **already present** `:138`; `prompt_cache_key` mapped `:141-144` (**no clamp anywhere in crate**); tools `:322-347`. Sampler defaults: `store` → `Some(false)` `client.rs:1214-1216`, `reasoning.encrypted_content` ensured `:1219-1222`, both inside `apply_response_defaults` `:1192` | ⚠️ the `From` impl cannot see `SamplerConfig` (§3.6.3) |
| System prompt path | templates → `PromptContext::render_with_renderer` (`xai-grok-agent/src/prompt/context.rs:278-309`) → `ConversationItem::system` at session setup `session/acp_session_impl/session_setup.rs:35-49` (`:39`); spawn `spawn.rs:1009-1021` + persist `conversation.first()` `:1046-1049` (constructor actually in `prompt_build.rs:310`); model switch `model_switch.rs:309-328` swaps **only the leading System message** via `chat_state_handle.replace_system_head` `:317-320` | ⚠️ single-leading-System-head invariant — do **not** prepend a second stored System item (§3.6.1) |
| Tool defs → wire | `turn_base_tool_specs` — `session/acp_session_impl/sampler_turn.rs:142-149` → per-backend serialization (Messages `messages.rs:296-300`, Responses `responses.rs:322-347`); name-override machinery in `xai-grok-tools/src/registry/types.rs:58` (`name_override`), resolver `:177-181` | ⚠️ per-provider remap + reverse-map needed at the shell seam |
| OAuth machinery | `run_login_flow_with_config` — `auth/oidc/login.rs:372`; paste parse `parse_pasted_input` `:37-69`; callback page `:72`; races `race_callback_and_client_ui` `:247`, `race_callback_and_stdin` `:301`; timeout const `:29-30`; RFC 8628 device — `auth/device_code.rs` (`DEVICE_GRANT_TYPE` `:19`, slow-down `:21`, `NotEnabled` fallback `:30-36`, browser open `:398`) | ✅ extractable; port strategy needs a mode switch (§3.3.1) |
| Refresh machinery | `TokenRefresher` — `auth/refresh/mod.rs:233-241`; `build_refresher` `:243-262` keys on **`auth_provider_command` presence**, takes one `Arc<AuthManager>`; `AuthManager` fields `manager.rs:160-249` (`scope` `:167`, `refresher` `:170`, `proactive_started` `:176`, `refresh_lock: Mutex<()>` `:178`); scope fixed at construction `:309-310`; 401 recovery `try_recover_unauthorized` `:2546`; `start_proactive_refresh` `:2615`; flock `auth/manager/lock.rs:338`; atomic write `auth/storage.rs:311` | ⚠️ **single-scope by construction** — one manager per provider (§3.3.7) |
| Storage | `AuthStore = BTreeMap<String, GrokAuth>` — `auth/model.rs:260`; `xai::api_key` `model.rs:15`; legacy scope `model.rs:12`; **`{issuer}::{client_id}` built in `auth/config.rs:219,264`** (`auth_scope()`), frozen test `config.rs:375`; `GrokAuth` `model.rs:48-108` (required `user_id: String`, `auth_mode: AuthMode`); `is_session_auth()` `:165` | ⚠️ xAI-shaped; extend |
| Identity config | `GrokComConfig` — `auth/config.rs:59-95`; frozen CORS allowlist `PROD_ACCOUNTS_APP_ORIGINS` `config.rs:137`, frozen test `:384-385`; `GROK_LOCAL_AUTH` → local dev **issuer** `:171-184` | untouched — new providers live outside it |
| Login UX | CLI `grok login` — `xai-grok-pager/src/app/cli.rs:25-46`; `run_cli_login` `auth/flow.rs:964`; TUI `/login` → `Action::Login` **`xai-grok-pager/src/slash/commands/login.rs:21-22`** (`event_loop.rs:1131` is the *startup auto-login*, guard `:1121`); both converge at `app/dispatch/router.rs:1162`; welcome device-code copy `views/welcome/mod.rs:996-1000`, `BrowserStatusKind::Device` `:1219-1223`, arm renderer `:1232`; ACP `x.ai/auth/get_url` — `extensions/auth.rs:21`; `AuthUrlMode` `flow.rs:169-176` (+impl to `:192`); `AuthStatus` — `xai-grok-shell/src/cli_models.rs:11-19` | ⚠️ add provider dimension |
| Model catalog shape | `xai-grok-models/default_models.json` rows deserialize into **`DefaultModelJson`** (`xai-grok-shell/src/agent/config.rs:3750`, "JSON-only subset"), converted in `default_models()` `:3785-3847` (base_url filled from endpoints `:3813-3814`) → `ModelEntryConfig` `:3849-3966`; models crate embeds raw JSON (`xai-grok-models/src/lib.rs:12`, row struct `:27-29`); no `provider` field on either struct; `model_provider: Option<String>` exists only on TOML overrides `:3996`; gating `ModelInfo::visible_for_auth` `:4324-4326`, BYOK forces `supported_in_api` `:4139-4142` | ⚠️ new JSONs are **not** `ModelEntryConfig`-shaped (§3.7) |
| Testing | `xai-grok-sampler/tests/request_query_and_headers.rs` (test `:15`); body capture is in the `#[cfg(test)]` module of **`src/client.rs`** — helper `:2317-2375`, test `:2377-2398`; `xai-grok-test-support/src/mock_server.rs` (`LogEntry` `:40-47` with `authorization` `:44` and lowercase `headers` `:46`, accessor `:51-57`, Messages endpoint classified `:828-846`), SSE generators `src/sse.rs` | ✅ pattern established |

---

## 3. Design

### 3.1 Provider registry — new `xai-grok-shell/src/auth/providers/`

```rust
// src/auth/providers/mod.rs
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionProvider { Xai, Anthropic, OpenaiCodex }

impl SubscriptionProvider {
    pub fn id(&self) -> &'static str;              // "xai" | "anthropic" | "openai-codex"
    pub fn display_name(&self) -> &'static str;    // "Grok", "Claude (Pro/Max)", "ChatGPT (Plus/Pro)"
    pub fn auth_scope(&self) -> &'static str;      // legacy xAI scopes / "anthropic::oauth" / "openai-codex::oauth"
    pub fn wire_identity(&self) -> WireIdentity;   // Grok | Impersonated (§3.5)
    pub fn all() -> [SubscriptionProvider; 3];
}
```
Providers are hardcoded first-class (like xAI is today), *not* user-config-defined.
`wire.rs` submodules hold the pinned constants from §1 (client ids, URLs, scopes,
impersonated version strings, beta flag strings) behind `pub(crate)` consts —
single module to bump per release.

Note on xAI scope keys: `SubscriptionProvider::Xai` does **not** own a single
static scope string; xAI scopes are built at runtime by
`GrokComConfig::auth_scope()` (`auth/config.rs:219,264`). The helper returns an
enum (`ScopeKey::Dynamic` for Xai, `ScopeKey::Static(&str)` for the new
providers) rather than pretending all three are static.

### 3.2 Storage — extend, don't fork

- `GrokAuth` (`auth/model.rs:48-108`) gains:
  - `provider: SubscriptionProvider` with `#[serde(default)]` → deserializes as
    `Xai` for every existing row (back-compat migration = free).
  - `account_id: Option<String>` (`skip_serializing_if none`) — ChatGPT claim.
  - `subscription_tier: Option<String>` if the provider returns one (display only).
- **Required-field decision for non-xAI rows** (`user_id: String`,
  `auth_mode: AuthMode`): provider rows set `user_id` to the provider account
  identifier when one is available (ChatGPT: `chatgpt_account_id`; Anthropic:
  empty string, since the token carries no stable public id) and add a single new
  `AuthMode::SubscriptionOauth` variant rather than overloading an xAI-semantics
  variant. T2 covers the serde round-trip for both.
- New scope keys written by the new flows: `anthropic::oauth`,
  `openai-codex::oauth`. Existing `{issuer}::{client_id}` and `xai::api_key` keys
  keep working and read back as `Xai`.
- `is_session_auth()` (`auth/model.rs:165`) keeps xAI semantics — it feeds
  `ModelInfo::visible_for_auth` (`agent/config.rs:4324-4326`) and must not start
  returning true for Anthropic/Codex rows. Per-provider questions are answered by
  the new `has_live_subscription(provider)` on the registry (§3.3.7).
- No changes to `storage.rs` atomicity (`storage.rs:311`) or `manager/lock.rs`
  flock (`:338`) — rotation safety rides both unchanged.

### 3.3 Auth flows — new modules beside `oidc/`

```
src/auth/providers/{mod.rs, registry.rs}
src/auth/anthropic/{mod.rs, login.rs, refresh.rs, wire.rs}
src/auth/openai_codex/{mod.rs, login.rs, device.rs, refresh.rs, wire.rs}
src/auth/pkce_loopback.rs   // extracted from oidc/login.rs, generic over provider
```

1. **Extract `pkce_loopback.rs`** from `oidc/login.rs` with an explicit port mode:

   ```rust
   pub enum LoopbackPort {
       Ephemeral,                       // xAI today
       Fixed { port: u16, path: &'static str },  // Anthropic 53692, Codex 1455
   }
   ```
   Anthropic and OpenAI redirect URIs are pre-registered, so ephemeral bind cannot
   work for them. `Fixed` bind failure (`EADDRINUSE`) is **not** fatal: fall back
   to paste-only capture with a message naming the occupied port, since the
   authorize URL is still valid and the user can paste `code#state`. Also extract:
   axum callback route, `webbrowser::open`, callback-vs-paste race
   (`race_callback_and_client_ui` `oidc/login.rs:247` /
   `race_callback_and_stdin` `:301`), callback success page (`:72`), 600 s timeout
   (`:29-30`), paste parsing (full URL / `code#state` / bare code — superset of
   `parse_pasted_input` `:37-69` and shuvpi's `parseAuthorizationInput`
   `anthropic.ts:52-79`).

   **Test-port knob:** do *not* reuse `GROK_LOCAL_AUTH` — it already means "use the
   local dev issuer `http://localhost:22255`" (`auth/config.rs:171-184`) and is set
   by existing PTY e2e tests. Introduce a distinct
   `GROK_OAUTH_LOOPBACK_PORT_OVERRIDE` read only by `pkce_loopback.rs`.

   OIDC flow becomes the first consumer; **no behavioral change** to xAI login. The
   frozen CORS allowlist (`config.rs:137`, test `:384`) is untouched —
   Anthropic/OpenAI callbacks are plain GET redirects with no POST-back, so the
   accounts-app CORS layer doesn't apply.
2. **Anthropic login** (`anthropic/login.rs`): PKCE S256, `state` = verifier,
   authorize params per §1.1, `LoopbackPort::Fixed{53692,"/callback"}` + paste
   race, token exchange (form-urlencoded, `User-Agent: claude-cli/<ver> (external,
   cli)`), build `GrokAuth { provider: Anthropic, auth_mode: SubscriptionOauth,
   key, refresh_token, expires_at (−5 min), … }`, persist under `anthropic::oauth`.
3. **Anthropic refresh** (`anthropic/refresh.rs`): implements `TokenRefresher`
   (`auth/refresh/mod.rs:233-241`); `grant_type=refresh_token` + `client_id`;
   **persist rotated refresh_token in the same locked write as the new access
   token**; missing refresh_token in response → keep old (shuvcode behavior,
   `anthropic-claude-code.ts:115`). 401-mid-request → existing
   `try_recover_unauthorized` (`manager.rs:2546`) → one retry.
4. **OpenAI browser login** (`openai_codex/login.rs`): PKCE S256, random hex
   `state`, params per §1.2, `LoopbackPort::Fixed{1455,"/auth/callback"}` + paste
   race, form-urlencoded exchange, decode JWT → `chatgpt_account_id` using the
   existing decode-only helper (`auth/jwt.rs:12` `insecure_decode`; `jsonwebtoken`
   is already a dep, `xai-grok-shell/Cargo.toml:117`) — no signature validation,
   it's the provider's own token — and fail with an actionable error if the claim
   is missing.
5. **OpenAI device login** (`openai_codex/device.rs`): non-RFC-8628 JSON endpoints
   per §1.2; reuse the *UX shape* of `device_code.rs` (channels, code+URL surface,
   polling interval, `deviceauth_authorization_pending`/`slow_down` handling) but
   new code — the endpoints and the server-supplied `code_verifier` make the
   existing module a poor fit. Surface via `AuthUrlMode::Device` (`flow.rs:169-176`).
6. **Refresher construction**: `build_refresher` (`auth/refresh/mod.rs:243-262`)
   currently keys on `auth_provider_command` presence and returns
   `ExternalBinaryRefresher` or `OidcRefresher`. It gains a preceding arm keyed on
   the manager's provider: `Anthropic` → `AnthropicRefresher`, `OpenaiCodex` →
   `CodexRefresher`, `Xai` → existing behavior unchanged.
7. **Credential registry — one `AuthManager` per provider.** This replaces the
   first draft's "parameterize AuthManager" idea, which the code does not support:
   an `AuthManager` is bound to exactly one scope at construction
   (`manager.rs:309-310`, `scope` field `:167`), its single-flight is one plain
   `Mutex<()>` per instance (`:178`), and it holds one refresher (`:170`).

   ```rust
   // src/auth/providers/registry.rs
   pub struct AuthRegistry {
       managers: HashMap<SubscriptionProvider, Arc<AuthManager>>,
   }
   impl AuthRegistry {
       pub fn manager(&self, p: SubscriptionProvider) -> Option<Arc<AuthManager>>;
       pub fn credential_for(&self, p: SubscriptionProvider) -> Option<GrokAuth>;
       pub fn has_live_subscription(&self, p: SubscriptionProvider) -> bool;
       pub fn start_proactive_refresh_all(&self, cancel: CancellationToken);
   }
   ```
   Consequences, all of which simplify the plan: single-flight is per-scope *by
   construction*; cross-process safety still rides the existing flock
   (`manager/lock.rs:338`); `start_proactive_refresh` (`manager.rs:2615`) stays
   per-manager and idempotent, and T21 is a fan-out loop over the registry;
   `WireValidBearerResolver` needs no new field — it is constructed with the
   correct `Arc<AuthManager>` (§3.4). The xAI manager remains the one created
   today, so every existing call site is unaffected.

### 3.4 Credential resolution & sampler wiring

`resolve_credentials` (`agent/config.rs:4787-4846`) gains one arm, placed after
the model's own credential and before the auth-provider arm:

```
model api_key/env_key > provider subscription credential > [auth_provider helper]
  > session token (xAI only) > XAI_API_KEY (xAI only) > fail-closed
```

- **New arm**: if `model.provider == Some(Anthropic|OpenaiCodex)` → fetch from
  `AuthRegistry::credential_for`; if absent → the model stays visible but
  unselectable, with reason "run `grok login --provider X`".
- **Fail-closed integration (three real sites, not `resolve_credentials`).** Today
  a provider-backed model on a non-xAI base URL is stamped
  `AuthProviderRef::fail_closed` during catalog resolution
  (`agent/config.rs:3568-3580`), which forces the auth-provider arm to return
  `None`. Subscription-provider models must be exempted from that stamp *and*
  classified correctly downstream:
  1. `agent/config.rs:3568-3580` — skip the fail-closed stamp when the entry
     carries a `SubscriptionProvider`; route it to the new arm instead.
  2. `agent/auth_method.rs:390-401` (`session_token_auth_gate`) — add a
     `SubscriptionOauth` classification that is **never** eligible for the xAI
     session token (same disposition as `Byok`).
  3. `agent/config.rs:5094-5096` — session bearer-resolver stamping stays gated on
     `is_xai_api_bearer_url`; provider models get their *own* resolver instead
     (below), never the session one.

  Regression tests mirror the existing ones: `model_providers.rs:335` and
  `config_tests.rs:826` ("session token must not leak"), plus
  `config_tests.rs:468` (`session_resolver_is_not_stamped_onto_third_party_samplers`).
- **Bearer resolver**: `WireValidBearerResolver` (`auth/credential_provider.rs:18`)
  keeps its shape — it wraps an `Arc<AuthManager>`. `sampling_config_for_model`
  fills the `bearer_resolver: None` slot (`config.rs:5209`) with a resolver built
  from `AuthRegistry::manager(provider)`, so per-request bearer and 401 refresh
  hit the right credential. Existing sites (`agent/subagent/mod.rs:748`,
  `sampler_turn.rs:520`) keep passing the xAI manager and are untouched.
- **`chatgpt-account-id`**: implement a `HeaderInjector` (`sampler/config.rs:186-188`)
  that reads the live OpenAI credential and stamps `chatgpt-account-id` plus
  `session-id` / `x-client-request-id`. It runs in `post()` at
  `client.rs:782-784`, after the bearer re-stamp, so token rotation and account id
  stay in sync. Fill the `header_injector: None` slot at `config.rs:5214`.
  (Note: `session-id` and `x-client-request-id` do not exist anywhere in the
  workspace today — the nearest analogues are `x-grok-session-id`/`x-grok-req-id`,
  which are exactly the headers §3.5 suppresses. These are new.)
- Static bits flow through existing data: Anthropic `query_params = {beta="true"}`
  (→ `EndpointTemplate`, `client.rs:711`), `extra_headers` (anthropic-beta, x-app,
  anthropic-dangerous-direct-browser-access), `auth_scheme = Bearer`,
  `api_backend = messages`; Codex `base_url = https://chatgpt.com/backend-api/codex`,
  `api_backend = responses`, `extra_headers` (originator, OpenAI-Beta). The
  impersonated **User-Agent is not an `extra_header`** — see §3.5.

### 3.5 Wire identity — first-party header suppression (blocking precondition)

**Decided (D5): full suppression.** Provider requests present as the impersonated
client and nothing else. This section covers headers; §3.6 covers the body
surfaces (identity block, prompt de-branding, tool shape) that must match for the
simulation to be coherent rather than merely header-deep.

**Problem.** `SamplingClient::new` unconditionally overwrites `User-Agent` at
`client.rs:663-673`, *after* `extra_headers` is applied at `:608-614`. Any
`user-agent` set through provider data is silently clobbered by the grok UA. The
same constructor always inserts `x-grok-client-identifier` (`:649-660`, with a
process default) and conditionally `x-grok-client-version` (`:624-631`),
`x-grok-deployment-id` (`:633-640`), `x-grok-user-id` (`:642-647`). Independently,
`GrokRequestHeaders::apply` (`client.rs:60-77`) stamps `x-grok-conv-id`,
`x-grok-req-id`, `x-grok-model-override`, `x-grok-session-id`, `x-grok-agent-id`,
`x-grok-turn-idx` on every request through all three backend send paths
(`:978`, `:1038`, `:1249` — chat-completions, responses, messages).

Left as-is this (a) leaks grok session/user/deployment identifiers to Anthropic
and OpenAI, and (b) defeats the entire §1.1 contract, since shuvcode documents
that a correctly-authenticated request that *presents* differently is billed as
pay-as-you-go extra usage (`anthropic-claude-code.ts:8-9`).

**Design.** Add to `SamplerConfig` (`sampler/config.rs:48-137`):

```rust
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WireIdentity {
    #[default]
    Grok,          // current behavior, bit-for-bit
    Impersonated,  // third-party subscription backends
}
pub wire_identity: WireIdentity,
```

Under `WireIdentity::Impersonated`:
1. Skip the entire `GrokRequestHeaders::apply` call on all three send paths.
2. Skip `x-grok-client-identifier`, `x-grok-client-version`,
   `x-grok-deployment-id`, `x-grok-user-id` in the constructor.
3. Set the default UA **before** `extra_headers` (or check
   `headers.contains_key(USER_AGENT)` before inserting at `:663-673`) so a
   configured `user-agent` wins. Precedence becomes:
   `extra_headers.user-agent > origin_client UA > fallback UA`.

`sampling_config_for_model` sets `wire_identity` from the resolved model's
provider (`Xai` → `Grok`, otherwise `Impersonated`). The `WireIdentity::Grok`
path must remain byte-identical — that is the regression bar for T6.

**Escape hatch / diagnostics.** Because this removes `x-grok-req-id` for these
providers, request correlation for provider traffic relies on the provider's own
`x-client-request-id` (Codex) / `X-Claude-Code-Session-Id` (Anthropic). T23's
telemetry work must not reintroduce grok headers to satisfy attribution.

**Tests (T6, and again in T14/T20 wire contracts).** Assert **absence** of every
`x-grok-*` header and assert the exact UA string on both provider paths, using
`xai-grok-test-support`'s `LogEntry.headers` (lowercase, arrival order,
`mock_server.rs:46`). Assert presence and exact values on an xAI request in the
same test module.

### 3.6 Wire shaping (body)

1. **Anthropic system identity — injected at wire time, not stored.** The first
   draft prepended a second `ConversationItem::System` at session setup. That
   breaks the single-leading-System-head invariant: `model_switch.rs:309-328`
   replaces *only* the leading System message (`replace_system_head` `:317-320`),
   and `spawn.rs:1046-1049` persists `conversation.first()` as *the* system
   prompt. With an identity block at index 0, a mid-session system-prompt swap
   would overwrite the identity instead of the real prompt, the persisted system
   prompt would become the identity string, and switching Anthropic↔xAI mid-session
   would strand or duplicate it.

   Instead: inject in the Messages builder (`sampling-types/src/conversation/messages.rs`,
   at the `system_blocks` collection point `:168-176`), gated by a new
   `ConversationRequest`-level flag (or a `MessagesShaping` param threaded from
   `SamplerConfig`, matching whichever mechanism §3.6.4 settles on). Result:
   `system_blocks[0]` = Claude Code identity, `[1..]` = the real prompt. Two blocks
   means the `SystemParam::Text` shortcut at `:283-285` is bypassed and
   `SystemParam::Blocks` is used at `:287` — note the shortcut requires
   **len == 1 AND `cache_control.is_none()`**, so a single cached block already
   takes the Blocks path today; the identity insert is additive either way.
   Stored conversation state stays provider-agnostic and model switching is free.
   Gate: Messages backend + Anthropic provider; BYOK Anthropic API keys opt in via
   the provider entry (default on — harmless, keeps parity).
2. **Anthropic system-prompt de-branding.** The identity block alone is not
   enough: block `[1..]` is grok's own system prompt, which names Grok
   repeatedly. Shipping `"You are Claude Code…"` immediately followed by
   `"You are Grok Code…"` is self-contradicting to the model and a trivial
   fingerprint. shuvpi handles this by rewriting the prompt in the OAuth branch of
   `buildParams`: `.replace(shuvpiSystemIdentity, claudeCodeSystemIdentity)` then
   `.replace(/shuvpi/gi, "Claude")`.

   Grok's prompt is assembled from templates via
   `PromptContext::render_with_renderer` (`xai-grok-agent/src/prompt/context.rs:278-309`),
   so we have two options:
   - **(a) Post-render substitution** at the same wire-shaping seam as the
     identity block — a small ordered rewrite table in `wire.rs`
     (`"You are Grok Code…" → Claude Code identity`, then `/grok/i → "Claude"`,
     plus any product nouns the prompt uses). Cheap, mirrors shuvpi, but is a
     blunt regex over prose and will mangle legitimate uses of the word (e.g. a
     user file literally about grok, tool descriptions naming `grok`, or the
     `GROK.md` project-context filename).
   - **(b) A provider-neutral prompt variant** — a `PromptMode`/template
     selection that renders the harness prompt without product branding for
     impersonated providers, so nothing needs rewriting after the fact.
     More work, but deterministic and testable.

   **Recommendation: (b), with (a) as a backstop scrub.** Render neutral, then run
   the substitution table as a safety net and assert in tests that the final
   system payload contains no case-insensitive `grok` outside allowlisted
   contexts. Note the scrub must *not* touch user content, file contents, or tool
   output — only the rendered system prompt. Tracked as D7.
3. **Anthropic tool shape** — three parts, all needed for a coherent payload:
   - **Name casing**: provider-keyed map applied in `turn_base_tool_specs`
     (`sampler_turn.rs:142-149`) — rename outgoing `ToolDefinition.name`
     (serialized at `messages.rs:297`), and reverse-map incoming `tool_calls`
     names before dispatch. Copy shuvpi's shape exactly: forward is a
     case-insensitive lookup against the 17-name `claudeCodeTools` list
     (`anthropic-messages.ts:84-101,106`); reverse matches case-insensitively
     against the *live* session tool list (`fromClaudeCodeName` `:107-115`)
     rather than a static inverse, so custom/MCP tools round-trip unchanged.
   - **History must be renamed too.** shuvpi applies `toClaudeCodeName` not only
     to tool *definitions* but to `tool_use` blocks in replayed assistant history
     (`convertMessages`) and to tool-reference names in tool results
     (`convertToolResult`). If we rename only `tools[]`, then from turn 2 onward
     the history would carry grok-cased `tool_use` names that match nothing in
     `tools[]` — an incoherent payload. The rename must be applied in the Messages
     builder across definitions **and** message history, in one place.
   - **Tool-use id normalization**: Anthropic constrains ids; shuvpi normalizes
     with `[^a-zA-Z0-9_-] → "_"` truncated to 64 (`normalizeToolCallId`). Apply
     the same normalization to outgoing ids and invert consistently on dispatch.
   - Optional fidelity: legacy `input_schema` three-key shape and
     `eager_input_streaming: true` (§1.1). Adopt if T14's live smoke shows any
     schema rejection; otherwise defer.

   Also stamp `X-Claude-Code-Session-Id` (session uuid) via the same
   `HeaderInjector` seam as Codex (§3.4), since it is per-session and must survive
   `WireIdentity::Impersonated` suppression.
4. **Codex `instructions` — moved in the sampler, not the `From` impl.** The first
   draft put a `system_prompt_as_instructions` flag on `SamplerConfig` but honored
   it inside `From<&ConversationRequest> for rs::CreateResponse`
   (`responses.rs:97-164`). That impl has no access to `SamplerConfig` and cannot
   get one without a signature change. Today System items become
   `EasyInputMessage{role: System}` in `input` (`responses.rs:203-209`) and
   `instructions` is hardcoded `None` (`:133`).

   Do it in `apply_response_defaults` (`sampler/client.rs:1192+`), which already
   owns Responses-body policy (`store` `:1214-1216`, `include` `:1219-1222`) and
   *does* see the config: post-conversion, drain System-role messages out of
   `input`, join them, and set `instructions`. Flag lives on `SamplerConfig`
   (`config.rs:48-137`) as `system_prompt_as_instructions: bool`, set by
   `sampling_config_for_model` when provider = OpenaiCodex.
   *(Alternative, if a future refactor prefers it: carry the flag on
   `ConversationRequest`. Pick one — do not split the logic across both.)*
5. **Codex body knobs**:
   - `store:false` ✅ already the default (`client.rs:1214-1216`).
   - `include: reasoning.encrypted_content` ✅ already ensured (`:1219-1222`,
     dedup-guarded).
   - `prompt_cache_key` ✅ mapped (`responses.rs:141-144`) but **no clamp exists
     anywhere in the crate** — add a 64-char clamp there
     (`OPENAI_PROMPT_CACHE_KEY_MAX_LENGTH`, mirroring
     `shuvpi/packages/ai/src/api/openai-prompt-cache.ts:1`).
   - `parallel_tool_calls` — the field **already exists** at `responses.rs:138`
     hardcoded to `None`; this is a wire-up, not an addition. Nothing in the
     workspace currently sends `true`, so it is genuinely new wire behavior and
     must be covered by the T20 contract test.
   - `text.verbosity` and `reasoning.summary:"auto"` — leave to the existing
     reasoning-effort mapping; verify against the live backend during the Phase 3
     smoke.
6. **max_tokens**: Anthropic default is already 128k
   (`ANTHROPIC_DEFAULT_MAX_TOKENS`, `client.rs:46`, applied `:1572-1577`); the
   catalog carries per-model caps.

### 3.7 Model catalogs & gating

- **Row shape.** `default_models.json` rows do *not* deserialize as
  `ModelEntryConfig`; they parse into `DefaultModelJson`
  (`agent/config.rs:3750`, a JSON-only subset with no `base_url`, `auth_scheme`,
  `extra_headers`, or `provider`) and are converted in `default_models()`
  (`:3785-3847`, base_url injected from endpoints `:3813-3814`). So the provider
  catalogs keep the JSON thin (id, model, name, description, context_window,
  max_output_tokens, reasoning flags) and a **per-provider conversion fn** in the
  shell fills `base_url`, `api_backend`, `auth_scheme`, `query_params`,
  `extra_headers`, and `wire_identity` from the `wire.rs` constants. That also
  satisfies the §3.1 goal of one module to bump per release.
- Add embedded `anthropic_models.json` and `openai_codex_models.json` to
  `xai-grok-models` (raw-embed pattern, `xai-grok-models/src/lib.rs:12`).
- New optional field `provider: Option<SubscriptionProvider>` on
  `DefaultModelJson` (serde default `None` = xAI/legacy) → `ModelEntryConfig`
  (`:3849-3966`) → `ModelInfo` → `sampling_config_for_model` / `resolve_credentials`.
  Distinct from the existing TOML-override `model_provider: Option<String>`
  (`:3996`), which stays as-is.
- **Gating.** `ModelInfo::visible_for_auth` (`:4324-4326`) currently reads
  `!hidden && (is_session_auth || supported_in_api)`. Generalize to: a
  subscription-provider model is selectable iff
  `AuthRegistry::has_live_subscription(provider)`, or the user set an explicit
  `api_key` on a `[model.*]` override (which already forces
  `supported_in_api = true`, `:4139-4142`). `is_session_auth()` keeps xAI
  semantics (§3.2). Status line: `Claude (Max) ✓ / ChatGPT (Pro) — not logged in`.
- Remote catalog stays authoritative for xAI; provider JSONs are static
  (subscription backends have no list endpoint).

### 3.8 Login UX

- **CLI**: `grok login` → when >0 alternative providers are enabled in the build,
  show a picker (xAI default first line); `grok login --provider
  <xai|anthropic|openai-codex>` and `--provider openai-codex --device-auth`
  bypass the prompt. Extends the existing flags at
  `xai-grok-pager/src/app/cli.rs:25-46` (`oauth`, `device_auth`, `legacy`,
  `devbox`) and `run_cli_login` (`auth/flow.rs:964`). `grok logout [--provider …]`
  symmetric (default: xAI, matching today). `grok login --api-key` unchanged.
  `AuthStatus` (`cli_models.rs:11-19`) gains per-provider variants for the banner.
- **TUI**: `/login` (`xai-grok-pager/src/slash/commands/login.rs:21-22`) renders
  the same picker; the startup auto-login path (`event_loop.rs:1121-1131`) keeps
  targeting xAI only. Both continue to converge at `dispatch/router.rs:1162`.
  Per-provider waiting states reuse the existing loopback-URL + paste box and the
  device-code view (`views/welcome/mod.rs:996-1000`, `BrowserStatusKind::Device`
  `:1219-1223`, renderer `:1232`). One-time notice on first Anthropic turn:
  subscription usage beyond plan limits bills as token-metered extra usage.
- **ACP**: `x.ai/auth/get_url` (`extensions/auth.rs:21`) gains an optional
  `provider` field; `AuthUrlInfo { url, mode, provider }`. Loopback mode for all
  three; device mode for OpenAI (and xAI as today) via `AuthUrlMode`
  (`flow.rs:169-176`).
- Feature-flag the whole surface behind `grok_build_alt_providers` (default on in
  this fork) so one flip disables everything if a provider changes terms.

### 3.9 Telemetry

Existing auth events get a `provider` dimension; add
`login_started/completed/failed`, `token_refreshed`, `refresh_rotation_failed`
per provider. No tokens/claims in events (account_id only as a stable hash).
**Constraint:** attribution for provider traffic must not reintroduce any
`x-grok-*` header suppressed by §3.5.

---

## 4. Phased tasks

### Phase 0 — decisions (before code)
- [ ] D1 Approve client impersonation (reusing Claude Code / Codex client ids, UA
      strings, system identity) — both references do it; Anthropic billing
      treatment appears to depend on it. Pin versions per release.
- [ ] D2 Anthropic token body: form-urlencoded + `claude-cli` UA (recommended;
      both JSON and form verified working — pick form for max fidelity).
- [ ] D3 Loopback+paste race for Anthropic (recommended) vs code-paste only.
      Note the fixed-port constraint and the paste-only fallback in §3.3.1.
- [ ] D4 Confirm `originator=grok` is acceptable for the Codex authorize call.
      shuvpi makes it a parameter defaulting to its own app name
      (`openai-codex.ts:301`), so it is structurally fine but unverified against
      OpenAI acceptance — carry to the Phase 3 live smoke as a checkpoint.
- [x] **D5 Wire identity policy (§3.5) — DECIDED: full suppression.** Provider
      requests present as the impersonated client and nothing else: no `x-grok-*`
      headers, no grok UA, no grok client identifier. Accepted consequence:
      grok-side request correlation for provider traffic is given up entirely, and
      T23 telemetry must attribute from provider-native ids
      (`x-client-request-id`, `X-Claude-Code-Session-Id`) only. Rationale:
      protecting subscription billing treatment outweighs correlation, and partial
      suppression risks the extra-usage treatment the feature exists to obtain.
- [ ] D6 Anthropic identity injection site: wire-time in the Messages builder
      (recommended, §3.6.1) vs stored conversation item. Recommendation stands on
      the `replace_system_head` invariant (`model_switch.rs:309-328`); revisit
      only if a shaping hook proves impractical.
- [ ] D7 **System-prompt de-branding approach (§3.6.2)** — provider-neutral
      prompt render (recommended) vs post-render regex scrub vs both. Follows
      directly from D5: full shape simulation is incoherent if the prompt body
      still says "Grok". Decide before T11, since it determines whether prompt
      templates need a neutral variant.

### Phase 1 — multi-credential foundation (no new network)
- [ ] T1 `SubscriptionProvider` enum + `providers/mod.rs`; scope-key helper
      (`ScopeKey::{Static,Dynamic}` — xAI scopes are built at runtime,
      `auth/config.rs:219,264`).
- [ ] T2 `GrokAuth` fields (`provider`, `account_id`, `subscription_tier`) +
      serde-default back-compat + `AuthMode::SubscriptionOauth` +
      `user_id` convention per §3.2; round-trip tests over legacy `auth.json`
      fixtures covering every existing scope form.
- [ ] T3 `AuthRegistry` (§3.3.7): one `AuthManager` per provider,
      `credential_for` / `has_live_subscription` / `manager` /
      `start_proactive_refresh_all`. xAI manager construction unchanged. Test that
      each provider gets an independent single-flight (`refresh_lock`,
      `manager.rs:178`) and an independent scope (`:167`).
- [ ] T4 `AuthStatus` extension + picker plumbing types (no UI yet).
- [ ] T5 Bearer-resolver wiring: `sampling_config_for_model` fills
      `bearer_resolver` (`config.rs:5209`) from `AuthRegistry::manager(provider)`;
      existing sites (`subagent/mod.rs:748`, `sampler_turn.rs:520`) keep the xAI
      manager. Unit test per-provider resolution.
- [ ] T6 **`WireIdentity` knob (§3.5)** — `SamplerConfig` field; suppress
      `GrokRequestHeaders` on all three send paths (`client.rs:978,1038,1249`),
      suppress `x-grok-client-identifier/-version/-deployment-id/-user-id`
      (`:624-660`), fix UA precedence at `:663-673`. Tests: `Grok` path
      byte-identical to today; `Impersonated` path asserts absence of every
      `x-grok-*` and the exact configured UA.
**Exit:** `cargo test -p xai-grok-shell auth` and `-p xai-grok-sampler` green;
zero behavior change for xAI (header snapshot test proves it).

### Phase 2 — Anthropic end-to-end
- [ ] T7 Extract `pkce_loopback.rs` from `oidc/login.rs` with
      `LoopbackPort::{Ephemeral,Fixed}` + `EADDRINUSE` → paste-only fallback +
      `GROK_OAUTH_LOOPBACK_PORT_OVERRIDE`; xAI login reroutes through it; full xAI
      login regression (loopback, paste, timeout).
- [ ] T8 `anthropic/{wire,login,refresh}.rs` per §3.3; persist under
      `anthropic::oauth`; `build_refresher` provider arm
      (`auth/refresh/mod.rs:243-262`); rotation-persistence test under contention
      (N concurrent refreshers against a mock token endpoint; exactly one rotation
      wins, store holds the winner's refresh token, keep-old-on-missing verified).
- [ ] T9 `anthropic_models.json` + per-provider conversion fn per §3.7;
      `provider` field through `DefaultModelJson` → `ModelEntryConfig` →
      `ModelInfo`.
- [ ] T10 Fail-closed integration per §3.4: exempt subscription models at
      `agent/config.rs:3568-3580`; `SubscriptionOauth` classification in
      `auth_method.rs:390-401`; confirm `config.rs:5094-5096` still refuses the
      session resolver. Regression tests mirroring `model_providers.rs:335`,
      `config_tests.rs:826`, `config_tests.rs:468`.
- [ ] T11 Sampler shaping: `?beta=true`; provider `extra_headers` including
      **`anthropic-version: 2023-06-01`** (absent from this repo today — §1.1);
      `wire_identity = Impersonated`; identity block at the Messages builder
      (§3.6.1); tool-name casing applied to definitions **and replayed history
      `tool_use` blocks / tool-result references** (§3.6.3); tool-use id
      normalization; `X-Claude-Code-Session-Id` via `HeaderInjector`.
- [ ] T11b System-prompt de-branding per D7 (§3.6.2): neutral prompt render
      and/or substitution table in `wire.rs`; scrub applies to the rendered system
      prompt only, never user content, file contents, or tool output. Test asserts
      the final system payload carries no case-insensitive `grok` outside
      allowlisted contexts, and that a multi-turn conversation stays clean.
- [ ] T12 `resolve_credentials` subscription arm + gating/status line;
      fail-closed error copy ("run `grok login --provider anthropic`").
- [ ] T13 CLI/TUI/ACP login+logout for Anthropic (picker, flags, views).
- [ ] T14 Tests: axum mock (`xai-grok-test-support`, Messages endpoint already
      classified `mock_server.rs:828-846`) asserting the full wire contract — URL
      `?beta=true`; exact header set **including absence of all `x-grok-*`**, the
      `claude-cli` UA, and `anthropic-version`; `system[0]` identity;
      **`system[1]` free of grok branding**; tool casing in `tools[]` **and in
      second-turn history `tool_use` blocks**; normalized tool-use ids; reversed
      names in dispatch; bearer = `sk-ant-oat…`; refresh rotation + 401 retry;
      expiry-margin math. Include a two-turn test — several shape bugs
      (history casing, id normalization) only appear after the first tool call.
**Exit:** live login against a real Claude Max account, one agent turn, token
refresh observed in `auth.json`, logout. Manually confirm the turn is charged to
the subscription, not extra usage.

### Phase 3 — OpenAI/Codex end-to-end
- [ ] T15 `openai_codex/{wire,login,device,refresh}.rs` per §3.3; JWT claim
      extraction via `auth/jwt.rs:12`; `account_id` persisted; tests for both
      capture paths (loopback race, paste) + device (mock usercode/token endpoints
      incl. `deviceauth_authorization_pending`/`slow_down`).
- [ ] T16 `HeaderInjector` for `chatgpt-account-id` + `session-id` +
      `x-client-request-id` (all new headers; §3.4), filling
      `config.rs:5214`; test that it re-stamps after simulated token rotation and
      runs after the bearer re-stamp (`client.rs:782-784`).
- [ ] T17 `system_prompt_as_instructions` on `SamplerConfig` honored in
      `apply_response_defaults` (`client.rs:1192+`, §3.6.3); `prompt_cache_key`
      64-char clamp at `responses.rs:141-144`; `parallel_tool_calls: Some(true)`
      wired at `responses.rs:138`.
- [ ] T18 `openai_codex_models.json` + conversion fn; gating; status line.
- [ ] T19 UX wiring (same surfaces as T13; device-code view for TUI/ACP).
- [ ] T20 Wire-contract tests against mock `/codex/responses` SSE: `store=false`,
      `include`, `instructions` populated with System content **and `input` free of
      System messages**, clamped `prompt_cache_key`, `parallel_tool_calls`,
      `chatgpt-account-id`, `originator`, **absence of all `x-grok-*`**.
**Exit:** live login (browser + device) against a real ChatGPT Pro account, one
agent turn with reasoning models, refresh observed.

### Phase 4 — polish & hardening
- [ ] T21 `AuthRegistry::start_proactive_refresh_all` fan-out (per-manager
      `start_proactive_refresh`, `manager.rs:2615`, already idempotent),
      wake-aware; logout clears per-provider sessions.
- [ ] T22 Feature flag `grok_build_alt_providers` gating registry entries.
- [ ] T23 Telemetry dimension + events (§3.9); **header-egress audit** — assert no
      `x-grok-*`, no UA leak, no oat/JWT fragments in logs for provider traffic
      (`sent_fragment_from_headers` covers attribution only).
- [ ] T24 Docs: user guide section (login, model selection, billing caveat, BYOK
      interplay), custom-models doc cross-link; changelog.
- [ ] T25 Upstream-sync review: confirm all changes additive; diff against next
      SOURCE_REV sync; keep frozen-contract tests green (CORS `config.rs:384`,
      scope format `config.rs:375`).

### Validation gates (whole plan)
- `cargo fmt --check && cargo clippy --workspace && cargo test --workspace`
- Phase 2/3 live smoke: real subscription accounts, one coding turn each,
  billing treatment confirmed.
- Back-compat: pre-change `auth.json` → login status, model list, and xAI turn all
  unchanged; xAI request headers byte-identical (T6 snapshot).

---

## 5. Risks & mitigations

| Risk | Mitigation |
|---|---|
| **Fingerprint leak / billing treatment** — grok headers or UA reach a provider | §3.5 `WireIdentity::Impersonated` (D5: full suppression); T6 asserts absence; T14/T20 re-assert per provider; T23 header-egress audit |
| **Body-level fingerprint** — prompt still says "Grok", history tool names uncased, missing `anthropic-version` | §3.6.2 de-branding (T11b) + §3.6.3 history-wide tool renaming + `anthropic-version` in T11; T14 asserts all three, including a two-turn case |
| De-branding scrub over-reaches into user content | Scrub is scoped to the rendered system prompt only; prefer a neutral prompt render (D7 option b) so the regex is a backstop, not the mechanism; allowlist test |
| Upstream monorepo syncs conflict | New code in new modules (`auth/providers`, `auth/anthropic`, `auth/openai_codex`, `pkce_loopback`); shared-seam edits additive (`GrokAuth` serde defaults, optional catalog field, one new `SamplerConfig` enum with a `#[default]` arm); frozen CORS + scope-format tests untouched |
| Anthropic refresh rotation race | Per-provider `AuthManager` → per-scope single-flight by construction (§3.3.7) + existing flock (`manager/lock.rs:338`); contention test (T8); keep-old-on-missing |
| Provider changes impersonation requirements (beta names, UA sniffing) | Constants isolated in per-provider `wire.rs`; feature flag to disable; version-pin bump per release |
| ChatGPT backend drift (OpenAI-Beta, WS-only modes) | SSE first; constants in `wire.rs`; T20 contract tests make drift visible on bump |
| Fixed loopback port occupied (53692 / 1455) | `LoopbackPort::Fixed` failure degrades to paste-only capture with a message naming the port, rather than failing login (§3.3.1) |
| System-prompt invariant breakage on model switch | Identity injected at wire time, never stored (§3.6.1); `replace_system_head` semantics unchanged; test an Anthropic↔xAI mid-session switch |
| Terms-of-service exposure for xAI (xAI-branded fork surface) | Feature flag default decision at release; document client-id provenance in `THIRD-PARTY-NOTICES` |
| Subscription misuse surprises users | One-time extra-usage notice (Anthropic), status-line plan labels, docs |
| JWT claim shape changes (`chatgpt_account_id`) | Fail login with actionable error; header-override escape hatch (`extra_headers` allows a manual `chatgpt-account-id`) |

## 6. Open decisions (carry from Phase 0)
- D5 is **decided** (full suppression). D1–D4, D6, D7 remain open.
- Whether subscription providers appear in remote settings gating (recommended:
  local-first, flag only).
- Whether `/model` picker groups by provider (recommended: yes, small pager change).
