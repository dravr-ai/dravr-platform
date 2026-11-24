// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The consents a connect offers without requiring them (Garmin's AI use and workout transfer), asked before it starts
// ABOUTME: Every box starts unticked and Continue is never held: the connect goes through with none, carrying the ticked keys

import { useEffect, useState } from 'react';
import { useTranslation } from '@pierre/i18n';
import { GARMIN_CONSENT_KEYS, type ConnectConsent } from '@pierre/shared-constants';
import { Button, Checkbox, Modal, ModalActions } from './ui';

interface ConnectConsentDialogProps {
  open: boolean;
  /** The boxes the connect offers (`connectConsentsFor`). */
  consents: readonly ConnectConsent[];
  onCancel: () => void;
  /**
   * The athlete continued, with the keys of the boxes they ticked — none is
   * an answer too. Called inside the Continue click, so a connect started
   * from it stays within the browser's user gesture.
   */
  onContinue: (keys: string[]) => void;
}

/**
 * Asks the consents a connect offers (carnet#1157, carnet#1390): one unticked
 * box per consent with its hint and, when it has one, the policy it links to.
 */
export default function ConnectConsentDialog({ open, consents, onCancel, onContinue }: ConnectConsentDialogProps) {
  const { t } = useTranslation();
  const [ticked, setTicked] = useState<ReadonlySet<string>>(new Set());

  // Each opening asks afresh: nothing ticked on a previous connect carries over.
  useEffect(() => {
    if (open) setTicked(new Set());
  }, [open]);

  const toggle = (key: string, checked: boolean) =>
    setTicked((current) => {
      const next = new Set(current);
      if (checked) next.add(key);
      else next.delete(key);
      return next;
    });

  return (
    <Modal isOpen={open} onClose={onCancel} title={t(GARMIN_CONSENT_KEYS.title)} size="md">
      <p className="text-sm text-on-surface-variant leading-relaxed mb-4">{t(GARMIN_CONSENT_KEYS.intro)}</p>
      <ul className="space-y-4 mb-2" data-testid="connect-consent-list">
        {consents.map((consent) => (
          <li key={consent.key}>
            <Checkbox
              id={`connect-consent-${consent.key}`}
              data-testid={`connect-consent-${consent.key}`}
              label={t(consent.labelKey)}
              description={t(consent.hintKey)}
              checked={ticked.has(consent.key)}
              onChange={(e) => toggle(consent.key, e.target.checked)}
            />
            {consent.linkUrl && (
              <a
                href={consent.linkUrl}
                target="_blank"
                rel="noopener noreferrer"
                className="ml-7 mt-1 inline-block text-xs font-medium text-primary underline hover:text-primary-hover"
                data-testid={`connect-consent-link-${consent.key}`}
              >
                {consent.linkKey ? t(consent.linkKey) : consent.linkUrl}
              </a>
            )}
          </li>
        ))}
      </ul>
      <ModalActions>
        <Button variant="secondary" onClick={onCancel}>
          {t('common.cancel')}
        </Button>
        <Button
          variant="primary"
          onClick={() => onContinue(consents.filter((c) => ticked.has(c.key)).map((c) => c.key))}
          data-testid="connect-consent-continue"
        >
          {t(GARMIN_CONSENT_KEYS.continue)}
        </Button>
      </ModalActions>
    </Modal>
  );
}
