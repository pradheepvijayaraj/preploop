import katex from "katex";

const MATH_CHUNK =
  /((?<!\\)\$\$[\s\S]+?(?<!\\)\$\$|(?<!\\)\$(?:\\.|[^$])+?(?<!\\)\$)/;
const ROMAN_TOKEN =
  /^(?=[ivxlcdm]+$)m{0,3}(?:cm|cd|d?c{0,3})(?:xc|xl|l?x{0,3})(?:ix|iv|v?i{0,3})$/i;
const ROMAN_MARKER = /\(([ivxlcdm]+)\)/i;

function escapeHtml(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function mapOutsideMath(input: string, fn: (plain: string) => string): string {
  return input
    .split(MATH_CHUNK)
    .map((chunk, i) => (i % 2 === 1 ? chunk : fn(chunk)))
    .join("");
}

/** $7$ $16$ $48.1$ → plain digits so options match body font. */
function unwrapSimpleMath(input: string): string {
  return input.replace(/\$([+-]?\d+(?:\.\d+)?)\$/g, "$1");
}

/**
 * Drop spurious list markers left when SuperKalam used "1. ![fig]"
 * or when a converter left a bare "1." line above a figure.
 * (RegExp ctor — TS misparses /(?=!\[)/ literals.)
 */
function stripFigureListMarkers(input: string): string {
  let s = input;
  s = s.replace(new RegExp(String.raw`^\s*\d+\.\s*(?=!\[)`, "gm"), "");
  s = s.replace(new RegExp(String.raw`^\s*\d+\.\s*\n(?=\s*!\[)`, "gm"), "");
  s = s.replace(new RegExp(String.raw`^\s*\d+\.\s*(?=\u0000IMG)`, "gm"), "");
  s = s.replace(
    new RegExp(String.raw`^\s*\d+\.\s*$\n(?=\s*\u0000IMG)`, "gm"),
    "",
  );
  return s;
}

function isDisplayEnv(tex: string): boolean {
  return /\\begin\{(p?matrix|b?matrix|vmatrix|Vmatrix|cases|aligned|align\*?|array|gather\*?)/.test(
    tex,
  );
}

function isContinuationLabel(line: string): boolean {
  return /^(?:(?:statement|conclusion)\s*(?:[- ]\s*(?:[ivxlcdm]+|\d+))?|question)\s*:\s*$/i.test(
    line,
  );
}

function isStandaloneDisplayMath(line: string): boolean {
  const displayMatch = line.match(/^\$\$([\s\S]+)\$\$$/);
  const inlineMatch = line.match(/^\$([^$]+)\$$/);
  const tex = displayMatch?.[1] ?? inlineMatch?.[1] ?? "";
  return Boolean(tex && isDisplayEnv(tex));
}

/**
 * OCR and PDF extraction often place a labelled statement's content on the
 * next line. Rejoin that continuation so "Statement I: $x < y$" renders as
 * one semantic line. Keep matrix-like display expressions on their own row.
 */
function mergeLabelContinuations(lines: string[]): string[] {
  const merged: string[] = [];

  for (let index = 0; index < lines.length; index++) {
    const line = lines[index]!;
    const next = lines[index + 1];
    if (next && isContinuationLabel(line) && !isStandaloneDisplayMath(next)) {
      merged.push(`${line} ${next}`);
      index += 1;
    } else {
      merged.push(line);
    }
  }

  return merged;
}

function shouldDisplay(
  tex: string,
  fullLine: string,
  matchStart: number,
  matchEnd: number,
  explicitDisplay: boolean,
): boolean {
  if (explicitDisplay) return true;
  if (!isDisplayEnv(tex)) return false;

  const before = fullLine
    .slice(0, matchStart)
    .replace(/^\s*(\([a-h]\)|\([ivxlcdm]+\))\s*/i, "")
    .trim();
  const after = fullLine
    .slice(matchEnd)
    .replace(/^\s*[.,;:…]*\s*$/u, "")
    .trim();

  return !before && !after;
}

function renderMath(tex: string, display: boolean): string {
  try {
    return katex.renderToString(tex, {
      displayMode: display,
      throwOnError: false,
      strict: "ignore",
      trust: false,
      output: "html",
      fleqn: false,
    });
  } catch {
    return escapeHtml(`$${tex}$`);
  }
}

function extractMarkdownImages(input: string): {
  text: string;
  images: string[];
} {
  const images: string[] = [];
  const text = input.replace(
    /!\[([^\]]*)\]\(([^)\s]+)\)/g,
    (_full, alt: string, src: string) => {
      const i = images.length;
      const safeSrc = escapeHtml(String(src || "").trim());
      const safeAlt = escapeHtml(String(alt || "Figure").trim() || "Figure");
      images.push(
        `<img class="math-text__img" src="${safeSrc}" alt="${safeAlt}" loading="lazy" decoding="async" />`,
      );
      return `\u0000IMG${i}\u0000`;
    },
  );
  return { text, images };
}

