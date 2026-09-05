// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the quiet search field — filled row, no border, a real search input with its spoken name
// ABOUTME: A regression here is a bordered search or one a screen reader cannot name

import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { SearchField } from '../SearchField';

describe('SearchField', () => {
  it('is a search input named for assistive tech and forwards its value', () => {
    const onChange = vi.fn();
    render(<SearchField aria-label="Search chats" placeholder="Search" value="" onChange={onChange} />);
    const input = screen.getByRole('searchbox', { name: 'Search chats' });
    expect(input).toHaveAttribute('type', 'search');
    fireEvent.change(input, { target: { value: 'camille' } });
    expect(onChange).toHaveBeenCalledTimes(1);
  });

  it('is a filled row with no hairline, not a boxed field', () => {
    const { container } = render(<SearchField aria-label="Search" />);
    const row = container.firstElementChild as HTMLElement;
    expect(row.className).toContain('bg-surface-container-low');
    expect(row.className).toContain('rounded-lg');
    expect(row.className).not.toMatch(/\bborder\b/);
    expect(row.className).not.toContain('ghost-border');
    const input = screen.getByRole('searchbox');
    expect(input.className).toContain('border-0');
    expect(input.className).toContain('bg-transparent');
  });

  it('takes an outer className so a page can size it without reaching inside', () => {
    const { container } = render(<SearchField aria-label="Search" className="w-72" />);
    expect((container.firstElementChild as HTMLElement).className).toContain('w-72');
  });
});
