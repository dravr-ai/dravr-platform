// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests ui/Switch — its aria, keyboard and disabled behaviour, and its track and thumb at 3:1 in both states and schemes
// ABOUTME: Measures SWITCH_TONE through the shared contrast kit: the state's edge against every ground, the thumb against its track

import React, { useState } from 'react';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { Switch, SWITCH_TONE } from '../Switch';
import { AA_GRAPHIC, contrast, GROUNDS, inkOnFill, SCHEMES, token, utilityFill } from '../../../test/contrast';

function Controlled({ initial = false, disabled = false }: { initial?: boolean; disabled?: boolean }) {
  const [on, setOn] = useState(initial);
  return <Switch checked={on} onChange={setOn} label="Auto-approve" disabled={disabled} data-testid="sw" />;
}

/** The token a `bg-*` or `border-*` class in a tone string names. */
function tokenOf(classes: string, prefix: 'bg' | 'border'): string {
  const match = classes.split(/\s+/).find((c) => c.startsWith(`${prefix}-`));
  if (!match) throw new Error(`no ${prefix}- class in ${classes}`);
  return match.slice(prefix.length + 1);
}

describe('Switch', () => {
  it('is a named switch whose aria-checked follows its state', async () => {
    const user = userEvent.setup();
    render(<Controlled />);
    const sw = screen.getByRole('switch', { name: 'Auto-approve' });
    expect(sw).toHaveAttribute('aria-checked', 'false');
    await user.click(sw);
    expect(sw).toHaveAttribute('aria-checked', 'true');
  });

  it('toggles with Space and with Enter', async () => {
    const user = userEvent.setup();
    render(<Controlled />);
    const sw = screen.getByRole('switch', { name: 'Auto-approve' });
    sw.focus();
    await user.keyboard(' ');
    expect(sw).toHaveAttribute('aria-checked', 'true');
    await user.keyboard('{Enter}');
    expect(sw).toHaveAttribute('aria-checked', 'false');
  });

  it('takes its name from a visible label through aria-labelledby', () => {
    render(
      <>
        <span id="lbl">Usage analytics</span>
        <Switch checked onChange={() => {}} aria-labelledby="lbl" />
      </>,
    );
    expect(screen.getByRole('switch', { name: 'Usage analytics' })).toHaveAttribute('aria-checked', 'true');
  });

  it('does nothing while disabled, and says so', async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(<Switch checked={false} onChange={onChange} label="Locked" disabled title="Set by the environment" />);
    const sw = screen.getByRole('switch', { name: 'Locked' });
    expect(sw).toBeDisabled();
    expect(sw).toHaveAttribute('title', 'Set by the environment');
    await user.click(sw);
    expect(onChange).not.toHaveBeenCalled();
  });

  it('draws each state from SWITCH_TONE and carries the focus ring', () => {
    const { rerender } = render(<Switch checked onChange={() => {}} label="s" />);
    const sw = screen.getByRole('switch');
    expect(sw).toHaveClass('focus-ring-visible', ...SWITCH_TONE.on.track.split(' '));
    rerender(<Switch checked={false} onChange={() => {}} label="s" />);
    expect(sw).toHaveClass(...SWITCH_TONE.off.track.split(' '));
  });
});

describe('SWITCH_TONE contrast', () => {
  it.each(SCHEMES)('each state edge clears 3:1 against every ground, and each thumb against its track, in %s', (scheme) => {
    const failures: string[] = [];
    for (const state of ['on', 'off'] as const) {
      const { track, thumb } = SWITCH_TONE[state];
      const edge = tokenOf(track, 'border');
      for (const ground of GROUNDS) {
        const ratio = inkOnFill(scheme, edge, { token: ground, alpha: 0 }, ground);
        if (ratio < AA_GRAPHIC) failures.push(`${state} edge over ${ground}: ${ratio.toFixed(2)}:1`);
      }
      const thumbOnTrack = inkOnFill(scheme, tokenOf(thumb, 'bg'), utilityFill(`bg-${tokenOf(track, 'bg')}`), 'surface');
      if (thumbOnTrack < AA_GRAPHIC) failures.push(`${state} thumb on track: ${thumbOnTrack.toFixed(2)}:1`);
    }
    expect(failures).toEqual([]);
  });

  it('needs the edge and the track ink: without them each state falls under 3:1', () => {
    // Off: a `surface-container-high` track with no edge on the canvas.
    expect(inkOnFill('light', 'surface-container-high', { token: 'surface', alpha: 0 }, 'surface')).toBeLessThan(AA_GRAPHIC);
    // On, dark: a white thumb on the dark scheme's light `primary`.
    expect(contrast([255, 255, 255], token('dark', 'primary'))).toBeLessThan(AA_GRAPHIC);
  });
});
