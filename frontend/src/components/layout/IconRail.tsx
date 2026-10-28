// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The athlete shell's left rail — the brand mark that leads Home, one icon per destination, the gear and the account menu
// ABOUTME: Names stay accessible (aria-label, title and an sr-only span) so nav helpers and tests keep finding each tab

import { useEffect, useId, useRef, useState, type ReactNode } from 'react';
import { clsx } from 'clsx';
import { LogOut, MessageSquareText, Settings } from 'lucide-react';
import { DravrLogo } from '../DravrLogo';
import { useTranslation } from '@pierre/i18n';
import { badgeLabel } from '@pierre/shared-constants';

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
  /** Settings is reached from the gear and from the account menu, never from the list of tabs. */
  onOpenSettings: () => void;
  /** The account menu's way into the feedback thread with the Dravr team. */
  onSendFeedback: () => void;
  onSignOut: () => void;
  /** The brand mark is the way Home, as a messenger's logo is. */
  onHome: () => void;
  settingsActive: boolean;
  userInitial: string;
  /** The athlete's name or email, at the top of the account menu. */
  userLabel: string;
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
  onSendFeedback,
  onSignOut,
  onHome,
  settingsActive,
  userInitial,
  userLabel,
}: IconRailProps) {
  const { t } = useTranslation();
  return (
    <aside
      data-testid="icon-rail"
      className="hidden md:flex fixed left-0 top-0 z-40 h-dvh w-[72px] flex-col items-center border-r ghost-border bg-surface py-4"
    >
      {/* The mark gains a button around it, never a new look (BRAND.md): no
          fill, no hover tint, only the focus ring when the keyboard is on it. */}
      <button
        type="button"
        onClick={onHome}
        aria-label={t('nav.home')}
        title={t('nav.home')}
        data-testid="rail-logo-home"
        className="flex h-11 w-11 items-center justify-center rounded-xl focus-ring"
      >
        <DravrLogo size={40} />
      </button>
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
                      {badgeLabel(tab.badge)}
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
        <AccountMenu
          userInitial={userInitial}
          userLabel={userLabel}
          onOpenSettings={onOpenSettings}
          onSendFeedback={onSendFeedback}
          onSignOut={onSignOut}
        />
      </div>
    </aside>
  );
}

interface AccountMenuProps {
  userInitial: string;
  userLabel: string;
  onOpenSettings: () => void;
  onSendFeedback: () => void;
  onSignOut: () => void;
}

const MENU_ITEM_CLASS =
  'flex h-10 w-full items-center gap-3 rounded-lg px-2.5 text-left text-sm font-medium text-on-surface hover:bg-surface-container-low focus-ring';

/**
 * The avatar opens what every messenger keeps under it: the account's name,
 * Settings, a way to tell the team something, and signing out. A disclosure
 * rather than an ARIA menu — plain buttons in Tab order — closed by Escape,
 * by a click outside it, and by any choice made in it.
 */
function AccountMenu({ userInitial, userLabel, onOpenSettings, onSendFeedback, onSignOut }: AccountMenuProps) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const menuId = useId();
  const containerRef = useRef<HTMLDivElement>(null);
  const toggleRef = useRef<HTMLButtonElement>(null);
  const firstItemRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!open) return;
    firstItemRef.current?.focus();
    const onPointerDown = (event: PointerEvent) => {
      if (!containerRef.current?.contains(event.target as Node)) setOpen(false);
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        setOpen(false);
        toggleRef.current?.focus();
      }
    };
    document.addEventListener('pointerdown', onPointerDown);
    document.addEventListener('keydown', onKeyDown);
    return () => {
      document.removeEventListener('pointerdown', onPointerDown);
      document.removeEventListener('keydown', onKeyDown);
    };
  }, [open]);

  const choose = (action: () => void) => () => {
    setOpen(false);
    action();
  };

  return (
    <div ref={containerRef} className="relative">
      <button
        ref={toggleRef}
        type="button"
        onClick={() => setOpen((value) => !value)}
        aria-label={t('feedback.accountMenu')}
        aria-expanded={open}
        aria-controls={menuId}
        title={t('feedback.accountMenu')}
        data-testid="rail-account-menu"
        className="flex h-11 w-11 items-center justify-center focus-ring rounded-full"
      >
        <span className="flex h-9 w-9 items-center justify-center rounded-full bg-primary-container text-sm font-semibold text-on-primary-container">
          {userInitial}
        </span>
      </button>
      {open && (
        <div
          id={menuId}
          className="absolute bottom-0 left-full z-50 ml-2 flex w-64 flex-col rounded-xl border ghost-border bg-surface-container-lowest p-1.5 shadow-floating"
        >
          <p className="truncate px-2.5 pb-2 pt-1.5 text-sm font-semibold text-on-surface">{userLabel}</p>
          <div className="mx-1 mb-1.5 h-px bg-outline-variant/40" aria-hidden="true" />
          <button ref={firstItemRef} type="button" onClick={choose(onOpenSettings)} className={MENU_ITEM_CLASS}>
            <Settings className="h-4 w-4 text-on-surface-variant" aria-hidden="true" />
            {t('shell.navSettings')}
          </button>
          <button type="button" onClick={choose(onSendFeedback)} className={MENU_ITEM_CLASS}>
            <MessageSquareText className="h-4 w-4 text-primary" aria-hidden="true" />
            {t('feedback.navSendFeedback')}
          </button>
          <div className="mx-1 my-1.5 h-px bg-outline-variant/40" aria-hidden="true" />
          <button type="button" onClick={choose(onSignOut)} className={MENU_ITEM_CLASS}>
            <LogOut className="h-4 w-4 text-on-surface-variant" aria-hidden="true" />
            {t('shell.navSignOut')}
          </button>
        </div>
      )}
    </div>
  );
}
