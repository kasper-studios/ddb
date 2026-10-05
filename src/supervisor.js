/**
 * DDB Shell Supervisor v1.0.0
 * Level 1 crash-shield for the Dirom Device Bridge.
 *
 * - Spawns and watchdogs the DDB Main Bridge worker (node src/index.js)
 * - Exposes a JSON HTTP API on :8790 for health, exec, restart, logs
 * - Collects bridge stdout/stderr into a ring buffer
 * - Records crash blackbox (last 20 crashes)
 * - Graceful shutdown on SIGTERM/SIGINT
 */

import { createServer } from 'node:http';
import { exec, spawn } from 'node:child_process';
import { platform, arch, hostname, freemem, totalmem, cpus } from 'node:os';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const VERSION = '1.0.0';
const PORT = parseInt(process.env.DDB_SHELL_PORT, 10) || 8790;
const RESPAWN_DELAY_MS = 3000;
const LOG_RING_SIZE = 500;
const LOG_SERVE_SIZE = 100;
const BLACKBOX_SIZE = 20;
const EXEC_TIMEOUT_DEFAULT = 30_000;
const EXEC_TIMEOUT_MAX = 120_000;

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
const PROJECT_ROOT = join(__dirname, '..');
const BRIDGE_ENTRY = join(__dirname, 'index.js');

// ── State ────────────────────────────────────────────────────────────────────

const startTime = Date.now();
let bridge = null;
let bridgeAlive = false;
let bridgePid = null;
let bridgeRestarts = 0;
let bridgeStartedAt = null;
let lastCrash = null;
let respawnTimer = null;
let shuttingDown = false;

let bridgePort = null;

/** Ring buffer for bridge log lines */
const logRing = [];

/** Blackbox of recent crashes */
const blackbox = [];

// ── Ring buffer helpers ──────────────────────────────────────────────────────

function pushLog(line) {
  logRing.push(line);
  if (logRing.length > LOG_RING_SIZE) {
    logRing.splice(0, logRing.length - LOG_RING_SIZE);
  }
}

function pushCrash(entry) {
  blackbox.push(entry);
  if (blackbox.length > BLACKBOX_SIZE) {
    blackbox.splice(0, blackbox.length - BLACKBOX_SIZE);
  }
}

// ── Bridge lifecycle ─────────────────────────────────────────────────────────

function spawnBridge() {
  if (shuttingDown) return;

  const env = {
    ...process.env,
    DDB_SUPERVISOR_PORT: String(PORT),
  };

  bridge = spawn(process.execPath, [BRIDGE_ENTRY], {
    cwd: PROJECT_ROOT,
    env,
    stdio: ['ignore', 'pipe', 'pipe'],
  });

  bridgeAlive = true;
  bridgePid = bridge.pid;
  bridgeStartedAt = Date.now();

  const ts = new Date().toISOString();
  pushLog(`[${ts}] [supervisor] Bridge spawned (pid=${bridge.pid})`);

  bridge.stdout.on('data', (chunk) => {
    const lines = chunk.toString().split('\n');
    for (const line of lines) {
      if (line.length > 0) pushLog(line);
    }
  });

  bridge.stderr.on('data', (chunk) => {
    const lines = chunk.toString().split('\n');
    for (const line of lines) {
      if (line.length > 0) pushLog(`[stderr] ${line}`);
    }
  });

  bridge.on('exit', (code, signal) => {
    const uptimeS = bridgeStartedAt ? Math.round((Date.now() - bridgeStartedAt) / 1000) : 0;
    const crashEntry = {
      timestamp: new Date().toISOString(),
      code,
      signal,
      uptime_s: uptimeS,
    };

    lastCrash = crashEntry;
    pushCrash(crashEntry);
    pushLog(`[${crashEntry.timestamp}] [supervisor] Bridge exited (code=${code}, signal=${signal}, uptime=${uptimeS}s)`);

    bridgeAlive = false;
    bridgePid = null;
    bridge = null;

    if (!shuttingDown) {
      bridgeRestarts++;
      pushLog(`[${new Date().toISOString()}] [supervisor] Respawning in ${RESPAWN_DELAY_MS}ms (restart #${bridgeRestarts})`);
      respawnTimer = setTimeout(spawnBridge, RESPAWN_DELAY_MS);
    }
  });

  bridge.on('error', (err) => {
    pushLog(`[${new Date().toISOString()}] [supervisor] Bridge spawn error: ${err.message}`);
  });
}

function killBridge() {
  if (bridge && bridgeAlive) {
    try {
      bridge.kill('SIGTERM');
    } catch { /* already dead */ }
  }
}

// ── HTTP helpers ─────────────────────────────────────────────────────────────

function readBody(req) {
  return new Promise((resolve, reject) => {
    const chunks = [];
    req.on('data', (c) => chunks.push(c));
    req.on('end', () => {
      try {
        const raw = Buffer.concat(chunks).toString();
        resolve(raw.length > 0 ? JSON.parse(raw) : {});
      } catch (e) {
        reject(new Error('Invalid JSON body'));
      }
    });
    req.on('error', reject);
  });
}

