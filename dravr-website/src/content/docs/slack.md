---
title: Connect to Slack
description: Link your Dravr account to Slack and start asking questions in your workspace.
order: 3
platform: slack
---

Slack works well for teams and coaches who already coordinate there. Once Dravr is added to your workspace, every team member can link their account and ask Dravr questions directly.

## Before you start

- You need a Dravr account. If you don't have one, ask your coach or admin.
- Dravr must have been added to your Slack workspace by a workspace admin. If you don't see it under **Apps** in the sidebar, ask your admin to add it.

## How to connect

1. In Slack, find **Dravr** under **Apps** in the left sidebar. Click it to open a direct message with the bot.
2. Send any message (for example, "Hi") to start the conversation.
3. Dravr replies: *"Hi! To link your Dravr account, please type your email address."*
4. Type the email address you use to log in to Dravr.
5. Dravr replies: *"I've sent a 6-digit code to j***@yourdomain.com"*
6. Check your email and type the 6-digit code into the Slack message thread.
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
| `/logout` | Unlink Slack from your Dravr account |

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
4. Then follow the connection steps above to link Slack so you can chat with Dravr.

---

## What you can ask Dravr

- "How was my training load this week?"
- "Am I recovered enough to do a hard session today?"
- "What's my fitness trend over the last month?"
- "Show me my longest runs this year."

---

## Disconnecting

Type `/logout` in the Dravr app DM. Dravr will unlink Slack from your account. Your data and group memberships are not affected.

---

## FAQ

**I don't see Dravr under Apps in Slack. What do I do?**
Your workspace admin needs to add Dravr to the workspace first. Ask them to install it from the Slack App Directory.

**I typed the wrong email. Can I start over?**
Type `cancel` and then start the flow again by sending any message.

**Can I use Dravr in a Slack channel (not just DMs)?**
Account linking must be done in a direct message with the Dravr app. Once linked, you may be able to mention Dravr in channels depending on how your admin configured it.

---

**See also:** [Dravr on Messaging](/docs/messaging) · [Connect to Telegram](/docs/telegram) · [Connect to WhatsApp](/docs/whatsapp) · [Connect to Discord](/docs/discord)
