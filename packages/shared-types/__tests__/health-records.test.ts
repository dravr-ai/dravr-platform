// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The health records wire shapes: omitted lists read as empty, null and omitted optionals read alike
// ABOUTME: A body of any other shape — a wrong status, a non-string check, a missing envelope — reads as null

import { describe, expect, it } from 'vitest';
import {
  parseHealthRecordReview,
  parseHealthRecordsPage,
  parseLabDocumentList,
  parseLabDocumentView,
  parseLabRecord,
} from '../src/health-records';

const FLAT = {
  code_system: 'http://loinc.org',
  code: '718-7',
  display: 'Hemoglobin',
  value: 150,
  unit: 'g/L',
  value_code: null,
  interpretation: null,
  ref_low: 135,
  ref_high: 175,
  status: 'final',
  components: [],
  effective_at: '2026-05-05',
};

const LAB_RECORD = {
  id: 'rec-1',
  flat: FLAT,
  checks: ['date_missing'],
  locator: 'text:page=1;line=4',
  review_decision: null,
  effective_at: '2026-05-05T00:00:00Z',
};

const DOCUMENT = {
  id: 'doc-1',
  status: 'read',
  pages: 2,
  size_bytes: 40_000,
  engine: 'rules',
  flagged_lines: 1,
  skipped: 0,
  dropped: 0,
  model_failure: null,
  received_at: '2026-11-20T10:00:00Z',
};

describe('parseHealthRecordsPage', () => {
  it('reads an empty body, since the server omits every empty list', () => {
    expect(parseHealthRecordsPage({})).toEqual({ records: [], selfReports: [], readings: [], daily: [] });
  });

  it('reads served records with their optionals omitted', () => {
    const page = parseHealthRecordsPage({
      records: [
        {
          id: 'r1',
          resource_type: 'Observation',
          tier: 't2',
          source: 'upload',
          resource: { resourceType: 'Observation' },
          flat: FLAT,
          derivation: 'read',
        },
      ],
    });
    expect(page?.records[0]).toMatchObject({
      id: 'r1',
      resourceType: 'Observation',
      sensitivityClass: null,
      effectiveAt: null,
      readBy: null,
      flat: { code: '718-7', refLow: 135, refHigh: 175, valueCode: null },
    });
  });

  it('reads glucose readings and days, and cycle tracking', () => {
    const page = parseHealthRecordsPage({
      readings: [{ at: '2026-11-20T08:00:00Z', mg_dl: 92, source: 'dexcom' }],
      daily: [{ date: '2026-11-20', mg_dl: 101.5, source: 'dexcom' }],
      self_reports: [
        { id: 's1', kind: 'menstrual_flow', start: 'a', end: 'b', value_code: 'light', cycle_start: true, source: 'apple' },
      ],
    });
    expect(page?.readings).toEqual([{ at: '2026-11-20T08:00:00Z', mgDl: 92, source: 'dexcom' }]);
    expect(page?.daily).toEqual([{ date: '2026-11-20', mgDl: 101.5, source: 'dexcom' }]);
    expect(page?.selfReports[0]).toMatchObject({ valueCode: 'light', value: null, unit: null, cycleStart: true });
  });

  it('refuses a list holding a malformed entry', () => {
    expect(parseHealthRecordsPage({ readings: [{ at: 'x', source: 'y' }] })).toBeNull();
    expect(parseHealthRecordsPage({ records: 'none' })).toBeNull();
    expect(parseHealthRecordsPage(null)).toBeNull();
  });
});

describe('parseLabRecord', () => {
  it('reads a pending record', () => {
    expect(parseLabRecord(LAB_RECORD)).toMatchObject({
      id: 'rec-1',
      checks: ['date_missing'],
      reviewDecision: null,
      flat: { display: 'Hemoglobin', value: 150, unit: 'g/L' },
    });
  });

  it('reads a null flat and null checks, which a stored column that did not parse becomes', () => {
    expect(parseLabRecord({ ...LAB_RECORD, flat: null, checks: null })).toMatchObject({ flat: null, checks: [] });
  });

  it('refuses an unknown decision and a non-string check', () => {
    expect(parseLabRecord({ ...LAB_RECORD, review_decision: 'maybe' })).toBeNull();
    expect(parseLabRecord({ ...LAB_RECORD, checks: [3] })).toBeNull();
    expect(parseLabRecord({ ...LAB_RECORD, flat: { ...FLAT, value: '150' } })).toBeNull();
  });
});

describe('parseHealthRecordReview', () => {
  it('reads the pending list out of its envelope', () => {
    expect(parseHealthRecordReview({ pending: [LAB_RECORD] })).toHaveLength(1);
    expect(parseHealthRecordReview({ pending: [] })).toEqual([]);
    expect(parseHealthRecordReview({})).toBeNull();
  });
});

describe('parseLabDocumentView', () => {
  it('reads an upload answer, new or already held', () => {
    const view = parseLabDocumentView({ document: DOCUMENT, records: [LAB_RECORD], new: true });
    expect(view).toMatchObject({ isNew: true, document: { id: 'doc-1', sizeBytes: 40_000, modelFailure: null } });
    expect(parseLabDocumentView({ document: DOCUMENT, records: [], new: false })?.isNew).toBe(false);
  });

  it('reads a scan with no text layer', () => {
    const view = parseLabDocumentView({ document: { ...DOCUMENT, status: 'no_text', engine: null }, records: [], new: true });
    expect(view?.document.status).toBe('no_text');
  });

  it('refuses an unknown status or a missing new', () => {
    expect(parseLabDocumentView({ document: { ...DOCUMENT, status: 'ocr' }, records: [], new: true })).toBeNull();
    expect(parseLabDocumentView({ document: DOCUMENT, records: [] })).toBeNull();
  });

  it('reads the documents list out of its envelope', () => {
    expect(parseLabDocumentList({ documents: [{ document: DOCUMENT, records: [], new: false }] })).toHaveLength(1);
    expect(parseLabDocumentList({ documents: null })).toBeNull();
  });
});
