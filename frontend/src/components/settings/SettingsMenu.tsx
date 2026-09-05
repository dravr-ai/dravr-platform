// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The settings column of the athlete shell — a title row, the identity row, then one 48px row per section with its hint inline, sign out last
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

/**
 * Boreal v2 opened this column with an 80px avatar centred over the name, then
 * 64px rows stacking a title over a hint. v2.1 gives the column the same
 * 52px title row every other column has, a 64px identity row (36px avatar,
 * name and email beside it), and 48px rows with the hint on the title's line,
 * so the whole menu fits above the fold and reads as one list.
 */
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
      <div className="flex h-[52px] shrink-0 items-center px-4">
        <h2 className="font-display text-xl font-semibold text-on-surface">{t('shell.navSettings')}</h2>
      </div>
      <div className="flex h-16 shrink-0 items-center gap-3 border-b ghost-border px-4">
        <span
          aria-hidden="true"
          className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-primary-container font-display text-xs font-semibold text-on-primary-container"
        >
          {initial}
        </span>
        <div className="min-w-0">
          <p className="truncate text-sm font-semibold text-on-surface">{displayName || email}</p>
          {displayName ? <p className="truncate text-xs text-on-surface-variant">{email}</p> : null}
        </div>
      </div>
      <ul className="min-h-0 flex-1 overflow-y-auto pt-1">
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
                  'flex min-h-[48px] w-full items-center gap-3 px-4 text-left transition-colors focus-ring',
                  active ? 'bg-surface-container-low' : 'hover:bg-surface-container-low/60',
                )}
              >
                <span
                  className={clsx(
                    'shrink-0 [&_svg]:h-4 [&_svg]:w-4',
                    active ? 'text-primary' : 'text-on-surface-variant',
                  )}
                >
                  {tab.icon}
                </span>
                <span className="flex min-h-[48px] min-w-0 flex-1 items-baseline gap-2 border-b ghost-border-faint py-[15px]">
                  <span className="text-sm text-on-surface">{t(tab.nameKey)}</span>
                  <span
                    id={`settings-menu-${tab.id}-hint`}
                    aria-hidden="true"
                    className="min-w-0 flex-1 truncate text-xs text-on-surface-variant"
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
        className="flex min-h-[48px] items-center gap-3 border-t ghost-border px-4 text-left text-sm text-on-surface-variant transition-colors hover:bg-surface-container-low hover:text-on-surface focus-ring"
      >
        <LogOut className="h-4 w-4 shrink-0" aria-hidden="true" />
        <span>{t('shell.navSignOut')}</span>
      </button>
    </nav>
  );
}
