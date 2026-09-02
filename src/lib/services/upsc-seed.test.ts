import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { StoredQuestionBank } from "$lib/types";

const services = vi.hoisted(() => ({
  archiveMissingBundledQuestionBanks: vi.fn(),
  getQuestionBanks: vi.fn(),
  syncBundledQuestionBank: vi.fn(),
}));

vi.mock("$lib/services/question-bank", () => services);
vi.mock("$lib/services/logger", () => ({ logError: vi.fn() }));

import { seedUpscBanksIfNeeded } from "$lib/services/upsc-seed";

const paperJson = JSON.stringify({
  metadata: {
    name: "Mains GS Paper II · 2025",
    exam: "UPSC CSE",
    year: 2025,
    stage: "Mains",
    paper: "GS2",
    section: "mains-gs2",
    totalQuestions: 1,
    defaultDuration: 10800,
    difficulty: "hard",
    contentVersion: 47,
    taxonomyVersion: 4,
  },
  questions: [],
});

const catalog = {
  contentVersion: 55,
  taxonomyVersion: 4,
  papers: [
    {
      path: "mains-gs2/2025.json",
      name: "Mains GS Paper II · 2025",
      exam: "UPSC CSE",
      year: 2025,
      stage: "Mains",
      paper: "GS2",
      section: "mains-gs2",
      totalQuestions: 1,
      defaultDuration: 10800,
      practiceMode: "descriptive",
      difficulty: "hard",
      contentVersion: 47,
    },
  ],
};

async function paperContentHash(): Promise<string> {
  const digest = await globalThis.crypto.subtle.digest(
    "SHA-256",
    new TextEncoder().encode(paperJson),
  );
  return Array.from(new Uint8Array(digest), (byte) =>
    byte.toString(16).padStart(2, "0"),
  ).join("");
}

function storedBank(metadata: Record<string, unknown>): StoredQuestionBank {
  return {
    id: "bank-id",
    name: "Mains GS Paper II · 2025",
    exam: "UPSC CSE",
    metadata: JSON.stringify({
      year: 2025,
      paper: "GS2",
      section: "mains-gs2",
      sourceId: "upsc_2025_gs2",
      taxonomyVersion: 4,
      ...metadata,
    }),
    totalQuestions: 1,
    difficulty: "hard",
    defaultDuration: 10800,
    importedAt: 1,
  };
}

describe("UPSC bundled seeding", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    services.archiveMissingBundledQuestionBanks.mockResolvedValue(0);
    services.syncBundledQuestionBank.mockResolvedValue({
      success: true,
      imported: true,
      bankId: "new-bank-id",
    });
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: string | URL | Request) => {
        const url = String(input);
        return new Response(
          url.endsWith("catalog.json") ? JSON.stringify(catalog) : paperJson,
          { status: 200, headers: { "content-type": "application/json" } },
        );
      }),
    );
  });

  afterEach(() => vi.unstubAllGlobals());

  it("imports a corrected paper as an immutable revision", async () => {
    const contentHash = await paperContentHash();
    services.getQuestionBanks.mockResolvedValue([
      storedBank({
        contentVersion: 47,
        bundledCatalogVersion: 55,
        bundledContentHash: "0".repeat(64),
      }),
    ]);

    await expect(seedUpscBanksIfNeeded()).resolves.toEqual({
      imported: 1,
      updated: 0,
      failed: 0,
    });
    expect(services.syncBundledQuestionBank).toHaveBeenCalledWith(
      paperJson,
      "mains-gs2:2025:GS2",
      contentHash,
      55,
      47,
    );
  });

  it("does not import an exact paper and taxonomy revision again", async () => {
    const contentHash = await paperContentHash();
    services.getQuestionBanks.mockResolvedValue([
      storedBank({
        contentVersion: 47,
        bundledCatalogVersion: 55,
        bundledContentHash: contentHash,
      }),
    ]);

    await expect(seedUpscBanksIfNeeded()).resolves.toEqual({
      imported: 0,
      updated: 0,
      failed: 0,
    });
    expect(services.syncBundledQuestionBank).not.toHaveBeenCalled();
    expect(services.archiveMissingBundledQuestionBanks).toHaveBeenCalledWith([
      "mains-gs2:2025:GS2",
    ]);
  });

  it("imports a new revision when only taxonomy changed", async () => {
    const contentHash = await paperContentHash();
    services.getQuestionBanks.mockResolvedValue([
      storedBank({
        taxonomyVersion: 3,
        contentVersion: 47,
        bundledCatalogVersion: 55,
        bundledContentHash: contentHash,
      }),
    ]);

    await seedUpscBanksIfNeeded();

    expect(services.syncBundledQuestionBank).toHaveBeenCalledOnce();
  });
});
