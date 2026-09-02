import {
  ACTIVE_UPSC_SECTIONS,
  MAINS_PAPER_TYPES,
  PRELIMS_PAPER_TYPES,
  type MainsPaperType,
  type PrelimsPaperType,
} from "$lib/constants/upsc-catalog";
import type { StoredQuestionBank } from "$lib/types";

export type CatalogScreen =
  | { kind: "home" }
  | { kind: "prelims" }
  | { kind: "prelims-history" }
  | { kind: "prelims-paper"; paper: PrelimsPaperType }
  | { kind: "mains" }
  | { kind: "mains-paper"; paper: MainsPaperType };

export interface CatalogRouteState {
  history: CatalogScreen[];
  screen: CatalogScreen;
}

export interface PaperListItem {
  bank: StoredQuestionBank;
  year: number;
  label: string;
  kind: "prelims" | "theory";
}

export interface StoredBankMetadata {
  year: number;
  section: string;
  paper: string;
}

const DEFAULT_SEARCH_EXCLUDED_SECTIONS = new Set([
  "prelims-csat",
  ...MAINS_PAPER_TYPES.filter((paper) => paper.optional).flatMap(
    (paper) => paper.sections,
  ),
]);
const OPTIONAL_MAINS_SECTIONS = new Set(
  MAINS_PAPER_TYPES.filter((paper) => paper.optional).flatMap(
    (paper) => paper.sections,
  ),
);
const DEFAULT_SEARCH_SECTIONS = Array.from(ACTIVE_UPSC_SECTIONS).filter(
  (section) => !DEFAULT_SEARCH_EXCLUDED_SECTIONS.has(section),
);
const PRELIMS_GENERAL_STUDIES_SECTIONS = PRELIMS_PAPER_TYPES.filter(
  (paper) => paper.id !== "csat",
).map((paper) => paper.section);

export function inferSectionFromName(name: string): string {
  const lower = name.toLowerCase();
  if (
    lower.includes("csat") ||
    (lower.includes("gs paper ii") && lower.includes("prelim"))
  )
    return "prelims-csat";
  if (lower.includes("prelims")) return "prelims-gs1";
  if (lower.includes("essay")) return "mains-essay";
  if (lower.includes("gs paper iv") || lower.includes("gs paper 4"))
    return "mains-gs4";
  if (lower.includes("gs paper iii") || lower.includes("gs paper 3"))
    return "mains-gs3";
  if (lower.includes("gs paper ii") || lower.includes("gs paper 2"))
    return "mains-gs2";
  if (lower.includes("gs paper i") || lower.includes("gs paper 1"))
    return "mains-gs1";
  if (lower.includes("mathematics") || lower.includes("math")) {
    return lower.includes("paper ii") || lower.includes("paper 2")
      ? "mains-maths2"
      : "mains-maths1";
  }
  return "prelims-gs1";
}

export function parseBankMetadata(
  bank: StoredQuestionBank,
): StoredBankMetadata {
  let metadata: Record<string, unknown> = {};
  try {
    metadata = JSON.parse(bank.metadata) as Record<string, unknown>;
  } catch {
    // Legacy rows may contain malformed metadata; the name remains usable.
  }
  const section =
    typeof metadata.section === "string"
      ? metadata.section
      : inferSectionFromName(bank.name);
  return {
    year:
      typeof metadata.year === "number"
        ? metadata.year
        : Number((bank.name.match(/(\d{4})/) ?? [])[1] ?? 0),
    section,
    paper:
      typeof metadata.paper === "string"
        ? metadata.paper
        : section.includes("maths2")
          ? "MATHS2"
          : section.includes("maths1")
            ? "MATHS1"
            : "GS1",
  };
}

export function banksForSections(
  banks: StoredQuestionBank[],
  sections: string[],
): StoredQuestionBank[] {
  return banks
    .filter((bank) => sections.includes(parseBankMetadata(bank).section))
    .sort((a, b) => {
      const left = parseBankMetadata(a);
      const right = parseBankMetadata(b);
      return (
        right.year - left.year || left.section.localeCompare(right.section)
      );
    });
}

export function paperItems(
  banks: StoredQuestionBank[],
  sections: string[],
  kind: PaperListItem["kind"],
): PaperListItem[] {
  return banksForSections(banks, sections).map((bank) => {
    const year = parseBankMetadata(bank).year;
    return { bank, year, label: String(year), kind };
  });
}

export function mainsPaperTypesForOptionals(optionalSubjectIds: string[]) {
  return MAINS_PAPER_TYPES.filter(
    (paper) => !paper.optional || optionalSubjectIds.includes(paper.id),
  );
}

