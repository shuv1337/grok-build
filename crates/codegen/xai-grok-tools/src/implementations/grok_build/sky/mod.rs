//! First-class Sky Computer Use tools (same namespace as `read_file` / `list_dir`).
//!
//! These spawn the standalone `bin/sky` CLI from `agustif/sky-re` (signed node
//! + SkyComputerUseClient). They are not MCP wrappers: the model sees
//! `list_apps` / `get_app_state` / `click` as GrokBuild tools.

use crate::types::output::ToolOutput;
use crate::types::requirements::{Expr, ToolRequirement};
use crate::types::tool::{ToolKind, ToolNamespace};
use crate::types::tool_io::ToolInput;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

const DEFAULT_TIMEOUT_SECS: u64 = 180;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct SkyOutput {
    pub text: String,
}

impl xai_tool_runtime::ToolOutput for SkyOutput {
    fn model_output(&self) -> Vec<xai_tool_runtime::ContentBlock> {
        vec![xai_tool_runtime::ContentBlock::Text {
            text: self.text.clone(),
        }]
    }
}

impl From<SkyOutput> for ToolOutput {
    fn from(output: SkyOutput) -> Self {
        Self::Text(output.text.into())
    }
}

impl From<String> for SkyOutput {
    fn from(text: String) -> Self {
        Self { text }
    }
}

