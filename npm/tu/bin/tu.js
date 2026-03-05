#!/usr/bin/env node
const { spawnSync } = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');

const isWindows = process.platform === 'win32';
const binName = isWindows ? 'tu.exe' : 'tu';
const binPath = path.join(__dirname, '..', 'vendor', binName);

if (!fs.existsSync(binPath)) {
  console.error('[tokenusage] Native binary is missing.');
  console.error('[tokenusage] Try reinstalling: npm i -g tokenusage');
  process.exit(1);
}

const child = spawnSync(binPath, process.argv.slice(2), {
  stdio: 'inherit',
});

if (child.error) {
  console.error(`[tokenusage] Failed to launch ${binName}:`, child.error.message);
  process.exit(1);
}

process.exit(child.status ?? 1);
