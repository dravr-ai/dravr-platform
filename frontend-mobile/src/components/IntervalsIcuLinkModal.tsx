// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Intervals.icu link sheet for mobile — the authorization when offered, and the athlete id + API key form
// ABOUTME: The key form is the coach's path (only a key reads coached athletes); the server validates the pair live before storing

import React, { useState, useEffect } from 'react';
import { View, Text, KeyboardAvoidingView, Platform } from 'react-native';
import { CheckCircle2 } from 'lucide-react-native';
import { describeApiError } from '@pierre/ui-logic';
import { useThemeColors } from '../constants/theme';
import { oauthApi } from '../services/api';
import { ProviderGlyph } from './ProviderGlyph';
import { PROVIDER_BRAND } from '../constants/brands';
import { Button, Input, Sheet } from './ui';
import { useTranslation } from '@pierre/i18n';

interface IntervalsIcuLinkModalProps {
  visible: boolean;
  onClose: () => void;
  onConnected: () => void;
  /**
   * Starts the Intervals.icu authorization. Given while the server offers it
   * (`intervalsConnectPath` answers `'choose'`): the sheet then leads with
   * the authorization and keeps the API-key form, the coach's path, one tap
   * away. Absent, the sheet is the key form alone — no client to authorize
   * under, or a key-linked row reconnecting.
   */
  onAuthorize?: () => void;
}

/** What the sheet shows: the choice of the two paths, or the key form. */
type LinkView = 'choose' | 'key';

export function IntervalsIcuLinkModal({ visible, onClose, onConnected, onAuthorize }: IntervalsIcuLinkModalProps) {
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
  const offersAuthorize = onAuthorize !== undefined;
  const [view, setView] = useState<LinkView>(offersAuthorize ? 'choose' : 'key');

  useEffect(() => {
    if (visible) {
      setAthleteId('');
      setApiKey('');
      setIsLoading(false);
      setError(null);
      setLinkedAthlete(null);
      setSuccess(false);
      setView(offersAuthorize ? 'choose' : 'key');
    }
  }, [visible, offersAuthorize]);

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
          <ProviderGlyph providerId="intervals_icu" label={PROVIDER_BRAND.intervalsIcu} />
          <View className="flex-1 min-w-0">
            <Text className="text-lg font-semibold text-text-primary">
              {t('app.connectProvider', { provider: PROVIDER_BRAND.intervalsIcu })}
            </Text>
            {view === 'key' && (
              <Text className="text-xs text-text-tertiary">{t('shell.intervalsApiKeyHint')}</Text>
            )}
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
        ) : view === 'choose' && onAuthorize ? (
          <View testID="intervals-authorize-choice">
            <Text className="text-sm text-text-secondary mb-4">{t('shell.intervalsAuthorizeHint')}</Text>
            <Button
              title={t('shell.intervalsAuthorizeAction')}
              onPress={onAuthorize}
              fullWidth
              testID="intervals-authorize"
            />
            <Text
              className="mt-4 text-center text-md font-medium text-primary"
              onPress={() => setView('key')}
              accessibilityRole="button"
              testID="intervals-use-api-key"
            >
              {t('shell.intervalsCoachKeyAction')}
            </Text>
          </View>
        ) : (
          <>
            {offersAuthorize && (
              <Text
                className="mb-3 text-md font-medium text-primary"
                onPress={() => setView('choose')}
                accessibilityRole="button"
                testID="intervals-back-to-authorize"
              >
                {t('common.back')}
              </Text>
            )}
            <Text className="text-sm text-text-secondary mb-4">
              {t('shell.intervalsCredentialsHelp')}
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
