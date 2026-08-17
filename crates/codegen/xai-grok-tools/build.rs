//! Build script for bundling ripgrep for the xai-grok-tools crate.
//!
//! - If `GROK_TOOLS_BUNDLE_RG_PATH` is set, always bundle it
//! - Otherwise, only bundle in release builds
use std::env;
use std::fs;
use std::io;
use std::path::PathBuf;

const RG_VER: &str = "15.0.0";
const BFS_VER: &str = "4.1";
const UGREP_VER: &str = "7.7.0";
const FD_VER: &str = "10.4.2";
// fd stopped publishing x86_64-apple-darwin assets after 10.3.0.
const FD_VER_MACOS_X64: &str = "10.3.0";

/// Pinned SHA-256 of each `(version, triple)` fd release tarball we embed.
/// Upstream-published SHA-256 of each ripgrep release tarball, read from the
/// `.tar.gz.sha256` file beside each asset. ripgrep was previously downloaded
/// and embedded with no verification at all, unlike fd.
const RG_TARBALL_SHA256: &[(&str, &str, &str)] = &[
    (
        "15.0.0",
        "x86_64-unknown-linux-musl",
        "253ad0fd5fef0d64cba56c70dccdacc1916d4ed70ad057cc525fcdb0c3bbd2a7",
    ),
    (
        "15.0.0",
        "aarch64-unknown-linux-gnu",
        "15f8cc2fab12d88491c54d49f38589922a9d6a7353c29b0a0856727bcdf80754",
    ),
    (
        "15.0.0",
        "aarch64-apple-darwin",
        "98bb2e61e7277ba0ea72d2ae2592497fd8d2940934a16b122448d302a6637e3b",
    ),
    (
        "15.0.0",
        "x86_64-apple-darwin",
        "44128c733d127ddbda461e01225a68b5f9997cfe7635242a797f645ca674a71a",
    ),
];

const FD_TARBALL_SHA256: &[(&str, &str, &str)] = &[
    (
        "10.4.2",
        "x86_64-unknown-linux-musl",
        "e3257d48e29a6be965187dbd24ce9af564e0fe67b3e73c9bdcd180f4ec11bdde",
    ),
    (
        "10.4.2",
        "aarch64-unknown-linux-musl",
        "f32d3657473fba74e2600babc8db0b93420d51169223b7e8143b2ed55d8fd9e8",
    ),
    (
        "10.4.2",
        "aarch64-apple-darwin",
        "623dc0afc81b92e4d4606b380d7bc91916ba7b97814263e554d50923a39e480a",
    ),
    (
        "10.3.0",
        "x86_64-apple-darwin",
        "50d30f13fe3d5914b14c4fff5abcbd4d0cdab4b855970a6956f4f006c17117a3",
    ),
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    bundle_rg()?;
    // fd is an optional vendored file-search binary backing a feature-gated
    // toolset; skip the download/embed entirely when that feature is off
    // (shipped TUI binaries).
    if env::var_os("CARGO_FEATURE_PI").is_some() {
        bundle_fd()?;
    }
    // bfs/ugrep back the bash-harness find/grep shadows (embedded_search_tools).
    bundle_search_tool("bfs", "BFS", BFS_VER)?;
    bundle_search_tool("ugrep", "UGREP", UGREP_VER)?;
    Ok(())
}

