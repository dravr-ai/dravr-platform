// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Home's one-time question on the phone: may the athlete's agent use its memory of them, and read their health records?
// ABOUTME: Shown until answered; saving gives the ticked consents, "Not now" gives neither, and the switches stay in Privacy (carnet#1544)

import React, { useState } from 'react';
import { Text, View } from 'react-native';
import { AGENT_CONSENT_PROMPT_KEYS } from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';
import { useAuth } from '../../contexts/AuthContext';
import { useAgentConsentPrompt, useAnswerAgentConsentPrompt } from '../../hooks/useAgentConsentPrompt';
import { ALERT_TONE, Button, Checkbox } from '../../components/ui';

/**
 * Dravr's own agent reads its memory and the athlete's notes only under
 * `sensitive_t3`, and their health records only under `sensitive_t2`, both
 * withheld until the athlete allows them (Law 25 opt-in). Every athlete is
 * asked here once, as on the web. Both boxes start unticked; a class already
 * given is not offered again and is never withdrawn by this question. Once
 * answered or dismissed it is not shown again; the switches stay in Privacy.
 */
export function AgentConsentPrompt() {
  const { t } = useTranslation();
  const { isAuthenticated } = useAuth();
  const prompt = useAgentConsentPrompt(isAuthenticated);
  const answer = useAnswerAgentConsentPrompt();
  const [memory, setMemory] = useState(false);
  const [medical, setMedical] = useState(false);

  const state = prompt.data;
  if (state === undefined || !state.due) {
    if (answer.isSuccess) {
      return (
        <View className="px-4" testID="agent-consent-prompt-saved">
          <Text className="text-sm text-text-secondary" accessibilityRole="text">
            {t(AGENT_CONSENT_PROMPT_KEYS.saved)}
          </Text>
        </View>
      );
    }
    return null;
  }

  return (
    <View className="px-4">
      <View
        className={`gap-2 rounded-xl border ${ALERT_TONE.info.edge} ${ALERT_TONE.info.fill} p-3`}
        accessibilityRole="summary"
        testID="agent-consent-prompt"
      >
        <Text className={`text-sm font-semibold ${ALERT_TONE.info.ink}`}>{t(AGENT_CONSENT_PROMPT_KEYS.title)}</Text>
        <Text className={`text-sm ${ALERT_TONE.info.ink}`}>{t(AGENT_CONSENT_PROMPT_KEYS.body)}</Text>
        {!state.memory && (
          <Checkbox
            checked={memory}
            onChange={setMemory}
            label={t(AGENT_CONSENT_PROMPT_KEYS.memoryLabel)}
            labelClassName={ALERT_TONE.info.ink}
            testID="agent-consent-prompt-memory"
          />
        )}
        {!state.medical && (
          <Checkbox
            checked={medical}
            onChange={setMedical}
            label={t(AGENT_CONSENT_PROMPT_KEYS.medicalLabel)}
            labelClassName={ALERT_TONE.info.ink}
            testID="agent-consent-prompt-medical"
          />
        )}
        <View className="flex-row flex-wrap gap-2">
          <Button
            title={t(AGENT_CONSENT_PROMPT_KEYS.save)}
            variant="primary"
            onPress={() => answer.mutate({ memory, medical })}
            disabled={answer.isPending}
            testID="agent-consent-prompt-save"
          />
          <Button
            title={t(AGENT_CONSENT_PROMPT_KEYS.notNow)}
            variant="ghost"
            onPress={() => answer.mutate({ dismissed: true })}
            disabled={answer.isPending}
            testID="agent-consent-prompt-not-now"
          />
        </View>
        {answer.isError && (
          <Text className="text-sm text-error" accessibilityRole="alert" testID="agent-consent-prompt-failed">
            {t(AGENT_CONSENT_PROMPT_KEYS.failed)}
          </Text>
        )}
      </View>
    </View>
  );
}