function restoreImagePlaceholders(html: string, images: string[]): string {
  return html.replace(/\u0000IMG(\d+)\u0000/g, (_m, idx: string) => {
    return images[Number(idx)] ?? "";
  });
}

function renderSegment(input: string): string {
  // Re-run bold in a safer way: extract ** before escape
  if (!input) return "";
  const boldParts: string[] = [];
  const withBoldPlaceholders = input.replace(
    /\*\*([^*]+)\*\*/g,
    (_m, inner: string) => {
      const i = boldParts.length;
      boldParts.push(inner);
      return `\u0000BOLD${i}\u0000`;
    },
  );
  const { text: withoutImgs, images } =
    extractMarkdownImages(withBoldPlaceholders);
  let work = stripFigureListMarkers(withoutImgs);

  const parts: string[] = [];
  const re =
    /(?<!\\)\$\$([\s\S]+?)(?<!\\)\$\$|(?<!\\)\$((?:\\.|[^$])+?)(?<!\\)\$/g;
  const renderPlain = (plain: string) =>
    escapeHtml(plain.replace(/\\\$/g, "$"));
  let last = 0;
  let match: RegExpExecArray | null;

  while ((match = re.exec(work)) !== null) {
    if (match.index > last) {
      parts.push(renderPlain(work.slice(last, match.index)));
    }
    const explicitDisplay = match[1] != null;
    const tex = (explicitDisplay ? match[1] : match[2]) ?? "";
    const end = match.index + match[0].length;
    const display = shouldDisplay(tex, work, match.index, end, explicitDisplay);
    parts.push(renderMath(tex, display));
    last = end;
  }

  if (last < work.length) {
    parts.push(renderPlain(work.slice(last)));
  }

  let html = restoreImagePlaceholders(parts.join(""), images);
  html = html.replace(/\u0000BOLD(\d+)\u0000/g, (_m, idx: string) => {
    return `<strong>${escapeHtml(boldParts[Number(idx)] ?? "")}</strong>`;
  });
  return html;
}

