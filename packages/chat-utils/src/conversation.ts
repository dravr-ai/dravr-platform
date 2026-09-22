// ABOUTME: Cross-surface conversation helpers shared by web + mobile chat UIs
// ABOUTME: Filters tool plumbing rows, strips residual tool XML, labels the channel a thread came from

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

/**
 * Whether a re-read transcript holds a reply the client did not have when it
 * sent a turn.
 *
 * Asked about a turn whose stream the client lost while the athlete was away
 * — the idle stop dropped it, or the platform dropped a backgrounded app's
 * connection. The server finishes a turn whether or not anyone is still
 * reading, so its answer may already be persisted. `heldIds` are the ids of
 * the rows the client held when it sent the turn, so an `assistant` row
 * outside them is written since — the reply, or the interrupted notice the
 * server writes when a shutdown drain gave up on the turn, which answers it
 * too.
 *
 * Both clients ask exactly this before they drop the note that told the
 * athlete the reply had not arrived.
 */
export function replyLandedSince(
  rows: readonly Pick<Message, 'id' | 'role'>[],
  heldIds: ReadonlySet<string>,
): boolean {
  return rows.some((row) => row.role === 'assistant' && !heldIds.has(row.id));
}

/** Origin of a conversation that started on an external messaging channel. */
export interface MessageChannelOrigin {
  /** Lowercased channel slug as stored, e.g. `telegram`, `whatsapp`. */
  channel: string;
  /** Title-cased label for display, e.g. `Telegram`, `WhatsApp`. */
  label: string;
}

// Channels whose canonical casing isn't just a capitalized first letter.
const CHANNEL_LABEL_OVERRIDES: Readonly<Record<string, string>> = {
  whatsapp: 'WhatsApp',
  imessage: 'iMessage',
  sms: 'SMS',
};

// In-app origins stored in `chat_conversations.channel_type` (NOT NULL DEFAULT
// 'web'). These are not external messaging channels, so they get no badge.
const IN_APP_CHANNELS: ReadonlySet<string> = new Set(['web', 'mobile', '']);

/**
 * Resolve a conversation's messaging-channel origin for the channel glyph.
 *
 * The durable `channel_type` column is the one signal: messaging-ingress
 * stamps it at creation and it survives a rename or `/reset`. The title
 * carries no channel — it names the room or the agent the thread is with —
 * so there is nothing to parse. Returns `null` for an ordinary in-app
 * (`web`/`mobile`) chat, which shows no glyph.
 *
 * @example
 * resolveChannelOrigin({ channel_type: 'telegram' }) // { channel: 'telegram', label: 'Telegram' }
 * resolveChannelOrigin({ channel_type: 'whatsapp' }) // { channel: 'whatsapp', label: 'WhatsApp' }
 * resolveChannelOrigin({ channel_type: 'web' })      // null
 */
export function resolveChannelOrigin(
  conversation: { channel_type?: string | null },
): MessageChannelOrigin | null {
  const channel = conversation.channel_type?.trim().toLowerCase();
  if (!channel || IN_APP_CHANNELS.has(channel)) return null;
  const label =
    CHANNEL_LABEL_OVERRIDES[channel] ??
    channel.charAt(0).toUpperCase() + channel.slice(1);
  return { channel, label };
}
