// @vitest-environment jsdom

import { render } from "@testing-library/svelte";
import { describe, expect, it } from "vitest";
import MathText from "$lib/components/math-text.svelte";

describe("MathText", () => {
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
});
