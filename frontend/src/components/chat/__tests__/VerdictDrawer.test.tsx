// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the verdict drawer's subline, its triage slot, and the two cards it draws
// ABOUTME: The athlete's card links studies and previews its source reply; only the admin's prints raw ids

import { describe, it, expect, vi } from 'vitest';
import { fireEvent, render, screen, within } from '@testing-library/react';
import VerdictDrawer from '../VerdictDrawer';
import type { VerdictSource } from '../VerdictDrawer';
import type { ClaimVerdict } from '@pierre/shared-types';

function verdict(id: string, overrides: Partial<ClaimVerdict> = {}): ClaimVerdict {
  return {
    id,
    conversation_id: 'conv-1',
    message_id: 'msg-1',
    agent_id: 'coach-1',
    claim_text: 'Creatine at 5 g per day improves high-intensity performance.',
    category: 'supplement',
    status: 'supported',
    evidence_strength: 'strong',
    confidence: 0.8,
    layer_fired: 'evidence',
    explanation: null,
    evidence_refs: null,
    created_at: '2026-04-13T18:00:01Z',
    ...overrides,
  };
}

describe('VerdictDrawer subline', () => {
  it('says the count alone when a reply drew several verdicts', () => {
    render(
      <VerdictDrawer
        verdicts={[verdict('v1'), verdict('v2', { status: 'contradicted' })]}
        onClose={vi.fn()}
      />,
    );

    // The chip's string is "{{count}} verdicts · {{qualifier}}", and the
    // drawer used to pass an empty qualifier and strip the dangling separator
    // with a regex — which only ever knew the separator English and French
    // happen to use. The subline has its own key now.
    const subline = screen.getByText(/2 verdicts/);
    expect(subline.textContent).toBe('2 verdicts');
    expect(subline.textContent).not.toMatch(/·/);
  });

  it('keeps the status qualifier when there is exactly one', () => {
    render(<VerdictDrawer verdicts={[verdict('v1')]} onClose={vi.fn()} />);

    expect(screen.getByText('1 verdict · supported')).toBeInTheDocument();
  });

  it('says it is still reading while the rows are in flight', () => {
    render(<VerdictDrawer verdicts={[]} loading onClose={vi.fn()} />);

    // The subline and the empty body both say it — the header while the count
    // is unknown, the body where the cards will land.
    expect(screen.getAllByText('Loading verdicts…')).toHaveLength(2);
  });
});


describe('VerdictDrawer triage slot', () => {
  it('renders nothing extra on the chat surface', () => {
    render(<VerdictDrawer verdicts={[verdict('v1')]} onClose={vi.fn()} />);

    expect(screen.queryByTestId('triage-slot')).not.toBeInTheDocument();
  });

  it('renders what the admin passes under each card, once per verdict', () => {
    const renderTriage = vi.fn((v: ClaimVerdict) => (
      <div data-testid="triage-slot">triage for {v.id}</div>
    ));
    render(
      <VerdictDrawer
        verdicts={[verdict('v1'), verdict('v2', { status: 'contradicted' })]}
        onClose={vi.fn()}
        renderTriage={renderTriage}
      />,
    );

    const slots = screen.getAllByTestId('triage-slot');
    expect(slots).toHaveLength(2);
    expect(slots[0]).toHaveTextContent('triage for v1');
    expect(slots[1]).toHaveTextContent('triage for v2');
    // The slot is a function of the verdict, so the panel it renders can
    // read the row's own disposition and knob.
    expect(renderTriage).toHaveBeenCalledWith(expect.objectContaining({ id: 'v1' }));
    expect(renderTriage).toHaveBeenCalledWith(expect.objectContaining({ id: 'v2' }));
  });
});

const SOURCE: VerdictSource = {
  title: 'Easy Tuesday run',
  content:
    'No HR zones on file, so this is an estimate.\n\n- If it felt smooth, count the session as a good one.',
  createdAt: '2026-04-13T18:00:00Z',
};

const SUPPORTED = verdict('v1', {
  claim_text: '- If it felt smooth, count the session as a good one.',
  category: 'training_prescription',
  explanation: 'Supported by Rønnestad and Mujika 2014',
  evidence_refs: 'doi:10.1111/sms.12104',
});

