// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for admin and dashboard domain APIs
// ABOUTME: Verifies admin tokens, user management, pre-approved emails, and error propagation

import { describe, it, expect, beforeEach, vi, afterEach } from 'vitest';
import { AxiosError } from 'axios';
import { adminApi, dashboardApi } from '../api/index';

// vi.hoisted runs before vi.mock hoisting, so this variable is available in the factory
const { mockAxiosInstance } = vi.hoisted(() => ({
  mockAxiosInstance: {
    get: vi.fn(),
    post: vi.fn(),
    put: vi.fn(),
    delete: vi.fn(),
    interceptors: {
      request: { use: vi.fn() },
      response: { use: vi.fn() },
    },
  },
}));

// Mock @pierre/api-client to return our mock axios instance
vi.mock('@pierre/api-client', () => ({
  createPierreApi: vi.fn(() => ({
    auth: { login: vi.fn() },
    chat: { getConversations: vi.fn() },
    coaches: { list: vi.fn() },
    oauth: {
      getStatus: vi.fn(),
      getProvidersStatus: vi.fn(),
      disconnectProvider: vi.fn(),
      linkIntervalsIcu: vi.fn(),
      disconnectIntervalsIcu: vi.fn(),
    },
    store: { browse: vi.fn() },
    user: { getStats: vi.fn(), getLlmSettings: vi.fn(), saveLlmCredentials: vi.fn(), validateLlmCredentials: vi.fn(), deleteLlmCredentials: vi.fn() },
    notifications: { getNotifications: vi.fn() },
    axios: mockAxiosInstance,
    adapter: {
      authStorage: {
        setCsrfToken: vi.fn(), getCsrfToken: vi.fn(),
        setUser: vi.fn(), getUser: vi.fn(), clear: vi.fn(),
        getToken: vi.fn(), setToken: vi.fn(), removeToken: vi.fn(),
        getRefreshToken: vi.fn(), setRefreshToken: vi.fn(),
      },
      httpConfig: { baseURL: '' },
      authFailure: { onAuthFailure: vi.fn() },
    },
  })),
}));

vi.mock('@pierre/api-client/adapters/web', () => ({
  createWebAdapter: vi.fn(() => ({
    authStorage: {
      setCsrfToken: vi.fn(), getCsrfToken: vi.fn(),
      setUser: vi.fn(), getUser: vi.fn(), clear: vi.fn(),
      getToken: vi.fn(), setToken: vi.fn(), removeToken: vi.fn(),
      getRefreshToken: vi.fn(), setRefreshToken: vi.fn(),
    },
    httpConfig: { baseURL: '' },
    authFailure: { onAuthFailure: vi.fn() },
  })),
}));

// Mock axios globally (domain modules import pierreApi.axios via client.ts)
vi.mock('axios', () => ({
  default: {
    create: vi.fn(() => mockAxiosInstance),
    defaults: {
      baseURL: '',
      withCredentials: true,
      headers: { common: {} },
    },
  },
  AxiosError: class extends Error {
    constructor(message: string, code?: string, _config?: unknown, _request?: unknown, response?: unknown) {
      super(message);
      this.code = code;
      this.config = _config;
      this.request = _request;
      this.response = response;
    }
    code?: string;
    config?: unknown;
    request?: unknown;
    response?: unknown;
    isAxiosError = true;
  },
}));

// Mock window events
Object.defineProperty(window, 'dispatchEvent', { value: vi.fn() });

