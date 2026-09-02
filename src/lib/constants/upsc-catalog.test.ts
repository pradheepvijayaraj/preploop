import { describe, expect, it } from "vitest";
import {
  ACTIVE_UPSC_SECTIONS,
  MAINS_PAPER_TYPES,
  PRELIMS_PAPER_TYPES,
} from "./upsc-catalog";

describe("UPSC catalog", () => {
  it("contains core Mains papers plus fully verified optional pairs", () => {
    expect(MAINS_PAPER_TYPES.map((paper) => paper.id)).toEqual([
      "essay",
      "gs1",
      "gs2",
      "gs3",
      "gs4",
      "anthropology",
      "commerce",
      "economics",
      "geography",
      "history",
      "law",
      "math",
      "medical",
      "philosophy",
      "psir",
      "pubad",
      "sociology",
    ]);

    expect(
      MAINS_PAPER_TYPES.filter((paper) => paper.optional)
        .map((p) => p.id)
        .sort(),
    ).toEqual(
      [
        "economics",
        "anthropology",
        "commerce",
        "geography",
        "history",
        "law",
        "math",
        "medical",
        "philosophy",
        "psir",
        "pubad",
        "sociology",
      ].sort(),
    );
  });

  it("exposes exactly the sections backed by the retained papers", () => {
    const expectedSections = [
      ...PRELIMS_PAPER_TYPES.map((paper) => paper.section),
      ...MAINS_PAPER_TYPES.flatMap((paper) => paper.sections),
    ];

    expect([...ACTIVE_UPSC_SECTIONS]).toEqual(expectedSections);
    expect(expectedSections).toHaveLength(31);
  });
});
