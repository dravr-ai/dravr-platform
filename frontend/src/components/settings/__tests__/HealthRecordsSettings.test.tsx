// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for the privacy settings' health records block — upload outcomes, the review queue's answers, delete, records by kind
// ABOUTME: The health records API is mocked at the services barrel; words are read through i18n.t so they follow the catalogue

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { i18n } from '@pierre/i18n';
import { formatDate } from '@pierre/chat-utils';
import type { FlatObservation, HealthRecordsPage, LabDocumentView, LabRecord } from '@pierre/shared-types';
import HealthRecordsSettings from '../HealthRecordsSettings';

const listRecords = vi.fn();
const getReview = vi.fn();
const confirm = vi.fn();
const reject = vi.fn();
const uploadLabDocument = vi.fn();
const listLabDocuments = vi.fn();
const deleteLabDocument = vi.fn();

vi.mock('../../../services/api/healthRecords', () => ({
  healthRecordsApi: {
    listRecords: (...a: unknown[]) => listRecords(...a),
    getReview: (...a: unknown[]) => getReview(...a),
    confirm: (...a: unknown[]) => confirm(...a),
    reject: (...a: unknown[]) => reject(...a),
    uploadLabDocument: (...a: unknown[]) => uploadLabDocument(...a),
    listLabDocuments: (...a: unknown[]) => listLabDocuments(...a),
    deleteLabDocument: (...a: unknown[]) => deleteLabDocument(...a),
  },
}));

vi.mock('../../../hooks/useAuth', () => ({
  useAuth: () => ({ isAuthenticated: true }),
}));

const FLAT: FlatObservation = {
  codeSystem: 'http://loinc.org',
  code: '718-7',
  display: 'Hemoglobin',
  value: 150,
  unit: 'g/L',
  valueCode: null,
  interpretation: null,
  refLow: 135,
  refHigh: 175,
  status: 'final',
  components: [],
  effectiveAt: null,
};

const PENDING: LabRecord = {
  id: 'rec-1',
  flat: FLAT,
  checks: ['date_missing'],
  locator: 'text:page=1;line=4',
  reviewDecision: null,
  effectiveAt: null,
};

function documentView(overrides: Partial<LabDocumentView> = {}, status: 'read' | 'no_text' = 'read'): LabDocumentView {
  return {
    document: {
      id: 'doc-1',
      status,
      pages: 1,
      sizeBytes: 1000,
      engine: 'rules',
      flaggedLines: 0,
      skipped: 0,
      dropped: 0,
      modelFailure: null,
      receivedAt: '2026-11-20T10:00:00Z',
    },
    records: [PENDING],
    isNew: true,
    ...overrides,
  };
}

const EMPTY_PAGE: HealthRecordsPage = { records: [], selfReports: [], readings: [], daily: [] };

// jsdom's Blob predates `arrayBuffer()`, which every browser the app supports
// has; read it through the FileReader jsdom does implement.
if (typeof Blob.prototype.arrayBuffer !== 'function') {
  Blob.prototype.arrayBuffer = function arrayBuffer(this: Blob) {
    return new Promise<ArrayBuffer>((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => resolve(reader.result as ArrayBuffer);
      reader.onerror = () => reject(reader.error);
      reader.readAsArrayBuffer(this);
    });
  };
}

function pdf(): File {
  return new File([new Uint8Array([0x25, 0x50, 0x44, 0x46])], 'labs.pdf', { type: 'application/pdf' });
}

function renderPane() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <HealthRecordsSettings />
    </QueryClientProvider>,
  );
}

