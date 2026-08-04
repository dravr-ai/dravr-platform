// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: OAuth token storage preferring the OS keychain (macOS Keychain, Windows Credential Manager, Linux Secret Service)
// ABOUTME: Falls back to an owner-only (0600) file whose AES layer is obfuscation, not a local-attacker boundary

// NOTE: @napi-rs/keyring is lazy-loaded to prevent D-Bus hangs in Linux CI environments
// Uses Rust-based keyring-rs bindings (replaces deprecated node-keytar)
import {
  readFileSync,
  writeFileSync,
  existsSync,
  unlinkSync,
  chmodSync,
  mkdirSync,
  statSync,
} from 'fs';
import { join, dirname } from 'path';
import { homedir, networkInterfaces } from 'os';
import { createCipheriv, createDecipheriv, randomBytes, createHash } from 'crypto';

const KEYCHAIN_SERVICE = 'pierre-mcp-client';
const KEYCHAIN_ACCOUNT_PREFIX = 'pierre-mcp-tokens';

/** Owner-only mode for the fallback token file. This is what actually protects it. */
const TOKEN_FILE_MODE = 0o600;

/** Owner-only mode applied when the fallback token file's parent directory must be created. */
const TOKEN_DIR_MODE = 0o700;

/**
 * Windows implements no POSIX mode bits: `mode` arguments are ignored there and
 * `chmodSync` toggles only the read-only flag. On that platform the fallback file
 * is protected by the ACLs it inherits from the user profile directory, and the
 * Windows Credential Manager reached through `KeychainTokenStorage` is the
 * properly protected store.
 */
const POSIX_FILE_MODES = process.platform !== 'win32';

/**
 * Token storage interface, implemented by the OS keychain store and by the
 * owner-only file store used when no keychain is available.
 */
export interface SecureTokenStorage {
  /**
   * Persist tokens to the backing store
   */
  saveTokens(tokens: Record<string, any>): Promise<void>;

  /**
   * Retrieve tokens from the backing store
   */
  getTokens(): Promise<Record<string, any> | null>;

  /**
   * Clear all tokens from the backing store
   */
  clearTokens(): Promise<void>;

  /**
   * Migrate tokens from a plaintext file into the backing store
   */
  migrateFromPlaintextFile(filePath: string): Promise<boolean>;
}

/**
 * OS Keychain-based secure storage implementation
 * Uses @napi-rs/keyring Entry API (Rust-based keyring-rs bindings)
 */
export class KeychainTokenStorage implements SecureTokenStorage {
  private log: (message: string, ...args: any[]) => void;
  private entry: any;

  constructor(entry: any, logFunction?: (message: string, ...args: any[]) => void) {
    this.entry = entry;
    this.log = logFunction || ((msg) => console.error(`[SecureStorage] ${msg}`));
  }

  async saveTokens(tokens: Record<string, any>): Promise<void> {
    try {
      const serialized = JSON.stringify(tokens);
      this.entry.setPassword(serialized);
      this.log('Saved tokens to OS keychain');
    } catch (error) {
      this.log(`Failed to save tokens to keychain: ${error}`);
      throw new Error(`Keychain storage failed: ${error}`);
    }
  }

  async getTokens(): Promise<Record<string, any> | null> {
    try {
      const serialized = this.entry.getPassword();

      if (!serialized) {
        this.log('No tokens found in keychain');
        return null;
      }

      const tokens = JSON.parse(serialized);
      this.log('Retrieved tokens from keychain');
      return tokens;
    } catch (error) {
      this.log(`Failed to retrieve tokens from keychain: ${error}`);
      return null;
    }
  }

  async clearTokens(): Promise<void> {
    try {
      this.entry.deletePassword();
      this.log('Cleared tokens from keychain');
    } catch (error) {
      this.log(`Failed to clear tokens from keychain: ${error}`);
      throw new Error(`Keychain clear failed: ${error}`);
    }
  }