/// Download + embed fd as an optional vendored file-search binary, mirroring
/// the ripgrep bundling
/// (release-only or `GROK_TOOLS_BUNDLE_FD_PATH` override), plus pinned
/// per-asset SHA-256 verification of the downloaded tarball.
fn bundle_fd() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-env-changed=GROK_TOOLS_BUNDLE_FD_PATH");
    println!("cargo:rustc-check-cfg=cfg(bundle_fd)");

    let gen_dir = PathBuf::from(env::var("OUT_DIR")?).join("bundle-fd");
    fs::create_dir_all(&gen_dir)?;

    // The consuming vendor extraction is unix-only — never bundle on
    // Windows targets, mirroring the bfs/ugrep skip.
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "windows" {
        return Ok(());
    }

    let path_override = env::var("GROK_TOOLS_BUNDLE_FD_PATH").ok();
    let is_release = env::var("PROFILE").as_deref() == Ok("release");
    if path_override.is_none() && !is_release {
        return Ok(());
    }

    // Per-target version: macOS x86_64 pins the last release with that asset.
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let (ver, asset_triple) = match (target_os.as_str(), target_arch.as_str()) {
        ("macos", "aarch64") => (FD_VER, "aarch64-apple-darwin"),
        ("macos", "x86_64") => (FD_VER_MACOS_X64, "x86_64-apple-darwin"),
        ("linux", "x86_64") => (FD_VER, "x86_64-unknown-linux-musl"),
        ("linux", "aarch64") => (FD_VER, "aarch64-unknown-linux-musl"),
        _ => {
            if path_override.is_none() {
                return Err(format!(
                    "Unsupported target for fd bundling: {target_os}-{target_arch}. Set GROK_TOOLS_BUNDLE_FD_PATH to a local fd binary for offline or unsupported builds.",
                )
                .into());
            }
            (FD_VER, "override")
        }
    };

    println!("cargo:rustc-cfg=bundle_fd");
    println!("cargo:rustc-env=GROK_TOOLS_FD_VER={ver}");

    if let Some(path) = path_override {
        let dest = gen_dir.join(format!("fd-{ver}-override.bin"));
        println!("cargo:rustc-env=GROK_TOOLS_FD_TARGET=override");
        let _ = fs::remove_file(&dest);
        fs::copy(PathBuf::from(path.clone()), &dest).map_err(|e| {
            format!(
                "Failed copying GROK_TOOLS_BUNDLE_FD_PATH: {e} from path {path} to dest {}",
                dest.display()
            )
        })?;
        return Ok(());
    }

    println!("cargo:rustc-env=GROK_TOOLS_FD_TARGET={asset_triple}");
    let dest = gen_dir.join(format!("fd-{ver}-{asset_triple}.bin"));
    let _ = fs::remove_file(&dest);

    let url = format!(
        "https://github.com/sharkdp/fd/releases/download/v{ver}/fd-v{ver}-{asset_triple}.tar.gz"
    );

    let expected_sha = FD_TARBALL_SHA256
        .iter()
        .find(|(v, t, _)| *v == ver && *t == asset_triple)
        .map(|(_, _, sha)| *sha)
        .ok_or_else(|| format!("No pinned SHA-256 for fd {ver} {asset_triple}"))?;
    let bytes: Vec<u8> = fetch_verified_asset(
        &url,
        &format!("fd-v{ver}-{asset_triple}.tar.gz"),
        expected_sha,
        "fd",
        "GROK_TOOLS_BUNDLE_FD_PATH",
    )?;

    let gz = flate2::read::GzDecoder::new(&bytes[..]);
    let mut ar = tar::Archive::new(gz);
    let mut found = false;
    for entry in ar.entries()? {
        let mut e = entry?;
        let p = e.path()?;
        if p.file_name().is_some_and(|n| n == "fd") {
            let data: Vec<u8> = {
                let mut v = Vec::new();
                io::copy(&mut e, &mut v)?;
                v
            };
            fs::write(&dest, &data)?;
            found = true;
            break;
        }
    }

    if !found {
        return Err(format!(
            "Could not find 'fd' in fd archive {url}. Set GROK_TOOLS_BUNDLE_FD_PATH for offline builds."
        )
        .into());
    }

    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    let mut hasher = sha2::Sha256::new();
    hasher.update(bytes);
    hex_encode(&hasher.finalize())
}

/// Where verified tarballs are kept between builds.
///
/// `OUT_DIR` is the wrong place: it is per-target, and `cargo clean` wipes it,
/// so every clean build re-downloaded every asset. This directory survives
/// both, so the network is a cold-start cost rather than a per-build one.
/// Override with `GROK_TOOLS_ASSET_CACHE`; CI caches this path directly.
fn asset_cache_dir() -> Option<PathBuf> {
    if let Some(dir) = env::var_os("GROK_TOOLS_ASSET_CACHE") {
        return Some(PathBuf::from(dir));
    }
    env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|h| PathBuf::from(h).join(".cargo")))
        .map(|home| home.join("xai-grok-tools-assets"))
}

