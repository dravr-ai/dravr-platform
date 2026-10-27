// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Reads a chat turn's response body chunk by chunk, as the runtime receives it — on web and on the phone
// ABOUTME: The chat domain feeds it to parseTurnBody on both clients, so a step frame paints the moment it lands on either

/**
 * Reads a response body progressively, as the runtime receives it.
 *
 * This is what lets a chat turn paint while the model is still working: each
 * chunk is decoded and handed on the moment it lands, instead of waiting for
 * the connection to close. It yields the body as text, so the turn parser has
 * one code path and never asks which platform it is on. A browser's `fetch`
 * streams its body, and so does the phone's — Expo 57's runtime installs
 * `expo/fetch` as the global `fetch`, whose `Response.body` is a
 * `ReadableStream` (and installs the `TextDecoder` this decodes with).
 *
 * A response with no body stream is still read, whole: that is React Native's
 * own XHR-backed `fetch`, which a build gets back only by setting
 * `EXPO_PUBLIC_USE_RN_FETCH`. Its turn arrives complete, steps and reply
 * together, through the same parser.
 *
 * Aborting the request errors the stream, so the pending read rejects and the
 * caller sees the abort. A consumer that stops early (the parser returns on a
 * terminal frame) cancels the rest of the body rather than leaving the
 * connection draining into nothing.
 */
export async function* readBodyProgressively(response: Response): AsyncGenerator<string> {
  const body = response.body;
  if (!body) {
    yield await response.text();
    return;
  }
  const reader = body.getReader();
  const decoder = new TextDecoder();
  let drained = false;
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) {
        drained = true;
        break;
      }
      yield decoder.decode(value, { stream: true });
    }
    // Flush whatever the decoder was holding for a split multi-byte sequence.
    const tail = decoder.decode();
    if (tail) yield tail;
  } finally {
    if (!drained) {
      // Nothing waits on the cancellation: an aborted stream rejects it, and
      // the outcome the caller acts on is already decided.
      reader.cancel().catch(() => undefined);
    }
    reader.releaseLock();
  }
}
