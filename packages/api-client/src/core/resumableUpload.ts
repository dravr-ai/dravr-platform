// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A chunked writer for a resumable upload session: Content-Range PUTs, the committed-offset query, resume after a failure
// ABOUTME: Never buffers the file, gives up visibly when nothing advances, and keeps the session URL out of every error it raises

/** Bytes per chunk: a multiple of the 256 KiB every chunk but the last must be. */
export const UPLOAD_CHUNK_BYTES = 8 * 1024 * 1024;

/** How many attempts in a row may advance nothing before the upload gives up. */
export const UPLOAD_MAX_STALLED_ATTEMPTS = 5;

/** The wait after the first failed attempt; each further one doubles it. */
const FIRST_RETRY_MS = 1000;

/** The longest wait between two attempts. */
const MAX_RETRY_MS = 30_000;

/** A file the uploader reads a slice at a time. */
export interface UploadBytes {
  /** Its length in bytes. */
  size: number;
  /** Bytes `start..end`. */
  read(start: number, end: number): Promise<Blob | ArrayBuffer | Uint8Array>;
  /** Release whatever reading it held. */
  close?(): void;
}

/** Why an upload stopped: nothing advanced, the session is gone, or the store refused the bytes. */
export type UploadFailure = 'stalled' | 'gone' | 'refused';

/** An upload that stopped. It names no URL, since a session's URL is a credential. */
export class ExportUploadError extends Error {
  readonly failure: UploadFailure;

  constructor(failure: UploadFailure, status?: number) {
    super(`upload ${failure}${status === undefined ? '' : ` (HTTP ${status})`}`);
    this.name = 'ExportUploadError';
    this.failure = failure;
  }
}

/** What writing to a session takes. */
export interface SessionUploadOptions {
  /** The session, a bearer credential for the write. */
  sessionUrl: string;
  file: UploadBytes;
  /** Bytes per chunk; {@link UPLOAD_CHUNK_BYTES} unless a test says otherwise. */
  chunkBytes?: number;
  /** Told how many bytes the store has committed, after every change. */
  onProgress?: (committed: number, total: number) => void;
  /** Stops the upload; the session stays resumable. */
  signal?: AbortSignal;
  /** The HTTP client; the platform's `fetch` unless a test says otherwise. */
  fetchImpl?: typeof fetch;
  /** Waits between attempts; a timer unless a test says otherwise. */
  wait?: (ms: number) => Promise<void>;
}

/** The committed length a `Range: bytes=0-N` header names; none means nothing is committed. */
export function committedLength(range: string | null): number {
  const match = range === null ? null : /^bytes=0-(\d+)$/.exec(range.trim());
  return match ? Number(match[1]) + 1 : 0;
}

/** Whether a store's answer is worth trying again: it may clear on its own. */
function isTransient(status: number): boolean {
  return status === 408 || status === 429 || status >= 500;
}

/** What one request to the session found. */
type SessionAnswer =
  /** The store holds the whole object. */
  | { kind: 'done' }
  /** The store holds this much so far. */
  | { kind: 'partial'; committed: number }
  /** Nothing learned: try again. */
  | { kind: 'retry' };

/** Read a session's answer, throwing on one that can never succeed. */
function answerOf(response: Response): SessionAnswer {
  const { status } = response;
  if (status === 200 || status === 201) return { kind: 'done' };
  if (status === 308) return { kind: 'partial', committed: committedLength(response.headers.get('Range')) };
  if (status === 404 || status === 410) throw new ExportUploadError('gone', status);
  if (isTransient(status)) return { kind: 'retry' };
  throw new ExportUploadError('refused', status);
}

/**
 * Write `file` to the session, from wherever the store says it stands: a
 * fresh session answers nothing committed, and a client that restarted picks
 * up where it left off. After a failed chunk it asks the store again rather
 * than trusting what it sent.
 *
 * @throws {ExportUploadError} when the session is gone, the store refuses the
 *   bytes, or {@link UPLOAD_MAX_STALLED_ATTEMPTS} attempts in a row advance
 *   nothing. A store that hides its `Range` header from this client reads as
 *   nothing committed, so that also ends here rather than looping.
 */
export async function uploadToSession(options: SessionUploadOptions): Promise<void> {
  const { sessionUrl, file, signal, onProgress } = options;
  const chunkBytes = options.chunkBytes ?? UPLOAD_CHUNK_BYTES;
  const fetchImpl = options.fetchImpl ?? ((input, init) => fetch(input, init));
  const wait = options.wait ?? ((ms) => new Promise((resolve) => setTimeout(resolve, ms)));
  const total = file.size;

  const send = async (contentRange: string, body?: Blob | ArrayBuffer | Uint8Array): Promise<SessionAnswer> => {
    try {
      const response = await fetchImpl(sessionUrl, {
        method: 'PUT',
        headers: { 'Content-Range': contentRange },
        body: body as BodyInit | undefined,
        credentials: 'omit',
        signal,
      });
      return answerOf(response);
    } catch (error) {
      if (error instanceof ExportUploadError) throw error;
      // Stopped on purpose; anything else is the network, and worth a retry.
      if (signal?.aborted) throw error;
      return { kind: 'retry' };
    }
  };
  const query = () => send(`bytes */${total}`);

  let stalled = 0;
  const pause = async () => {
    stalled += 1;
    if (stalled >= UPLOAD_MAX_STALLED_ATTEMPTS) throw new ExportUploadError('stalled');
    await wait(Math.min(FIRST_RETRY_MS * 2 ** (stalled - 1), MAX_RETRY_MS));
  };

  let committed: number | null = null;
  for (;;) {
    if (committed === null) {
      const answer = await query();
      if (answer.kind === 'done') {
        onProgress?.(total, total);
        return;
      }
      if (answer.kind === 'retry') {
        await pause();
        continue;
      }
      committed = answer.committed;
      onProgress?.(committed, total);
    }
    // The whole file is committed but the store has not closed the object:
    // nothing is left to send, and only a query can finish it.
    const start: number = committed;
    if (start >= total) {
      await pause();
      committed = null;
      continue;
    }
    const end = Math.min(start + chunkBytes, total);
    const bytes = await file.read(start, end);
    const answer = await send(`bytes ${start}-${end - 1}/${total}`, bytes);
    if (answer.kind === 'done') {
      onProgress?.(total, total);
      return;
    }
    if (answer.kind === 'partial' && answer.committed > start) {
      stalled = 0;
      committed = answer.committed;
      onProgress?.(committed, total);
      continue;
    }
    await pause();
    committed = null;
  }
}
