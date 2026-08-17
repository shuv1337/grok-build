# Handoff — ShuvGrok fork state

Updated 2026-08-17 (PDT). `main` is at `623e510`, pushed to `shuv1337/shuvgrok`.
Latest published release is **v1.0.4**; v1.0.5 was tagged and is mid-flight.

## Where things stand

On top of upstream `eb267fef`:

| Commit | What |
|---|---|
| `ae47de6` | Anthropic + OpenAI Codex subscription providers |
| `c20f6bb` | ShuvGrok rebrand + release pipeline |
| `5b66026` | Release v1.0.4 (plus the three CI fixes below) |
| `63cf6bb` | `/login-claude` + `/login-codex`, docs say `shuvgrok` |
| `25b6da2b` | Expired-provider refresh deadlock fixed |
| `623e510` | Retry build-time release-asset downloads |

Read the commit messages first — they carry the reasoning and are not
duplicated here. Then [`FORK.md`](../FORK.md) for the identity/compatibility
boundary, and [`RELEASING.md`](RELEASING.md) for the release mechanics.

Installed locally as `~/.cargo/bin/shuvgrok` → `target/release/xai-grok-pager`.
The old `grok` command was removed on purpose. Reports
`shuvgrok 1.0.3 (c20f6bb)`.

Rebuild: `cargo build --release -p xai-grok-pager-bin --bin xai-grok-pager`
(`build.rs` only re-stamps the commit when it or `.git/HEAD` changes; `touch`
it if the hash looks stale). Tests need `RUST_MIN_STACK=8388608`.

## Done since this doc was written

**v1.0.4 is released.** npm bootstrap, trusted publishing, and the full
pipeline all work; see `docs/RELEASING.md` for the recorded setup. Verified by
installing `@shuv1337/shuvgrok@1.0.4` from npm into a clean prefix: it reports
`shuvgrok 1.0.4 (5b66026)`, decompresses and runs, and lists provider models.
All seven packages carry SLSA provenance.

Three things broke on the way and are fixed in `main`:

1. `protoc` was absent in CI (`bin/protoc` is a DotSlash stub) — now fetched at
   the pinned 29.3 and exported as `$PROTOC`.
2. Proto codegen used `/dev/stdout` and `/dev/null`, which do not exist on
   Windows — both Windows targets failed. Now writes real files under `OUT_DIR`.
   **All six targets build, Windows included.**
3. `release.mjs` ran `git push origin HEAD`, which fails because jj keeps git
   HEAD detached. Now pushes an explicit refspec.

Note npm 12 blocks postinstall scripts by default, so a plain
`npm i -g @shuv1337/shuvgrok` prints a warning and skips `postinstall`. It
still works — the `bin/shuvgrok` trampoline decompresses on first run — but the
warning looks alarming. Worth deciding whether to document it or make the
install script-free.

## Fixed since: the expired-provider deadlock

An expired Claude or Codex access token used to hide that provider's models
permanently — a hidden model cannot be selected, and only selecting one
triggered a refresh. Both halves are fixed:

- Liveness no longer conflates a lapsed *access token* with a lapsed
  *subscription*. A credential holding a refresh token stays live; one without
  is still hidden, because that genuinely needs a new login.
- The pre-turn hook gained an arm for subscription providers, whose credential
  lives in its own `AuthManager` and was invisible to both the session refresh
  and the JWT check.
- `AuthRegistry::start_proactive_refresh_all` is gone. It was never called
  while reading as though background refresh existed.

Verified against real credentials with forged expiry under an isolated
`GROK_HOME`: 0 → 9 Claude and 0 → 8 Codex models, a successful turn on each,
and the refreshed tokens persisted to disk.

## Known-fragile: build-time downloads

`crates/codegen/xai-grok-tools/build.rs` downloads ripgrep and fd from GitHub
releases **during the build**, per target. v1.0.5 failed on one of six targets
with `HTTP 403 Forbidden` while the other five succeeded — GitHub rate-limits by
source IP and CI runners share them. Those downloads now retry with backoff and
a User-Agent, but the dependency itself remains: a release still needs the
network, six times.

If it bites again, the deterministic fix is to fetch both binaries once in a
workflow step and export `GROK_TOOLS_BUNDLE_RG_PATH` / `GROK_TOOLS_BUNDLE_FD_PATH`,
which the build script already honours. That was not done because the asset
triple must match the *target*, not the runner, so the workflow would have to
duplicate the script's asset-selection logic.

Note the bundling only runs when `PROFILE=release`; a debug build silently skips
it, so exercise this path with `cargo build --release -p xai-grok-tools`.

