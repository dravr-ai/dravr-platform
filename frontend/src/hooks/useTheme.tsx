// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Boreal Editorial light/dark theme context for the web app
// ABOUTME: Toggles a `.dark` class on <html> so CSS variables flip every surface

import React, { createContext, useCallback, useContext, useEffect, useMemo, useState } from 'react';

type Scheme = 'light' | 'dark';
type Preference = Scheme | 'system';

interface ThemeContextValue {
  /** The current preference — what the user asked for. */
  preference: Preference;
  /** The resolved scheme actually applied to <html>. */
  scheme: Scheme;
  /** Persist a new preference. `system` follows `prefers-color-scheme`. */
  setPreference: (next: Preference) => void;
  /** Flip between light and dark (ignores system preference). */
  toggle: () => void;
}

const STORAGE_KEY = 'dravr.theme';
const ThemeContext = createContext<ThemeContextValue | null>(null);

function readPreference(): Preference {
  if (typeof window === 'undefined') return 'system';
  const stored = window.localStorage.getItem(STORAGE_KEY);
  if (stored === 'light' || stored === 'dark' || stored === 'system') return stored;
  return 'system';
}

function resolveScheme(pref: Preference): Scheme {
  if (pref === 'system') {
    if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') {
      return 'light';
    }
    return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
  }
  return pref;
}

export function ThemeProvider({ children }: { children: React.ReactNode }) {
  const [preference, setPreferenceState] = useState<Preference>(() => readPreference());
  const [scheme, setScheme] = useState<Scheme>(() => resolveScheme(readPreference()));

  // Apply the resolved scheme to <html> whenever preference or OS scheme changes.
  useEffect(() => {
    const resolved = resolveScheme(preference);
    setScheme(resolved);
    const root = document.documentElement;
    root.classList.toggle('dark', resolved === 'dark');
  }, [preference]);

  // Listen for OS scheme changes when following the system.
  useEffect(() => {
    if (preference !== 'system') return;
    if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') return;
    const mq = window.matchMedia('(prefers-color-scheme: dark)');
    const listener = (e: MediaQueryListEvent) => {
      const next: Scheme = e.matches ? 'dark' : 'light';
      setScheme(next);
      document.documentElement.classList.toggle('dark', next === 'dark');
    };
    mq.addEventListener('change', listener);
    return () => mq.removeEventListener('change', listener);
  }, [preference]);

  const setPreference = useCallback((next: Preference) => {
    window.localStorage.setItem(STORAGE_KEY, next);
    setPreferenceState(next);
  }, []);

  const toggle = useCallback(() => {
    setPreference(scheme === 'dark' ? 'light' : 'dark');
  }, [scheme, setPreference]);

  const value = useMemo<ThemeContextValue>(
    () => ({ preference, scheme, setPreference, toggle }),
    [preference, scheme, setPreference, toggle]
  );

  return <ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>;
}

export function useTheme(): ThemeContextValue {
  const ctx = useContext(ThemeContext);
  if (!ctx) {
    throw new Error('useTheme must be used inside <ThemeProvider>');
  }
  return ctx;
}
