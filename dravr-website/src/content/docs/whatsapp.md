---
title: Connect to WhatsApp
description: Link your Dravr account to WhatsApp and ask questions the way you already communicate.
order: 5
platform: whatsapp
---

WhatsApp is a good choice if you and your team already communicate there. Dravr works as a contact in your WhatsApp — just message it the same way you'd message a person.

## Before you start

- You need a Dravr account. If you don't have one, ask your coach or admin.
- Your coach or admin will give you the Dravr WhatsApp phone number.

## How to connect

1. Add the Dravr phone number to your contacts, then open a WhatsApp chat with it. Or open the link your coach shared directly — it will open the chat automatically.
2. Send any message (for example, "Hi") to start the conversation.
3. Dravr replies: *"Hi! To link your Dravr account, please type your email address."*
4. Type the email address you use to log in to Dravr.
5. Dravr replies: *"I've sent a 6-digit code to j***@yourdomain.com"*
6. Check your email and type the 6-digit code into the WhatsApp chat.
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
| `/logout` | Unlink WhatsApp from your Dravr account |

---

## Group onboarding

### Setting up a group (for coaches and admins)

1. Link your account using the steps above.
2. Type `/coach` — Dravr shows a list of AI coaches with descriptions.
3. Tap a coach to select it. Dravr creates a new group automatically.
4. Type `/group invite` — Dravr replies with a link valid for 7 days.
5. Share that link with your athletes.

### Joining a group (as an athlete)

1. Open the invite link your coach sent you. It opens in a browser.
2. Log in to Dravr (or create an account).
3. You are now a member of the group.
4. Then follow the connection steps above to link WhatsApp so you can chat with Dravr.

---

## What you can ask Dravr

- "How was my training load this week?"
- "Am I recovered enough to do a hard session today?"
- "What's my fitness trend over the last month?"
- "Show me my longest runs this year."

---

## Disconnecting

Type `/logout` in the WhatsApp chat. Dravr will unlink WhatsApp from your account. Your data and group memberships are not affected.

---

## FAQ

**I don't see a reply after sending my email. What should I do?**
Wait a few seconds — WhatsApp message delivery can occasionally be slow. If there's still no reply after a minute, check with your admin that the Dravr number is active.

**I typed the wrong email. Can I start over?**
Type `cancel` and then start the flow again by sending any message.

**Will Dravr ever message me first?**
Dravr only replies when you send a message. It does not send unprompted notifications via WhatsApp.

---

**See also:** [Dravr on Messaging](/docs/messaging) · [Connect to Telegram](/docs/telegram) · [Connect to Slack](/docs/slack) · [Connect to Discord](/docs/discord)
