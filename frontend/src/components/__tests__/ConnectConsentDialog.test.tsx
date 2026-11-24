// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Garmin connect's offered consents — both boxes start unticked and Continue is never held (carnet#1157, carnet#1390)
// ABOUTME: Pins the keys Continue hands over, the privacy-policy link in a new tab, and the fresh boxes on every opening

import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import {
  GARMIN_AI_CONSENT,
  GARMIN_PRIVACY_URL,
  GARMIN_WORKOUT_TRANSFER_CONSENT,
  connectConsentsFor,
} from '@pierre/shared-constants';
import ConnectConsentDialog from '../ConnectConsentDialog';
import { ThemeProvider } from '../../hooks/useTheme';

function renderDialog(open = true, onContinue = vi.fn(), onCancel = vi.fn()) {
  const view = render(
    <ThemeProvider>
      <ConnectConsentDialog
        open={open}
        consents={connectConsentsFor('sciotte_garmin')}
        onCancel={onCancel}
        onContinue={onContinue}
      />
    </ThemeProvider>,
  );
  const rerender = (nextOpen: boolean) =>
    view.rerender(
      <ThemeProvider>
        <ConnectConsentDialog
          open={nextOpen}
          consents={connectConsentsFor('sciotte_garmin')}
          onCancel={onCancel}
          onContinue={onContinue}
        />
      </ThemeProvider>,
    );
  return { onContinue, onCancel, rerender };
}

describe('ConnectConsentDialog', () => {
  it('starts with both boxes unticked and continues with none', async () => {
    const { onContinue } = renderDialog();
    expect(screen.getByRole('dialog')).toHaveTextContent('Before you connect Garmin');
    expect(screen.getByLabelText(/Let my AI agent read my Garmin data/)).not.toBeChecked();
    const ai = screen.getByTestId(`connect-consent-${GARMIN_AI_CONSENT}`);
    const transfer = screen.getByTestId(`connect-consent-${GARMIN_WORKOUT_TRANSFER_CONSENT}`);
    expect(ai).not.toBeChecked();
    expect(transfer).not.toBeChecked();

    const proceed = screen.getByTestId('connect-consent-continue');
    expect(proceed).toBeEnabled();
    await userEvent.setup().click(proceed);

    expect(onContinue).toHaveBeenCalledWith([]);
  });

  it('hands over every box ticked, by key', async () => {
    const { onContinue } = renderDialog();
    const user = userEvent.setup();
    await user.click(screen.getByTestId(`connect-consent-${GARMIN_AI_CONSENT}`));
    await user.click(screen.getByTestId(`connect-consent-${GARMIN_WORKOUT_TRANSFER_CONSENT}`));
    await user.click(screen.getByTestId('connect-consent-continue'));

    expect(onContinue).toHaveBeenCalledWith([GARMIN_AI_CONSENT, GARMIN_WORKOUT_TRANSFER_CONSENT]);
  });

  it('hands over only the box ticked', async () => {
    const { onContinue } = renderDialog();
    const user = userEvent.setup();
    await user.click(screen.getByTestId(`connect-consent-${GARMIN_WORKOUT_TRANSFER_CONSENT}`));
    await user.click(screen.getByTestId('connect-consent-continue'));

    expect(onContinue).toHaveBeenCalledWith([GARMIN_WORKOUT_TRANSFER_CONSENT]);
  });

  it('links the transfer box to Garmin\'s privacy policy, in a new tab', () => {
    renderDialog();
    const link = screen.getByTestId(`connect-consent-link-${GARMIN_WORKOUT_TRANSFER_CONSENT}`);
    expect(link).toHaveAttribute('href', GARMIN_PRIVACY_URL);
    expect(link).toHaveAttribute('target', '_blank');
    expect(link).toHaveAttribute('rel', 'noopener noreferrer');
    expect(screen.queryByTestId(`connect-consent-link-${GARMIN_AI_CONSENT}`)).not.toBeInTheDocument();
  });

  it('cancels without continuing', async () => {
    const { onContinue, onCancel } = renderDialog();
    await userEvent.setup().click(screen.getByRole('button', { name: 'Cancel' }));
    expect(onCancel).toHaveBeenCalledTimes(1);
    expect(onContinue).not.toHaveBeenCalled();
  });

  it('asks afresh each time it opens', async () => {
    const { rerender } = renderDialog();
    await userEvent.setup().click(screen.getByTestId(`connect-consent-${GARMIN_AI_CONSENT}`));
    expect(screen.getByTestId(`connect-consent-${GARMIN_AI_CONSENT}`)).toBeChecked();

    rerender(false);
    rerender(true);

    expect(screen.getByTestId(`connect-consent-${GARMIN_AI_CONSENT}`)).not.toBeChecked();
  });
});
