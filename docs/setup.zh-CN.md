# Setup

English documentation: [setup.md](setup.md)

用 setup 创建或替换本机配置并启动 service。带固定目标群的 provider 会发送一条测试通知。Feishu/Lark Personal Agent 使用 project rooms，所以 setup 会以 room binding 指引结束，不发送测试消息。

```bash
agents-router setup
```

如果还没有配置，setup 会显示推荐默认值。如果已经有配置，setup 会用 `Current` 显示当前答案，直接按 Enter 会保留当前值。
Webhook URL 只显示 host，签名 secret 和私有 provider key 只显示已配置状态，不会把完整敏感内容打印到终端里。

如果要清空飞书/Lark Custom Bot 签名 secret，输入 `none`。

## 语言

Setup 会先问 CLI 语言：

```text
1. English
2. 简体中文
```

默认是 English。选择后会写入 config：

```toml
[cli]
language = "zh-CN"
```

如果要改回英文，使用 `language = "en"`。
也可以在运行 setup 前设置 `AGENTS_ROUTER_LANGUAGE=zh-CN`，这样中文会成为默认选择。
Setup 的问题和完成提示会使用你选择的语言。

## Agent

选择 Agents Router 要监听哪个 agent：

```text
1. Codex Desktop
2. Codex CLI
3. Claude Code
4. Cursor CLI
5. OpenCode CLI
6. OpenClaw
7. Hermes Agent CLI
8. GitHub Copilot CLI
9. Gemini CLI
10. Aider
```

Codex Desktop 当前在 macOS 和 Windows 上提供。Linux 上，setup 会从 Codex CLI 开始，提供这些 hook-based CLI sources。

当当前生效的 config 包含 Codex CLI 时，Agents Router 会在 setup、start、watch 和成功的
hot reload 里确保 `~/.codex/config.toml` 有 Stop hook。它不会改已有的 Codex CLI
`notify` 命令，因为 `notify` 是单个命令槽位，可能已经属于其他本地集成。手动 config
必须使用 `id = "codex_cli"`。手动 hook 和 `notify` fallback 的细节见
[Codex CLI](agents/codex-cli.zh-CN.md)。

Codex Desktop 和 Codex CLI 可以同时启用。因为它们可能共享同一份 Codex config，
Desktop 也可能触发 CLI Stop hook。Agents Router 会把 Codex Desktop 作为 Desktop
session 的权威来源，只忽略能够明确证明属于 Codex Desktop 的 Stop hook payload。
无法确认来源的 session 仍然会走 `codex_cli`。

当当前生效的 config 包含 Claude Code 时，Agents Router 会在 setup、start、watch 和成功的
hot reload 里确保 `~/.claude/settings.json` 有所需 hooks。它会添加 `SessionStart`、
`UserPromptSubmit`、`Stop` 和 `Notification` hooks，并保留已有 Claude Code settings。
手动 config 必须使用 `id = "claude_code"`。hook 结构见
[Claude Code](agents/claude-code.zh-CN.md)。

## Provider

选择通知要发到哪里：

```text
1. Slack
2. Discord
3. Telegram
4. Microsoft Teams
5. Email SMTP
6. ntfy
7. Pushover
8. Feishu/Lark
9. Webhook
10. WhatsApp
11. 微信
```

如果还没有已配置 provider，setup 会默认推荐 Feishu/Lark。

Provider 教程：

- [飞书/Lark Personal Agent](providers/feishu-lark-app-bot.md)
- [Slack](providers/slack.zh-CN.md)
- [Discord](providers/discord.zh-CN.md)
- [Telegram](providers/telegram.zh-CN.md)
- [Microsoft Teams](providers/microsoft-teams.zh-CN.md)
- [Email SMTP](providers/email-smtp.zh-CN.md)
- [ntfy](providers/ntfy.zh-CN.md)
- [Pushover](providers/pushover.zh-CN.md)
- [飞书/Lark Custom Bot](providers/feishu-lark-custom-bot.zh-CN.md)
- [Webhook](providers/webhook.zh-CN.md)
- [WhatsApp](providers/whatsapp.zh-CN.md)
- [微信](providers/wechat.zh-CN.md)

选择 Feishu/Lark 时，setup 会继续询问 mode：

```text
Personal Agent App — Experimental
  扫码创建 Personal Agent。私聊用 `/new` 开 Codex 任务，用 `/bind` 选择项目群。

已有自建应用 — 高级
  使用已有自建应用连接一个固定群。高级配置；默认推荐 Personal Agent。Codex Desktop thread 回复仍是实验性。

群 Webhook — 单向备用
  单向发送通知到一个群。不支持回复或项目群。
```

