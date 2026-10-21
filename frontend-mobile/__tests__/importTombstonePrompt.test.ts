// ABOUTME: Unit tests for the phone's provider-connect tombstone prompt — wording, clear, reconnect
// ABOUTME: The clear runs only on the athlete's tap, and the connect starts again only once the clear held

import { Alert } from 'react-native';
import { waitFor } from '@testing-library/react-native';
import { i18n } from '@pierre/i18n';
import { promptImportTombstoneClear } from '../src/utils/importTombstone';

const mockClearImportTombstone = jest.fn();
jest.mock('../src/services/api', () => ({
  userApi: {
    clearImportTombstone: (...args: unknown[]) => mockClearImportTombstone(...args),
  },
}));

const t = (key: string, params?: Record<string, string | number>) => i18n.t(key, params) as string;
const refusal = { tombstoneId: 'tomb-1', deletedAt: '2026-10-20T08:00:00+00:00' };

type AlertButtons = Parameters<typeof Alert.alert>[2];

describe('provider-connect tombstone prompt', () => {
  let alert: jest.SpyInstance;

  beforeEach(() => {
    jest.clearAllMocks();
    alert = jest.spyOn(Alert, 'alert').mockImplementation(() => undefined);
  });

  afterEach(() => alert.mockRestore());

  it('words the refusal with its date and clears, then connects again, only on the tap', async () => {
    mockClearImportTombstone.mockResolvedValue({ cleared: [], user_status: 'active' });
    const onCleared = jest.fn();

    promptImportTombstoneClear({ refusal, providerName: 'Strava', language: 'en', t, onCleared });

    expect(alert).toHaveBeenCalledTimes(1);
    const [title, message, buttons] = alert.mock.calls[0] as [string, string, AlertButtons];
    expect(title).toBe('Strava');
    expect(message).toBe(
      'This Strava account belonged to a Dravr account deleted on Oct 20, 2026. Its data stays deleted until you choose to start fresh with it.'
    );
    expect(mockClearImportTombstone).not.toHaveBeenCalled();

    const clear = buttons?.find((b) => b.text === 'Start fresh and connect');
    clear?.onPress?.();

    await waitFor(() => expect(onCleared).toHaveBeenCalledTimes(1));
    expect(mockClearImportTombstone).toHaveBeenCalledWith('tomb-1');
  });

  it('never connects again when the clear is refused', async () => {
    mockClearImportTombstone.mockRejectedValue({ response: { status: 404, data: { message: 'No tombstone' } } });
    const onCleared = jest.fn();

    promptImportTombstoneClear({ refusal, providerName: 'Strava', language: 'en', t, onCleared });
    const buttons = (alert.mock.calls[0] as [string, string, AlertButtons])[2];
    buttons?.find((b) => b.text === 'Start fresh and connect')?.onPress?.();

    // The refusal is said in a second alert, and nothing connects.
    await waitFor(() => expect(alert).toHaveBeenCalledTimes(2));
    expect(onCleared).not.toHaveBeenCalled();
  });
});
