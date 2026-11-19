// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Mobile navigation — fixed bottom tab bar + off-canvas drawer.
// ABOUTME: Active <768px; desktop sidebar continues to render at >=768px.

import React from 'react';
import { clsx } from 'clsx';
import { ArrowLeftRight, MessageSquareText } from 'lucide-react';
import { useTranslation } from '@pierre/i18n';
import { useDialog } from '../../hooks/useDialog';
import { badgeLabel, countKey, UNREAD_COUNT_ARIA_KEYS } from '@pierre/shared-constants';
import type { ModeSwitch } from './IconRail';

export interface MobileNavTab {
  id: string;
  name: string;
  icon: React.ReactNode;
  badge?: number;
}

interface BottomTabBarProps {
  primary: MobileNavTab[];
  activeTab: string;
  onSelect: (id: string) => void;
  onOpenDrawer: () => void;
  drawerHasBadge?: boolean;
}

export const BottomTabBar: React.FC<BottomTabBarProps> = ({
  primary,
  activeTab,
  onSelect,
  onOpenDrawer,
  drawerHasBadge,
}) => {
  const { t, language } = useTranslation();
  return (
    <nav
      aria-label={t('shell.mobileNavPrimary')}
      className="md:hidden fixed bottom-0 left-0 right-0 z-40 bg-surface border-t ghost-border"
      style={{ paddingBottom: 'env(safe-area-inset-bottom, 0px)' }}
    >
      <ul className="flex items-stretch justify-around">
        {primary.map((tab) => {
          const active = activeTab === tab.id;
          return (
            <li key={tab.id} className="flex-1">
              <button
                type="button"
                onClick={() => onSelect(tab.id)}
                aria-current={active ? 'page' : undefined}
                aria-label={tab.name}
                className={clsx(
                  'w-full min-h-[56px] flex flex-col items-center justify-center gap-0.5 px-1 py-1.5 relative',
                  'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-inset',
                  active ? 'text-primary' : 'text-on-surface-variant',
                )}
              >
                <span className="relative inline-flex">
                  {tab.icon}
                  {tab.badge !== undefined && tab.badge > 0 && (
                    <span
                      className="absolute -top-1.5 -right-2 bg-primary text-on-primary text-xs rounded-full h-[18px] min-w-[18px] px-1 flex items-center justify-center font-semibold ring-2 ring-surface"
                      aria-label={t(countKey(tab.badge, language, UNREAD_COUNT_ARIA_KEYS), { count: tab.badge })}
                    >
                      {badgeLabel(tab.badge)}
                    </span>
                  )}
                </span>
                <span className="text-xs font-medium leading-none">{tab.name}</span>
              </button>
            </li>
          );
        })}
        <li className="flex-1">
          <button
            type="button"
            onClick={onOpenDrawer}
            aria-label={t('shell.mobileNavOpen')}
            className="w-full min-h-[56px] flex flex-col items-center justify-center gap-0.5 px-1 py-1.5 relative text-on-surface-variant focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-inset"
          >
            <span className="relative inline-flex">
              <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 6h16M4 12h16M4 18h16" />
              </svg>
              {drawerHasBadge && (
                <span
                  className="absolute -top-1 -right-1.5 bg-primary rounded-full h-2.5 w-2.5 ring-2 ring-surface"
                  aria-hidden="true"
                />
              )}
            </span>
            <span className="text-xs font-medium leading-none">{t('shell.mobileNavMenu')}</span>
          </button>
        </li>
      </ul>
    </nav>
  );
};

interface MobileDrawerProps {
  open: boolean;
  onClose: () => void;
  secondary: MobileNavTab[];
  activeTab: string;
  onSelect: (id: string) => void;
  userLabel: string;
  userInitial: string;
  userRole?: string;
  onOpenSettings: () => void;
  /** The way into the feedback thread with the Dravr team, beside Settings. */
  onSendFeedback: () => void;
  onSignOut: () => void;
  /** The Dravr team's replies waiting unread in the feedback thread. */
  feedbackUnread: number;
  /** Offered to an admin only: switch between the console and the athlete app. */
  modeSwitch?: ModeSwitch;
}

