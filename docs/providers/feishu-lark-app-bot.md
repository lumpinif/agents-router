# Feishu/Lark Personal Agent

Use this when you want Feishu or Lark to become a two-way control surface for local Codex Desktop work. Codex Desktop thread replies are Experimental.

The default setup is:

```text
agents-router setup
-> Feishu/Lark
-> Personal Agent App
-> scan the QR code
-> direct chat: /bind /absolute/project/path, then send any message
-> project room: add the Personal Agent, mention it, then send /bind /absolute/project/path
```

Direct chat is for starting new Codex threads from Lark. Set a default project with `/bind /absolute/project/path`, then send any plain message to start a new Codex thread in that project.

Project rooms are for project updates and team control. After `/bind`, new Codex Desktop updates from that project land in the bound room. One Codex thread maps to one Feishu/Lark thread. In shared rooms and threads, mention the Personal Agent to continue the same Codex thread through the Experimental reply path. In a one-on-one chat with the Personal Agent, no mention is needed.

There is no history backfill. Old Codex sessions and old Feishu/Lark messages are not copied into Lark. Agents Router only sends new updates after the room is connected.

Use [Feishu/Lark Custom Bot](feishu-lark-custom-bot.md) only when you want one-way notifications and do not need replies or project rooms.

This guide uses Long Connection / WebSocket. You do not need a public webhook URL.

## Official Links

- Lark Developer Console: <https://open.larksuite.com/app>
- Feishu Developer Console: <https://open.feishu.cn/app>
- Lark long connection guide: <https://open.larksuite.com/document/ukTMukTMukTM/uYDNxYjL2QTM24iN0EjN/event-subscription-configure-/use-websocket>
- Lark send message API: <https://open.larksuite.com/document/uAjLw4CM/ukTMukTMukTM/reference/im-v1/message/create>

## Default Setup

Run:

```bash
agents-router setup
```

Choose:

```text
Feishu/Lark
Personal Agent App
```

Setup shows a QR code in your terminal. Scan it with Feishu or Lark, then finish creating the Personal Agent app on the page that opens. Keep the terminal open after scanning; setup continues automatically after the app is created. Agents Router then stores the Personal Agent app credentials in your local config.

Then choose how you want to use it.

For direct chat:

1. Open a one-on-one chat with the Personal Agent.
2. Send:

```text
/bind /Users/you/path/to/project
```

3. After that, send any plain message to start a new Codex thread in that project.

For a project room:

1. Create or open the room you want to use for a project.
2. Add the Personal Agent to that room.
3. Mention the Personal Agent and send this as a normal room message:

```text
@Agents Router /bind /Users/you/path/to/project
```

Use a real absolute folder path on this computer. The first version does not scan your projects or show a project picker.

After `/bind`, new updates from that project are sent to that room. If a new Codex Desktop thread creates an update, Agents Router creates or reuses the matching Feishu/Lark thread for that Codex thread. Mention the Personal Agent in that Feishu/Lark thread to continue the same Codex thread.

## Commands

In shared rooms and threads, mention the Personal Agent before each command. In a one-on-one chat with the Personal Agent, no mention is needed.

Direct chat:

```text
/help
/status
/bind /absolute/project/path
/new what you want Codex to do
/new /absolute/project/path what you want Codex to do
/unbind
```

Room:

```text
/help
/status
/bind /absolute/project/path
/new what you want Codex to do
/unbind /absolute/project/path
/unbind
```

Use `/help` to list the commands. Use `/status` in direct chat to see the default project. Use `/status` in a project room to see which local projects are connected to that room. Use `/unbind` in direct chat to clear the default project. Use `/unbind /absolute/project/path` in a room to disconnect one project from the room, or `/unbind` to disconnect all projects from that room.

## What Setup Writes

Personal Agent setup writes Feishu/Lark app credentials without choosing one fixed room:

```toml
[[providers]]
id = "feishu_lark"
type = "feishu_lark"
mode = "app_bot"
domain = "lark"
app_id = "cli_..."
app_secret = "..."
app_registration_source = "agents-router"
```

That is intentional. Direct chat default projects and project rooms are selected by `/bind`, not by a single global `chat_id`.
`mode = "app_bot"` is the internal Feishu/Lark API mode. It does not mean you chose the advanced fixed-room setup.
The `app_registration_source` line records which local Agents Router runtime created the Personal Agent. It is local metadata, not proof that Feishu/Lark events are arriving. The real check is sending a command in direct chat or in a project room.

Do not paste App Secret into chat, docs, screenshots, or issue reports. Setup stores it only in your local Agents Router config.

Old notifications, setup test messages, and messages outside a bound thread are not continuation surfaces.

## Existing Self-built App

Use this mode only when you already manage a self-built app and want one fixed room.

Choose:

```text
Feishu/Lark
Existing Self-built App
```

You need:

- A Custom App / self-built app.
- Bot capability enabled.
- App ID.
- App Secret from the app console.
- Tenant Key.
- Room Chat ID.
- The bot added to that room.

Enable these permissions:

```text
im:message:send_as_bot
im:message.group_msg:readonly
im:chat:readonly
```

Subscribe to:

```text
im.message.receive_v1
```

Choose:

```text
Long Connection / WebSocket
```

Publish a new app version after changing Bot capability, permissions, or event subscriptions. If your workspace requires admin review, wait until the version is approved.

Then run setup and enter:

```text
domain: lark
App ID: cli_...
App Secret: paste from the app console
Tenant Key: ...
Room Chat ID: oc_...
```

Existing Self-built App can send a setup test message because it has one fixed room. That test message only confirms send permission. It is not a Codex continuation surface.

## Troubleshooting

Check these first:

- The Personal Agent was added to the room.
- `/bind` used an absolute folder path on this computer.
- The local Agents Router service is running.
- `agents-router status` shows the Personal Agent and the expected project room binding.
- The project path in `/bind` matches the project that produced the Codex Desktop update.
- In shared rooms and threads, the Feishu/Lark reply mentions the Personal Agent.
- The Feishu/Lark reply is inside the Agents Router thread, not a new root message.
- App Secret is present in the local config, or `app_secret_env` is configured manually.

For Existing Self-built App, also check:

- The app version was published after permission or event changes.
- Admin approval is complete, if required by your workspace.
- Bot capability is enabled.
- The bot is in the target room.
- The app has `im:message:send_as_bot`.
- The app has `im:message.group_msg:readonly`.
- The app has `im:chat:readonly`.
- Event subscription uses Long Connection / WebSocket.
- `im.message.receive_v1` is subscribed.
- `tenant_key` and `chat_id` are from the same workspace and room.
