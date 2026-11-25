// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests ui/Tabs — the ARIA tabs keyboard pattern: one tab in the tab order, arrows, Home and End moving and selecting
// ABOUTME: Red if Tab stops on every tab, an arrow moves focus without selecting, a disabled tab is landed on, or a click stops working

import { useState } from 'react';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { Tabs, type Tab } from '../Tabs';

const TABS: Tab[] = [
  { id: 'history', label: 'History' },
  { id: 'dravr', label: 'Dravr' },
  { id: 'group', label: 'Group' },
];

/** The row as a host holds it: the selected tab is state, every change is recorded. */
function Controlled({ tabs = TABS, initial = 'history', onChange = vi.fn() }: { tabs?: Tab[]; initial?: string; onChange?: (id: string) => void }) {
  const [active, setActive] = useState(initial);
  return (
    <>
      <button type="button">before</button>
      <Tabs
        tabs={tabs}
        activeTab={active}
        onChange={(id) => {
          onChange(id);
          setActive(id);
        }}
        aria-label="Panel"
      />
      <button type="button">after</button>
    </>
  );
}

const tab = (name: string) => screen.getByRole('tab', { name });

describe('Tabs', () => {
  it('puts only the selected tab in the tab order, so Tab crosses the row in one stop', async () => {
    const user = userEvent.setup();
    render(<Controlled initial="dravr" />);

    expect(tab('Dravr')).toHaveAttribute('tabindex', '0');
    expect(tab('History')).toHaveAttribute('tabindex', '-1');
    expect(tab('Group')).toHaveAttribute('tabindex', '-1');

    await user.click(screen.getByRole('button', { name: 'before' }));
    await user.tab();
    expect(tab('Dravr')).toHaveFocus();
    await user.tab();
    expect(screen.getByRole('button', { name: 'after' })).toHaveFocus();
  });

  it('moves to the next and previous tab with the arrows and selects it, wrapping round the ends', async () => {
    const onChange = vi.fn();
    const user = userEvent.setup();
    render(<Controlled onChange={onChange} />);

    await user.click(tab('History'));
    onChange.mockClear();

    await user.keyboard('{ArrowRight}');
    expect(tab('Dravr')).toHaveFocus();
    expect(tab('Dravr')).toHaveAttribute('aria-selected', 'true');
    expect(tab('Dravr')).toHaveAttribute('tabindex', '0');
    expect(tab('History')).toHaveAttribute('tabindex', '-1');

    await user.keyboard('{ArrowRight}{ArrowRight}');
    expect(tab('History')).toHaveFocus();

    await user.keyboard('{ArrowLeft}');
    expect(tab('Group')).toHaveFocus();
    expect(tab('Group')).toHaveAttribute('aria-selected', 'true');
    expect(onChange.mock.calls.map(([id]) => id)).toEqual(['dravr', 'group', 'history', 'group']);
  });

  it('goes to the first tab with Home and the last with End', async () => {
    const user = userEvent.setup();
    render(<Controlled initial="dravr" />);

    await user.click(tab('Dravr'));
    await user.keyboard('{End}');
    expect(tab('Group')).toHaveFocus();
    expect(tab('Group')).toHaveAttribute('aria-selected', 'true');
    await user.keyboard('{Home}');
    expect(tab('History')).toHaveFocus();
    expect(tab('History')).toHaveAttribute('aria-selected', 'true');
  });

  it('passes over a disabled tab with every key, and a click on it selects nothing', async () => {
    const onChange = vi.fn();
    const tabs: Tab[] = [{ id: 'off', label: 'Off', disabled: true }, ...TABS, { id: 'last-off', label: 'Last off', disabled: true }];
    const user = userEvent.setup();
    render(<Controlled tabs={tabs} onChange={onChange} />);

    await user.click(tab('Group'));
    onChange.mockClear();
    await user.keyboard('{ArrowRight}');
    expect(tab('History')).toHaveFocus();
    await user.keyboard('{ArrowLeft}');
    expect(tab('Group')).toHaveFocus();
    await user.keyboard('{Home}');
    expect(tab('History')).toHaveFocus();
    await user.keyboard('{End}');
    expect(tab('Group')).toHaveFocus();
    expect(onChange.mock.calls.map(([id]) => id)).toEqual(['history', 'group', 'history', 'group']);

    await user.click(tab('Off'));
    expect(onChange).toHaveBeenCalledTimes(4);
    expect(tab('Group')).toHaveAttribute('aria-selected', 'true');
  });

  it('keeps the row reachable when no tab is selected: the first enabled one takes the tab stop', () => {
    render(<Controlled tabs={[{ id: 'off', label: 'Off', disabled: true }, ...TABS]} initial="none" />);
    expect(tab('History')).toHaveAttribute('tabindex', '0');
    expect(tab('Off')).toHaveAttribute('tabindex', '-1');
  });

  it('leaves every other key to the page', async () => {
    const onChange = vi.fn();
    const user = userEvent.setup();
    render(<Controlled onChange={onChange} />);

    await user.click(tab('History'));
    onChange.mockClear();
    await user.keyboard('{ArrowDown}a');
    expect(tab('History')).toHaveFocus();
    expect(onChange).not.toHaveBeenCalled();
  });
});
