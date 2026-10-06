/**
 * DDB Interactive TUI
 * Terminal User Interface with mascot and system controls
 */

import readline from 'node:readline';
import http from 'node:http';
import { spawn, exec } from 'node:child_process';
import { promisify } from 'node:util';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import { printLogo } from './colored_logo.js';

const execAsync = promisify(exec);
const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
const SUPERVISOR_SCRIPT = join(__dirname, 'supervisor.js');

const SUPERVISOR_PORT = parseInt(process.env.DDB_SHELL_PORT, 10) || 8790;

function checkSupervisorStatus() {
  return new Promise((resolve) => {
    const req = http.get(`http://127.0.0.1:${SUPERVISOR_PORT}/status`, { timeout: 1500 }, (res) => {
      let data = '';
      res.on('data', (c) => (data += c));
      res.on('end', () => {
        try {
          resolve(JSON.parse(data));
        } catch {
          resolve(null);
        }
      });
    });
    req.on('error', () => resolve(null));
    req.on('timeout', () => { req.destroy(); resolve(null); });
  });
}

function postSupervisor(path, body = {}) {
  return new Promise((resolve) => {
    const payload = JSON.stringify(body);
    const req = http.request(
      {
        hostname: '127.0.0.1',
        port: SUPERVISOR_PORT,
        path,
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          'Content-Length': Buffer.byteLength(payload),
        },
        timeout: 5000,
      },
      (res) => {
        let data = '';
        res.on('data', (c) => (data += c));
        res.on('end', () => {
          try {
            resolve(JSON.parse(data));
          } catch {
            resolve(null);
          }
        });
      }
    );
    req.on('error', () => resolve(null));
    req.on('timeout', () => { req.destroy(); resolve(null); });
    req.end(payload);
  });
}

async function installSystemd() {
  const serviceContent = `[Unit]
Description=DDB Shell Supervisor (Dirom Device Bridge)
After=network.target

[Service]
Type=simple
ExecStart=${process.execPath} ${SUPERVISOR_SCRIPT}
Restart=always
RestartSec=5
Environment=DDB_SHELL_PORT=8790

[Install]
WantedBy=default.target
`;

  try {
    const serviceDir = join(process.env.HOME || '~', '.config/systemd/user');
    await execAsync(`mkdir -p "${serviceDir}"`);
    const servicePath = join(serviceDir, 'ddb-supervisor.service');
    const { writeFile } = await import('node:fs/promises');
    await writeFile(servicePath, serviceContent, 'utf-8');
    await execAsync('systemctl --user daemon-reload');
    await execAsync('systemctl --user enable --now ddb-supervisor.service');
    return { ok: true, message: 'Служба ddb-supervisor установлена и запущена через systemd --user!' };
  } catch (err) {
    return { ok: false, error: err.message };
  }
}

export async function runTUI() {
  console.clear();
  printLogo();

  console.log('\n  \x1b[1m\x1b[35m=== DDB (Dirom Device Bridge) v3.0.0 ===\x1b[0m');
  console.log('  \x1b[90mКраш-Шилд Супервайзер & Агент Хоста Цитадели\x1b[0m\n');

  const status = await checkSupervisorStatus();
  if (status && status.ok) {
    console.log(`  🟢 \x1b[32mСупервайзер активен на порту :${SUPERVISOR_PORT}\x1b[0m (PID: ${status.bridge?.pid || '?'}, аптайм: ${status.uptime}s)`);
    console.log(`  🖥️  Хост: \x1b[36m${status.host?.hostname}\x1b[0m (${status.host?.platform} ${status.host?.arch}, RAM свободно: ${Math.round((status.host?.freemem || 0)/1024/1024)}MB)`);
  } else {
    console.log(`  🔴 \x1b[31mСупервайзер не запущен\x1b[0m (порт :${SUPERVISOR_PORT} свободен)`);
  }

  console.log('\n  \x1b[1mВыберите действие:\x1b[0m');
  console.log('  \x1b[33m[1]\x1b[0m 🚀 Запустить DDB Супервайзер в фоне');
  console.log('  \x1b[33m[2]\x1b[0m ⚙️  Установить в автозагрузку (systemd --user)');
  console.log('  \x1b[33m[3]\x1b[0m 📊 Проверить статус хоста и Hermes');
  console.log('  \x1b[33m[4]\x1b[0m 🔄 Перезапустить DDB Bridge');
  console.log('  \x1b[33m[5]\x1b[0m 🛑 Остановить DDB Супервайзер');
  console.log('  \x1b[90m[0]\x1b[0m ❌ Выход\n');

  const rl = readline.createInterface({
    input: process.stdin,
    output: process.stdout,
  });

  rl.question('  > ', async (choice) => {
    rl.close();
    const c = choice.trim();

    if (c === '1') {
      console.log('\n🚀 Запускаю супервайзер...');
      const child = spawn(process.execPath, [SUPERVISOR_SCRIPT], {
        detached: true,
        stdio: 'ignore',
        env: { ...process.env, DDB_SHELL_PORT: String(SUPERVISOR_PORT) },
      });
      child.unref();
      await new Promise((r) => setTimeout(r, 1500));
      const s = await checkSupervisorStatus();
      if (s && s.ok) {
        console.log('✅ Супервайзер успешно запущен на порту :8790!');
      } else {
        console.log('⚠️  Супервайзер запущен в фоне (проверьте порт :8790).');
      }
    } else if (c === '2') {
      console.log('\n⚙️  Настраиваю автозагрузку systemd --user...');
      const res = await installSystemd();
      if (res.ok) console.log(`✅ ${res.message}`);
      else console.log(`❌ Ошибка: ${res.error}`);
    } else if (c === '3') {
      console.log('\n📊 Запрашиваю статус...');
      const s = await checkSupervisorStatus();
      if (s) {
        console.log(JSON.stringify(s, null, 2));
      } else {
        console.log('❌ Супервайзер недоступен.');
      }
    } else if (c === '4') {
      console.log('\n🔄 Перезапускаю воркер Bridge...');
      const res = await postSupervisor('/restart');
      if (res && res.ok) console.log('✅ Воркер перезапущен!');
      else console.log('❌ Не удалось перезапустить.');
    } else if (c === '5') {
      console.log('\n🛑 Останавливаю супервайзер...');
      try {
        await execAsync(`kill $(lsof -ti :${SUPERVISOR_PORT}) 2>/dev/null`);
        console.log('✅ Супервайзер остановлен.');
      } catch {
        console.log('Супервайзер уже был неактивен.');
      }
    } else if (c === '0') {
      console.log('\nДо встречи, какун! 👋');
      process.exit(0);
    } else {
      console.log('\nНеверный выбор.');
    }

    console.log('\nНажмите Enter для возврата в меню...');
    const pause = readline.createInterface({ input: process.stdin, output: process.stdout });
    pause.question('', () => {
      pause.close();
      runTUI();
    });
  });
}
