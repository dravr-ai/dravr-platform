// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
// ABOUTME: Hook for monitoring Pierre server reachability via /health endpoint
// ABOUTME: Checks on mount, app focus, and periodic interval; exposes isServerReachable state

import { useState, useEffect, useCallback, useRef } from 'react';
import { AppState, type AppStateStatus } from 'react-native';
import { apiClient } from '../services/api';

const HEALTH_CHECK_INTERVAL_MS = 30_000;
const HEALTH_CHECK_TIMEOUT_MS = 5_000;

export interface ServerStatus {
  isServerReachable: boolean;
  isChecking: boolean;
  lastCheckedAt: Date | null;
  checkNow: () => void;
}

export function useServerStatus(): ServerStatus {
  const [isServerReachable, setIsServerReachable] = useState(true);
  const [isChecking, setIsChecking] = useState(false);
  const [lastCheckedAt, setLastCheckedAt] = useState<Date | null>(null);
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const checkHealth = useCallback(async () => {
    setIsChecking(true);
    try {
      await apiClient.get('/health', { timeout: HEALTH_CHECK_TIMEOUT_MS });
      setIsServerReachable(true);
    } catch {
      setIsServerReachable(false);
    } finally {
      setIsChecking(false);
      setLastCheckedAt(new Date());
    }
  }, []);

  // Check on mount
  useEffect(() => {
    checkHealth();
  }, [checkHealth]);

  // Periodic polling
  useEffect(() => {
    intervalRef.current = setInterval(checkHealth, HEALTH_CHECK_INTERVAL_MS);
    return () => {
      if (intervalRef.current) {
        clearInterval(intervalRef.current);
      }
    };
  }, [checkHealth]);

  // Check on app returning to foreground
  useEffect(() => {
    const handleAppState = (nextState: AppStateStatus) => {
      if (nextState === 'active') {
        checkHealth();
      }
    };
    const subscription = AppState.addEventListener('change', handleAppState);
    return () => subscription.remove();
  }, [checkHealth]);

  return { isServerReachable, isChecking, lastCheckedAt, checkNow: checkHealth };
}
