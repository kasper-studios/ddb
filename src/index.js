/**
 * DDB Bridge — Main Entry Point (Worker Process)
 *
 * Spawned by the DDB Supervisor. Provides:
 *  • Foolproof guard  — refuses direct launch (requires supervisor)
 *  • Heartbeat loop   — pings supervisor every 10s
 *  • Action HTTP API  — POST /action {action, params}
 *  • Hermes auto-discovery & periodic re-verification
 *
 * Zero external dependencies — Node.js stdlib only.
 */

import http from 'node:http';
import { exec } from 'node:child_process';
import { promisify } from 'node:util';
import { readFile, writeFile, readdir, stat } from 'node:fs/promises';
import { platform, arch, hostname, uptime, freemem, totalmem } from 'node:os';
import { join, resolve } from 'node:path';

import { ActionRegistry } from './bridge.js';
import { startHermesWatch, getHermesState } from './hermes_discover.js';

const execAsync = promisify(exec);

// ═══════════════════════════════════════════════════════════════
// 1. FOOLPROOF GUARD
// ═══════════════════════════════════════════════════════════════

const SUPERVISOR_PORT = process.env.DDB_SUPERVISOR_PORT;

async function supervisorAlive(port) {
  return new Promise((res) => {
    const req = http.get(
      `http://127.0.0.1:${port}/ping`,
      { timeout: 2000 },
      (resp) => {
        let data = '';
        resp.on('data', (c) => (data += c));
        resp.on('end', () => res(true));
      },
    );
    req.on('error', () => res(false));
    req.on('timeout', () => { req.destroy(); res(false); });
  });
}

if (!SUPERVISOR_PORT) {
  console.error(
    '\x1b[1m\x1b[31m\n⚠️  Какун, меня напрямую не запускай!\n   Запусти шелл: ddbshell\n\x1b[0m',
  );
  process.exit(1);
}

if (!(await supervisorAlive(SUPERVISOR_PORT))) {
  console.error(
    '\x1b[1m\x1b[31m\n⚠️  Какун, меня напрямую не запускай!\n   Запусти шелл: ddbshell\n\x1b[0m',
  );
  process.exit(1);
}

// ═══════════════════════════════════════════════════════════════
// 2. HEARTBEAT LOOP
// ═══════════════════════════════════════════════════════════════

let _heartbeatMisses = 0;

function sendHeartbeat() {
  const body = JSON.stringify({ ts: Date.now() });
  const req = http.request(
    {
      hostname: '127.0.0.1',
      port: Number(SUPERVISOR_PORT),
      path: '/heartbeat',
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        'Content-Length': Buffer.byteLength(body),
      },
      timeout: 2000,
    },
    (resp) => {
      _heartbeatMisses = 0;
      resp.resume(); // drain
    },
  );
  req.on('error', () => {
    _heartbeatMisses++;
    if (_heartbeatMisses >= 3) {
      console.warn(
        `[heartbeat] Supervisor unreachable for ${_heartbeatMisses} consecutive beats`,
      );
    }
  });
  req.on('timeout', () => {
    req.destroy();
    _heartbeatMisses++;
  });
  req.end(body);
}

const heartbeatTimer = setInterval(sendHeartbeat, 10_000);
heartbeatTimer.unref();
// Send the first beat immediately
sendHeartbeat();

// ═══════════════════════════════════════════════════════════════
// 3. HERMES DISCOVERY
// ═══════════════════════════════════════════════════════════════

const hermesState = await startHermesWatch(60_000);

// ═══════════════════════════════════════════════════════════════
// 4. ACTION REGISTRATION
// ═══════════════════════════════════════════════════════════════

const registry = new ActionRegistry();

