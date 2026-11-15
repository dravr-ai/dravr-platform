// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for ParqForm — seven questions verbatim, the common answer in one tap, Non/Oui when some apply
// ABOUTME: Pins the one-tap seven "no", the 44px answer pair, Continue waiting on every answer, and a refusal kept on screen (carnet#1530)

import { describe, it, expect, vi } from 'vitest';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { ParqForm } from '../ParqForm';



const QUESTIONS = [
  'heart_condition',
  'chest_pain',
  'dizziness',
  'chronic_condition',
  'medication',
  'joint_problem',
  'supervised_only',
].map((id, i) => ({ id, text: `Question ${i + 1} as the server words it` }));

function renderForm(onSubmit = vi.fn().mockResolvedValue(undefined), onSkip?: () => void) {
  render(<ParqForm questions={QUESTIONS} onSubmit={onSubmit} onSkip={onSkip} />);
  return { onSubmit };
}

describe('ParqForm', () => {
  it('shows the seven questions word for word, numbered, before any answer', () => {
    renderForm();
    const items = screen.getAllByRole('listitem');
    expect(items).toHaveLength(7);
    QUESTIONS.forEach((q, i) => {
      expect(items[i]).toHaveTextContent(q.text);
      expect(items[i]).toHaveTextContent(String(i + 1));
    });
    expect(screen.queryByRole('button', { name: 'No' })).toBeNull();
  });

  it('sends seven "no" answers in one tap', async () => {
    const { onSubmit } = renderForm();
    await userEvent.click(screen.getByRole('button', { name: 'None of these apply' }));
    expect(onSubmit).toHaveBeenCalledExactlyOnceWith(QUESTIONS.map((q) => ({ id: q.id, yes: false })));
  });

  it('reveals a 44px No/Yes pair per question when some apply, and Continue waits on each', async () => {
    const { onSubmit } = renderForm();
    await userEvent.click(screen.getByRole('button', { name: 'Some apply' }));

    expect(screen.getByTestId('parq-answer-each')).toBeInTheDocument();
    const continueButton = screen.getByTestId('parq-continue');
    expect(continueButton).toBeDisabled();
    for (const q of QUESTIONS) {
      const no = screen.getByTestId(`parq-${q.id}-no`);
      expect(no).toHaveClass('h-11');
      expect(screen.getByTestId(`parq-${q.id}-yes`)).toHaveClass('h-11');
    }
    for (const q of QUESTIONS.slice(1)) await userEvent.click(screen.getByTestId(`parq-${q.id}-no`));
    expect(continueButton).toBeDisabled();
    await userEvent.click(screen.getByTestId('parq-heart_condition-yes'));
    expect(screen.getByTestId('parq-yes-note')).toHaveTextContent('Nothing stops you from training');
    expect(continueButton).toBeEnabled();

    await userEvent.click(continueButton);
    expect(onSubmit).toHaveBeenCalledExactlyOnceWith(
      QUESTIONS.map((q) => ({ id: q.id, yes: q.id === 'heart_condition' })),
    );
  });

  it('goes back to the one-tap answer, forgetting what was ticked', async () => {
    renderForm();
    await userEvent.click(screen.getByRole('button', { name: 'Some apply' }));
    await userEvent.click(screen.getByTestId('parq-medication-yes'));
    await userEvent.click(screen.getByTestId('parq-back-to-none'));
    expect(screen.getByRole('button', { name: 'None of these apply' })).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Some apply' }));
    expect(within(screen.getByTestId('parq-question-medication')).getByTestId('parq-medication-yes')).toHaveAttribute(
      'aria-pressed',
      'false',
    );
  });

  it('keeps « Skip for now » where the host offers it', async () => {
    const onSkip = vi.fn();
    renderForm(undefined, onSkip);
    await userEvent.click(screen.getByTestId('parq-skip'));
    expect(onSkip).toHaveBeenCalledTimes(1);
  });

  it('stays on screen and says so when the answers could not be stored', async () => {
    renderForm(vi.fn().mockRejectedValue(new Error('offline')));
    await userEvent.click(screen.getByRole('button', { name: 'None of these apply' }));
    expect(await screen.findByRole('alert')).toHaveTextContent("Your answers couldn't be saved. Try again.");
    await waitFor(() => expect(screen.getByRole('button', { name: 'None of these apply' })).toBeEnabled());
  });
});
