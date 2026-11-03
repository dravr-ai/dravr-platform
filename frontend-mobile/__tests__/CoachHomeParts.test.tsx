// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Coach Home's shared pieces in both themes — the phase fills the season bars are told apart by, the switch's raised chip
// ABOUTME: Red if a dark phase fill goes back to the light tint that vanished on the dark canvas, or the chip sinks below its track

import React from 'react';
import { render } from '@testing-library/react-native';
import { phaseFill } from '../src/screens/coach-home/parts';
import { Text } from 'react-native';
import { CoachHomeSwitch } from '../src/screens/coach-home/CoachHomeSwitch';
import { Section } from '../src/components/ui/Section';

const darkClass = (classes: string) => classes.split(/\s+/).find((c) => c.startsWith('dark:')) ?? null;

describe('phase fills', () => {
  it('lifts each told-apart phase on the dark canvas, and keeps base and build distinct there', () => {
    for (const kind of ['prep', 'base', 'build', 'specialty', 'peak', 'taper'] as const) {
      expect(darkClass(phaseFill(kind))).not.toBeNull();
    }
    expect(darkClass(phaseFill('base'))).not.toBe(darkClass(phaseFill('build')));
    expect(darkClass(phaseFill('build'))).not.toBe(darkClass(phaseFill('specialty')));
  });
});

describe('the Mes athlètes / Mon entraînement switch', () => {
  it('raises the selected chip above the track in dark as in light', () => {
    const { getByTestId } = render(<CoachHomeSwitch scope="athletes" onChange={() => {}} />);
    const selected = getByTestId('coach-home-switch-athletes').props.className as string;
    expect(selected).toContain('bg-surface-container-lowest');
    // The lowest tier sits below the dark canvas; the chip takes the tier above the track there.
    expect(selected).toContain('dark:bg-surface-container-highest');
    expect(getByTestId('coach-home-switch').props.className).toContain('bg-surface-container-high');
  });
});

describe('a section with an action', () => {
  it('pulls the 44 target onto the title line instead of hanging 13 below it', () => {
    const { toJSON } = render(
      <Section title="Recent activities" actions={<Text testID="action">Upload .fit</Text>}>
        <Text>row</Text>
      </Section>,
    );
    type Node = { props: Record<string, unknown>; children: Array<Node | string> | null };
    const find = (node: Node | string | null, test: (n: Node) => boolean): Node | null => {
      if (node === null || typeof node === 'string') return null;
      if (test(node)) return node;
      for (const child of node.children ?? []) {
        const hit = find(child, test);
        if (hit) return hit;
      }
      return null;
    };
    const holder = find(toJSON() as Node, (n) => (n.children ?? []).some((c) => typeof c !== 'string' && c.props.testID === 'action'));
    expect(String(holder?.props.className)).toMatch(/-my-3/);
  });
});