function splitParts(raw: string): string[] {
  let s = (raw ?? "").replace(/\r\n/g, "\n").trim();
  if (!s) return [];

  s = mapOutsideMath(s, (plain) =>
    plain.replace(
      /(\([a-h]\))\s*(\(([ivxlcdm]+)\))/gi,
      (match, letter: string, marker: string, token: string) => {
        if (!isRomanToken(token)) return match;
        return token.toLowerCase() === "i"
          ? `${letter} ${marker}`
          : `\n${marker}`;
      },
    ),
  );
  s = mapOutsideMath(s, (plain) =>
    plain.replace(/(?<!\n)\s+(\([a-h]\))\s+/gi, "\n$1 "),
  );
  s = mapOutsideMath(s, splitInlineRomanMarkers);
  s = mapOutsideMath(s, (plain) =>
    plain.replace(/;\s*(\([IVX]+\))\s*/g, "\n$1 "),
  );
  s = mapOutsideMath(s, (plain) =>
    plain.replace(/;?\s+\b(?:and|or)\s*(?=\n\()/gi, ""),
  );

  const lines = mergeLabelContinuations(
    s
      .split(/\n+/)
      .map((line) => line.trim())
      .filter(Boolean),
  );

  const peeled: string[] = [];
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i]!;
    const next = lines[i + 1] ?? "";
    const nextIsIi = /^\(ii\)/i.test(next);
    const isPartI = /^\([a-h]\)\s+\(i\)\s+/i.test(line);
    const isBareI = /^\(i\)\s+/i.test(line);

    if (nextIsIi && !isPartI && !isBareI && /\(i\)\s+/.test(line)) {
      const m = line.match(/^(.*?)(\s+)(\(i\)\s+[\s\S]+)$/i);
      if (m && (m[1] ?? "").trim().length > 0) {
        peeled.push((m[1] ?? "").trim());
        peeled.push((m[3] ?? "").trim());
        continue;
      }
    }
    peeled.push(line);
  }

  return moveLeadingFigureAfterRomanList(peeled);
}

function normalizeMatchingLists(input: string): string {
  const lines = input.replace(/\r\n/g, "\n").split("\n");
  const firstHeader = lines.findIndex((line) =>
    /^List(?:-|\s+)I\b/i.test(line.trim()),
  );
  if (firstHeader < 0) return input;

  const secondHeader = lines.findIndex(
    (line, index) =>
      index > firstHeader && /^List(?:-|\s+)II\b/i.test(line.trim()),
  );
  if (secondHeader < 0) return input;

  const explicitCodesIndex = lines.findIndex(
    (line, index) => index > secondHeader && /^Codes?\b/i.test(line.trim()),
  );
  const codesIndex = explicitCodesIndex < 0 ? lines.length : explicitCodesIndex;

  const left = lines
    .slice(firstHeader + 1, secondHeader)
    .map((line) => line.trim())
    .filter((line) => /^[A-H]\.\s+/.test(line));
  const right = lines
    .slice(secondHeader + 1, codesIndex)
    .map((line) => line.trim())
    .filter((line) => /^\d+\.\s+/.test(line));
  if (left.length === 0 || left.length !== right.length) return input;

  const table = [
    `| ${lines[firstHeader]!.trim()} | ${lines[secondHeader]!.trim()} |`,
    "| --- | --- |",
    ...left.map((item, index) => `| ${item} | ${right[index]} |`),
  ];
  return [
    ...lines.slice(0, firstHeader),
    ...table,
    ...lines.slice(explicitCodesIndex < 0 ? codesIndex : codesIndex + 1),
  ].join("\n");
}

function normalizeLabelledDetailRows(input: string): string {
  if (input.includes("|")) return input;
  const lines = input.replace(/\r\n/g, "\n").split("\n");
  const rows = lines
    .map((line, index) => ({ line: line.trim(), index }))
    .filter(({ line }) => /^\d+\.\s+[^:]+:\s+.+;\s+[^:]+:\s+/.test(line));
  if (rows.length < 2) return input;

  const parsed = rows.map(({ line }) => {
    const number = line.match(/^(\d+)\./)?.[1] ?? "";
    const fields = line
      .replace(/^\d+\.\s*/, "")
      .split(/;\s+(?=[^:]+:\s*)/)
      .map((field) => {
        const separator = field.indexOf(":");
        return [
          field.slice(0, separator).trim(),
          field.slice(separator + 1).trim(),
        ];
      });
    return { number, fields };
  });
  const labels = parsed[0]!.fields.map(([label]) => label);
  if (
    labels.length < 2 ||
    parsed.some(
      ({ fields }) =>
        fields.length !== labels.length ||
        fields.some(([label], index) => label !== labels[index]),
    )
  ) {
    return input;
  }

  const table = [
    `| Sl. No. | ${labels.join(" | ")} |`,
    `| ${["---", ...labels.map(() => "---")].join(" | ")} |`,
    ...parsed.map(
      ({ number, fields }) =>
        `| ${number}. | ${fields.map(([, value]) => value).join(" | ")} |`,
    ),
  ];
  const rowIndexes = new Set(rows.map(({ index }) => index));
  return lines
    .flatMap((line, index) =>
      index === rows[0]!.index ? table : rowIndexes.has(index) ? [] : [line],
    )
    .join("\n");
}

