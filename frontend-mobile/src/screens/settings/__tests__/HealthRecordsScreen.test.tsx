// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the phone's health records screen — a picked PDF goes up as its bytes, the review queue's answers, delete, kinds
// ABOUTME: The health records API is mocked; words are read through i18n.t so they follow the catalogue

import React from 'react';
import { Alert } from 'react-native';
import { fireEvent, render, screen, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import * as DocumentPicker from 'expo-document-picker';
import { i18n } from '@pierre/i18n';
import { formatDate } from '@pierre/chat-utils';
import type { FlatObservation, HealthRecordsPage, LabDocumentView, LabRecord } from '@pierre/shared-types';
import { HealthRecordsScreen } from '../HealthRecordsScreen';
import { healthRecordsApi } from '../../../services/api';

jest.mock('../../../services/api', () => ({
  healthRecordsApi: {
    listRecords: jest.fn(),
    getReview: jest.fn(),
    confirm: jest.fn(),
    reject: jest.fn(),
    uploadLabDocument: jest.fn(),
    listLabDocuments: jest.fn(),
    deleteLabDocument: jest.fn(),
  },
}));
jest.mock('../../../contexts/AuthContext', () => ({
  useAuth: () => ({ isAuthenticated: true }),
}));

const { __mockFileContents: files } = require('expo-file-system') as { __mockFileContents: Record<string, string> };

const api = jest.mocked(healthRecordsApi);

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
  locator: null,
  reviewDecision: null,
  effectiveAt: null,
};

