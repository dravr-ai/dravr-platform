// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Which tenants an admin asks the scoped admin screens to read, shared across those screens
// ABOUTME: Only admin screens mount it; the server decides what any request may actually read

import { useCallback, useSyncExternalStore } from 'react';

/** What a scoped endpoint reports it read: the server's decision, never the client's. */
export type AdminScope =
  | { kind: 'all_tenants' }
  | { kind: 'tenant'; tenant_id: string };

/** The `tenant_id` value asking for every tenant. */
export const ALL_TENANTS = 'all';

const STORAGE_KEY = 'pierre.admin_scope';

const listeners = new Set<() => void>();

/** A pick the browser refused to store, which holds until the page reloads. */
let unstoredPick: string | null = null;

function readStored(): string {
  if (unstoredPick !== null) return unstoredPick;
  try {
    return localStorage.getItem(STORAGE_KEY) ?? ALL_TENANTS;
  } catch {
    return ALL_TENANTS;
  }
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  window.addEventListener('storage', listener);
  return () => {
    listeners.delete(listener);
    window.removeEventListener('storage', listener);
  };
}

/**
 * The scope the admin asks for.
 *
 * `requested` is the `tenant_id` a scoped read sends: every tenant until the
 * admin picks one. Every screen that reads it sits behind the Dashboard's
 * route guard, which mounts no admin tab for anyone but an admin.
 */
export function useAdminScope() {
  const requested = useSyncExternalStore(subscribe, readStored);

  const pick = useCallback((value: string) => {
    try {
      localStorage.setItem(STORAGE_KEY, value);
      unstoredPick = null;
    } catch {
      unstoredPick = value;
    }
    listeners.forEach((listener) => listener());
  }, []);

  return { requested, pick } as const;
}
