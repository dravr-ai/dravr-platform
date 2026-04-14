// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unified status filter component for list views
// ABOUTME: Provides consistent Active/All/Inactive filtering across Admin Tokens, API Keys, and A2A Clients

import React from 'react';

export type StatusFilterValue = 'active' | 'all' | 'inactive';

export interface StatusFilterProps {
  value: StatusFilterValue;
  onChange: (value: StatusFilterValue) => void;
  activeCount?: number;
  inactiveCount?: number;
  totalCount?: number;
  className?: string;
}

export const StatusFilter: React.FC<StatusFilterProps> = ({
  value,
  onChange,
  activeCount,
  inactiveCount,
  totalCount,
  className = '',
}) => {
  const options: { value: StatusFilterValue; label: string; count?: number }[] = [
    { value: 'active', label: 'Active', count: activeCount },
    { value: 'all', label: 'All', count: totalCount },
    { value: 'inactive', label: 'Inactive', count: inactiveCount },
  ];

  return (
    <div className={`inline-flex rounded-lg border ghost-border bg-surface-container-low/60 p-1 ${className}`}>
      {options.map((option) => (
        <button
          key={option.value}
          onClick={() => onChange(option.value)}
          className={`
            px-3 py-1.5 text-sm font-medium rounded-md transition-all duration-150
            ${value === option.value
              ? 'bg-surface-container-high text-on-surface shadow-sm'
              : 'text-on-surface-variant hover:text-on-surface hover:bg-surface-container-low'
            }
          `}
        >
          {option.label}
          {option.count !== undefined && (
            <span className={`ml-1.5 text-xs ${value === option.value ? 'text-outline' : 'text-outline'}`}>
              ({option.count})
            </span>
          )}
        </button>
      ))}
    </div>
  );
};
