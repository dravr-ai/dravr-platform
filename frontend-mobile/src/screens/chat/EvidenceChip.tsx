// ABOUTME: A plan day's evidence chip — where the session's evidence sits — opening a sheet with the studies behind it
// ABOUTME: The chip's words, its citation and the note in its place arrive resolved by the server; only the sheet's title is the app's

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React, { useState } from 'react';
import { Pressable, Text } from 'react-native';
import { useTranslation } from '@pierre/i18n';
import type { PlanDayEvidence } from '@pierre/shared-types';
import { Sheet } from '../../components/ui';

/**
 * The sheet a chip opens: the studies the evidence rests on, never the
 * catalogue's English caveat, or, when the badge cites none, the server's note
 * saying why.
 */
export function EvidenceSheet({
  evidence,
  visible,
  onClose,
}: {
  evidence: PlanDayEvidence;
  visible: boolean;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const detail = evidence.basis ?? evidence.note;
  return (
    <Sheet visible={visible} onClose={onClose} testID="plan-day-evidence-sheet">
      <Text className="text-lg font-semibold text-text-primary">{t('plan.card.evidence.sheetTitle')}</Text>
      <Text className="mt-2 text-sm font-medium text-text-primary">{evidence.label}</Text>
      {detail ? <Text className="mt-1 text-sm text-text-secondary">{detail}</Text> : null}
    </Sheet>
  );
}

/**
 * The neutral chip a plan day carries when its evidence is weaker than a
 * trial in its sport (carnet#1495). A press opens the {@link EvidenceSheet}.
 * Its own press target, so a row that opens a draft on press (Home) still
 * lets the chip open its sheet; the slop makes the 24-tall chip a 44 target.
 */
export function EvidenceChip({ evidence }: { evidence: PlanDayEvidence }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <Pressable
        className="mt-1 self-start rounded-full bg-surface-container-high px-3 py-1"
        accessibilityRole="button"
        accessibilityLabel={evidence.label}
        hitSlop={10}
        onPress={() => setOpen(true)}
        testID="plan-day-evidence"
      >
        <Text className="text-xs font-medium text-text-primary">{evidence.label}</Text>
      </Pressable>
      <EvidenceSheet evidence={evidence} visible={open} onClose={() => setOpen(false)} />
    </>
  );
}