// ── exec ────────────────────────────────────────────────────────
registry.register('exec', async ({ command, cwd, timeout = 30_000 }) => {
  if (!command) throw new Error('Missing required param: command');
  const opts = { timeout, maxBuffer: 4 * 1024 * 1024 };
  if (cwd) opts.cwd = cwd;
  try {
    const { stdout, stderr } = await execAsync(command, opts);
    return { stdout, stderr, code: 0 };
  } catch (err) {
    return {
      stdout: err.stdout || '',
      stderr: err.stderr || err.message,
      code: err.code ?? 1,
    };
  }
});

// ── status ──────────────────────────────────────────────────────
registry.register('status', async () => {
  const h = getHermesState();
  return {
    platform: platform(),
    arch: arch(),
    hostname: hostname(),
    uptime: uptime(),
    freemem: freemem(),
    totalmem: totalmem(),
    hermes: { found: h.found, port: h.port || null },
  };
});

// ── screen (Wayland grim screenshot) ────────────────────────────
registry.register('screen', async ({ output } = {}) => {
  if (platform() !== 'linux') {
    throw new Error('Screen capture only supported on Linux/Wayland (grim)');
  }
  const args = output ? `grim -o ${output} -t png -` : 'grim -t png -';
  try {
    const { stdout } = await execAsync(args, {
      encoding: 'buffer',
      maxBuffer: 20 * 1024 * 1024,
      timeout: 10_000,
    });
    return { base64: stdout.toString('base64'), format: 'png' };
  } catch (err) {
    throw new Error(
      `Screen capture failed (is grim installed and Wayland running?): ${err.message}`,
    );
  }
});

// ── fs.read ─────────────────────────────────────────────────────
registry.register('fs.read', async ({ path: filePath, offset = 1, limit = 2000 }) => {
  if (!filePath) throw new Error('Missing required param: path');
  const resolved = resolve(filePath);
  const raw = await readFile(resolved, 'utf-8');
  const lines = raw.split('\n');
  const start = Math.max(0, offset - 1);
  const slice = lines.slice(start, start + limit);
  return {
    path: resolved,
    content: slice.join('\n'),
    total_lines: lines.length,
    returned_from: start + 1,
    returned_to: Math.min(start + limit, lines.length),
  };
});

// ── fs.write ────────────────────────────────────────────────────
registry.register('fs.write', async ({ path: filePath, content }) => {
  if (!filePath) throw new Error('Missing required param: path');
  if (content === undefined) throw new Error('Missing required param: content');
  const resolved = resolve(filePath);
  await writeFile(resolved, content, 'utf-8');
  return { path: resolved, bytes: Buffer.byteLength(content, 'utf-8') };
});

// ── fs.list ─────────────────────────────────────────────────────
registry.register('fs.list', async ({ path: dirPath = '.' }) => {
  const resolved = resolve(dirPath);
  const entries = await readdir(resolved);
  const results = [];
  for (const name of entries) {
    try {
      const st = await stat(join(resolved, name));
      results.push({
        name,
        type: st.isDirectory() ? 'dir' : st.isFile() ? 'file' : 'other',
        size: st.size,
        mtime: st.mtime.toISOString(),
      });
    } catch {
      results.push({ name, type: 'unknown', size: 0, mtime: null });
    }
  }
  return { path: resolved, entries: results };
});

// ── hermes.sessions ─────────────────────────────────────────────
registry.register('hermes.sessions', async () => {
  const h = getHermesState();
  if (!h.found || !h.port) throw new Error('Hermes not discovered on this host');

  return new Promise((res, rej) => {
    const headers = {};
    if (h.token) headers['X-Hermes-Session-Token'] = h.token;

    const req = http.get(
      `http://127.0.0.1:${h.port}/api/sessions?limit=50`,
      { headers, timeout: 5000 },
      (resp) => {
        let data = '';
        resp.on('data', (c) => (data += c));
        resp.on('end', () => {
          try {
            res(JSON.parse(data));
          } catch {
            res({ raw: data });
          }
        });
      },
    );
    req.on('error', (e) => rej(new Error(`Hermes API error: ${e.message}`)));
    req.on('timeout', () => { req.destroy(); rej(new Error('Hermes API timeout')); });
  });
});

