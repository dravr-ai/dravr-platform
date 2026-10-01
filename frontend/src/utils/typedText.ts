// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tells whether the page holds text the athlete typed that a reload would discard
// ABOUTME: Read by the service worker update prompt to decide between reloading and asking

/** Input types whose value is text the athlete typed. */
const TYPED_INPUT_TYPES = new Set(['', 'text', 'search', 'email', 'password', 'url', 'tel', 'number']);

/**
 * Whether the page holds text the athlete typed and a reload would discard:
 * a non-empty text field, textarea or editable region.
 */
export function typedTextWouldBeLost(): boolean {
  const fields = document.querySelectorAll<HTMLInputElement | HTMLTextAreaElement>(
    'textarea, input',
  );
  for (const field of fields) {
    const typed =
      field instanceof HTMLTextAreaElement ||
      TYPED_INPUT_TYPES.has(field.getAttribute('type')?.toLowerCase() ?? '');
    if (typed && field.value.trim() !== '') {
      return true;
    }
  }
  const editable = document.querySelectorAll('[contenteditable=""], [contenteditable="true"]');
  for (const region of editable) {
    if ((region.textContent ?? '').trim() !== '') {
      return true;
    }
  }
  return false;
}