function normalizeDelimitedPairRows(input: string): string {
  if (input.includes("|") || !/\b(?:pair|pairs|matched)\b/i.test(input)) {
    return input;
  }
  // Some legacy OCR records append the closing question to the final numbered
  // pair. Restore that prose boundary before detecting table rows.
  const separated = input.replace(
    /(^\s*\d+\.\s+[^\n]+?)\s+((?:which|how many|in how many|how much|what number|select)\b[^?\n]*\?)\s*$/gim,
    "$1\n$2",
  );
  const lines = separated.replace(/\r\n/g, "\n").split("\n");
  const rows = lines
    .map((line, index) => ({ line: line.trim(), index }))
    .filter(({ line }) => /^\d+\.\s+.+?(?:\s+[—–:]\s+|:\s*).+/.test(line));
  if (rows.length < 2) return input;

  const parsed = rows.map(({ line }) => {
    const match = line.match(/^(\d+\.\s+.+?)(?:\s+[—–:]\s+|:\s*)(.+)$/);
    return match ? [match[1]!, match[2]!] : [];
  });
  if (parsed.some((row) => row.length !== 2)) return input;

  const firstRowIndex = rows[0]!.index;
  const preceding = lines
    .map((line, index) => ({ line: line.trim(), index }))
    .filter(({ line, index }) => index < firstRowIndex && line.length > 0);
  const delimitedHeader = [...preceding]
    .reverse()
    .find(
      ({ line }) =>
        !/^(?:consider|which|with reference|select)\b/i.test(line) &&
        /^.+?(?:\s+[—–:]\s+|:\s+).+$/.test(line),
    );
  let header: string[] | null = null;
  const headerIndexes = new Set<number>();
  let inlineHeaderPrefix: { index: number; text: string } | null = null;

  if (delimitedHeader) {
    const match = delimitedHeader.line.match(/^(.+?)(?:\s+[—–:]\s+|:\s+)(.+)$/);
    if (match) {
      header = [match[1]!.trim(), match[2]!.trim()];
      headerIndexes.add(delimitedHeader.index);
    }
  } else {
    const candidates = preceding.slice(-2);
    if (
      candidates.length === 2 &&
      candidates.every(({ line }) => line.length <= 60) &&
      !/^(?:consider|which|with reference|select)\b/i.test(candidates[0]!.line)
    ) {
      header = candidates.map(({ line }) => line);
      candidates.forEach(({ index }) => headerIndexes.add(index));
    }
  }

  if (!header) {
    const intro = preceding.at(-1);
    const inline = intro?.line.match(/^(.*?\bpairs?)\s*:?[ \t]+(.+)$/i);
    if (intro && inline) {
      const tail = inline[2]!.trim();
      const descriptiveSecond = tail.match(
        /^(.+?)\s+([A-Z][\w/-]*(?:\s+[a-z][\w/-]*)+.*)$/,
      );
      const finalHeading = tail.match(/^(.+?)\s+([A-Z][\w/-]*)$/);
      const split = descriptiveSecond ?? finalHeading;
      if (split) {
        header = [
          split[1]!.replace(/\s+[—–:-]\s*$/, "").trim(),
          split[2]!.replace(/^\s*[—–:-]\s+/, "").trim(),
        ];
        inlineHeaderPrefix = { index: intro.index, text: inline[1]!.trim() };
      }
    }
  }

  const table = [
    ...(header ? [`| ${header[0]} | ${header[1]} |`, "| --- | --- |"] : []),
    ...parsed.map(([left, right]) => `| ${left} | ${right} |`),
  ];
  const rowIndexes = new Set([
    ...rows.map(({ index }) => index),
    ...headerIndexes,
  ]);
  return lines
    .flatMap((line, index) => {
      if (index === firstRowIndex) return table;
      if (rowIndexes.has(index)) return [];
      if (inlineHeaderPrefix?.index === index) return [inlineHeaderPrefix.text];
      return [line];
    })
    .join("\n");
}

