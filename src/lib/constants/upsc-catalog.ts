/**
 * Fixed UPSC CSE navigation for the market-demand build.
 * Content is bundled under /static/upsc and seeded on first launch.
 */

/** Mains paper-type picker. Dual-paper optionals group Paper I + Paper II. */
export interface MainsPaperType {
  id: string;
  /** Short tile label */
  label: string;
  /** Short catalog description. */
  description: string;
  /** Section ids used in bank metadata / static paths. */
  sections: string[];
  /** When true, year list splits into Paper I / Paper II (sections[0]/[1]). */
  dualPaper?: boolean;
  /** Theory optional (vs Essay / GS). */
  optional?: boolean;
}

export const MAINS_PAPER_TYPES: MainsPaperType[] = [
  {
    id: "essay",
    label: "Essay",
    description: "Long-Form Argument & Expression",
    sections: ["mains-essay"],
  },
  {
    id: "gs1",
    label: "GS 1",
    description: "Culture, History, Society & Geography",
    sections: ["mains-gs1"],
  },
  {
    id: "gs2",
    label: "GS 2",
    description: "Governance, Polity & International Relations",
    sections: ["mains-gs2"],
  },
  {
    id: "gs3",
    label: "GS 3",
    description: "Economy, Environment, Technology & Security",
    sections: ["mains-gs3"],
  },
  {
    id: "gs4",
    label: "GS 4",
    description: "Ethics, Integrity & Aptitude",
    sections: ["mains-gs4"],
  },
  {
    id: "anthropology",
    label: "Anthropology",
    description: "Optional Paper I & Paper II",
    sections: ["mains-anthropology1", "mains-anthropology2"],
    dualPaper: true,
    optional: true,
  },
  {
    id: "commerce",
    label: "Commerce & Accountancy",
    description: "Optional Paper I & Paper II",
    sections: ["mains-commerce1", "mains-commerce2"],
    dualPaper: true,
    optional: true,
  },
  {
    id: "economics",
    label: "Economics",
    description: "Optional Paper I & Paper II",
    sections: ["mains-economics1", "mains-economics2"],
    dualPaper: true,
    optional: true,
  },
  {
    id: "geography",
    label: "Geography",
    description: "Optional Paper I & Paper II",
    sections: ["mains-geography1", "mains-geography2"],
    dualPaper: true,
    optional: true,
  },
  {
    id: "history",
    label: "History",
    description: "Optional Paper I & Paper II",
    sections: ["mains-history1", "mains-history2"],
    dualPaper: true,
    optional: true,
  },
  {
    id: "law",
    label: "Law",
    description: "Optional Paper I & Paper II",
    sections: ["mains-law1", "mains-law2"],
    dualPaper: true,
    optional: true,
  },
  {
    id: "math",
    label: "Mathematics",
    description: "Optional Paper I & Paper II",
    sections: ["mains-maths1", "mains-maths2"],
    dualPaper: true,
    optional: true,
  },
  {
    id: "medical",
    label: "Medical Science",
    description: "Optional Paper I & Paper II",
    sections: ["mains-medical-science1", "mains-medical-science2"],
    dualPaper: true,
    optional: true,
  },
  {
    id: "philosophy",
    label: "Philosophy",
    description: "Optional Paper I & Paper II",
    sections: ["mains-philosophy1", "mains-philosophy2"],
    dualPaper: true,
    optional: true,
  },
  {
    id: "psir",
    label: "Political Science & International Relations",
    description: "Optional Paper I & Paper II",
    sections: ["mains-psir1", "mains-psir2"],
    dualPaper: true,
    optional: true,
  },
  {
    id: "pubad",
    label: "Public Administration",
    description: "Optional Paper I & Paper II",
    sections: ["mains-pubad1", "mains-pubad2"],
    dualPaper: true,
    optional: true,
  },
  {
    id: "sociology",
    label: "Sociology",
    description: "Optional Paper I & Paper II",
    sections: ["mains-sociology1", "mains-sociology2"],
    dualPaper: true,
    optional: true,
  },
];

export interface PrelimsPaperType {
  id: string;
  label: string;
  description: string;
  section: string;
}

export const PRELIMS_PAPER_TYPES: PrelimsPaperType[] = [
  {
    id: "gs1",
    label: "GS 1",
    description: "General Studies",
    section: "prelims-gs1",
  },
  {
    id: "csat",
    label: "CSAT",
    description: "Aptitude & Reasoning",
    section: "prelims-csat",
  },
];

export const ACTIVE_UPSC_SECTIONS = new Set([
  ...PRELIMS_PAPER_TYPES.map((paper) => paper.section),
  ...MAINS_PAPER_TYPES.flatMap((paper) => paper.sections),
]);
