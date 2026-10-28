// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The in-app feedback thread wire shape — what an athlete writes to the Dravr team, and the reply it may be about
// ABOUTME: With its parsers, so a malformed body is an error rather than a thread drawn from guesses (carnet#1235)

/** The longest message the thread accepts, in characters — the server's `MAX_BODY_CHARS`. */
export const FEEDBACK_MESSAGE_MAX_CHARS = 4000;

/** The agent reply a feedback message is about, when the thread was opened from a thumbs-down. */
export interface FeedbackAbout {
  conversation_id: string;
  message_id: string;
}

/** One message of the athlete's feedback thread. */
export interface FeedbackMessage {
  id: string;
  body: string;
  /** RFC 3339 instant the message was sent. */
  created_at: string;
  about: FeedbackAbout | null;
}

/** `GET /api/me/feedback`: the newest messages, oldest first. */
export interface FeedbackThreadResponse {
  messages: FeedbackMessage[];
}

/** `POST /api/me/feedback`. Everything but `body` is optional context the client adds. */
export interface SendFeedbackRequest {
  body: string;
  /** The web route the thread was opened from. */
  screen?: string;
  /** The client's build. */
  app_version?: string;
  /** The client's language. */
  locale?: string;
  /** The reply the message is about; only one the athlete rated is accepted. */
  about?: FeedbackAbout;
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

/** Read one thread message, or null when it is not one. */
export function parseFeedbackMessage(value: unknown): FeedbackMessage | null {
  if (!isRecord(value)) return null;
  const { id, body, created_at: createdAt } = value;
  if (typeof id !== 'string' || typeof body !== 'string' || typeof createdAt !== 'string') {
    return null;
  }
  const about = parseAbout(value.about);
  if (about === undefined) return null;
  return { id, body, created_at: createdAt, about };
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