function isRomanToken(token: string): boolean {
  return ROMAN_TOKEN.test(token);
}

function splitInlineRomanMarkers(plain: string): string {
  return plain.replace(
    /\s+(\(([ivxlcdm]+)\))\s+/gi,
    (match, marker: string, token: string) => {
      if (
        !isRomanToken(token) ||
        token.toLowerCase() === "i" ||
        /^[a-h]$/i.test(token)
      ) {
        return match;
      }
      return `\n${marker} `;
    },
  );
}

function romanMarkerToken(line: string): string | null {
  const match = ROMAN_MARKER.exec(line.trimStart());
  if (!match || match.index !== 0) return null;
  const token = match[1] ?? "";
  return isRomanToken(token) ? token.toLowerCase() : null;
}

function isRomanSubLine(line: string): boolean {
  const token = romanMarkerToken(line);
  return token !== null && !/^[a-h]$/i.test(token);
}

function isStandaloneMarkdownImage(line: string): boolean {
  return /^!\[[^\]]*\]\([^\s)]+\)$/.test(line.trim());
}

/** Keep map questions consistent: instructions, complete place list, then map. */
function moveLeadingFigureAfterRomanList(lines: string[]): string[] {
  const normalized = [...lines];

  for (let index = 0; index + 1 < normalized.length; index++) {
    if (
      !isStandaloneMarkdownImage(normalized[index]!) ||
      romanMarkerToken(normalized[index + 1]!) !== "i"
    ) {
      continue;
    }

    let afterList = index + 1;
    while (
      afterList < normalized.length &&
      isRomanSubLine(normalized[afterList]!)
    ) {
      afterList++;
    }

    const [figure] = normalized.splice(index, 1);
    normalized.splice(afterList - 1, 0, figure!);
    index = afterList - 1;
  }

  return normalized;
}

function isTableSepLine(line: string): boolean {
  const t = line.trim();
  if (!t.includes("-")) return false;
  // GFM separator: |---|:---:| or ---|---
  return /^[:\s|.-]+$/.test(t);
}

function isTableRowLine(line: string): boolean {
  const t = line.trim();
  if (!t.includes("|")) return false;
  // At least two cells worth of pipes
  const pipes = (t.match(/\|/g) ?? []).length;
  if (pipes < 1) return false;
  // Avoid treating prose "a | b" mid-sentence as a table alone
  return t.startsWith("|") || t.endsWith("|") || pipes >= 2;
}

function parseCells(line: string): string[] {
  let t = line.trim();
  if (t.startsWith("|")) t = t.slice(1);
  if (t.endsWith("|")) t = t.slice(0, -1);
  return t.split("|").map((c) => c.trim());
}

