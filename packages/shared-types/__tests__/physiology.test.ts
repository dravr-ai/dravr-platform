// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the physiology parser — a served body reads through as sent, any other shape rejects the body
// ABOUTME: Red if a screen would print an anchor without the words that carry its estimate marker or its source

import { describe, expect, it } from 'vitest';
import { parsePhysiologyResponse } from '../src/physiology';

const MAP = {
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

const CRITICAL_SPEED = {
  anchor: 'critical_speed',
  unit: 'metres_per_second',
  value: 4,
  low: 3.92,
  high: 4.08,
  origin: null,
  as_of: null,
  method: null,
  confidence: 'low',
  value_text: 'about 4:05–4:15/km (estimate)',
  range_text: null,
  source_text: 'your own value',
};

describe('parsePhysiologyResponse', () => {
  it('reads a served body as sent', () => {
    const body = {
      sports: [
        { sport: 'ride', anchors: [MAP] },
        { sport: 'run', anchors: [CRITICAL_SPEED] },
      ],
    };
    expect(parsePhysiologyResponse(body)).toEqual(body);
  });

  it('reads an athlete with no anchor', () => {
    expect(parsePhysiologyResponse({ sports: [] })).toEqual({ sports: [] });
  });

  it.each([
    ['no sports', {}],
    ['a family the server does not name', { sports: [{ sport: 'row', anchors: [MAP] }] }],
    ['an anchor the server does not name', { sports: [{ sport: 'ride', anchors: [{ ...MAP, anchor: 'lthr' }] }] }],
    ['a confidence the writer cannot state', { sports: [{ sport: 'ride', anchors: [{ ...MAP, confidence: 'certain' }] }] }],
    ['an anchor without its words', { sports: [{ sport: 'ride', anchors: [{ ...MAP, value_text: '' }] }] }],
    ['an anchor without its source', { sports: [{ sport: 'ride', anchors: [{ ...MAP, source_text: undefined }] }] }],
    ['an omitted range end', { sports: [{ sport: 'ride', anchors: [{ ...MAP, low: undefined }] }] }],
    ['a day that is not a civil date', { sports: [{ sport: 'ride', anchors: [{ ...MAP, as_of: '2 Nov' }] }] }],
    ['a value that is not a number', { sports: [{ sport: 'ride', anchors: [{ ...MAP, value: '380' }] }] }],
  ])('rejects a body with %s', (_why, body) => {
    expect(parsePhysiologyResponse(body)).toBeNull();
  });
});
