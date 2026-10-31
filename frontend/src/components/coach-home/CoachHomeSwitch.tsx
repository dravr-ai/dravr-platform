// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The "Mes athlètes / Mon entraînement" switch a coach who also trains finds at the top of Home (D2, carnet#1253)
// ABOUTME: Two pressed-state buttons in one pill; the first is the coach's Home, the second today's athlete Home

import { clsx } from 'clsx';
import { useTranslation } from '@pierre/i18n';

type HomeScope = 'athletes' | 'me';

interface CoachHomeSwitchProps {
  scope: HomeScope;
  onChange: (scope: HomeScope) => void;
  /** Fill the row, for a phone. */
  stretch?: boolean;
}

const BUTTON =
  'h-7 rounded-lg px-3 text-sm transition-colors focus-ring-visible';

export function CoachHomeSwitch({ scope, onChange, stretch = false }: CoachHomeSwitchProps) {
  const { t } = useTranslation();
  const items: { id: HomeScope; label: string }[] = [
    { id: 'athletes', label: t('coachHome.switch.athletes') },
    { id: 'me', label: t('coachHome.switch.me') },
  ];
  return (
    <div
      role="group"
      aria-label={t('coachHome.switch.aria')}
      data-testid="coach-home-switch"
      className={clsx('flex rounded-[10px] bg-surface-container-high p-0.5', stretch && 'w-full')}
    >
      {items.map((item) => (
        <button
          key={item.id}
          type="button"
          aria-pressed={scope === item.id}
          data-testid={`coach-home-switch-${item.id}`}
          onClick={() => scope !== item.id && onChange(item.id)}
          className={clsx(
            BUTTON,
            stretch && 'h-9 flex-1',
            scope === item.id
              ? 'bg-surface-container-lowest font-semibold text-on-surface ring-1 ring-outline-variant'
              : 'font-medium text-on-surface-variant hover:text-on-surface',
          )}
        >
          {item.label}
        </button>
      ))}
    </div>
  );
}
