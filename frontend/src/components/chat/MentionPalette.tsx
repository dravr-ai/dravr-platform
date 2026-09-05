// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: @handle autocomplete rendered above the web composer
// ABOUTME: Lists the athlete's installed coaches by handle; holds no coach list of its own

import { clsx } from 'clsx';
import type { MentionCandidate } from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';

export interface MentionPaletteProps {
  /** The installed coaches to offer, already filtered and ordered by handle. */
  matches: MentionCandidate[];
  /** Index of the highlighted row, driven by the composer's arrow keys. */
  highlightedIndex: number;
  /** Insert this coach's handle into the composer. */
  onSelect: (candidate: MentionCandidate) => void;
}

/**
 * The `@` autocomplete over the composer.
 *
 * Every row is a coach on the athlete's own list — the set the server resolves
 * a mention against — so a selected handle is one the next turn will route to.
 * Renders nothing when there is nothing to offer, which is what closes it.
 */
export default function MentionPalette({
  matches,
  highlightedIndex,
  onSelect,
}: MentionPaletteProps) {
  const { t } = useTranslation();
  if (matches.length === 0) return null;

  return (
    <div
      role="listbox"
      aria-label={t('chat.coachMentionsAria')}
      data-testid="mention-palette"
      className="mb-2 max-h-64 overflow-y-auto rounded-xl border ghost-border bg-surface-container-low"
    >
      {matches.map((candidate, index) => (
        <button
          key={candidate.handle}
          type="button"
          role="option"
          aria-selected={index === highlightedIndex}
          data-testid={`mention-palette-option-${candidate.handle}`}
          // Keep the composer focused: a blur here would close the palette
          // before the click lands and the selection would never happen.
          onMouseDown={(e) => e.preventDefault()}
          onClick={() => onSelect(candidate)}
          className={clsx(
            'w-full text-left px-4 py-2.5 flex items-baseline gap-3 transition-colors',
            index === highlightedIndex ? 'bg-surface-container-high' : 'hover:bg-surface-container',
          )}
        >
          <span className="font-mono text-sm text-on-surface whitespace-nowrap">@{candidate.handle}</span>
          <span className="text-xs text-on-surface-variant truncate">{candidate.title}</span>
        </button>
      ))}
    </div>
  );
}
