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

  it('draws no back button of its own — the native header carries the chevron', () => {
    const { queryByTestId } = renderScreen();
    expect(queryByTestId('back-button')).toBeNull();
    expect(mockBack).not.toHaveBeenCalled();
  });

  // Turns red if the cards come back: each persona is a radio row, the rules
  // and the enforcement word sit under the selected row only, and nothing in
  // the tree is a pill, a ring or an "Active" label (Boreal v2.2, DESIGN.md §10).
  it('renders the personas as rows, the contract under the selected one, with no card or pill', async () => {
    const { findByTestId, getByTestId, queryByTestId, queryByText, toJSON } = renderScreen();

    const selected = await findByTestId('persona-card-casual');
    expect(selected.props.accessibilityRole).toBe('radio');
    expect(getByTestId('persona-card-casual-inner').props.className).toContain('min-h-[52px]');

    expect(getByTestId('persona-details-casual')).toBeTruthy();
    expect(queryByTestId('persona-details-enthusiast')).toBeNull();
    expect(queryByTestId('persona-details-power_athlete')).toBeNull();
    expect(queryByTestId('persona-details-coach')).toBeNull();
    expect(queryByTestId('persona-enforcement-advisory')).toBeNull();

    const enforcement = getByTestId('persona-enforcement-verified');
    expect(enforcement.props.className).toContain('text-sm');
    expect(enforcement.props.className).toContain('text-success');
    expect(enforcement.props.className).not.toContain('rounded');
    expect(queryByText('Active')).toBeNull();

    const serialised = JSON.stringify(toJSON());
    expect(serialised).not.toContain('rounded-full');
    expect(serialised).not.toContain('bg-success/15');
    const styles = allStyles(rootOf(toJSON));
    expect(styles.some((s) => typeof s.borderRadius === 'number' && s.borderRadius >= 999)).toBe(false);
    expect(styles.some((s) => s.borderRadius === 16)).toBe(false);
  });
});

type Json = { type: string; props: Record<string, unknown>; children: Array<Json | string> | null };

/** Every style object in a rendered tree, flat or nested arrays alike. */
function allStyles(node: Json | string | null | undefined, out: Array<Record<string, unknown>> = []) {
  if (!node || typeof node === 'string') return out;
  const flatten = (style: unknown): void => {
    if (Array.isArray(style)) style.forEach(flatten);
    else if (style && typeof style === 'object') out.push(style as Record<string, unknown>);
  };
  flatten(node.props.style);
  for (const child of node.children ?? []) allStyles(child, out);
  return out;
}

/** The rendered tree as one node, whichever shape `toJSON()` returned. */
function rootOf(tree: ReturnType<typeof render>['toJSON']): Json {
  const rendered = tree() as Json | Json[] | null;
  if (!rendered) throw new Error('nothing rendered');
  return Array.isArray(rendered) ? rendered[0] : rendered;
}
