// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Modal dialog for joining a coaching group via invite code
// ABOUTME: Includes invite code input, consent checkbox, and error handling for invalid codes

import { useState } from 'react';
import { useJoinGroup } from '../../hooks/useGroups';
import { Modal, ModalActions, Button, Input, useErrorToast, useSuccessToast } from '../ui';

interface JoinGroupModalProps {
  isOpen: boolean;
  onClose: () => void;
}

export default function JoinGroupModal({ isOpen, onClose }: JoinGroupModalProps) {
  const { joinGroup, isPending } = useJoinGroup();
  const showError = useErrorToast();
  const showSuccess = useSuccessToast();

  const [inviteCode, setInviteCode] = useState('');
  const [consentGiven, setConsentGiven] = useState(false);
  const [fieldError, setFieldError] = useState('');

  const resetForm = () => {
    setInviteCode('');
    setConsentGiven(false);
    setFieldError('');
  };

  const handleClose = () => {
    resetForm();
    onClose();
  };

  const handleSubmit = async () => {
    const code = inviteCode.trim();
    if (!code) {
      setFieldError('Please enter an invite code.');
      return;
    }
    if (!consentGiven) {
      setFieldError('You must agree to share your data with the group coach.');
      return;
    }
    setFieldError('');

    try {
      await joinGroup({ invite_code: code });
      showSuccess('Joined group', 'You are now a member of the group.');
      handleClose();
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Invalid or expired invite code.';
      setFieldError(message);
      showError('Join failed', message);
    }
  };

  const isFormValid = inviteCode.trim().length > 0 && consentGiven;

  return (
    <Modal
      isOpen={isOpen}
      onClose={handleClose}
      title="Join a Group"
      size="sm"
      footer={
        <ModalActions>
          <Button variant="secondary" onClick={handleClose}>
            Cancel
          </Button>
          <Button
            variant="primary"
            onClick={handleSubmit}
            loading={isPending}
            disabled={!isFormValid}
          >
            Join Group
          </Button>
        </ModalActions>
      }
    >
      <div className="space-y-5">
        <p className="text-sm text-zinc-400">
          Enter the invite code you received to join a coaching group.
        </p>

        <Input
          label="Invite Code"
          variant="dark"
          placeholder="e.g., abc123"
          value={inviteCode}
          onChange={(e) => {
            setInviteCode(e.target.value);
            setFieldError('');
          }}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && isFormValid) {
              handleSubmit();
            }
          }}
          error={fieldError}
          autoFocus
        />

        <label className="flex items-start gap-3 cursor-pointer group">
          <input
            type="checkbox"
            checked={consentGiven}
            onChange={(e) => {
              setConsentGiven(e.target.checked);
              setFieldError('');
            }}
            className="mt-0.5 w-4 h-4 rounded border-white/20 bg-white/5 text-pierre-violet focus:ring-pierre-violet focus:ring-offset-0"
          />
          <span className="text-sm text-zinc-300 group-hover:text-white transition-colors">
            I agree to share my training data with the group coach for personalized coaching.
            Peer data sharing depends on group settings and your consent.
          </span>
        </label>
      </div>
    </Modal>
  );
}
