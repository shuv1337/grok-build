# ShuvGrok

ShuvGrok is a fork of [xai-org/grok-build](https://github.com/xai-org/grok-build) —
Grok in your terminal, with a fast, flicker-free CLI built for plans, subagents,
and parallel work.

**[Source](https://github.com/shuv1337/grok-build)** | **[Upstream docs](https://docs.x.ai/build/overview)**

## Install

```bash
npm i -g @shuv1337/shuvgrok
```

The install pulls the matching platform binary via `optionalDependencies` and
links it as `shuvgrok` in `~/.grok/bin` (or `$GROK_HOME/bin`). The command is
`shuvgrok`, so it never collides with an upstream `grok` install in the same
home directory.

## Get Started

```bash
# Launch the interactive TUI
shuvgrok

# Run a single task
shuvgrok -p "Explain this codebase"
```

On first launch, ShuvGrok opens your browser to authenticate. For CI or
headless environments, use an API key from [console.x.ai](https://console.x.ai):

```bash
export XAI_API_KEY="xai-..."
```

## Update

Self-update is disabled in this fork. Upgrade through npm:

```bash
npm i -g @shuv1337/shuvgrok@latest
```

## Supported Platforms

| Platform | Architecture |
|---|---|
| macOS | Apple Silicon (arm64), x86_64 |
| Linux | x86_64, arm64 |
| Windows | x86_64, arm64 |

## Compatibility notes

This fork keeps upstream's compatibility surfaces unchanged: the `~/.grok`
home directory, the `GROK_*` environment variables, and the `x.ai/...` ACP
method names. Only the distribution name and the installed command differ.

## Feedback

Open an issue at
[shuv1337/grok-build](https://github.com/shuv1337/grok-build/issues).
