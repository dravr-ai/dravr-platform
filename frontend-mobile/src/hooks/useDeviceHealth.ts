// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The phone's health store row and its background sync — toggle, last sync, counts and delete for Connections; a sync on every return to the app
// ABOUTME: State lives on the device (it is the device's store), so this reads the sync service rather than React Query; errors are logged by message only

import { useCallback, useEffect, useState } from 'react';
import { AppState, type AppStateStatus } from 'react-native';
import { isAxiosError } from 'axios';
import type { DeviceHealthCounts, DeviceHealthSource } from '@pierre/shared-types';
import { useAuth } from '../contexts/AuthContext';
import {
  deleteDeviceHealthData,
  deviceHealthAvailable,
  deviceHealthSource,
  disableDeviceHealth,
  enableDeviceHealth,
  isDeviceHealthSyncing,
  readDeviceHealthState,
  subscribeDeviceHealth,
  syncDeviceHealth,
  type DeviceHealthState,
} from '../services/deviceHealth';

/** What went wrong last, for the row's one error line. */
export type DeviceHealthFailure = 'declined' | 'enable' | 'sync' | 'delete';

export interface DeviceHealthRow {
  /** Whether the row is shown: a signed-in athlete on a phone whose store exists. */
  visible: boolean;
  source: DeviceHealthSource | null;
  enabled: boolean;
  syncing: boolean;
  /** A turn-on or delete is in flight. */
  busy: boolean;
  lastSyncAt: string | null;
  lastCounts: DeviceHealthCounts | null;
  /** Whether any sync has written anything. */
  received: boolean;
  failure: DeviceHealthFailure | null;
  turnOn: () => Promise<void>;
  turnOff: () => Promise<void>;
  deleteSyncedData: () => Promise<void>;
}

/**
 * A log line that never holds the request: an axios error carries its
 * request body, and that body is the athlete's health samples.
 */
function messageOf(error: unknown): string {
  if (isAxiosError(error)) return `HTTP ${error.response?.status ?? 'unreachable'}`;
  return String(error);
}

function useSignedInUserId(): string | undefined {
  const { user, isAuthenticated } = useAuth();
  return isAuthenticated ? user?.id : undefined;
}

/** The Connections row for this phone's health store. */
export function useDeviceHealth(): DeviceHealthRow {
  const userId = useSignedInUserId();
  const source = deviceHealthSource();
  const [available, setAvailable] = useState(false);
  const [state, setState] = useState<DeviceHealthState | null>(null);
  const [syncing, setSyncing] = useState(false);
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<DeviceHealthFailure | null>(null);

  useEffect(() => {
    let live = true;
    void deviceHealthAvailable().then((ready) => {
      if (live) setAvailable(ready);
    });
    return () => {
      live = false;
    };
  }, []);

  useEffect(() => {
    if (!userId) return undefined;
    let live = true;
    const refresh = () => {
      setSyncing(isDeviceHealthSyncing(userId));
      void readDeviceHealthState(userId).then((read) => {
        if (live) setState(read);
      });
    };
    refresh();
    const unsubscribe = subscribeDeviceHealth(refresh);
    return () => {
      live = false;
      unsubscribe();
    };
  }, [userId]);

  const runSync = useCallback(async (id: string) => {
    try {
      await syncDeviceHealth(id);
      setFailure(null);
    } catch (error) {
      console.warn('Device health sync failed:', messageOf(error));
      setFailure('sync');
    }
  }, []);

  const turnOn = useCallback(async () => {
    if (!userId) return;
    setBusy(true);
    try {
      const outcome = await enableDeviceHealth(userId);
      if (outcome === 'enabled') {
        setFailure(null);
        void runSync(userId);
      } else {
        setFailure(outcome === 'declined' ? 'declined' : 'enable');
      }
    } catch (error) {
      console.warn('Device health could not be turned on:', messageOf(error));
      setFailure('enable');
    } finally {
      setBusy(false);
    }
  }, [userId, runSync]);

  const turnOff = useCallback(async () => {
    if (!userId) return;
    setFailure(null);
    await disableDeviceHealth(userId);
  }, [userId]);

  const deleteSyncedData = useCallback(async () => {
    if (!userId) return;
    setBusy(true);
    try {
      await deleteDeviceHealthData(userId);
      setFailure(null);
    } catch (error) {
      console.warn('Device health data could not be deleted:', messageOf(error));
      setFailure('delete');
    } finally {
      setBusy(false);
    }
  }, [userId]);

  return {
    visible: Boolean(userId) && source !== null && available,
    source,
    enabled: state?.enabled ?? false,
    syncing,
    busy,
    lastSyncAt: state?.lastSyncAt ?? null,
    lastCounts: state?.lastCounts ?? null,
    received: state?.received ?? false,
    failure,
    turnOn,
    turnOff,
    deleteSyncedData,
  };
}

/**
 * Sync the store when the signed-in app opens and on every return to the
 * foreground. Does nothing while the athlete's toggle is off.
 */
export function useDeviceHealthForegroundSync(): void {
  const userId = useSignedInUserId();

  useEffect(() => {
    if (!userId || deviceHealthSource() === null) return undefined;
    const run = () => {
      syncDeviceHealth(userId).catch((error: unknown) => {
        console.warn('Device health sync failed:', messageOf(error));
      });
    };
    run();
    const subscription = AppState.addEventListener('change', (status: AppStateStatus) => {
      if (status === 'active') run();
    });
    return () => subscription.remove();
  }, [userId]);
}
