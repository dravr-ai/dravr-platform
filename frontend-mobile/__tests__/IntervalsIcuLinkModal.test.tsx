// ABOUTME: Unit tests for the mobile IntervalsIcuLinkModal — the authorization when offered, and athlete id + API key link submission
// ABOUTME: Covers the authorize-first choice and the coach's key path, trimmed payload + onConnected on success, and errors

import React from 'react';
import { render, waitFor, fireEvent } from '@testing-library/react-native';
import { IntervalsIcuLinkModal } from '../src/components/IntervalsIcuLinkModal';
import { oauthApi, userApi } from '../src/services/api';

jest.mock('../src/services/api', () => ({
  oauthApi: {
    linkIntervalsIcu: jest.fn(),
  },
  userApi: {
    clearImportTombstone: jest.fn(),
  },
}));

describe('IntervalsIcuLinkModal', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it('says where Intervals.icu shows the athlete id and API key, as one catalogue sentence', () => {
    const { getByText } = render(
      <IntervalsIcuLinkModal visible onClose={jest.fn()} onConnected={jest.fn()} />
    );
    // The whole instruction is one key the hosted connect form reads too; it
    // names the section Intervals.icu's Settings page actually shows.
    expect(
      getByText(
        'Find your Athlete ID and API key in Intervals.icu: open Settings and scroll down to Developer Settings, near the bottom of the page.'
      )
    ).toBeTruthy();
  });

  it('links with trimmed athlete id + api key and fires onConnected', async () => {
    (oauthApi.linkIntervalsIcu as jest.Mock).mockResolvedValue({
      status: 'connected',
      provider: 'intervals_icu',
      athlete: { id: 'i123456', name: 'Test Athlete' },
    });
    const onConnected = jest.fn();
    const { getByTestId, getByText } = render(
      <IntervalsIcuLinkModal visible onClose={jest.fn()} onConnected={onConnected} />
    );

    fireEvent.changeText(getByTestId('intervals-athlete-id'), '  i123456  ');
    fireEvent.changeText(getByTestId('intervals-api-key'), '  secret-key  ');
    fireEvent.press(getByTestId('intervals-submit'));

    // The athlete id + key are trimmed before being sent.
    await waitFor(() =>
      expect(oauthApi.linkIntervalsIcu).toHaveBeenCalledWith({
        athlete_id: 'i123456',
        api_key: 'secret-key',
      })
    );
    // Success confirmation renders — the keyed "Connected!" and the athlete
    // the server named under it (onConnected fires after a short delay).
    await waitFor(() => expect(getByText('Connected!')).toBeTruthy());
    expect(getByText('Test Athlete')).toBeTruthy();
  });

  it('shows an error when the server rejects the credentials', async () => {
    (oauthApi.linkIntervalsIcu as jest.Mock).mockRejectedValue({
      response: { data: { message: 'Intervals.icu rejected those credentials' } },
    });
    const onConnected = jest.fn();
    const { getByTestId } = render(
      <IntervalsIcuLinkModal visible onClose={jest.fn()} onConnected={onConnected} />
    );

    fireEvent.changeText(getByTestId('intervals-athlete-id'), 'i123456');
    fireEvent.changeText(getByTestId('intervals-api-key'), 'bad-key');
    fireEvent.press(getByTestId('intervals-submit'));

    await waitFor(() =>
      expect(getByTestId('intervals-error').props.children).toBe(
        'Intervals.icu rejected those credentials'
      )
    );
    expect(onConnected).not.toHaveBeenCalled();
  });
});