function renderTable(blockLines: string[]): string {
  // Drop separator rows; first row is header if a sep exists anywhere
  const hasSep = blockLines.some(isTableSepLine);
  const dataLines = blockLines.filter((l) => !isTableSepLine(l));
  if (dataLines.length === 0) return "";

  const rows = dataLines.map(parseCells);
  const colCount = Math.max(...rows.map((r) => r.length), 1);
  const normalized = rows.map((r) => {
    const copy = [...r];
    while (copy.length < colCount) copy.push("");
    return copy.slice(0, colCount);
  });

  const head = hasSep ? normalized[0] : null;
  const body = hasSep ? normalized.slice(1) : normalized;
  const serialClass = /^Sl\.?\s*No\.?$/i.test(head?.[0]?.trim() ?? "")
    ? " math-text__table--serial"
    : "";
  const columnLengths = Array.from({ length: colCount }, (_, column) =>
    Math.max(...normalized.map((row) => row[column]?.length ?? 0), 1),
  );
  // Size every table from its actual content. Long cells are capped because
  // they should wrap rather than force an oversized table.
  const hasVeryLongColumn = columnLengths.some((length) => length > 72);
  const contentWidth =
    columnLengths.reduce((total, length) => total + Math.min(length, 44), 0) +
    colCount * 4;
  // Proportional UI text is substantially narrower than one CSS `ch` per
  // character. Convert the content count to a visual-width estimate, except
  // when a truly long cell should deliberately use the whole pane.
  const estimatedWidth = hasVeryLongColumn
    ? 72
    : Math.max(30, Math.round(contentWidth * 0.78));
  const densityClass =
    estimatedWidth <= 50
      ? " math-text__table--compact"
      : estimatedWidth <= 68
        ? " math-text__table--regular"
        : " math-text__table--wide";
  const adaptiveClass = colCount === 2 ? " math-text__table--adaptive" : "";
  const minimumColumnShare = hasVeryLongColumn ? 28 : 40;
  const maximumColumnShare = hasVeryLongColumn ? 72 : 60;
  const firstColumnStyle =
    colCount === 2
      ? `; --math-table-first-column: ${Math.min(
          maximumColumnShare,
          Math.max(
            minimumColumnShare,
            Math.round(
              (columnLengths[0]! / (columnLengths[0]! + columnLengths[1]!)) *
                100,
            ),
          ),
        )}%`
      : "";
  const tableStyle = ` style="--math-table-width: ${estimatedWidth}ch${firstColumnStyle}"`;

  let html = `<div class="math-text__table-wrap"><table class="math-text__table math-text__table--${colCount}-column${serialClass}${densityClass}${adaptiveClass}"${tableStyle}>`;
  if (head) {
    html += "<thead><tr>";
    for (const cell of head) {
      html += `<th>${renderSegment(cell)}</th>`;
    }
    html += "</tr></thead>";
  }
  html += "<tbody>";
  for (const row of body) {
    html += "<tr>";
    for (const cell of row) {
      html += `<td>${renderSegment(cell)}</td>`;
    }
    html += "</tr>";
  }
  html += "</tbody></table></div>";
  return html;
}

function renderLine(line: string): string {
  const cls = isRomanSubLine(line)
    ? "math-text__line math-text__line--sub"
    : "math-text__line";
  return `<p class="${cls}">${renderSegment(line)}</p>`;
}

export function renderMathText(text: string): string {
  try {
    let raw = unwrapSimpleMath(text ?? "");
    raw = normalizeMatchingLists(raw);
    raw = normalizeLabelledDetailRows(raw);
    raw = normalizeDelimitedPairRows(raw);
    raw = stripFigureListMarkers(raw);
    const lines = splitParts(raw);
    if (lines.length === 0) return "";

    const out: string[] = [];
    let i = 0;
    let guard = 0;
    const maxSteps = Math.max(lines.length * 4, 64);
    while (i < lines.length) {
      guard++;
      if (guard > maxSteps) break;
      const line = lines[i]!;
      // Start of a markdown table: row followed by sep or another row
      if (
        isTableRowLine(line) &&
        i + 1 < lines.length &&
        (isTableSepLine(lines[i + 1]!) || isTableRowLine(lines[i + 1]!))
      ) {
        const block: string[] = [];
        while (
          i < lines.length &&
          (isTableRowLine(lines[i]!) || isTableSepLine(lines[i]!))
        ) {
          guard++;
          if (guard > maxSteps) break;
          block.push(lines[i]!);
          i++;
        }
        out.push(renderTable(block));
        continue;
      }
      // Drop bare numbered stubs ("1.") that are not real content
      if (/^\d+\.\s*$/.test(line)) {
        i++;
        continue;
      }
      out.push(renderLine(line));
      i++;
    }
    return out.join("");
  } catch {
    // Never block the practice session on a render edge-case
    return escapeHtml(text ?? "");
  }
}
