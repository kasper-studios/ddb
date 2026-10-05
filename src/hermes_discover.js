/**
 * DDB — Hermes Desktop Auto-Discovery
 *
 * Scans /proc/ for a running `hermes serve` process, extracts
 * HERMES_DASHBOARD_SESSION_TOKEN from its environ, and resolves
 * the listening port via /proc/<pid>/net/tcp (falls back to `ss`).
 *
 * Zero external dependencies — Node.js stdlib only.
 */

import { readdir, readFile } from 'node:fs/promises';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';

const execFileAsync = promisify(execFile);

// ── internal helpers ────────────────────────────────────────────

/**
 * Read /proc/<pid>/cmdline and return the command string.
 * cmdline is NUL-delimited; we join with spaces.
 */
async function readCmdline(pid) {
  try {
    const raw = await readFile(`/proc/${pid}/cmdline`, 'utf-8');
    return raw.split('\0').join(' ').trim();
  } catch {
    return '';
  }
}

/**
 * Read /proc/<pid>/environ (NUL-separated KEY=VAL pairs)
 * and return the value for `varName`, or null.
 */
async function readEnvVar(pid, varName) {
  try {
    const raw = await readFile(`/proc/${pid}/environ`, 'utf-8');
    const entries = raw.split('\0');
    const prefix = `${varName}=`;
    for (const entry of entries) {
      if (entry.startsWith(prefix)) return entry.slice(prefix.length);
    }
  } catch {
    // permission denied or process gone — swallow
  }
  return null;
}

/**
 * Try to find the TCP port Hermes is listening on.
 * Strategy 1 (primary): shell out to `ss -tlnp` and grep for the exact pid
 * Strategy 2 (fallback): parse /proc/<pid>/net/tcp (less reliable — shows namespace-wide)
 */
async function resolvePort(pid) {
  const SKIP_PORTS = new Set([8644, 8790]); // webhook listener, ddb shell
  
  // Strategy 1 — ss (most reliable: shows actual owning pid)
  try {
    const { stdout } = await execFileAsync('ss', ['-tlnp'], { timeout: 3000 });
    const candidates = [];
    for (const line of stdout.split('\n')) {
      // Match exact pid (pid=NNNNN, or pid=NNNNN,)
      if (!line.includes(`pid=${pid},`) && !line.includes(`pid=${pid})`)) continue;
      const m = line.match(/:(\d+)\s/);
      if (m) {
        const port = parseInt(m[1], 10);
        if (!SKIP_PORTS.has(port)) candidates.push(port);
      }
    }
    if (candidates.length > 0) {
      // Prefer highest port (ephemeral — hermes serve uses --port 0)
      candidates.sort((a, b) => b - a);
      return candidates[0];
    }
  } catch {
    // ss not available — fall through
  }
  
  // Strategy 2 — /proc net/tcp (fallback, namespace-wide so less precise)
  try {
    const raw = await readFile(`/proc/${pid}/net/tcp`, 'utf-8');
    const lines = raw.split('\n').slice(1);
    const candidates = [];
    for (const line of lines) {
      const cols = line.trim().split(/\s+/);
      if (!cols[1]) continue;
      if (cols[3] !== '0A') continue; // LISTEN only
      const [hexIp, hexPort] = cols[1].split(':');
      const port = parseInt(hexPort, 16);
      if (port <= 0) continue;
      if (hexIp !== '0100007F' && hexIp !== '00000000') continue;
      if (SKIP_PORTS.has(port)) continue;
      candidates.push(port);
    }
    if (candidates.length > 0) {
      candidates.sort((a, b) => b - a);
      return candidates[0];
    }
  } catch {
    // not available
  }

  return null;
}

// ── main discovery ──────────────────────────────────────────────

/** @returns {{ found: boolean, pid?: number, port?: number, token?: string }} */
export async function discoverHermes() {
  try {
    const entries = await readdir('/proc');
    const pidDirs = entries.filter(e => /^\d+$/.test(e)).map(Number);

    for (const pid of pidDirs) {
      const cmdline = await readCmdline(pid);
      if (!cmdline) continue;

      // Match both `hermes serve` and variants like `hermes-agent serve`
      const lc = cmdline.toLowerCase();
      if (!(lc.includes('hermes') && lc.includes('serve'))) continue;

      const token = await readEnvVar(pid, 'HERMES_DASHBOARD_SESSION_TOKEN');
      const port  = await resolvePort(pid);

      return { found: true, pid, port, token };
    }
  } catch (err) {
    // /proc not readable (non-Linux?) — that's fine
  }

  return { found: false };
}

// ── periodic re-verifier ────────────────────────────────────────

let _hermesState = { found: false };
let _verifyTimer = null;

/**
 * Start periodic re-verification of a discovered Hermes instance.
 * Returns the initial state and keeps updating the internal ref.
 */
export async function startHermesWatch(intervalMs = 60_000) {
  _hermesState = await discoverHermes();

  if (_verifyTimer) clearInterval(_verifyTimer);
  _verifyTimer = setInterval(async () => {
    const fresh = await discoverHermes();
    if (_hermesState.found && !fresh.found) {
      console.warn('[hermes-discover] Hermes process gone — marking not found');
    } else if (!_hermesState.found && fresh.found) {
      console.log(`[hermes-discover] Hermes appeared — pid=${fresh.pid} port=${fresh.port}`);
    }
    _hermesState = fresh;
  }, intervalMs);

  // Don't keep Node alive just for this timer
  if (_verifyTimer.unref) _verifyTimer.unref();

  return _hermesState;
}

/** Get the latest cached Hermes state (non-async). */
export function getHermesState() {
  return _hermesState;
}