Personal Agent App 是默认推荐选项，因为它支持双向 project room 体验：扫码 setup、私聊 `/new /absolute/project/path ...` 开新 Codex thread、私聊 `/bind /absolute/project/path` 打开项目群选择菜单、项目群 `/bind` 直接连接当前群、一个 Codex thread 对应一个 Lark thread。setup 会显示 QR code，扫码创建 Personal Agent app，并在完成后提示你私聊可以用 `/new /absolute/project/path what you want Codex to do` 开新 Codex thread；也可以在私聊里发送 `/bind /absolute/project/path`，选择建新群、使用已有群，或者忽略这个项目。项目群里则把 Personal Agent 拉进群，@ 它并发送 `/bind /absolute/project/path`。如果之后 Codex 从一个还没连接过的项目产生新更新，Agents Router 会在私聊里给出同一套选择。

如果你已经有自建应用并且明确想连接一个固定群，请选择“已有自建应用”。这个模式需要 Bot 能力、包含 `im:chat:create` 和 `cardkit:card:write` 的消息、群和卡片权限、`im.message.receive_v1`、`im.chat.member.bot.added_v1`、`card.action.trigger`、Long Connection / WebSocket、Tenant Key、Room Chat ID，以及已发布的新应用版本。如果你只想要一个简单的单向 fallback，请选择“群 Webhook”。

## Provider ID

Setup 默认使用 provider type 作为 provider id。例如默认的 Slack provider 是：

```toml
[[providers]]
id = "slack"
type = "slack"
```

Route 引用的是 provider id：

```toml
[[routes]]
sources = ["codex_desktop"]
providers = ["slack"]
```

如果需要两个同类型 provider，就给每个 provider 一个清晰且唯一的 id：

```toml
[[providers]]
id = "slack_engineering"
type = "slack"

[[providers]]
id = "slack_personal"
type = "slack"
```

## 通知偏好

选择哪些完成的任务需要发送通知：

```text
1. Every completed task (Recommended)
2. Tasks 5 minutes or longer
3. Custom minimum duration
```

直接按 Enter 会使用 `Every completed task`。

默认行为：

- 如果没有设置 `minimum_task_duration_minutes`，完成的任务不会按耗时过滤。
- 选择 `Every completed task` 时，不会写入 `minimum_task_duration_minutes` 字段。
- 选择 `Tasks 5 minutes or longer` 时，会写入 `minimum_task_duration_minutes = 5`。
- 选择 `Custom minimum duration` 时，会写入你输入的正整数分钟数。
- 如果某条 route 设置了 `minimum_task_duration_minutes`，但某个 Signal 没有 `lifecycle.duration_ms`，这条 route 仍然会匹配。Agents Router 只会过滤已知耗时且低于阈值的任务，避免因为耗时缺失而静默漏掉完成通知。
- setup 只会在所选集成能可靠提供任务耗时时询问这个偏好，目前包括 Codex Desktop，以及同时配置了 `UserPromptSubmit` 和 `Stop` hooks 的 Claude Code。如果你用 wrapper 接入 agent，需要手动配置 route，并通过 `agents-router emit --duration-ms` 或 structured hook 的 `lifecycle.duration_ms` 字段传入真实耗时。

手动配置：

```toml
[[routes]]
sources = ["codex_desktop"]
providers = ["slack"]
minimum_task_duration_minutes = 5

[[routes]]
sources = ["agents_router"]
providers = ["slack"]
```

Wrapper 示例：

```bash
agents-router emit \
  --source aider \
  --title "Aider" \
  --body "Aider finished a task." \
  --duration-ms 420000
```

`agents_router` route 会故意保持独立且不加过滤。setup 测试通知使用这个内部 source，
所以即使真实 agent 通知只转发长时间任务，测试通知仍然能验证你的 provider 是否可用。

如果同一条 route 同时设置了 `minimum_task_duration_minutes` 和
`only_forward_from_project_paths`，两个条件都满足后才会发送通知。

Feishu/Lark Personal Agent 的 project room 不在 setup 里使用
`only_forward_from_project_paths`。这个模式下，`/bind /absolute/project/path` 只负责把项目连接到群。
在群里发送 `/bind /absolute/project/path` 会把项目连接到当前群。私聊开新 Codex thread 时，用 `/new /absolute/project/path what you want Codex to do` 明确 working directory；私聊发送 `/bind /absolute/project/path` 会打开这个项目的项目群选择菜单。

## Answer Detail

选择通知里包含多少回答内容：

```text
1. Preview (Recommended)
2. Full Answer
```

直接按 Enter 会使用 `Preview`。
Full Answer 会包含用户能看到的 assistant 回答，并忽略 Codex App 控制指令。

Answer detail 只对没有小型消息长度限制或本地发送保护线的 provider 开放。

Agents Router 会对这些 provider 固定使用 `Preview`：