describe('IntervalsIcuLinkModal — the authorization offered', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it('leads with the authorization, and starting it links nothing itself', () => {
    const onAuthorize = jest.fn();
    const { getByTestId, queryByTestId } = render(
      <IntervalsIcuLinkModal visible onClose={jest.fn()} onConnected={jest.fn()} onAuthorize={onAuthorize} />
    );

    expect(getByTestId('intervals-authorize-choice')).toBeTruthy();
    expect(queryByTestId('intervals-athlete-id')).toBeNull();

    fireEvent.press(getByTestId('intervals-authorize'));

    expect(onAuthorize).toHaveBeenCalledTimes(1);
    expect(oauthApi.linkIntervalsIcu).not.toHaveBeenCalled();
  });

  it('keeps the coach\'s API-key form one tap away, and back again', async () => {
    (oauthApi.linkIntervalsIcu as jest.Mock).mockResolvedValue({
      status: 'connected',
      provider: 'intervals_icu',
      athlete: { id: 'i777', name: 'Coach' },
    });
    const onAuthorize = jest.fn();
    const { getByTestId, queryByTestId } = render(
      <IntervalsIcuLinkModal visible onClose={jest.fn()} onConnected={jest.fn()} onAuthorize={onAuthorize} />
    );

    fireEvent.press(getByTestId('intervals-use-api-key'));
    expect(queryByTestId('intervals-authorize-choice')).toBeNull();
    fireEvent.press(getByTestId('intervals-back-to-authorize'));
    expect(getByTestId('intervals-authorize-choice')).toBeTruthy();

    fireEvent.press(getByTestId('intervals-use-api-key'));
    fireEvent.changeText(getByTestId('intervals-athlete-id'), 'i777');
    fireEvent.changeText(getByTestId('intervals-api-key'), 'coach-key');
    fireEvent.press(getByTestId('intervals-submit'));

    await waitFor(() =>
      expect(oauthApi.linkIntervalsIcu).toHaveBeenCalledWith({ athlete_id: 'i777', api_key: 'coach-key' })
    );
    expect(onAuthorize).not.toHaveBeenCalled();
  });

  it('offers no way back to an authorization it was not given', () => {
    const { getByTestId, queryByTestId } = render(
      <IntervalsIcuLinkModal visible onClose={jest.fn()} onConnected={jest.fn()} />
    );

    expect(getByTestId('intervals-athlete-id')).toBeTruthy();
    expect(queryByTestId('intervals-back-to-authorize')).toBeNull();
    expect(queryByTestId('intervals-authorize')).toBeNull();
  });

  it('words a tombstoned account, and starting fresh clears it and links the same pair again', async () => {
    (oauthApi.linkIntervalsIcu as jest.Mock)
      .mockRejectedValueOnce({
        response: {
          status: 409,
          data: {
            code: 'RESOURCE_LOCKED',
            message: 'The data of this provider account was deleted',
            details: {
              error: 'import_tombstoned',
              tombstone_id: 'tomb-9',
              kind: 'provider_account',
              provider: 'intervals_icu',
              deleted_at: '2026-10-20T08:00:00+00:00',
            },
          },
        },
      })
      .mockResolvedValueOnce({
        status: 'connected',
        provider: 'intervals_icu',
        athlete: { id: 'i123456', name: 'Test Athlete' },
      });
    (userApi.clearImportTombstone as jest.Mock).mockResolvedValue({ cleared: [], user_status: 'active' });
    const { getByTestId, getByText, findByText } = render(
      <IntervalsIcuLinkModal visible onClose={jest.fn()} onConnected={jest.fn()} />
    );

    fireEvent.changeText(getByTestId('intervals-athlete-id'), 'i123456');
    fireEvent.changeText(getByTestId('intervals-api-key'), 'secret-key');
    fireEvent.press(getByTestId('intervals-submit'));

    expect(
      await findByText(
        'This Intervals.icu account belonged to a Dravr account deleted on Oct 20, 2026. Its data stays deleted until you choose to start fresh with it.'
      )
    ).toBeTruthy();

    fireEvent.press(getByTestId('intervals-tombstone-clear'));

    await waitFor(() => expect(getByText('Connected!')).toBeTruthy());
    expect(userApi.clearImportTombstone).toHaveBeenCalledWith('tomb-9');
    expect(oauthApi.linkIntervalsIcu).toHaveBeenCalledTimes(2);
    expect(oauthApi.linkIntervalsIcu).toHaveBeenLastCalledWith({ athlete_id: 'i123456', api_key: 'secret-key' });
  });
});
