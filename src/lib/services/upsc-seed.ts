/**
 * Seed bundled UPSC CSE papers into SQLite.
 *
 * Each paper is compared by SHA-256 and imported as a new immutable revision.
 * Historical revisions remain available to existing attempts.
 */
import {
  archiveMissingBundledQuestionBanks,
  getQuestionBanks,
  syncBundledQuestionBank,
} from "$lib/services/question-bank";
import { markUninterruptible } from "$lib/services/loading-timeout";
import { logError } from "$lib/services/logger";

export interface UpscCatalogEntry {
  path: string;
  name: string;
  exam: string;
  year: number;
  stage: string;
  paper: string;
  section: string;
  totalQuestions: number;
  defaultDuration: number;
  practiceMode: "mcq" | "descriptive";
  difficulty: string;
  contentVersion?: number;
}

interface UpscCatalogFile {
  contentVersion: number;
  taxonomyVersion: number;
  papers: UpscCatalogEntry[];
}

const CATALOG_URL = "/upsc/catalog.json";

/** Expected seed shape version; bump with conversion script. */
export const UPSC_CONTENT_VERSION = 55;
export const UPSC_TAXONOMY_VERSION = 4;
let seedPromise: Promise<SeedResult> | null = null;

export interface SeedResult {
  imported: number;
  updated: number;
  failed: number;
}

interface StoredBundledMetadata {
  contentVersion?: unknown;
  bundledCatalogVersion?: unknown;
  bundledContentHash?: unknown;
  bundledCatalogKey?: unknown;
  sourceId?: unknown;
  taxonomyVersion?: unknown;
}

function storedBundledMetadata(metadataJson: string): StoredBundledMetadata {
  try {
    return JSON.parse(metadataJson) as StoredBundledMetadata;
  } catch {
    return {};
  }
}

function isManagedBundledMetadata(metadata: StoredBundledMetadata): boolean {
  return (
    typeof metadata.bundledCatalogKey === "string" ||
    (typeof metadata.sourceId === "string" &&
      metadata.sourceId.startsWith("upsc_"))
  );
}

function catalogEntryKey(entry: UpscCatalogEntry): string {
  return `${entry.section}:${entry.year}:${entry.paper}`;
}