describe('VerdictDrawer athlete card', () => {
  it('prints no raw id and no verifier layer', () => {
    render(<VerdictDrawer verdicts={[SUPPORTED]} onClose={vi.fn()} source={SOURCE} />);

    expect(screen.queryByText('conv-1')).not.toBeInTheDocument();
    expect(screen.queryByText('msg-1')).not.toBeInTheDocument();
    expect(screen.queryByText('Provenance')).not.toBeInTheDocument();
    expect(screen.queryByText('evidence')).not.toBeInTheDocument();
    expect(screen.getByTestId('verdict-meta')).toHaveTextContent(
      'Training Prescription · evidence: strong · confidence: 80%',
    );
  });

  it('links one study as "Read the study", opening doi.org in a new tab', () => {
    render(<VerdictDrawer verdicts={[SUPPORTED]} onClose={vi.fn()} />);

    const link = screen.getByRole('link', { name: 'Read the study' });
    expect(link).toHaveAttribute('href', 'https://doi.org/10.1111/sms.12104');
    expect(link).toHaveAttribute('target', '_blank');
    expect(link).toHaveAttribute('rel', 'noopener noreferrer');
  });

  it('numbers the studies when there are several', () => {
    render(
      <VerdictDrawer
        verdicts={[verdict('v1', { evidence_refs: 'doi:10.1111/sms.12104,pmid:22389869' })]}
        onClose={vi.fn()}
      />,
    );

    expect(screen.getByRole('link', { name: 'Study 1' })).toHaveAttribute('href', 'https://doi.org/10.1111/sms.12104');
    expect(screen.getByRole('link', { name: 'Study 2' })).toHaveAttribute(
      'href',
      'https://pubmed.ncbi.nlm.nih.gov/22389869/',
    );
  });

  it('names the conversation, and shows the reply only on hover until pressed', () => {
    render(<VerdictDrawer verdicts={[SUPPORTED]} onClose={vi.fn()} source={SOURCE} />);

    const pill = screen.getByTestId('verdict-source-pill');
    expect(pill).toHaveTextContent('Easy Tuesday run');
    expect(pill).toHaveAttribute('aria-expanded', 'false');

    const preview = screen.getByTestId('verdict-source-preview');
    // Hidden at rest; the hover and keyboard-focus variants reveal it.
    expect(preview).toHaveClass('hidden');
    expect(preview.className).toContain('group-hover/source:block');
    // The claim is marked inside the reply, without the list bullet.
    expect(within(preview).getByText('If it felt smooth, count the session as a good one.').tagName).toBe('MARK');
    expect(preview).toHaveTextContent('No HR zones on file, so this is an estimate.');

    fireEvent.click(pill);
    expect(pill).toHaveAttribute('aria-expanded', 'true');
    expect(preview).not.toHaveClass('hidden');
  });

  it('closes the drawer from the preview, onto the reply the chip hangs under', () => {
    const onClose = vi.fn();
    render(<VerdictDrawer verdicts={[SUPPORTED]} onClose={onClose} source={SOURCE} />);

    fireEvent.click(screen.getByRole('button', { name: 'See it in the conversation' }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('hands support the reference from the actions menu', () => {
    const onCopyReference = vi.fn();
    render(<VerdictDrawer verdicts={[SUPPORTED]} onClose={vi.fn()} onCopyReference={onCopyReference} />);

    fireEvent.click(screen.getByRole('button', { name: 'Verdict actions' }));
    fireEvent.click(screen.getByRole('menuitem', { name: 'Copy reference for support' }));

    expect(onCopyReference).toHaveBeenCalledWith(expect.objectContaining({ id: 'v1' }));
    expect(screen.queryByRole('menu')).not.toBeInTheDocument();
  });

  it('closes the menu on Escape without closing the drawer', () => {
    const onClose = vi.fn();
    render(<VerdictDrawer verdicts={[SUPPORTED]} onClose={onClose} onCopyReference={vi.fn()} />);

    fireEvent.click(screen.getByRole('button', { name: 'Verdict actions' }));
    fireEvent.keyDown(document, { key: 'Escape' });

    expect(screen.queryByRole('menu')).not.toBeInTheDocument();
    expect(onClose).not.toHaveBeenCalled();
  });

  it('offers no actions menu when the surface cannot copy', () => {
    render(<VerdictDrawer verdicts={[SUPPORTED]} onClose={vi.fn()} />);

    expect(screen.queryByRole('button', { name: 'Verdict actions' })).not.toBeInTheDocument();
  });
});

describe('VerdictDrawer operator card', () => {
  it('keeps the raw ids and the layer, with each reference linked by its id', () => {
    render(
      <VerdictDrawer
        verdicts={[SUPPORTED]}
        onClose={vi.fn()}
        renderTriage={() => <div data-testid="triage-slot" />}
      />,
    );

    expect(screen.getByText('conv-1')).toBeInTheDocument();
    expect(screen.getByText('msg-1')).toBeInTheDocument();
    expect(screen.getByText('evidence')).toBeInTheDocument();
    expect(screen.getByRole('link', { name: 'doi:10.1111/sms.12104' })).toHaveAttribute(
      'href',
      'https://doi.org/10.1111/sms.12104',
    );
    expect(screen.queryByTestId('verdict-pills')).not.toBeInTheDocument();
  });
});
