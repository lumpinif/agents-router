# Feishu/Lark App Bot

Use this when you want the default Feishu/Lark two-way bridge path. Agents Router sends notifications through a Lark or Feishu app bot, can receive thread replies through Long Connection / WebSocket, and can connect project rooms with `/bind`. Lark Codex Desktop thread replies are Experimental.

Lark App Bot thread replies for Codex Desktop are the verified Experimental path. Feishu App Bot uses the same setup shape, but validate it in your workspace before relying on replies. Use Feishu/Lark Custom Bot only when you want a one-way fallback.

This guide uses Long Connection / WebSocket. You do not need a public webhook URL.

## Official Links

- Lark Developer Console: <https://open.larksuite.com/app>
- Feishu Developer Console: <https://open.feishu.cn/app>
- Lark long connection guide: <https://open.larksuite.com/document/ukTMukTMukTM/uYDNxYjL2QTM24iN0EjN/event-subscription-configure-/use-websocket>
- Lark send message API: <https://open.larksuite.com/document/uAjLw4CM/ukTMukTMukTM/reference/im-v1/message/create>

## What You Need

- A Custom App / self-built app.
- Bot capability enabled.
- The bot added to the target group.
- App ID.
- App Secret from the app console.
- Tenant Key.
- Chat ID for the target group.

Do not paste App Secret into chat, docs, screenshots, or issue reports. Setup stores it only in your local Agents Router config.

## 1. Create the App

Open:

```text
https://open.larksuite.com/app
```

Create a Custom App / self-built app.

Then open the app and copy:

```text
App ID
```

Keep `App Secret` private. Setup stores it in your local Agents Router config and never prints it in summaries or logs.

Advanced: if you prefer environment variables, you can hand-edit the config to use `app_secret_env` instead of `app_secret`.

## 2. Enable Bot

In the app console, add the Bot capability.

If the console asks you to configure bot availability, make the app available to the users or group where you will install it.

## 3. Add Permissions

Open Permissions / Scopes and enable these permissions:

```text
im:message:send_as_bot
im:message.group_msg:readonly
im:chat:readonly
```

Use the current console permission names if Lark shows localized labels. They usually map to:

```text
Send messages as bot
Read all group chat messages
Get group information
```

Do not use only the "messages mentioning the bot" permission. The experimental reply path uses normal thread replies, not only @bot messages.

## 4. Subscribe to Events

Open Event Subscriptions.

Choose:

```text
Long Connection / WebSocket
```

Subscribe to:

```text
im.message.receive_v1
```

Do not configure a public webhook URL for this mode.

## 5. Publish the App Version

After changing Bot capability, permissions, or event subscriptions, create and publish a new app version.

If your workspace requires admin review, wait until the version is approved. Permissions and events do not reliably work until the published version is active.

## 6. Add the Bot to the Group

In Lark:

```text
Target group -> Group settings -> Bots -> Add Bot
```

Select the app bot you created.

Confirm the bot can send messages to this group.

## 7. Get Chat ID

Use an official path:

- API Explorer for `GET /open-apis/im/v1/chats`, using the app's tenant access token, then pick the target group.
- Or send a normal message in the group after Long Connection is configured and read `message.chat_id` from the official `im.message.receive_v1` event payload.

The Chat ID usually starts with:

```text
oc_
```

## 8. Get Tenant Key

Use an official app response or event payload:

- Send a bot message through the official send message API and read `sender.tenant_key` from the response.
- Or read `tenant_key` from an official Long Connection event payload for the same workspace.

Do not read private local files or guess Tenant Key.

## 9. Connect Agents Router

Run:

```bash
agents-router setup
```

Choose:

```text
Feishu/Lark
App Bot — Experimental
```

Enter:

```text
domain: lark
App ID: cli_...
App Secret: paste from the Lark console
Tenant Key: ...
Chat ID: oc_...
```

For the first version, use the simple route:

```text
Codex Desktop -> one Feishu/Lark App Bot group
```

## Test Message

Setup can send one test message.

That test message only confirms the app bot can send to the target group. It is not a Codex continuation surface. Replying to the setup test message will not continue Codex.

To test Experimental Lark thread replies, wait for a new real Codex Desktop completion notification in the group. Reply in that notification's thread. Codex sends the result back to the same thread.

Old notifications and setup test messages are not replyable continuation surfaces.

## Troubleshooting

Check these first:

- The app version was published after permission or event changes.
- Admin approval is complete, if required by your workspace.
- Bot capability is enabled.
- The bot is in the target group.
- The app has `im:message:send_as_bot`.
- The app has `im:message.group_msg:readonly`.
- The app has `im:chat:readonly`.
- Event subscription uses Long Connection / WebSocket.
- `im.message.receive_v1` is subscribed.
- App Secret is present in the local config, or `app_secret_env` is configured manually.
- `tenant_key` and `chat_id` are from the same workspace and group.
