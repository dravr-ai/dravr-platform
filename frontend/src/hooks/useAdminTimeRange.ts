// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Shared hook for admin dashboard time range selection
// ABOUTME: Persists the selected time range to localStorage so it is consistent across all analytics tabs

import { useState, useEffect, useCallback } from 'react';

/** Valid time range values in days */
export type AdminTimeRange = 7 | 14 | 30 | 90;

/** Labels for each time range option */
export const ADMIN_TIME_RANGE_LABELS: Record<AdminTimeRange, string> = {
  7: '7 Days',
  14: '14 Days',
  30: '30 Days',
  90: '90 Days',
};

/** localStorage key shared across all admin analytics tabs */
const STORAGE_KEY = 'pierre.admin_time_range';

/** Default time range when nothing is stored */
const DEFAULT_RANGE: AdminTimeRange = 7;

const VALID_RANGES = new Set<number>([7, 14, 30, 90]);

function isValidRange(value: number): value is AdminTimeRange {
  return VALID_RANGES.has(value);
}

/**
 * Reads the persisted admin time range from localStorage.
 * Returns the default (7) if nothing is stored or the stored value is invalid.
 */
function readStoredRange(): AdminTimeRange {
  const stored = localStorage.getItem(STORAGE_KEY);
  if (stored !== null) {
    const parsed = Number(stored);
    if (isValidRange(parsed)) {
      return parsed;
    }
  }
  return DEFAULT_RANGE;
}

/**
 * Shared hook for the admin time range.
 * All admin analytics tabs (UsageAnalytics, LlmConsumptionPanel, EngagementTab)
 * use this hook so the time range stays in sync across tab switches.
 */
export function useAdminTimeRange() {
  const [timeRange, setTimeRangeState] = useState<AdminTimeRange>(readStoredRange);

  const setTimeRange = useCallback((range: AdminTimeRange) => {
    setTimeRangeState(range);
    localStorage.setItem(STORAGE_KEY, String(range));
  }, []);

  // Sync with localStorage on mount in case another tab changed the value
  useEffect(() => {
    const handleStorage = (e: StorageEvent) => {
      if (e.key === STORAGE_KEY && e.newValue !== null) {
        const parsed = Number(e.newValue);
        if (isValidRange(parsed)) {
          setTimeRangeState(parsed);
        }
      }
    };
    window.addEventListener('storage', handleStorage);
    return () => window.removeEventListener('storage', handleStorage);
  }, []);

  return { timeRange, setTimeRange } as const;
}
