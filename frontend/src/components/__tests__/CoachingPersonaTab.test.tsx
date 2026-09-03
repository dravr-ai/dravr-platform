// ABOUTME: Tests for the coaching-persona tab — the cards are the server's, not a hand-written table
// ABOUTME: Asserts rendered contract rules, the enforcement badge, and selection by slug
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { PersonasResponse } from '@pierre/shared-types';
import CoachingPersonaTab from '../CoachingPersonaTab';

vi.mock('../../services/api', () => ({
  personasApi: { list: vi.fn() },
  userApi: { setCoachingPersona: vi.fn() },
}));

vi.mock('../../hooks/useAuth', () => ({
  useAuth: () => ({
    user: {
      id: 'user-1',
      email: 'athlete@example.com',
      tenant_id: 'tenant-a',
      is_admin: false,
      role: 'user',
      coaching_persona: 'casual',
      created_at: '2026-01-01T00:00:00Z',
    },
  }),
}));

const { personasApi, userApi } = await import('../../services/api');

/**
 * What `GET /api/personas` sends: the summary and every rule already rendered
 * from the flattened contract, with the contract's own numbers interpolated.
 */
const CARDS: PersonasResponse = {
  personas: [
    {
      slug: 'casual',
      display_name: 'Casual',
      summary: 'Short answers, no jargon.',
      rules: [
        { key: 'persona.rule.wordCap', text: 'Replies stay under 120 words.' },
        { key: 'persona.rule.noZones', text: 'No training zones unless asked.' },
      ],
      enforcement: 'verified',
      enforcement_label: 'Verified',
    },
    {
      slug: 'power_athlete',
      display_name: 'Power-athlete',
      summary: 'Zones, load and gaps, no hedging.',
      rules: [],
      enforcement: 'advisory',
      enforcement_label: 'Advisory',
    },
  ],
};

function renderTab() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <CoachingPersonaTab />
    </QueryClientProvider>,
  );
}

describe('CoachingPersonaTab', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(personasApi.list).mockResolvedValue(CARDS);
  });

  it('renders one card per persona the server sent, rules included', async () => {
    renderTab();

    expect(await screen.findByTestId('persona-card-casual')).toBeInTheDocument();
    expect(screen.getByTestId('persona-card-power_athlete')).toBeInTheDocument();

    // The word cap is a contract number the client has no way to know, which
    // is the whole reason these sentences come from the server.
    expect(screen.getByText('Replies stay under 120 words.')).toBeInTheDocument();
    expect(screen.getByText('No training zones unless asked.')).toBeInTheDocument();
    expect(screen.getByText('Short answers, no jargon.')).toBeInTheDocument();
  });

  it('says whether a persona contract is enforced or only advised', async () => {
    renderTab();

    const casual = await screen.findByTestId('persona-card-casual');
    expect(casual).toHaveAttribute('data-enforcement', 'verified');
    expect(casual).toHaveTextContent('Verified');

    const power = screen.getByTestId('persona-card-power_athlete');
    expect(power).toHaveAttribute('data-enforcement', 'advisory');
    expect(power).toHaveTextContent('Advisory');
  });

  it('marks the stored persona active and sends the slug on selection', async () => {
    vi.mocked(userApi.setCoachingPersona).mockResolvedValue({
      message: 'ok',
      persona: 'power_athlete',
    });
    renderTab();

    const casual = await screen.findByTestId('persona-card-casual');
    expect(casual).toHaveAttribute('aria-checked', 'true');

    fireEvent.click(screen.getByTestId('persona-card-power_athlete'));

    await waitFor(() => expect(userApi.setCoachingPersona).toHaveBeenCalledWith('power_athlete'));
    // The confirmation names the persona the way the card does.
    expect(await screen.findByTestId('persona-status')).toHaveTextContent('Power-athlete');
  });

  it('shows the cards nothing rather than a hand-written fallback when the read fails', async () => {
    vi.mocked(personasApi.list).mockRejectedValue(new Error('503'));
    renderTab();

    expect(await screen.findByTestId('persona-error')).toBeInTheDocument();
    expect(screen.queryByTestId('persona-card-casual')).not.toBeInTheDocument();
  });
});
