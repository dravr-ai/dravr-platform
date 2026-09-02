// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The typing indicator's geometry and its motion — it hugs its content and the dots breathe
// ABOUTME: A full-width slab of nothing with three frozen dots reads as a broken bubble, not as typing

import React from 'react';
import { Animated, StyleSheet } from 'react-native';
import { act, render } from '@testing-library/react-native';

import { MessageList, typingDotAnimation } from '../src/screens/chat/MessageList';
import type { Message } from '../src/types';

/** Far enough into the first fade that every dot has left its resting value. */
const TIME_INTO_THE_FIRST_FADE_MS = 700;

const message: Message = {
  id: 'msg-1',
  role: 'user',
  content: 'Comment était ma semaine ?',
  created_at: '2026-09-02T10:00:00Z',
};

function renderSending() {
  return render(
    <MessageList
      bottomInset={0}
      messages={[message]}
      isLoading={false}
      isSending
      messageFeedback={{}}
      messageFeedbackComment={{}}
      flatListRef={React.createRef()}
      onScrollToBottom={jest.fn()}
      onThumbsUp={jest.fn()}
      onThumbsDown={jest.fn()}
      onSubmitFeedbackReason={jest.fn()}
      onRetryMessage={jest.fn()}
      onOpenUrl={jest.fn()}
      onReconnectProvider={jest.fn()}
    />,
  );
}

/** The dot's opacity as the surface resolved it for this frame. */
function dotOpacity(element: { props: { style?: unknown } }): number {
  const style = StyleSheet.flatten(element.props.style as never) as { opacity?: number };
  return style.opacity as number;
}

describe('MessageList typing indicator', () => {
  it('hugs its own content instead of stretching across the thread', () => {
    const { getByTestId } = renderSending();

    const indicator = getByTestId('thinking-indicator');
    const style = StyleSheet.flatten(indicator.props.style) as {
      alignSelf?: string;
      maxWidth?: string | number;
    };

    // A plain parent stretches its child across the cross axis; alignSelf is
    // what makes the row size to the mark and the three dots.
    expect(style.alignSelf).toBe('flex-start');
    // And it is not a bubble capped at 85% of the screen any more: nothing
    // caps it, because nothing stretches it.
    expect(style.maxWidth).toBeUndefined();
  });

  it('gives each dot a different opacity, so the three read as a wave', () => {
    jest.useFakeTimers();
    try {
      const { getByTestId } = renderSending();

      act(() => {
        jest.advanceTimersByTime(TIME_INTO_THE_FIRST_FADE_MS);
      });

      const opacities = [0, 1, 2].map((i) => dotOpacity(getByTestId(`typing-dot-${i}`)));
      // The stagger is the point: at any instant mid-cycle the three dots are
      // at three different points of the same fade.
      expect(new Set(opacities).size).toBe(3);
      for (const opacity of opacities) {
        expect(opacity).toBeGreaterThanOrEqual(0);
        expect(opacity).toBeLessThanOrEqual(1);
      }
    } finally {
      jest.useRealTimers();
    }
  });

  it('keeps every dot moving rather than resting on a fixed opacity', () => {
    jest.useFakeTimers();
    try {
      const { getByTestId } = renderSending();

      const start = [0, 1, 2].map((i) => dotOpacity(getByTestId(`typing-dot-${i}`)));
      act(() => {
        jest.advanceTimersByTime(TIME_INTO_THE_FIRST_FADE_MS);
      });
      const later = [0, 1, 2].map((i) => dotOpacity(getByTestId(`typing-dot-${i}`)));

      // Three fixed opacities — what the indicator used to draw — would give
      // the same three numbers here.
      expect(later).not.toEqual(start);
      for (let i = 0; i < 3; i += 1) {
        expect(later[i]).not.toBe(start[i]);
      }
    } finally {
      jest.useRealTimers();
    }
  });
});

/**
 * Every value the animation pushes into `value`, in order.
 *
 * `addListener` is Animated's own readout, and with the JS driver it fires on
 * each frame — so the list is the dot's whole trajectory, not one snapshot.
 */
function recordValues(value: Animated.Value): number[] {
  const seen: number[] = [];
  value.addListener((state) => seen.push(state.value));
  return seen;
}

/** Up, down, and the stagger padding either side — one dot's whole cycle. */
const TYPING_DOT_CYCLE_MS = 2 * 380 + 2 * 180;

describe('typingDotAnimation', () => {
  it('loops, so the dot is still breathing long after the first fade', () => {
    jest.useFakeTimers();
    try {
      const opacity = new Animated.Value(0.25);
      const seen = recordValues(opacity);
      const animation = typingDotAnimation(opacity, 0);
      animation.start();

      act(() => {
        jest.advanceTimersByTime(2 * TYPING_DOT_CYCLE_MS);
      });
      animation.stop();

      // Two cycles' worth of time, so a dot that loops reaches full opacity
      // twice; a single fade — or a fixed opacity — reaches it at most once.
      const peaks = seen.filter((value) => value > 0.999).length;
      expect(peaks).toBeGreaterThanOrEqual(2);
      expect(Math.min(...seen)).toBeLessThan(0.3);
    } finally {
      jest.useRealTimers();
    }
  });

  it('gives every dot the same cycle length, so the wave never drifts apart', () => {
    jest.useFakeTimers();
    try {
      const values = [0, 1, 2].map(() => new Animated.Value(0.25));
      const trajectories = values.map(recordValues);
      const animations = values.map((value, index) => typingDotAnimation(value, index));
      for (const animation of animations) animation.start();

      act(() => {
        jest.advanceTimersByTime(TYPING_DOT_CYCLE_MS);
      });
      for (const animation of animations) animation.stop();

      // One full cycle later every dot is back at its dimmest, whatever its
      // place in the row — the stagger is split before and after the fade, so
      // the period does not grow with the index.
      for (const trajectory of trajectories) {
        expect(trajectory.length).toBeGreaterThan(0);
        expect(trajectory[trajectory.length - 1]).toBeCloseTo(0.25, 2);
      }
    } finally {
      jest.useRealTimers();
    }
  });
});
