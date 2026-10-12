// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
//
// ABOUTME: Unit tests for IntervalsIcuLinkModal — the authorization when offered, athlete id + API key entry and link submission
// ABOUTME: Covers closed render, the authorize-first choice and the coach's key path, successful link, and error display

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

vi.mock('../../services/api', () => ({
  providersApi: {
    linkIntervalsIcu: vi.fn(),
  },
}));

import IntervalsIcuLinkModal from '../IntervalsIcuLinkModal';
import { providersApi } from '../../services/api';

const mockOnClose = vi.fn();
const mockOnConnected = vi.fn();

function renderModal(isOpen = true) {
  return render(
    <IntervalsIcuLinkModal isOpen={isOpen} onClose={mockOnClose} onConnected={mockOnConnected} />
  );
}

describe('IntervalsIcuLinkModal', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders nothing when closed', () => {
    const { container } = renderModal(false);
    expect(container).toBeEmptyDOMElement();
  });

  it('says where Intervals.icu shows the athlete id and API key, as one catalogue sentence', () => {
    renderModal();
    // The whole instruction is one key the hosted connect form reads too; it
    // names the section Intervals.icu's Settings page actually shows.
    expect(
      screen.getByText(
        'Find your Athlete ID and API key in Intervals.icu: open Settings and scroll down to Developer Settings, near the bottom of the page.'
      )
    ).toBeInTheDocument();
  });

  it('links with trimmed athlete id + api key and fires onConnected', async () => {
    vi.mocked(providersApi.linkIntervalsIcu).mockResolvedValue({
      status: 'connected',
      provider: 'intervals_icu',
      athlete: { id: 'i123456', name: 'Test Athlete' },
    });
    const user = userEvent.setup();
    renderModal();

    await user.type(screen.getByLabelText('Athlete ID'), '  i123456  ');
    await user.type(screen.getByLabelText('API Key'), '  secret-key  ');
    await user.click(screen.getByRole('button', { name: 'Connect' }));

    expect(providersApi.linkIntervalsIcu).toHaveBeenCalledWith({
      athlete_id: 'i123456',
      api_key: 'secret-key',
    });
    // onConnected fires after a short success-confirmation delay (1200ms).
    await waitFor(() => expect(mockOnConnected).toHaveBeenCalled(), { timeout: 2500 });
  });

  it('shows an error when the server rejects the credentials', async () => {
    vi.mocked(providersApi.linkIntervalsIcu).mockRejectedValue({
      response: { status: 400, data: { message: 'Intervals.icu rejected those credentials' } },
    });
    const user = userEvent.setup();
    renderModal();

    await user.type(screen.getByLabelText('Athlete ID'), 'i123456');
    await user.type(screen.getByLabelText('API Key'), 'bad-key');
    await user.click(screen.getByRole('button', { name: 'Connect' }));

    expect(await screen.findByText('Intervals.icu rejected those credentials')).toBeInTheDocument();
    expect(mockOnConnected).not.toHaveBeenCalled();
  });

  it('keeps the submit button disabled until both fields are filled', async () => {
    const user = userEvent.setup();
    renderModal();
    const submit = screen.getByRole('button', { name: 'Connect' });
    expect(submit).toBeDisabled();
    await user.type(screen.getByLabelText('Athlete ID'), 'i123456');
    expect(submit).toBeDisabled();
    await user.type(screen.getByLabelText('API Key'), 'secret-key');
    expect(submit).toBeEnabled();
  });
});

describe('IntervalsIcuLinkModal — the authorization offered', () => {
  const onAuthorize = vi.fn();

  beforeEach(() => {
    vi.clearAllMocks();
  });

  function renderOffering() {
    return render(
      <IntervalsIcuLinkModal
        isOpen
        onClose={mockOnClose}
        onConnected={mockOnConnected}
        onAuthorize={onAuthorize}
      />
    );
  }

  it('leads with the authorization, and starting it links nothing itself', async () => {
    const user = userEvent.setup();
    renderOffering();

    expect(screen.getByTestId('intervals-authorize-choice')).toBeInTheDocument();
    expect(screen.queryByLabelText('Athlete ID')).not.toBeInTheDocument();

    await user.click(screen.getByTestId('intervals-authorize'));

    expect(onAuthorize).toHaveBeenCalledTimes(1);
    expect(providersApi.linkIntervalsIcu).not.toHaveBeenCalled();
  });

  it('keeps the coach\'s API-key form one tap away, and back again', async () => {
    vi.mocked(providersApi.linkIntervalsIcu).mockResolvedValue({
      status: 'connected',
      provider: 'intervals_icu',
      athlete: { id: 'i777', name: 'Coach' },
    });
    const user = userEvent.setup();
    renderOffering();

    await user.click(screen.getByTestId('intervals-use-api-key'));
    expect(screen.queryByTestId('intervals-authorize-choice')).not.toBeInTheDocument();
    await user.type(screen.getByLabelText('Athlete ID'), 'i777');
    await user.type(screen.getByLabelText('API Key'), 'coach-key');
    await user.click(screen.getByRole('button', { name: 'Connect' }));

    expect(providersApi.linkIntervalsIcu).toHaveBeenCalledWith({ athlete_id: 'i777', api_key: 'coach-key' });
    expect(onAuthorize).not.toHaveBeenCalled();
  });

  it('returns from the key form to the authorization', async () => {
    const user = userEvent.setup();
    renderOffering();

    await user.click(screen.getByTestId('intervals-use-api-key'));
    await user.click(screen.getByTestId('intervals-back-to-authorize'));

    expect(screen.getByTestId('intervals-authorize-choice')).toBeInTheDocument();
  });

  it('offers no way back to an authorization it was not given', () => {
    renderModal();

    expect(screen.getByLabelText('Athlete ID')).toBeInTheDocument();
    expect(screen.queryByTestId('intervals-back-to-authorize')).not.toBeInTheDocument();
    expect(screen.queryByTestId('intervals-authorize')).not.toBeInTheDocument();
  });
});
