// ABOUTME: Authentication context types and creation
// ABOUTME: Defines User type with role/status and AuthContext for app-wide auth state
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { createContext } from 'react';
import type { User, UserRole, UserStatus, FirebaseLoginResponse } from '@pierre/shared-types';
import type { SignInFailure } from '@pierre/ui-logic';

interface ImpersonationState {
  isImpersonating: boolean;
  targetUser: {
    id: string;
    email: string;
    display_name?: string;
    role: string;
  } | null;
  sessionId: string | null;
  originalUser: User | null;
}

interface AuthContextType {
  user: User | null;
  token: string | null;
  isAuthenticated: boolean;
  isLoading: boolean;
  loading: boolean; // For test compatibility
  /**
   * Open the server's hosted sign-in (carnet#787). Resolves only if the page
   * could not be left; on success the browser is already navigating away.
   */
  startSignIn: () => Promise<void>;
  /** Why the last hosted sign-in returned without a session, if it did. */
  signInFailure: SignInFailure | null;
  clearSignInFailure: () => void;
  loginWithFirebase: (idToken: string) => Promise<FirebaseLoginResponse>;
  logout: () => void;
  impersonation: ImpersonationState;
  startImpersonation: (targetUserId: string, reason?: string) => Promise<void>;
  endImpersonation: () => Promise<void>;
}

export const AuthContext = createContext<AuthContextType | undefined>(undefined);
export type { User, AuthContextType, UserRole, UserStatus, ImpersonationState };
