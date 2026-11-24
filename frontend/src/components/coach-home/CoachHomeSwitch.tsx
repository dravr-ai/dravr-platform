// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The "Mes athlètes / Mon entraînement" switch a coach who also trains finds at the top of Home (D2, carnet#1253)
// ABOUTME: Two pressed-state buttons in one pill (ui/SegmentedControl); the first is the coach's Home, the second today's athlete Home

import { useTranslation } from '@pierre/i18n';
import { SegmentedControl } from '../ui';

type HomeScope = 'athletes' | 'me';

interface CoachHomeSwitchProps {
  scope: HomeScope;
  onChange: (scope: HomeScope) => void;
  /** Fill the row, for a phone. */
  stretch?: boolean;
}

export function CoachHomeSwitch({ scope, onChange, stretch = false }: CoachHomeSwitchProps) {
  const { t } = useTranslation();
  return (
    <SegmentedControl
      value={scope}
      onChange={onChange}
      stretch={stretch}
      aria-label={t('coachHome.switch.aria')}
      data-testid="coach-home-switch"
      options={[
        { value: 'athletes', label: t('coachHome.switch.athletes'), 'data-testid': 'coach-home-switch-athletes' },
        { value: 'me', label: t('coachHome.switch.me'), 'data-testid': 'coach-home-switch-me' },
      ]}
    />
  );
}