/// Fetch a release asset, verified against a pinned hash and cached on disk.
///
/// Two problems this solves, which cost a release each:
///
/// 1. **The network was a hard per-build dependency.** A single unauthenticated
///    GET failed a whole release at random — observed as `HTTP 403 Forbidden`
///    on exactly one of six cross-compile targets while the other five fetched
///    the same asset fine. GitHub rate-limits by source IP and CI runners share
///    them, so the odds worsen the more targets build in parallel.
/// 2. **Nothing verified ripgrep.** `fd` was hash-pinned; `rg` was not, so a
///    substituted tarball would have been embedded into shipped binaries.
///
/// The cache is only ever read after its contents hash to `expected_sha`, so a
/// corrupted or tampered entry is re-fetched rather than trusted. Downloads
/// retry 403/408/429/5xx with backoff, send a User-Agent (GitHub is harsher on
/// clients sending none), and use `GITHUB_TOKEN` when present, which raises the
/// rate limit ceiling substantially. A 404 means a wrong version or asset name
/// and fails immediately instead of retrying.
fn fetch_verified_asset(
    url: &str,
    cache_key: &str,
    expected_sha: &str,
    what: &str,
    offline_env: &str,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    const ATTEMPTS: u32 = 4;

    let cached = asset_cache_dir().map(|dir| dir.join(cache_key));
    if let Some(path) = cached.as_ref()
        && let Ok(bytes) = fs::read(path)
    {
        if sha256_hex(&bytes) == expected_sha {
            return Ok(bytes);
        }
        println!("cargo:warning=cached {what} failed verification; re-downloading");
        let _ = fs::remove_file(path);
    }

    let mut builder = reqwest::blocking::Client::builder()
        .user_agent("shuvgrok-build")
        .timeout(std::time::Duration::from_secs(180));
    // Authenticated requests get a far higher rate limit. reqwest drops the
    // Authorization header on the cross-host redirect to the asset CDN, so this
    // never leaks the token off github.com.
    let token = env::var("GITHUB_TOKEN")
        .or_else(|_| env::var("GH_TOKEN"))
        .ok();
    if let Some(ref t) = token
        && !t.trim().is_empty()
    {
        let mut headers = reqwest::header::HeaderMap::new();
        let mut value = reqwest::header::HeaderValue::from_str(&format!("Bearer {t}"))?;
        value.set_sensitive(true);
        headers.insert(reqwest::header::AUTHORIZATION, value);
        builder = builder.default_headers(headers);
    }
    let client = builder.build()?;

    let mut last = String::from("no attempt made");
    for attempt in 1..=ATTEMPTS {
        match client.get(url).send() {
            Ok(resp) if resp.status().is_success() => {
                let bytes = resp.bytes()?.to_vec();
                let actual = sha256_hex(&bytes);
                if actual != expected_sha {
                    return Err(format!(
                        "SHA-256 mismatch for {url}:\n  expected {expected_sha}\n  actual   {actual}"
                    )
                    .into());
                }
                // Cache only verified bytes. A failure to cache is not a build
                // failure — it just costs the next build a download.
                if let Some(path) = cached.as_ref()
                    && let Some(parent) = path.parent()
                    && fs::create_dir_all(parent).is_ok()
                {
                    let _ = fs::write(path, &bytes);
                }
                return Ok(bytes);
            }
            Ok(resp) => {
                let status = resp.status();
                last = format!("HTTP {status}");
                let retryable =
                    status.is_server_error() || matches!(status.as_u16(), 403 | 408 | 429);
                if !retryable {
                    break;
                }
            }
            Err(e) => last = e.to_string(),
        }
        if attempt < ATTEMPTS {
            let backoff = std::time::Duration::from_secs(1 << attempt);
            println!(
                "cargo:warning={what} download attempt {attempt}/{ATTEMPTS} failed ({last}); \
                 retrying in {}s",
                backoff.as_secs()
            );
            std::thread::sleep(backoff);
        }
    }

    Err(format!(
        "{last} downloading {what} after {ATTEMPTS} attempts. \
         Set {offline_env} to a local binary for offline builds."
    )
    .into())
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Bundle a prebuilt **static** search-tool binary (`bfs`/`ugrep`) when
/// `GROK_TOOLS_BUNDLE_<NAME>_PATH` points at one (supplied by the release
/// pipeline). Emits
/// `cfg(bundle_<name>)` so the crate's `include_bytes!` + self-extract engages.
///
/// No auto-download (unlike ripgrep): bfs/ugrep publish no prebuilt static
/// release assets, so the release pipeline supplies the path. Unset → not
/// bundled (the runtime resolver falls back to `~/.grok/vendor` / `$PATH`);
/// never a hard failure, so an un-wired build still succeeds.
fn bundle_search_tool(
    name: &str,
    name_uc: &str,
    ver: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let override_env = format!("GROK_TOOLS_BUNDLE_{name_uc}_PATH");
    println!("cargo:rerun-if-env-changed={override_env}");
    // Always declare the cfg so `#[cfg(bundle_<name>)]` is lint-clean when unset.
    println!("cargo:rustc-check-cfg=cfg(bundle_{name})");

    // The consumer (`embedded_search_tools`) is `#[cfg(unix)]`, so embedding on a
    // Windows target is dead weight — skip (mirrors the ripgrep Windows skip).
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        return Ok(());
    }

    let Some(src) = env::var(&override_env).ok().filter(|s| !s.is_empty()) else {
        return Ok(());
    };

    let gen_dir = PathBuf::from(env::var("OUT_DIR")?).join(format!("bundle-{name}"));
    fs::create_dir_all(&gen_dir)?;
    let dest = gen_dir.join(format!("{name}-{ver}-override.bin"));
    let _ = fs::remove_file(&dest);
    fs::copy(&src, &dest)
        .map_err(|e| format!("copy {override_env} from {src} to {}: {e}", dest.display()))?;

    println!("cargo:rustc-cfg=bundle_{name}");
    println!("cargo:rustc-env=GROK_TOOLS_{name_uc}_VER={ver}");
    println!("cargo:rustc-env=GROK_TOOLS_{name_uc}_TARGET=override");
    Ok(())
}

