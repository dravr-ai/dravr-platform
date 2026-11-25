// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Web dialog behaviour — focus trap, focus restore, Escape and scroll lock — and the popover layer that takes Escape alone
// ABOUTME: One stack of open layers behind one keydown listener, so a key reaches only the topmost dialog or popover

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
 * One layer over the page that a key can dismiss: a dialog, or a popover — a
 * menu, a full-screen overlay — that takes Escape but holds neither focus nor
 * the page's scroll.
 */
interface Layer {
  /** Escape, delivered only while this layer is the top one. */
  onEscape: () => void;
  /**
   * Tab, delivered to the highest layer that has one. A popover has none, so a
   * menu open inside a dialog leaves Tab to that dialog's focus trap.
   */
  onTab?: (event: KeyboardEvent) => void;
  /** Whether the page behind stays frozen while this layer is up. */
  locksScroll: boolean;
  /**
   * The layer's own element, when it has one: what decides its place when a
   * layer inside it registered first (see `pushLayer`).
   */
  node: HTMLElement | null;
}

/**
 * Every open layer, bottom to top; the last is the one on top. A layer goes on
 * in the order it opened, except under a layer registered from inside it (see
 * `pushLayer`).
 *
 * Each dialog and each popover used to listen for keys on `document` or
 * `window` itself, so one Escape reached every layer that was up: a plan day's
 * evidence sheet opened from Home's month sheet closed with the month sheet
 * under it, and a row menu in the history sheet took the sheet down with it
 * (carnet#1553). Tab fought the same way: the lower dialog saw focus outside
 * its own container — the top one is portalled beside it — and pulled it back
 * to its first control.
 *
 * An entry leaves by identity, never by `pop`, so a lower layer closed while
 * the top stays open, a layer unmounted without closing, and StrictMode's
 * mount → cleanup → mount all leave the stack exactly as the screen is.
 */
const layers: Layer[] = [];

/** How many open layers freeze the page; it thaws when the last one closes. */
let scrollLocks = 0;

/**
 * The one listener every open layer shares, so a key reaches the top layer and
 * nothing under it. One listener rather than one per layer checking whether it
 * is on top: a close that commits mid-dispatch would make the next layer down
 * the top one while the same keydown is still being delivered.
 *
 * A key something inside the layer already handled — the rename field's
 * Escape, which cancels the rename; the composer palette's Tab, which
 * completes a command — is that control's, and closes nothing.
 */
function onDocumentKeyDown(event: KeyboardEvent) {
  if (event.defaultPrevented) return;
  if (event.key === 'Escape') {
    layers[layers.length - 1]?.onEscape();
    return;
  }
  if (event.key !== 'Tab') return;
  for (let index = layers.length - 1; index >= 0; index -= 1) {
    const onTab = layers[index].onTab;
    if (onTab) {
      onTab(event);
      return;
    }
  }
}

/**
 * Put a layer on the stack; the returned function takes it off.
 *
 * A layer goes on top, except under a layer already registered from inside its
 * own element. React runs a child's effects before its parent's, so a dialog
 * and a menu or a dialog inside it that open in the same render — the info
 * panel opened with its participants list already expanded — register child
 * first; by push order alone the parent would land on top and take the first
 * Escape. A layer with no element (a popover that passed no ref) keeps push
 * order. Containment is the DOM's: a child portalled out of its parent's
 * element is not seen as inside it, which holds only while such a child opens
 * after its parent, as every portalled sheet here does.
 *
 * The scroll lock is counted rather than toggled. The previous implementation
 * set `document.body.style.overflow = 'hidden'` on open and `'unset'` on
 * close. With two dialogs stacked — the credential login opens the OAuth setup
 * modal over itself — closing the inner one unfroze the page while the outer
 * was still up, and the background scrolled under it.
 */
function pushLayer(layer: Layer): () => void {
  if (layers.length === 0) {
    document.addEventListener('keydown', onDocumentKeyDown);
  }
  const { node } = layer;
  const firstInside =
    node === null ? -1 : layers.findIndex((other) => other.node !== null && other.node !== node && node.contains(other.node));
  layers.splice(firstInside === -1 ? layers.length : firstInside, 0, layer);
  if (layer.locksScroll) {
    if (scrollLocks === 0) document.body.style.overflow = 'hidden';
    scrollLocks += 1;
  }
  return () => {
    const index = layers.indexOf(layer);
    if (index === -1) return;
    layers.splice(index, 1);
    if (layer.locksScroll) {
      scrollLocks -= 1;
      if (scrollLocks === 0) document.body.style.overflow = '';
    }
    if (layers.length === 0) {
      document.removeEventListener('keydown', onDocumentKeyDown);
    }
  };
}

/**
 * Make an open popover the layer Escape closes first.
 *
 * For a menu or an overlay that is not a dialog: while `active`, one Escape
 * runs `onEscape` and reaches nothing under it — not the sheet the menu sits
 * in, not the page. It holds no focus trap and no scroll lock; Tab stays with
 * the dialog under it, if there is one. A dialog takes `useDialog` instead.
 *
 * Returns a ref for the popover's element. Put it on the element so a dialog
 * that contains the popover and opens in the same render still lands under it.
 */
export function useEscapeLayer<T extends HTMLElement = HTMLDivElement>(active: boolean, onEscape: () => void) {
  const nodeRef = useRef<T>(null);
  // Stable across renders, so a parent's new closure does not re-stack the
  // layer above one that opened after it.
  const onEscapeRef = useRef(onEscape);
  onEscapeRef.current = onEscape;

  useEffect(() => {
    if (!active) return;
    return pushLayer({ onEscape: () => onEscapeRef.current(), locksScroll: false, node: nodeRef.current });
  }, [active]);

  return nodeRef;
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
export function useDialog<T extends HTMLElement = HTMLDivElement>({
  open,
  onClose,
  closeOnEscape = true,
}: UseDialogOptions) {
  const containerRef = useRef<T>(null);
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
    // The node itself, not the ref: React detaches the ref before this
    // effect's cleanup runs, and StrictMode's rehearsal cleanup then read
    // focus as "outside", kept it on the dialog's own first control, and the
    // rerun took that control as the opener — focus went to <body> on close.
    const container = containerRef.current;

    const onTab = (event: KeyboardEvent) => {
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

    const leaveStack = pushLayer({
      // A dialog that must be resolved still takes Escape, and so still
      // shields every layer under it.
      onEscape: () => {
        if (closeOnEscapeRef.current) onCloseRef.current();
      },
      onTab,
      locksScroll: true,
      node: container,
    });

    // Initial focus: the first real control, so a keyboard user starts on the
    // email field rather than having to Tab in from the container.
    const items = focusables();
    (items[0] ?? containerRef.current)?.focus();

    return () => {
      leaveStack();
      // Only take focus back if it is still inside the dialog we are tearing
      // down; a close that already moved focus somewhere deliberate wins.
      const active = document.activeElement;
      const stillInside = active instanceof Node && (container?.contains(active) ?? false);
      if ((stillInside || active === document.body) && restoreTo?.isConnected) {
        restoreTo.focus();
      }
    };
  }, [open, focusables]);

  return { containerRef };
}
