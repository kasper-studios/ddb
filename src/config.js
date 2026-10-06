/**
 * DDB Host Configuration & Permissions Manager
 * Manages config.json, permissions matrix, and tunnels
 */

import { readFile, writeFile } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import crypto from 'node:crypto';

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
const CONFIG_PATH = join(__dirname, '..', 'config.json');

export const DEFAULT_CONFIG = {
  host_name: 'ponosik',
  api_key: '',
  cloud_url: 'wss://api.kasperstudios.xyz/ddb/ws',
  tunnel_type: 'lan', // 'cloud' | 'cloudflare' | 'ngrok' | 'lan'
  tunnel_token: '',
  tunnel_url: '',
  permissions: {
    display_control: {
      enabled: true,
      allow_click: true,
      allow_type: true,
      allow_keys: true,
    },
    screen: true,
    audio: true,
    fs: {
      read: true,
      write: false,
      list: true,
    },
    exec: true,
    hermes: true,
  },
};

let cachedConfig = null;

export async function loadConfig() {
  if (cachedConfig) return cachedConfig;

  if (existsSync(CONFIG_PATH)) {
    try {
      const raw = await readFile(CONFIG_PATH, 'utf-8');
      cachedConfig = { ...DEFAULT_CONFIG, ...JSON.parse(raw) };
      return cachedConfig;
    } catch {
      // fallback
    }
  }

  // Create initial config
  const initial = { ...DEFAULT_CONFIG };
  if (!initial.api_key) {
    initial.api_key = 'ddb_' + crypto.randomBytes(16).toString('hex');
  }
  await saveConfig(initial);
  cachedConfig = initial;
  return cachedConfig;
}

export async function saveConfig(cfg) {
  cachedConfig = cfg;
  await writeFile(CONFIG_PATH, JSON.stringify(cfg, null, 2), 'utf-8');
  return cachedConfig;
}

/**
 * Check if a specific action is permitted by the host configuration
 */
export async function isActionPermitted(actionName, params = {}) {
  const cfg = await loadConfig();
  const perms = cfg.permissions || {};

  if (actionName === 'exec') {
    return perms.exec !== false;
  }

  if (actionName === 'screen') {
    return perms.screen !== false;
  }

  if (actionName.startsWith('hermes.')) {
    return perms.hermes !== false;
  }

  if (actionName.startsWith('fs.')) {
    const fsPerms = perms.fs;
    if (fsPerms === false) return false;
    if (typeof fsPerms === 'object') {
      if (actionName === 'fs.read' && fsPerms.read === false) return false;
      if (actionName === 'fs.write' && fsPerms.write === false) return false;
      if (actionName === 'fs.list' && fsPerms.list === false) return false;
    }
    return true;
  }

  if (actionName === 'display_control') {
    const dc = perms.display_control;
    if (dc === false) return false;
    if (typeof dc === 'object') {
      const subAction = params.action;
      if (subAction === 'click' && dc.allow_click === false) return false;
      if (subAction === 'type' && dc.allow_type === false) return false;
      if (subAction === 'key' && dc.allow_keys === false) return false;
    }
    return true;
  }

  if (actionName === 'sound_steal' || actionName === 'audio.play') {
    return perms.audio !== false;
  }

  return true;
}