/// Download + embed ripgrep. Unchanged behavior; split out of `main` so the new
/// search-tool bundling runs regardless of ripgrep's early returns.
fn bundle_rg() -> Result<(), Box<dyn std::error::Error>> {
    // Only bundle in release builds to avoid slowing down cargo check.
    println!("cargo:rerun-if-env-changed=GROK_TOOLS_BUNDLE_RG_PATH");
    // Declare our custom cfg to the compiler so cfg(bundle_rg) is recognized by lints
    println!("cargo:rustc-check-cfg=cfg(bundle_rg)");

    let gen_dir = PathBuf::from(env::var("OUT_DIR")?).join("bundle-rg");
    fs::create_dir_all(&gen_dir)?;

    // Decide whether to bundle: path override OR release build
    let path_override = env::var("GROK_TOOLS_BUNDLE_RG_PATH").ok();
    let is_release = env::var("PROFILE").as_deref() == Ok("release");
    if path_override.is_none() && !is_release {
        return Ok(());
    }

    // Skip auto-bundling on Windows: ripgrep ships .zip on Windows (not
    // .tar.gz) and we have no zip-extraction path. Returning here BEFORE
    // emitting `cargo:rustc-cfg=bundle_rg` keeps include_bytes! macros gated
    // on cfg(bundle_rg) compiled-out, so the runtime falls back to `rg` on
    // PATH. Users install ripgrep separately (winget / scoop). An explicit
    // GROK_TOOLS_BUNDLE_RG_PATH still bundles regardless of target.
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "windows" && path_override.is_none() {
        return Ok(());
    }

    // Expose cfg so the crate can include the bundled bytes.
    println!("cargo:rustc-cfg=bundle_rg");
    println!("cargo:rustc-env=GROK_TOOLS_RG_VER={}", RG_VER);

    // If a local rg binary is provided, copy it directly (skips target check).
    if let Some(path) = path_override {
        let dest = gen_dir.join(format!("rg-{}-override.bin", RG_VER));
        println!("cargo:rustc-env=GROK_TOOLS_RG_TARGET=override");
        let _ = fs::remove_file(&dest);
        fs::copy(PathBuf::from(path.clone()), &dest).map_err(|e| {
            format!(
                "Failed copying GROK_TOOLS_BUNDLE_RG_PATH: {e} from path {path} to dest {}",
                dest.display()
            )
        })?;
        return Ok(());
    }

    // Determine supported ripgrep asset triple for auto-download.
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let asset_triple = match (target_os.as_str(), target_arch.as_str()) {
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("linux", "x86_64") => "x86_64-unknown-linux-musl",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        _ => {
            return Err(format!(
                "Unsupported target for ripgrep bundling: {os}-{arch}. Set GROK_TOOLS_BUNDLE_RG_PATH to a local rg binary for offline or unsupported builds.",
                os = target_os,
                arch = target_arch
            ).into());
        }
    };

    println!("cargo:rustc-env=GROK_TOOLS_RG_TARGET={}", asset_triple);
    let dest = gen_dir.join(format!("rg-{}-{}.bin", RG_VER, asset_triple));
    let _ = fs::remove_file(&dest);

    let url = format!(
        "https://github.com/BurntSushi/ripgrep/releases/download/{v}/ripgrep-{v}-{t}.tar.gz",
        v = RG_VER,
        t = asset_triple
    );

    let expected_sha = RG_TARBALL_SHA256
        .iter()
        .find(|(v, t, _)| *v == RG_VER && *t == asset_triple)
        .map(|(_, _, sha)| *sha)
        .ok_or_else(|| format!("No pinned SHA-256 for ripgrep {RG_VER} {asset_triple}"))?;
    let bytes: Vec<u8> = fetch_verified_asset(
        &url,
        &format!("ripgrep-{RG_VER}-{asset_triple}.tar.gz"),
        expected_sha,
        "ripgrep",
        "GROK_TOOLS_BUNDLE_RG_PATH",
    )?;

    let gz = flate2::read::GzDecoder::new(&bytes[..]);
    let mut ar = tar::Archive::new(gz);
    let mut found = false;
    for entry in ar.entries()? {
        let mut e = entry?;
        let p = e.path()?;
        if p.file_name().is_some_and(|n| n == "rg") {
            let data: Vec<u8> = {
                let mut v = Vec::new();
                io::copy(&mut e, &mut v)?;
                v
            };
            fs::write(&dest, &data)?;
            found = true;
            break;
        }
    }

    if !found {
        return Err(format!(
            "Could not find 'rg' in ripgrep archive {}. Set GROK_TOOLS_BUNDLE_RG_PATH for offline builds.",
            url
        )
        .into());
    }

    Ok(())
}
