// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The one reader of a chat turn's response body — SSE frames and the single-JSON answer alike
// ABOUTME: Every surface's send path funnels through parseTurnBody; nothing else parses a turn off the wire

import type { ReplyBlock, TurnEnvelope, TurnProgress } from '@pierre/shared-types';

/**
 * What a caller learns while a turn is in flight and when it finishes.
 *
 * Exactly one terminal callback fires per turn: `onDone` with the envelope,
 * or `onError` with the reason. A caller that registers both has a complete
 * outcome contract and needs no `try`/`catch` around the send.
 */
export interface TurnCallbacks {
  /** A tool call the turn observed, with its latest known status. */
  onProgress?: (progress: TurnProgress) => void;
  /** The next slice of assistant text to append to the in-flight bubble. */
  onDelta?: (delta: string) => void;
  /**
   * One renderable piece of the finished reply, in the order the server put
   * them in. Fired for every block before `onDone`, so a client lays out what
   * it was given in one walk instead of hunting the list for each block type
   * it happens to draw.
   */
  onBlock?: (block: ReplyBlock) => void;
  /** The finished turn. Terminal. */
  onDone?: (turn: TurnEnvelope) => void;
  /** The turn did not finish. Terminal. */
  onError?: (error: Error) => void;
}

/**
 * A turn the server refused before it began.
 *
 * Carries the status and the decoded error body so a caller can format the
 * refusal the way it formats every other API refusal — a quota 429 names the
 * limit it hit rather than degrading to "request failed". `sendTurn` bypasses
 * axios to read frames, so this is what stands in for `AxiosError.response`.
 */
export class TurnRequestError extends Error {
  /** HTTP status the server answered with. */
  readonly status: number;
  /** The decoded error body, or `null` when it was not JSON. */
  readonly body: unknown;

  constructor(message: string, status: number, body: unknown) {
    super(message);
    this.name = 'TurnRequestError';
    this.status = status;
    this.body = body;
  }

  /**
   * The same two facts under the names an `AxiosError` uses.
   *
   * A turn is the one request that cannot ride axios — it is read frame by
   * frame — so its refusals used to arrive in a second shape, and every
   * formatter grew a branch to unwrap whichever carrier it was handed. Wearing
   * the axios shape means one classifier reads both, and a client asking "what
   * went wrong" never has to ask "which transport asked".
   */
  get response(): { status: number; data: unknown } {
    return { status: this.status, data: this.body };
  }
}

/** Prefixes that mark a body as a `text/event-stream` document. */
const SSE_LINE_PREFIXES = ['event:', 'data:', 'id:', 'retry:', ':'] as const;

/** Frames are separated by a blank line, in either newline convention. */
const FRAME_SEPARATORS = ['\n\n', '\r\n\r\n'] as const;

/**
 * Does this body announce itself as an event stream?
 *
 * Read off the bytes rather than off a `Content-Type` header, because the
 * same endpoint answers a slash command with a single JSON document and an
 * LLM turn with a frame stream, and a client that sniffed the header had two
 * readers to keep in agreement. An SSE body's very first line always carries
 * one of the field prefixes; a JSON document's first character is `{`.
 */
function looksLikeEventStream(buffer: string): boolean {
  // \uFEFF spelled out rather than pasted: an invisible BOM in a regex is
  // unreadable in review and easy to delete by accident.
  const trimmed = buffer.replace(/^\uFEFF/, '').trimStart();
  return SSE_LINE_PREFIXES.some((prefix) => trimmed.startsWith(prefix));
}

/** Split off the first complete frame, or `null` while one is still arriving. */
function takeFrame(buffer: string): { frame: string; rest: string } | null {
  let cut = -1;
  let width = 0;
  for (const separator of FRAME_SEPARATORS) {
    const at = buffer.indexOf(separator);
    if (at !== -1 && (cut === -1 || at < cut)) {
      cut = at;
      width = separator.length;
    }
  }
  if (cut === -1) return null;
  return { frame: buffer.slice(0, cut), rest: buffer.slice(cut + width) };
}

/** One SSE frame decoded into its event name and joined data payload. */
export interface SseFrame {
  /** The frame's `event:` name, or `"message"` when it carried none. */
  event: string;
  /** The frame's `data:` lines, joined with newlines. */
  data: string;
}

/**
 * Decode one frame's lines.
 *
 * Returns `null` for a frame that carries no `data:` line — the keep-alive
 * comments the server sends every 15 seconds to hold the connection open
 * through nginx are exactly that shape.
 */
function decodeFrame(raw: string): SseFrame | null {
  let event = 'message';
  const data: string[] = [];
  for (const line of raw.split(/\r?\n/)) {
    if (line.startsWith('event:')) {
      event = line.slice(line[6] === ' ' ? 7 : 6).trim();
    } else if (line.startsWith('data:')) {
      data.push(line.slice(line[5] === ' ' ? 6 : 5));
    }
  }
  if (data.length === 0) return null;
  return { event, data: data.join('\n') };
}

function parseJson<T>(raw: string): T | null {
  try {
    return JSON.parse(raw) as T;
  } catch {
    return null;
  }
}

