// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Whether the athlete is typing in a chat composer right now, read from focus anywhere in the document
// ABOUTME: A phone folds its tab bar and Home's Today line away while it is true, so the composer sits on the keyboard

import { useEffect, useState } from 'react';

/** The chat composer's own field, marked `data-composer="true"` by MessageInput. */
function isComposer(node: EventTarget | null): boolean {
  return node instanceof HTMLElement && node.dataset.composer === 'true';
}

/**
 * True while focus is in a chat composer. Read off the document's focus
 * events rather than threaded through props, because the tab bar that steps
 * aside belongs to the dashboard shell and the composer to the thread inside
 * it. Moving from one composer straight to another stays true.
 */
export function useComposerFocused(): boolean {
  const [focused, setFocused] = useState(() =>
    isComposer(typeof document === 'undefined' ? null : document.activeElement),
  );

  useEffect(() => {
    const onFocusIn = (event: FocusEvent) => setFocused(isComposer(event.target));
    const onFocusOut = (event: FocusEvent) => {
      if (isComposer(event.target)) setFocused(isComposer(event.relatedTarget));
    };
    document.addEventListener('focusin', onFocusIn);
    document.addEventListener('focusout', onFocusOut);
    return () => {
      document.removeEventListener('focusin', onFocusIn);
      document.removeEventListener('focusout', onFocusOut);
    };
  }, []);

  return focused;
}
