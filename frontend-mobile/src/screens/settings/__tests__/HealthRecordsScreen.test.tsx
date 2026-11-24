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
      `${i18n.t('healthRecords.checksLabel')}: ${i18n.t('healthRecords.checks.date_missing', { defaultValue: 'date_missing' })}`,
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

  it('asks before deleting a lab report, and deletes it once confirmed', async () => {
    renderScreen();
    fireEvent.press(await screen.findByTestId('health-document-delete-doc-1'));
    const alert = jest.mocked(Alert.alert);
    expect(alert).toHaveBeenCalledWith(
      i18n.t('healthRecords.deleteConfirmTitle'),
      i18n.t('healthRecords.deleteConfirmBody'),
      expect.any(Array),
    );
    expect(api.deleteLabDocument).not.toHaveBeenCalled();
    const buttons = alert.mock.calls[0][2] ?? [];
    expect(buttons.map((button) => button.text)).toEqual([
      i18n.t('common.cancel'),
      i18n.t('healthRecords.deleteDocument'),
    ]);
    expect(buttons[1].style).toBe('destructive');
    buttons[1].onPress?.();
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

  it('names a cycle-tracking entry by its kind and its coded value', async () => {
    api.listRecords.mockImplementation(async (kind) =>
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
    renderScreen();
    await screen.findByTestId('health-records-empty');
    fireEvent.press(screen.getByTestId('health-records-kind-self_reports'));
    const row = await screen.findByTestId('health-self-report-s1');
    expect(row).toHaveTextContent(
      i18n.t('healthRecords.selfReport.kind.menstrual_flow', { defaultValue: 'menstrual_flow' }),
      { exact: false },
    );
    expect(row).toHaveTextContent(i18n.t('healthRecords.selfReport.value.light', { defaultValue: 'light' }), {
      exact: false,
    });
  });

  it('paints every ink and fill from a theme token, so both schemes resolve it', async () => {
    // Colour on the phone is NativeWind classes over the token tree, which
    // binds each token per scheme: a class naming a token renders in light and
    // dark alike, a stock palette class or a literal does not.
    const tailwind = require('../../../../tailwind.config.js') as { theme: { extend: { colors: object } } };
    const tokens = new Set<string>();
    const walk = (node: object, prefix: string) => {
      for (const [key, value] of Object.entries(node)) {
        const name = key === 'DEFAULT' ? prefix.replace(/-$/, '') : `${prefix}${key}`;
        if (typeof value === 'object' && value !== null) walk(value, `${name}-`);
        else tokens.add(name);
      }
    };
    walk(tailwind.theme.extend.colors, '');
    const view = renderScreen();
    await screen.findByTestId('health-review-rec-1');
    await screen.findByTestId('health-document-doc-1');
    await screen.findByTestId('health-records-empty');
    const classes = view.UNSAFE_root.findAll((node) => typeof node.props.className === 'string').flatMap(
      (node) => (node.props.className as string).split(/\s+/),
    );
    const colours = classes.filter((cls) => /^(text|bg|border)-/.test(cls)).filter(
      (cls) => !/^text-(xs|sm|base|lg|xl|\dxl|left|right|center)$/.test(cls) && !/^border(-[trblxy])?(-[0-9]+)?$/.test(cls) &&
        !/^(bg|border|text)-transparent$/.test(cls),
    );
    expect(colours.length).toBeGreaterThan(0);
    const untokened = colours.filter((cls) => !tokens.has(cls.replace(/^(text|bg|border)-/, '').replace(/\/\d+$/, '')));
    expect(untokened).toEqual([]);
  });
});

