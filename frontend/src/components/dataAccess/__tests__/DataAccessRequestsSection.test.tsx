// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the web "Ask a company for your data" section — the form's checks, a draft written, its letter, and the steps after sending it
// ABOUTME: Runs the real shared hooks over a mocked API, so the request each step sends and the due date it shows are the server's (carnet#1646)

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, fireEvent, waitFor, within } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { DataAccessRequest } from '@pierre/shared-types';
import DataAccessRequestsSection from '../DataAccessRequestsSection';

const { api, openImportWindow } = vi.hoisted(() => ({
  api: {
    list: vi.fn(),
    create: vi.fn(),
    get: vi.fn(),
    edit: vi.fn(),
    letter: vi.fn(),
    markSent: vi.fn(),
    recordExtension: vi.fn(),
    markAnswered: vi.fn(),
    close: vi.fn(),
  },
  openImportWindow: vi.fn(),
}));

vi.mock('../../../services/api', () => ({ dataAccessRequestsApi: api }));
vi.mock('../../import/openImportWindow', () => ({ openImportWindow }));

const DRAFT: DataAccessRequest = {
  id: 'r1',
  reference: 'DRV-2026-AB3XZ7',
  controller: 'Acme Fitness',
  controller_contact: 'privacy@acme.example',
  athlete_identifier: 'ana@example.com',
  law: 'quebec_p39_1',
  scope: 'access_portability',
  letter_locale: 'en',
  status: 'drafted',
  derived_state: null,
  sent_on: null,
  due_on: null,
  extended_until: null,
  answered_on: null,
  created_at: '2026-11-18T15:00:00Z',
  updated_at: '2026-11-18T15:00:00Z',
};

const LETTER = {
  locale: 'en',
  subject: 'Request for access to personal information — reference DRV-2026-AB3XZ7',
  body: 'Date: 18 November 2026\n\nTo the person in charge of the protection of personal information at Acme Fitness,',
  text: 'Request for access to personal information — reference DRV-2026-AB3XZ7\n\nDate: 18 November 2026\n',
  filename: 'DRV-2026-AB3XZ7.txt',
  mailto: 'mailto:privacy@acme.example?subject=x&body=y',
};

function renderSection(props: Parameters<typeof DataAccessRequestsSection>[0] = {}) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <DataAccessRequestsSection {...props} />
    </QueryClientProvider>,
  );
}

