// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Contract tests for the Boreal Editorial form fields (DESIGN.md §5)
// ABOUTME: Asserts Input/Textarea/Select share one label, chrome and error language

import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, it, expect, vi } from 'vitest';
import { Input } from '../Input';
import { Textarea } from '../Textarea';
import { Select } from '../Select';
import { Checkbox, Radio } from '../Checkbox';

const OPTIONS = [
  { value: 'a', label: 'Alpha' },
  { value: 'b', label: 'Beta' },
];

// The regression this suite exists to prevent: three fields in one card
// rendering three different design languages. Each field is checked against the
// SAME expectations, so a future divergence fails here rather than in a
// screenshot six months later.
describe('Boreal editorial form fields', () => {
  const cases = [
    {
      name: 'Input',
      render: () => render(<Input label="Group Name" />),
      field: () => screen.getByLabelText('Group Name'),
    },
    {
      name: 'Textarea',
      render: () => render(<Textarea label="Description" />),
      field: () => screen.getByLabelText('Description'),
    },
    {
      name: 'Select',
      render: () => render(<Select label="Coach" options={OPTIONS} />),
      field: () => screen.getByLabelText('Coach'),
    },
  ];

  for (const c of cases) {
    describe(c.name, () => {
      it('wears the shared underline chrome, never an enclosing box', () => {
        c.render();
        const field = c.field();
        expect(field.className).toContain('boreal-underline-input');
        expect(field.className).toContain('bg-transparent');
        // A boxed field is the exact drift this primitive replaced.
        expect(field.className).not.toMatch(/\brounded-lg\b/);
        expect(field.className).not.toMatch(/\bborder\b/);
        expect(field.className).not.toContain('bg-surface-container-low');
      });

      it('renders the sentence-case label in the body face', () => {
        c.render();
        // Label is associated via htmlFor/id — getByLabelText above already
        // proves the wiring; here we pin the typography. Boreal v2 retired the
        // 11px tracked caps: the only tracked text in the product is the wordmark.
        const label = document.querySelector('label');
        expect(label).not.toBeNull();
        expect(label?.className).toContain('text-sm');
        expect(label?.className).toContain('font-medium');
        expect(label?.className).toContain('text-on-surface-variant');
        expect(label?.className).not.toContain('uppercase');
        expect(label?.className).not.toContain('font-label');
        expect(label?.style.letterSpacing).toBe('');
      });
    });
  }

  it('gives every field a unique id when several share a page', () => {
    render(
      <>
        <Input label="First" />
        <Input label="Second" />
      </>,
    );
    const first = screen.getByLabelText('First');
    const second = screen.getByLabelText('Second');
    expect(first.id).not.toBe('');
    expect(first.id).not.toBe(second.id);
  });
});

// The underline chrome carries !important (it has to beat @tailwindcss/forms),
// which also beats inline styles — so the error state must arrive as a class or
// it silently never paints.
describe('error state', () => {
  it('marks Input invalid with the error modifier class', () => {
    render(<Input label="Email" error="Required" />);
    const field = screen.getByLabelText('Email');
    expect(field.className).toContain('boreal-underline-input--error');
    expect(field).toHaveAttribute('aria-invalid', 'true');
    expect(screen.getByText('Required')).toBeInTheDocument();
  });

  it('marks Textarea invalid with the error modifier class', () => {
    render(<Textarea label="Bio" error="Too short" />);
    const field = screen.getByLabelText('Bio');
    expect(field.className).toContain('boreal-underline-input--error');
    expect(field).toHaveAttribute('aria-invalid', 'true');
  });

  it('marks Select invalid with the error modifier class', () => {
    render(<Select label="Tier" options={OPTIONS} error="Pick one" />);
    const field = screen.getByLabelText('Tier');
    expect(field.className).toContain('boreal-underline-input--error');
    expect(field).toHaveAttribute('aria-invalid', 'true');
  });

  it('hides help text once an error is present', () => {
    render(<Input label="Email" helpText="We never share it" error="Required" />);
    expect(screen.queryByText('We never share it')).not.toBeInTheDocument();
    expect(screen.getByText('Required')).toBeInTheDocument();
  });

  it('leaves aria-invalid off when the field is valid', () => {
    render(<Textarea label="Bio" helpText="Optional" />);
    const field = screen.getByLabelText('Bio');
    expect(field).not.toHaveAttribute('aria-invalid');
    expect(screen.getByText('Optional')).toBeInTheDocument();
  });
});

