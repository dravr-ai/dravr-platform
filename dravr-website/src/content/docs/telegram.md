---
title: Connect to Telegram
description: Link your Dravr account to Telegram for fast, private access to your training data.
order: 4
platform: telegram
---

Telegram is great for athletes who want fast, reliable access to Dravr through a lightweight chat app. Conversations are private by default and work well on both phone and desktop.

## Before you start

- You need a Dravr account. If you don't have one, ask your coach or admin.
- Your coach or admin will give you the Dravr bot username (it looks like `@DravrBot` or similar).

## How to connect

1. Open Telegram and search for the bot username your coach gave you, or open `t.me/{botname}` in your browser.
2. Tap **Start** to open a conversation with the bot.
3. Dravr replies: *"Hi! To link your Dravr account, please type your email address."*
4. Type the email address you use to log in to Dravr.
5. Dravr replies: *"I've sent a 6-digit code to j***@yourdomain.com"*
6. Check your email and type the 6-digit code into the Telegram chat.
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
| `/logout` | Unlink Telegram from your Dravr account |

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
4. Then follow the connection steps above to link Telegram so you can chat with Dravr.

---

## What you can ask Dravr

- "How was my training load this week?"
- "Am I recovered enough to do a hard session today?"
- "What's my fitness trend over the last month?"
- "Show me my longest runs this year."

---

## Disconnecting

Type `/logout` in the Telegram chat. Dravr will unlink Telegram from your account. Your data and group memberships are not affected.

---

## FAQ

**The bot isn't responding. What do I do?**
Make sure you tapped **Start** to begin the conversation. If the bot still doesn't reply, check with your admin that the bot is running.

**I typed the wrong email. Can I start over?**
Type `cancel` and then start the flow again by sending any message.

**Can I use Dravr in a Telegram group chat?**
Group chats are not supported for linking accounts. Use the bot in a direct (private) message.

---

**See also:** [Dravr on Messaging](/docs/messaging) · [Connect to WhatsApp](/docs/whatsapp) · [Connect to Slack](/docs/slack) · [Connect to Discord](/docs/discord)
