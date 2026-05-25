# agents-router

两三分钟就能 setup 好。
然后你就可以在 Slack、Discord、Telegram、Microsoft Teams、Email、ntfy、Pushover、飞书、Lark、Webhook、WhatsApp 或微信里收到本地 coding agents 的消息。

---

English documentation: [../README.md](../README.md)

[快速开始](#quick-start)

> _想象一下：你的 [Codex Desktop App](https://openai.com/codex/) 在后台工作，你去煮咖啡或者洗衣服，或者暂时离开电脑。_
>
> _任务完成的那一刻，你会收到通知，然后知道现在该回来了。_

⚡ 为 AI coding agents 提供本地优先的通知。

适用于 [Codex Desktop](https://openai.com/codex/)、[Codex CLI](https://github.com/openai/codex)、[Claude Code](https://claude.com/product/claude-code)、GitHub Copilot CLI、Gemini CLI、Aider 这类本地 agent。

使用 Rust 🦀 构建。快速、小巧，并且安静地在后台运行。

```text
你电脑上的 Agent -> Agents Router -> 你的通知渠道
```

不需要云账号。不需要托管后端。不需要额外 dashboard。

## ✅ 支持情况

Agents：

- macOS 和 Windows 上的 [Codex Desktop App](https://openai.com/codex/)
- 在 macOS、Linux 和 Windows 上通过 hooks 接入的 [Codex CLI](https://github.com/openai/codex)
- 在 macOS、Linux 和 Windows 上通过 hooks 接入的 [Claude Code](https://claude.com/product/claude-code)
- 在 macOS、Linux 和 Windows 上通过 hooks 接入的 [GitHub Copilot CLI](https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-command-reference)
- 在 macOS、Linux 和 Windows 上通过 hooks 接入的 [Gemini CLI](https://google-gemini.github.io/gemini-cli/)
- 在 macOS、Linux 和 Windows 上通过 notification command 接入的 [Aider](https://aider.chat/)
- 在 macOS、Linux 和 Windows 上通过 completion wrapper 接入的 [Cursor CLI](https://docs.cursor.com/en/cli/overview)
- 在 macOS、Linux 和 Windows 上通过 plugins 接入的 [OpenCode CLI](https://opencode.ai/)
- 在 macOS、Linux 和 Windows 上通过 plugin hooks 接入的 [OpenClaw](https://docs.openclaw.ai/)
- 在 macOS、Linux 和 Windows 上通过 plugin hooks 接入的 [Hermes Agent CLI](https://hermes-agent.nousresearch.com/docs/user-guide/features/hooks)

Providers（你想在哪里收到通知？）：

- [Slack](https://docs.slack.dev/messaging/sending-messages-using-incoming-webhooks/)
- [Discord](https://docs.discord.com/developers/resources/webhook)
- [Telegram](https://core.telegram.org/bots/api)
- [Microsoft Teams](https://learn.microsoft.com/en-us/microsoftteams/platform/webhooks-and-connectors/how-to/add-incoming-webhook)
- [Email SMTP](https://www.rfc-editor.org/rfc/rfc6409)
- [ntfy](https://ntfy.sh/)
- [Pushover](https://pushover.net/api)
- Feishu/Lark Custom Bot（[飞书](https://open.feishu.cn/document/client-docs/bot-v3/add-custom-bot) / [Lark](https://open.larksuite.com/document/client-docs/bot-v3/add-custom-bot)）
- Feishu/Lark App Bot（[指南](providers/feishu-lark-app-bot.md)）
- Webhook
- [WhatsApp](https://developers.facebook.com/docs/whatsapp)
- 微信（个人微信 iLink）

## 🔒 隐私

Agents Router 在本地运行。

你的数据不会发送到 Agents Router 的云端，因为这个项目没有托管云服务。

通知会直接从你的电脑发到你配置的 provider。

对于 Codex Desktop，它只读取生成通知所需的完成信息：

- 项目名
- 项目路径
- session
- Codex thread 链接
- 运行时长
- 分支
- 时间
- 默认发送最终回答 preview，也可以改成完整回答
- 只有用户明确开启时才发送 prompt
- 电脑名称

在 Feishu/Lark 中，通知会以 Codex 风格的 interactive card 发送，并带有可点击的 Open in Codex 按钮。
这个按钮会先打开一个本地浏览器 URL，然后再交给 Codex Desktop。

<a id="quick-start"></a>

## ⚙️ 安装 - Step 1

安装方式任选一种就够。

推荐方式：

复制到 Terminal 里运行：

```bash
npx --yes --prefer-online agents-router@latest setup
```

如果你更喜欢持久 npm 安装：

```bash
npm install -g agents-router
agents-router setup
```

不用 Node.js/npm 的话：

```bash
curl -fsSL https://raw.githubusercontent.com/lumpinif/agents-router/main/install.sh | sh
agents-router setup
```

Windows PowerShell：

```powershell
irm https://raw.githubusercontent.com/lumpinif/agents-router/main/install.ps1 | iex
agents-router setup
```

之后需要升级时，重新运行第一次使用的同一种安装方式即可。如果本机 service 已经在运行，
安装器会在替换 binary 后重启它，让后台 service 也切到新版本。

从源码安装：

```bash
git clone https://github.com/lumpinif/agents-router.git
cd agents-router
cargo install --path .
agents-router setup
```

## 🚀 设置 - Step 2

```bash
agents-router setup
```

先选择 CLI 语言。默认是英文，也可以选择简体中文。

然后回答 3 个问题：

1. 要监听哪个 agent？
2. 通知要发到哪里？
3. 哪些完成的任务需要发送通知？

然后它会写入配置、启动 service，并发送一条测试通知。

Answer detail、是否包含 prompt、高级项目过滤等设置见 [Setup](setup.zh-CN.md)。

## 🎉 就这样

这个 service 会在本机运行：

- macOS：LaunchAgent
- Linux：systemd user service
- Windows：Task Scheduler

不想继续使用 service 时，运行 `agents-router stop` 关闭。

Provider 设置教程：

- [Slack](providers/slack.zh-CN.md)
- [Discord](providers/discord.zh-CN.md)
- [Telegram](providers/telegram.zh-CN.md)
- [Microsoft Teams](providers/microsoft-teams.zh-CN.md)
- [Email SMTP](providers/email-smtp.zh-CN.md)
- [ntfy](providers/ntfy.zh-CN.md)
- [Pushover](providers/pushover.zh-CN.md)
- [飞书/Lark Custom Bot](providers/feishu-lark-custom-bot.zh-CN.md)
- [飞书/Lark App Bot](providers/feishu-lark-app-bot.md)
- [Webhook](providers/webhook.zh-CN.md)
- [WhatsApp](providers/whatsapp.zh-CN.md)
- [微信](providers/wechat.zh-CN.md)

Agent 设置教程：

- [Codex CLI](agents/codex-cli.zh-CN.md)
- [Claude Code](agents/claude-code.zh-CN.md)
- [GitHub Copilot CLI](agents/github-copilot-cli.zh-CN.md)
- [Gemini CLI](agents/gemini-cli.zh-CN.md)
- [Aider](agents/aider.zh-CN.md)
- [Cursor CLI](agents/cursor-cli.md)
- [OpenCode CLI](agents/opencode-cli.md)
- [OpenClaw](agents/openclaw.md)
- [Hermes Agent CLI](agents/hermes-agent-cli.md)

## 🧹 卸载

一行命令干净卸载：

```bash
npx --yes agents-router uninstall
```

如果你是用全局 npm 安装的，本地清理完成后再删除 npm package：

```bash
agents-router uninstall
npm uninstall -g agents-router
```

## 🧭 命令

```bash
agents-router setup    # 设置或修改 agent/provider
agents-router start    # 启动已有 service
agents-router status   # 查看 service 状态
agents-router safety reset # 清除 delivery safety pause 状态
agents-router stop     # 停止 service
agents-router uninstall # 删除 service、配置、日志和状态
agents-router watch    # 前台 debug worker
```

当当前生效的 config 包含 `codex_cli` 时，Agents Router 会自动安装推荐的 Codex CLI Stop hook。
手动 CLI hooks 可以这样提交事件：

```bash
agents-router ingest --source codex_cli --format codex_cli_stop
```

Codex Desktop 和 Codex CLI 可以同时启用。如果共享的 Codex Stop hook 为一个可识别的
Codex Desktop session 触发，Agents Router 会忽略这次 hook，并由 Desktop watcher 处理
这次完成通知。

Agents Router 也有 delivery safety guard。如果同一条逻辑消息在 10 秒内发给同一个
provider 5 次，这条消息线会被抑制。如果 2 分钟内有 3 条不同消息线触发抑制，
provider delivery 会暂停，直到你运行 `agents-router safety reset`。被抑制的通知是主动丢弃，
不是延迟到之后补发。

```bash
agents-router emit \
  --source claude_code \
  --title "Claude Code" \
  --body "Claude Code finished a task."
```

```bash
agents-router emit \
  --source gemini_cli \
  --title "Gemini CLI" \
  --body "Gemini CLI finished a task."
```

`emit` 只和本地 service 通信。真正发送到 provider 的动作由 service 完成。

## ✨ 示例

```text
Codex Desktop

Project: agents-router
Session: README polish
Open in Codex: codex://threads/019e1049-2d6d-7de2-bcdf-f47346930b71
Duration: 1m 32s
Branch: main
Time: 2026-05-10 01:35:42 +08:00
Session ID: 019e1049-2d6d-7de2-bcdf-f47346930b71

Preview: Updated the README with a clearer setup flow...
```

## 📝 配置

```text
~/.config/agents-router/config.toml
```

大多数用户应该直接使用 `agents-router setup`。
正在运行的 service 会自动加载有效的 config 修改。

## 🧩 核心

```text
Source -> Signal -> Router -> Provider
```

核心保持简单。后续会逐步支持更多 agents 和 providers。

欢迎贡献。💛
