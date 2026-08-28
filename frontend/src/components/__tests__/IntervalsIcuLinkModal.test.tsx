// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
//
// ABOUTME: Unit tests for IntervalsIcuLinkModal — athlete id + API key entry and link submission
// ABOUTME: Covers closed render, successful link (trimmed payload + onConnected), and error display

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
