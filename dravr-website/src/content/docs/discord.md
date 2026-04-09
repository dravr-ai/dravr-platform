---
title: Connect to Discord
description: Link your Dravr account to Discord and start asking questions in your server.
order: 2
platform: discord
---

Discord is a good fit for athletic communities and group training servers. Once Dravr is added to a server, members can link their accounts and ask Dravr questions directly.

## Before you start

- You need a Dravr account. If you don't have one, ask your coach or admin.
- Dravr must have been added to your Discord server by a server admin. If you don't see `@Dravr` in your server, ask your admin to add it.

## How to connect

1. In Discord, open a direct message with the Dravr bot. You can find it by clicking its name in the server member list or by mentioning `@Dravr` in a channel and then clicking its profile.
2. Send any message (for example, "Hi") to start the conversation.
3. Dravr replies: *"Hi! To link your Dravr account, please type your email address."*
4. Type the email address you use to log in to Dravr.
5. Dravr replies: *"I've sent a 6-digit code to j***@yourdomain.com"*
6. Check your email and type the 6-digit code into the Discord DM.
7. Dravr replies: *"Your account has been linked successfully!"*

**Note:** The code expires in 10 minutes. You have 3 attempts. Type `cancel` at any time to stop.

---

## Commands

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
| `/logout` | Unlink Discord from your Dravr account |

---

## Group onboarding

### Setting up a group (for coaches and admins)

1. Link your account using the steps above.
2. Type `/coach` — Dravr shows a list of AI coaches with descriptions.
3. Click a coach to select it. Dravr creates a new group automatically.
4. Type `/group invite` — Dravr replies with a link valid for 7 days.
5. Share that link with your athletes.

### Joining a group (as an athlete)

1. Open the invite link your coach sent you. It opens in a browser.
2. Log in to Dravr (or create an account).
3. You are now a member of the group.
4. Then follow the connection steps above to link Discord so you can chat with Dravr.

---

## What you can ask Dravr

- "How was my training load this week?"
- "Am I recovered enough to do a hard session today?"
- "What's my fitness trend over the last month?"
- "Show me my longest runs this year."

---

## Disconnecting

Type `/logout` in the DM with Dravr. Dravr will unlink Discord from your account. Your data and group memberships are not affected.

---

## FAQ

**I don't see Dravr in my Discord server. What do I do?**
Your server admin needs to add Dravr to the server first. Ask them to invite the bot using the Dravr admin panel.

**I typed the wrong email. Can I start over?**
Type `cancel` and then start the flow again by sending any message.

**Can I ask Dravr questions in a server channel instead of a DM?**
Account linking must be done in a direct message with the bot. Once linked, you may be able to interact with Dravr in server channels depending on your server's configuration.

---

**See also:** [Dravr on Messaging](/docs/messaging) · [Connect to Telegram](/docs/telegram) · [Connect to WhatsApp](/docs/whatsapp) · [Connect to Slack](/docs/slack)
