# Handoff — next up: first ShuvGrok release + npm setup

Written 2026-08-15 (PDT). Current `main` is `c20f6bb`, pushed to
`shuv1337/grok-build`. Working copy is clean.

## Where things stand

Two commits sit on top of upstream `eb267fef`:

| Commit | What |
|---|---|
| `ae47de6` | Anthropic + OpenAI Codex subscription providers |
| `c20f6bb` | ShuvGrok rebrand + release pipeline |

Read the commit messages first — they carry the reasoning and are not
duplicated here. Then [`FORK.md`](../FORK.md) for the identity/compatibility
boundary, and [`RELEASING.md`](RELEASING.md) for the release mechanics.

Installed locally as `~/.cargo/bin/shuvgrok` → `target/release/xai-grok-pager`.
The old `grok` command was removed on purpose. Reports
`shuvgrok 1.0.3 (c20f6bb)`.

Rebuild: `cargo build --release -p xai-grok-pager-bin --bin xai-grok-pager`
(`build.rs` only re-stamps the commit when it or `.git/HEAD` changes; `touch`
it if the hash looks stale). Tests need `RUST_MIN_STACK=8388608`.

## The task

Cut the first real release. It cannot work yet — see the blocker below.

### Blocker: npm trusted publishing cannot bootstrap a package

Verified 2026-08-15:

- The `@shuv1337` scope exists (`@shuv1337/shuvpi-coding-agent` is published).
- None of the seven ShuvGrok package names exist yet.
- This machine is **not** logged into npm (`npm whoami` → 401).

npm's trusted-publisher config is set *on an existing package*, so CI's OIDC
exchange fails until each name has been published once. The one-time manual
bootstrap and all the out-of-repo console steps are written up in
[`RELEASING.md` § One-time setup](RELEASING.md#one-time-setup). Follow that
rather than improvising; the ordering matters (six platform packages before the
meta package, because the meta pins them by exact version).

`scripts/publish-npm.mjs` takes `--dry-run` and `--no-provenance`. Provenance
needs CI's OIDC token, so the local bootstrap run must pass `--no-provenance`.

### Then

```bash
node scripts/release.mjs patch     # bumps Cargo.toml + all 7 manifests, tags, pushes
```

The tag push starts `.github/workflows/release.yml`.

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
