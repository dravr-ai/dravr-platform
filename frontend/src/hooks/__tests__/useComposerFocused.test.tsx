// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the composer-focus signal a phone folds its tab bar on — true only while a chat composer holds focus
// ABOUTME: Moving from one composer to another stays true; focus anywhere else is not typing in the chat

import { describe, it, expect } from 'vitest';
import { render, screen, act } from '@testing-library/react';
import { useComposerFocused } from '../useComposerFocused';

function Probe() {
  const composing = useComposerFocused();
  return (
    <div>
      {/* The hook reads the marker, not the element: MessageInput puts it on its field. */}
      <div role="textbox" tabIndex={0} aria-label="first composer" data-composer="true" />
      <div role="textbox" tabIndex={0} aria-label="second composer" data-composer="true" />
      <button type="button">elsewhere</button>
      <output data-testid="state">{String(composing)}</output>
    </div>
  );
}

describe('useComposerFocused', () => {
  it('is true only while a chat composer holds focus', () => {
    render(<Probe />);
    expect(screen.getByTestId('state')).toHaveTextContent('false');

    act(() => screen.getByLabelText('first composer').focus());
    expect(screen.getByTestId('state')).toHaveTextContent('true');

    act(() => screen.getByLabelText('second composer').focus());
    expect(screen.getByTestId('state')).toHaveTextContent('true');

    act(() => screen.getByRole('button', { name: 'elsewhere' }).focus());
    expect(screen.getByTestId('state')).toHaveTextContent('false');

    act(() => screen.getByLabelText('first composer').focus());
    act(() => screen.getByLabelText('first composer').blur());
    expect(screen.getByTestId('state')).toHaveTextContent('false');
  });
});