function json(res, status, data) {
  const body = JSON.stringify(data);
  res.writeHead(status, {
    'Content-Type': 'application/json',
    'Content-Length': Buffer.byteLength(body),
  });
  res.end(body);
}

// ── Route handlers ───────────────────────────────────────────────────────────

function handlePing(_req, res) {
  json(res, 200, {
    ok: true,
    uptime: Math.round((Date.now() - startTime) / 1000),
    bridge_alive: bridgeAlive,
    bridge_restarts: bridgeRestarts,
    pid: process.pid,
  });
}

function handleStatus(_req, res) {
  json(res, 200, {
    ok: true,
    uptime: Math.round((Date.now() - startTime) / 1000),
    bridge: {
      alive: bridgeAlive,
      pid: bridgePid,
      port: bridgePort,
      restarts: bridgeRestarts,
      last_crash: lastCrash,
    },
    host: {
      platform: platform(),
      arch: arch(),
      hostname: hostname(),
      freemem: freemem(),
      totalmem: totalmem(),
      cpus: cpus().length,
    },
  });
}

async function handleExec(req, res) {
  let body;
  try {
    body = await readBody(req);
  } catch {
    return json(res, 400, { ok: false, error: 'Invalid JSON body' });
  }

  const { cmd } = body;
  if (!cmd || typeof cmd !== 'string') {
    return json(res, 400, { ok: false, error: 'Missing or invalid "cmd" field' });
  }

  let timeout = typeof body.timeout === 'number' ? body.timeout * 1000 : EXEC_TIMEOUT_DEFAULT;
  if (timeout > EXEC_TIMEOUT_MAX) timeout = EXEC_TIMEOUT_MAX;
  if (timeout < 1000) timeout = 1000;

  exec(cmd, { timeout, cwd: PROJECT_ROOT }, (err, stdout, stderr) => {
    const code = err ? (err.code ?? err.killed ? 137 : 1) : 0;
    json(res, 200, { ok: code === 0, stdout, stderr, code });
  });
}

function handleRestart(_req, res) {
  killBridge();
  // The exit handler will respawn automatically, but clear any pending timer
  // and force an immediate respawn after a short delay.
  if (respawnTimer) {
    clearTimeout(respawnTimer);
    respawnTimer = null;
  }
  // Give the bridge a moment to die, then spawn fresh
  setTimeout(() => {
    if (!bridgeAlive) {
      bridgeRestarts++;
      spawnBridge();
    }
  }, 500);

  json(res, 200, { ok: true, message: 'Bridge restarting' });
}

function handleUpdate(_req, res) {
  json(res, 200, { ok: false, message: 'Not implemented yet' });
}

function handleLogs(_req, res) {
  const lines = logRing.slice(-LOG_SERVE_SIZE);
  json(res, 200, { ok: true, lines, count: lines.length });
}

// ── HTTP server ──────────────────────────────────────────────────────────────

const server = createServer(async (req, res) => {
  const url = new URL(req.url, `http://localhost:${PORT}`);
  const path = url.pathname;
  const method = req.method;

  try {
    if (method === 'GET' && path === '/ping') return handlePing(req, res);
    if (method === 'GET' && path === '/status') return handleStatus(req, res);
    if (method === 'POST' && path === '/exec') return await handleExec(req, res);
    if (method === 'POST' && path === '/restart') return handleRestart(req, res);
    if (method === 'POST' && path === '/update') return handleUpdate(req, res);
    if (method === 'GET' && path === '/logs') return handleLogs(req, res);

    // Bridge IPC endpoints
    if (method === 'POST' && path === '/heartbeat') return json(res, 200, { ok: true });
    if (method === 'POST' && path === '/bridge-port') {
      try {
        const body = await readBody(req);
        if (body.bridge_port) bridgePort = body.bridge_port;
        return json(res, 200, { ok: true });
      } catch {
        return json(res, 400, { ok: false, error: 'Invalid body' });
      }
    }

    json(res, 404, { ok: false, error: 'Not found' });
  } catch (err) {
    json(res, 500, { ok: false, error: err.message });
  }
});

// ── Graceful shutdown ────────────────────────────────────────────────────────

function shutdown(signal) {
  if (shuttingDown) return;
  shuttingDown = true;
  console.log(`[supervisor] Received ${signal}, shutting down...`);

  if (respawnTimer) {
    clearTimeout(respawnTimer);
    respawnTimer = null;
  }

  killBridge();

  server.close(() => {
    console.log('[supervisor] HTTP server closed.');
    process.exit(0);
  });

  // Force exit after 5s if server won't close
  setTimeout(() => process.exit(1), 5000).unref();
}

process.on('SIGTERM', () => shutdown('SIGTERM'));
process.on('SIGINT', () => shutdown('SIGINT'));

// ── Start ────────────────────────────────────────────────────────────────────

server.listen(PORT, () => {
  console.log(`DDB Shell Supervisor v${VERSION} | Crash-Shield active on :${PORT}`);
  spawnBridge();
});
