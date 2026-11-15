// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The pieces every Group info section is built from — a box of rows, a person row, a disclosure row, a quiet exit row
// ABOUTME: One shape for every role's drawer, so the order changes with the viewer and the look never does

import { useId, useState, type ReactNode } from 'react';
import { clsx } from 'clsx';
import { Bot, ChevronRight } from 'lucide-react';
import { avatarSlot, initialsFor } from '@pierre/chat-utils';
import { avatarShapeClass, avatarSlotClass } from '../chat/avatarSlots';

/** The hairline between two rows of one box; the first row of a box draws none. */
export const ROW_DIVIDER = 'border-t ghost-border-faint first:border-t-0';

/**
 * The rows of one section on the drawer's paper: the white shape with a
 * hairline, as the composer is on the thread. It never clips, so a row's
 * `⋯` menu can open over the rows below it.
 */
export function RowBox({ children, className, ...rest }: { children: ReactNode; className?: string; 'data-testid'?: string }) {
  return (
    <div className={clsx('rounded-xl border ghost-border bg-surface-container-lowest', className)} {...rest}>
      {children}
    </div>
  );
}

/** A person's initials on the hue their plan thread wears, so one athlete keeps one colour across the app. */
export function PersonAvatar({ userId, name }: { userId: string; name: string }) {
  const slot = avatarSlot({ id: userId, agent_id: null, group_id: null, subject_user_id: userId });
  return (
    <span
      aria-hidden="true"
      className={clsx(
        'flex h-9 w-9 shrink-0 select-none items-center justify-center text-xs font-semibold',
        avatarShapeClass('circle'),
        avatarSlotClass(slot),
      )}
    >
      {initialsFor(name)}
    </span>
  );
}

/** The agent's glyph, on the quiet tint: it is a party in the room, not a person. */
export function AgentAvatar() {
  return (
    <span
      aria-hidden="true"
      className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-surface-container-high text-on-surface-variant"
    >
      <Bot className="h-[18px] w-[18px]" />
    </span>
  );
}

interface PersonRowProps {
  avatar: ReactNode;
  /** The first line: a name, and what marks the viewer's own row. */
  title: ReactNode;
  /** The second line: a role word and a state, each its own span. */
  detail?: ReactNode;
  /** The row's one control, at its end: a `⋯` menu. */
  trailing?: ReactNode;
  className?: string;
  'data-testid'?: string;
  'data-state'?: string;
}

/** One person per row: a 36px avatar, the name, one line under it, and at most one control. */
export function PersonRow({ avatar, title, detail, trailing, className, ...rest }: PersonRowProps) {
  return (
    <div className={clsx('flex items-center gap-3 px-4 py-2.5', ROW_DIVIDER, className)} {...rest}>
      {avatar}
      <div className="min-w-0 flex-1">
        <div className="truncate text-sm font-medium text-on-surface">{title}</div>
        {detail ? (
          <div className="flex min-w-0 flex-wrap items-center gap-x-1.5 text-xs text-on-surface-variant">{detail}</div>
        ) : null}
      </div>
      {trailing}
    </div>
  );
}

/** A setting read in place: its name on the left, its value on the right, changed under Group settings. */
export function ValueRow({ label, value, ...rest }: { label: string; value: string; 'data-testid'?: string }) {
  return (
    <div className={clsx('flex min-h-11 items-center gap-3 px-4 py-2 text-sm', ROW_DIVIDER)} data-testid={rest['data-testid']}>
      <span className="min-w-0 flex-1 text-on-surface">{label}</span>
      <span className="min-w-0 truncate text-xs text-on-surface-variant">{value}</span>
    </div>
  );
}

/** The dot that says, beside its word, whether a member shares their training with the group. */
export function SharingState({ on, onLabel, offLabel }: { on: boolean; onLabel: string; offLabel: string }) {
  return (
    <span className="inline-flex items-center gap-1.5">
      <span aria-hidden="true" className={clsx('h-2 w-2 shrink-0 rounded-full', on ? 'bg-activity' : 'bg-outline-variant')} />
      <span className={on ? 'font-medium text-on-activity-container' : 'text-on-surface-variant'}>
        {on ? onLabel : offLabel}
      </span>
    </span>
  );
}

interface DisclosureRowProps {
  label: string;
  /** What is folded inside, on the right of the label. */
  hint?: string;
  children: ReactNode;
  'data-testid'?: string;
}

/**
 * A folded section: one 44px row that opens in place. What it holds is
 * mounted only once opened, so a folded room transcript or weekly report
 * costs no request.
 */
export function DisclosureRow({ label, hint, children, ...rest }: DisclosureRowProps) {
  const [open, setOpen] = useState(false);
  const regionId = useId();
  return (
    <div className={ROW_DIVIDER} data-testid={rest['data-testid']}>
      <button
        type="button"
        aria-expanded={open}
        aria-controls={regionId}
        onClick={() => setOpen((was) => !was)}
        className="flex min-h-11 w-full items-center gap-2 rounded-xl px-4 text-left text-sm font-medium text-on-surface transition-colors hover:bg-surface-container-low focus-ring"
      >
        <span className="min-w-0 flex-1 truncate">{label}</span>
        {hint ? <span className="min-w-0 truncate text-xs font-normal text-on-surface-variant">{hint}</span> : null}
        <ChevronRight
          className={clsx('h-4 w-4 shrink-0 text-outline transition-transform', open && 'rotate-90')}
          aria-hidden="true"
        />
      </button>
      {open ? (
        <div id={regionId} className="px-4 pb-4 pt-2">
          {children}
        </div>
      ) : null}
    </div>
  );
}

/**
 * Leaving or deleting, at the very bottom: `error` ink on a text row, the
 * way Settings signs out. The confirm it opens is the one place the danger
 * is filled.
 */
export function QuietExitRow({ label, onClick, ...rest }: { label: string; onClick: () => void; 'data-testid'?: string }) {
  return (
    <button
      type="button"
      onClick={onClick}
      data-testid={rest['data-testid']}
      className="self-start rounded-lg px-1 py-2 text-sm font-medium text-error transition-colors hover:underline focus-ring touch-target"
    >
      {label}
    </button>
  );
}
