// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: An athlete's anchors per sport family, each with its value, range, source, date and confidence, and its Edit button
// ABOUTME: Settings › Physiology and the coach's view of one athlete list them the same way; the server words each value

import { useTranslation } from '@pierre/i18n';
import { PHYSIOLOGY_ANCHOR_LABEL_KEY, PHYSIOLOGY_EDIT_KEY, SPORT_FAMILY_LABEL_KEY } from '@pierre/shared-constants';
import type { PhysiologyAnchor, SportFamily, SportPhysiology } from '@pierre/shared-types';
import { physiologyAnchorFacts } from '@pierre/ui-logic';
import { Button, Section } from '../ui';

interface AnchorRowProps {
  sport: SportFamily;
  anchor: PhysiologyAnchor;
  onEdit: (sport: SportFamily, anchor: PhysiologyAnchor) => void;
}

/**
 * One anchor: its label and the facts under it on the left, its value and
 * who set it on the right with its Edit button — stacked under the label on
 * a narrow pane. The button's visible word leads its accessible name, the
 * anchor's label after it (WCAG 2.5.3).
 */
function AnchorRow({ sport, anchor, onEdit }: AnchorRowProps) {
  const { t, language } = useTranslation();
  const label = t(PHYSIOLOGY_ANCHOR_LABEL_KEY[anchor.anchor]);
  return (
    <li
      className="flex flex-col gap-1 py-3 sm:flex-row sm:items-start sm:justify-between sm:gap-4"
      data-testid={`physiology-${sport}-${anchor.anchor}`}
    >
      <div className="min-w-0">
        <p className="text-sm font-medium text-on-surface">{label}</p>
        <p className="mt-0.5 text-xs text-on-surface-variant" data-testid={`physiology-${sport}-${anchor.anchor}-facts`}>
          {physiologyAnchorFacts(t, anchor, language).join(' · ')}
        </p>
      </div>
      <div className="flex min-w-0 items-start gap-3 sm:shrink-0">
        <div className="min-w-0 sm:text-right">
          <p className="text-sm font-medium tabular-nums text-on-surface" data-testid={`physiology-${sport}-${anchor.anchor}-value`}>
            {anchor.value_text}
          </p>
          <p className="mt-0.5 text-xs text-on-surface-variant" data-testid={`physiology-${sport}-${anchor.anchor}-source`}>
            {anchor.source_text}
          </p>
        </div>
        <Button
          variant="outline"
          size="sm"
          onClick={() => onEdit(sport, anchor)}
          data-testid={`physiology-${sport}-${anchor.anchor}-edit`}
        >
          {t(PHYSIOLOGY_EDIT_KEY.edit)}
          <span className="sr-only">
            {' '}
            · {label} · {t(SPORT_FAMILY_LABEL_KEY[sport])}
          </span>
        </Button>
      </div>
    </li>
  );
}

interface PhysiologyAnchorListProps {
  sports: SportPhysiology[];
  onEdit: (sport: SportFamily, anchor: PhysiologyAnchor) => void;
}

/** Each sport family the athlete holds an anchor in, as its own section. */
export default function PhysiologyAnchorList({ sports, onEdit }: PhysiologyAnchorListProps) {
  const { t } = useTranslation();
  return (
    <div className="space-y-8">
      {sports.map((family) => (
        <Section
          key={family.sport}
          title={t(SPORT_FAMILY_LABEL_KEY[family.sport])}
          headingLevel={3}
          data-testid={`physiology-${family.sport}`}
        >
          <ul className="divide-y divide-outline-variant" aria-label={t(SPORT_FAMILY_LABEL_KEY[family.sport])}>
            {family.anchors.map((anchor) => (
              <AnchorRow key={anchor.anchor} sport={family.sport} anchor={anchor} onEdit={onEdit} />
            ))}
          </ul>
        </Section>
      ))}
    </div>
  );
}
