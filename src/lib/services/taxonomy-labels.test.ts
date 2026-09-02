import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const logger = vi.hoisted(() => ({ logError: vi.fn() }));

vi.mock("$lib/services/logger", () => logger);

describe("taxonomy label loader", () => {
  beforeEach(() => {
    vi.resetModules();
    vi.clearAllMocks();
  });

  afterEach(() => vi.unstubAllGlobals());

  it("loads the public taxonomy once and shares parsed label maps", async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(
        JSON.stringify({
          mainTags: [{ id: 440, label: "Linear Algebra" }],
          subtags: [{ id: 283, label: "Vector Spaces" }],
        }),
        { status: 200 },
      ),
    );
    vi.stubGlobal("fetch", fetchMock);

    const { loadTaxonomyLabels } = await import("./taxonomy-labels");
    const [first, second] = await Promise.all([
      loadTaxonomyLabels(),
      loadTaxonomyLabels(),
    ]);

    expect(fetchMock).toHaveBeenCalledOnce();
    expect(fetchMock).toHaveBeenCalledWith("/upsc/taxonomy.json");
    expect(first).toBe(second);
    expect(first.mainTags.get(440)).toBe("Linear Algebra");
    expect(first.subtags.get(283)).toBe("Vector Spaces");
  });

  it("returns empty fallback maps and logs malformed public data", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        new Response(JSON.stringify({ mainTags: {}, subtags: [] }), {
          status: 200,
        }),
      ),
    );

    const { loadTaxonomyLabels } = await import("./taxonomy-labels");
    const labels = await loadTaxonomyLabels();

    expect(labels.mainTags.size).toBe(0);
    expect(labels.subtags.size).toBe(0);
    expect(logger.logError).toHaveBeenCalledOnce();
  });
});
