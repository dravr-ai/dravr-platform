// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
// ABOUTME: Hook for monitoring Pierre server reachability via /health endpoint
// ABOUTME: Checks on mount, app focus, and periodic interval; exposes isServerReachable state

import { useState, useEffect, useCallback, useRef } from 'react';
import { AppState, type AppStateStatus } from 'react-native';
import { apiClient } from '../services/api';

const HEALTH_CHECK_INTERVAL_MS = 30_000;
// Cloud Run cold-starts the revision after ~15 min idle; the first /health
// after a long pause can take 6-8s before nginx returns 200. A 5s timeout
// flashed the "Server unreachable" banner on every cold start; 10s absorbs
// that without keeping users staring at a spinner if the server is really down.
const HEALTH_CHECK_TIMEOUT_MS = 10_000;
// Require two consecutive failed checks before flipping the banner so a
// single transient blip (network hiccup, brief cold start past the 10s
// window) doesn't flicker the UI between reachable/unreachable.
const FAILURE_THRESHOLD = 2;

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
  const failureCountRef = useRef(0);

  const checkHealth = useCallback(async () => {
    setIsChecking(true);
    try {
      await apiClient.get('/health', { timeout: HEALTH_CHECK_TIMEOUT_MS });
      // Success — reset the consecutive-failure counter and mark reachable.
      failureCountRef.current = 0;
      setIsServerReachable(true);
    } catch {
      // Failure — increment counter and only flip the banner once the
      // threshold of consecutive misses is reached. Avoids single-blip flicker.
      failureCountRef.current += 1;
      if (failureCountRef.current >= FAILURE_THRESHOLD) {
        setIsServerReachable(false);
      }
    } finally {
      setIsChecking(false);
      setLastCheckedAt(new Date());
    }
  }, []);

  // Check on mount
  useEffect(() => {
    checkHealth();
  }, [checkHealth]);

  // Periodic polling, gated on app foreground state. The interval runs ONLY
  // while the app is active; it is torn down when the app backgrounds and
  // re-armed (with an immediate check) on return. An always-on poll would keep
  // pinging the backend from a backgrounded/idle app, holding the serverless
  // instance warm and defeating scale-to-zero — the cost it is meant to observe.
  useEffect(() => {
    const startPolling = () => {
      if (intervalRef.current) {
        return;
      }
      intervalRef.current = setInterval(checkHealth, HEALTH_CHECK_INTERVAL_MS);
    };
    const stopPolling = () => {
      if (intervalRef.current) {
        clearInterval(intervalRef.current);
        intervalRef.current = null;
      }
    };

    if (AppState.currentState === 'active') {
      startPolling();
    }

    const handleAppState = (nextState: AppStateStatus) => {
      if (nextState === 'active') {
        checkHealth();
        startPolling();
      } else {
        stopPolling();
      }
    };
    const subscription = AppState.addEventListener('change', handleAppState);

    return () => {
      subscription.remove();
      stopPolling();
    };
  }, [checkHealth]);

  return { isServerReachable, isChecking, lastCheckedAt, checkNow: checkHealth };
}
