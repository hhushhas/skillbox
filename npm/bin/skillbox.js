#!/usr/bin/env node

"use strict";

const { createHash } = require("node:crypto");
const {
  chmodSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  renameSync,
  rmSync,
  writeFileSync,
} = require("node:fs");
const { homedir } = require("node:os");
const { join } = require("node:path");
const { spawnSync } = require("node:child_process");
const https = require("node:https");

const metadata = require("../package.json");
const OWNER = "hhushhas";
const REPOSITORY = "skillbox";
const RELEASE_BASE = `https://github.com/${OWNER}/${REPOSITORY}/releases/download/v${metadata.version}`;

function artifactTarget(platform = process.platform, architecture = process.arch) {
  const targets = {
    "darwin:arm64": "aarch64-apple-darwin",
    "darwin:x64": "x86_64-apple-darwin",
    "linux:arm64": "aarch64-unknown-linux-gnu",
    "linux:x64": "x86_64-unknown-linux-gnu",
    "win32:x64": "x86_64-pc-windows-msvc",
  };
  const target = targets[`${platform}:${architecture}`];
  if (!target) throw new Error(`unsupported platform: ${platform}/${architecture}`);
  return target;
}

function artifactName(target, platform = process.platform) {
  const extension = platform === "win32" ? "zip" : "tar.gz";
  return `skillbox-${metadata.version}-${target}.${extension}`;
}

function cacheDirectory(target) {
  const base = process.env.SKILLBOX_NPM_CACHE_DIR
    || process.env.XDG_CACHE_HOME
    || join(homedir(), ".cache");
  return join(base, "skillbox", "npm", metadata.version, target);
}

function download(url, redirects = 0) {
  if (redirects > 5) return Promise.reject(new Error("too many redirects while downloading Skillbox"));
  return new Promise((resolve, reject) => {
    https.get(url, { headers: { "user-agent": "skillbox-npm-launcher" } }, (response) => {
      const code = response.statusCode || 0;
      if (code >= 300 && code < 400 && response.headers.location) {
        response.resume();
        download(new URL(response.headers.location, url).href, redirects + 1).then(resolve, reject);
        return;
      }
      if (code !== 200) {
        response.resume();
        reject(new Error(`download failed with HTTP ${code}: ${url}`));
        return;
      }
      const chunks = [];
      response.on("data", (chunk) => chunks.push(chunk));
      response.on("end", () => resolve(Buffer.concat(chunks)));
      response.on("error", reject);
    }).on("error", reject);
  });
}

function expectedChecksum(checksums, asset) {
  const line = checksums
    .toString("utf8")
    .split(/\r?\n/)
    .find((candidate) => candidate.trimEnd().endsWith(`  ${asset}`) || candidate.trimEnd().endsWith(` *${asset}`));
  const checksum = line?.trim().split(/\s+/)[0];
  if (!checksum || !/^[a-f0-9]{64}$/i.test(checksum)) {
    throw new Error(`release checksum missing for ${asset}`);
  }
  return checksum.toLowerCase();
}

function extract(archive, destination, isWindows) {
  const argumentsForTar = isWindows
    ? ["-xf", archive, "-C", destination]
    : ["-xzf", archive, "-C", destination];
  const result = spawnSync("tar", argumentsForTar, { encoding: "utf8" });
  if (result.error || result.status !== 0) {
    const detail = result.error?.message || result.stderr?.trim() || `exit code ${result.status}`;
    throw new Error(`failed to extract the Skillbox release: ${detail}`);
  }
}

async function downloadBinary() {
  const target = artifactTarget();
  const isWindows = process.platform === "win32";
  const binaryName = isWindows ? "skillbox.exe" : "skillbox";
  const asset = artifactName(target);
  const directory = cacheDirectory(target);
  const cached = join(directory, binaryName);
  if (existsSync(cached)) return cached;

  mkdirSync(directory, { recursive: true, mode: 0o700 });
  const temporary = mkdtempSync(join(directory, ".download-"));
  try {
    const [archiveBytes, checksumBytes] = await Promise.all([
      download(`${RELEASE_BASE}/${asset}`),
      download(`${RELEASE_BASE}/SHA256SUMS`),
    ]);
    const expected = expectedChecksum(checksumBytes, asset);
    const actual = createHash("sha256").update(archiveBytes).digest("hex");
    if (actual !== expected) throw new Error(`checksum mismatch for ${asset}`);

    const archivePath = join(temporary, asset);
    const extracted = join(temporary, "extracted");
    writeFileSync(archivePath, archiveBytes, { mode: 0o600 });
    mkdirSync(extracted, { mode: 0o700 });
    extract(archivePath, extracted, isWindows);
    const unpacked = join(extracted, binaryName);
    if (!existsSync(unpacked)) throw new Error(`release archive did not contain ${binaryName}`);
    renameSync(unpacked, cached);
    if (!isWindows) chmodSync(cached, 0o755);
    return cached;
  } finally {
    rmSync(temporary, { recursive: true, force: true });
  }
}

async function resolveBinary() {
  if (process.env.SKILLBOX_NPM_BINARY) return process.env.SKILLBOX_NPM_BINARY;
  return downloadBinary();
}

async function main() {
  const binary = await resolveBinary();
  const result = spawnSync(binary, process.argv.slice(2), {
    env: process.env,
    stdio: "inherit",
  });
  if (result.error) throw result.error;
  process.exitCode = result.status ?? 1;
}

if (require.main === module) {
  main().catch((error) => {
    console.error(`skillbox npm launcher: ${error.message}`);
    process.exitCode = 1;
  });
}

module.exports = { artifactName, artifactTarget, expectedChecksum };
