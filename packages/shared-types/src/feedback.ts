// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The in-app feedback thread wire shape — the athlete and the Dravr team writing to each other, and its unread count
// ABOUTME: With its parsers, so a malformed body is an error rather than a thread drawn from guesses (carnet#1235, #1392)

/** The longest message the thread accepts, in characters — the server's `MAX_BODY_CHARS`. */
export const FEEDBACK_MESSAGE_MAX_CHARS = 4000;

/** The largest screenshot a message carries, in bytes — the server's `MAX_IMAGE_BYTES`. */
export const FEEDBACK_IMAGE_MAX_BYTES = 5 * 1024 * 1024;

/** The screenshot types the thread takes; the server checks each by its own leading bytes. */
export const FEEDBACK_IMAGE_TYPES = ['image/png', 'image/jpeg', 'image/webp', 'image/gif'] as const;

/** One of the screenshot types the thread takes. */
export type FeedbackImageType = (typeof FEEDBACK_IMAGE_TYPES)[number];

/** A screenshot as `POST /api/me/feedback` takes it: its type and its bytes in standard base64. */
export interface FeedbackImageUpload {
  media_type: FeedbackImageType;
  data: string;
}

/** The agent reply a feedback message is about, when the thread was opened from a thumbs-down. */
export interface FeedbackAbout {
  conversation_id: string;
  message_id: string;
}

/** Who wrote a message: the athlete, or the Dravr team replying from Slack. */
export type FeedbackAuthor = 'athlete' | 'team';

/** One message of the athlete's feedback thread. */
export interface FeedbackMessage {
  id: string;
  /** The team's replies are signed "Dravr team"; the replier is never named. */
  author: FeedbackAuthor;
  body: string;
  /** RFC 3339 instant the message was sent. */
  created_at: string;
  about: FeedbackAbout | null;
  /** `GET /api/me/feedback/{id}/image` serves the screenshot this message carries. */
  has_image: boolean;
}

/** `GET /api/me/feedback`: the newest messages, oldest first. */
export interface FeedbackThreadResponse {
  messages: FeedbackMessage[];
}

/** `GET /api/me/feedback/unread`: the team replies the athlete has not opened. */
export interface FeedbackUnreadResponse {
  unread: number;
}

/** `POST /api/me/feedback`. Everything but `body` is optional context the client adds. */
export interface SendFeedbackRequest {
  /** May be empty when a screenshot goes with it. */
  body: string;
  /** The web route the thread was opened from. */
  screen?: string;
  /** The client's build. */
  app_version?: string;
  /** The client's language. */
  locale?: string;
  /** The reply the message is about; only one the athlete rated is accepted. */
  about?: FeedbackAbout;
  /** One screenshot. */
  image?: FeedbackImageUpload;
}

/** Whether `type` is a screenshot type the thread takes. */
export function isFeedbackImageType(type: string): type is FeedbackImageType {
  return (FEEDBACK_IMAGE_TYPES as readonly string[]).includes(type);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function parseAbout(value: unknown): FeedbackAbout | null | undefined {
  if (value === null || value === undefined) return null;
  if (!isRecord(value)) return undefined;
  const { conversation_id: conversationId, message_id: messageId } = value;
  if (typeof conversationId !== 'string' || typeof messageId !== 'string') return undefined;
  return { conversation_id: conversationId, message_id: messageId };
}

/** A message's author, or undefined when it names neither side. */
function parseAuthor(value: unknown): FeedbackAuthor | undefined {
  return value === 'athlete' || value === 'team' ? value : undefined;
}

/** Read one thread message, or null when it is not one. */
export function parseFeedbackMessage(value: unknown): FeedbackMessage | null {
  if (!isRecord(value)) return null;
  const { id, body, created_at: createdAt } = value;
  if (typeof id !== 'string' || typeof body !== 'string' || typeof createdAt !== 'string') {
    return null;
  }
  const author = parseAuthor(value.author);
  if (author === undefined) return null;
  const about = parseAbout(value.about);
  if (about === undefined) return null;
  const { has_image: hasImage } = value;
  if (typeof hasImage !== 'boolean') return null;
  return { id, author, body, created_at: createdAt, about, has_image: hasImage };
}

/** Read a `GET /api/me/feedback` body, or null when it is not one. */
export function parseFeedbackThread(body: unknown): FeedbackThreadResponse | null {
  if (!isRecord(body) || !Array.isArray(body.messages)) return null;
  const messages: FeedbackMessage[] = [];
  for (const entry of body.messages) {
    const message = parseFeedbackMessage(entry);
    if (message === null) return null;
    messages.push(message);
  }
  return { messages };
}

/** Read the message a `201` from `POST /api/me/feedback` carries, or null when it is not one. */
export function parseFeedbackCreated(body: unknown): FeedbackMessage | null {
  return isRecord(body) ? parseFeedbackMessage(body.message) : null;
}

/** Read a `GET /api/me/feedback/unread` body, or null when it is not one. */
export function parseFeedbackUnread(body: unknown): FeedbackUnreadResponse | null {
  if (!isRecord(body)) return null;
  const { unread } = body;
  if (typeof unread !== 'number' || !Number.isInteger(unread) || unread < 0) return null;
  return { unread };
}
