// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tells whether the page holds text the athlete typed that a reload would discard
// ABOUTME: Counts a field only once the athlete edited it, so text the page pre-filled never asks

/** Input types whose value is text the athlete typed. */
const TYPED_INPUT_TYPES = new Set(['', 'text', 'search', 'email', 'password', 'url', 'tel', 'number']);

/** An editable region, as the attribute declares it. */
const EDITABLE_SELECTOR = '[contenteditable=""], [contenteditable="true"]';

/**
 * The fields and editable regions the athlete has edited since the page loaded.
 *
 * A non-empty value alone does not mean typed text: a settings form arrives
 * filled with the saved display name, and reloading over it loses nothing.
 * `defaultValue` cannot tell the two apart, because React keeps the `value`
 * attribute of a controlled input in step with the live value, so every field
 * would read as untouched — the composer draft included. An `input` event is
 * what only an edit fires, so the fields it reaches are recorded here.
 */
const edited = new WeakSet<Element>();

/** Whether `field` is a textarea or an input whose value is typed text. */
function isTypedField(field: Element): field is HTMLInputElement | HTMLTextAreaElement {
  return (
    field instanceof HTMLTextAreaElement ||
    (field instanceof HTMLInputElement &&
      TYPED_INPUT_TYPES.has(field.getAttribute('type')?.toLowerCase() ?? ''))
  );
}

/** The outermost editable region holding `node`, which is the one an edit changes. */
function editingHost(node: Element): Element | null {
  let host = node.closest(EDITABLE_SELECTOR);
  let outer = host?.parentElement?.closest(EDITABLE_SELECTOR) ?? null;
  while (outer) {
    host = outer;
    outer = outer.parentElement?.closest(EDITABLE_SELECTOR) ?? null;
  }
  return host;
}

/** Record the field or editable region an `input` event came from. */
function recordEdit(event: Event): void {
  const target = event.target;
  if (!(target instanceof Element)) {
    return;
  }
  if (isTypedField(target)) {
    edited.add(target);
    return;
  }
  const host = editingHost(target);
  if (host) {
    edited.add(host);
  }
}

// Capture phase, so a handler that stops the event's propagation cannot hide
// an edit. Guarded for environments that load the module without a DOM.
if (typeof document !== 'undefined') {
  document.addEventListener('input', recordEdit, true);
}

/**
 * Whether the page holds text the athlete typed and a reload would discard:
 * a text field, textarea or editable region that the athlete edited, that is
 * still on the page, and that is still non-empty. A value the page filled in
 * itself does not count until the athlete edits it, and a draft typed and
 * then cleared counts no longer.
 */
export function typedTextWouldBeLost(): boolean {
  const fields = document.querySelectorAll('textarea, input');
  for (const field of fields) {
    if (edited.has(field) && isTypedField(field) && field.value.trim() !== '') {
      return true;
    }
  }
  const editable = document.querySelectorAll(EDITABLE_SELECTOR);
  for (const region of editable) {
    if (edited.has(region) && (region.textContent ?? '').trim() !== '') {
      return true;
    }
  }
  return false;
}
