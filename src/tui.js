/**
 * DDB Interactive TUI
 * Terminal User Interface with mascot, permissions matrix, tunnel setup, and controls
 */

import readline from 'node:readline';
import http from 'node:http';
import os from 'node:os';
import { spawn, exec } from 'node:child_process';
import { promisify } from 'node:util';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import { printLogo } from './colored_logo.js';
import { loadConfig, saveConfig } from './config.js';

const execAsync = promisify(exec);
const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
const SUPERVISOR_SCRIPT = join(__dirname, 'supervisor.js');
const SUPERVISOR_PORT = parseInt(process.env.DDB_SHELL_PORT, 10) || 8790;

import { readlinkSync } from 'node:fs';

function getSelfExecutable() {
  // On Linux, /proc/self/exe is the absolute rock-solid path to the running binary
  if (process.platform === 'linux') {
    try {
      return readlinkSync('/proc/self/exe');
    } catch {
      // fallback
    }
  }
  return process.argv[0] || process.execPath;
}

function spawnSupervisorProcess() {
  const isBunBinary = import.meta.url && import.meta.url.includes('$bunfs');
  const env = { ...process.env, DDB_SHELL_PORT: String(SUPERVISOR_PORT) };

  if (isBunBinary) {
    // Compiled binary: re-spawn self with --daemon
    return spawn(getSelfExecutable(), ['--daemon'], {
      detached: true,
      stdio: 'ignore',
      env,
    });
  }

  // Node dev script: spawn node supervisor.js
  return spawn(process.execPath, [SUPERVISOR_SCRIPT], {
    detached: true,
    stdio: 'ignore',
    env,
  });
}

function ask(questionText) {
  return new Promise((resolve) => {
    const rl = readline.createInterface({ input: process.stdin, output: process.stdout });
    rl.question(questionText, (ans) => {
      rl.close();
      resolve(ans.trim());
    });
  });
}

function pause() {
  return ask('\n  Нажмите Enter для продолжения...');
}

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

function getLocalIp() {
  const nets = os.networkInterfaces();
  for (const name of Object.keys(nets)) {
    for (const net of nets[name]) {
      if (net.family === 'IPv4' && !net.internal) {
        return net.address;
      }
    }
  }
  return '127.0.0.1';
}