## What is and is not verified

**Verified live** (real accounts, real endpoints): OAuth for both providers;
Claude and Codex turns including multi-turn tool calling; Luna at `max` effort;
the `/usage` quota panel against live Anthropic and ChatGPT data; the `/login`
picker; the fork-boundary check failing correctly when identity drifts.

**Never executed**: the entire release workflow. Specifically unexercised and
most likely to break first:

- `aarch64-pc-windows-msvc` cross-links from an x64 `windows-latest` image and
  may need extra MSVC ARM64 components.
- `aarch64-unknown-linux-gnu` (jemalloc under the cross toolchain).
- The npm publish job end to end. It was smoke-tested locally against six
  *fake* binaries — assemble + `--dry-run` only, nothing published.

**Do not remove `GROK_VERSION` from the build job.** `build.rs` treats its
absence as a dev build, which silently switches folder-trust to auto-trust.
There is an inline comment saying so; keep it.

## Test baseline

`/tmp/opencode/base.txt` holds the 35 known-failing `xai-grok-shell` lib tests
captured from a clean checkout of upstream `eb267fef`, before any of this work.
They are pre-existing and environment-dependent — mostly tests that call
`jsonwebtoken` without `ensure_crypto_provider()`, plus `/tmp` and git-env
tests. Compare set membership against it rather than reading a raw count
(a clean `GROK_HOME` run yields ~31, a subset):

```bash
GROK_HOME=$(mktemp -d) RUST_MIN_STACK=8388608 cargo test -p xai-grok-shell --lib 2>&1 \
  | sed -n '/^failures:$/,/^test result/p' | grep -E "^    [a-z]" | sort -u > /tmp/now.txt
comm -23 /tmp/now.txt /tmp/opencode/base.txt
```

`/tmp` is ephemeral — if `base.txt` is gone, regenerate it by running that same
pipeline in a worktree checked out at `eb267fef`, or just treat any failure not
in the list above as suspect and confirm it reproduces on upstream.

Always pass a temp `GROK_HOME`: several tests read the real `~/.grok` and flip
once you have actually used the CLI (filed as papercut `pc_55a1c80e6299`).
`xai-grok-pager` has one known pre-existing failure
(`dashboard_change_location_to_non_git_clears_worktree_toggle`) and one flaky
test (`thinking_quote_line_selection_excludes_bar_prefix`, papercut
`pc_94aa045a1aa8`) that passes on rerun — re-run before believing a failure.

Gates before any release: `cargo fmt --all --check`, `cargo clippy --workspace`
(0 new lints), and `node scripts/check-fork-boundary.mjs`. The release script
runs the boundary check itself.

## Things a fresh agent will otherwise get wrong

- **There is nothing to merge from upstream.** `upstream/main` == `eb267fef` ==
  our base, no tags. Any "new version available" signal comes from xAI's
  binary release channel, not this repo. Do not go looking for an upstream
  merge.
- **Both providers do expose quota endpoints.** An earlier conclusion that they
  did not was wrong; `~/repos/codex-quota` has working examples and the live
  payload shapes are pinned in the tests in
  `crates/codegen/xai-grok-shell/src/auth/providers/usage.rs`.
- **Do not rename compatibility surfaces** to "finish" the rebrand — `~/.grok`,
  `GROK_*`, `x.ai/...` ACP methods, auth scope keys, `xai-grok-*` crate names.
  `FORK.md` explains each; the boundary check enforces them.
- Reference for the pipeline's shape is `~/repos/shuvpi`
  (`.github/workflows/build-binaries.yml`, `scripts/publish.mjs`).

## Suggested skills

- **`shuv-fork-it`** — owns this work. Its "Validate and publish" and "Install
  locally" branches are exactly the remaining scope.
- **`discord-release-notify`** — for the `DISCORD_RELEASE_WEBHOOK_URL` secret
  and verifying the notify step. The webhook is shared and lives in host env;
  never commit it.
- **`shark`** — ping the user for the npm-console steps, which need a human.
- **`terminal-control`** (`termctrl`) — how the TUI was verified; reuse it
  rather than reasoning about rendering.
- **`no-mistakes`** — if you want a full gate run before tagging.

## Secrets

None in this document or the repo. `DISCORD_RELEASE_WEBHOOK_URL` is a GitHub
Actions secret plus host env. npm publishing uses OIDC — there is deliberately
no npm token anywhere. Note that live OAuth access tokens sit in `~/.grok/auth.json`;
do not print that file or dump a full `SamplerConfig` at debug level, which
includes the bearer.
