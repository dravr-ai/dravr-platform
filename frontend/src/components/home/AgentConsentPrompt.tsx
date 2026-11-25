// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Home's one-time question: may the athlete's agent use its memory of them, and read their health records?
// ABOUTME: Shown until answered; saving gives the ticked consents, "Not now" gives neither, and the switches stay in Privacy & Data (carnet#1544)

import { useId, useState } from 'react';
import { AGENT_CONSENT_PROMPT_KEYS } from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';
import { useAuth } from '../../hooks/useAuth';
import { useAgentConsentPrompt, useAnswerAgentConsentPrompt } from '../../hooks/useAgentConsentPrompt';
import { ALERT_TONE, Button, Checkbox } from '../ui';

/**
 * Dravr's own agent reads its memory and the athlete's notes only under
 * `sensitive_t3`, and their health records only under `sensitive_t2`, both
 * withheld until the athlete allows them (Law 25 opt-in). Every athlete is
 * asked here once. Both boxes start unticked; a class already given is not
 * offered again and is never withdrawn by this question. Once answered or
 * dismissed it is not shown again; the switches stay in Privacy & Data.
 */
export function AgentConsentPrompt() {
  const { t } = useTranslation();
  const { isAuthenticated } = useAuth();
  const titleId = useId();
  const prompt = useAgentConsentPrompt(isAuthenticated);
  const answer = useAnswerAgentConsentPrompt();
  const [memory, setMemory] = useState(false);
  const [medical, setMedical] = useState(false);

  const state = prompt.data;
  if (state === undefined || !state.due) {
    if (answer.isSuccess) {
      return (
        <p role="status" className="text-sm text-on-surface-variant" data-testid="agent-consent-prompt-saved">
          {t(AGENT_CONSENT_PROMPT_KEYS.saved)}
        </p>
      );
    }
    return null;
  }

  return (
    <div
      role="region"
      aria-labelledby={titleId}
      className={`${ALERT_TONE.info.tone} space-y-3 rounded-lg border p-4`}
      data-testid="agent-consent-prompt"
    >
      <p id={titleId} className="text-sm font-medium">
        {t(AGENT_CONSENT_PROMPT_KEYS.title)}
      </p>
      <p className="text-sm">{t(AGENT_CONSENT_PROMPT_KEYS.body)}</p>
      <div className="space-y-2">
        {!state.memory && (
          <Checkbox
            label={t(AGENT_CONSENT_PROMPT_KEYS.memoryLabel)}
            labelClassName="text-sm"
            checked={memory}
            onChange={(event) => setMemory(event.target.checked)}
            disabled={answer.isPending}
            data-testid="agent-consent-prompt-memory"
          />
        )}
        {!state.medical && (
          <Checkbox
            label={t(AGENT_CONSENT_PROMPT_KEYS.medicalLabel)}
            labelClassName="text-sm"
            checked={medical}
            onChange={(event) => setMedical(event.target.checked)}
            disabled={answer.isPending}
            data-testid="agent-consent-prompt-medical"
          />
        )}
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <Button
          variant="primary"
          size="sm"
          disabled={answer.isPending}
          onClick={() => answer.mutate({ memory, medical })}
          data-testid="agent-consent-prompt-save"
        >
          {t(AGENT_CONSENT_PROMPT_KEYS.save)}
        </Button>
        <Button
          variant="tertiary"
          size="sm"
          disabled={answer.isPending}
          onClick={() => answer.mutate({ dismissed: true })}
          data-testid="agent-consent-prompt-not-now"
        >
          {t(AGENT_CONSENT_PROMPT_KEYS.notNow)}
        </Button>
      </div>
      {answer.isError && (
        <p role="alert" className="text-sm text-error">
          {t(AGENT_CONSENT_PROMPT_KEYS.failed)}
        </p>
      )}
    </div>
  );
}