- ntfy，因为 ntfy 有可配置的 message body size limit，默认是 4096 个字符。
- Pushover，因为 Pushover message 最多 1024 个字符。
- Slack，因为 Agents Router 对 Slack 文本使用 4000 字符本地保护线。
- Discord，因为 Discord webhook content 最多 2000 个字符。
- Telegram，因为 Telegram Bot API text message 最多 4096 个字符。
- WhatsApp，因为 Agents Router 对 WhatsApp text body 使用 4096 字符本地保护线。
- 微信，因为 Agents Router 对 微信 iLink text message 使用 3800 字符本地保护线。
- Microsoft Teams，因为 Teams webhook message 有官方文档记录的 28 KB 大小限制。

## Prompt Detail

选择通知里是否包含你发给 agent 的原始 prompt：

```text
1. No (Recommended)
2. Yes
```

直接按 Enter 会使用 `No`。Prompt detail 默认关闭，因为 prompt 里可能包含私有需求、代码、日志、路径或 secret。
对于 Codex Desktop，prompt 来自 Codex 本地 `user_message` 记录。这个记录可能包含 IDE context，例如 active file 和 open tabs。
如果某个 source 没有提供 prompt，通知里就不会显示 Prompt 区块。

手动配置：

```toml
[notification]
answer_detail = "preview"
prompt_detail = "off"
```

Prompt detail 只对没有小型消息长度限制或本地发送保护线的 provider 开放。

Agents Router 会对这些 provider 禁用 prompt detail：

- ntfy，因为 ntfy 有可配置的 message body size limit，默认是 4096 个字符。
- Pushover，因为 Pushover message 最多 1024 个字符。
- Slack，因为 Agents Router 对 Slack 文本使用 4000 字符本地保护线。
- Discord，因为 Discord webhook content 最多 2000 个字符。
- Telegram，因为 Telegram Bot API text message 最多 4096 个字符。
- WhatsApp，因为 Agents Router 对 WhatsApp text body 使用 4096 字符本地保护线。
- 微信，因为 Agents Router 对 微信 iLink text message 使用 3800 字符本地保护线。
- Microsoft Teams，因为 Teams webhook message 有官方文档记录的 28 KB 大小限制。

如果要包含 prompt：

```toml
[notification]
prompt_detail = "on"
```

如果要发送完整回答：

```toml
[notification]
answer_detail = "full"
```

如果要同时发送完整回答并包含 prompt：

```toml
[notification]
answer_detail = "full"
prompt_detail = "on"
```

正在运行的 service 会自动加载有效的 config 修改。如果 service 没有运行，启动它：

```bash
agents-router start
```

如果手动修改后的 config 无效，正在运行的 service 会继续使用上一份有效 config，并在日志里记录 reload 失败。

## 高级项目过滤

如果只想转发具体项目下的通知，可以手动把项目路径加到真实 agent 的 route 上：

```toml
[[routes]]
sources = ["codex_desktop"]
providers = ["slack"]
only_forward_from_project_paths = [
  "/Users/felix/Desktop/felix-projects/agents-router",
  "/Users/felix/Desktop/felix-projects/another-project",
]

[[routes]]
sources = ["agents_router"]
providers = ["slack"]
```

设置这个过滤后，只有当 Signal 的 `workspace.project_path` 等于这些路径之一，或位于这些路径之下时，
Agents Router 才会转发。项目路径必须是干净的绝对路径。如果某个 source 没有提供
`workspace.project_path`，带项目过滤的 route 就不会匹配。

默认行为：

- 如果没有设置 `only_forward_from_project_paths`，或它是空数组，通知不会按项目路径过滤。
- Setup 不会询问这个选项。需要只转发指定项目时，手动把它加到真实 agent 的 route 上。
- Feishu/Lark Personal Agent setup 会清空所选 agent route 上的这个过滤。私聊 `/bind /absolute/project/path` 可以选择项目群，群里 `/bind /absolute/project/path` 会连接当前群。
- 这个值是数组，所以同一条 route 可以允许多个项目路径。
- 路径必须是非空绝对路径，不能包含 `.` 或 `..` 路径组件。
- 匹配使用路径组件语义，不是字符串前缀。例如 `/Users/me/app` 会匹配 `/Users/me/app/api`，但不会匹配 `/Users/me/app-copy`。
- 如果同一条 route 同时设置了 `only_forward_from_project_paths` 和 `minimum_task_duration_minutes`，两个条件都必须满足。

正在运行的 service 会自动加载有效的 config 修改。如果 service 没有运行，启动它：

```bash
agents-router start
```

如果手动修改后的 config 无效，正在运行的 service 会继续使用上一份有效 config，并在日志里记录 reload 失败。

## 结果

Setup 会写入：

```text
~/.config/agents-router/config.toml
```

然后它会启动本机 service。带固定目标的 provider 会通过同一条 provider 投递链路发送测试通知。Feishu/Lark Personal Agent 使用 project rooms，所以 setup 会以 QR 和 `/bind` 步骤结束，不发送测试消息。
macOS 使用 LaunchAgent，Linux 使用 systemd user service，Windows 使用 Task Scheduler。
