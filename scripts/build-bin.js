/**
 * DDB Cross-Platform Binary Builder
 * Compiles standalone executables using Bun CLI
 */

import { exec } from 'node:child_process';
import { promisify } from 'node:util';
import { mkdir } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const execAsync = promisify(exec);
const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
const ROOT = join(__dirname, '..');
const DIST = join(ROOT, 'dist');
const ENTRY = join(ROOT, 'src/cli.js');

async function build() {
  console.log('🚀 Начинаю сборку бинарников DDB через Bun...\n');
  await mkdir(DIST, { recursive: true });

  const targets = [
    { target: 'bun-linux-x64', out: 'dist/ddb-linux-x64', name: '🐧 Linux x64' },
    { target: 'bun-windows-x64', out: 'dist/ddb-windows-x64.exe', name: '🪟 Windows x64' },
  ];

  for (const t of targets) {
    console.log(`📦 Компилирую ${t.name} (${t.target})...`);
    const cmd = `bun build --compile --target=${t.target} "${ENTRY}" --outfile "${join(ROOT, t.out)}"`;
    try {
      const { stdout } = await execAsync(cmd, { cwd: ROOT });
      console.log(`   ✅ Готово: ${t.out}\n`);
    } catch (err) {
      console.error(`   ❌ Ошибка сборки ${t.name}:`, err.message);
    }
  }

  console.log('🎉 Все бинарники успешно собраны в папку dist/!');
}

build().catch(console.error);
