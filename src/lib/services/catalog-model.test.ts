import { describe, expect, it } from "vitest";

import {
  MAINS_PAPER_TYPES,
  PRELIMS_PAPER_TYPES,
} from "$lib/constants/upsc-catalog";
import {
  banksForSelectedOptionals,
  mainsPaperTypesForOptionals,
  searchScope,
} from "$lib/services/catalog-model";
import type { StoredQuestionBank } from "$lib/types";

function bank(section: string, totalQuestions: number): StoredQuestionBank {
  return {
    id: section,
    name: section,
    exam: "UPSC CSE",
    metadata: JSON.stringify({ section, year: 2025 }),
    totalQuestions,
    difficulty: "medium",
    defaultDuration: 0,
    importedAt: 0,
  };
}

describe("catalog optional visibility", () => {
  it("keeps core papers and only selected optional paper types", () => {
    const visible = mainsPaperTypesForOptionals(["math", "philosophy"]);

    expect(visible.map((paper) => paper.id)).toEqual([
      "essay",
      "gs1",
      "gs2",
      "gs3",
      "gs4",
      "math",
      "philosophy",
    ]);
  });

  it("filters optional banks while preserving prelims and core mains totals", () => {
    const banks = [
      bank("prelims-gs1", 100),
      bank("mains-gs2", 20),
      bank("mains-maths1", 8),
      bank("mains-maths2", 8),
      bank("mains-philosophy1", 6),
    ];

    const visible = banksForSelectedOptionals(banks, ["math"]);

    expect(visible.map((item) => item.id)).toEqual([
      "prelims-gs1",
      "mains-gs2",
      "mains-maths1",
      "mains-maths2",
    ]);
    expect(
      visible.reduce((total, item) => total + item.totalQuestions, 0),
    ).toBe(136);
  });
});

describe("catalog search scope", () => {
  it("keeps CSAT and every optional out of the default all-paper search", () => {
    const scope = searchScope({ kind: "home" });

    expect(scope.sections).not.toContain("prelims-csat");
    expect(scope.sections).not.toContain("mains-maths1");
    expect(scope.sections).not.toContain("mains-maths2");
    expect(scope.sections).not.toContain("mains-geography1");
    expect(scope.sections).not.toContain("mains-philosophy2");
    expect(scope.sections).toContain("prelims-gs1");
    expect(scope.sections).toContain("mains-gs2");
  });

  it("adds only selected optionals when optional results are enabled", () => {
    const scope = searchScope({ kind: "home" }, ["math", "philosophy"], true);

    expect(scope.sections).toEqual(
      expect.arrayContaining([
        "mains-maths1",
        "mains-maths2",
        "mains-philosophy1",
        "mains-philosophy2",
      ]),
    );
    expect(scope.sections).not.toContain("mains-geography1");
  });

  it("does not add selected optionals until optional results are enabled", () => {
    const scope = searchScope({ kind: "mains" }, ["math"], false);

    expect(scope.sections).not.toContain("mains-maths1");
  });

  it("returns no searchable sections for a stale unselected optional screen", () => {
    const math = MAINS_PAPER_TYPES.find((paper) => paper.id === "math");
    expect(math).toBeDefined();

    expect(
      searchScope({ kind: "mains-paper", paper: math! }, [], true).sections,
    ).toEqual([]);
  });

  it("keeps CSAT searchable from its dedicated paper", () => {
    const csat = PRELIMS_PAPER_TYPES.find((paper) => paper.id === "csat");
    expect(csat).toBeDefined();

    expect(
      searchScope({ kind: "prelims-paper", paper: csat! }).sections,
    ).toEqual(["prelims-csat"]);
  });

  it("keeps CSAT out of aggregate Prelims search", () => {
    expect(searchScope({ kind: "prelims" }).sections).toEqual(["prelims-gs1"]);
    expect(searchScope({ kind: "prelims-history" }).sections).toEqual([
      "prelims-gs1",
    ]);
  });
});
