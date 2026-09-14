// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the provider dialog — one row per provider, a brand glyph for a known id, initials for an unknown one
// ABOUTME: The connected word rides on a connected row, the cancel control closes, and the dialog floats on a 12 radius

import React from 'react';
import { TouchableOpacity } from 'react-native';
import { fireEvent, render, within } from '@testing-library/react-native';
import Svg from 'react-native-svg';
import type { ExtendedProviderStatus } from '@pierre/shared-types';

import { ProviderModal } from '../src/screens/chat/ProviderModal';

function provider(overrides: Partial<ExtendedProviderStatus> & { provider: string }): ExtendedProviderStatus {
  return {
    display_name: overrides.provider,
    requires_oauth: true,
    connected: false,
    needs_reauth: false,
    capabilities: [],
    ...overrides,
  } as ExtendedProviderStatus;
}

const PROVIDERS = [
  provider({ provider: 'sciotte', display_name: 'Strava', requires_oauth: false, connected: true }),
  provider({ provider: 'sciotte_garmin', display_name: 'Garmin', requires_oauth: false }),
  provider({ provider: 'whoop', display_name: 'Whoop' }),
  provider({ provider: 'intervals_icu', display_name: 'intervals.icu', requires_oauth: false }),
  provider({ provider: 'fitbit', display_name: 'Fitbit' }),
];

function renderModal(overrides: Partial<React.ComponentProps<typeof ProviderModal>> = {}) {
  const handlers = {
    onClose: jest.fn(),
    onSelectConnected: jest.fn(),
    onConnectProvider: jest.fn(),
    onConnectSciotte: jest.fn(),
    onConnectIntervals: jest.fn(),
  };
  const view = render(
    <ProviderModal visible providers={PROVIDERS} connectingProvider={null} {...handlers} {...overrides} />,
  );
  return { ...view, handlers };
}

describe('ProviderModal', () => {
  it('draws one row per provider, each carrying its id', () => {
    const { getByTestId } = renderModal();
    for (const { provider: id } of PROVIDERS) {
      expect(getByTestId(`provider-row-${id}`)).toBeTruthy();
    }
    expect(getByTestId('provider-modal')).toBeTruthy();
  });

  it.each(['sciotte', 'sciotte_garmin', 'whoop', 'intervals_icu'])(
    '%s renders its brand glyph as an Svg at 24',
    (id) => {
      const { getByTestId } = renderModal();
      const svg = within(getByTestId(`provider-row-${id}`)).UNSAFE_getByType(Svg);
      expect(svg.props.width).toBe(24);
      expect(svg.props.height).toBe(24);
    },
  );

  it('an unknown id renders the provider initials instead of a glyph', () => {
    const { getByTestId } = renderModal();
    const row = within(getByTestId('provider-row-fitbit'));
    // The initials circle hides itself from the accessibility tree (the row's
    // name is what VoiceOver reads), so the query has to look past that.
    expect(row.queryAllByText('F', { includeHiddenElements: true })).toHaveLength(1);
    expect(row.UNSAFE_queryAllByType(Svg)).toHaveLength(0);
  });

  it('says the connected word on a connected row only', () => {
    const { getByTestId } = renderModal();
    expect(within(getByTestId('provider-row-sciotte')).getByText('Connected ✓')).toBeTruthy();
    expect(within(getByTestId('provider-row-whoop')).queryByText('Connected ✓')).toBeNull();
  });

  it('keeps the spinner in the glyph slot while a provider connects', () => {
    const { getByTestId } = renderModal({ connectingProvider: 'whoop' });
    const row = within(getByTestId('provider-row-whoop'));
    expect(row.UNSAFE_queryAllByType(Svg)).toHaveLength(0);
    expect(row.getByText('Connecting Whoop…')).toBeTruthy();
  });

  it('routes a press to the flow the provider needs', () => {
    const { getByTestId, handlers } = renderModal();
    fireEvent.press(getByTestId('provider-row-sciotte'));
    fireEvent.press(getByTestId('provider-row-sciotte_garmin'));
    fireEvent.press(getByTestId('provider-row-intervals_icu'));
    fireEvent.press(getByTestId('provider-row-whoop'));
    expect(handlers.onSelectConnected).toHaveBeenCalledWith('sciotte');
    expect(handlers.onConnectSciotte).toHaveBeenCalledWith('garmin');
    expect(handlers.onConnectIntervals).toHaveBeenCalledTimes(1);
    expect(handlers.onConnectProvider).toHaveBeenCalledWith('whoop');
  });

  it('cancel closes the dialog', () => {
    const { getByTestId, handlers } = renderModal();
    fireEvent.press(getByTestId('provider-modal-cancel'));
    expect(handlers.onClose).toHaveBeenCalledTimes(1);
  });

  it('floats as a 12-radius panel on the secondary ground, rows on a faint hairline', () => {
    const { getByTestId, UNSAFE_getAllByType } = renderModal();
    const panel = getByTestId('provider-modal');
    expect(panel.props.className).toContain('rounded-xl');
    expect(panel.props.className).toContain('shadow-floating');
    expect(panel.props.className).not.toContain('border-primary');
    // The row touchables carry their className on the element, not the host
    // view, so the rows are read off the TouchableOpacity elements by id.
    const rowElements = UNSAFE_getAllByType(TouchableOpacity).filter((el) =>
      String(el.props.testID ?? '').startsWith('provider-row-'),
    );
    expect(rowElements).toHaveLength(PROVIDERS.length);
    for (const el of rowElements.slice(0, -1)) {
      expect(el.props.className).toContain('border-b border-border-faint');
      expect(el.props.className).not.toContain('border-primary');
    }
    expect(rowElements[rowElements.length - 1].props.className).not.toContain('border-b');
  });
});