// ── hermes.model (current model info) ───────────────────────────
registry.register('hermes.model', async () => {
  const h = getHermesState();
  if (!h.found || !h.port) throw new Error('Hermes not discovered on this host');

  return new Promise((res, rej) => {
    const headers = {};
    if (h.token) headers['X-Hermes-Session-Token'] = h.token;

    const req = http.get(
      `http://127.0.0.1:${h.port}/api/model/info`,
      { headers, timeout: 5000 },
      (resp) => {
        let data = '';
        resp.on('data', (c) => (data += c));
        resp.on('end', () => {
          try {
            res(JSON.parse(data));
          } catch {
            res({ raw: data });
          }
        });
      },
    );
    req.on('error', (e) => rej(new Error(`Hermes API error: ${e.message}`)));
    req.on('timeout', () => { req.destroy(); rej(new Error('Hermes API timeout')); });
  });
});

// ── hermes.models (available models catalog) ────────────────────
registry.register('hermes.models', async ({ provider } = {}) => {
  const h = getHermesState();
  if (!h.found || !h.port) throw new Error('Hermes not discovered on this host');

  return new Promise((res, rej) => {
    const headers = {};
    if (h.token) headers['X-Hermes-Session-Token'] = h.token;

    const req = http.get(
      `http://127.0.0.1:${h.port}/api/model/options`,
      { headers, timeout: 5000 },
      (resp) => {
        let data = '';
        resp.on('data', (c) => (data += c));
        resp.on('end', () => {
          try {
            const parsed = JSON.parse(data);
            const providers = parsed.providers || [];
            if (provider) {
              const matched = providers.find(
                (p) => p.name.toLowerCase() === provider.toLowerCase() ||
                       (p.id && p.id.toLowerCase() === provider.toLowerCase())
              );
              return res(matched ? { provider: matched.name, models: matched.models } : { error: 'PROVIDER_NOT_FOUND', providers: providers.map(p => p.name) });
            }
            res({
              providers: providers.map((p) => ({
                name: p.name,
                id: p.id,
                models_count: (p.models || []).length,
                models: (p.models || []).slice(0, 30), // top 30 preview
              })),
            });
          } catch {
            res({ raw: data });
          }
        });
      },
    );
    req.on('error', (e) => rej(new Error(`Hermes API error: ${e.message}`)));
    req.on('timeout', () => { req.destroy(); rej(new Error('Hermes API timeout')); });
  });
});

// ── hermes.model_set (switch model in session or globally) ─────
registry.register('hermes.model_set', async ({ model, session_id, once = false }) => {
  const h = getHermesState();
  if (!h.found || !h.port) throw new Error('Hermes not discovered on this host');
  if (!model) throw new Error('Missing required param: model');

  const switchCmd = once ? `/model ${model} --once` : `/model ${model}`;

  // Deliver via prompt.submit to the target session (or active session)
  return new Promise((res, rej) => {
    const tokenParam = h.token ? encodeURIComponent(h.token) : '';
    const wsPath = `/api/ws${tokenParam ? '?token=' + tokenParam : ''}`;

    const req = http.request({
      hostname: '127.0.0.1',
      port: h.port,
      path: wsPath,
      method: 'GET',
      headers: {
        'Connection': 'Upgrade',
        'Upgrade': 'websocket',
        'Sec-WebSocket-Version': '13',
        'Sec-WebSocket-Key': Buffer.from(
          Array.from({ length: 16 }, () => Math.floor(Math.random() * 256)),
        ).toString('base64'),
      },
      timeout: 10_000,
    });

    req.on('upgrade', (_resp, socket) => {
      const rpcPayload = JSON.stringify({
        jsonrpc: '2.0',
        id: Date.now(),
        method: 'prompt.submit',
        params: {
          session_id: session_id || undefined,
          text: switchCmd,
        },
      });
      const payloadBuf = Buffer.from(rpcPayload, 'utf-8');
      const frame = buildWsFrame(payloadBuf);
      socket.write(frame);

      let received = '';
      socket.on('data', (chunk) => {
        const parsed = parseWsFrame(chunk);
        if (parsed) received += parsed;
      });

      setTimeout(() => {
        socket.end();
        try {
          res({ ok: true, command: switchCmd, response: JSON.parse(received) });
        } catch {
          res({ ok: true, command: switchCmd, raw: received || '(applied)' });
        }
      }, 1500);
    });

    req.on('error', (e) => rej(new Error(`Hermes WS error: ${e.message}`)));
    req.on('timeout', () => { req.destroy(); rej(new Error('Hermes WS timeout')); });
    req.end();
  });
});

