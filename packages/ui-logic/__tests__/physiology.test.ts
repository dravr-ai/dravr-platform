// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for Settings › Physiology's shared logic — the facts under an anchor and the read keyed on language and units
// ABOUTME: A missing day says so, a range shows only beside a point, and a units switch reads the anchors again

import { describe, it, expect, vi } from 'vitest';
import { createElement, type ReactNode } from 'react';
import { renderHook, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import type { PhysiologyAnchor, PhysiologyResponse } from '@pierre/shared-types';
import { createPhysiologyHook, physiologyAnchorFacts } from '../src/physiology';
import { UnitsContext } from '../src/unitsHook';

const MAP: PhysiologyAnchor = {
  anchor: 'map',
  unit: 'watts',
  value: 380,
  low: 365,
  high: 395,
  origin: 'lab',
  as_of: '2026-11-02',
  method: 'ramp peak',
  confidence: 'high',
  value_text: '380 W',
  range_text: '365–395 W',
  source_text: 'from lab',
};

/** A `t` that shows the key and its values, so a test reads which line was chosen. */
const t = (key: string, values?: Record<string, unknown>) =>
  values === undefined ? key : `${key} ${JSON.stringify(values)}`;

describe('physiologyAnchorFacts', () => {
  it('reads the day, the range beside a point value, then the stored confidence', () => {
    expect(physiologyAnchorFacts(t, MAP, 'en')).toEqual([
      'physiology.asOf {"date":"Nov 2, 2026"}',
      'physiology.range {"range":"365–395 W"}',
      'physiology.confidence.high',
    ]);
  });

  it('writes the day in the app language, on its calendar day wherever the device is', () => {
    const [day] = physiologyAnchorFacts(t, MAP, 'fr');
    expect(day).toBe('physiology.asOf {"date":"2 nov. 2026"}');
  });

  it('says no day is recorded, and adds no range the value already reads as', () => {
    const estimate: PhysiologyAnchor = {
      ...MAP,
      as_of: null,
      confidence: 'low',
      value_text: 'about 365–395 W (estimate)',
      range_text: null,
    };
    expect(physiologyAnchorFacts(t, estimate, 'en')).toEqual(['physiology.noDate', 'physiology.confidence.low']);
  });
});

describe('usePhysiology', () => {
  it('reads the anchors in the app language and again when the units change', async () => {
    const body: PhysiologyResponse = { sports: [{ sport: 'ride', anchors: [MAP] }] };
    const getPhysiology = vi.fn().mockResolvedValue(body);
    const usePhysiology = createPhysiologyHook({ getPhysiology }, { useLanguage: () => 'fr' });
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    let units: 'metric' | 'imperial' = 'metric';
    const wrapper = ({ children }: { children: ReactNode }) =>
      createElement(
        QueryClientProvider,
        { client },
        createElement(UnitsContext.Provider, { value: units }, children),
      );

    const { result, rerender } = renderHook(() => usePhysiology(), { wrapper });
    await waitFor(() => expect(result.current.data).toEqual(body));
    expect(getPhysiology).toHaveBeenCalledWith('fr');
    expect(client.getQueryData(QUERY_KEYS.user.physiology('fr', 'metric'))).toEqual(body);

    units = 'imperial';
    rerender();
    await waitFor(() => expect(getPhysiology).toHaveBeenCalledTimes(2));
    expect(client.getQueryData(QUERY_KEYS.user.physiology('fr', 'imperial'))).toEqual(body);
  });
});
