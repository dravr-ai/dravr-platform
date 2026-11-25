// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Reusable Tabs component — the underline variant is the Boreal v2 text-tab language
// ABOUTME: Sentence-case words with a primary underline on the active one and a mono count beside it; the ARIA tabs keyboard pattern moves between them

import React, { useCallback, useRef } from 'react';

export interface Tab {
  id: string;
  label: string;
  icon?: React.ReactNode;
  badge?: string | number;
  disabled?: boolean;
}

export interface TabsProps {
  tabs: Tab[];
  activeTab: string;
  onChange: (tabId: string) => void;
  variant?: 'underline' | 'pills';
  size?: 'sm' | 'md' | 'lg';
  className?: string;
  /**
   * Gives each tab the id `<idPrefix>-tab-<id>` and points the active one at
   * the panel `<idPrefix>-panel-<id>`, for a host that draws its own `tabpanel`.
   */
  idPrefix?: string;
  /** The tab list's accessible name. */
  'aria-label'?: string;
}

/** The keys that move between tabs, and which way: a step either side, or to either end. */
const MOVE_KEYS: Record<string, 'previous' | 'next' | 'first' | 'last'> = {
  ArrowLeft: 'previous',
  ArrowRight: 'next',
  Home: 'first',
  End: 'last',
};

/**
 * The index of the tab a `move` from `from` lands on: the nearest enabled one
 * that way, wrapping round the ends, or the first or last enabled one.
 * `undefined` when no tab is enabled.
 */
function landingTab(tabs: readonly Tab[], from: number, move: 'previous' | 'next' | 'first' | 'last'): number | undefined {
  const count = tabs.length;
  const enabled = (index: number) => !tabs[index].disabled;
  if (move === 'first' || move === 'last') {
    const order = [...tabs.keys()];
    return (move === 'first' ? order : order.reverse()).find(enabled);
  }
  const step = move === 'next' ? 1 : -1;
  for (let distance = 1; distance <= count; distance += 1) {
    const index = (((from + step * distance) % count) + count) % count;
    if (enabled(index)) return index;
  }
  return undefined;
}

/**
 * A row of tabs with the ARIA tabs keyboard pattern: only the selected tab is
 * in the tab order, and ArrowLeft, ArrowRight, Home and End move to another
 * enabled tab and select it. A disabled tab is passed over.
 */
export const Tabs: React.FC<TabsProps> = ({
  tabs,
  activeTab,
  onChange,
  variant = 'underline',
  size = 'md',
  className = '',
  idPrefix,
  'aria-label': ariaLabel,
}) => {
  const handleTabClick = useCallback(
    (tabId: string, disabled?: boolean) => {
      if (!disabled) {
        onChange(tabId);
      }
    },
    [onChange]
  );

  // Roving tab order: the selected tab is the one Tab stops on; with none
  // selected, the first enabled tab stands in so the row stays reachable.
  const tabRefs = useRef<(HTMLButtonElement | null)[]>([]);
  const inTabOrder = tabs.some((tab) => tab.id === activeTab)
    ? activeTab
    : tabs.find((tab) => !tab.disabled)?.id;

  const handleKeyDown = (event: React.KeyboardEvent<HTMLButtonElement>, index: number) => {
    const move = MOVE_KEYS[event.key];
    if (move === undefined) return;
    event.preventDefault();
    const target = landingTab(tabs, index, move);
    if (target === undefined || target === index) return;
    tabRefs.current[target]?.focus();
    onChange(tabs[target].id);
  };

  // Text tabs carry no horizontal padding: the row's gap spaces them, so the
  // underline is exactly as wide as the word. The boxed variants keep theirs.
  const sizeClasses =
    variant === 'underline'
      ? { sm: 'text-sm pb-2 pt-1.5', md: 'text-sm pb-2.5 pt-2', lg: 'text-base pb-3 pt-2' }
      : { sm: 'text-sm px-3 py-2', md: 'text-sm px-4 py-3', lg: 'text-base px-5 py-4' };

  const getTabClasses = (tab: Tab) => {
    const isActive = tab.id === activeTab;
    const baseClasses = `
      flex items-center gap-2 font-medium transition-all duration-base
      whitespace-nowrap flex-shrink-0
      ${sizeClasses[size]}
      ${tab.disabled ? 'cursor-not-allowed opacity-50' : 'cursor-pointer'}
    `;

    switch (variant) {
      case 'pills':
        return `${baseClasses} rounded-lg ${
          isActive
            ? 'bg-primary text-on-primary'
            : 'text-on-surface-variant hover:bg-surface-container hover:text-on-surface'
        }`;

      case 'underline':
      default:
        return `${baseClasses} -mb-px touch-target justify-center border-b-2 ${
          isActive
            ? 'border-primary text-on-surface'
            : 'border-transparent text-on-surface-variant hover:text-on-surface'
        }`;
    }
  };

  // overflow-x-auto so a wide tab row scrolls horizontally on a narrow phone
  // instead of forcing the whole document wider (paired with the
  // whitespace-nowrap + flex-shrink-0 tabs above). Mirrors the pattern already
  // used by StoreScreen / NotificationsPanel category rows.
  const containerClasses = {
    underline: 'flex gap-5 border-b ghost-border overflow-x-auto',
    pills: 'flex gap-2 p-1 bg-surface-container-low/60 rounded-lg overflow-x-auto',
  };

  return (
    <div className={`${containerClasses[variant]} ${className}`} role="tablist" aria-label={ariaLabel}>
      {tabs.map((tab, index) => (
        <button
          key={tab.id}
          ref={(element) => {
            tabRefs.current[index] = element;
          }}
          type="button"
          role="tab"
          id={idPrefix ? `${idPrefix}-tab-${tab.id}` : undefined}
          // Only the active tab's panel is drawn, and a reference to a panel
          // that is not in the page is an invalid one.
          aria-controls={idPrefix && tab.id === activeTab ? `${idPrefix}-panel-${tab.id}` : undefined}
          aria-selected={tab.id === activeTab}
          aria-disabled={tab.disabled}
          tabIndex={tab.id === inTabOrder ? 0 : -1}
          onClick={() => handleTabClick(tab.id, tab.disabled)}
          onKeyDown={(event) => handleKeyDown(event, index)}
          className={getTabClasses(tab)}
        >
          {tab.icon && <span className="flex-shrink-0">{tab.icon}</span>}
          <span>{tab.label}</span>
          {tab.badge !== undefined && (
            <span
              className={
                variant === 'underline'
                  ? 'font-mono text-xs text-outline'
                  : `px-2 py-0.5 text-xs font-semibold rounded-full ${
                      tab.id === activeTab
                        ? variant === 'pills'
                          ? 'bg-surface-container-highest text-on-surface'
                          : 'bg-primary-container text-on-primary-container'
                        : 'bg-surface-container-high text-on-surface-variant'
                    }`
              }
            >
              {tab.badge}
            </span>
          )}
        </button>
      ))}
    </div>
  );
};

// Tab Panel component for content
export interface TabPanelProps {
  id: string;
  activeTab: string;
  children: React.ReactNode;
  className?: string;
}

export const TabPanel: React.FC<TabPanelProps> = ({ id, activeTab, children, className = '' }) => {
  if (id !== activeTab) return null;

  return (
    <div role="tabpanel" aria-labelledby={`tab-${id}`} className={`animate-fade-in ${className}`}>
      {children}
    </div>
  );
};