/**
 * Read a response body that may or may not be an event stream.
 *
 * The single frame reader in the workspace. Whether the body is announced as
 * an event stream is read off its own first bytes rather than off a
 * `Content-Type` header, because the chat endpoint answers a slash command
 * with a single JSON document and an LLM turn with a frame stream — a client
 * that sniffed the header had two readers to keep in agreement.
 *
 * @param chunks the body as it arrives.
 * @param onFrame called with each complete frame; return `true` to stop
 *   reading (the stream reached a terminal event).
 * @param onDocument called once with the whole body when it was not an event
 *   stream. Omit it where the endpoint only ever streams.
 */
export async function readEventStream(
  chunks: AsyncIterable<string>,
  onFrame: (frame: SseFrame) => boolean | void,
  onDocument?: (body: string) => void,
): Promise<void> {
  let buffer = '';
  let streaming: boolean | null = null;

  for await (const chunk of chunks) {
    buffer += chunk;
    if (streaming === null && buffer.trimStart().length > 0) {
      streaming = looksLikeEventStream(buffer);
    }
    if (!streaming) continue;

    for (let next = takeFrame(buffer); next !== null; next = takeFrame(buffer)) {
      buffer = next.rest;
      const frame = decodeFrame(next.frame);
      if (frame && onFrame(frame) === true) return;
    }
  }

  if (streaming === false) {
    onDocument?.(buffer);
  }
}

/** The live half of {@link TurnCallbacks}: what a caller learns mid-turn. */
export type TurnProgressSink = Pick<TurnCallbacks, 'onDelta' | 'onProgress' | 'onBlock'>;

/**
 * Read a turn off its response body.
 *
 * The one rail a turn arrives on. Handles both shapes the chat endpoint
 * answers with, in one pass and with no content-type sniff:
 *
 * - a `text/event-stream` body: `progress` frames report the stage or tool
 *   the turn is working on, `delta` frames append assistant text, `block`
 *   frames carry each renderable piece of the reply as the server decides it,
 *   a `failed` frame ends the turn, and the `done` frame carries the finished
 *   {@link TurnEnvelope};
 * - a single JSON document: a slash command answers before the streaming
 *   branch is ever chosen, so the whole body *is* the envelope, and its block
 *   list is walked here so the caller's `onBlock` fires either way.
 *
 * Frames are dispatched as they complete, so a caller passing `onDelta`
 * renders text while the turn is still running; a body that arrives in one
 * piece drives the same callbacks, just all at once.
 *
 * @param chunks the body as it arrives — one chunk where the runtime has no
 *   streaming reader, many where it does.
 * @param sink live callbacks for progress, deltas and reply blocks.
 * @returns the finished turn.
 * @throws when the body carried a `failed` frame, or ended without a reply.
 */
export async function parseTurnBody(
  chunks: AsyncIterable<string>,
  sink: TurnProgressSink = {},
): Promise<TurnEnvelope> {
  // Held in an object rather than in `let`s: the reader writes them from
  // inside callbacks, where narrowing a plain local would leave the compiler
  // convinced nothing was ever assigned.
  const outcome: {
    envelope: TurnEnvelope | null;
    failure: Error | null;
    streamedBlocks: boolean;
  } = { envelope: null, failure: null, streamedBlocks: false };

  await readEventStream(
    chunks,
    frame => {
      if (frame.event === 'progress') {
        const parsed = parseJson<TurnProgress>(frame.data);
        if (parsed?.kind) sink.onProgress?.(parsed);
      } else if (frame.event === 'delta') {
        const parsed = parseJson<{ delta?: string }>(frame.data);
        if (parsed?.delta) sink.onDelta?.(parsed.delta);
      } else if (frame.event === 'block') {
        const parsed = parseJson<ReplyBlock>(frame.data);
        if (parsed?.type) {
          outcome.streamedBlocks = true;
          sink.onBlock?.(parsed);
        }
      } else if (frame.event === 'done') {
        outcome.envelope = parseJson<TurnEnvelope>(frame.data);
        if (!outcome.envelope) {
          outcome.failure = new Error('The turn finished with an unreadable payload.');
        }
      } else if (frame.event === 'failed') {
        const parsed = parseJson<{ error?: string }>(frame.data);
        outcome.failure = new Error(
          parsed?.error ?? 'The server ended the turn with an error.',
        );
      }
    },
    // A slash command answers before the streaming branch is ever chosen, so
    // the whole body is the envelope.
    body => {
      outcome.envelope = parseJson<TurnEnvelope>(body);
    },
  );

  if (outcome.failure) throw outcome.failure;
  if (!outcome.envelope?.assistant) {
    throw new Error('The turn ended without a reply.');
  }
  // A document body carried no `block` frames, so its pieces are walked here.
  // Streamed turns already fired them in the order the server decided.
  if (!outcome.streamedBlocks) {
    for (const block of outcome.envelope.assistant.blocks ?? []) {
      sink.onBlock?.(block);
    }
  }
  return outcome.envelope;
}