export function banksForSelectedOptionals(
  banks: StoredQuestionBank[],
  optionalSubjectIds: string[],
): StoredQuestionBank[] {
  const selectedSections = new Set(
    MAINS_PAPER_TYPES.filter(
      (paper) => paper.optional && optionalSubjectIds.includes(paper.id),
    ).flatMap((paper) => paper.sections),
  );

  return banks.filter((bank) => {
    const section = parseBankMetadata(bank).section;
    return (
      !OPTIONAL_MAINS_SECTIONS.has(section) || selectedSections.has(section)
    );
  });
}

export function searchScope(
  screen: CatalogScreen,
  optionalSubjectIds: string[] = [],
  showOptionalResults = false,
): {
  sections: string[];
  label: string;
} {
  const selectedOptionalSections = mainsPaperTypesForOptionals(
    optionalSubjectIds,
  )
    .filter((paper) => paper.optional)
    .flatMap((paper) => paper.sections);
  const broadSearchSections = showOptionalResults
    ? [...DEFAULT_SEARCH_SECTIONS, ...selectedOptionalSections]
    : DEFAULT_SEARCH_SECTIONS;

  if (screen.kind === "home")
    return { sections: broadSearchSections, label: "All Papers" };
  if (screen.kind === "prelims" || screen.kind === "prelims-history")
    return {
      sections: PRELIMS_GENERAL_STUDIES_SECTIONS,
      label: "Prelims",
    };
  if (screen.kind === "prelims-paper")
    return { sections: [screen.paper.section], label: screen.paper.label };
  if (screen.kind === "mains")
    return {
      sections: broadSearchSections.filter((section) =>
        section.startsWith("mains-"),
      ),
      label: "Mains",
    };
  if (
    screen.kind === "mains-paper" &&
    screen.paper.optional &&
    !optionalSubjectIds.includes(screen.paper.id)
  )
    return { sections: [], label: screen.paper.label };
  return { sections: screen.paper.sections, label: screen.paper.label };
}

export function catalogHeading(screen: CatalogScreen): {
  title: string;
  trail: string | null;
} {
  if (screen.kind === "home")
    return { title: "UPSC CSE", trail: "Question Paper Archive" };
  if (screen.kind === "prelims") return { title: "Prelims", trail: "UPSC CSE" };
  if (screen.kind === "prelims-history")
    return { title: "Test History", trail: "UPSC CSE  ·  Prelims" };
  if (screen.kind === "prelims-paper")
    return { title: screen.paper.label, trail: "UPSC CSE  ·  Prelims" };
  if (screen.kind === "mains") return { title: "Mains", trail: "UPSC CSE" };
  return { title: screen.paper.label, trail: "UPSC CSE  ·  Mains" };
}

function screenKey(screen: CatalogScreen): string {
  if (screen.kind === "prelims-paper")
    return `prelims-paper:${screen.paper.id}`;
  if (screen.kind === "mains-paper") return `mains-paper:${screen.paper.id}`;
  return screen.kind;
}

function screenFromKey(value: string): CatalogScreen | null {
  if (
    value === "home" ||
    value === "prelims" ||
    value === "prelims-history" ||
    value === "mains"
  ) {
    return { kind: value };
  }
  const [kind, paperId] = value.split(":", 2);
  if (kind === "prelims-paper") {
    const paper = PRELIMS_PAPER_TYPES.find((entry) => entry.id === paperId);
    return paper ? { kind, paper } : null;
  }
  if (kind === "mains-paper") {
    const paper = MAINS_PAPER_TYPES.find((entry) => entry.id === paperId);
    return paper ? { kind, paper } : null;
  }
  return null;
}

/** Encode a catalog stack for safe return navigation across route changes. */
export function catalogReturnTo(route: CatalogRouteState): string {
  return `/?catalog=${route.history.concat(route.screen).map(screenKey).join(",")}`;
}

/** Restore only known catalog screens; malformed URLs fall back to Home. */
export function catalogRouteFromSearchParams(
  searchParams: URLSearchParams,
): CatalogRouteState | null {
  const raw = searchParams.get("catalog");
  if (!raw) return null;
  const screens = raw.split(",").map(screenFromKey);
  if (screens.length === 0 || screens.some((screen) => screen === null))
    return null;
  const resolved = screens as CatalogScreen[];
  const screen = resolved.at(-1);
  return screen ? { history: resolved.slice(0, -1), screen } : null;
}

export function isCatalogReturnTo(value: string): boolean {
  if (!value.startsWith("/?")) return false;
  return (
    catalogRouteFromSearchParams(new URLSearchParams(value.slice(2))) !== null
  );
}
