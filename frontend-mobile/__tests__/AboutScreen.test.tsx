// ABOUTME: Pins the About pane's four rows — version, the model that answers, help and legal
// ABOUTME: The help and legal rows opened dravr.ai/help and /privacy, both of which answered 404

import React from 'react';
import { render, fireEvent, waitFor } from '@testing-library/react-native';
import { Linking } from 'react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { APP_VERSION, HELP_URL, LEGAL_URL, settingsPaneSections } from '@pierre/shared-constants';


jest.mock('expo-router', () => ({
  useRouter: () => ({ push: jest.fn(), back: jest.fn() }),
}));

jest.mock('@expo/vector-icons', () => ({
  Feather: () => null,
}));

jest.mock('../src/contexts/AuthContext', () => ({
  useAuth: () => ({
    user: { id: 'user-1', email: 'test@pierre.dev', role: 'user' },
    isAuthenticated: true,
  }),
}));

const mockGetLlmSettings = jest.fn();
jest.mock('../src/services/api', () => ({
  userApi: {
    getLlmSettings: () => mockGetLlmSettings(),
  },
}));

import { AboutScreen } from '../src/screens/settings/AboutScreen';

function renderAbout() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <AboutScreen />
    </QueryClientProvider>,
  );
}

describe('AboutScreen', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockGetLlmSettings.mockResolvedValue({
      providers: [],
      user_credentials: [],
      system_provider: {
        name: 'copilot_headless',
        display_name: 'Copilot Headless',
        model: 'claude-sonnet-5',
      },
    });
  });

  it('renders the four rows the shared declaration holds, in order', async () => {
    const { getByTestId } = renderAbout();
    await waitFor(() => {
      expect(getByTestId('about-screen')).toBeTruthy();
    });
    expect([...settingsPaneSections('about')]).toEqual(['version', 'coach-model', 'help', 'legal']);
    for (const section of settingsPaneSections('about')) {
      expect(getByTestId(`about-section-${section}`)).toBeTruthy();
    }
  });

  it('states which model answers the athlete, read-only', async () => {
    // The AI-provider pane was the only place this was visible, and it invited
    // an API key alongside. The fact survives; the key field does not.
    const { getByTestId, queryByTestId } = renderAbout();
    await waitFor(() => {
      expect(getByTestId('about-coach-model-value').props.children).toBe(
        'Copilot Headless · claude-sonnet-5',
      );
    });
    expect(queryByTestId('llm-api-key-input')).toBeNull();
  });

  it('says so plainly when the server names no model', async () => {
    mockGetLlmSettings.mockResolvedValue({ providers: [], user_credentials: [] });
    const { getByTestId } = renderAbout();
    await waitFor(() => {
      expect(getByTestId('about-coach-model-value').props.children).toBe('Not available');
    });
  });

  it('reports the shared release string rather than a second copy of it', async () => {
    const { getByTestId } = renderAbout();
    await waitFor(() => {
      expect(getByTestId('about-section-version')).toBeTruthy();
    });
    expect(APP_VERSION).toBe('1.0.0');
  });

  it('opens help and legal at a page that answers', async () => {
    const openSpy = jest.spyOn(Linking, 'openURL').mockResolvedValue(true);
    const { getByTestId } = renderAbout();

    fireEvent.press(getByTestId('about-section-help'));
    await waitFor(() => {
      expect(openSpy).toHaveBeenCalledWith(HELP_URL);
    });

    fireEvent.press(getByTestId('about-section-legal'));
    await waitFor(() => {
      expect(openSpy).toHaveBeenCalledWith(LEGAL_URL);
    });

    // dravr.ai/help, /privacy and /terms are each a 404; /docs is the page
    // that answers. Asserting the constants rather than the calls alone keeps
    // a repoint from silently going back.
    for (const url of [HELP_URL, LEGAL_URL]) {
      expect(url).toBe('https://dravr.ai/docs');
    }
    openSpy.mockRestore();
  });
});
