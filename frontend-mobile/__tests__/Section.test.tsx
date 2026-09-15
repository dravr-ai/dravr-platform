// ABOUTME: Pins the Section — a 13 / 600 title, an optional 13 secondary description, an actions slot, the content 12 below
// ABOUTME: And its inset contract: the header pays 16 itself, the content is full-bleed, and the slot is absent when there is no content

import React from 'react';
import { Text } from 'react-native';
import { render } from '@testing-library/react-native';
import { Section } from '../src/components/ui/Section';

type Instance = { type: unknown; parent: Instance | null; props: { className?: string } };

/** The nearest host view above an element — `.parent` alone lands on the composite `Text` wrapper. */
function hostParent(el: Instance): Instance {
  let node = el.parent;
  while (node && typeof node.type !== 'string') node = node.parent;
  if (!node) throw new Error('no host parent');
  return node;
}

describe('Section', () => {
  it('renders the title at 13 / 600 in the primary ink', () => {
    const { getByText } = render(
      <Section title="Account">
        <Text>Row</Text>
      </Section>,
    );
    const title = getByText('Account');
    expect(title.props.className).toContain('text-sm');
    expect(title.props.className).toContain('font-semibold');
    expect(title.props.className).toContain('text-text-primary');
  });

  it('renders the description under the title in the secondary ink', () => {
    const { getByText } = render(
      <Section title="Coaches" description="The coaches you can talk to.">
        <Text>Row</Text>
      </Section>,
    );
    const description = getByText('The coaches you can talk to.');
    expect(description.props.className).toContain('text-sm');
    expect(description.props.className).toContain('text-text-secondary');
    expect(description.props.className).toContain('mt-0.5');
  });

  it('renders no description line when none is given', () => {
    const { queryByText, toJSON } = render(
      <Section title="Coaches">
        <Text>Row</Text>
      </Section>,
    );
    expect(queryByText(/coaches you can/)).toBeNull();
    expect(JSON.stringify(toJSON())).not.toContain('mt-0.5');
  });

  it('renders the actions on the trailing side of the header row', () => {
    const { getByTestId, getByText } = render(
      <Section title="Invites" actions={<Text testID="add">Add</Text>}>
        <Text>Row</Text>
      </Section>,
    );
    expect(getByText('Add')).toBeTruthy();
    const slot = hostParent(getByTestId('add'));
    expect(slot.props.className).toContain('shrink-0');
    const header = hostParent(slot);
    expect(header.props.className).toContain('flex-row');
    expect(header.props.className).toContain('justify-between');
    expect(header.props.className).toContain('gap-4');
    expect(header.props.className).toContain('px-4');
  });

  it('insets the header 16 itself and leaves the wrapper and the content slot full-bleed', () => {
    const { getByTestId, getByText } = render(
      <Section title="Account" testID="account-section">
        <Text>The row</Text>
      </Section>,
    );
    const header = hostParent(hostParent(getByText('Account')));
    expect(header.props.className).toContain('px-4');
    expect(getByTestId('account-section').props.className).not.toContain('px-4');
    // A Row or an EmptyState pays its own inset, so the slot adds none.
    expect(hostParent(getByText('The row')).props.className).toBe('mt-3');
  });

  it('draws no content slot when there is nothing to put in it', () => {
    const renders = [
      render(<Section title="Usage analytics" />),
      render(<Section title="Usage analytics">{null}</Section>),
      render(<Section title="Usage analytics">{false}</Section>),
      render(<Section title="Usage analytics">{[]}</Section>),
      render(<Section title="Usage analytics">{undefined}</Section>),
    ];
    for (const { toJSON } of renders) {
      expect(JSON.stringify(toJSON())).not.toContain('mt-3');
    }
  });

  it('draws the content slot once there is content', () => {
    const { toJSON } = render(
      <Section title="Usage analytics">
        {false}
        <Text>One line</Text>
      </Section>,
    );
    expect(JSON.stringify(toJSON())).toContain('mt-3');
  });

  it('sets the children 12 below the header', () => {
    const { getByText } = render(
      <Section title="Account">
        <Text>The row</Text>
      </Section>,
    );
    expect(hostParent(getByText('The row')).props.className).toBe('mt-3');
  });

  it('has no fill, no border and no radius on the wrapper, and forwards testID and className', () => {
    const { getByTestId } = render(
      <Section title="Account" testID="account-section" className="mb-2">
        <Text>Row</Text>
      </Section>,
    );
    const wrapper = getByTestId('account-section');
    const className = wrapper.props.className as string;
    expect(className).toContain('mb-2');
    expect(className).not.toContain('bg-');
    expect(className).not.toContain('border');
    expect(className).not.toContain('rounded');
    expect(wrapper.props.style).toBeUndefined();
  });
});
