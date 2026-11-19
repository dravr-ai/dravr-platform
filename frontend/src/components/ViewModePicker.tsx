// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The choice an admin makes at every web sign-in — the admin console, or the athlete app on their own account
// ABOUTME: Two option rows on the bare signed-in screen; the choice lives in AuthContext and the app redraws for it

import { Activity, LayoutDashboard, LogOut, type LucideIcon } from 'lucide-react';
import { useTranslation } from '@pierre/i18n';
import type { ViewMode } from '@pierre/shared-types';
import { useAuth } from '../hooks/useAuth';
import { DravrLogo } from './DravrLogo';

interface ModeOption {
  mode: ViewMode;
  icon: LucideIcon;
  titleKey: string;
  bodyKey: string;
}

/** Athlete first: it is the product; the console is the admin's other hat. */
const MODE_OPTIONS: readonly ModeOption[] = [
  { mode: 'athlete', icon: Activity, titleKey: 'shell.modeAthleteTitle', bodyKey: 'shell.modeAthleteBody' },
  { mode: 'admin', icon: LayoutDashboard, titleKey: 'shell.modeAdminTitle', bodyKey: 'shell.modeAdminBody' },
];

/**
 * Shown to an admin once signed in, before anything else is drawn
 * (carnet#1631). The athlete mode is the athlete app on the admin's own
 * account, onboarding included; the admin mode is today's console. The menu
 * switches between them later, and the next sign-in asks again.
 */
export default function ViewModePicker() {
  const { t } = useTranslation();
  const { user, chooseViewMode, logout } = useAuth();

  return (
    <div
      className="min-h-dvh flex items-center justify-center py-12 px-4 sm:px-6 lg:px-8 bg-surface pad-safe-top"
      data-testid="view-mode-picker"
    >
      <div className="max-w-md w-full">
        <div className="px-4 py-6">
          <div className="flex flex-col items-center text-center">
            <DravrLogo size={64} />
            <h1 className="mt-6 font-display text-xl font-semibold text-on-surface">
              {t('shell.modePickerTitle')}
            </h1>
            <p className="mt-3 text-sm text-on-surface-variant max-w-sm">{t('shell.modePickerBody')}</p>
          </div>

          <ul className="mt-8 flex flex-col gap-3">
            {MODE_OPTIONS.map(({ mode, icon: Icon, titleKey, bodyKey }) => (
              <li key={mode}>
                <button
                  type="button"
                  onClick={() => chooseViewMode(mode)}
                  data-testid={`view-mode-${mode}`}
                  className="flex w-full items-center gap-4 rounded-xl border ghost-border bg-surface-container-lowest px-4 py-4 text-left transition-colors hover:bg-surface-container-low focus-ring"
                >
                  <span
                    className="flex h-11 w-11 shrink-0 items-center justify-center rounded-full bg-primary-container text-on-primary-container"
                    aria-hidden="true"
                  >
                    <Icon className="h-5 w-5" />
                  </span>
                  <span className="flex min-w-0 flex-col">
                    <span className="text-base font-semibold text-on-surface">{t(titleKey)}</span>
                    <span className="text-sm text-on-surface-variant">{t(bodyKey)}</span>
                  </span>
                </button>
              </li>
            ))}
          </ul>

          <div className="mt-8 flex flex-col items-center gap-1 text-center">
            <p className="text-xs text-on-surface-variant truncate max-w-full">
              {user?.display_name || user?.email}
            </p>
            <button
              type="button"
              onClick={logout}
              className="touch-target inline-flex items-center gap-2 px-3 text-sm text-on-surface-variant hover:text-on-surface focus-ring rounded-lg"
            >
              <LogOut className="h-4 w-4" aria-hidden="true" />
              {t('shell.navSignOut')}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
