// ABOUTME: Self-serve account deletion on the mobile privacy screen — a destructive action behind a typed-email sheet
// ABOUTME: Previews what the delete asks for and what blocks it, deletes, then signs the app out (App Store 5.1.1(v))
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React, { useState } from 'react';
import { Text, View } from 'react-native';
import { useMutation, useQuery } from '@tanstack/react-query';
import { useTranslation } from '@pierre/i18n';
import type { AccountDeletionBlocker } from '@pierre/shared-types';
import {
  describeAccountDeletionBlocker,
  describeAccountDeletionFailure,
  emailConfirms,
} from '@pierre/ui-logic';
import { Button, Input, Section, Sheet } from '../../components/ui';
import { userApi } from '../../services/api';
import { useAuth } from '../../contexts/AuthContext';
import { deleteFirebaseAccount } from '../../firebase';

const PREVIEW_QUERY_KEY = ['account-deletion-preview'] as const;

export function AccountDeletionSection(): React.JSX.Element {
  const { t } = useTranslation();
  const { logout } = useAuth();
  const [open, setOpen] = useState(false);
  const [confirmEmail, setConfirmEmail] = useState('');
  const [password, setPassword] = useState('');
  const [failure, setFailure] = useState<string | null>(null);
  const [refusedBlockers, setRefusedBlockers] = useState<AccountDeletionBlocker[]>([]);

  const preview = useQuery({
    queryKey: PREVIEW_QUERY_KEY,
    queryFn: () => userApi.getAccountDeletionPreview(),
    enabled: open,
    staleTime: 0,
  });

  const close = (): void => {
    setOpen(false);
    setConfirmEmail('');
    setPassword('');
    setFailure(null);
    setRefusedBlockers([]);
  };

  const mutation = useMutation({
    mutationFn: () =>
      userApi.deleteAccount({
        confirm_email: confirmEmail,
        password: preview.data?.requires_password ? password : undefined,
      }),
    onSuccess: async () => {
      // The Dravr account is gone; the Google identity behind a Firebase
      // sign-in goes too while Firebase still holds a recent session. The app
      // signs out whatever Firebase answers, since the account no longer
      // exists: a Firebase failure is the identity staying, never a failed
      // delete.
      await deleteFirebaseAccount().catch(() => false);
      await logout();
    },
    onError: (err: unknown) => {
      const described = describeAccountDeletionFailure(err, t);
      setFailure(described.message);
      setRefusedBlockers(described.blockers);
    },
  });

  const data = preview.data;
  const blockers = refusedBlockers.length > 0 ? refusedBlockers : (data?.blockers ?? []);
  const canDelete =
    data !== undefined &&
    blockers.length === 0 &&
    emailConfirms(confirmEmail, data.email) &&
    (!data.requires_password || password.length > 0);

  return (
    <Section
      title={t('accountDeletion.title')}
      description={t('accountDeletion.description')}
      testID="privacy-section-account-deletion"
    >
      <View className="px-4">
        <Button
          title={t('accountDeletion.action')}
          variant="danger"
          onPress={() => setOpen(true)}
          testID="account-deletion-open"
        />
      </View>

      <Sheet visible={open} onClose={close} testID="account-deletion-sheet">
        <View className="gap-4">
          <Text className="text-lg font-semibold text-text-primary">
            {t('accountDeletion.confirmTitle')}
          </Text>

          {preview.isLoading && (
            <Text className="text-sm text-text-secondary">{t('accountDeletion.loading')}</Text>
          )}
          {preview.isError && (
            <Text className="text-sm text-error">{t('accountDeletion.failed')}</Text>
          )}

          {data && blockers.length > 0 && (
            <View className="gap-2" testID="account-deletion-blockers">
              <Text className="text-sm font-semibold text-text-primary">
                {t('accountDeletion.blockedTitle')}
              </Text>
              <Text className="text-sm text-text-secondary">{t('accountDeletion.blockedBody')}</Text>
              {blockers.map((blocker) => (
                <Text
                  key={`${blocker.kind}:${blocker.detail}`}
                  className="text-sm text-text-primary"
                >
                  {`• ${describeAccountDeletionBlocker(blocker, t)}`}
                </Text>
              ))}
            </View>
          )}

          {data && blockers.length === 0 && (
            <View className="gap-4">
              <Text className="text-sm text-text-primary">{t('accountDeletion.confirmBody')}</Text>
              {data.providers.length > 0 && (
                <Text className="text-sm text-text-secondary" testID="account-deletion-providers">
                  {t('accountDeletion.providersLine', { providers: data.providers.join(', ') })}
                </Text>
              )}
              <Input
                label={t('accountDeletion.emailLabel')}
                placeholder={t('accountDeletion.emailHint', { email: data.email })}
                value={confirmEmail}
                onChangeText={setConfirmEmail}
                autoCapitalize="none"
                autoCorrect={false}
                keyboardType="email-address"
                testID="account-deletion-email"
              />
              {data.requires_password && (
                <Input
                  label={t('accountDeletion.passwordLabel')}
                  value={password}
                  onChangeText={setPassword}
                  secureTextEntry
                  showPasswordToggle
                  autoCapitalize="none"
                  testID="account-deletion-password"
                />
              )}
              {failure && (
                <Text className="text-sm text-error" testID="account-deletion-error">
                  {failure}
                </Text>
              )}
              <Button
                title={t('accountDeletion.confirmAction')}
                variant="danger"
                onPress={() => mutation.mutate()}
                disabled={!canDelete}
                loading={mutation.isPending}
                fullWidth
                testID="account-deletion-confirm"
              />
            </View>
          )}

          <Button
            title={t('accountDeletion.cancel')}
            variant="secondary"
            onPress={close}
            fullWidth
            testID="account-deletion-cancel"
          />
        </View>
      </Sheet>
    </Section>
  );
}
