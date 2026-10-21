// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Home choices in Settings — the switches that bring back the plan suggestion and Getting started
// ABOUTME: Writes the same server-side preferences Home's "Hide" links set, so web and mobile agree (carnet#820, carnet#1098)

import { useTranslation } from '@pierre/i18n';
import type { HomePreferencesUpdate } from '@pierre/shared-types';
import { useHomePreferences } from '../../hooks/useHome';

interface HomeSwitchProps {
  label: string;
  hint: string;
  /** On: Home shows the thing; off: the athlete hid it. */
  shown: boolean;
  disabled: boolean;
  onToggle: () => void;
  'data-testid': string;
}

function HomeSwitch({ label, hint, shown, disabled, onToggle, 'data-testid': testId }: HomeSwitchProps) {
  return (
    <div className="flex items-center justify-between gap-4">
      <div className="min-w-0">
        <p className="text-sm text-on-surface">{label}</p>
        <p className="mt-0.5 text-xs text-on-surface-variant">{hint}</p>
      </div>
      <button
        type="button"
        role="switch"
        aria-checked={shown}
        aria-label={label}
        data-testid={testId}
        onClick={onToggle}
        disabled={disabled}
        className={`relative inline-flex h-6 w-11 flex-shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out focus:outline-none focus:ring-2 focus:ring-primary focus:ring-offset-2 focus:ring-offset-zinc-900 disabled:opacity-50 ${
          shown ? 'bg-primary' : 'bg-surface-container-high'
        }`}
      >
        <span
          className={`pointer-events-none inline-block h-5 w-5 transform rounded-full bg-white shadow ring-0 transition duration-200 ease-in-out ${
            shown ? 'translate-x-5' : 'translate-x-0'
          }`}
        />
      </button>
    </div>
  );
}

/**
 * Two switches: whether Home suggests building a training plan while the
 * athlete has none, and whether Today opens on Getting started. Home's "Hide"
 * links turn them off; this is the way back. Each switch writes only its own
 * choice, so the other stays as stored.
 */
export default function HomeSettings() {
  const { t } = useTranslation();
  const home = useHomePreferences();
  const prefs = home.preferences;
  const disabled = prefs === null || home.isUpdating;
  const planOffered = prefs ? !prefs.plan_suggestion_hidden : true;
  const gettingStartedShown = prefs ? prefs.getting_started_hidden !== true : true;
  const change = (update: HomePreferencesUpdate) => home.update(update);

  return (
    <div className="mt-8 border-t ghost-border-faint pt-6" data-testid="home-settings">
      <h3 className="mb-2 text-sm font-medium text-on-surface">{t('home.settings.title')}</h3>
      <div className="space-y-4">
        <HomeSwitch
          label={t('home.settings.planSuggestion')}
          hint={t('home.settings.planSuggestionHint')}
          shown={planOffered}
          disabled={disabled}
          onToggle={() => change({ plan_suggestion_hidden: planOffered })}
          data-testid="home-settings-plan-suggestion"
        />
        <HomeSwitch
          label={t('home.settings.gettingStarted')}
          hint={t('home.settings.gettingStartedHint')}
          shown={gettingStartedShown}
          disabled={disabled}
          onToggle={() => change({ getting_started_hidden: gettingStartedShown })}
          data-testid="home-settings-getting-started"
        />
      </div>
      {home.isError && (
        <p role="alert" className="mt-2 text-sm text-error">
          {t('common.error')}
        </p>
      )}
    </div>
  );
}
