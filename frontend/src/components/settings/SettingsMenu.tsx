// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The settings column of the athlete shell — the avatar and name, then one row per section with its hint, sign out last
// ABOUTME: A navigation landmark whose rows carry aria-current and are named by their label alone; the hint is their description

import { clsx } from 'clsx';
import { ChevronRight, LogOut } from 'lucide-react';
import { useTranslation } from '@pierre/i18n';
import type { SettingsTab, SettingsTabDef } from './settingsTabs';

interface SettingsMenuProps {
  tabs: SettingsTabDef[];
  activeTab: SettingsTab | null;
  onSelect: (id: SettingsTab) => void;
  displayName: string;
  email: string;
  onSignOut: () => void;
}

export default function SettingsMenu({
  tabs,
  activeTab,
  onSelect,
  displayName,
  email,
  onSignOut,
}: SettingsMenuProps) {
  const { t } = useTranslation();
  const initial = (displayName || email).charAt(0).toUpperCase() || '?';
  return (
    <nav
      aria-label={t('settings.tabsLabel')}
      data-testid="settings-menu"
      className="flex h-full min-h-0 flex-col bg-surface"
    >
      <div className="flex flex-col items-center px-6 pb-6 pt-8 text-center">
        <span
          aria-hidden="true"
          className="flex h-20 w-20 items-center justify-center rounded-full bg-primary-container font-display text-2xl font-semibold text-on-primary-container"
        >
          {initial}
        </span>
        <h2 className="mt-4 font-display text-xl font-semibold text-on-surface">{displayName || email}</h2>
        {displayName ? <p className="mt-0.5 text-sm text-on-surface-variant">{email}</p> : null}
      </div>
      <ul className="min-h-0 flex-1 overflow-y-auto">
        {tabs.map((tab) => {
          const active = activeTab === tab.id;
          return (
            <li key={tab.id}>
              <button
                type="button"
                onClick={() => onSelect(tab.id)}
                aria-current={active ? 'page' : undefined}
                aria-describedby={`settings-menu-${tab.id}-hint`}
                data-testid={`settings-menu-${tab.id}`}
                className={clsx(
                  'flex w-full items-center gap-4 px-6 py-3 text-left transition-colors min-h-[64px] focus-ring',
                  active ? 'bg-surface-container-low' : 'hover:bg-surface-container-low/60',
                )}
              >
                <span className={clsx('shrink-0', active ? 'text-primary' : 'text-on-surface-variant')}>{tab.icon}</span>
                <span className="flex min-w-0 flex-1 flex-col border-b ghost-border py-2">
                  <span className="text-base text-on-surface">{t(tab.nameKey)}</span>
                  <span
                    id={`settings-menu-${tab.id}-hint`}
                    aria-hidden="true"
                    className="truncate text-sm text-on-surface-variant"
                  >
                    {t(tab.hintKey)}
                  </span>
                </span>
                <ChevronRight className="h-4 w-4 shrink-0 text-outline lg:hidden" aria-hidden="true" />
              </button>
            </li>
          );
        })}
      </ul>
      <button
        type="button"
        onClick={onSignOut}
        className="flex items-center gap-4 border-t ghost-border px-6 py-4 text-left text-error transition-colors hover:bg-surface-container-low min-h-[56px] focus-ring"
      >
        <LogOut className="h-5 w-5 shrink-0" aria-hidden="true" />
        <span className="text-base">{t('shell.navSignOut')}</span>
      </button>
    </nav>
  );
}
