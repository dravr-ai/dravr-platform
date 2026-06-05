// ABOUTME: Unit tests for the mobile IntervalsIcuLinkModal — athlete id + API key link submission
// ABOUTME: Covers trimmed payload + onConnected on success, and error display on rejection

import React from 'react';
import { render, waitFor, fireEvent } from '@testing-library/react-native';
import { IntervalsIcuLinkModal } from '../src/components/IntervalsIcuLinkModal';
import { oauthApi } from '../src/services/api';

jest.mock('../src/services/api', () => ({
  oauthApi: {
    linkIntervalsIcu: jest.fn(),
  },
}));

describe('IntervalsIcuLinkModal', () => {
  beforeEach(() => {
    jest.clearAllMocks();
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
    // Success confirmation renders (onConnected fires after a short delay).
    await waitFor(() => expect(getByText('Connected Test Athlete')).toBeTruthy());
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