fn timeout_secs() -> u64 {
    std::env::var("SKY_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|secs| *secs > 0)
        .unwrap_or(DEFAULT_TIMEOUT_SECS)
}

fn sky_bin() -> Result<PathBuf, xai_tool_runtime::ToolError> {
    if let Ok(explicit) = std::env::var("SKY_BIN") {
        let path = PathBuf::from(explicit);
        if path.is_file() {
            return Ok(path);
        }
    }
    if let Ok(root) = std::env::var("SKY_STANDALONE_ROOT").or_else(|_| std::env::var("SKY_ROOT")) {
        let path = Path::new(&root).join("bin/sky");
        if path.is_file() {
            return Ok(path);
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        for candidate in [
            cwd.join("bin/sky"),
            cwd.join("sky-re/bin/sky"),
            cwd.join("../sky-re/bin/sky"),
        ] {
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    if let Ok(path) = which::which("sky") {
        return Ok(path);
    }
    if let Some(home) = dirs::home_dir() {
        for rel in [
            "sky-re/bin/sky",
            "code/sky-re/bin/sky",
            "src/sky-re/bin/sky",
            "Projects/sky-re/bin/sky",
            "dev/sky-re/bin/sky",
        ] {
            let path = home.join(rel);
            if path.is_file() {
                return Ok(path);
            }
        }
    }
    Err(xai_tool_runtime::ToolError::custom(
        "sky_not_found",
        "sky-standalone not found. Clone agustif/sky-re, run examples/setup.sh, set SKY_STANDALONE_ROOT.",
    ))
}

async fn run_sky(args: &[String]) -> Result<String, xai_tool_runtime::ToolError> {
    let bin = sky_bin()?;
    let mut command = Command::new(&bin);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let child = command.spawn().map_err(|error| {
        xai_tool_runtime::ToolError::execution(
            xai_tool_protocol::ToolId::new("sky").expect("id"),
            format!("failed to spawn {}: {error}", bin.display()),
        )
    })?;
    let output = tokio::time::timeout(
        Duration::from_secs(timeout_secs()),
        child.wait_with_output(),
    )
    .await
    .map_err(|_| {
        xai_tool_runtime::ToolError::execution(
            xai_tool_protocol::ToolId::new("sky").expect("id"),
            format!(
                "sky {} timed out after {}s",
                args.first().map(String::as_str).unwrap_or("command"),
                timeout_secs()
            ),
        )
    })?
    .map_err(|error| {
        xai_tool_runtime::ToolError::execution(
            xai_tool_protocol::ToolId::new("sky").expect("id"),
            format!("sky failed: {error}"),
        )
    })?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        let out = String::from_utf8_lossy(&output.stdout);
        return Err(xai_tool_runtime::ToolError::execution(
            xai_tool_protocol::ToolId::new("sky").expect("id"),
            format!("{err}{out}"),
        ));
    }
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr);
    if let Some(line) = stderr.lines().find(|line| line.starts_with("screenshot ")) {
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(line);
        text.push('\n');
    }
    Ok(text)
}

fn push_flag(args: &mut Vec<String>, flag: &str, value: impl ToString) {
    args.push(flag.to_owned());
    args.push(value.to_string());
}

fn push_opt(args: &mut Vec<String>, flag: &str, value: Option<impl ToString>) {
    if let Some(value) = value {
        push_flag(args, flag, value);
    }
}

macro_rules! sky_tool {
    ($Tool:ident, $id:literal, $desc:literal, $kind:expr, $Input:ident, |$input:ident| $body:block) => {
        #[derive(Debug, Default)]
        pub struct $Tool;

        impl From<$Input> for ToolInput {
            fn from(value: $Input) -> Self {
                ToolInput::Dynamic(serde_json::to_value(value).expect("sky tool input serializes"))
            }
        }

        impl crate::types::tool_metadata::ToolMetadata for $Tool {
            fn kind(&self) -> ToolKind {
                $kind
            }
            fn tool_namespace(&self) -> ToolNamespace {
                ToolNamespace::GrokBuild
            }
            fn description_template(&self) -> &str {
                $desc
            }
            fn requires_expr(&self) -> Expr<ToolRequirement> {
                Expr::True
            }
        }

        impl xai_tool_runtime::Tool for $Tool {
            type Args = $Input;
            type Output = SkyOutput;

            fn id(&self) -> xai_tool_protocol::ToolId {
                xai_tool_protocol::ToolId::new($id).expect("valid tool id")
            }

            fn description(
                &self,
                _ctx: &::xai_tool_runtime::ListToolsContext,
            ) -> xai_tool_types::ToolDescription {
                xai_tool_types::ToolDescription::new(
                    $id,
                    crate::types::tool_metadata::ToolMetadata::sanitized_description_template(self),
                )
            }

            fn capabilities(&self) -> xai_tool_protocol::ToolCapabilities {
                xai_tool_protocol::ToolCapabilities {
                    is_read_only: matches!($kind, ToolKind::Read | ToolKind::List),
                    tool_scope: Some(if matches!($kind, ToolKind::Read | ToolKind::List) {
                        xai_tool_protocol::ToolScope::Read
                    } else {
                        xai_tool_protocol::ToolScope::Write
                    }),
                    ..Default::default()
                }
            }

            #[tracing::instrument(name = "tool.sky", skip_all, fields(tool = $id))]
            async fn run(
                &self,
                _ctx: xai_tool_runtime::ToolCallContext,
                $input: $Input,
            ) -> Result<SkyOutput, xai_tool_runtime::ToolError> {
                let _ = &$input;
                $body.map(SkyOutput::from)
            }
        }
    };
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct ListAppsInput {}

sky_tool!(
    ListAppsTool,
    "list_apps",
    "List local macOS apps targetable by Sky Computer Use (running/recent, canonical ids). Does not launch ChatGPT.",
    ToolKind::List,
    ListAppsInput,
    |_input| {
        run_sky(&["list_apps".into()]).await
    }
);

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct GetAppStateInput {
    #[schemars(description = "App display name, .app path, or bundle id from list_apps")]
    pub app: String,
    #[serde(default)]
    #[schemars(description = "Return a full accessibility tree instead of a diff")]
    pub disable_diff: Option<bool>,
}

sky_tool!(
    GetAppStateTool,
    "get_app_state",
    "Capture an app window screenshot and indexed accessibility text. Call before acting and after each action. Do not reuse stale element indexes. Does not launch ChatGPT.",
    ToolKind::Read,
    GetAppStateInput,
    |input| {
        let mut args = vec!["get_app_state".into()];
        push_flag(&mut args, "--app", input.app);
        if input.disable_diff.unwrap_or(false) {
            push_flag(&mut args, "--disableDiff", "true");
        }
        run_sky(&args).await
    }
);

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct ClickInput {
    pub app: String,
    pub element_index: Option<i64>,
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub mouse_button: Option<String>,
    pub click_count: Option<i64>,
}

sky_tool!(
    ClickTool,
    "click",
    "Click an indexed element from the latest get_app_state tree, or a screenshot coordinate.",
    ToolKind::Other,
    ClickInput,
    |input| {
        let mut args = vec!["click".into()];
        push_flag(&mut args, "--app", input.app);
        push_opt(&mut args, "--element_index", input.element_index);
        push_opt(&mut args, "--x", input.x);
        push_opt(&mut args, "--y", input.y);
        push_opt(&mut args, "--mouse_button", input.mouse_button);
        push_opt(&mut args, "--click_count", input.click_count);
        run_sky(&args).await
    }
);

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct DragInput {
    pub app: String,
    pub from_x: f64,
    pub from_y: f64,
    pub to_x: f64,
    pub to_y: f64,
}

sky_tool!(
    DragTool,
    "drag",
    "Drag between two app-window screenshot-relative coordinates.",
    ToolKind::Other,
    DragInput,
    |input| {
        let mut args = vec!["drag".into()];
        push_flag(&mut args, "--app", input.app);
        push_flag(&mut args, "--from_x", input.from_x);
        push_flag(&mut args, "--from_y", input.from_y);
        push_flag(&mut args, "--to_x", input.to_x);
        push_flag(&mut args, "--to_y", input.to_y);
        run_sky(&args).await
    }
);

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct PerformSecondaryActionInput {
    pub app: String,
    pub element_index: i64,
    pub action: String,
}

sky_tool!(
    PerformSecondaryActionTool,
    "perform_secondary_action",
    "Invoke a secondary accessibility action explicitly exposed for an indexed element.",
    ToolKind::Other,
    PerformSecondaryActionInput,
    |input| {
        let mut args = vec!["perform_secondary_action".into()];
        push_flag(&mut args, "--app", input.app);
        push_flag(&mut args, "--element_index", input.element_index);
        push_flag(&mut args, "--action", input.action);
        run_sky(&args).await
    }
);

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct PressKeyInput {
    pub app: String,
    pub key: String,
}

sky_tool!(
    PressKeyTool,
    "press_key",
    "Press a key or + separated X keysym-style chord (Return, Tab, Control_L+a, Super_L+d).",
    ToolKind::Other,
    PressKeyInput,
    |input| {
        let mut args = vec!["press_key".into()];
        push_flag(&mut args, "--app", input.app);
        push_flag(&mut args, "--key", input.key);
        run_sky(&args).await
    }
);

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct ScrollInput {
    pub app: String,
    pub element_index: i64,
    pub direction: String,
    pub pages: Option<f64>,
}

sky_tool!(
    ScrollTool,
    "scroll",
    "Scroll an indexed app element in a direction by a number of pages.",
    ToolKind::Other,
    ScrollInput,
    |input| {
        let mut args = vec!["scroll".into()];
        push_flag(&mut args, "--app", input.app);
        push_flag(&mut args, "--element_index", input.element_index);
        push_flag(&mut args, "--direction", input.direction);
        push_opt(&mut args, "--pages", input.pages);
        run_sky(&args).await
    }
);

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct SelectTextInput {
    pub app: String,
    pub element_index: i64,
    pub text: String,
    pub prefix: Option<String>,
    pub suffix: Option<String>,
    pub selection_type: Option<String>,
}

sky_tool!(
    SelectTextTool,
    "select_text",
    "Select exact text in an indexed editable element or place the cursor before/after it.",
    ToolKind::Other,
    SelectTextInput,
    |input| {
        let mut args = vec!["select_text".into()];
        push_flag(&mut args, "--app", input.app);
        push_flag(&mut args, "--element_index", input.element_index);
        push_flag(&mut args, "--text", input.text);
        push_opt(&mut args, "--prefix", input.prefix);
        push_opt(&mut args, "--suffix", input.suffix);
        push_opt(&mut args, "--selection_type", input.selection_type);
        run_sky(&args).await
    }
);

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct SetValueInput {
    pub app: String,
    pub element_index: i64,
    pub value: String,
}

sky_tool!(
    SetValueTool,
    "set_value",
    "Replace the value of an indexed settable accessibility element.",
    ToolKind::Other,
    SetValueInput,
    |input| {
        let mut args = vec!["set_value".into()];
        push_flag(&mut args, "--app", input.app);
        push_flag(&mut args, "--element_index", input.element_index);
        push_flag(&mut args, "--value", input.value);
        run_sky(&args).await
    }
);

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct TypeTextInput {
    pub app: String,
    pub text: String,
}

sky_tool!(
    TypeTextTool,
    "type_text",
    "Type literal text into the current focus in the specified app.",
    ToolKind::Other,
    TypeTextInput,
    |input| {
        let mut args = vec!["type_text".into()];
        push_flag(&mut args, "--app", input.app);
        push_flag(&mut args, "--text", input.text);
        run_sky(&args).await
    }
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_ids_match_sky_cli() {
        assert_eq!(
            xai_tool_runtime::Tool::id(&ListAppsTool).as_str(),
            "list_apps"
        );
        assert_eq!(
            xai_tool_runtime::Tool::id(&GetAppStateTool).as_str(),
            "get_app_state"
        );
        assert_eq!(xai_tool_runtime::Tool::id(&ClickTool).as_str(), "click");
        assert_eq!(xai_tool_runtime::Tool::id(&DragTool).as_str(), "drag");
        assert_eq!(
            xai_tool_runtime::Tool::id(&PerformSecondaryActionTool).as_str(),
            "perform_secondary_action"
        );
        assert_eq!(
            xai_tool_runtime::Tool::id(&PressKeyTool).as_str(),
            "press_key"
        );
        assert_eq!(xai_tool_runtime::Tool::id(&ScrollTool).as_str(), "scroll");
        assert_eq!(
            xai_tool_runtime::Tool::id(&SelectTextTool).as_str(),
            "select_text"
        );
        assert_eq!(
            xai_tool_runtime::Tool::id(&SetValueTool).as_str(),
            "set_value"
        );
        assert_eq!(
            xai_tool_runtime::Tool::id(&TypeTextTool).as_str(),
            "type_text"
        );
    }

    #[test]
    fn read_tools_are_read_only() {
        assert!(xai_tool_runtime::Tool::capabilities(&ListAppsTool).is_read_only);
        assert!(xai_tool_runtime::Tool::capabilities(&GetAppStateTool).is_read_only);
        assert!(!xai_tool_runtime::Tool::capabilities(&ClickTool).is_read_only);
        assert!(!xai_tool_runtime::Tool::capabilities(&TypeTextTool).is_read_only);
    }

    #[test]
    fn inputs_convert_to_dynamic_tool_input() {
        let input = ToolInput::from(ListAppsInput {});
        match input {
            ToolInput::Dynamic(value) => assert!(value.is_object()),
            other => panic!("expected Dynamic, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn list_apps_runs_via_bin_sky() {
        if sky_bin().is_err() {
            return;
        }
        let resources = crate::types::resources::Resources::new();
        let result = xai_tool_runtime::Tool::run(
            &ListAppsTool,
            crate::types::tool_metadata::test_ctx(resources.into_shared()),
            ListAppsInput {},
        )
        .await
        .expect("list_apps should succeed when bin/sky exists");
        assert!(
            result.text.contains(".app") || result.text.contains("com."),
            "unexpected list_apps output: {}",
            result.text
        );
    }
}
