---
title: Dravr on Messaging
description: How Dravr works across messaging platforms — overview, commands, and group setup.
order: 1
---

Dravr connects to the messaging apps you already use — so you can ask about your training, check your recovery, and work with your coach without opening a separate app.

## Supported channels

| Channel | Best for | Requires |
|---------|----------|----------|
| Telegram | Athletes who want a fast, reliable personal chat | A Telegram account |
| WhatsApp | Athletes already using WhatsApp for communication | A WhatsApp account |
| Slack | Teams and coaches on a shared workspace | A Slack workspace with Dravr added by an admin |
| Discord | Athletic communities and group training servers | A Discord server with Dravr added |

See the channel-specific guides to get started:
- [Connect Dravr to Telegram](/docs/telegram)
- [Connect Dravr to WhatsApp](/docs/whatsapp)
- [Connect Dravr to Slack](/docs/slack)
- [Connect Dravr to Discord](/docs/discord)

---

## How to connect your account

All channels use the same OTP (one-time code) flow. The bot contact or username is provided by your coach or workspace admin.

1. Message the Dravr bot on your chosen channel.
2. Dravr replies: *"Hi! To link your Dravr account, please type your email address."*
3. Type the email address you use to log in to Dravr.
4. Dravr replies: *"I've sent a 6-digit code to j***@yourdomain.com"*
5. Check your email and type the 6-digit code into the chat.
6. Dravr replies: *"Your account has been linked successfully!"*

**Note:** The code expires in 10 minutes. You have 3 attempts. Type `cancel` at any time to stop.

---

## Commands

Once connected, these slash commands are available on all channels.

| Command | What it does |
|---------|-------------|
| `/help` | List all available commands |
| `/status` | See your connected providers, groups, and active channel |
| `/coach` | Browse available coaches (interactive card with buttons) |
| `/coach select <id>` | Pick a coach — auto-creates a group if you have none |
| `/coach assign <coach_id> <group_id>` | Reassign a coach to a specific group (admins only) |
| `/group` | List your groups (name, members, your role) |
| `/group status` | Show aggregate stats for your group |
| `/group members` | List members of your group |
| `/group invite` | Generate a 7-day invite link — admin/owner only |
| `/group leave` | Leave your group (asks for confirmation) |
| `/logout` | Unlink this channel from your Dravr account |

---

## Group onboarding

### Setting up a group (for coaches and admins)

1. Link your account to the channel using the OTP flow above.
2. Type `/coach` — Dravr shows a list of AI coaches with descriptions.
3. Tap or click a coach to select it. Dravr creates a new group automatically.
4. Type `/group invite` — Dravr replies with a link valid for 7 days.
5. Share that link with your athletes.

### Joining a group (as an athlete)

1. Open the invite link your coach sent you. It opens in a browser.
2. Log in to Dravr (or create an account if you don't have one).
3. You are now a member of the group.
4. Then link your messaging channel using the OTP flow above so you can chat with Dravr.

---

## Disconnecting

Type `/logout` in any channel. Dravr will unlink that channel from your account. Your data and group memberships are not affected.

---

## What you can ask Dravr

- "How was my training load this week?"
- "Am I recovered enough to do a hard session today?"
- "What's my fitness trend over the last month?"
- "Show me my longest runs this year."