  async migrateFromPlaintextFile(filePath: string): Promise<boolean> {
    try {
      if (!existsSync(filePath)) {
        this.log(`No plaintext token file found at ${filePath}, skipping migration`);
        return false;
      }

      this.log(`Migrating tokens from plaintext file: ${filePath}`);

      // Read plaintext tokens
      const plaintextData = readFileSync(filePath, 'utf8');
      const tokens = JSON.parse(plaintextData);

      // Save to keychain
      await this.saveTokens(tokens);

      // Verify migration succeeded by reading back from keychain
      const verified = await this.getTokens();
      if (!verified) {
        this.log('Migration verification failed - keeping plaintext file');
        return false;
      }

      // Delete plaintext file (no plaintext backup left on disk)
      unlinkSync(filePath);
      this.log(`Deleted plaintext token file: ${filePath}`);

      this.log('Successfully migrated tokens to keychain');
      return true;
    } catch (error) {
      this.log(`Failed to migrate tokens: ${error}`);
      // Don't throw - migration failure shouldn't break the app
      return false;
    }
  }
}

/**
 * File-based token storage, used only when the OS keychain is unavailable.
 *
 * The AES-256-GCM layer here is obfuscation, not confidentiality. `deriveEncryptionKey`
 * builds its key from MAC addresses and the home directory path — public machine data
 * that any local process can read — and the ciphertext sits on disk next to everything
 * needed to reproduce that key. It keeps tokens out of casual disk greps and backup
 * archives; it stops nobody who can run code on this machine.
 *
 * What actually protects this store is the file mode: the token file is created 0600
 * and any directory this class creates for it is 0700, so only the owner can read it.
 * `KeychainTokenStorage` is preferred wherever the OS keychain works — this class is
 * the degraded path.
 */
export class EncryptedFileStorage implements SecureTokenStorage {
  private log: (message: string, ...args: any[]) => void;
  private encryptedFilePath: string;
  private encryptionKey: Buffer;

  constructor(logFunction?: (message: string, ...args: any[]) => void) {
    this.log = logFunction || ((msg) => console.error(`[SecureStorage] ${msg}`));
    this.encryptedFilePath = join(homedir(), '.pierre-mcp-tokens.enc');
    this.encryptionKey = this.deriveEncryptionKey();

    if (!POSIX_FILE_MODES) {
      this.log(
        'Windows fallback storage: the token file is protected by the ACLs it inherits from ' +
          'the user profile directory, not by permissions this SDK sets. The Windows Credential ' +
          'Manager is the protected store where it is reachable.'
      );
    }
  }

  /**
   * Derive the AES key from machine-specific data.
   *
   * MAC addresses and the home directory path are stable across runs, which is what lets
   * a stored file be read back on the next start. They are not secret, so this key is
   * reproducible by anything running on the machine — see the class doc for what that
   * means for the guarantees of this store.
   */
  private deriveEncryptionKey(): Buffer {
    const interfaces = networkInterfaces();
    const macAddresses: string[] = [];

    // Collect MAC addresses for machine fingerprint
    for (const name of Object.keys(interfaces)) {
      const iface = interfaces[name];
      if (iface) {
        for (const addr of iface) {
          if (addr.mac && addr.mac !== '00:00:00:00:00:00') {
            macAddresses.push(addr.mac);
          }
        }
      }
    }

    // Combine MAC addresses and homedir for stable machine-specific seed
    const macSeed = macAddresses.sort().join(':') || 'default-seed';
    const homeSeed = homedir();
    const combinedSeed = `${macSeed}:${homeSeed}:pierre-mcp-encryption-v1`;

    // Derive 32-byte key using SHA-256
    return createHash('sha256').update(combinedSeed).digest();
  }

  /**
   * Encrypt data using AES-256-GCM
   */
  private encrypt(data: string): string {
    const iv = randomBytes(12); // 96-bit IV for GCM
    const cipher = createCipheriv('aes-256-gcm', this.encryptionKey, iv);

    let encrypted = cipher.update(data, 'utf8', 'hex');
    encrypted += cipher.final('hex');

    const authTag = cipher.getAuthTag();

    // Format: iv:authTag:encrypted
    return `${iv.toString('hex')}:${authTag.toString('hex')}:${encrypted}`;
  }

