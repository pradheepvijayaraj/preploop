#!/usr/bin/env bun
/** Collect the complete installer matrix without bundler staging files. */

import {
  copyFileSync,
  existsSync,
  lstatSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  writeFileSync,
} from "node:fs";
import { createHash } from "node:crypto";
import { basename, join, resolve } from "node:path";

type Environment = Record<string, string | undefined>;
type ArtifactFiles = Map<string, Map<string, string>>;

interface TauriConfig {
  version: string;
  plugins: {
    updater: {
      pubkey: string;
    };
  };
}

interface PlatformRelease {
  url: string;
  signature: string;
}

function requireEnvironment(environment: Environment, name: string): string {
  const value = environment[name];
  if (!value) throw new Error(`Missing required environment variable: ${name}`);
  return value;
}

function decodeBase64(value: string, label: string): Buffer {
  const valid =
    /^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/;
  if (!valid.test(value)) throw new Error(`Invalid base64 in ${label}`);
  return Buffer.from(value, "base64");
}

function decodeUtf8(value: Buffer, label: string): string {
  try {
    return new TextDecoder("utf-8", { fatal: true }).decode(value);
  } catch {
    throw new Error(`Invalid UTF-8 in ${label}`);
  }
}

function secondLine(value: string, label: string): string {
  const line = value.split(/\r\n|\n|\r/)[1];
  if (line === undefined) throw new Error(`Missing encoded key in ${label}`);
  return line;
}

function collectMatches(root: string, suffix: string): string[] {
  const matches: string[] = [];
  for (const entry of readdirSync(root, { withFileTypes: true })) {
    const path = join(root, entry.name);
    if (entry.isDirectory()) {
      matches.push(...collectMatches(path, suffix));
    } else if (entry.name.endsWith(suffix)) {
      matches.push(path);
    }
  }
  return matches.sort();
}

