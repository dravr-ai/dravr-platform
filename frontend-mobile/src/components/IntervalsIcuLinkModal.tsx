// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Intervals.icu link sheet for mobile — collects athlete id + API key (non-OAuth) in the one bottom sheet
// ABOUTME: Server validates the HTTP Basic ("API_KEY":api_key) pair live before storing

import React, { useState, useEffect } from 'react';
import { View, Text, KeyboardAvoidingView, Platform } from 'react-native';
import { CheckCircle2 } from 'lucide-react-native';
import { describeApiError } from '@pierre/ui-logic';
import { useThemeColors } from '../constants/theme';
import { oauthApi } from '../services/api';
import { IntervalsIcuLogo } from './icons/BrandIcons';
import { Button, Input, Sheet } from './ui';
import { useTranslation } from '@pierre/i18n';

interface IntervalsIcuLinkModalProps {
  visible: boolean;
  onClose: () => void;
  onConnected: () => void;
}

export function IntervalsIcuLinkModal({ visible, onClose, onConnected }: IntervalsIcuLinkModalProps) {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const [athleteId, setAthleteId] = useState('');
  const [apiKey, setApiKey] = useState('');
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // The linked athlete once the server accepted the pair: the name it knows,
  // else the id, else nothing but the confirmation word.
  const [linkedAthlete, setLinkedAthlete] = useState<string | null>(null);
  const [success, setSuccess] = useState(false);

  useEffect(() => {
    if (visible) {
      setAthleteId('');
      setApiKey('');
      setIsLoading(false);
      setError(null);
      setLinkedAthlete(null);
      setSuccess(false);
    }
  }, [visible]);

  const handleSubmit = async () => {
    setIsLoading(true);
    setError(null);
    try {
      const result = await oauthApi.linkIntervalsIcu({
        athlete_id: athleteId.trim(),
        api_key: apiKey.trim(),
      });
      setLinkedAthlete(result.athlete?.name || result.athlete?.id || null);
      setSuccess(true);
      setTimeout(onConnected, 1200);
    } catch (err) {
      setError(describeApiError(err, { t, fallbackKey: 'shell.intervalsLinkFailed' }));
      setIsLoading(false);
    }
  };

  const canSubmit = athleteId.trim().length > 0 && apiKey.trim().length > 0 && !isLoading;

  return (
    <Sheet visible={visible} onClose={onClose} testID="intervals-sheet" backdropTestID="intervals-sheet-backdrop">
      <KeyboardAvoidingView behavior={Platform.OS === 'ios' ? 'padding' : undefined}>
        <View className="flex-row items-center gap-3 mb-4">
          <IntervalsIcuLogo size={24} />
          <View className="flex-1 min-w-0">
            <Text className="text-lg font-semibold text-text-primary">
              {t('app.connectProvider', { provider: 'Intervals.icu' })}
            </Text>
            <Text className="text-xs text-text-tertiary">{t('app.apiKeyNoOauth')}</Text>
          </View>
        </View>

        {success ? (
          <View className="items-center py-6">
            <CheckCircle2 size={40} color={colors.success} />
            <Text className="mt-3 text-base font-medium text-text-primary">{t('app.connectedBang')}</Text>
            {linkedAthlete !== null && (
              <Text className="mt-1 text-sm text-text-secondary">{linkedAthlete}</Text>
            )}
          </View>
        ) : (
          <>
            <Text className="text-sm text-text-secondary mb-4">
              {t('frag.findAthleteIdUnder')} {t('shell.intervalsSettingsPath')}
            </Text>

            <Input
              label={t('app.athleteId')}
              placeholder="i123456"
              value={athleteId}
              onChangeText={setAthleteId}
              autoCapitalize="none"
              autoCorrect={false}
              testID="intervals-athlete-id"
            />

            <Input
              label={t('app.apiKey')}
              placeholder={t('app.apiKeyLower')}
              value={apiKey}
              onChangeText={setApiKey}
              secureTextEntry
              showPasswordToggle
              autoCapitalize="none"
              autoCorrect={false}
              testID="intervals-api-key"
            />
            {error && (
              <Text className="text-sm text-error mb-3" testID="intervals-error">
                {error}
              </Text>
            )}

            <Button
              title={t('app.connect')}
              onPress={handleSubmit}
              disabled={!canSubmit}
              loading={isLoading}
              fullWidth
              testID="intervals-submit"
            />
          </>
        )}
      </KeyboardAvoidingView>
    </Sheet>
  );
}