  /**
   * Decrypt data using AES-256-GCM
   */
  private decrypt(encryptedData: string): string {
    const parts = encryptedData.split(':');
    if (parts.length !== 3) {
      throw new Error('Invalid encrypted data format');
    }

    const iv = Buffer.from(parts[0], 'hex');
    const authTag = Buffer.from(parts[1], 'hex');
    const encrypted = parts[2];

    const decipher = createDecipheriv('aes-256-gcm', this.encryptionKey, iv);
    decipher.setAuthTag(authTag);

    let decrypted = decipher.update(encrypted, 'hex', 'utf8');
    decrypted += decipher.final('utf8');

    return decrypted;
  }

  /**
   * Create the token file's parent directory owner-only when it is missing.
   *
   * `mkdir` applies the mode itself, so the directory is never briefly group- or
   * world-readable. An existing directory is left alone: the default path's parent is
   * the user's home directory, whose permissions belong to the user and are not this
   * SDK's to rewrite.
   */
  private ensureParentDirectory(): void {
    const parent = dirname(this.encryptedFilePath);
    if (existsSync(parent)) {
      return;
    }

    mkdirSync(parent, { recursive: true, mode: TOKEN_DIR_MODE });
  }

  /**
   * Tighten an existing token file to owner-only before anything is written into it.
   *
   * `writeFileSync`'s `mode` option is honoured only when the file is created, so a
   * file left group- or world-readable by an earlier version keeps that mode through a
   * plain overwrite. Tightening runs before the write rather than after it, so fresh
   * tokens are never written into a file others can still read.
   */
  private restrictFilePermissions(): void {
    if (!POSIX_FILE_MODES || !existsSync(this.encryptedFilePath)) {
      return;
    }

    const currentMode = statSync(this.encryptedFilePath).mode & 0o777;
    if (currentMode === TOKEN_FILE_MODE) {
      return;
    }

    chmodSync(this.encryptedFilePath, TOKEN_FILE_MODE);
    this.log(
      `Tightened token file permissions from ${currentMode.toString(8)} to ${TOKEN_FILE_MODE.toString(8)}`
    );
  }

  async saveTokens(tokens: Record<string, any>): Promise<void> {
    try {
      const serialized = JSON.stringify(tokens);
      const encrypted = this.encrypt(serialized);
      this.ensureParentDirectory();
      this.restrictFilePermissions();
      writeFileSync(this.encryptedFilePath, encrypted, {
        encoding: 'utf8',
        mode: TOKEN_FILE_MODE,
      });
      this.log('Saved tokens to owner-only token file');
    } catch (error) {
      this.log(`Failed to save tokens to encrypted file: ${error}`);
      throw new Error(`Encrypted file storage failed: ${error}`);
    }
  }

  async getTokens(): Promise<Record<string, any> | null> {
    try {
      if (!existsSync(this.encryptedFilePath)) {
        this.log('No encrypted token file found');
        return null;
      }

      try {
        // A file written by an earlier version may still be world-readable, and a session
        // that only ever reads would otherwise never correct it.
        this.restrictFilePermissions();
      } catch (error) {
        // The file is readable but its mode cannot be corrected. Report it and still return
        // the tokens rather than locking the user out of their own session.
        this.log(`Could not tighten token file permissions: ${error}`);
      }

      const encryptedData = readFileSync(this.encryptedFilePath, 'utf8');
      const decrypted = this.decrypt(encryptedData);
      const tokens = JSON.parse(decrypted);

      this.log('Retrieved tokens from encrypted file');
      return tokens;
    } catch (error) {
      this.log(`Failed to retrieve tokens from encrypted file: ${error}`);
      return null;
    }
  }

  async clearTokens(): Promise<void> {
    try {
      if (existsSync(this.encryptedFilePath)) {
        unlinkSync(this.encryptedFilePath);
        this.log('Cleared tokens from encrypted file');
      } else {
        this.log('No encrypted token file found to clear');
      }
    } catch (error) {
      this.log(`Failed to clear tokens from encrypted file: ${error}`);
      throw new Error(`Encrypted file clear failed: ${error}`);
    }
  }