function releaseUrlPart(value: string): string {
  return encodeURIComponent(value).replace(
    /[!'()*]/g,
    (character) => `%${character.charCodeAt(0).toString(16).toUpperCase()}`,
  );
}

function sha256(path: string): string {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

export function prepareReleaseAssets(
  source: string,
  destination: string,
  environment: Environment = process.env,
  root = process.cwd(),
): string {
  const expected = new Map<string, readonly string[]>();
  for (const arch of ["x64", "arm64"] as const) {
    expected.set(`PrepLoop-macOS-${arch}`, [
      ".dmg",
      ".app.tar.gz",
      ".app.tar.gz.sig",
    ]);
    expected.set(`PrepLoop-Windows-${arch}`, [".exe", ".exe.sig"]);
    expected.set(`PrepLoop-Linux-${arch}-AppImage`, [
      ".AppImage",
      ".AppImage.sig",
    ]);
  }

  const tag = requireEnvironment(environment, "RELEASE_TAG");
  const repo = requireEnvironment(environment, "RELEASE_REPO");
  const config = JSON.parse(
    readFileSync(resolve(root, "src-tauri/tauri.conf.json"), "utf8"),
  ) as TauriConfig;
  const version = config.version;
  const publicText = decodeUtf8(
    decodeBase64(config.plugins.updater.pubkey, "updater public key"),
    "updater public key",
  );
  const publicKey = decodeBase64(
    secondLine(publicText, "updater public key"),
    "updater public key",
  );
  if (tag !== `v${version}`) {
    throw new Error("Release tag must match the app version");
  }

  // Validate all six installers and their updater-only artifacts. Signatures
  // are embedded in latest.json instead of appearing as normal downloads.
  const artifactFiles: ArtifactFiles = new Map();
  for (const [artifact, suffixes] of expected) {
    const files = new Map<string, string>();
    for (const suffix of [...suffixes].sort()) {
      const matches = collectMatches(resolve(source, artifact), suffix);
      if (matches.length !== 1) {
        throw new Error(
          `${artifact}: expected one ${suffix}, found ${matches.length}`,
        );
      }
      const path = matches[0];
      const stat = lstatSync(path);
      if (!stat.isFile() || stat.isSymbolicLink() || stat.size === 0) {
        throw new Error(`Invalid or empty installer: ${path}`);
      }
      files.set(suffix, path);
    }
    artifactFiles.set(artifact, files);
  }

  const releaseAssets = new Map<string, string>();
  const releaseNames = new Map<string, string>();
  for (const [artifact, files] of artifactFiles) {
    for (const [suffix, path] of files) {
      if (suffix.endsWith(".sig")) continue;
      let releaseName = basename(path);
      if (suffix === ".app.tar.gz") {
        const arch = artifact.endsWith("-x64") ? "x64" : "aarch64";
        const appName = releaseName.slice(0, -".app.tar.gz".length);
        releaseName = `${appName}_${version}_${arch}.app.tar.gz`;
      }
      if (releaseAssets.has(releaseName)) {
        throw new Error(`Duplicate release filename: ${releaseName}`);
      }
      releaseAssets.set(releaseName, path);
      releaseNames.set(`${artifact}\0${suffix}`, releaseName);
    }
  }

  const platforms: Record<string, PlatformRelease> = {};
  const updaterTargets = [
    ["PrepLoop-macOS-x64", "darwin-x86_64", ".app.tar.gz"],
    ["PrepLoop-macOS-arm64", "darwin-aarch64", ".app.tar.gz"],
    ["PrepLoop-Windows-x64", "windows-x86_64", ".exe"],
    ["PrepLoop-Windows-arm64", "windows-aarch64", ".exe"],
    ["PrepLoop-Linux-x64-AppImage", "linux-x86_64", ".AppImage"],
    ["PrepLoop-Linux-arm64-AppImage", "linux-aarch64", ".AppImage"],
  ] as const;

  for (const [artifact, target, suffix] of updaterTargets) {
    const files = artifactFiles.get(artifact);
    const path = files?.get(suffix);
    const signaturePath = files?.get(`${suffix}.sig`);
    const releaseName = releaseNames.get(`${artifact}\0${suffix}`);
    if (!path || !signaturePath || !releaseName) {
      throw new Error(
        `Missing validated updater artifact: ${artifact} ${suffix}`,
      );
    }

    const signature = readFileSync(signaturePath, "utf8").trim();
    const signatureText = decodeUtf8(
      decodeBase64(signature, `${basename(path)} signature`),
      `${basename(path)} signature`,
    );
    const signatureBytes = decodeBase64(
      secondLine(signatureText, `${basename(path)} signature`),
      `${basename(path)} signature`,
    );
    if (
      signatureBytes.length !== 74 ||
      !signatureBytes.subarray(2, 10).equals(publicKey.subarray(2, 10))
    ) {
      throw new Error(
        `Updater signature does not match the app key: ${basename(path)}`,
      );
    }
    platforms[target] = {
      url: `https://github.com/${repo}/releases/download/${releaseUrlPart(tag)}/${releaseUrlPart(releaseName)}`,
      signature,
    };
  }

  // Validate the complete matrix before writing anything; refuse stale output.
  if (existsSync(destination)) {
    throw new Error(`Release destination already exists: ${destination}`);
  }
  mkdirSync(destination, { recursive: true });
  const manifest = join(destination, "latest.json");
  writeFileSync(
    manifest,
    `${JSON.stringify({ version, platforms }, null, 2)}\n`,
    "utf8",
  );

  const checksums = [`${sha256(manifest)}  latest.json\n`];
  for (const [name, path] of [...releaseAssets.entries()].sort(
    ([left], [right]) => left.localeCompare(right),
  )) {
    const target = join(destination, name);
    copyFileSync(path, target);
    checksums.push(`${sha256(target)}  ${name}\n`);
  }
  writeFileSync(
    join(destination, "SHA256SUMS.txt"),
    checksums.join(""),
    "utf8",
  );

  return (
    "Prepared 6 installers, 2 macOS updater packages, latest.json and " +
    "SHA256SUMS.txt (signatures embedded in latest.json)"
  );
}

if (import.meta.main) {
  const [source, destination] = process.argv.slice(2);
  if (!source || !destination) {
    console.error(
      "Usage: bun scripts/prepare-release-assets.ts <source> <destination>",
    );
    process.exit(2);
  }

  try {
    console.log(prepareReleaseAssets(source, destination));
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    console.error(`Release asset preparation failed: ${message}`);
    process.exit(1);
  }
}