describe('behaviour', () => {
  it('Textarea forwards typed input and honours rows', async () => {
    const onChange = vi.fn();
    render(<Textarea label="Notes" rows={5} onChange={onChange} />);
    const field = screen.getByLabelText('Notes');
    expect(field).toHaveAttribute('rows', '5');
    await userEvent.type(field, 'hi');
    expect(onChange).toHaveBeenCalledTimes(2);
  });

  it('Select renders every option and reports the chosen value', async () => {
    const onChange = vi.fn();
    render(<Select label="Coach" options={OPTIONS} defaultValue="a" onChange={onChange} />);
    const field = screen.getByLabelText('Coach') as HTMLSelectElement;
    expect(screen.getAllByRole('option')).toHaveLength(2);
    await userEvent.selectOptions(field, 'b');
    expect(field.value).toBe('b');
    expect(onChange).toHaveBeenCalled();
  });

  it('Select renders a placeholder that cannot be re-chosen', () => {
    render(<Select label="Coach" options={OPTIONS} placeholder="Select a coach..." />);
    const placeholder = screen.getByRole('option', { name: 'Select a coach...' });
    expect(placeholder).toBeDisabled();
    expect(screen.getAllByRole('option')).toHaveLength(3);
  });
});

// Checkbox and Radio arrived last; before them every choice control in the app
// was hand-rolled, which is how the label/description treatment drifted per
// call site. These pin the shared contract.
describe('choice controls', () => {
  it('Checkbox binds its label to the control and reports toggles', async () => {
    const onChange = vi.fn();
    render(
      <Checkbox
        label="Enable peer data sharing"
        description="Allows members who consent to see each other's data."
        onChange={onChange}
      />,
    );
    const box = screen.getByLabelText(/Enable peer data sharing/);
    expect(box).toHaveAttribute('type', 'checkbox');
    expect(screen.getByText(/Allows members who consent/)).toBeInTheDocument();
    await userEvent.click(box);
    expect(onChange).toHaveBeenCalledTimes(1);
  });

  it('Radio groups by name so only one stays selected', async () => {
    const onA = vi.fn();
    const onB = vi.fn();
    render(
      <>
        <Radio name="mode" label="Summary" value="a" onChange={onA} />
        <Radio name="mode" label="Detailed" value="b" onChange={onB} />
      </>,
    );
    const a = screen.getByLabelText('Summary') as HTMLInputElement;
    const b = screen.getByLabelText('Detailed') as HTMLInputElement;
    expect(a).toHaveAttribute('type', 'radio');
    await userEvent.click(a);
    expect(a.checked).toBe(true);
    await userEvent.click(b);
    expect(b.checked).toBe(true);
    expect(a.checked).toBe(false);
  });

  it('both surface errors in the same voice as the text fields', () => {
    const { unmount } = render(<Checkbox label="Accept" error="Required" />);
    expect(screen.getByLabelText('Accept')).toHaveAttribute('aria-invalid', 'true');
    expect(screen.getByText('Required')).toBeInTheDocument();
    unmount();
    render(<Radio name="r" label="Pick" error="Required" />);
    expect(screen.getByLabelText('Pick')).toHaveAttribute('aria-invalid', 'true');
  });

  it('gives each control a unique id so labels never cross-bind', () => {
    render(
      <>
        <Checkbox label="First" />
        <Checkbox label="Second" />
      </>,
    );
    expect(screen.getByLabelText('First').id).not.toBe(screen.getByLabelText('Second').id);
  });
});