  async migrateFromPlaintextFile(filePath: string): Promise<boolean> {
    try {
      if (!existsSync(filePath)) {
        this.log(`No plaintext token file found at ${filePath}, skipping migration`);
        return false;
      }

      this.log(`Migrating tokens from plaintext file to encrypted storage: ${filePath}`);

      // Read plaintext tokens
      const plaintextData = readFileSync(filePath, 'utf8');
      const tokens = JSON.parse(plaintextData);

      // Save to encrypted file
      await this.saveTokens(tokens);

      // Verify migration succeeded by reading back from encrypted storage
      const verified = await this.getTokens();
      if (!verified) {
        this.log('Migration verification failed - keeping plaintext file');
        return false;
      }

      // Delete plaintext file (no plaintext backup left on disk)
      unlinkSync(filePath);
      this.log(`Deleted plaintext token file: ${filePath}`);

      this.log('Successfully migrated tokens to encrypted file');
      return true;
    } catch (error) {
      this.log(`Failed to migrate tokens: ${error}`);
      return false;
    }
  }
}

/**
 * Factory function to create secure storage with automatic fallback
 * Tries OS keychain first, falls back to encrypted file if unavailable
 */
export async function createSecureStorage(
  logFunction?: (message: string, ...args: any[]) => void
): Promise<SecureTokenStorage> {
  const log = logFunction || ((msg: string) => console.error(`[SecureStorage] ${msg}`));

  // DEBUG: Log environment variable values
  log('[DEBUG] createSecureStorage() called');
  log(`[DEBUG]   process.env.CI = "${process.env.CI}"`);
  log(`[DEBUG]   process.env.GITHUB_ACTIONS = "${process.env.GITHUB_ACTIONS}"`);
  log(`[DEBUG]   CI check: ${process.env.CI === 'true' || process.env.GITHUB_ACTIONS === 'true'}`);

  // KNOWN ISSUE: Keyring hangs on Linux CI due to D-Bus access requirements.
  // Workaround: Use encrypted file storage in CI environments to prevent MCP validator timeout.
  // Background: OS keychains require D-Bus for credential storage on Linux, which is not available in CI containers.
  if (process.env.CI === 'true' || process.env.GITHUB_ACTIONS === 'true') {
    log('CI environment detected - using owner-only file storage (keyring disabled)');
    const encryptedStorage = new EncryptedFileStorage(logFunction);

    // Attempt migration from plaintext file
    const legacyTokenPath = join(homedir(), '.pierre-mcp-tokens.json');
    await encryptedStorage.migrateFromPlaintextFile(legacyTokenPath);

    return encryptedStorage;
  }

  // Try OS keychain first (lazy-load @napi-rs/keyring to avoid D-Bus hangs on Linux CI)
  try {
    log('[DEBUG] Attempting to lazy-load @napi-rs/keyring...');
    // Use dynamic import to avoid loading keyring at module-import time
    const { Entry } = await import('@napi-rs/keyring');
    log('[DEBUG] @napi-rs/keyring loaded successfully');

    // Create entry for pierre-mcp tokens
    const entry = new Entry(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT_PREFIX);
    const keychainStorage = new KeychainTokenStorage(entry, logFunction);

    // Test keychain availability by trying to get tokens
    await keychainStorage.getTokens();

    log('Using OS keychain for secure token storage');

    // Attempt automatic migration from legacy plaintext file
    const legacyTokenPath = join(homedir(), '.pierre-mcp-tokens.json');
    await keychainStorage.migrateFromPlaintextFile(legacyTokenPath);

    return keychainStorage;
  } catch (error) {
    log(`OS keychain unavailable: ${error}`);
    log(
      'Falling back to owner-only file storage - degraded: tokens are protected by file ' +
        'permissions alone, since the file obfuscation key is derivable from machine data'
    );

    // Fallback to encrypted file storage
    const encryptedStorage = new EncryptedFileStorage(logFunction);

    // Attempt migration with fallback storage
    const legacyTokenPath = join(homedir(), '.pierre-mcp-tokens.json');
    await encryptedStorage.migrateFromPlaintextFile(legacyTokenPath);

    return encryptedStorage;
  }
}
