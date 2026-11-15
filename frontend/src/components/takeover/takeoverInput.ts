// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Turns the athlete's pointer, keys and typed text into a login takeover's input messages
// ABOUTME: Each beforeinput carries its own text, so a phone keyboard's repeated letters and edits arrive exact

import { takeoverTextChunks } from '@pierre/ui-logic';
import type { SciotteTakeoverInput, SciotteTakeoverKey } from '@pierre/shared-types';

/** The keys forwarded as key events; every printable key arrives as text instead. */
const FORWARDED_KEYS: ReadonlySet<string> = new Set<SciotteTakeoverKey>([
  'Backspace',
  'Tab',
  'Enter',
  'Escape',
  'Delete',
  'ArrowLeft',
  'ArrowUp',
  'ArrowRight',
  'ArrowDown',
]);

/**
 * The takeover key a keyboard event names, or `null` for one the viewer
 * leaves to the browser: a printable key (it arrives as `beforeinput` text),
 * a modified one (Cmd+V pastes), and Shift+Tab, which stays the way out of
 * the view for a keyboard user.
 */
export function forwardedKey(event: Pick<KeyboardEvent, 'key' | 'shiftKey' | 'ctrlKey' | 'metaKey' | 'altKey'>): SciotteTakeoverKey | null {
  if (event.ctrlKey || event.metaKey || event.altKey) return null;
  if (event.key === 'Tab' && event.shiftKey) return null;
  return FORWARDED_KEYS.has(event.key) ? (event.key as SciotteTakeoverKey) : null;
}

// The keys a soft keyboard's edits stand for, named once: key names, not words.
const BACKSPACE: SciotteTakeoverKey = 'Backspace';
const DELETE: SciotteTakeoverKey = 'Delete';
const ENTER: SciotteTakeoverKey = 'Enter';

/** A key pressed and released. */
function tap(key: SciotteTakeoverKey): SciotteTakeoverInput[] {
  return [
    { type: 'key', action: 'down', key },
    { type: 'key', action: 'up', key },
  ];
}

/** Text as the messages that carry it, each within the server's size. */
export function textInputs(text: string): SciotteTakeoverInput[] {
  return takeoverTextChunks(text).map((chunk) => ({ type: 'text', text: chunk }));
}

/** What a `beforeinput` on the hidden field carries. */
export interface BeforeInputLike {
  inputType: string;
  data: string | null;
  isComposing?: boolean;
  /** The pasted text, for `insertFromPaste`. */
  pasted?: string | null;
}

/**
 * The messages a `beforeinput` on the hidden field becomes, and whether the
 * viewer takes it (cancelling it, so the field never accumulates text), or
 * `null` to leave it to the browser — composition, which an IME must be let
 * finish and which `compositionend` forwards.
 *
 * Each event carries its own text, so nothing reads or clears the field
 * between keystrokes: the approach a phone keyboard typed repeated letters
 * through exactly, where reading the field's value had dropped them.
 */
export function beforeInputMessages(event: BeforeInputLike): SciotteTakeoverInput[] | null {
  if (event.isComposing) return null;
  switch (event.inputType) {
    case 'insertText':
    case 'insertReplacementText':
      return event.data ? textInputs(event.data) : [];
    case 'deleteContentBackward':
      return tap(BACKSPACE);
    case 'deleteContentForward':
      return tap(DELETE);
    case 'insertLineBreak':
    case 'insertParagraph':
      return tap(ENTER);
    case 'insertFromPaste':
    case 'insertFromDrop':
      return event.pasted ? textInputs(event.pasted) : [];
    default:
      return null;
  }
}

/**
 * Where a pointer at (`clientX`, `clientY`) lands on the page, in the frame's
 * own pixels: the canvas is drawn at whatever size the layout gives it, and
 * the frame's pixels are the page's coordinate space.
 */
export function framePoint(
  clientX: number,
  clientY: number,
  rect: Pick<DOMRect, 'left' | 'top' | 'width' | 'height'>,
  frame: { width: number; height: number },
): { x: number; y: number } | null {
  if (!Number.isFinite(clientX) || !Number.isFinite(clientY)) return null;
  if (rect.width <= 0 || rect.height <= 0 || frame.width <= 0 || frame.height <= 0) return null;
  const x = ((clientX - rect.left) * frame.width) / rect.width;
  const y = ((clientY - rect.top) * frame.height) / rect.height;
  return {
    x: Math.min(Math.max(x, 0), frame.width),
    y: Math.min(Math.max(y, 0), frame.height),
  };
}