function documentView(overrides: Partial<LabDocumentView> = {}): LabDocumentView {
  return {
    document: {
      id: 'doc-1',
      status: 'read',
      pages: 1,
      sizeBytes: 4,
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

/** The picker hands back one file, `labs.pdf`, holding `content`. */
function picks(content: string) {
  const uri = 'file:///cache/labs.pdf';
  files[uri] = content;
  jest.mocked(DocumentPicker.getDocumentAsync).mockResolvedValueOnce({
    canceled: false,
    assets: [{ uri, name: 'labs.pdf', size: content.length, mimeType: 'application/pdf', lastModified: 0 }],
  } as DocumentPicker.DocumentPickerResult);
}

function renderScreen() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <HealthRecordsScreen />
    </QueryClientProvider>,
  );
}

describe('HealthRecordsScreen', () => {
  let pending: LabRecord[];

  beforeEach(() => {
    jest.clearAllMocks();
    jest.spyOn(Alert, 'alert').mockImplementation(() => undefined);
    pending = [PENDING];
    api.getReview.mockImplementation(async () => pending);
    api.listLabDocuments.mockResolvedValue([documentView({ isNew: false })]);
    api.listRecords.mockResolvedValue(EMPTY_PAGE);
    api.confirm.mockImplementation(async () => {
      pending = [];
    });
    api.reject.mockImplementation(async () => {
      pending = [];
    });
    api.deleteLabDocument.mockResolvedValue(undefined);
  });

  it('shows each pending record with its value, reference range and checks', async () => {
    renderScreen();
    expect(await screen.findByTestId('health-review-value-rec-1')).toHaveTextContent('150 g/L (135–175)');
    expect(screen.getByText('Hemoglobin')).toBeTruthy();
    expect(screen.getByTestId('health-review-checks-rec-1')).toHaveTextContent(
      `${i18n.t('healthRecords.checksLabel')}: date_missing`,
    );
  });

  it('confirms a pending record, then reads the queue again', async () => {
    renderScreen();
    fireEvent.press(await screen.findByTestId('health-review-confirm-rec-1'));
    await waitFor(() => expect(api.confirm).toHaveBeenCalledWith('rec-1'));
    expect(await screen.findByTestId('health-records-review-empty')).toHaveTextContent(
      i18n.t('healthRecords.reviewEmpty'),
    );
  });

  it('rejects a pending record', async () => {
    renderScreen();
    fireEvent.press(await screen.findByTestId('health-review-reject-rec-1'));
    await waitFor(() => expect(api.reject).toHaveBeenCalledWith('rec-1'));
    expect(api.confirm).not.toHaveBeenCalled();
  });

  it('sends the picked PDF as its bytes and says how many results were read', async () => {
    api.uploadLabDocument.mockResolvedValue(documentView({ records: [PENDING, { ...PENDING, id: 'rec-2' }] }));
    picks('%PDF');
    renderScreen();
    fireEvent.press(screen.getByTestId('health-records-upload'));
    expect(await screen.findByTestId('health-records-outcome-uploaded')).toHaveTextContent(
      i18n.t('healthRecords.uploaded', { count: 2 }),
    );
    expect(DocumentPicker.getDocumentAsync).toHaveBeenCalledWith(expect.objectContaining({ type: 'application/pdf' }));
    const [sent] = api.uploadLabDocument.mock.calls[0];
    expect(Array.from(new Uint8Array(sent as ArrayBuffer))).toEqual([37, 80, 68, 70]);
  });

  it('says a report was already uploaded', async () => {
    api.uploadLabDocument.mockResolvedValue(documentView({ isNew: false }));
    picks('%PDF');
    renderScreen();
    fireEvent.press(screen.getByTestId('health-records-upload'));
    expect(await screen.findByTestId('health-records-outcome-alreadyUploaded')).toHaveTextContent(
      i18n.t('healthRecords.alreadyUploaded'),
    );
  });

  it('says a refused upload failed', async () => {
    api.uploadLabDocument.mockRejectedValue(new Error('400'));
    picks('%PDF');
    renderScreen();
    fireEvent.press(screen.getByTestId('health-records-upload'));
    expect(await screen.findByTestId('health-records-outcome-failed')).toHaveTextContent(
      i18n.t('healthRecords.uploadFailed'),
    );
  });

  it('sends nothing when the athlete cancels the picker', async () => {
    jest.mocked(DocumentPicker.getDocumentAsync).mockResolvedValueOnce({ canceled: true, assets: null });
    renderScreen();
    fireEvent.press(screen.getByTestId('health-records-upload'));
    await waitFor(() => expect(DocumentPicker.getDocumentAsync).toHaveBeenCalled());
    expect(api.uploadLabDocument).not.toHaveBeenCalled();
  });

  it('says a failed read failed, never that nothing waits', async () => {
    api.getReview.mockRejectedValue(new Error('offline'));
    api.listLabDocuments.mockRejectedValue(new Error('offline'));
    renderScreen();
    expect(await screen.findByTestId('health-records-review-failed')).toHaveTextContent(i18n.t('healthRecords.failed'));
    expect(await screen.findByTestId('health-records-documents-failed')).toBeTruthy();
    expect(screen.queryByTestId('health-records-review-empty')).toBeNull();
    expect(screen.queryByTestId('health-records-documents-empty')).toBeNull();
  });

  it('says a served lab result on the day it was taken', async () => {
    api.listRecords.mockResolvedValue({
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
    renderScreen();
    const row = await screen.findByTestId('health-record-served-1');
    expect(row).toHaveTextContent(
      `Hemoglobin150 g/L (135–175) · ${formatDate('2026-05-05T00:00:00Z', i18n.language, 'UTC')}`,
    );
  });

  it('deletes a lab report', async () => {
    renderScreen();
    fireEvent.press(await screen.findByTestId('health-document-delete-doc-1'));
    await waitFor(() => expect(api.deleteLabDocument).toHaveBeenCalledWith('doc-1'));
    await waitFor(() => expect(Alert.alert).toHaveBeenCalledWith(i18n.t('healthRecords.deleted')));
  });

  it('reads the records of the chosen kind; glucose shows readings and daily values', async () => {
    api.listRecords.mockImplementation(async (kind) =>
      kind === 'glucose'
        ? {
            ...EMPTY_PAGE,
            readings: [{ at: '2026-11-20T08:00:00Z', mgDl: 92, source: 'dexcom' }],
            daily: [{ date: '2026-11-20', mgDl: 101, source: 'dexcom' }],
          }
        : EMPTY_PAGE,
    );
    renderScreen();
    expect(await screen.findByTestId('health-records-empty')).toBeTruthy();
    expect(api.listRecords).toHaveBeenCalledWith('labs', undefined);

    fireEvent.press(screen.getByTestId('health-records-kind-glucose'));
    expect(await screen.findByTestId('health-glucose-reading')).toHaveTextContent('92 mg/dL', { exact: false });
    expect(screen.getByTestId('health-glucose-day')).toHaveTextContent('101 mg/dL', { exact: false });
    expect(screen.getByText(i18n.t('healthRecords.dailyGlucose'))).toBeTruthy();
  });
});
