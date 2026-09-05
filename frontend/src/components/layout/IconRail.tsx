// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The athlete shell's left rail — the brand mark, one icon per destination, the settings gear and the avatar
// ABOUTME: Names stay accessible (aria-label, title and an sr-only span) so nav helpers and tests keep finding each tab

import type { ReactNode } from 'react';
import { clsx } from 'clsx';
import { Settings } from 'lucide-react';
import { DravrLogo } from '../DravrLogo';
import { useTranslation } from '@pierre/i18n';

/** One destination the rail offers. */
export interface RailTab {
  id: string;
  name: string;
  icon: ReactNode;
  badge?: number;
}

interface IconRailProps {
  tabs: RailTab[];
  activeTab: string;
  onSelect: (id: string) => void;
  /** Settings is reached from the gear and from the avatar, never from the list of tabs. */
  onOpenSettings: () => void;
  settingsActive: boolean;
  userInitial: string;
}

/**
 * The 72px rail every messenger keeps on the left, on the same paper as the
 * list beside it — a hairline separates them, not a fill step.
 *
 * The tab list is a real `<ul>` of exactly the destinations the athlete has;
 * the gear and the avatar sit below it, outside the list, so a count of the
 * navigation items stays a count of destinations.
 */
export function IconRail({
  tabs,
  activeTab,
  onSelect,
  onOpenSettings,
  settingsActive,
  userInitial,
}: IconRailProps) {
  const { t } = useTranslation();
  return (
    <aside
      data-testid="icon-rail"
      className="hidden md:flex fixed left-0 top-0 z-40 h-dvh w-[72px] flex-col items-center border-r ghost-border bg-surface py-4"
    >
      <DravrLogo size={40} />
      <nav className="mt-4 flex-1" aria-label={t('shell.mobileNavPrimary')}>
        <ul className="flex flex-col items-center gap-1">
          {tabs.map((tab) => {
            const active = activeTab === tab.id;
            return (
              <li key={tab.id}>
                <button
                  type="button"
                  onClick={() => onSelect(tab.id)}
                  aria-label={tab.name}
                  aria-current={active ? 'page' : undefined}
                  title={tab.name}
                  className={clsx(
                    'relative flex h-11 w-11 items-center justify-center rounded-xl transition-colors focus-ring',
                    active
                      ? 'bg-primary-container text-primary'
                      : 'text-on-surface-variant hover:bg-surface-container-low hover:text-on-surface',
                  )}
                >
                  {tab.icon}
                  <span className="sr-only">{tab.name}</span>
                  {tab.badge !== undefined && tab.badge > 0 && (
                    <span
                      data-testid="pending-users-badge"
                      className="absolute -right-1 -top-1 flex h-[18px] min-w-[18px] items-center justify-center rounded-full bg-primary px-1 text-xs font-semibold text-on-primary ring-2 ring-surface"
                    >
                      {tab.badge > 99 ? '99+' : tab.badge}
                    </span>
                  )}
                </button>
              </li>
            );
          })}
        </ul>
      </nav>
      <div className="flex flex-col items-center gap-2">
        <button
          type="button"
          onClick={onOpenSettings}
          aria-label={t('shell.navSettings')}
          title={t('shell.navSettings')}
          className={clsx(
            'flex h-11 w-11 items-center justify-center rounded-xl transition-colors focus-ring',
            settingsActive
              ? 'bg-primary-container text-primary'
              : 'text-on-surface-variant hover:bg-surface-container-low hover:text-on-surface',
          )}
        >
          <Settings className="h-5 w-5" aria-hidden="true" />
        </button>
        <button
          type="button"
          onClick={onOpenSettings}
          aria-label={t('shell.navOpenSettings')}
          title={t('shell.navOpenSettings')}
          className="flex h-11 w-11 items-center justify-center focus-ring rounded-full"
        >
          <span className="flex h-9 w-9 items-center justify-center rounded-full bg-primary-container text-sm font-semibold text-on-primary-container">
            {userInitial}
          </span>
        </button>
      </div>
    </aside>
  );
}
