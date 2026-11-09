// ABOUTME: The "Bring in your history" sheet lists the card's choices: sign in once, import the export, not now
// ABOUTME: Pins that the export is offered only for a source the server put on the card and this build has the words for
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { i18n } from '@pierre/i18n';
import { importSourceCopy } from '@pierre/ui-logic';
import BringHistorySheet, { type BringHistoryCard } from '../BringHistorySheet';

/** The Garmin card after its grant, its history still to bring in. */
function garmin(overrides: Partial<BringHistoryCard['history'] & object> = {}, state: 'needed' | 'incomplete' | 'imported' = 'needed'): BringHistoryCard {
  return {
    provider: 'sciotte_garmin',
    display_name: 'Garmin',
    connected: true,
    connected_backend: 'garmin',
    needs_reauth: false,
    requires_oauth: false,
    consent_required: false,
    history: { state, sign_in_target: 'sciotte_garmin', archive_source: 'garmin_archive', ...overrides },
  };
}

function renderSheet(card: BringHistoryCard) {
  const onSignIn = vi.fn();
  const onClose = vi.fn();
  render(<BringHistorySheet card={card} onSignIn={onSignIn} onClose={onClose} />);
  return { onSignIn, onClose, user: userEvent.setup() };
}

describe('BringHistorySheet', () => {
  const open = vi.fn();
  beforeEach(() => {
    open.mockReset().mockReturnValue({ closed: false });
    vi.stubGlobal('open', open);
  });
  afterEach(() => vi.unstubAllGlobals());

  it('is a dialog named by its title, saying what the provider sends and listing both choices', () => {
    renderSheet(garmin());

    const sheet = screen.getByRole('dialog', { name: i18n.t('providers.history.sheetTitle', { provider: 'Garmin' }) });
    expect(sheet).toHaveTextContent(i18n.t('providers.history.sheetBody', { provider: 'Garmin' }));
    // Each choice is named by its visible words (WCAG 2.5.3, label in name).
    expect(
      within(sheet).getByRole('button', { name: new RegExp(i18n.t('providers.history.choiceSignIn')) }),
    ).toHaveTextContent(i18n.t('providers.history.explainer', { provider: 'Garmin' }));
    expect(
      within(sheet).getByRole('button', {
        name: new RegExp(i18n.t('providers.history.choiceArchive', { provider: 'Garmin' })),
      }),
    ).toHaveTextContent(i18n.t('providers.history.choiceArchiveHint', { provider: 'Garmin' }));
    expect(within(sheet).getByRole('button', { name: i18n.t('providers.history.notNow') })).toBeInTheDocument();
  });

  it('closes, then hands the sign-in its target', async () => {
    const { onSignIn, onClose, user } = renderSheet(garmin());

    await user.click(screen.getByTestId('bring-history-sign-in'));

    expect(onClose).toHaveBeenCalledTimes(1);
    expect(onSignIn).toHaveBeenCalledWith('garmin');
    expect(onClose.mock.invocationCallOrder[0]).toBeLessThan(onSignIn.mock.invocationCallOrder[0]);
  });

  it('opens the import window on the card\'s export', async () => {
    const { onSignIn, onClose, user } = renderSheet(garmin());

    await user.click(screen.getByTestId('bring-history-archive'));

    expect(open).toHaveBeenCalledWith('/import#garmin', '_blank');
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(onSignIn).not.toHaveBeenCalled();
  });

  it('hides the export when the server names no source (imports off, or none offered)', () => {
    renderSheet(garmin({ archive_source: null }));

    expect(screen.getByTestId('bring-history-sign-in')).toBeInTheDocument();
    expect(screen.queryByTestId('bring-history-archive')).not.toBeInTheDocument();
  });

  it('says where an uploaded export is found, in its source\'s own hint, never the link one', () => {
    const copy = importSourceCopy('whoop_archive');
    expect(copy?.intake).toBe('upload');
    renderSheet({ ...garmin({ archive_source: 'whoop_archive', sign_in_target: null }), provider: 'whoop', display_name: 'WHOOP' });

    const archive = screen.getByTestId('bring-history-archive');
    expect(archive).toHaveTextContent(i18n.t(copy?.hint ?? ''));
    expect(archive).not.toHaveTextContent(i18n.t('providers.history.choiceArchiveHint', { provider: 'WHOOP' }));
  });

  it('hides a source this build has no words for', () => {
    renderSheet(garmin({ archive_source: 'fit_zip_archive' }));

    expect(screen.queryByTestId('bring-history-archive')).not.toBeInTheDocument();
  });

  it('offers the sign-in again for a copy that did not finish, and only the export once the history is in', () => {
    const { unmount } = render(<BringHistorySheet card={garmin({}, 'incomplete')} onSignIn={vi.fn()} onClose={vi.fn()} />);
    expect(screen.getByTestId('bring-history-sign-in')).toBeInTheDocument();
    unmount();

    render(<BringHistorySheet card={garmin({}, 'imported')} onSignIn={vi.fn()} onClose={vi.fn()} />);
    expect(screen.queryByTestId('bring-history-sign-in')).not.toBeInTheDocument();
    expect(screen.getByTestId('bring-history-archive')).toBeInTheDocument();
  });

  it('closes on Not now, opening nothing', async () => {
    const { onSignIn, onClose, user } = renderSheet(garmin());

    await user.click(screen.getByTestId('bring-history-not-now'));

    expect(onClose).toHaveBeenCalledTimes(1);
    expect(onSignIn).not.toHaveBeenCalled();
    expect(open).not.toHaveBeenCalled();
  });
});
