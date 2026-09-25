// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The notice a provider requires before connecting (TrainingPeaks, COROS, WHOOP) and its required checkbox
// ABOUTME: One callout for the credential login and the dialog shown before an OAuth provider's authorization page

import { useState } from 'react';
import { useTranslation } from '@pierre/i18n';
import { PROVIDER_NOTICES, type ProviderNoticeKeys } from '@pierre/shared-constants';
import { Button, Checkbox, Modal, ModalActions } from './ui';

interface ProviderNoticeProps {
  notice: ProviderNoticeKeys;
  /** The checkbox's id, unique on the page. */
  id: string;
  accepted: boolean;
  onAcceptedChange: (accepted: boolean) => void;
}

/**
 * The notice callout: title, body and the checkbox that states the
 * acceptance, written in the amber tint's own ink.
 */
export function ProviderNotice({ notice, id, accepted, onAcceptedChange }: ProviderNoticeProps) {
  const { t } = useTranslation();
  return (
    <div role="note" className="mb-4 rounded-lg border border-warning/40 bg-warning/10 p-3">
      <p className="text-sm font-medium text-on-warning-container mb-1">{t(notice.titleKey)}</p>
      <p className="text-sm text-on-warning-container mb-3">{t(notice.bodyKey)}</p>
      <Checkbox
        id={id}
        label={t(notice.consentKey)}
        checked={accepted}
        onChange={(e) => onAcceptedChange(e.target.checked)}
        labelClassName="text-on-warning-container"
      />
    </div>
  );
}

interface ProviderNoticeDialogProps {
  /** The backend whose notice is shown, or `null` while the dialog is closed. */
  provider: string | null;
  onCancel: () => void;
  /**
   * The athlete accepted the notice. Called inside the Continue click, so an
   * OAuth window opened from it stays within the browser's user gesture.
   */
  onAccept: () => void;
}

/**
 * The notice an OAuth provider (WHOOP) requires, shown before its
 * authorization page opens. Continue stays disabled until the box is ticked.
 */
export function ProviderNoticeDialog({ provider, onCancel, onAccept }: ProviderNoticeDialogProps) {
  const { t } = useTranslation();
  const notice = provider ? PROVIDER_NOTICES[provider] : undefined;
  const [accepted, setAccepted] = useState(false);

  const close = () => {
    setAccepted(false);
    onCancel();
  };

  return (
    <Modal isOpen={notice !== undefined} onClose={close} title={notice ? t(notice.titleKey) : undefined} size="md">
      {notice && (
        <>
          <ProviderNotice
            notice={notice}
            id={`provider-notice-${provider}`}
            accepted={accepted}
            onAcceptedChange={setAccepted}
          />
          <ModalActions>
            <Button variant="secondary" onClick={close}>
              {t('common.cancel')}
            </Button>
            <Button
              variant="primary"
              disabled={!accepted}
              onClick={() => {
                // Held until ticked, whatever the button's own state says.
                if (!accepted) return;
                setAccepted(false);
                onAccept();
              }}
            >
              {t('app.continue')}
            </Button>
          </ModalActions>
        </>
      )}
    </Modal>
  );
}
