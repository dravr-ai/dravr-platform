// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The "Bring in your history" sheet a provider card opens: sign in once to copy past workouts, import the export, or not now
// ABOUTME: Renders the card's history choices from connectPlan; the export is offered only for a source the server put on the card

import { useId } from 'react';
import { useTranslation } from '@pierre/i18n';
import { HISTORY_ACTION_KEYS, type ConnectPlanCard } from '@pierre/shared-constants';
import type { ExtendedProviderStatus, SciotteTarget } from '@pierre/shared-types';
import { importSourceCopy, shownHistoryChoices } from '@pierre/ui-logic';
import { Sheet } from '../ui/Sheet';
import { useIsMobile } from '../../hooks/useBreakpoint';
import { importWindowHash, openImportWindow } from './openImportWindow';

/** The card the sheet is for: what its plan reads, and the name its copy says. */
export type BringHistoryCard = ConnectPlanCard & Pick<ExtendedProviderStatus, 'display_name'>;

interface BringHistorySheetProps {
  card: BringHistoryCard;
  /** The athlete chose the one-time sign-in for `target`; the sheet has closed. */
  onSignIn: (target: SciotteTarget) => void;
  onClose: () => void;
}

// One row per choice: the action in the body ink, its hint under it, a faint
// hairline between rows (DESIGN.md §5 list rows).
const CHOICE_CLASS =
  'group flex w-full items-start gap-3 border-t ghost-border-faint px-4 py-3 text-left first:border-t-0 hover:bg-surface-container-low focus:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-primary/50';

export default function BringHistorySheet({ card, onSignIn, onClose }: BringHistorySheetProps) {
  const { t } = useTranslation();
  const isMobile = useIsMobile();
  const bodyId = useId();
  const name = { provider: card.display_name };
  const choices = shownHistoryChoices(card);

  return (
    <Sheet
      side={isMobile ? 'bottom' : 'right'}
      title={t('providers.history.sheetTitle', name)}
      onClose={onClose}
      data-testid="bring-history-sheet"
    >
      <p id={bodyId} className="px-4 pb-3 text-sm text-on-surface-variant">
        {t('providers.history.sheetBody', name)}
      </p>
      <div role="group" aria-describedby={bodyId} className="border-y ghost-border-faint">
        {choices.map((choice) => {
          if (choice.kind === 'sign_in') {
            return (
              <button
                key="sign_in"
                type="button"
                className={CHOICE_CLASS}
                data-testid="bring-history-sign-in"
                onClick={() => {
                  onClose();
                  onSignIn(choice.target);
                }}
              >
                <span className="flex min-w-0 flex-1 flex-col">
                  <span className="text-sm font-medium text-on-surface">{t('providers.history.choiceSignIn')}</span>
                  <span className="text-xs text-on-surface-variant">{t(HISTORY_ACTION_KEYS.explainer, name)}</span>
                </span>
                <span aria-hidden="true" className="text-on-surface-variant group-hover:text-on-surface">›</span>
              </button>
            );
          }
          // A link export (Strava, Garmin) is asked for, then its emailed link
          // pasted; an uploaded one (WHOOP, COROS) says where its file is found.
          const copy = importSourceCopy(choice.source);
          if (!copy) return null;
          return (
            <button
              key={`archive-${choice.source}`}
              type="button"
              className={CHOICE_CLASS}
              data-testid="bring-history-archive"
              data-source={choice.source}
              onClick={() => {
                onClose();
                openImportWindow(importWindowHash(choice.source));
              }}
            >
              <span className="flex min-w-0 flex-1 flex-col">
                <span className="text-sm font-medium text-on-surface">{t('providers.history.choiceArchive', name)}</span>
                <span className="text-xs text-on-surface-variant">
                  {copy.intake === 'link' ? t('providers.history.choiceArchiveHint', name) : t(copy.hint)}
                </span>
              </span>
              <span aria-hidden="true" className="text-on-surface-variant group-hover:text-on-surface">›</span>
            </button>
          );
        })}
      </div>
      <div className="flex justify-end px-4 py-3">
        <button
          type="button"
          onClick={onClose}
          data-testid="bring-history-not-now"
          className="rounded-lg px-3 py-2 text-sm font-medium text-primary hover:bg-surface-container-low hover:text-primary-hover focus:outline-none focus-visible:ring-2 focus-visible:ring-primary/50"
        >
          {t('providers.history.notNow')}
        </button>
      </div>
    </Sheet>
  );
}
