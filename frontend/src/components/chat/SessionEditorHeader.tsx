// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The day editor's head — the session's title written in place as the headline, and the line under it: sport · minutes · span (carnet#1727)
// ABOUTME: In the family mode the headline is the format's name, read only, with the minutes the kernel's preview measured; plus the edited row

import { useId } from 'react';
import { useTranslation } from '@pierre/i18n';
import type { SessionSport } from '@pierre/shared-types';
import { SESSION_EDITOR_KEYS, handSummary, sportLabel, type HandSession } from '@pierre/ui-logic';
import { Button, Input } from '../ui';

/** The headline the family's format is read in, as the session's title is written in. */
const HEADLINE = 'py-1 font-display text-[1.5rem] font-semibold leading-8 text-on-surface sm:text-[1.625rem]';

/** What the family mode's head shows: the format's name, its sport and the minutes its preview measured. */
export interface FamilyHead {
  name: string | null;
  sport: SessionSport | null;
  minutes: number | null;
}

interface SessionEditorHeaderProps {
  hand: HandSession;
  onTitle: (workout: string) => void;
  /** The family mode's head, while that mode is on screen; `null` in the written mode. */
  family: FamilyHead | null;
}

/**
 * The session's title as a large field on a dashed underline — named
 * "Title" for a reader, "Session title" as its placeholder — and under it
 * the sport, the minutes its steps total (else the minutes stated) and the
 * span their targets cover (else the intensity as written).
 */
export function SessionEditorHeader({ hand, onTitle, family }: SessionEditorHeaderProps) {
  const { t } = useTranslation();
  const minutes = (count: number | null) =>
    count === null ? null : t(SESSION_EDITOR_KEYS.minutes, { count: String(count) });

  if (family !== null) {
    if (family.name === null) return null;
    return (
      <div data-testid="session-editor-family-title">
        <p className={HEADLINE}>{family.name}</p>
        <SummaryLine parts={[family.sport ? sportLabel(t, family.sport) : null, minutes(family.minutes)]} />
      </div>
    );
  }

  const summary = handSummary(hand);
  const sport = hand.sport.trim();
  return (
    <div>
      <Input
        size="title"
        aria-label={t(SESSION_EDITOR_KEYS.workout)}
        placeholder={t(SESSION_EDITOR_KEYS.titlePlaceholder)}
        value={hand.workout}
        autoComplete="off"
        onChange={(event) => onTitle(event.target.value)}
        data-testid="session-editor-workout"
      />
      <SummaryLine parts={[sport ? sportLabel(t, sport) : null, minutes(summary.minutes), summary.span]} />
    </div>
  );
}

/** The part of the summary line that is a figure: the minutes, set in mono and body ink. */
const FIGURE_PART = 1;

/** The line under the headline: the parts it has, one dot between each. */
function SummaryLine({ parts }: { parts: readonly (string | null)[] }) {
  const shown = parts.flatMap((text, at) => (text ? [{ text, at }] : []));
  if (shown.length === 0) return null;
  return (
    <p className="mt-2 text-sm text-on-surface-variant" data-testid="session-editor-summary">
      {shown.map(({ text, at }, index) => (
        <span key={at}>
          {index > 0 && ' · '}
          <span className={at === FIGURE_PART ? 'font-mono text-on-surface' : undefined}>{text}</span>
        </span>
      ))}
    </p>
  );
}

/**
 * The quiet row an edited or added session opens with: that the coach
 * changed it, and "Back to the original", described by what it puts back —
 * or that it takes an added session off. The sheet asks before it does either.
 */
export function EditedRow({
  added,
  note,
  onRestore,
}: {
  /** Whether the coach added the session, rather than edited one that was there. */
  added: boolean;
  note: string;
  onRestore: () => void;
}) {
  const { t } = useTranslation();
  const noteId = useId();
  return (
    <div
      className="flex items-start justify-between gap-3 rounded-lg bg-surface-container-low px-3 py-2.5"
      data-testid="session-editor-edited"
    >
      <div className="min-w-0 space-y-0.5">
        <p className="text-sm text-on-surface">
          {t(added ? SESSION_EDITOR_KEYS.addedNote : SESSION_EDITOR_KEYS.editedNote)}
        </p>
        <p id={noteId} className="text-xs text-on-surface-variant">
          {note}
        </p>
      </div>
      <Button
        variant="tertiary"
        size="sm"
        className="-mr-1 shrink-0"
        aria-haspopup="dialog"
        aria-describedby={noteId}
        onClick={onRestore}
        data-testid="session-editor-restore"
      >
        {t(SESSION_EDITOR_KEYS.restoreOriginal)}
      </Button>
    </div>
  );
}