async function installSystemd() {
  const execCmd = `${getSelfExecutable()} --daemon`;
  const serviceContent = `[Unit]
Description=DDB Shell Supervisor (Dirom Device Bridge)
After=network.target

[Service]
Type=simple
ExecStart=${execCmd}
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

// ── Permissions Menu ──────────────────────────────────────────────

async function permissionsMenu() {
  while (true) {
    console.clear();
    printLogo();
    const cfg = await loadConfig();
    const p = cfg.permissions;

    console.log('\n  \x1b[1m\x1b[35m[ ПРАВА И РАЗРЕШЕНИЯ ]\x1b[0m');
    console.log('  \x1b[90mУправляйте тем, что Диром имеет право делать на этой машине:\x1b[0m\n');

    console.log(`  [1] Display Control (клики/ввод):   ${p.display_control?.enabled ? '\x1b[32m[ВКЛ]\x1b[0m' : '\x1b[31m[ВЫКЛ]\x1b[0m'}`);
    console.log(`  [2] Скриншоты экрана (grim/CUA):    ${p.screen ? '\x1b[32m[ВКЛ]\x1b[0m' : '\x1b[31m[ВЫКЛ]\x1b[0m'}`);
    console.log(`  [3] Звук и Микрофон (sound_steal):  ${p.audio ? '\x1b[32m[ВКЛ]\x1b[0m' : '\x1b[31m[ВЫКЛ]\x1b[0m'}`);
    console.log(`  [4] Чтение файлов (fs.read/list):   ${p.fs?.read ? '\x1b[32m[ВКЛ]\x1b[0m' : '\x1b[31m[ВЫКЛ]\x1b[0m'}`);
    console.log(`  [5] Запись файлов (fs.write):       ${p.fs?.write ? '\x1b[32m[ВКЛ]\x1b[0m' : '\x1b[31m[ВЫКЛ]\x1b[0m'}`);
    console.log(`  [6] Выполнение шелл-команд (exec):  ${p.exec ? '\x1b[32m[ВКЛ]\x1b[0m' : '\x1b[31m[ВЫКЛ]\x1b[0m'}`);
    console.log(`  [7] Hermes Desktop Bridge:          ${p.hermes ? '\x1b[32m[ВКЛ]\x1b[0m' : '\x1b[31m[ВЫКЛ]\x1b[0m'}`);
    console.log('  \x1b[90m[0] <- Назад в главное меню\x1b[0m\n');

    const choice = await ask('  Выберите пункт для переключения: ');
    if (choice === '0') break;

    if (choice === '1') {
      if (!p.display_control) p.display_control = { enabled: false };
      p.display_control.enabled = !p.display_control.enabled;
    } else if (choice === '2') {
      p.screen = !p.screen;
    } else if (choice === '3') {
      p.audio = !p.audio;
    } else if (choice === '4') {
      if (typeof p.fs !== 'object') p.fs = { read: true, write: false, list: true };
      p.fs.read = !p.fs.read;
      p.fs.list = p.fs.read;
    } else if (choice === '5') {
      if (typeof p.fs !== 'object') p.fs = { read: true, write: false, list: true };
      p.fs.write = !p.fs.write;
    } else if (choice === '6') {
      p.exec = !p.exec;
    } else if (choice === '7') {
      p.hermes = !p.hermes;
    }

    await saveConfig(cfg);
  }
}

// ── Tunnel Setup Menu ─────────────────────────────────────────────

async function tunnelMenu() {
  console.clear();
  printLogo();
  const cfg = await loadConfig();

  console.log('\n  \x1b[1m\x1b[36m[ СЕТЬ И ТУННЕЛИ ]\x1b[0m');
  console.log('  \x1b[90mВыберите, как Диром будет связываться с этим компьютером:\x1b[0m\n');

  console.log('  \x1b[33m[1]\x1b[0m Kasper Cloud API (Zero-Config WSS)');
  console.log('  \x1b[33m[2]\x1b[0m Cloudflare Tunnel (токен туннеля)');
  console.log('  \x1b[33m[3]\x1b[0m Ngrok Tunnel (authtoken)');
  console.log('  \x1b[33m[4]\x1b[0m Локальная сеть LAN (прямой IP)');
  console.log('  \x1b[90m[0]\x1b[0m <- Назад\n');

  const c = await ask('  > ');
  if (c === '1') {
    const key = await ask('  Введите ваш Cloud API ключ (выдаётся Создателем / Диромом): ');
    if (key) {
      cfg.tunnel_type = 'cloud';
      cfg.tunnel_token = key;
      await saveConfig(cfg);
      console.log('  [OK] Режим переключен на Kasper Cloud API!');
    }
  } else if (c === '2') {
    const token = await ask('  Введите токен Cloudflare Tunnel: ');
    if (token) {
      cfg.tunnel_type = 'cloudflare';
      cfg.tunnel_token = token;
      await saveConfig(cfg);
      console.log('  [OK] Токен Cloudflare сохранён!');
    }
  } else if (c === '3') {
    const token = await ask('  Введите Ngrok Authtoken: ');
    if (token) {
      cfg.tunnel_type = 'ngrok';
      cfg.tunnel_token = token;
      await saveConfig(cfg);
      console.log('  [OK] Токен Ngrok сохранён!');
    }
  } else if (c === '4') {
    cfg.tunnel_type = 'lan';
    await saveConfig(cfg);
    console.log('  [OK] Переключено на прямую связь по локальной сети LAN!');
  }

  await pause();
}

// ── Feedback Menu ─────────────────────────────────────────────────

async function feedbackMenu() {
  console.clear();
  printLogo();
  console.log('\n  \x1b[1m\x1b[32m[ ОБРАТНАЯ СВЯЗЬ И ИДЕИ ]\x1b[0m');
  console.log('  \x1b[90mКуда вы хотите направить идею или баг-репорт:\x1b[0m\n');
  console.log('  \x1b[33m[1]\x1b[0m Discord сервер Kasper Studios (https://discord.gg/kasper)');
  console.log('  \x1b[33m[2]\x1b[0m GitHub Issues репозитория DDB');
  console.log('  \x1b[90m[0]\x1b[0m <- Назад\n');

  const c = await ask('  > ');
  if (c === '1') {
    console.log('\n  Открываю Discord...');
    execAsync('xdg-open "https://discord.gg/kasper" 2>/dev/null || open "https://discord.gg/kasper" 2>/dev/null');
  } else if (c === '2') {
    console.log('\n  Открываю GitHub Issues...');
    execAsync('xdg-open "https://github.com/kasper-studios/ddb/issues" 2>/dev/null || open "https://github.com/kasper-studios/ddb/issues" 2>/dev/null');
  }
  await pause();
}

// ── Main TUI Runner ───────────────────────────────────────────────

export async function runInstall() {
  return await installSystemd();
}

export async function runTUI() {
  while (true) {
    console.clear();
    printLogo();

    const cfg = await loadConfig();
    const status = await checkSupervisorStatus();
    const localIp = getLocalIp();

    console.log('\n  \x1b[1m\x1b[35m=== DDB (Dirom Device Bridge) v3.0.0 ===\x1b[0m');
    console.log('  \x1b[90mКраш-Шилд Супервайзер & Агент Хоста Цитадели\x1b[0m\n');

    if (status && status.ok) {
      console.log(`  \x1b[32m[+]\x1b[0m \x1b[32mСупервайзер:\x1b[0m АКТИВЕН (:8790, PID: ${status.bridge?.pid || '?'}, аптайм: ${status.uptime}s)`);
      console.log(`  \x1b[36m[*]\x1b[0m \x1b[36mХост:\x1b[0m ${cfg.host_name} (LAN IP: \x1b[1m${localIp}\x1b[0m, RAM: ${Math.round((status.host?.freemem || 0)/1024/1024)}MB)`);
      console.log(`  \x1b[33m[#]\x1b[0m \x1b[33mAPI Ключ:\x1b[0m ${cfg.api_key}`);
      
      const targetUrl = cfg.tunnel_type === 'lan' ? `http://${localIp}:${SUPERVISOR_PORT}` : (cfg.tunnel_url || 'Cloud Tunnel');
      console.log(`\n  \x1b[1m\x1b[32mСкопируйте для Дирома:\x1b[0m`);
      console.log(`  \x1b[1m\x1b[33mdevice.reg(name='${cfg.host_name}', url='${targetUrl}', key='${cfg.api_key}')\x1b[0m`);
    } else {
      console.log(`  \x1b[31m[-]\x1b[0m \x1b[31mСупервайзер:\x1b[0m НЕ ЗАПУЩЕН (порт :${SUPERVISOR_PORT} свободен)`);
    }

    console.log('\n  \x1b[1mМеню управления:\x1b[0m');
    console.log('  \x1b[33m[1]\x1b[0m Запустить DDB Супервайзер в фоне');
    console.log('  \x1b[33m[2]\x1b[0m Настройка разрешений (Permissions Matrix)');
    console.log('  \x1b[33m[3]\x1b[0m Настройка сети и туннелей');
    console.log('  \x1b[33m[4]\x1b[0m Установить в автозагрузку (systemd --user)');
    console.log('  \x1b[33m[5]\x1b[0m Проверить статус хоста и Hermes');
    console.log('  \x1b[33m[6]\x1b[0m Перезапустить DDB Bridge воркер');
    console.log('  \x1b[33m[7]\x1b[0m Обратная связь / Сообщить об ошибке');
    console.log('  \x1b[33m[8]\x1b[0m Остановить DDB Супервайзер');
    console.log('  \x1b[90m[0]\x1b[0m Выход\n');

    const choice = await ask('  > ');

    if (choice === '1') {
      console.log('\n🚀 Запускаю супервайзер...');
      const child = spawnSupervisorProcess();
      child.unref();
      await new Promise((r) => setTimeout(r, 1500));
      await pause();
    } else if (choice === '2') {
      await permissionsMenu();
    } else if (choice === '3') {
      await tunnelMenu();
    } else if (choice === '4') {
      console.log('\n⚙️  Настраиваю автозагрузку systemd --user...');
      const res = await installSystemd();
      if (res.ok) console.log(`✅ ${res.message}`);
      else console.log(`❌ Ошибка: ${res.error}`);
      await pause();
    } else if (choice === '5') {
      console.log('\n📊 Запрашиваю статус...');
      const s = await checkSupervisorStatus();
      console.log(JSON.stringify(s || { error: 'Супервайзер оффлайн' }, null, 2));
      await pause();
    } else if (choice === '6') {
      console.log('\n🔄 Перезапускаю воркер Bridge...');
      const res = await postSupervisor('/restart');
      if (res && res.ok) console.log('✅ Воркер перезапущен!');
      else console.log('❌ Не удалось перезапустить.');
      await pause();
    } else if (choice === '7') {
      await feedbackMenu();
    } else if (choice === '8') {
      console.log('\n🛑 Останавливаю супервайзер...');
      try {
        await execAsync(`kill $(lsof -ti :${SUPERVISOR_PORT}) 2>/dev/null`);
        console.log('✅ Супервайзер остановлен.');
      } catch {
        console.log('Супервайзер уже был неактивен.');
      }
      await pause();
    } else if (choice === '0') {
      console.log('\nДо встречи, какун! 👋');
      process.exit(0);
    }
  }
}
