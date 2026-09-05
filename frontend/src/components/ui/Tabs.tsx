// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Reusable Tabs component — the underline variant is the Boreal v2 text-tab language
// ABOUTME: Sentence-case words with a primary underline on the active one and a mono count beside it

import React, { useCallback } from 'react';

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
  variant?: 'underline' | 'pills' | 'bordered';
  size?: 'sm' | 'md' | 'lg';
  className?: string;
}

export const Tabs: React.FC<TabsProps> = ({
  tabs,
  activeTab,
  onChange,
  variant = 'underline',
  size = 'md',
  className = '',
}) => {
  const handleTabClick = useCallback(
    (tabId: string, disabled?: boolean) => {
      if (!disabled) {
        onChange(tabId);
      }
    },
    [onChange]
  );

  // Text tabs carry no horizontal padding: the row's gap spaces them, so the
  // underline is exactly as wide as the word. The boxed variants keep theirs.
  const sizeClasses =
    variant === 'underline'
      ? { sm: 'text-sm pb-2 pt-1', md: 'text-sm pb-2.5 pt-1', lg: 'text-base pb-3 pt-1' }
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

      case 'bordered':
        return `${baseClasses} border-2 rounded-lg ${
          isActive
            ? 'border-primary text-primary bg-primary/5'
            : 'border-transparent text-on-surface-variant hover:ghost-border hover:text-on-surface'
        }`;

      case 'underline':
      default:
        return `${baseClasses} -mb-px min-w-[44px] justify-center border-b-2 ${
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
    bordered: 'flex gap-2 overflow-x-auto',
  };

  return (
    <div className={`${containerClasses[variant]} ${className}`} role="tablist">
      {tabs.map((tab) => (
        <button
          key={tab.id}
          type="button"
          role="tab"
          aria-selected={tab.id === activeTab}
          aria-disabled={tab.disabled}
          onClick={() => handleTabClick(tab.id, tab.disabled)}
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
