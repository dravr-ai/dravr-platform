// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Viewport breakpoint hooks for responsive layout switching.
// ABOUTME: Single source of truth for mobile/tablet/desktop gates across the app.

import { useEffect, useState } from 'react';

const QUERY_MOBILE = '(max-width: 767px)';
const QUERY_TABLET = '(min-width: 768px) and (max-width: 1023px)';
const QUERY_DESKTOP = '(min-width: 1024px)';

function readMatch(query: string): boolean {
  if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') {
    return false;
  }
  return window.matchMedia(query).matches;
}

function useMediaQuery(query: string): boolean {
  const [matches, setMatches] = useState<boolean>(() => readMatch(query));

  useEffect(() => {
    if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') {
      return;
    }
    const mql = window.matchMedia(query);
    const update = (event: MediaQueryListEvent | MediaQueryList) => {
      setMatches(event.matches);
    };
    update(mql);
    mql.addEventListener('change', update);
    return () => mql.removeEventListener('change', update);
  }, [query]);

  return matches;
}

export function useIsMobile(): boolean {
  return useMediaQuery(QUERY_MOBILE);
}

export function useIsTablet(): boolean {
  return useMediaQuery(QUERY_TABLET);
}

export function useIsDesktop(): boolean {
  return useMediaQuery(QUERY_DESKTOP);
}
