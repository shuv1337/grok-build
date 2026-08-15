pub mod auto_update;
pub mod version;
mod version_policy;

/// Whether the **background** updater may check for and install a release.
///
/// **Off in this fork.** This is the gate on the startup banner and on the
/// detached installer it spawns. Two reasons:
///
/// 1. This build is normally a local `cargo build`, not a managed npm install.
///    A background job that resolves a published version and swaps it in
///    replaces the binary under test with a different one, silently.
/// 2. It is a network call and a banner nobody asked for, on a fork that
///    ships from source.
///
/// This does **not** disable the explicit path: `shuvgrok update` and the
/// doctor still work, and they now resolve this fork's npm package rather than
/// upstream's, so an explicit upgrade installs this fork.
pub const SELF_UPDATE_ENABLED: bool = false;

pub use auto_update::UpdateStatus;
pub use version::{UpdateConfig, channel_label, channel_name, write_version_cache};
pub use version_policy::enforce_version_policy_or_exit;
