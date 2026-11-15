// ABOUTME: A plan day's evidence chip — where the session's evidence sits — opening a sheet with the studies behind it
// ABOUTME: The chip's words, its citation and the note in its place arrive resolved by the server; only the sheet's title is the client's

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState } from 'react';
import { useTranslation } from '@pierre/i18n';
import type { PlanDayEvidence } from '@pierre/shared-types';
import { Sheet } from '../ui/Sheet';
import { useIsMobile } from '../../hooks/useBreakpoint';

/**
 * The neutral chip a plan day carries when its evidence is weaker than a
 * trial in its sport (carnet#1495). It opens a sheet naming the studies the
 * evidence rests on, never the catalogue's English caveat, or, when the badge
 * cites none, the server's note saying why.
 */
export function EvidenceChip({ evidence }: { evidence: PlanDayEvidence }) {
  const { t } = useTranslation();
  const isMobile = useIsMobile();
  const [open, setOpen] = useState(false);
  const detail = evidence.basis ?? evidence.note;
  return (
    <>
      <button
        type="button"
        className="ml-2 badge focus-ring"
        aria-haspopup="dialog"
        onClick={() => setOpen(true)}
        data-testid="plan-day-evidence"
      >
        {evidence.label}
      </button>
      {open && (
        <Sheet
          side={isMobile ? 'bottom' : 'right'}
          title={t('plan.card.evidence.sheetTitle')}
          onClose={() => setOpen(false)}
          data-testid="plan-day-evidence-sheet"
        >
          <div className="space-y-2 px-4 py-4 text-sm">
            <p className="font-medium text-on-surface">{evidence.label}</p>
            {detail && <p className="text-on-surface-variant">{detail}</p>}
          </div>
        </Sheet>
      )}
    </>
  );
}
