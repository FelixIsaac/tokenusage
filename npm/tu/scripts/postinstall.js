const fs = require('node:fs');
const path = require('node:path');
const https = require('node:https');
const { execSync } = require('node:child_process');

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
  const vendorDir = path.join(__dirname, '..', 'vendor');
  const outFile = path.join(vendorDir, isWindows ? 'tu.exe' : 'tu');

  if (isWindows) {
    // Windows: download bare .exe directly.
    const assetName = `tu-${VERSION_TAG}-${target}.exe`;
    const url = `https://github.com/${OWNER}/${REPO}/releases/download/${VERSION_TAG}/${assetName}`;

    try {
      await downloadWithRedirect(url, outFile);
      console.log(`[tokenusage] Installed ${assetName}`);
    } catch (err) {
      console.warn(`[tokenusage] Failed to download release binary: ${err.message}`);
      console.warn('[tokenusage] You can still build/install via cargo install tokenusage --bin tu');
    }
  } else {
    // macOS / Linux: download tar.gz archive and extract the binary.
    const assetName = `tu-${VERSION_TAG}-${target}.tar.gz`;
    const url = `https://github.com/${OWNER}/${REPO}/releases/download/${VERSION_TAG}/${assetName}`;
    const archivePath = path.join(vendorDir, assetName);

    try {
      await downloadWithRedirect(url, archivePath);

      // Extract: archive contains <dir>/tu — extract just the binary.
      const innerDir = `tu-${VERSION_TAG}-${target}`;
      execSync(`tar -xzf "${archivePath}" -C "${vendorDir}" "${innerDir}/tu"`, {
        stdio: 'pipe',
      });

      // Move the binary from the extracted subdirectory to vendor/.
      const extractedBin = path.join(vendorDir, innerDir, 'tu');
      await fs.promises.rename(extractedBin, outFile);
      await fs.promises.chmod(outFile, 0o755);

      // Clean up archive and extracted directory.
      await fs.promises.rm(archivePath, { force: true });
      await fs.promises.rm(path.join(vendorDir, innerDir), { recursive: true, force: true });

      console.log(`[tokenusage] Installed tu from ${assetName}`);
    } catch (err) {
      // Clean up on failure.
      try { await fs.promises.rm(archivePath, { force: true }); } catch {}
      console.warn(`[tokenusage] Failed to download release binary: ${err.message}`);
      console.warn('[tokenusage] You can still build/install via cargo install tokenusage --bin tu');
    }
  }
}

main();
