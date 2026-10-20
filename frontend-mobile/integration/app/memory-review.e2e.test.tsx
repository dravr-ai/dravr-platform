// ABOUTME: Drafts the agent proposed, end to end on mobile: the Memory screen lists them under To review with their quote
// ABOUTME: Keep posts the approval and Discard the delete over a stubbed transport; each draft then leaves the section

import React from 'react';
import { render, fireEvent, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { MemoryFactRow } from '@pierre/api-client';
import { i18n } from '@pierre/i18n';

import { installHttpStub, type HttpStub } from './helpers/httpStub';

jest.mock('expo-router', () => require('../../jest.expo-router').createExpoRouterMock());

import { MemoryScreen } from '../../src/screens/memory/MemoryScreen';

function draft(id: string, sentence: string, evidence: string): MemoryFactRow {
  return {
    id,
    agent_id: null,
    agent_title: null,
    kind: 'physiology',
    predicate_code: 'states',
    object: sentence,
    sentence,
    confidence: 0.7,
    source_msg_id: null,
    updated_at: '2026-10-19T18:00:00Z',
    status: 'draft',
    evidence,
    evidence_source: 'athlete',
  };
}

function renderScreen() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } });
  return render(
    <QueryClientProvider client={client}>
      <MemoryScreen />
    </QueryClientProvider>,
  );
}

describe('memory review — mobile', () => {
  let stub: HttpStub;

  afterEach(() => {
    stub.restore();
  });

  it('keeps one draft and discards the other, each on the wire', async () => {
    let drafts = [
      draft('d-ftp', 'Your FTP is around 250 W', 'I held 250 for the whole 20-min test'),
      draft('d-am', 'You prefer morning sessions', 'mornings are the only time I can ride'),
    ];
    let kept: MemoryFactRow[] = [];
    stub = installHttpStub({
      'GET /api/memory/facts?status=draft&limit=100': () => ({
        data: { facts: drafts, total: drafts.length },
      }),
      'GET /api/memory/facts?limit=100': () => ({ data: { facts: kept, total: kept.length } }),
      'POST /api/memory/facts/d-ftp/approve': () => {
        const fact = { ...drafts[0], status: 'approved' as const };
        drafts = drafts.filter((d) => d.id !== 'd-ftp');
        kept = [fact];
        return { data: { fact } };
      },
      'DELETE /api/memory/facts/d-am': () => {
        drafts = drafts.filter((d) => d.id !== 'd-am');
        return { data: { deleted: true } };
      },
    });

    const { getByTestId, getByText, queryByTestId } = renderScreen();

    await waitFor(() => {
      expect(getByText(i18n.t('shell.memoryReviewTitle', { count: 2 }))).toBeTruthy();
    });
    expect(getByTestId('memory-review-d-ftp-evidence')).toHaveTextContent(
      /I held 250 for the whole 20-min test/,
    );

    fireEvent.press(getByTestId('memory-review-d-ftp-keep'));
    await waitFor(() => {
      expect(stub.requestsFor('POST').map((r) => r.url)).toEqual(['/api/memory/facts/d-ftp/approve']);
    });
    await waitFor(() => {
      expect(getByText(i18n.t('shell.memoryReviewTitle', { count: 1 }))).toBeTruthy();
    });

    fireEvent.press(getByTestId('memory-review-d-am-discard'));
    await waitFor(() => {
      expect(stub.requestsFor('DELETE').map((r) => r.url)).toEqual(['/api/memory/facts/d-am']);
    });
    await waitFor(() => {
      expect(queryByTestId('memory-review')).toBeNull();
    });
  });
});
