#!/usr/bin/env node
/**
 * DDB (Dirom Device Bridge) — Main CLI Entry Point
 *
 * Usage:
 *   ddb               # Interactive TUI mode (with mascot & menu)
 *   ddb --daemon      # Start Supervisor in background/daemon mode (:8790)
 *   ddb status        # Print host and bridge health summary
 *   ddb restart       # Trigger bridge worker restart
 *   ddb install       # Install to systemd --user autostart
 */

import { runTUI } from './tui.js';
import http from 'node:http';

const args = process.argv.slice(2);
const SUPERVISOR_PORT = parseInt(process.env.DDB_SHELL_PORT, 10) || 8790;

async function printStatus() {
  return new Promise((resolve) => {
    const req = http.get(`http://127.0.0.1:${SUPERVISOR_PORT}/status`, { timeout: 2000 }, (res) => {
      let data = '';
      res.on('data', (c) => (data += c));
      res.on('end', () => {
        try {
          const s = JSON.parse(data);
          console.log(`\x1b[32m● DDB Supervisor:\x1b[0m ONLINE (:8790, pid: ${s.bridge?.pid}, uptime: ${s.uptime}s)`);
          console.log(`\x1b[36m● Host:\x1b[0m ${s.host?.hostname} (${s.host?.platform} ${s.host?.arch}, cpus: ${s.host?.cpus})`);
          console.log(`\x1b[35m● Bridge Worker:\x1b[0m ${s.bridge?.alive ? 'ALIVE (port ' + s.bridge?.port + ')' : 'DEAD'} (restarts: ${s.bridge?.restarts})`);
          resolve(true);
        } catch {
          console.log('Error parsing supervisor response.');
          resolve(false);
        }
      });
    });
    req.on('error', () => {
      console.log('\x1b[31m○ DDB Supervisor:\x1b[0m OFFLINE (порт :8790 не отвечает)');
      resolve(false);
    });
    req.on('timeout', () => { req.destroy(); resolve(false); });
  });
}

async function triggerRestart() {
  return new Promise((resolve) => {
    const req = http.request(
      {
        hostname: '127.0.0.1',
        port: SUPERVISOR_PORT,
        path: '/restart',
        method: 'POST',
      },
      (res) => {
        console.log('✅ Сигнал перезапуска отправлен в DDB супервайзер.');
        resolve(true);
      }
    );
    req.on('error', (e) => {
      console.log(`❌ Ошибка связи с супервайзером: ${e.message}`);
      resolve(false);
    });
    req.end();
  });
}

// ── Routing ──────────────────────────────────────────────────

if (args.includes('--daemon') || args.includes('--supervisor')) {
  // Headless supervisor mode
  await import('./supervisor.js');
} else if (args[0] === 'status') {
  await printStatus();
} else if (args[0] === 'restart') {
  await triggerRestart();
} else if (args[0] === 'ping') {
  http.get(`http://127.0.0.1:${SUPERVISOR_PORT}/ping`, (res) => {
    res.pipe(process.stdout);
  }).on('error', (e) => console.log('offline'));
} else {
  // Interactive TUI
  await runTUI();
}
