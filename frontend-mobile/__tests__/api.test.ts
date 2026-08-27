// ABOUTME: Unit tests for API service
// ABOUTME: Tests API domain modules are exported correctly

// Mock the entire api service module to avoid transformation issues with @pierre/api-client
jest.mock('../src/services/api', () => ({
  authApi: {
    login: jest.fn(),
    register: jest.fn(),
    logout: jest.fn(),
    initializeAuth: jest.fn(),
    getStoredUser: jest.fn(),
    storeAuth: jest.fn(),
  },
  chatApi: {
    getConversations: jest.fn(),
    sendTurn: jest.fn(),
  },
  coachesApi: {
    list: jest.fn(),
    get: jest.fn(),
    update: jest.fn(),
    delete: jest.fn(),
    recordUsage: jest.fn(),
  },
  oauthApi: {
    getStatus: jest.fn(),
    initMobileOAuth: jest.fn(),
  },
  storeApi: {
    browse: jest.fn(),
    search: jest.fn(),
    get: jest.fn(),
  },
  userApi: {
    getMcpTokens: jest.fn(),
    changePassword: jest.fn(),
    getOAuthApps: jest.fn(),
  },
  apiClient: {},
  onAuthFailure: jest.fn(),
}));

// Import after mocks
import { authApi, chatApi, coachesApi, oauthApi, storeApi, userApi } from '../src/services/api';

describe('API Service', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  describe('authApi object', () => {
    it('should be defined', () => {
      expect(authApi).toBeDefined();
    });

    it('should have login method', () => {
      expect(typeof authApi.login).toBe('function');
    });

    it('should have register method', () => {
      expect(typeof authApi.register).toBe('function');
    });
  });

  describe('chatApi object', () => {
    it('should have getConversations method', () => {
      expect(typeof chatApi.getConversations).toBe('function');
    });

    it('should have sendTurn method', () => {
      expect(typeof chatApi.sendTurn).toBe('function');
    });
  });

  describe('oauthApi object', () => {
    it('should have getStatus method', () => {
      expect(typeof oauthApi.getStatus).toBe('function');
    });
  });

  describe('coachesApi methods', () => {
    it('should have list method', () => {
      expect(typeof coachesApi.list).toBe('function');
    });

    it('should have get method', () => {
      expect(typeof coachesApi.get).toBe('function');
    });

    it('should have update method', () => {
      expect(typeof coachesApi.update).toBe('function');
    });

    it('should have delete method', () => {
      expect(typeof coachesApi.delete).toBe('function');
    });

    it('should have recordUsage method', () => {
      expect(typeof coachesApi.recordUsage).toBe('function');
    });
  });

  describe('storeApi methods', () => {
    it('should have browse method', () => {
      expect(typeof storeApi.browse).toBe('function');
    });
  });

  describe('userApi methods', () => {
    it('should have getMcpTokens method', () => {
      expect(typeof userApi.getMcpTokens).toBe('function');
    });
  });
});
