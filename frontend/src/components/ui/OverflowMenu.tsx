// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The one `⋯` menu on the web — a single trigger and a short list of actions, opened in place over the content
// ABOUTME: Arrow keys move between items; Escape closes it alone and hands focus back, never the sheet or drawer it sits in

import { useEffect, useId, useRef, useState, type KeyboardEvent, type ReactNode } from 'react';
import { clsx } from 'clsx';
import { MoreHorizontal, MoreVertical } from 'lucide-react';
import { IconButton } from './IconButton';
import { useEscapeLayer } from '../../hooks/useDialog';

export interface OverflowMenuItem {
  label: string;
  onSelect: () => void;
  /** A glyph before the label, in its own ink. */
  icon?: ReactNode;
  /** A destructive action: `error` ink, never a fill — the fill belongs to the confirm it opens. */
  danger?: boolean;
  'data-testid'?: string;
}

export interface OverflowMenuProps {
  /** Names the trigger, and the menu with it: "More actions for Léa Gagnon". */
  label: string;
  items: OverflowMenuItem[];
  /**
   * The open state, for a host that opens the menu another way too — a
   * conversation row opens it on a right-click. Left out, the menu keeps
   * its own.
   */
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
  /** `tonal` sits on a row that shows it only on hover; `ghost` (the default) is the quiet ink trigger. */
  variant?: 'ghost' | 'tonal';
  size?: 'sm' | 'md';
  /** Three dots stacked, where a row's other glyphs run vertically; three in a line by default. */
  vertical?: boolean;
  /** Classes for the wrapper: how a host places the trigger, or hides it until the row is hovered. */
  className?: string;
  'data-testid'?: string;
  'data-menu-testid'?: string;
}

/**
 * One trigger, one short list. Destruction never shows on the row itself:
 * the row carries this quiet `⋯`, the menu names what it does, and the
 * confirm dialog it opens is where the danger is drawn.
 *
 * Every row menu on the web is this one — Group info's member, coach and
 * invite rows, a conversation row, a verdict card — so they share one
 * keyboard: the first item takes focus, ArrowUp and ArrowDown move, Tab
 * leaves, and Escape closes the menu alone and returns focus to the trigger.
 */
export function OverflowMenu({
  label,
  items,
  open: openProp,
  onOpenChange,
  variant = 'ghost',
  size = 'md',
  vertical = false,
  className,
  ...rest
}: OverflowMenuProps) {
  const [ownOpen, setOwnOpen] = useState(false);
  const open = openProp ?? ownOpen;
  const rootRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const itemRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const menuId = useId();
  const onOpenChangeRef = useRef(onOpenChange);
  onOpenChangeRef.current = onOpenChange;

  const setOpen = (next: boolean) => {
    if (openProp === undefined) setOwnOpen(next);
    onOpenChangeRef.current?.(next);
  };

  useEffect(() => {
    if (!open) return;
    itemRefs.current[0]?.focus();
    const onPointerDown = (event: MouseEvent) => {
      if (rootRef.current && !rootRef.current.contains(event.target as Node)) {
        if (openProp === undefined) setOwnOpen(false);
        onOpenChangeRef.current?.(false);
      }
    };
    document.addEventListener('mousedown', onPointerDown);
    return () => document.removeEventListener('mousedown', onPointerDown);
  }, [open, openProp]);

  const close = () => {
    setOpen(false);
    triggerRef.current?.focus();
  };

  // The open menu is the top layer on useDialog's stack: one Escape closes it
  // alone, wherever focus sits, and the sheet or drawer under it stays up.
  const menuRef = useEscapeLayer<HTMLDivElement>(open, close);

  const onMenuKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const enabled = itemRefs.current.filter((item): item is HTMLButtonElement => item !== null);
    const at = enabled.indexOf(document.activeElement as HTMLButtonElement);
    if (event.key === 'ArrowDown') {
      event.preventDefault();
      enabled[(at + 1) % enabled.length]?.focus();
    } else if (event.key === 'ArrowUp') {
      event.preventDefault();
      enabled[(at - 1 + enabled.length) % enabled.length]?.focus();
    } else if (event.key === 'Tab') {
      setOpen(false);
    }
  };

  if (items.length === 0) return null;

  const Dots = vertical ? MoreVertical : MoreHorizontal;

  return (
    <div ref={rootRef} className={clsx('relative shrink-0', className)}>
      <IconButton
        ref={triggerRef}
        variant={variant}
        size={size}
        onClick={() => setOpen(!open)}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={open ? menuId : undefined}
        aria-label={label}
        title={label}
        data-testid={rest['data-testid']}
      >
        <Dots className={size === 'sm' ? 'h-4 w-4' : 'h-[18px] w-[18px]'} aria-hidden="true" />
      </IconButton>
      {open && (
        <div
          ref={menuRef}
          id={menuId}
          role="menu"
          aria-label={label}
          onKeyDown={onMenuKeyDown}
          data-testid={rest['data-menu-testid']}
          className="absolute right-0 top-full z-30 mt-1 w-max min-w-[12rem] max-w-[min(18rem,calc(100vw-2rem))] rounded-xl border ghost-border bg-surface-container-lowest p-1.5 shadow-floating"
        >
          {items.map((item, index) => (
            <button
              key={item.label}
              ref={(node) => {
                itemRefs.current[index] = node;
              }}
              type="button"
              role="menuitem"
              onClick={() => {
                // Focus goes back to the trigger first, so a confirm the item
                // opens returns there when it closes, not to a detached item.
                triggerRef.current?.focus();
                setOpen(false);
                item.onSelect();
              }}
              data-testid={item['data-testid']}
              className={clsx(
                'flex w-full items-center gap-2.5 rounded-lg px-3 py-2 text-left text-sm transition-colors hover:bg-surface-container-low focus-visible:bg-surface-container-low focus-visible:outline-none touch-target',
                item.danger ? 'text-error' : 'text-on-surface',
              )}
            >
              {item.icon}
              <span>{item.label}</span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
