#!/usr/bin/env bun
/** Validate PrepLoop's synchronized application and release versions. */

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { TOML } from "bun";

const SEMVER =
  /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-((?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*)(?:\.(?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*))*))?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/;

type Environment = Record<string, string | undefined>;
type ParsedVersion = readonly [bigint, bigint, bigint, string | null];
type UnknownRecord = Record<string, unknown>;

function asRecord(value: unknown, label: string): UnknownRecord {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error(`Expected ${label} to be an object`);
  }
  return value as UnknownRecord;
}

function asString(value: unknown, label: string): string {
  if (typeof value !== "string") {
    throw new Error(`Expected ${label} to be a string`);
  }
  return value;
}

function readJson(path: string): UnknownRecord {
  return asRecord(JSON.parse(readFileSync(path, "utf8")), path);
}

export function parseVersion(value: string): ParsedVersion {
  const match = SEMVER.exec(value);
  if (match === null) {
    throw new Error(`Invalid semantic version: ${value}`);
  }
  return [
    BigInt(match[1]),
    BigInt(match[2]),
    BigInt(match[3]),
    match[4] ?? null,
  ];
}

export function isNewer(candidate: string, previous: string): boolean {
  const candidateParts = parseVersion(candidate);
  const previousParts = parseVersion(previous);
  const candidateCore: readonly [bigint, bigint, bigint] = [
    candidateParts[0],
    candidateParts[1],
    candidateParts[2],
  ];
  const previousCore: readonly [bigint, bigint, bigint] = [
    previousParts[0],
    previousParts[1],
    previousParts[2],
  ];

  for (let index = 0; index < 3; index += 1) {
    if (candidateCore[index] !== previousCore[index]) {
      return candidateCore[index] > previousCore[index];
    }
  }

  const candidatePre = candidateParts[3];
  const previousPre = previousParts[3];
  if (candidatePre === null || previousPre === null) {
    return candidatePre === null && previousPre !== null;
  }

  const candidateIdentifiers = candidatePre.split(".");
  const previousIdentifiers = previousPre.split(".");
  const sharedLength = Math.min(
    candidateIdentifiers.length,
    previousIdentifiers.length,
  );
  for (let index = 0; index < sharedLength; index += 1) {
    const candidateIdentifier = candidateIdentifiers[index];
    const previousIdentifier = previousIdentifiers[index];
    if (candidateIdentifier === previousIdentifier) continue;

    const candidateNumeric = /^\d+$/.test(candidateIdentifier);
    const previousNumeric = /^\d+$/.test(previousIdentifier);
    if (candidateNumeric && previousNumeric) {
      return BigInt(candidateIdentifier) > BigInt(previousIdentifier);
    }
    if (candidateNumeric !== previousNumeric) {
      return !candidateNumeric;
    }
    return candidateIdentifier > previousIdentifier;
  }
  return candidateIdentifiers.length > previousIdentifiers.length;
}

export function applicationVersions(
  root = process.cwd(),
): Record<string, string> {
  const packageJson = readJson(resolve(root, "package.json"));
  const tauriPath = resolve(root, "src-tauri/tauri.conf.json");
  const tauri = readJson(tauriPath);
  const cargoPath = resolve(root, "src-tauri/Cargo.toml");
  const cargo = asRecord(
    TOML.parse(readFileSync(cargoPath, "utf8")),
    cargoPath,
  );
  const cargoPackage = asRecord(cargo.package, `${cargoPath} package`);
  const lockPath = resolve(root, "src-tauri/Cargo.lock");
  const lock = asRecord(TOML.parse(readFileSync(lockPath, "utf8")), lockPath);
  if (!Array.isArray(lock.package)) {
    throw new Error(`Expected ${lockPath} package to be an array`);
  }

  const lockMatches = lock.package
    .map((item, index) => asRecord(item, `${lockPath} package ${index}`))
    .filter((item) => item.name === "preploop")
    .map((item) => asString(item.version, `${lockPath} preploop version`));
  if (lockMatches.length !== 1) {
    throw new Error(
      `Expected one preploop package in Cargo.lock, found ${lockMatches.length}`,
    );
  }

  return {
    "package.json": asString(packageJson.version, "package.json version"),
    "src-tauri/tauri.conf.json": asString(
      tauri.version,
      "src-tauri/tauri.conf.json version",
    ),
    "src-tauri/Cargo.toml": asString(
      cargoPackage.version,
      "src-tauri/Cargo.toml package version",
    ),
    "src-tauri/Cargo.lock": lockMatches[0],
  };
}

export function checkVersion(
  root = process.cwd(),
  environment: Environment = process.env,
): string {
  const versions = applicationVersions(root);
  const distinct = new Set(Object.values(versions));
  if (distinct.size !== 1) {
    const details = Object.entries(versions)
      .map(([path, version]) => `${path}=${version}`)
      .join(", ");
    throw new Error(`Application versions do not match: ${details}`);
  }

  const version = distinct.values().next().value;
  if (version === undefined) {
    throw new Error("No application version was found");
  }
  parseVersion(version);

  const releaseTag = environment.RELEASE_TAG ?? "";
  if (releaseTag && releaseTag !== `v${version}`) {
    throw new Error(
      `Release tag ${releaseTag} does not match application version ${version}`,
    );
  }

  const latestTag = environment.LATEST_RELEASE_TAG ?? "";
  if (latestTag) {
    if (!latestTag.startsWith("v")) {
      throw new Error(`Latest release tag is not versioned: ${latestTag}`);
    }
    if (!isNewer(version, latestTag.slice(1))) {
      throw new Error(
        `Release version ${version} must be newer than ${latestTag.slice(1)}`,
      );
    }
  }

  let message = `Application version ${version} is synchronized`;
  if (releaseTag) message += ` and matches ${releaseTag}`;
  if (latestTag) message += ` (newer than ${latestTag})`;
  return message;
}

if (import.meta.main) {
  try {
    console.log(checkVersion());
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    console.error(`Version check failed: ${message}`);
    process.exit(1);
  }
}
