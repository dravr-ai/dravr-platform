// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Home choices in Settings — today, the switch that brings back the suggestion to build a training plan
// ABOUTME: Writes the same server-side preference Home's "Hide" sets, so web and mobile agree (carnet#820)

import { useTranslation } from '@pierre/i18n';
import { useHomePreferences } from '../../hooks/useHome';

/**
 * One switch: whether Home suggests building a training plan while the
 * athlete has none. Home's "Hide" turns it off; this is the way back.
 */
export default function HomeSettings() {
  const { t } = useTranslation();
  const home = useHomePreferences();
  const offered = home.preferences ? !home.preferences.plan_suggestion_hidden : true;
  const label = t('home.settings.planSuggestion');

  return (
    <div className="mt-8 border-t ghost-border-faint pt-6" data-testid="home-settings">
      <h3 className="mb-2 text-sm font-medium text-on-surface">{t('home.settings.title')}</h3>
      <div className="flex items-center justify-between gap-4">
        <div className="min-w-0">
          <p className="text-sm text-on-surface">{label}</p>
          <p className="mt-0.5 text-xs text-on-surface-variant">{t('home.settings.planSuggestionHint')}</p>
        </div>
        <button
          type="button"
          role="switch"
          aria-checked={offered}
          aria-label={label}
          data-testid="home-settings-plan-suggestion"
          onClick={() =>
            home.update({ ...(home.preferences ?? { plan_suggestion_hidden: false }), plan_suggestion_hidden: offered })
          }
          disabled={home.preferences === null || home.isUpdating}
          className={`relative inline-flex h-6 w-11 flex-shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out focus:outline-none focus:ring-2 focus:ring-primary focus:ring-offset-2 focus:ring-offset-zinc-900 disabled:opacity-50 ${
            offered ? 'bg-primary' : 'bg-surface-container-high'
          }`}
        >
          <span
            className={`pointer-events-none inline-block h-5 w-5 transform rounded-full bg-white shadow ring-0 transition duration-200 ease-in-out ${
              offered ? 'translate-x-5' : 'translate-x-0'
            }`}
          />
        </button>
      </div>
      {home.isError && (
        <p role="alert" className="mt-2 text-sm text-error">
          {t('common.error')}
        </p>
      )}
    </div>
  );
}