// ── hermes.send ─────────────────────────────────────────────────
registry.register('hermes.send', async ({ session_id, prompt }) => {
  const h = getHermesState();
  if (!h.found || !h.port) throw new Error('Hermes not discovered on this host');
  if (!prompt) throw new Error('Missing required param: prompt');

  // Hermes WS endpoint: ws://127.0.0.1:<port>/api/ws?token=<token>
  // We use a raw HTTP upgrade since we have no ws dependency.
  // Send prompt.submit JSON-RPC, collect first response, close.
  return new Promise((res, rej) => {
    const tokenParam = h.token ? encodeURIComponent(h.token) : '';
    const wsPath = `/api/ws${tokenParam ? '?token=' + tokenParam : ''}`;

    const req = http.request({
      hostname: '127.0.0.1',
      port: h.port,
      path: wsPath,
      method: 'GET',
      headers: {
        'Connection': 'Upgrade',
        'Upgrade': 'websocket',
        'Sec-WebSocket-Version': '13',
        'Sec-WebSocket-Key': Buffer.from(
          Array.from({ length: 16 }, () => Math.floor(Math.random() * 256)),
        ).toString('base64'),
      },
      timeout: 10_000,
    });

    req.on('upgrade', (_res, socket) => {
      // Send JSON-RPC prompt.submit frame
      const rpcPayload = JSON.stringify({
        jsonrpc: '2.0',
        id: 1,
        method: 'prompt.submit',
        params: {
          session_id: session_id || undefined,
          text: `[Dirom Reqwest]\n${prompt}`,
        },
      });
      const payloadBuf = Buffer.from(rpcPayload, 'utf-8');
      const frame = buildWsFrame(payloadBuf);
      socket.write(frame);

      let received = '';
      socket.on('data', (chunk) => {
        // Very minimal WS frame reading for text frames
        const parsed = parseWsFrame(chunk);
        if (parsed) received += parsed;
      });

      // Give Hermes a moment then close
      setTimeout(() => {
        socket.end();
        try {
          res(JSON.parse(received));
        } catch {
          res({ raw: received || '(no response)' });
        }
      }, 3000);
    });

    req.on('error', (e) => rej(new Error(`Hermes WS error: ${e.message}`)));
    req.on('timeout', () => { req.destroy(); rej(new Error('Hermes WS timeout')); });
    req.end();
  });
});

// Minimal WebSocket frame helpers (client-masked, text opcode)
function buildWsFrame(payload) {
  const len = payload.length;
  const header = [];
  header.push(0x81); // FIN + text opcode
  // Client frames MUST be masked
  if (len < 126) {
    header.push(0x80 | len);
  } else if (len < 65536) {
    header.push(0x80 | 126, (len >> 8) & 0xff, len & 0xff);
  } else {
    header.push(0x80 | 127);
    for (let i = 7; i >= 0; i--) header.push((len >> (i * 8)) & 0xff);
  }
  const mask = Buffer.from([
    Math.random() * 256 | 0,
    Math.random() * 256 | 0,
    Math.random() * 256 | 0,
    Math.random() * 256 | 0,
  ]);
  const masked = Buffer.alloc(payload.length);
  for (let i = 0; i < payload.length; i++) masked[i] = payload[i] ^ mask[i & 3];
  return Buffer.concat([Buffer.from(header), mask, masked]);
}

