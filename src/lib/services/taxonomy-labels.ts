import { logError } from "$lib/services/logger";

const TAXONOMY_URL = "/upsc/taxonomy.json";

export interface TaxonomyLabels {
  mainTags: ReadonlyMap<number, string>;
  subtags: ReadonlyMap<number, string>;
}

let labelsPromise: Promise<TaxonomyLabels> | null = null;

function labelMap(value: unknown, field: string): ReadonlyMap<number, string> {
  if (!Array.isArray(value)) {
    throw new Error(`Taxonomy ${field} must be an array`);
  }

  const labels = new Map<number, string>();
  for (const entry of value) {
    if (
      typeof entry !== "object" ||
      entry === null ||
      !("id" in entry) ||
      !("label" in entry) ||
      !Number.isInteger(entry.id) ||
      typeof entry.label !== "string"
    ) {
      throw new Error(`Taxonomy ${field} contains an invalid label entry`);
    }
    labels.set(entry.id as number, entry.label);
  }
  return labels;
}

async function fetchTaxonomyLabels(): Promise<TaxonomyLabels> {
  const response = await fetch(TAXONOMY_URL);
  if (!response.ok) {
    throw new Error(`HTTP ${response.status} for ${TAXONOMY_URL}`);
  }

  const taxonomy = (await response.json()) as Record<string, unknown>;
  return {
    mainTags: labelMap(taxonomy.mainTags, "mainTags"),
    subtags: labelMap(taxonomy.subtags, "subtags"),
  };
}

/** Load and parse the public taxonomy once for every component consumer. */
export function loadTaxonomyLabels(): Promise<TaxonomyLabels> {
  labelsPromise ??= fetchTaxonomyLabels().catch((error) => {
    void logError("Failed to load taxonomy labels", error);
    return { mainTags: new Map(), subtags: new Map() };
  });
  return labelsPromise;
}
