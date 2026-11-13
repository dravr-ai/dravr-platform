// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Settings › Physiology — the athlete's anchors per sport family, each with its value, range, source, date and confidence
// ABOUTME: Read-only: the server words each value as the envelope notice does, a low-confidence one as a range or an estimate (carnet#1516)

import { useTranslation } from '@pierre/i18n';
import {
  PHYSIOLOGY_ANCHOR_LABEL_KEY,
  PHYSIOLOGY_KEY,
  SPORT_FAMILY_LABEL_KEY,
} from '@pierre/shared-constants';
import type { PhysiologyAnchor, SportPhysiology } from '@pierre/shared-types';
import { describeApiError, physiologyAnchorFacts } from '@pierre/ui-logic';
import { EmptyState, Section } from '../ui';
import { usePhysiology } from '../../hooks/usePhysiology';

/**
 * One anchor: its label and the facts under it on the left, its value and
 * who set it on the right — stacked under the label on a narrow pane.
 */
function AnchorRow({ sport, anchor }: { sport: SportPhysiology['sport']; anchor: PhysiologyAnchor }) {
  const { t, language } = useTranslation();
  return (
    <li
      className="flex flex-col gap-1 py-3 sm:flex-row sm:items-start sm:justify-between sm:gap-4"
      data-testid={`physiology-${sport}-${anchor.anchor}`}
    >
      <div className="min-w-0">
        <p className="text-sm font-medium text-on-surface">{t(PHYSIOLOGY_ANCHOR_LABEL_KEY[anchor.anchor])}</p>
        <p className="mt-0.5 text-xs text-on-surface-variant" data-testid={`physiology-${sport}-${anchor.anchor}-facts`}>
          {physiologyAnchorFacts(t, anchor, language).join(' · ')}
        </p>
      </div>
      <div className="min-w-0 sm:shrink-0 sm:text-right">
        <p className="text-sm font-medium tabular-nums text-on-surface" data-testid={`physiology-${sport}-${anchor.anchor}-value`}>
          {anchor.value_text}
        </p>
        <p className="mt-0.5 text-xs text-on-surface-variant" data-testid={`physiology-${sport}-${anchor.anchor}-source`}>
          {anchor.source_text}
        </p>
      </div>
    </li>
  );
}

export default function PhysiologySettings() {
  const { t } = useTranslation();
  const physiology = usePhysiology();

  const renderBody = () => {
    if (physiology.isPending) {
      return <div className="pierre-spinner h-5 w-5" role="status" aria-label={t('common.loading')} />;
    }
    if (physiology.isError) {
      return (
        <p role="alert" className="text-sm text-error" data-testid="physiology-load-failed">
          {describeApiError(physiology.error, { t, fallbackKey: PHYSIOLOGY_KEY.loadFailed })}
        </p>
      );
    }
    if (physiology.data.sports.length === 0) {
      return <EmptyState data-testid="physiology-empty">{t(PHYSIOLOGY_KEY.empty)}</EmptyState>;
    }
    return (
      <div className="space-y-8">
        {physiology.data.sports.map((family) => (
          <Section
            key={family.sport}
            title={t(SPORT_FAMILY_LABEL_KEY[family.sport])}
            headingLevel={3}
            data-testid={`physiology-${family.sport}`}
          >
            <ul className="divide-y divide-outline-variant" aria-label={t(SPORT_FAMILY_LABEL_KEY[family.sport])}>
              {family.anchors.map((anchor) => (
                <AnchorRow key={anchor.anchor} sport={family.sport} anchor={anchor} />
              ))}
            </ul>
          </Section>
        ))}
      </div>
    );
  };

  return (
    <div className="space-y-10" data-testid="physiology-settings">
      <Section title={t(PHYSIOLOGY_KEY.title)} description={t(PHYSIOLOGY_KEY.blurb)}>
        {renderBody()}
      </Section>
    </div>
  );
}
