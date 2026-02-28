const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const https = require('node:https');

const pkg = require('../package.json');

const OWNER = 'hanbu97';
const REPO = 'tokenusage';
const VERSION_TAG = `v${pkg.version}`;

function resolveTarget() {
  const platform = process.platform;
  const arch = process.arch;

  if (platform === 'darwin' && arch === 'arm64') return 'aarch64-apple-darwin';
  if (platform === 'darwin' && arch === 'x64') return 'x86_64-apple-darwin';
  if (platform === 'linux' && arch === 'x64') return 'x86_64-unknown-linux-gnu';
  if (platform === 'win32' && arch === 'x64') return 'x86_64-pc-windows-msvc';

  return null;
}

function request(url) {
  return new Promise((resolve, reject) => {
    https
      .get(
        url,
        {
          headers: {
            'user-agent': 'tokenusage-installer',
            accept: 'application/octet-stream',
          },
        },
        (res) => resolve(res),
      )
      .on('error', reject);
  });
}

async function downloadWithRedirect(url, outFile, redirects = 0) {
  if (redirects > 8) {
    throw new Error('Too many redirects while downloading binary');
  }

  const res = await request(url);

  if (res.statusCode && res.statusCode >= 300 && res.statusCode < 400 && res.headers.location) {
    res.resume();
    return downloadWithRedirect(res.headers.location, outFile, redirects + 1);
  }

  if (res.statusCode !== 200) {
    const chunks = [];
    for await (const chunk of res) chunks.push(chunk);
    const body = Buffer.concat(chunks).toString('utf8');
    throw new Error(`Download failed (${res.statusCode}) ${url}\n${body.slice(0, 500)}`);
  }

  await fs.promises.mkdir(path.dirname(outFile), { recursive: true });
  const tmp = `${outFile}.tmp`;

  await new Promise((resolve, reject) => {
    const file = fs.createWriteStream(tmp, { mode: 0o755 });
    res.pipe(file);
    res.on('error', reject);
    file.on('error', reject);
    file.on('finish', resolve);
  });

  await fs.promises.rename(tmp, outFile);
}

async function main() {
  const target = resolveTarget();
  if (!target) {
    console.warn(`[tokenusage] Unsupported platform: ${process.platform}/${process.arch}.`);
    console.warn('[tokenusage] Please install from source: cargo install tokenusage --bin tu');
    return;
  }

  const isWindows = process.platform === 'win32';
  const assetName = isWindows
    ? `tu-${VERSION_TAG}-${target}.exe`
    : `tu-${VERSION_TAG}-${target}`;

  const url = `https://github.com/${OWNER}/${REPO}/releases/download/${VERSION_TAG}/${assetName}`;
  const outFile = path.join(__dirname, '..', 'vendor', isWindows ? 'tu.exe' : 'tu');

  try {
    await downloadWithRedirect(url, outFile);
    if (!isWindows) {
      await fs.promises.chmod(outFile, 0o755);
    }
    console.log(`[tokenusage] Installed ${assetName}`);
  } catch (err) {
    console.warn(`[tokenusage] Failed to download release binary: ${err.message}`);
    console.warn('[tokenusage] You can still build/install via cargo install tokenusage --bin tu');
  }
}

main();
