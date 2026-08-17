# Handoff — next up: expired-provider refresh deadlock

Updated 2026-08-16 (PDT). `main` and tag `v1.0.4` are at `5b66026`, pushed to
`shuv1337/shuvgrok`.

## Where things stand

On top of upstream `eb267fef`:

| Commit | What |
|---|---|
| `ae47de6` | Anthropic + OpenAI Codex subscription providers |
| `c20f6bb` | ShuvGrok rebrand + release pipeline |
| `5b66026` | Release v1.0.4 (plus the three CI fixes below) |

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

## The task: expired provider credentials deadlock

A provider whose access token has expired disappears permanently and cannot
recover on its own.

Reproduced on 2026-08-16: the `anthropic::oauth` token expired 2026-08-15, and
`shuvgrok models` now lists **zero** Claude models while Codex (unexpired)
lists nine. Running a full session does not fix it.

The cycle:

1. `LiveProviders::detect` (`auth/providers/mod.rs`) marks a provider live only
   when `!is_expired(auth)`, so an expired credential hides every model it owns.
2. A hidden model cannot be selected, so nothing ever exercises that provider's
   refresher.
3. `AuthRegistry::start_proactive_refresh_all` — which exists precisely to
   refresh these in the background — **is never called from anywhere.** Confirm
   with `grep -rn start_proactive_refresh_all crates/`.

So the only escape is `shuvgrok login --provider anthropic`, even though a
valid `refresh_token` is sitting in `auth.json`.

Suggested shape of the fix, in order of value:

- Treat *refreshable* as live: a credential with a `refresh_token` should keep
  its models visible even when the access token has lapsed. Absence of a
  credential and expiry of one are different states and should not render the
  same.
- Make a turn against a provider model refresh that provider's token first.
  `WireValidBearerResolver` yields `None` once hard-expired, so the request
  fails rather than refreshing; the custom-provider path already does this via
  `refresh_provider_token_pre_turn` and subscription providers need an
  equivalent.
- Either wire up `start_proactive_refresh_all` or delete it. Dead code that
  looks like it solves this problem is worse than no code.

Do not verify this by waiting for a natural expiry — force it by editing
`expires_at` in a copied `auth.json` and pointing `GROK_HOME` at it.

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
