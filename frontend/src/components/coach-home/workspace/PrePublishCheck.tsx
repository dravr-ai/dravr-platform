// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The athlete workspace's line before publish — each draft session resting on convention or outside its evidence, not yet the coach's call
// ABOUTME: « C'est mon choix » keeps it as the coach's call, saved on the day; « Revoir » opens its editor (carnet#1814, #1816)

import { useId, useRef } from 'react';
import { TriangleAlert } from 'lucide-react';
import { useTranslation } from '@pierre/i18n';
import { COACH_CALL_KEYS, type PrePublishItem } from '@pierre/chat-utils';
import { PLAN_WORKSPACE_KEYS, ROW_DATE, formatCivilDate } from '@pierre/ui-logic';

interface PrePublishCheckProps {
  items: PrePublishItem[];
  /** The athlete, as the sentence names them. */
  athleteName: string;
  /** « C'est mon choix »: keep the session as the coach's call; resolves whether it was kept. */
  onKeep: (item: PrePublishItem) => Promise<boolean>;
  /** A call is being saved: the buttons wait. */
  keeping: boolean;
  /** « Revoir »: open the editor on the session. */
  onReview: (item: PrePublishItem) => void;
  /** The last session the check named was kept and the check is gone: take focus past it. */
  onCleared: () => void;
}

/** A session's place in the check, as the day writes name it. */
function itemKey(item: PrePublishItem): string {
  return `${item.day.date}-${item.position}`;
}

/**
 * What the coach is asked before publishing: one line per session whose
 * evidence is coaching convention or that sits outside its family's
 * evidence, with the two answers. Nothing at all when there is none — the
 * check is a question, never a standing banner.
 */
export function PrePublishCheck({ items, athleteName, onKeep, keeping, onReview, onCleared }: PrePublishCheckProps) {
  const { t, language } = useTranslation();
  const headingId = useId();
  const hintId = useId();
  const keepButtons = useRef(new Map<string, HTMLButtonElement>());
  if (items.length === 0) return null;

  // A kept session leaves the check with the button that kept it, so focus
  // moves to the next session's « C'est mon choix », or the one before it,
  // and past the check once none is left.
  const keep = (item: PrePublishItem, index: number) => {
    if (keeping) return;
    const next = items[index + 1] ?? items[index - 1];
    void onKeep(item).then((kept) => {
      if (!kept) return;
      const button = next === undefined ? undefined : keepButtons.current.get(itemKey(next));
      if (button === undefined) onCleared();
      else button.focus();
    });
  };

  return (
    <section
      aria-labelledby={headingId}
      className="rounded-lg border ghost-border bg-surface-container-lowest px-4 py-3"
      data-testid="workspace-prepublish"
    >
      <h2 id={headingId} className="m-0 flex items-center gap-2 text-sm font-semibold text-on-surface">
        <TriangleAlert className="h-4 w-4 shrink-0 text-on-warning-container" aria-hidden="true" />
        {t(PLAN_WORKSPACE_KEYS.checkTitle)}
      </h2>
      <ul className="m-0 mt-1 flex list-none flex-col p-0">
        {items.map((item, index) => (
          <li
            key={itemKey(item)}
            className="flex flex-wrap items-center gap-x-4 gap-y-1 border-t ghost-border-faint py-2 first:border-t-0"
            data-testid={`workspace-check-${item.day.date}-${item.position}`}
          >
            <span id={`${headingId}-line-${index}`} className="min-w-[220px] flex-1 text-sm text-on-surface">
              {t(item.reason === 'outside' ? PLAN_WORKSPACE_KEYS.checkOutside : PLAN_WORKSPACE_KEYS.checkConvention, {
                session: item.day.workout,
                date: formatCivilDate(item.day.date, language, ROW_DATE),
                name: athleteName,
              })}
            </span>
            <span className="flex items-center gap-4 text-sm font-medium">
              <button
                type="button"
                ref={(node) => {
                  if (node === null) keepButtons.current.delete(itemKey(item));
                  else keepButtons.current.set(itemKey(item), node);
                }}
                // Held back while a call is saved, never disabled: a disabled
                // button drops the focus it holds.
                aria-disabled={keeping || undefined}
                // Each answer is told apart by the line it answers.
                aria-describedby={`${headingId}-line-${index} ${hintId}`}
                onClick={() => keep(item, index)}
                data-testid={`workspace-check-keep-${item.day.date}-${item.position}`}
                className="rounded text-primary transition-colors hover:text-primary-hover aria-disabled:cursor-not-allowed aria-disabled:opacity-50 focus-ring touch-target"
              >
                {t(COACH_CALL_KEYS.action)}
              </button>
              <button
                type="button"
                aria-haspopup="dialog"
                aria-describedby={`${headingId}-line-${index}`}
                onClick={() => onReview(item)}
                data-testid={`workspace-check-review-${item.day.date}-${item.position}`}
                className="rounded text-primary transition-colors hover:text-primary-hover focus-ring touch-target"
              >
                {t(PLAN_WORKSPACE_KEYS.review)}
              </button>
            </span>
          </li>
        ))}
      </ul>
      {/* What keeping a session as the coach's call means, read with each « C'est mon choix ». */}
      <span id={hintId} className="sr-only">
        {t(COACH_CALL_KEYS.hint)}
      </span>
    </section>
  );
}