function storedBankKey(metadataJson: string): string | null {
  try {
    const meta = JSON.parse(metadataJson) as {
      bundledCatalogKey?: unknown;
      section?: unknown;
      year?: unknown;
      paper?: unknown;
    };
    if (
      typeof meta.bundledCatalogKey === "string" &&
      meta.bundledCatalogKey.trim() !== ""
    ) {
      return meta.bundledCatalogKey;
    }
    if (
      typeof meta.section !== "string" ||
      typeof meta.year !== "number" ||
      typeof meta.paper !== "string"
    ) {
      return null;
    }
    return `${meta.section}:${meta.year}:${meta.paper}`;
  } catch {
    return null;
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isPositiveInteger(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value > 0;
}

function validateCatalogEntry(value: unknown, index: number): UpscCatalogEntry {
  const path = `papers[${index}]`;
  if (!isRecord(value)) {
    throw new Error(`Invalid UPSC catalog: ${path} must be an object`);
  }
  for (const field of [
    "path",
    "name",
    "exam",
    "stage",
    "paper",
    "section",
    "difficulty",
  ] as const) {
    if (typeof value[field] !== "string" || value[field].trim() === "") {
      throw new Error(
        `Invalid UPSC catalog: ${path}.${field} must be a non-empty string`,
      );
    }
  }
  const paperPath = value.path as string;
  if (
    paperPath.startsWith("/") ||
    paperPath.split("/").includes("..") ||
    !paperPath.endsWith(".json")
  ) {
    throw new Error(
      `Invalid UPSC catalog: ${path}.path must be a relative JSON path`,
    );
  }
  if (value.exam !== "UPSC CSE") {
    throw new Error(`Invalid UPSC catalog: ${path}.exam must be UPSC CSE`);
  }
  if (value.stage !== "Prelims" && value.stage !== "Mains") {
    throw new Error(`Invalid UPSC catalog: ${path}.stage is unsupported`);
  }
  if (!["easy", "medium", "hard"].includes(value.difficulty as string)) {
    throw new Error(`Invalid UPSC catalog: ${path}.difficulty is unsupported`);
  }
  for (const field of ["year", "totalQuestions", "defaultDuration"] as const) {
    if (!isPositiveInteger(value[field])) {
      throw new Error(
        `Invalid UPSC catalog: ${path}.${field} must be a positive integer`,
      );
    }
  }
  if (value.practiceMode !== "mcq" && value.practiceMode !== "descriptive") {
    throw new Error(
      `Invalid UPSC catalog: ${path}.practiceMode is unsupported`,
    );
  }
  if (
    value.contentVersion !== undefined &&
    !isPositiveInteger(value.contentVersion)
  ) {
    throw new Error(
      `Invalid UPSC catalog: ${path}.contentVersion must be a positive integer`,
    );
  }
  return {
    path: value.path as string,
    name: value.name as string,
    exam: value.exam as string,
    year: value.year as number,
    stage: value.stage as string,
    paper: value.paper as string,
    section: value.section as string,
    totalQuestions: value.totalQuestions as number,
    defaultDuration: value.defaultDuration as number,
    practiceMode: value.practiceMode as "mcq" | "descriptive",
    difficulty: value.difficulty as string,
    ...(value.contentVersion !== undefined
      ? { contentVersion: value.contentVersion as number }
      : {}),
  };
}

function validateCatalog(
  contentVersion: unknown,
  taxonomyVersion: unknown,
  papers: unknown,
): UpscCatalogFile {
  if (!isPositiveInteger(contentVersion)) {
    throw new Error(
      "Invalid UPSC catalog: contentVersion must be a positive integer",
    );
  }
  if (
    typeof taxonomyVersion !== "number" ||
    !Number.isSafeInteger(taxonomyVersion) ||
    taxonomyVersion < 0
  ) {
    throw new Error(
      "Invalid UPSC catalog: taxonomyVersion must be a non-negative integer",
    );
  }
  if (!Array.isArray(papers) || papers.length === 0) {
    throw new Error("Invalid UPSC catalog: papers must be a non-empty array");
  }
  const validated = papers.map(validateCatalogEntry);
  const keys = new Set<string>();
  for (const entry of validated) {
    const key = catalogEntryKey(entry);
    if (keys.has(key)) {
      throw new Error(`Invalid UPSC catalog: duplicate paper key ${key}`);
    }
    keys.add(key);
  }
  return { contentVersion, taxonomyVersion, papers: validated };
}

export async function loadUpscCatalog(): Promise<UpscCatalogFile> {
  const response = await fetch(CATALOG_URL);
  if (!response.ok) {
    throw new Error(`Failed to load UPSC catalog (${response.status})`);
  }
  const data: unknown = await response.json();
  // Back-compat: old catalogs were a bare array
  if (Array.isArray(data)) {
    return validateCatalog(1, 0, data);
  }
  if (!isRecord(data)) {
    throw new Error("Invalid UPSC catalog: expected an object");
  }
  return validateCatalog(
    data.contentVersion,
    data.taxonomyVersion,
    data.papers,
  );
}

/**
 * Ensure DB has current bundled UPSC papers.
 * Returns the number of imported and taxonomy-updated banks.
 */
export function seedUpscBanksIfNeeded(): Promise<SeedResult> {
  seedPromise ??= markUninterruptible(
    seedUpscBanks().finally(() => {
      seedPromise = null;
    }),
  );
  return seedPromise;
}

async function sha256Hex(content: string): Promise<string> {
  const digest = await globalThis.crypto.subtle.digest(
    "SHA-256",
    new TextEncoder().encode(content),
  );
  return Array.from(new Uint8Array(digest), (byte) =>
    byte.toString(16).padStart(2, "0"),
  ).join("");
}

async function seedUpscBanks(): Promise<SeedResult> {
  const catalog = await loadUpscCatalog();
  // catalog.contentVersion → backend's bundledCatalogVersion. The catalog
  // declares the version of its overall corpus; each paper may override with
  // its own contentVersion. Both are stamped by sync_bundled_question_bank.
  const targetVersion = catalog.contentVersion || UPSC_CONTENT_VERSION;
  const targetTaxonomyVersion =
    catalog.taxonomyVersion || UPSC_TAXONOMY_VERSION;
  const existing = await getQuestionBanks();
  const upscBanks = existing.filter((bank) => bank.exam === "UPSC CSE");
  const expectedKeys = new Set(catalog.papers.map(catalogEntryKey));
  const currentByKey = new Map<string, typeof upscBanks>();
  for (const bank of upscBanks) {
    if (!isManagedBundledMetadata(storedBundledMetadata(bank.metadata))) {
      continue;
    }
    const key = storedBankKey(bank.metadata);
    if (!key) continue;
    const grouped = currentByKey.get(key) ?? [];
    grouped.push(bank);
    currentByKey.set(key, grouped);
  }

  let imported = 0;
  let failed = 0;
  for (let i = 0; i < catalog.papers.length; i++) {
    const entry = catalog.papers[i]!;
    const catalogKey = catalogEntryKey(entry);
    const contentVersion = entry.contentVersion ?? targetVersion;
    try {
      const response = await fetch(`/upsc/${entry.path}`);
      if (!response.ok) {
        throw new Error(`HTTP ${response.status} for ${entry.path}`);
      }
      const jsonContent = await response.text();
      const contentHash = await sha256Hex(jsonContent);
      const current = currentByKey.get(catalogKey) ?? [];
      const currentMetadata =
        current.length === 1 ? storedBundledMetadata(current[0]!.metadata) : {};
      const alreadyCurrent =
        current.length === 1 &&
        currentMetadata.bundledContentHash === contentHash &&
        currentMetadata.bundledCatalogVersion === targetVersion &&
        currentMetadata.contentVersion === contentVersion &&
        currentMetadata.taxonomyVersion === targetTaxonomyVersion;
      if (alreadyCurrent) {
        continue;
      }

      const result = await syncBundledQuestionBank(
        jsonContent,
        catalogKey,
        contentHash,
        targetVersion,
        contentVersion,
      );
      if (result.success) {
        if (result.imported) imported += 1;
      } else {
        failed += 1;
        await logError(
          `Failed to import ${entry.path}`,
          result.error ?? result.validationErrors,
        );
      }
    } catch (error) {
      failed += 1;
      await logError(`Failed to seed ${entry.path}`, error);
    }
    if (i % 10 === 9) {
      await new Promise((r) => setTimeout(r, 0));
    }
  }

  try {
    await archiveMissingBundledQuestionBanks([...expectedKeys]);
  } catch (error) {
    failed += 1;
    await logError("Failed to archive removed UPSC papers", error);
  }

  return { imported, updated: 0, failed };
}