describe('HealthRecordsSettings', () => {
  let pending: LabRecord[];

  beforeEach(() => {
    vi.clearAllMocks();
    pending = [PENDING];
    getReview.mockImplementation(async () => pending);
    listLabDocuments.mockResolvedValue([documentView({ isNew: false })]);
    listRecords.mockResolvedValue(EMPTY_PAGE);
    confirm.mockImplementation(async () => {
      pending = [];
    });
    reject.mockImplementation(async () => {
      pending = [];
    });
    deleteLabDocument.mockResolvedValue(undefined);
  });

  it('shows each pending record with its value, reference range and checks', async () => {
    renderPane();
    const row = await screen.findByTestId('health-review-rec-1');
    expect(within(row).getByText('Hemoglobin')).toBeInTheDocument();
    expect(screen.getByTestId('health-review-value-rec-1')).toHaveTextContent('150 g/L');
    expect(screen.getByTestId('health-review-value-rec-1')).toHaveTextContent('135–175');
    expect(screen.getByTestId('health-review-checks-rec-1')).toHaveTextContent(
      `${i18n.t('healthRecords.checksLabel')}: ${i18n.t('healthRecords.checks.date_missing', { defaultValue: 'date_missing' })}`,
    );
  });

  it('confirms a pending record, then reads the queue again', async () => {
    const user = userEvent.setup();
    renderPane();
    await user.click(await screen.findByTestId('health-review-confirm-rec-1'));
    expect(confirm).toHaveBeenCalledWith('rec-1');
    expect(await screen.findByTestId('health-records-review-empty')).toHaveTextContent(
      i18n.t('healthRecords.reviewEmpty'),
    );
  });

  it('rejects a pending record', async () => {
    const user = userEvent.setup();
    renderPane();
    await user.click(await screen.findByTestId('health-review-reject-rec-1'));
    expect(reject).toHaveBeenCalledWith('rec-1');
    expect(confirm).not.toHaveBeenCalled();
  });

  it('says how many results a new report gave', async () => {
    uploadLabDocument.mockResolvedValue(documentView({ records: [PENDING, { ...PENDING, id: 'rec-2' }] }));
    const user = userEvent.setup();
    renderPane();
    await user.upload(await screen.findByTestId('health-records-upload-input'), pdf());
    expect(await screen.findByTestId('health-records-outcome-uploaded')).toHaveTextContent(
      i18n.t('healthRecords.uploaded', { count: 2 }),
    );
    expect(uploadLabDocument).toHaveBeenCalledTimes(1);
  });

  it('says a report was already uploaded, or had no text', async () => {
    uploadLabDocument.mockResolvedValueOnce(documentView({ isNew: false }));
    uploadLabDocument.mockResolvedValueOnce(documentView({ records: [] }, 'no_text'));
    const user = userEvent.setup();
    renderPane();
    const input = await screen.findByTestId('health-records-upload-input');
    await user.upload(input, pdf());
    expect(await screen.findByTestId('health-records-outcome-alreadyUploaded')).toHaveTextContent(
      i18n.t('healthRecords.alreadyUploaded'),
    );
    await user.upload(input, pdf());
    expect(await screen.findByTestId('health-records-outcome-noText')).toHaveTextContent(i18n.t('healthRecords.noText'));
  });

  it('says a refused upload failed', async () => {
    uploadLabDocument.mockRejectedValue(new Error('400'));
    const user = userEvent.setup();
    renderPane();
    await user.upload(await screen.findByTestId('health-records-upload-input'), pdf());
    expect(await screen.findByRole('alert')).toHaveTextContent(i18n.t('healthRecords.uploadFailed'));
  });

  it('asks before deleting a lab report, and keeps it when the athlete cancels', async () => {
    const user = userEvent.setup();
    renderPane();
    await user.click(await screen.findByTestId('health-document-delete-doc-1'));
    const dialog = await screen.findByRole('dialog');
    expect(within(dialog).getByText(i18n.t('healthRecords.deleteConfirmTitle'))).toBeInTheDocument();
    expect(within(dialog).getByText(i18n.t('healthRecords.deleteConfirmBody'))).toBeInTheDocument();
    await user.click(within(dialog).getByRole('button', { name: i18n.t('common.cancel') }));
    expect(deleteLabDocument).not.toHaveBeenCalled();
  });

  it('deletes a lab report once confirmed', async () => {
    const user = userEvent.setup();
    renderPane();
    await user.click(await screen.findByTestId('health-document-delete-doc-1'));
    const dialog = await screen.findByRole('dialog');
    await user.click(within(dialog).getByRole('button', { name: i18n.t('healthRecords.deleteDocument') }));
    expect(deleteLabDocument).toHaveBeenCalledWith('doc-1');
    expect(await screen.findByRole('status')).toHaveTextContent(i18n.t('healthRecords.deleted'));
  });

  it('says a failed read failed, never that nothing waits', async () => {
    getReview.mockRejectedValue(new Error('offline'));
    listLabDocuments.mockRejectedValue(new Error('offline'));
    renderPane();
    expect(await screen.findByTestId('health-records-review-failed')).toHaveTextContent(i18n.t('healthRecords.failed'));
    expect(await screen.findByTestId('health-records-documents-failed')).toBeInTheDocument();
    expect(screen.queryByTestId('health-records-review-empty')).not.toBeInTheDocument();
    expect(screen.queryByTestId('health-records-documents-empty')).not.toBeInTheDocument();
  });

  it('says when no report was uploaded and nothing waits', async () => {
    pending = [];
    listLabDocuments.mockResolvedValue([]);
    renderPane();
    expect(await screen.findByTestId('health-records-documents-empty')).toBeInTheDocument();
    expect(await screen.findByTestId('health-records-review-empty')).toBeInTheDocument();
  });

  it('reads the records of the chosen kind; glucose shows readings and daily values', async () => {
    listRecords.mockImplementation(async (kind: string) =>
      kind === 'glucose'
        ? {
            ...EMPTY_PAGE,
            readings: [{ at: '2026-11-20T08:00:00Z', mgDl: 92, source: 'dexcom' }],
            daily: [{ date: '2026-11-20', mgDl: 101, source: 'dexcom' }],
          }
        : EMPTY_PAGE,
    );
    const user = userEvent.setup();
    renderPane();
    expect(await screen.findByTestId('health-records-empty')).toBeInTheDocument();
    expect(listRecords).toHaveBeenCalledWith('labs', undefined);
    expect(screen.getByTestId('health-records-kind-labs')).toHaveAttribute('aria-pressed', 'true');

    await user.click(screen.getByTestId('health-records-kind-glucose'));
    expect(listRecords).toHaveBeenCalledWith('glucose', undefined);
    expect(await screen.findByTestId('health-glucose-reading')).toHaveTextContent('92 mg/dL');
    expect(screen.getByTestId('health-glucose-day')).toHaveTextContent('101 mg/dL');
    expect(screen.getByText(i18n.t('healthRecords.dailyGlucose'))).toBeInTheDocument();
  });

  it('lists served lab results with their range', async () => {
    listRecords.mockResolvedValue({
      ...EMPTY_PAGE,
      records: [
        {
          id: 'served-1',
          resourceType: 'Observation',
          tier: 't2',
          sensitivityClass: null,
          effectiveAt: '2026-05-05T00:00:00Z',
          source: 'upload',
          resource: {},
          flat: FLAT,
          derivation: 'confirmed',
          readBy: 'rules',
        },
      ],
    });
    renderPane();
    const row = await screen.findByTestId('health-record-served-1');
    expect(row).toHaveTextContent('Hemoglobin');
    expect(row).toHaveTextContent('150 g/L');
    expect(row).toHaveTextContent('(135–175)');
    // A lab date is a day, stored at midnight UTC: it is said as that day wherever the athlete is.
    expect(row).toHaveTextContent(formatDate('2026-05-05T00:00:00Z', i18n.language, 'UTC'));
    expect(formatDate('2026-05-05T00:00:00Z', i18n.language, 'UTC')).toContain('5');
  });

  it('names a cycle-tracking entry by its kind and its coded value', async () => {
    listRecords.mockImplementation(async (kind: string) =>
      kind === 'self_reports'
        ? {
            ...EMPTY_PAGE,
            selfReports: [
              {
                id: 's1',
                kind: 'menstrual_flow',
                start: '2026-11-20T00:00:00Z',
                end: '2026-11-20T00:00:00Z',
                valueCode: 'light',
                value: null,
                unit: null,
                cycleStart: true,
                source: 'apple_health',
              },
            ],
          }
        : EMPTY_PAGE,
    );
    const user = userEvent.setup();
    renderPane();
    await screen.findByTestId('health-records-empty');
    await user.click(screen.getByTestId('health-records-kind-self_reports'));
    const row = await screen.findByTestId('health-self-report-s1');
    expect(row).toHaveTextContent(
      i18n.t('healthRecords.selfReport.kind.menstrual_flow', { defaultValue: 'menstrual_flow' }),
    );
    expect(row).toHaveTextContent(i18n.t('healthRecords.selfReport.value.light', { defaultValue: 'light' }));
  });
});
