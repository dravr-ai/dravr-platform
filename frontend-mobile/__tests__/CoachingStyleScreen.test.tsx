// ABOUTME: Unit tests for CoachingStyleScreen — the cards come from the server, selection is optimistic
// ABOUTME: Asserts the rendered card is the server's content, not a hand-written option table

import React from 'react';
import { render, fireEvent, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { CoachingPersona, PersonasResponse, User } from '@pierre/shared-types';

const mockBack = jest.fn();
jest.mock('expo-router', () => ({
  useRouter: () => ({ back: mockBack, push: jest.fn() }),
}));

const mockSetCoachingPersona = jest.fn();
const mockListPersonas = jest.fn();
jest.mock('../src/services/api', () => ({
  userApi: {
    setCoachingPersona: (persona: CoachingPersona) => mockSetCoachingPersona(persona),
  },
  personasApi: {
    list: (...args: unknown[]) => mockListPersonas(...args),
  },
}));

const mockUpdateUser = jest.fn();
const mockUseAuth = jest.fn();
jest.mock('../src/contexts/AuthContext', () => ({
  useAuth: () => mockUseAuth(),
}));

import { CoachingStyleScreen } from '../src/screens/settings/CoachingStyleScreen';

const baseUser: Partial<User> = {
  id: 'user-1',
  email: 'mobile@example.com',
  is_admin: false,
  role: 'user',
  user_status: 'active',
  coaching_persona: 'casual',
};

/**
 * What `GET /api/personas` sends, in French: the summary and every rule are
 * already rendered from the flattened contract, word caps interpolated.
 */
const CARDS: PersonasResponse = {
  personas: [
    {
      slug: 'casual',
      display_name: 'Casual',
      summary: 'Des réponses courtes, sans jargon.',
      rules: [{ key: 'persona.rule.wordCap', text: 'Réponses de 120 mots au maximum.' }],
      enforcement: 'verified',
      enforcement_label: 'Vérifié',
    },
    {
      slug: 'enthusiast',
      display_name: 'Enthusiast',
      summary: 'Les chiffres qui comptent, expliqués.',
      rules: [],
      enforcement: 'advisory',
      enforcement_label: 'Indicatif',
    },
    {
      slug: 'power_athlete',
      display_name: 'Power-athlete',
      summary: 'Zones, charge et écarts, sans détour.',
      rules: [],
      enforcement: 'advisory',
      enforcement_label: 'Indicatif',
    },
    {
      slug: 'coach',
      display_name: 'Coach',
      summary: 'Le raisonnement complet derrière chaque séance.',
      rules: [],
      enforcement: 'advisory',
      enforcement_label: 'Indicatif',
    },
  ],
};

function renderScreen() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <CoachingStyleScreen />
    </QueryClientProvider>,
  );
}

describe('CoachingStyleScreen', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockListPersonas.mockResolvedValue(CARDS);
    mockUseAuth.mockReturnValue({
      user: baseUser as User,
      updateUser: mockUpdateUser,
    });
  });

  it('renders the cards the server sent, with their rules and enforcement badge', async () => {
    const { findByTestId, getByTestId, getByText, queryByText } = renderScreen();

    expect(await findByTestId('persona-card-casual')).toBeTruthy();
    expect(getByTestId('persona-card-enthusiast')).toBeTruthy();
    expect(getByTestId('persona-card-power_athlete')).toBeTruthy();
    expect(getByTestId('persona-card-coach')).toBeTruthy();

    // The summary and the rule are the server's sentences — the word cap in
    // particular is a contract number the client has no way to know.
    expect(getByText('Des réponses courtes, sans jargon.')).toBeTruthy();
    expect(getByText('Réponses de 120 mots au maximum.')).toBeTruthy();
    expect(getByTestId('persona-enforcement-verified')).toBeTruthy();
    expect(getByText('Vérifié')).toBeTruthy();

    // No corpus key leaked through as a literal.
    expect(queryByText(/app\.style/)).toBeNull();
  });

  it('asks for the cards in the language the app is rendering', async () => {
    const { findByTestId } = renderScreen();
    await findByTestId('persona-card-casual');

    expect(mockListPersonas).toHaveBeenCalledWith(expect.any(String));
  });

  it('marks the user current persona as active on mount', async () => {
    mockUseAuth.mockReturnValue({
      user: { ...baseUser, coaching_persona: 'power_athlete' } as User,
      updateUser: mockUpdateUser,
    });
    const { findByTestId, getByTestId } = renderScreen();

    expect((await findByTestId('persona-card-power_athlete')).props.accessibilityState.selected).toBe(true);
    expect(getByTestId('persona-card-casual').props.accessibilityState.selected).toBe(false);
  });

  it('calls userApi.setCoachingPersona and updateUser on selection', async () => {
    mockSetCoachingPersona.mockResolvedValueOnce({ message: 'ok', persona: 'enthusiast' });
    const { findByTestId } = renderScreen();

    fireEvent.press(await findByTestId('persona-card-enthusiast'));

    await waitFor(() => {
      expect(mockSetCoachingPersona).toHaveBeenCalledWith('enthusiast');
    });
    expect(mockUpdateUser).toHaveBeenCalledWith({ coaching_persona: 'enthusiast' });
  });

  it('confirms the change by the persona brand name, not its stored slug', async () => {
    mockSetCoachingPersona.mockResolvedValueOnce({ message: 'ok', persona: 'power_athlete' });
    const { findByTestId, getByTestId } = renderScreen();

    fireEvent.press(await findByTestId('persona-card-power_athlete'));

    await waitFor(() => expect(getByTestId('persona-status')).toBeTruthy());
    // The line used to interpolate `power_athlete` here while web said
    // "Power-athlete" for the same change.
    expect(getByTestId('persona-status')).toHaveTextContent(/Power-athlete/);
    expect(getByTestId('persona-status')).not.toHaveTextContent(/power_athlete/);
  });

  it('rolls back the selected card on mutation error', async () => {
    mockSetCoachingPersona.mockRejectedValueOnce(new Error('500 internal'));
    const { findByTestId, getByTestId } = renderScreen();

    fireEvent.press(await findByTestId('persona-card-coach'));

    await waitFor(() => {
      expect(getByTestId('persona-status')).toBeTruthy();
    });
    // Selection rolls back to the original persona.
    expect(getByTestId('persona-card-casual').props.accessibilityState.selected).toBe(true);
    expect(getByTestId('persona-card-coach').props.accessibilityState.selected).toBe(false);
    expect(mockUpdateUser).not.toHaveBeenCalled();
  });

  it('does not refire the mutation when tapping the already-selected card', async () => {
    const { findByTestId } = renderScreen();
    fireEvent.press(await findByTestId('persona-card-casual'));
    expect(mockSetCoachingPersona).not.toHaveBeenCalled();
  });

  it('navigates back when the back button is pressed', () => {
    const { getByTestId } = renderScreen();
    fireEvent.press(getByTestId('back-button'));
    expect(mockBack).toHaveBeenCalled();
  });
});
