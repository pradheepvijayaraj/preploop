// @vitest-environment jsdom

import { render } from "@testing-library/svelte";
import { describe, expect, it } from "vitest";
import MathText from "$lib/components/math-text.svelte";

describe("MathText", () => {
  it("renders escaped dollar operators as text instead of math delimiters", () => {
    const source = String.raw`If \$ means divided by, then 10#5@1\$5 is`;
    const { container } = render(MathText, { props: { text: source } });

    expect(container.textContent).toBe(
      "If $ means divided by, then 10#5@1$5 is",
    );
    expect(container.querySelector(".katex-error")).toBeNull();
  });

  it("renders literal blank underscores in a series without a KaTeX error", () => {
    const source = String.raw`In the series $\_b\_a\_ba\_b\_abab\_aab$`;
    const { container } = render(MathText, { props: { text: source } });

    expect(container.textContent).toContain("_b_a_ba_b_abab_aab");
    expect(container.querySelector(".katex-error")).toBeNull();
  });

  it("renders mixed prose and matrix LaTeX instead of falling back to raw text", () => {
    const source = String.raw`(a) Find the inverse of the matrix:
$$ A = \begin{bmatrix} 1 & 3 & 1 \\ 2 & -1 & 7 \\ 3 & 2 & -1 \end{bmatrix} $$
by using elementary row operations. Hence solve the system of linear equations
$$x + 3y + z = 10$$
$$2x - y + 7z = 21$$
$$3x + 2y - z = 4$$
(b) Let $A$ be a square matrix and $A^*$ be its adjoint, show that the eigenvalues of matrices $AA^*$ and $A^*A$ are real. Further show that $\text{trace}(AA^*) = \text{trace}(A^*A)$.
(c) Evaluate $\int_0^1 \left(2x \sin \frac{1}{x} - \cos \frac{1}{x}\right) dx$.
(d) Find the equation of the plane which passes through the points $(0, 1, 1)$ and $(2, 0, -1)$, and is parallel to the line joining the points $(-1, 1, -2)$, $(3, -2, 4)$.
(e) A sphere $S$ has points $(0, 1, 0)$, $(3, -5, 2)$ at opposite ends of a diameter.`;
    const { container } = render(MathText, { props: { text: source } });

    expect(container.querySelectorAll(".katex").length).toBeGreaterThan(10);
    expect(container.querySelector(".mtable")).not.toBeNull();
    expect(container.textContent).not.toContain("$A=\\begin{pmatrix}");
  });

  it("renders several inline expressions and an integral in one question", () => {
    const source = String.raw`Let $A^{*}$ be the adjoint. Show $AA^{*}$ is real. Evaluate $\displaystyle\int_{0}^{1}\left(2x\sin\frac{1}{x}-\cos\frac{1}{x}\right)\,dx$.`;
    const { container } = render(MathText, { props: { text: source } });

    expect(container.querySelectorAll(".katex")).toHaveLength(3);
    expect(container.textContent).not.toContain("\\displaystyle");
    expect(container.textContent).not.toContain("$AA^{*}$");
  });

  it("keeps ordinary prose intact while removing a bare figure-list marker", () => {
    const source = "Question text\n1. ![Diagram](/upsc/assets/figure.png)";
    const { container } = render(MathText, { props: { text: source } });
    const image = container.querySelector("img");

    expect(container.textContent).toContain("Question text");
    expect(container.textContent).not.toContain("1.");
    expect(image?.getAttribute("src")).toBe("/upsc/assets/figure.png");
  });

  it("recognizes Roman list entries without a hard-coded list length", () => {
    const source =
      "(a) Mark these locations: (i) One (ii) Two (iii) Three (iv) Four (v) Five (vi) Six (vii) Seven (viii) Eight (ix) Nine (x) Ten (xi) Eleven (xx) Twenty (xl) Forty.";
    const { container } = render(MathText, { props: { text: source } });
    const lines = Array.from(
      container.querySelectorAll<HTMLElement>(".math-text__line"),
    );
    const romanLines = Array.from(
      container.querySelectorAll<HTMLElement>(".math-text__line--sub"),
    );

    expect(lines.map((line) => line.textContent)).toEqual([
      "(a) Mark these locations:",
      "(i) One",
      "(ii) Two",
      "(iii) Three",
      "(iv) Four",
      "(v) Five",
      "(vi) Six",
      "(vii) Seven",
      "(viii) Eight",
      "(ix) Nine",
      "(x) Ten",
      "(xi) Eleven",
      "(xx) Twenty",
      "(xl) Forty.",
    ]);
    expect(romanLines).toHaveLength(13);
  });

  it("places a leading map after its complete Roman location list", () => {
    const source = `Identify the places below.
![Map](/upsc/assets/history/map.jpg)
(i) First place
(x) Tenth place
(xi) Eleventh place
(xx) Twentieth place`;
    const { container } = render(MathText, { props: { text: source } });
    const lines = Array.from(
      container.querySelectorAll<HTMLElement>(".math-text__line"),
    );

    expect(lines.map((line) => line.textContent)).toEqual([
      "Identify the places below.",
      "(i) First place",
      "(x) Tenth place",
      "(xi) Eleventh place",
      "(xx) Twentieth place",
      "",
    ]);
    expect(lines.at(-1)?.querySelector("img")?.getAttribute("src")).toBe(
      "/upsc/assets/history/map.jpg",
    );
    expect(
      lines
        .slice(1, -1)
        .every((line) => line.classList.contains("math-text__line--sub")),
    ).toBe(true);
  });

  it("keeps labelled statement equations on the same rendered line", () => {
    const source = String.raw`For two distinct real numbers $x$ and $y$, which is bigger?
Statement I :
$x^2 < y < 1$
Statement II :
$y < \sqrt{x} < 1$`;
    const { container } = render(MathText, { props: { text: source } });
    const lines = Array.from(
      container.querySelectorAll<HTMLElement>(".math-text__line"),
    );
    const statementOne = lines.find((line) =>
      line.textContent?.startsWith("Statement I :"),
    );
    const statementTwo = lines.find((line) =>
      line.textContent?.startsWith("Statement II :"),
    );

    expect(lines).toHaveLength(3);
    expect(statementOne?.querySelector(".katex")).not.toBeNull();
    expect(statementTwo?.querySelector(".katex")).not.toBeNull();
  });

  it("does not merge plural list headings with their first item", () => {
    const source = "Statements:\n1. Some men are great.\n2. Some men are wise.";
    const { container } = render(MathText, { props: { text: source } });
    const lines = Array.from(
      container.querySelectorAll<HTMLElement>(".math-text__line"),
    );

    expect(lines).toHaveLength(3);
    expect(lines[0]?.textContent).toBe("Statements:");
    expect(lines[1]?.textContent).toBe("1. Some men are great.");
  });

  it("keeps short two-column tables compact without changing long tables", () => {
    const shortTable = `| Place | Hills |
| --- | --- |
| 1. Nokrek Bio-sphere Reserve | Garo Hills |
| 2. Loktak Lake | Barail Range |`;
    const longTable = `| Organisation | Description |
| --- | --- |
| Directorate General of Systems and Data Management | Carrying out big data analytics to assist tax officers for better policy and nabbing tax evaders |`;

    const compact = render(MathText, { props: { text: shortTable } });
    const wrapped = render(MathText, { props: { text: longTable } });

    expect(
      compact.container.querySelector(".math-text__table--compact"),
    ).not.toBeNull();
    expect(
      wrapped.container.querySelector(".math-text__table--compact"),
    ).toBeNull();
    const adaptive = wrapped.container.querySelector<HTMLTableElement>(
      ".math-text__table--adaptive",
    );
    expect(adaptive).not.toBeNull();
    expect(adaptive?.style.getPropertyValue("--math-table-first-column")).toBe(
      "34%",
    );
    expect(
      compact.container
        .querySelector<HTMLTableElement>("table")
        ?.style.getPropertyValue("--math-table-width"),
    ).toBe("37ch");
    expect(
      wrapped.container.querySelector(".math-text__table--wide"),
    ).not.toBeNull();
  });

  it("extracts inline pair headings from legacy imported questions", () => {
    const source = `Consider the following pairs National Park River flowing through the Park
1. Corbett National Park: Ganga
2. Kaziranga National Park: Manas
3. Silent Valley National Park: Kaveri`;
    const { container } = render(MathText, { props: { text: source } });
    const headings = Array.from(container.querySelectorAll("th"), (heading) =>
      heading.textContent?.trim(),
    );

    expect(headings).toEqual([
      "National Park",
      "River flowing through the Park",
    ]);
    expect(container.querySelector(".math-text__line")?.textContent).toBe(
      "Consider the following pairs",
    );

    const shortHeadings = render(MathText, {
      props: {
        text: `Consider the following pairs: Tribe State
1. Limboo (Limbu): Sikkim
2. Karbi: Himachal Pradesh`,
      },
    });
    expect(
      Array.from(shortHeadings.container.querySelectorAll("th"), (heading) =>
        heading.textContent?.trim(),
      ),
    ).toEqual(["Tribe", "State"]);
  });

  it("keeps a closing question appended to the final pair outside the table", () => {
    const source = `With reference to Buddhist history, consider the following pairs: Famous shrine Location
1. Tabo monastery and temple complex: Spiti Valley
2. Lhotsava Lhakhang temple, Nako: Zanskar Valley
3. Alchi temple complex: Ladakh Which of the pairs given above is/are correctly matched?`;
    const { container } = render(MathText, { props: { text: source } });
    const rows = container.querySelectorAll("tbody tr");
    const closingQuestion = Array.from(
      container.querySelectorAll<HTMLElement>(".math-text__line"),
    ).at(-1);

    expect(rows).toHaveLength(3);
    expect(rows[2]?.querySelectorAll("td")[1]?.textContent).toBe("Ladakh");
    expect(closingQuestion?.textContent).toBe(
      "Which of the pairs given above is/are correctly matched?",
    );
  });

  it("removes legacy separator punctuation from inferred pair headings", () => {
    const source = `Consider the following pairs: Traditions - Communities
1. Chaliha Sahib Festival — Sindhis
2. Nanda Raj Jaat Yatra — Gonds
3. Wari-Warkari — Santhals`;
    const { container } = render(MathText, { props: { text: source } });

    expect(
      Array.from(container.querySelectorAll("th"), (heading) =>
        heading.textContent?.trim(),
      ),
    ).toEqual(["Traditions", "Communities"]);
  });

  it("keeps ordinary two-column headers balanced while allowing long content to dominate", () => {
    const ordinary = `| Commonly used/consumed materials | Unwanted or controversial chemicals likely to be found in them |
| --- | --- |
| 1. Lipstick | Lead |
| 2. Soft drinks | Brominated vegetable oils |`;
    const long = `| Famous work of sculpture | Site |
| --- | --- |
| 1. A grand image of Buddha's Mahaparinirvana with numerous celestial musicians above and the sorrowful figures of his followers below | Ajanta |`;
    const ordinaryTable = render(MathText, { props: { text: ordinary } });
    const longTable = render(MathText, { props: { text: long } });

    expect(
      ordinaryTable.container
        .querySelector<HTMLTableElement>("table")
        ?.style.getPropertyValue("--math-table-first-column"),
    ).toBe("40%");
    expect(
      longTable.container
        .querySelector<HTMLTableElement>("table")
        ?.style.getPropertyValue("--math-table-first-column"),
    ).toBe("72%");
  });
});
