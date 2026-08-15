# Releasing ShuvGrok

```bash
node scripts/release.mjs patch     # or: minor | major | 1.2.3
```

That bumps `Cargo.toml` and all seven npm manifests in lockstep, runs the fork
boundary check, `cargo fmt --all --check`, and `cargo check --workspace`,
commits `Release vX.Y.Z`, tags, and pushes. Pushing the tag is what starts CI.

`--dry-run` writes the version files and stops before committing.

## What CI does

`.github/workflows/release.yml`, on a `v*` tag:

1. **build** — cross-compiles six targets. Exports `GROK_VERSION=${TAG#v}`;
   without it `option_env!("GROK_VERSION")` is `None`, which the codebase reads
   as "dev build" and uses to auto-trust workspace folders. Do not remove it.
2. **publish-npm** — assembles the platform packages (brotli-compressed; the
   raw binaries exceed npm's tarball limit) and publishes the six platform
   packages *before* the meta package, which pins them by exact version.
   Authenticates by OIDC trusted publishing — there is no npm token.
3. **publish-github-release** — `gh release create --generate-notes` against
   the tag's commit SHA.
4. **notify-discord** — posts the release to the shared webhook.

## One-time setup

Required before the first tag will produce a successful run.

### 1. Publish each package once, manually

npm's trusted publishing is configured *on an existing package*, so it cannot
bootstrap a name that has never been published. Until each of the seven names
exists, CI fails at the OIDC exchange.

Build all six targets, then:

```bash
cd crates/codegen/xai-grok-pager/npm/shuvgrok
GROK_DARWIN_ARM64=… GROK_DARWIN_X64=… GROK_LINUX_X64=… \
GROK_LINUX_ARM64=… GROK_WIN32_X64=… GROK_WIN32_ARM64=… \
  node scripts/assemble-platform-packages.js

cd /home/shuv/repos/grok-build
npm login                                  # account owning the @shuv1337 scope
node scripts/publish-npm.mjs --no-provenance
```

Provenance requires OIDC, so it is unavailable for this local bootstrap run.
Publish order matters and the script already enforces it.

### 2. Configure a trusted publisher for all seven packages

On npmjs.com, per package: **Settings → Trusted publishers → GitHub Actions**.

| Field | Value |
|---|---|
| Organization or user | `shuv1337` |
| Repository | `grok-build` |
| Workflow filename | `release.yml` |
| Environment | `npm-publish` |

All four must match exactly or the exchange is rejected.

### 3. Create the `npm-publish` GitHub environment

**Settings → Environments → New environment → `npm-publish`**. The publish job
declares it, and npm checks the environment name. Add a required reviewer here
if you want publishes gated.

### 4. Add the Discord secret

**Settings → Secrets and variables → Actions → `DISCORD_RELEASE_WEBHOOK_URL`**.

The CI job fails on an empty webhook. The local path
(`scripts/notify-discord-release.mjs`) instead skips silently when the variable
is unset, so a local release without it still succeeds.

### 5. Permissions and allowed actions

- **Settings → Actions → General → Workflow permissions**: the release job
  requests `contents: write`.
- If the repo restricts actions, allow `SethCohen/github-releases-to-discord`,
  `taiki-e/setup-cross-toolchain-action`, and `Swatinem/rust-cache`.

## Not yet exercised

The `aarch64-pc-windows-msvc` and `aarch64-unknown-linux-gnu` matrix legs are
written but have never run. Expect the first release to need a fix there —
Windows-on-ARM cross-links from an x64 image and may need extra MSVC ARM64
components.
