import { render, waitFor } from "@testing-library/svelte";
import { describe, expect, it } from "vitest";
import SearchText from "$lib/components/search-text.svelte";

describe("SearchText", () => {
  it("highlights complete words and prefixes in tables without changing maths or figures", async () => {
    const text =
      "| Metal | Notice |\n| --- | --- |\n| Silver | Silverfish |\n$x^2 + y^2$\n![Silver](https://example.test/silver.png)\nArticle 20 and Article 201";
    const { container } = render(SearchText, {
      text,
      terms: [
        { text: "silver", prefix: false },
        { text: "not", prefix: true },
        { text: "20", prefix: false },
      ],
    });
    await waitFor(() =>
      expect(container.querySelectorAll("mark").length).toBe(3),
    );
    expect(
      Array.from(
        container.querySelectorAll("mark"),
        (mark) => mark.textContent,
      ),
    ).toEqual(["Notice", "Silver", "20"]);
    expect(container.querySelector("table")).toBeTruthy();
    expect(container.querySelector(".katex")).toBeTruthy();
    expect(container.querySelector(".katex mark")).toBeNull();
    expect(container.querySelector("img")?.getAttribute("alt")).toBe("Silver");
  });

  it("updates marks with new text and never interprets a query as HTML", async () => {
    const view = render(SearchText, {
      text: "Silver Notice <script>alert(1)</script>",
      terms: [{ text: "silver", prefix: true }],
    });
    await waitFor(() =>
      expect(view.container.querySelector("mark")?.textContent).toBe("Silver"),
    );
    await view.rerender({
      text: "Copper mining",
      terms: [{ text: "copp", prefix: true }],
    });
    await waitFor(() =>
      expect(view.container.querySelector("mark")?.textContent).toBe("Copper"),
    );
    expect(view.container.querySelectorAll("mark").length).toBe(1);
    await view.rerender({
      text: "<img src=x onerror=alert(1)> Silver",
      terms: [{ text: "<img src=x onerror=alert(1)>", prefix: true }],
    });
    await waitFor(() =>
      expect(view.container.querySelector("mark")).toBeNull(),
    );
    expect(view.container.querySelector("script, img, [onerror]")).toBeNull();
  });
});
