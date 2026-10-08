// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the Home tab's weekly volume — this week's figures, the sport filter, the bars per week, the empty and failed answers
// ABOUTME: Red if a week the server did not send is drawn, a filter leaks another sport's figures, or nothing stored reads as zeros

import React from 'react';
import { fireEvent, render } from '@testing-library/react-native';
import type { TrainingVolumeResponse } from '@pierre/shared-types';
import { HomeVolume } from '../src/screens/home/HomeVolume';
import { VOLUME_RESPONSE } from '../integration/app/helpers/homeFixtures';

function renderVolume(response: TrainingVolumeResponse | null, isError = false, onRetry = jest.fn()) {
  return render(<HomeVolume response={response} isError={isError} onRetry={onRetry} />);
}

/** The chart reports its width, as the first layout does on a device. */
function layOut(screen: ReturnType<typeof renderVolume>) {
  fireEvent(screen.getByTestId('home-volume-trend'), 'layout', {
    nativeEvent: { layout: { width: 240, height: 72 } },
  });
}

describe('HomeVolume', () => {
  it("sums this week's distance, time and climbing over every sport", () => {
    const screen = renderVolume(VOLUME_RESPONSE);

    expect(screen.getByTestId('home-volume')).toHaveTextContent(/Weekly volume/);
    expect(screen.getByTestId('home-volume-distance')).toHaveTextContent('55.0 km');
    // 12 630 s rounds to 3 h 31 min: a week's time is never read in seconds.
    expect(screen.getByTestId('home-volume-time')).toHaveTextContent('3h 31m');
    expect(screen.getByTestId('home-volume-elevation')).toHaveTextContent('530 m');
    expect(screen.getByTestId('home-volume-count')).toHaveTextContent('3 activities');
  });

  it('draws one bar per week the server sent, this week in full ink and read out', () => {
    const screen = renderVolume(VOLUME_RESPONSE);
    layOut(screen);

    const bars = screen.getAllByTestId('home-volume-bar');
    expect(bars).toHaveLength(3);
    // The quiet week is a bar of no height, never left out or padded.
    expect(bars[1].props.height).toBe(0);
    expect(bars[2].props.fillOpacity).toBe(1);
    expect(bars[0].props.fillOpacity).toBeLessThan(1);
    expect(screen.getByTestId('home-volume-trend-label')).toHaveTextContent('Distance per week, last 3 weeks');
    expect(screen.getByTestId('home-volume-readout')).toHaveTextContent('Week of Sep 21: 55.0 km');
    expect(screen.getByTestId('home-volume-trend').props.accessibilityLabel).toBe(
      'Distance per week, last 3 weeks: from 21.0 km in the week of Sep 7 to 55.0 km this week',
    );
  });

  it('reads out the week a tap lands on', () => {
    const screen = renderVolume(VOLUME_RESPONSE);
    layOut(screen);

    fireEvent.press(screen.getByTestId('home-volume-trend'), { nativeEvent: { locationX: 10 } });
    expect(screen.getByTestId('home-volume-readout')).toHaveTextContent('Week of Sep 7: 21.0 km');
  });

  it('filters every figure to one sport, offered most time first', () => {
    const screen = renderVolume(VOLUME_RESPONSE);

    expect(screen.getByTestId('home-volume-filter-all')).toHaveTextContent('All sports');
    expect(screen.getByTestId('home-volume-filter-run')).toBeTruthy();
    fireEvent.press(screen.getByTestId('home-volume-filter-ride'));

    expect(screen.getByTestId('home-volume-distance')).toHaveTextContent('40.0 km');
    expect(screen.getByTestId('home-volume-elevation')).toHaveTextContent('450 m');
    expect(screen.getByTestId('home-volume-count')).toHaveTextContent('1 activity');
  });

  it('draws time when the selection never recorded a distance, with no filter for one sport', () => {
    const lift = { sport_type: 'strength_training', distance_meters: 0, elevation_gain_meters: 0 };
    const screen = renderVolume({
      today: '2026-09-24',
      weeks: [
        { week_start: '2026-09-14', sports: [{ ...lift, activities: 1, duration_seconds: 2_700 }] },
        { week_start: '2026-09-21', sports: [{ ...lift, activities: 2, duration_seconds: 5_400 }] },
      ],
    });

    expect(screen.getByTestId('home-volume-trend-label')).toHaveTextContent('Time per week, last 2 weeks');
    expect(screen.getByTestId('home-volume-readout')).toHaveTextContent('Week of Sep 21: 1h 30m');
    expect(screen.queryByTestId('home-volume-filter')).toBeNull();
  });

  it('says in words that one week is not yet a trend', () => {
    const screen = renderVolume({ today: '2026-09-24', weeks: [VOLUME_RESPONSE.weeks[2]] });

    expect(screen.getByTestId('home-volume-trend-short')).toHaveTextContent(
      'Your weekly trend shows up here as the weeks add up.',
    );
    expect(screen.queryByTestId('home-volume-trend')).toBeNull();
  });

  it('says nothing is stored yet instead of drawing empty weeks', () => {
    const screen = renderVolume({ today: '2026-09-24', weeks: [] });

    expect(screen.getByTestId('home-volume-empty')).toHaveTextContent(/No activities stored yet\./);
    expect(screen.queryByTestId('home-volume-week')).toBeNull();
  });

  it('offers a retry when the volume could not be read, and nothing while it loads', () => {
    const onRetry = jest.fn();
    const failed = renderVolume(null, true, onRetry);
    expect(failed.getByTestId('home-volume-failed')).toHaveTextContent(/Your weekly volume couldn't be loaded\./);
    fireEvent.press(failed.getByTestId('home-volume-retry'));
    expect(onRetry).toHaveBeenCalledTimes(1);
    failed.unmount();

    const loading = renderVolume(null);
    expect(loading.queryByTestId('home-volume-failed')).toBeNull();
    expect(loading.queryByTestId('home-volume-reading')).toBeNull();
  });
});
