// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Web dialog behaviour — focus trap, focus restore, Escape, refcounted scroll lock
// ABOUTME: One implementation, because the two hand-rolled overlays had none of it at all

import { useCallback, useEffect, useRef } from 'react';

/**
 * Everything inside a dialog that a keyboard can land on.
 *
 * `:not([disabled])` and the negative-tabindex exclusion matter: a disabled
 * submit button is the last child of most of our forms, and trapping onto it
 * would strand the cycle on something that cannot be activated.
 */
const FOCUSABLE = [
  'a[href]',
  'button:not([disabled])',
  'input:not([disabled])',
  'select:not([disabled])',
  'textarea:not([disabled])',
  '[tabindex]:not([tabindex="-1"])',
].join(',');

/**
 * How many dialogs currently want the page behind them frozen.
 *
 * The previous implementation set `document.body.style.overflow = 'hidden'` on
 * open and `'unset'` on close. With two dialogs stacked — the Sciotte login
 * opens the OAuth setup modal over itself — closing the inner one unfroze the
 * page while the outer was still up, and the background scrolled under it.
 */
let scrollLockDepth = 0;

function lockScroll(): () => void {
  if (scrollLockDepth === 0) {
    document.body.style.overflow = 'hidden';
  }
  scrollLockDepth += 1;
  let released = false;
  return () => {
    if (released) return;
    released = true;
    scrollLockDepth = Math.max(0, scrollLockDepth - 1);
    if (scrollLockDepth === 0) {
      document.body.style.overflow = '';
    }
  };
}

export interface UseDialogOptions {
  open: boolean;
  onClose: () => void;
  /** Default true. Off for a dialog that must be resolved, not dismissed. */
  closeOnEscape?: boolean;
}

/**
 * Wire a dialog up to the keyboard the way the ARIA authoring practices expect.
 *
 * `aria-modal="true"` tells assistive tech the rest of the page is inert, but
 * it does not constrain Tab — a sighted keyboard user tabbed straight out of
 * our modals and into the page behind, where every control was still live and
 * invisible focus moved through it. And closing dropped focus onto `<body>`,
 * so the next Tab restarted from the top of the document instead of returning
 * to the control that opened the dialog.
 *
 * Returns the ref to put on the dialog container. The container should keep
 * `tabIndex={-1}` so it can hold focus when it has no focusable children yet.
 */
export function useDialog({ open, onClose, closeOnEscape = true }: UseDialogOptions) {
  const containerRef = useRef<HTMLDivElement>(null);
  // Stable across renders so the effect below does not re-run — and re-steal
  // focus — every time a parent re-renders with a new closure.
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;
  const closeOnEscapeRef = useRef(closeOnEscape);
  closeOnEscapeRef.current = closeOnEscape;

  const focusables = useCallback((): HTMLElement[] => {
    const root = containerRef.current;
    if (!root) return [];
    return Array.from(root.querySelectorAll<HTMLElement>(FOCUSABLE)).filter((el) => {
      // NOT `offsetParent !== null`: that is null for anything inside a
      // `position: fixed` ancestor, which is every dialog overlay we render —
      // the filter would have emptied the list and quietly disabled the trap
      // in exactly the case it exists for. `hidden` and `aria-hidden` are the
      // two states that genuinely take a control out of the tab order without
      // the selector above already excluding it.
      if (el.closest('[hidden]') !== null) return false;
      if (el.closest('[aria-hidden="true"]') !== null) return false;
      return true;
    });
  }, []);

  useEffect(() => {
    if (!open) return;

    // Whatever had focus when the dialog opened gets it back on close.
    const restoreTo = document.activeElement as HTMLElement | null;
    const releaseScroll = lockScroll();

    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape' && closeOnEscapeRef.current) {
        onCloseRef.current();
        return;
      }
      if (event.key !== 'Tab') return;

      const items = focusables();
      if (items.length === 0) {
        // Nothing to cycle through — keep focus on the container rather than
        // letting Tab escape into the page behind.
        event.preventDefault();
        containerRef.current?.focus();
        return;
      }
      const first = items[0];
      const last = items[items.length - 1];
      const active = document.activeElement;

      if (event.shiftKey && (active === first || active === containerRef.current)) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && active === last) {
        event.preventDefault();
        first.focus();
      } else if (active instanceof Node && !containerRef.current?.contains(active)) {
        // Focus was outside the dialog entirely (a stray programmatic blur, or
        // the browser restoring focus after an alert). Pull it back in.
        event.preventDefault();
        first.focus();
      }
    };

    document.addEventListener('keydown', onKeyDown);

    // Initial focus: the first real control, so a keyboard user starts on the
    // email field rather than having to Tab in from the container.
    const items = focusables();
    (items[0] ?? containerRef.current)?.focus();

    return () => {
      document.removeEventListener('keydown', onKeyDown);
      releaseScroll();
      // Only take focus back if it is still inside the dialog we are tearing
      // down; a close that already moved focus somewhere deliberate wins.
      const active = document.activeElement;
      const stillInside =
        active instanceof Node && (containerRef.current?.contains(active) ?? false);
      if ((stillInside || active === document.body) && restoreTo?.isConnected) {
        restoreTo.focus();
      }
    };
  }, [open, focusables]);

  return { containerRef };
}
