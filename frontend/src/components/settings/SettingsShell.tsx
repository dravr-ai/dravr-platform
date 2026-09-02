// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The athlete's settings surface — the section menu beside the open section, one at a time on a narrow screen
// ABOUTME: The section itself is UserSettings with its strip hidden; this shell only decides which pane a viewport shows

import { Suspense, lazy, useMemo } from 'react';
import { ArrowLeft } from 'lucide-react';
import { useTranslation } from '@pierre/i18n';
import { useAuth } from '../../hooks/useAuth';
import { FEATURE_KEYS, useFeatureFlags } from '../../hooks/useFeatureFlags';
import { useIsDesktop } from '../../hooks/useBreakpoint';
import SettingsMenu from './SettingsMenu';
import { ADMIN_HIDDEN_TABS, SETTINGS_TABS, type SettingsTab } from './settingsTabs';

const UserSettings = lazy(() => import('../UserSettings'));

interface SettingsShellProps {
  /** The open section, or `null` for the menu alone on a narrow screen. */
  tab: SettingsTab | null;
  onSelect: (id: SettingsTab) => void;
  /** Back to the menu, where the list is hidden behind the open section. */
  onBack: () => void;
}

export default function SettingsShell({ tab, onSelect, onBack }: SettingsShellProps) {
  const { t } = useTranslation();
  const { user, logout } = useAuth();
  const { flags } = useFeatureFlags();
  const isDesktop = useIsDesktop();
  const isAdminUser = user?.role === 'admin' || user?.role === 'super_admin';

  const tabs = useMemo(() => {
    const base = isAdminUser ? SETTINGS_TABS.filter((entry) => !ADMIN_HIDDEN_TABS.has(entry.id)) : SETTINGS_TABS;
    return flags[FEATURE_KEYS.apiTokens] ? base : base.filter((entry) => entry.id !== 'tokens');
  }, [isAdminUser, flags]);

  // A wide screen always shows a section; the first one when none is chosen.
  const openTab: SettingsTab | null = tab ?? (isDesktop ? 'profile' : null);
  const showMenu = isDesktop || openTab === null;
  const showDetail = openTab !== null;
  const openName = openTab ? tabs.find((entry) => entry.id === openTab)?.nameKey : undefined;

  return (
    <div className="flex h-full min-h-0 bg-surface-container-low" data-testid="settings-shell">
      {showMenu && (
        <div className="flex min-h-0 w-full shrink-0 flex-col border-r ghost-border lg:w-[360px] xl:w-[400px]">
          <SettingsMenu
            tabs={tabs}
            activeTab={openTab}
            onSelect={onSelect}
            displayName={user?.display_name ?? ''}
            email={user?.email ?? ''}
            onSignOut={logout}
          />
        </div>
      )}
      {showDetail && openTab && (
        <div className="flex min-h-0 min-w-0 flex-1 flex-col" data-testid="settings-pane">
          {!isDesktop && (
            <div className="flex items-center gap-2 border-b ghost-border bg-surface-container px-3 py-2">
              <button
                type="button"
                onClick={onBack}
                aria-label={t('settings.backToMenu')}
                title={t('settings.backToMenu')}
                className="flex h-11 w-11 shrink-0 items-center justify-center rounded-full text-on-surface-variant hover:bg-surface-container-high hover:text-on-surface focus-ring"
              >
                <ArrowLeft className="h-5 w-5" aria-hidden="true" />
              </button>
              <h2 className="text-base font-semibold text-on-surface">{openName ? t(openName) : ''}</h2>
            </div>
          )}
          <div className="min-h-0 flex-1 overflow-y-auto p-4 md:p-6">
            <Suspense fallback={<div className="flex justify-center py-8"><div className="pierre-spinner"></div></div>}>
              <UserSettings key={openTab} initialTab={openTab} hideTabNav />
            </Suspense>
          </div>
        </div>
      )}
    </div>
  );
}