function parseWsFrame(buf) {
  if (buf.length < 2) return null;
  const opcode = buf[0] & 0x0f;
  if (opcode !== 0x01) return null; // only text
  const masked = (buf[1] & 0x80) !== 0;
  let payloadLen = buf[1] & 0x7f;
  let offset = 2;
  if (payloadLen === 126) {
    payloadLen = buf.readUInt16BE(2);
    offset = 4;
  } else if (payloadLen === 127) {
    payloadLen = Number(buf.readBigUInt64BE(2));
    offset = 10;
  }
  let maskKey;
  if (masked) {
    maskKey = buf.slice(offset, offset + 4);
    offset += 4;
  }
  const data = buf.slice(offset, offset + payloadLen);
  if (masked && maskKey) {
    for (let i = 0; i < data.length; i++) data[i] ^= maskKey[i & 3];
  }
  return data.toString('utf-8');
}

// ═══════════════════════════════════════════════════════════════
// 5. HTTP SERVER
// ═══════════════════════════════════════════════════════════════

const BRIDGE_PORT = process.env.DDB_BRIDGE_PORT
  ? Number(process.env.DDB_BRIDGE_PORT)
  : 0;

const server = http.createServer(async (req, res) => {
  // CORS headers for local dev
  res.setHeader('Access-Control-Allow-Origin', '*');
  res.setHeader('Access-Control-Allow-Methods', 'POST, OPTIONS');
  res.setHeader('Access-Control-Allow-Headers', 'Content-Type');

  if (req.method === 'OPTIONS') {
    res.writeHead(204);
    return res.end();
  }

  if (req.method === 'GET' && req.url === '/ping') {
    res.writeHead(200, { 'Content-Type': 'application/json' });
    return res.end(JSON.stringify({ ok: true, service: 'ddb-bridge' }));
  }

  if (req.method === 'POST' && req.url === '/action') {
    let body = '';
    for await (const chunk of req) body += chunk;

    let parsed;
    try {
      parsed = JSON.parse(body);
    } catch {
      res.writeHead(400, { 'Content-Type': 'application/json' });
      return res.end(JSON.stringify({ ok: false, error: 'INVALID_JSON' }));
    }

    const { action, params } = parsed;
    if (!action) {
      res.writeHead(400, { 'Content-Type': 'application/json' });
      return res.end(
        JSON.stringify({ ok: false, error: 'MISSING_ACTION' }),
      );
    }

    const result = await registry.execute(action, params || {});
    res.writeHead(result.ok ? 200 : 400, {
      'Content-Type': 'application/json',
    });
    return res.end(JSON.stringify(result));
  }

  res.writeHead(404, { 'Content-Type': 'application/json' });
  res.end(JSON.stringify({ ok: false, error: 'NOT_FOUND' }));
});

server.listen(BRIDGE_PORT, '127.0.0.1', () => {
  const addr = server.address();
  const hermesTag = hermesState.found
    ? `found (pid=${hermesState.pid} port=${hermesState.port})`
    : 'not found';

  console.log(
    `DDB Bridge v1.0.0 | Worker online | port=${addr.port} | Hermes: ${hermesTag}`,
  );

  // Announce our port to supervisor
  sendPortAnnounce(addr.port);
});

function sendPortAnnounce(port) {
  const body = JSON.stringify({ bridge_port: port });
  const req = http.request(
    {
      hostname: '127.0.0.1',
      port: Number(SUPERVISOR_PORT),
      path: '/bridge-port',
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        'Content-Length': Buffer.byteLength(body),
      },
      timeout: 2000,
    },
    (resp) => resp.resume(),
  );
  req.on('error', () => {}); // supervisor may not have this endpoint yet
  req.end(body);
}
