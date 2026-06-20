// ABOUTME: Cross-surface conversation helpers shared by web + mobile chat UIs
// ABOUTME: Filters tool plumbing rows, strips residual tool XML, derives channel badges

import type { Message, MessageRole } from '@pierre/shared-types';

/**
 * Roles that are pure LLM plumbing and must never be shown in a user-facing
 * thread. `tool_call` / `tool_result` rows hold raw `<tool_call>` /
 * `<tool_result>` XML scaffolding the model uses to invoke tools and read
 * their output; they are persisted in `chat_messages` (so messaging-channel
 * conversations replay correctly through the prompt builder) but are not
 * conversational turns.
 */
const PLUMBING_ROLES: ReadonlySet<MessageRole> = new Set<MessageRole>([
  'tool_call',
  'tool_result',
]);

/**
 * True when a message is an internal tool-plumbing row that should be hidden
 * from the rendered conversation.
 */
export function isToolPlumbingMessage(message: Pick<Message, 'role'>): boolean {
  return PLUMBING_ROLES.has(message.role);
}

/**
 * Drop tool-plumbing rows (`tool_call` / `tool_result`) from a message list
 * before rendering. The remaining `user` / `assistant` / `system` turns are the
 * only ones a human should ever see. Used identically by the web and mobile
 * chat thread renderers so both surfaces behave the same — including for
 * conversations that originated on a messaging channel (Telegram, WhatsApp, …),
 * which carry the same scaffolding rows as web chat.
 *
 * @typeParam T - any message-like object that exposes a `role`.
 */
export function filterDisplayMessages<T extends Pick<Message, 'role'>>(
  messages: readonly T[],
): T[] {
  return messages.filter((m) => !isToolPlumbingMessage(m));
}

// Matches a full `<tool_call>…</tool_call>` or `<tool_result>…</tool_result>`
// block (including the tags), across newlines, non-greedy so adjacent blocks
// don't merge. Defensive only — the row-level filter is the primary guard;
// this strips any scaffolding that leaked into an otherwise-visible turn.
const TOOL_BLOCK_REGEX = /<(tool_call|tool_result)\b[^>]*>[\s\S]*?<\/\1>\s*/gi;

// Matches a stray, unclosed opening tag (e.g. a truncated stream) so a bare
// `<tool_call>` doesn't render as literal text at the end of a turn.
const TOOL_OPEN_TAG_REGEX = /<\/?(tool_call|tool_result)\b[^>]*>/gi;

/**
 * Defensively strip any residual `<tool_call>` / `<tool_result>` XML blocks
 * from message content that is being displayed. Row-level filtering via
 * {@link filterDisplayMessages} removes whole plumbing rows; this guards the
 * rarer case where scaffolding ends up embedded inside an assistant turn's
 * own content. Returns the content unchanged when no scaffolding is present.
 */
export function stripToolScaffolding(content: string): string {
  if (!content.includes('<tool_call') && !content.includes('<tool_result')) {
    return content;
  }
  return content
    .replace(TOOL_BLOCK_REGEX, '')
    .replace(TOOL_OPEN_TAG_REGEX, '')
    .trim();
}

/** Origin of a conversation that started on an external messaging channel. */
export interface MessageChannelOrigin {
  /** Lowercased channel slug as stored, e.g. `telegram`, `whatsapp`. */
  channel: string;
  /** Title-cased label for display, e.g. `Telegram`, `WhatsApp`. */
  label: string;
}

// Messaging-ingress conversations are titled `Messaging: <channel>` by the
// backend (services/messaging_ingress/session.rs). That title prefix is the
// only client-visible signal of a conversation's channel origin — there is no
// dedicated `channel` field on the conversation API.
const MESSAGING_TITLE_REGEX = /^Messaging:\s*([a-z0-9_-]+)\s*$/i;

// Channels whose canonical casing isn't just a capitalized first letter.
const CHANNEL_LABEL_OVERRIDES: Readonly<Record<string, string>> = {
  whatsapp: 'WhatsApp',
  imessage: 'iMessage',
  sms: 'SMS',
};

/**
 * Derive a messaging-channel origin from a conversation title.
 *
 * Messaging-origin conversations are created with the title
 * `"Messaging: <channel>"` (e.g. `"Messaging: telegram"`). Returns the channel
 * slug and a display label, or `null` for an ordinary web/app conversation
 * (which shows no channel badge). The match is case-insensitive and tolerant
 * of surrounding whitespace.
 *
 * @example
 * deriveMessageChannel('Messaging: telegram') // { channel: 'telegram', label: 'Telegram' }
 * deriveMessageChannel('Messaging: whatsapp') // { channel: 'whatsapp', label: 'WhatsApp' }
 * deriveMessageChannel('Chat Jun 7 11:15 AM') // null
 */
export function deriveMessageChannel(
  title: string | null | undefined,
): MessageChannelOrigin | null {
  if (!title) return null;
  const match = title.match(MESSAGING_TITLE_REGEX);
  if (!match) return null;
  const channel = match[1].toLowerCase();
  const label =
    CHANNEL_LABEL_OVERRIDES[channel] ??
    channel.charAt(0).toUpperCase() + channel.slice(1);
  return { channel, label };
}

// In-app origins stored in `chat_conversations.channel_type` (NOT NULL DEFAULT
// 'web'). These are not external messaging channels, so they get no badge.
const IN_APP_CHANNELS: ReadonlySet<string> = new Set(['web', 'mobile', '']);

/**
 * Resolve a conversation's messaging-channel origin for the channel badge.
 *
 * Prefers the **durable** `channel_type` column (stamped by messaging-ingress
 * at creation; survives a later title rename or `/reset`), and falls back to
 * parsing the `"Messaging: <channel>"` {@link deriveMessageChannel | title} for
 * conversations created before the column was populated (or not yet
 * backfilled). Returns `null` for an ordinary in-app (`web`/`mobile`) chat,
 * which shows no badge.
 *
 * @example
 * resolveChannelOrigin({ channel_type: 'telegram', title: 'My plan' })   // { channel: 'telegram', label: 'Telegram' }
 * resolveChannelOrigin({ channel_type: 'web', title: 'Messaging: slack' }) // { channel: 'slack', label: 'Slack' } (fallback)
 * resolveChannelOrigin({ channel_type: 'web', title: 'Chat Jun 7' })      // null
 */
export function resolveChannelOrigin(
  conversation: { title?: string | null; channel_type?: string | null },
): MessageChannelOrigin | null {
  const channel = conversation.channel_type?.trim().toLowerCase();
  if (channel && !IN_APP_CHANNELS.has(channel)) {
    const label =
      CHANNEL_LABEL_OVERRIDES[channel] ??
      channel.charAt(0).toUpperCase() + channel.slice(1);
    return { channel, label };
  }
  return deriveMessageChannel(conversation.title);
}