export const MobileDrawer: React.FC<MobileDrawerProps> = ({
  open,
  onClose,
  secondary,
  activeTab,
  onSelect,
  userLabel,
  userInitial,
  userRole,
  onOpenSettings,
  onSendFeedback,
  onSignOut,
  feedbackUnread,
  modeSwitch,
}) => {
  const { t, language } = useTranslation();

  // Focus trap, focus restore, Escape and the scroll lock (WCAG 2.4.3): Tab
  // cannot reach the still-mounted BottomTabBar behind the panel, and closing
  // hands focus back to the control that opened it. The `inert` prop below is
  // applied in the commit, before this hook's effect moves focus into the
  // panel, so the panel is never inert when focus arrives.
  const { containerRef: asideRef } = useDialog<HTMLElement>({ open, onClose });

  return (
    <div
      className={clsx('md:hidden fixed inset-0 z-50', !open && 'pointer-events-none')}
      aria-hidden={!open}
      // Closed, the whole subtree leaves the tab order and the accessibility
      // tree, which is what axe expects when contents sit under `aria-hidden`.
      inert={!open}
    >
      <div
        className={clsx(
          'absolute inset-0 bg-black/60 backdrop-blur-sm transition-opacity duration-200',
          open ? 'opacity-100' : 'opacity-0',
        )}
        onClick={onClose}
        aria-hidden="true"
      />
      <aside
        ref={asideRef}
        tabIndex={-1}
        role="dialog"
        aria-modal="true"
        aria-label={t('shell.mobileNavSecondary')}
        className={clsx(
          'absolute left-0 top-0 bottom-0 w-[78vw] max-w-[320px] bg-surface border-r ghost-border flex flex-col pad-safe-top focus:outline-none',
          'transition-transform duration-200 ease-out',
          open ? 'translate-x-0' : '-translate-x-full',
        )}
      >
        <header className="flex items-center justify-between px-5 py-4 border-b ghost-border">
          <div className="flex items-center gap-3 min-w-0">
            <div className="w-10 h-10 rounded-full bg-primary-container flex items-center justify-center flex-shrink-0">
              <span className="text-sm font-semibold text-on-primary-container">{userInitial}</span>
            </div>
            <div className="min-w-0">
              <p className="text-sm font-semibold text-on-surface truncate">{userLabel}</p>
              {userRole && (
                <p className="text-xs capitalize text-on-surface-variant">{userRole}</p>
              )}
            </div>
          </div>
          <button
            type="button"
            onClick={onClose}
            aria-label={t('shell.mobileNavClose')}
            className="touch-target flex items-center justify-center text-on-surface-variant hover:text-on-surface"
          >
            <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </header>

        <nav aria-label={t('shell.mobileNavSecondaryDest')} className="flex-1 overflow-y-auto px-3 py-3">
          <ul className="space-y-1">
            {secondary.map((tab) => {
              const active = activeTab === tab.id;
              return (
                <li key={tab.id}>
                  <button
                    type="button"
                    onClick={() => {
                      onSelect(tab.id);
                      onClose();
                    }}
                    aria-current={active ? 'page' : undefined}
                    className={clsx(
                      'w-full flex items-center gap-3 px-3 py-3 rounded-lg text-sm font-medium min-h-[48px]',
                      'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary',
                      active
                        ? 'bg-primary-container text-on-primary-container'
                        : 'text-on-surface-variant hover:bg-surface-container-low hover:text-on-surface',
                    )}
                  >
                    <span className="flex-shrink-0">{tab.icon}</span>
                    <span className="flex-1 text-left">{tab.name}</span>
                    {tab.badge !== undefined && tab.badge > 0 && (
                      <span className="bg-primary text-on-primary text-xs rounded-full h-5 min-w-5 px-1.5 flex items-center justify-center font-semibold">
                        {badgeLabel(tab.badge)}
                      </span>
                    )}
                  </button>
                </li>
              );
            })}
          </ul>
        </nav>

        <footer className="border-t ghost-border px-3 py-3 space-y-1" style={{ paddingBottom: 'calc(env(safe-area-inset-bottom, 0px) + 0.75rem)' }}>
          <button
            type="button"
            onClick={() => {
              onOpenSettings();
              onClose();
            }}
            className="w-full flex items-center gap-3 px-3 py-3 rounded-lg text-sm font-medium text-on-surface-variant hover:bg-surface-container-low hover:text-on-surface min-h-[48px]"
          >
            <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z" />
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
            </svg>
            <span>{t('nav.settings')}</span>
          </button>
          <button
            type="button"
            onClick={() => {
              onSendFeedback();
              onClose();
            }}
            className="w-full flex items-center gap-3 px-3 py-3 rounded-lg text-sm font-medium text-on-surface-variant hover:bg-surface-container-low hover:text-on-surface min-h-[48px]"
          >
            <MessageSquareText className="w-5 h-5" aria-hidden="true" />
            <span>{t('feedback.navSendFeedback')}</span>
            {feedbackUnread > 0 && (
              <span
                data-testid="drawer-feedback-unread-count"
                className="ml-auto flex h-[18px] min-w-[18px] items-center justify-center rounded-full bg-primary px-1 text-xs font-semibold text-on-primary"
              >
                <span aria-hidden="true">{badgeLabel(feedbackUnread)}</span>
                <span className="sr-only">
                  {t(countKey(feedbackUnread, language, UNREAD_COUNT_ARIA_KEYS), { count: feedbackUnread })}
                </span>
              </span>
            )}
          </button>
          {modeSwitch && (
            <button
              type="button"
              data-testid="drawer-switch-view-mode"
              onClick={() => {
                onClose();
                modeSwitch.onSelect();
              }}
              className="w-full flex items-center gap-3 px-3 py-3 rounded-lg text-sm font-medium text-on-surface-variant hover:bg-surface-container-low hover:text-on-surface min-h-[48px]"
            >
              <ArrowLeftRight className="w-5 h-5" aria-hidden="true" />
              <span>{modeSwitch.label}</span>
            </button>
          )}
          <button
            type="button"
            onClick={() => {
              onSignOut();
              onClose();
            }}
            className="w-full flex items-center gap-3 px-3 py-3 rounded-lg text-sm font-medium text-on-surface-variant hover:bg-surface-container-low hover:text-on-surface min-h-[48px]"
          >
            <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M17 16l4-4m0 0l-4-4m4 4H7m6 4v1a3 3 0 01-3 3H6a3 3 0 01-3-3V7a3 3 0 013-3h4a3 3 0 013 3v1" />
            </svg>
            <span>{t('shell.mobileNavSignOut')}</span>
          </button>
        </footer>
      </aside>
    </div>
  );
};