describe('DataAccessRequestsSection', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    api.letter.mockResolvedValue(LETTER);
  });

  it("opens the request a reminder names, once the list has it (carnet#1659)", async () => {
    const sent = { ...DRAFT, status: 'sent' as const, sent_on: '2026-11-10', due_on: '2026-12-10' };
    api.list.mockResolvedValue([{ ...DRAFT, id: 'r0', controller: 'Other Co' }, sent]);
    const onRequestOpened = vi.fn();
    renderSection({ openRequestId: 'r1', onRequestOpened });

    expect(await screen.findByTestId('data-access-detail-status')).toHaveTextContent('Sent');
    expect(screen.getByTestId('data-access-row-r1')).toHaveAttribute('aria-current', 'true');
    expect(screen.getByTestId('data-access-row-r0')).not.toHaveAttribute('aria-current');
    expect(onRequestOpened).toHaveBeenCalledTimes(1);
  });

  it('opens nothing for a reminder whose request is gone, and lets it go', async () => {
    api.list.mockResolvedValue([DRAFT]);
    const onRequestOpened = vi.fn();
    renderSection({ openRequestId: 'deleted', onRequestOpened });

    expect(await screen.findByTestId('data-access-list')).toBeInTheDocument();
    await waitFor(() => expect(onRequestOpened).toHaveBeenCalledTimes(1));
    expect(screen.queryByTestId('data-access-detail')).not.toBeInTheDocument();
  });

  it('says there are no requests yet, and links the import screen as the first path', async () => {
    api.list.mockResolvedValue([]);
    renderSection();

    expect(await screen.findByTestId('data-access-empty')).toBeInTheDocument();
    fireEvent.click(screen.getByTestId('data-access-fallback-action'));
    expect(openImportWindow).toHaveBeenCalledTimes(1);
  });

  it('names each missing field and sends nothing', async () => {
    api.list.mockResolvedValue([]);
    renderSection();

    fireEvent.click(await screen.findByTestId('data-access-new'));
    fireEvent.click(screen.getByTestId('data-access-submit'));

    expect(await screen.findByText("Enter the company's name.")).toBeInTheDocument();
    expect(screen.getByText("Enter the company's privacy contact address.")).toBeInTheDocument();
    expect(screen.getByText('Enter the identifier you use with that company.')).toBeInTheDocument();
    expect(api.create).not.toHaveBeenCalled();
  });

  it('writes a draft and shows its letter, with copy, download and mail', async () => {
    api.list.mockResolvedValue([]);
    api.create.mockResolvedValue(DRAFT);
    renderSection();

    fireEvent.click(await screen.findByTestId('data-access-new'));
    fireEvent.change(screen.getByTestId('data-access-company'), { target: { value: ' Acme Fitness ' } });
    fireEvent.change(screen.getByTestId('data-access-contact'), { target: { value: 'privacy@acme.example' } });
    fireEvent.change(screen.getByTestId('data-access-identifier'), { target: { value: 'ana@example.com' } });
    fireEvent.click(screen.getByTestId('data-access-law-gdpr'));
    fireEvent.click(screen.getByTestId('data-access-scope-access'));
    fireEvent.change(screen.getByTestId('data-access-language'), { target: { value: 'de' } });
    fireEvent.click(screen.getByTestId('data-access-submit'));

    await waitFor(() =>
      expect(api.create).toHaveBeenCalledWith({
        controller: 'Acme Fitness',
        controller_contact: 'privacy@acme.example',
        athlete_identifier: 'ana@example.com',
        law: 'gdpr',
        scope: 'access',
        letter_locale: 'de',
      }),
    );
    expect(await screen.findByTestId('data-access-letter')).toHaveTextContent('reference DRV-2026-AB3XZ7');
    expect(api.letter).toHaveBeenCalledWith('r1', 'en');
    expect(screen.getByTestId('data-access-mailto')).toHaveAttribute('href', LETTER.mailto);
    expect(within(screen.getByTestId('data-access-list')).getByText('Draft')).toBeInTheDocument();
  });

  it('marks a draft sent on the day given and shows the due date the server counted', async () => {
    api.list.mockResolvedValue([DRAFT]);
    api.markSent.mockResolvedValue({ ...DRAFT, status: 'sent', sent_on: '2026-11-10', due_on: '2026-12-10' });
    renderSection();

    fireEvent.click(await screen.findByTestId('data-access-row-r1'));
    fireEvent.click(await screen.findByTestId('data-access-mark-sent'));
    fireEvent.change(screen.getByTestId('data-access-step-date'), { target: { value: '2026-11-10' } });
    fireEvent.click(screen.getByTestId('data-access-step-confirm'));

    await waitFor(() => expect(api.markSent).toHaveBeenCalledWith('r1', '2026-11-10'));
    expect(await screen.findByText('Due December 10, 2026')).toBeInTheDocument();
    expect(screen.getByTestId('data-access-detail-status')).toHaveTextContent('Sent');
    // A Quebec request has no extension.
    expect(screen.queryByTestId('data-access-record-extension')).not.toBeInTheDocument();
  });

  it('refuses a sent day in the future before asking the server', async () => {
    api.list.mockResolvedValue([DRAFT]);
    renderSection();

    fireEvent.click(await screen.findByTestId('data-access-row-r1'));
    fireEvent.click(await screen.findByTestId('data-access-mark-sent'));
    fireEvent.change(screen.getByTestId('data-access-step-date'), { target: { value: '2999-01-01' } });
    fireEvent.click(screen.getByTestId('data-access-step-confirm'));

    expect(await screen.findByText("The date sent can't be in the future.")).toBeInTheDocument();
    expect(api.markSent).not.toHaveBeenCalled();
  });

  it('offers an extension on an overdue GDPR request and names the recourse', async () => {
    const overdue: DataAccessRequest = {
      ...DRAFT,
      law: 'gdpr',
      status: 'sent',
      derived_state: 'overdue',
      sent_on: '2026-10-01',
      due_on: '2026-11-01',
    };
    api.list.mockResolvedValue([overdue]);
    renderSection();

    expect(await screen.findByText('Overdue')).toBeInTheDocument();
    fireEvent.click(screen.getByTestId('data-access-row-r1'));
    expect(await screen.findByTestId('data-access-recourse')).toHaveTextContent('supervisory authority');
    fireEvent.click(screen.getByTestId('data-access-record-extension'));
    fireEvent.change(screen.getByTestId('data-access-step-date'), { target: { value: '2027-02-01' } });
    fireEvent.click(screen.getByTestId('data-access-step-confirm'));

    expect(
      await screen.findByText('The new due date must be after the current one, and at most two more months away.'),
    ).toBeInTheDocument();
    expect(api.recordExtension).not.toHaveBeenCalled();
  });

  it('says so when the list cannot be read', async () => {
    api.list.mockRejectedValue(new Error('down'));
    renderSection();

    expect(await screen.findByTestId('data-access-load-failure')).toBeInTheDocument();
  });
});
