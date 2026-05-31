# Claude Code

中文文档：[claude-code.zh-CN.md](claude-code.zh-CN.md)

Use Claude Code integration when you want Claude Code lifecycle hooks to submit completion or attention events to the running Agents Router service.

Claude Code hooks are user-defined commands that run at lifecycle points such as `SessionStart`, `UserPromptSubmit`, `Stop`, and `Notification`. Agents Router uses that official hook path and reads the hook JSON Claude Code sends on stdin.

Official Claude Code hook reference: <https://code.claude.com/docs/en/hooks>

## What Agents Router Needs

For structured notifications, Claude Code must run:

```bash
agents-router ingest --source claude_code --format claude_code_hook
```

`ingest` reads the hook payload from stdin and preserves fields Claude Code exposes, including project path, session id, attention message, model, and the last assistant message. Claude Code exposes `model` on `SessionStart`, so Agents Router records that session context and merges it into the later `Stop` signal for the same session. `UserPromptSubmit` records the turn start time, and `Stop` records the completion time, so Agents Router can compute `duration_ms` for notification preferences such as "tasks 5 minutes or longer." Agents Router validates `transcript_path` when Claude Code sends it, but does not forward that local path to providers.

If you only need a simple custom message, Claude Code can run this command instead:

```bash
agents-router emit \
  --source claude_code \
  --title "Claude Code" \
  --body "Claude Code finished a task."
```

`ingest` and `emit` do not send notifications directly. They submit the event to the local service ingress, and the service routes it to your configured providers.

## 1. Set Up the Service

Run:

```bash
agents-router setup
```

Choose:

```text
Claude Code
```

Then choose a provider.

Setup writes the Agents Router config, starts the local service, and ensures the Claude Code hooks in:

```text
~/.claude/settings.json
```

It preserves existing Claude Code settings and hooks.

Providers with a fixed destination send a setup test notification. Feishu/Lark Personal Agent uses
project rooms instead, so setup finishes with QR and `/bind` steps instead of sending a test message.

## 2. Manual Hook Reference

Most users do not need to edit Claude Code settings manually. This section shows what setup writes,
and is useful if you keep custom Claude Code settings.

Use `SessionStart` when you want completion notifications to include the model. Use
`UserPromptSubmit` when you want duration-based notification preferences. Use `Stop` when you want a
notification after Claude finishes responding. Use `Notification` when you want Claude Code
attention prompts to reach your provider too.

Agents Router uses the user-level settings file:

```text
~/.claude/settings.json
```

You can also configure one project manually:

```text
.claude/settings.local.json
```

Example:

```json
{
  "hooks": {
    "SessionStart": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "agents-router ingest --source claude_code --format claude_code_hook"
          }
        ]
      }
    ],
    "UserPromptSubmit": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "agents-router ingest --source claude_code --format claude_code_hook"
          }
        ]
      }
    ],
    "Stop": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "agents-router ingest --source claude_code --format claude_code_hook"
          }
        ]
      }
    ],
    "Notification": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "agents-router ingest --source claude_code --format claude_code_hook"
          }
        ]
      }
    ]
  }
}
```

Keep this command in runtime hook configuration. Do not ask the agent model to run it manually.

When structured hook stdin is not available, use the simple `emit` command shown above.

## 3. Test the Route

After the service is running, test the same ingress path:

```bash
agents-router emit \
  --source claude_code \
  --title "Claude Code" \
  --body "Test notification from Claude Code."
```

If the provider receives this notification, the Agents Router side is working.

If Claude Code itself cannot run on your machine, this manual `emit` test is still the right local validation for Agents Router. It verifies the same local ingress, source adapter, router, and provider path that a Claude Code hook uses.

## If It Fails

Check these first:

- The local service is running:

```bash
agents-router status
```

- Your config includes the `claude_code` source.
- The route includes `claude_code`.
- The hook command uses `--source claude_code`.
- Structured hooks use `agents-router ingest --source claude_code --format claude_code_hook`.
- `agents-router` is available in the shell environment Claude Code uses for hooks.
