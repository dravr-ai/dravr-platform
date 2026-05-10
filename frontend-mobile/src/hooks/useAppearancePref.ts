// ABOUTME: Persisted appearance preference (System / Light / Dark) for mobile theming
// ABOUTME: Defaults to 'dark' on first launch — Boreal Editorial is dark-first on mobile

import { useCallback, useEffect, useState } from 'react';
import AsyncStorage from '@react-native-async-storage/async-storage';

export type AppearancePref = 'system' | 'light' | 'dark';

const STORAGE_KEY = 'pierre.appearance_pref';
const DEFAULT_PREF: AppearancePref = 'dark';

const VALID: readonly AppearancePref[] = ['system', 'light', 'dark'] as const;

function isValidPref(value: unknown): value is AppearancePref {
  return typeof value === 'string' && (VALID as readonly string[]).includes(value);
}

interface UseAppearancePrefResult {
  /** Resolved preference; `loading` is true on the first frame while storage reads. */
  pref: AppearancePref;
  loading: boolean;
  setPref: (next: AppearancePref) => Promise<void>;
}

/**
 * Reads/writes the user's appearance preference from AsyncStorage. The first
 * frame returns `loading: true` so the ThemeProvider can avoid a flash on
 * cold start. Defaults to `'dark'` when no value has been persisted yet.
 */
export function useAppearancePref(): UseAppearancePrefResult {
  const [pref, setPrefState] = useState<AppearancePref>(DEFAULT_PREF);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const raw = await AsyncStorage.getItem(STORAGE_KEY);
        if (cancelled) return;
        if (isValidPref(raw)) {
          setPrefState(raw);
        }
      } catch {
        // Read failure is non-fatal — fall back to the default.
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const setPref = useCallback(async (next: AppearancePref) => {
    setPrefState(next);
    try {
      await AsyncStorage.setItem(STORAGE_KEY, next);
    } catch {
      // Persistence failure is non-fatal — the in-memory state still flips.
    }
  }, []);

  return { pref, loading, setPref };
}
