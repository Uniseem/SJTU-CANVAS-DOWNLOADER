import { randomBytes } from 'node:crypto';
import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

import { safeStorage } from 'electron';

const FILE = 'session-key.bin';

/**
 * The key that protects the saved Canvas login. The engine encrypts its
 * cookies with it and only ever receives it in memory; on disk it is kept
 * encrypted by the OS (DPAPI on Windows, the login keychain on macOS) through
 * Electron's safeStorage. Without OS encryption the login is not saved and
 * lasts only until the app quits. SJTU_CANVAS_SESSION_KEY replaces it in
 * tests.
 */
export function loadOrCreateSessionKey(directory: string, log: (line: string) => void): string | null {
  const override = process.env.SJTU_CANVAS_SESSION_KEY?.trim();
  if (override) {
    return override;
  }
  if (!safeStorage.isEncryptionAvailable()) {
    log('OS encryption is unavailable; the login will not be saved');
    return null;
  }
  const path = join(directory, FILE);
  if (existsSync(path)) {
    try {
      const key = safeStorage.decryptString(readFileSync(path)).trim();
      if (isKey(key)) {
        return key;
      }
      log('the stored session key is invalid; creating a new one');
    } catch (error) {
      log(`cannot read the session key (${error instanceof Error ? error.message : error}); creating a new one`);
    }
  }
  const key = randomBytes(32).toString('base64');
  try {
    writeFileSync(path, safeStorage.encryptString(key), { mode: 0o600 });
  } catch (error) {
    log(`cannot store the session key: ${error instanceof Error ? error.message : error}`);
    return null;
  }
  return key;
}

function isKey(value: string): boolean {
  try {
    return Buffer.from(value, 'base64').length === 32;
  } catch {
    return false;
  }
}
