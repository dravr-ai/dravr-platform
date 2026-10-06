// ABOUTME: Self-serve account deletion on the web privacy pane — a destructive action behind a typed-email confirmation
// ABOUTME: Previews what the delete asks for and what blocks it, deletes, then signs the browser out

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState } from 'react';
import { useMutation, useQuery } from '@tanstack/react-query';
import { useTranslation } from '@pierre/i18n';
import type { AccountDeletionBlocker } from '@pierre/shared-types';
import {
  describeAccountDeletionBlocker,
  describeAccountDeletionFailure,
  emailConfirms,
} from '@pierre/ui-logic';
import { userApi } from '../services/api';
import { useAuth } from '../hooks/useAuth';
import { Button, Input, Modal, ModalActions, Section } from './ui';

const PREVIEW_QUERY_KEY = ['account-deletion-preview'] as const;

export default function AccountDeletionSection() {
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

  const close = () => {
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
      // sign-in goes too when Firebase still holds a recent session. The
      // browser signs out whatever Firebase answers, since the account no
      // longer exists: a Firebase failure is the identity staying, never a
      // failed delete.
      await import('../firebase/firebase')
        .then(({ deleteFirebaseAccount }) => deleteFirebaseAccount())
        .catch(() => false);
      logout();
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
      data-testid="account-deletion-section"
    >
      <Button
        variant="danger"
        size="sm"
        onClick={() => setOpen(true)}
        data-testid="account-deletion-open"
      >
        {t('accountDeletion.action')}
      </Button>

      <Modal
        isOpen={open}
        onClose={close}
        title={t('accountDeletion.confirmTitle')}
        footer={
          <ModalActions>
            <Button variant="secondary" size="sm" onClick={close}>
              {t('accountDeletion.cancel')}
            </Button>
            {blockers.length === 0 && (
              <Button
                variant="danger"
                size="sm"
                onClick={() => mutation.mutate()}
                disabled={!canDelete}
                loading={mutation.isPending}
                data-testid="account-deletion-confirm"
              >
                {t('accountDeletion.confirmAction')}
              </Button>
            )}
          </ModalActions>
        }
      >
        {preview.isLoading && (
          <p className="text-sm text-on-surface-variant">{t('accountDeletion.loading')}</p>
        )}
        {preview.isError && (
          <p className="text-sm text-error" role="alert">
            {t('accountDeletion.failed')}
          </p>
        )}
        {data && blockers.length > 0 && (
          <div className="space-y-3" data-testid="account-deletion-blockers">
            <h3 className="text-sm font-medium text-on-surface">{t('accountDeletion.blockedTitle')}</h3>
            <p className="text-sm text-on-surface-variant">{t('accountDeletion.blockedBody')}</p>
            <ul className="space-y-1.5 text-sm text-on-surface">
              {blockers.map((blocker) => (
                <li key={`${blocker.kind}:${blocker.detail}`} className="flex items-start gap-2">
                  <span className="mt-1.5 h-1.5 w-1.5 flex-shrink-0 rounded-full bg-error" aria-hidden="true" />
                  {describeAccountDeletionBlocker(blocker, t)}
                </li>
              ))}
            </ul>
          </div>
        )}
        {data && blockers.length === 0 && (
          <form
            className="space-y-4"
            onSubmit={(event) => {
              event.preventDefault();
              if (canDelete) mutation.mutate();
            }}
          >
            <p className="text-sm text-on-surface">{t('accountDeletion.confirmBody')}</p>
            {data.providers.length > 0 && (
              <p className="text-sm text-on-surface-variant" data-testid="account-deletion-providers">
                {t('accountDeletion.providersLine', { providers: data.providers.join(', ') })}
              </p>
            )}
            <Input
              label={t('accountDeletion.emailLabel')}
              placeholder={t('accountDeletion.emailHint', { email: data.email })}
              type="email"
              autoComplete="off"
              value={confirmEmail}
              onChange={(event) => setConfirmEmail(event.target.value)}
              data-testid="account-deletion-email"
            />
            {data.requires_password && (
              <Input
                label={t('accountDeletion.passwordLabel')}
                type="password"
                autoComplete="current-password"
                value={password}
                onChange={(event) => setPassword(event.target.value)}
                data-testid="account-deletion-password"
              />
            )}
            {failure && (
              <p className="text-sm text-error" role="alert" data-testid="account-deletion-error">
                {failure}
              </p>
            )}
          </form>
        )}
      </Modal>
    </Section>
  );
}