describe('API Service - Admin Functionality', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  describe('Dashboard Analytics Endpoints', () => {
    it('should fetch usage analytics with correct parameters', async () => {
      const mockAnalytics = {
        time_series: [
          { date: '2025-01-01', request_count: 145 },
          { date: '2025-01-02', request_count: 203 }
        ]
      };

      mockAxiosInstance.get.mockResolvedValueOnce({ data: mockAnalytics });

      const result = await dashboardApi.getUsageAnalytics(7);

      expect(mockAxiosInstance.get).toHaveBeenCalledWith('/api/dashboard/analytics?days=7');
      expect(result).toEqual(mockAnalytics);
    });
  });

  describe('Admin Token Management', () => {
    it('should fetch admin tokens with parameters from the data envelope', async () => {
      const listing = {
        count: 1,
        tokens: [
          {
            id: 'token-1',
            service_name: 'Test Service',
            permissions: ['list_keys'],
            is_super_admin: false,
            is_active: true,
            created_at: '2025-01-01T00:00:00Z',
            usage_count: 0,
            token_prefix: 'at_abc123',
          },
        ],
      };

      mockAxiosInstance.get.mockResolvedValueOnce({
        data: { success: true, message: 'Retrieved 1 admin tokens', data: listing },
      });

      const result = await adminApi.getAdminTokens({ include_inactive: true });

      expect(mockAxiosInstance.get).toHaveBeenCalledWith('/api/admin/tokens?include_inactive=true');
      expect(result).toEqual(listing);
    });

    it('should create admin token successfully', async () => {
      const tokenRequest = {
        service_name: 'New Service',
        service_description: 'Test service',
        permissions: ['list_keys'],
        is_super_admin: false,
        expires_in_days: 90
      };

      const created = {
        token_id: 'token-2',
        service_name: 'New Service',
        jwt_token: 'eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9...',
        token_prefix: 'at_def456',
        is_super_admin: false,
        expires_at: '2025-04-01T00:00:00Z',
      };

      mockAxiosInstance.post.mockResolvedValueOnce({
        data: { success: true, message: 'Admin token created successfully', data: created },
      });

      const result = await adminApi.createAdminToken(tokenRequest);

      expect(mockAxiosInstance.post).toHaveBeenCalledWith('/api/admin/tokens', tokenRequest);
      expect(result).toEqual(created);
    });

    it('should revoke admin token', async () => {
      const mockResponse = { success: true, message: 'Token revoked' };

      mockAxiosInstance.post.mockResolvedValueOnce({ data: mockResponse });

      const result = await adminApi.revokeAdminToken('token-1');

      expect(mockAxiosInstance.post).toHaveBeenCalledWith('/api/admin/tokens/token-1/revoke');
      expect(result).toEqual(mockResponse);
    });

    it('should rotate admin token and return the replacement', async () => {
      const rotateRequest = { expires_in_days: 180 };
      const rotated = {
        old_token_id: 'token-1',
        token_id: 'token-9',
        service_name: 'Test Service',
        jwt_token: 'new.jwt.token',
        token_prefix: 'at_new123',
        is_super_admin: false,
        expires_at: null,
      };

      mockAxiosInstance.post.mockResolvedValueOnce({
        data: { success: true, message: 'Admin token rotated successfully', data: rotated },
      });

      const result = await adminApi.rotateAdminToken('token-1', rotateRequest);

      expect(mockAxiosInstance.post).toHaveBeenCalledWith('/api/admin/tokens/token-1/rotate', rotateRequest);
      expect(result.jwt_token).toBe('new.jwt.token');
      expect(result.old_token_id).toBe('token-1');
    });
  });

  describe('User Management', () => {
    it('should fetch pending users', async () => {
      const mockPendingUsers = [
        { id: 'user-1', email: 'user1@example.com', display_name: 'User One', created_at: '2025-01-01T00:00:00Z' },
        { id: 'user-2', email: 'user2@example.com', display_name: 'User Two', created_at: '2025-01-02T00:00:00Z' }
      ];

      // The admin handler answers { success, message, data: { count, users } }
      mockAxiosInstance.get.mockResolvedValueOnce({
        data: { success: true, message: 'Retrieved 2 pending users', data: { count: 2, users: mockPendingUsers } },
      });

      const result = await adminApi.getPendingUsers();

      expect(mockAxiosInstance.get).toHaveBeenCalledWith('/api/admin/pending-users');
      // getPendingUsers extracts the users array
      expect(result).toEqual(mockPendingUsers);
    });

    it('should approve user with reason', async () => {
      const mockResponse = { success: true, message: 'User approved' };

      mockAxiosInstance.post.mockResolvedValueOnce({ data: mockResponse });

      const result = await adminApi.approveUser('user-1', 'Valid business use case');

      expect(mockAxiosInstance.post).toHaveBeenCalledWith('/api/admin/approve-user/user-1', {
        reason: 'Valid business use case'
      });
      expect(result).toEqual(mockResponse);
    });

    it('should suspend user with reason', async () => {
      const mockResponse = { success: true, message: 'User suspended' };

      mockAxiosInstance.post.mockResolvedValueOnce({ data: mockResponse });

      const result = await adminApi.suspendUser('user-1', 'Policy violation');

      expect(mockAxiosInstance.post).toHaveBeenCalledWith('/api/admin/suspend-user/user-1', {
        reason: 'Policy violation'
      });
      expect(result).toEqual(mockResponse);
    });

    it('should page every status through the listing cursor', async () => {
      const first = [
        { id: 'user-1', email: 'user1@example.com', user_status: 'active' },
        { id: 'user-2', email: 'user2@example.com', user_status: 'pending' },
      ];
      const second = [{ id: 'user-3', email: 'user3@example.com', user_status: 'suspended' }];

      mockAxiosInstance.get
        .mockResolvedValueOnce({
          data: { success: true, data: { users: first, total: 2, has_more: true, next_cursor: 'c1' } },
        })
        .mockResolvedValueOnce({
          data: { success: true, data: { users: second, total: 1, has_more: false } },
        });

      const result = await adminApi.getAllUsers();

      expect(mockAxiosInstance.get).toHaveBeenNthCalledWith(1, '/api/admin/users?status=all&limit=100');
      expect(mockAxiosInstance.get).toHaveBeenNthCalledWith(
        2,
        '/api/admin/users?status=all&limit=100&cursor=c1'
      );
      expect(result).toEqual([...first, ...second]);
    });
  });

  describe('Password reset', () => {
    it('should return the issued token from the data envelope', async () => {
      const issued = {
        user_id: 'user-1',
        email: 'target@example.com',
        reset_token: 'sel.verifier',
        expires_in_seconds: 3600,
        reset_by: 'admin-1',
        note: 'Deliver this token to the user.',
      };
      mockAxiosInstance.post.mockResolvedValueOnce({
        data: { success: true, message: 'Password reset token issued', data: issued },
      });

      const result = await adminApi.resetUserPassword('user-1');

      expect(mockAxiosInstance.post).toHaveBeenCalledWith('/api/admin/users/user-1/reset-password');
      expect(result).toEqual(issued);
    });
  });

  describe('Pre-approved emails', () => {
    it('should list pre-approved emails from the data envelope', async () => {
      const entries = [
        {
          email: 'alpha@example.com',
          note: 'alpha cohort',
          created_at: '2026-08-26T12:00:00Z',
          allowed_by: 'operator-1',
          allowed_by_email: 'admin@example.com',
          account_status: null,
        },
      ];

      mockAxiosInstance.get.mockResolvedValueOnce({
        data: { success: true, message: '1 pre-approved email(s)', data: { emails: entries, total: 1 } },
      });

      const result = await adminApi.getPreApprovedEmails();

      expect(mockAxiosInstance.get).toHaveBeenCalledWith('/api/admin/pre-approved-emails');
      expect(result).toEqual(entries);
    });

    it('should allow an email with a note and surface the outcome', async () => {
      mockAxiosInstance.post.mockResolvedValueOnce({
        data: {
          success: true,
          message: 'alpha@example.com pre-approved',
          data: { email: 'alpha@example.com', outcome: 'recorded', approved_user_id: null },
        },
      });

      const result = await adminApi.allowEmail('alpha@example.com', 'alpha cohort');

      expect(mockAxiosInstance.post).toHaveBeenCalledWith('/api/admin/pre-approved-emails', {
        email: 'alpha@example.com',
        note: 'alpha cohort',
        send_invite: false,
      });
      expect(result.outcome).toBe('recorded');
      expect(result.message).toBe('alpha@example.com pre-approved');
      expect(result.approved_user_id).toBeNull();
    });

    it('should report the promoted account when the allow approved a pending user', async () => {
      mockAxiosInstance.post.mockResolvedValueOnce({
        data: {
          success: true,
          message: 'queued@example.com had a pending account — approved now (status: active)',
          data: { email: 'queued@example.com', outcome: 'pending_approved', approved_user_id: 'user-9' },
        },
      });

      const result = await adminApi.allowEmail('queued@example.com');

      expect(mockAxiosInstance.post).toHaveBeenCalledWith('/api/admin/pre-approved-emails', {
        email: 'queued@example.com',
        note: null,
        send_invite: false,
      });
      expect(result.outcome).toBe('pending_approved');
      expect(result.approved_user_id).toBe('user-9');
    });

    it('should ask for the sign-up link and report that it was mailed', async () => {
      mockAxiosInstance.post.mockResolvedValueOnce({
        data: {
          success: true,
          message: 'new@example.com pre-approved',
          data: { email: 'new@example.com', outcome: 'recorded', approved_user_id: null, invited: true },
        },
      });

      const result = await adminApi.allowEmail('new@example.com', undefined, true);

      expect(mockAxiosInstance.post).toHaveBeenCalledWith('/api/admin/pre-approved-emails', {
        email: 'new@example.com',
        note: null,
        send_invite: true,
      });
      expect(result.invited).toBe(true);
    });

    it('should url-encode the address when removing a pre-approval', async () => {
      mockAxiosInstance.delete.mockResolvedValueOnce({
        data: {
          success: true,
          message: 'user+tag@example.com removed from the pre-approved list',
          data: { email: 'user+tag@example.com', removed: true, account_status: null },
        },
      });

      const result = await adminApi.disallowEmail('user+tag@example.com');

      expect(mockAxiosInstance.delete).toHaveBeenCalledWith(
        '/api/admin/pre-approved-emails/user%2Btag%40example.com'
      );
      expect(result.removed).toBe(true);
    });
  });

  describe('Error Handling', () => {
    it('should handle 404 errors appropriately', async () => {
      const error = new AxiosError('Not Found', '404', undefined, undefined, {
        status: 404,
        data: { error: 'Coach not found' }
      } as never);

      mockAxiosInstance.get.mockRejectedValueOnce(error);

      await expect(adminApi.getSystemCoach('missing-coach')).rejects.toThrow('Not Found');
    });

    it('should handle network errors', async () => {
      const error = new AxiosError('Network Error', 'NETWORK_ERROR');

      mockAxiosInstance.get.mockRejectedValueOnce(error);

      await expect(dashboardApi.getUsageAnalytics()).rejects.toThrow('Network Error');
    });

    it('should handle 403 unauthorized errors', async () => {
      const error = new AxiosError('Forbidden', '403', undefined, undefined, {
        status: 403,
        data: { error: 'Insufficient permissions' }
      } as never);

      mockAxiosInstance.post.mockRejectedValueOnce(error);

      await expect(adminApi.createAdminToken({
        service_name: 'Test',
        permissions: ['super_admin']
      })).rejects.toThrow('Forbidden');
    });
  });
});
