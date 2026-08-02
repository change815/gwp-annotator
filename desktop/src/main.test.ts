import { describe, expect, it } from "vitest";

describe("placeholder shell", () => {
  it("renders an offline local status", async () => {
    document.body.innerHTML = '<div id="app"></div>';
    await import("./main");

    expect(document.querySelector('[data-testid="offline-status"]')?.textContent).toBe(
      "Offline by default",
    );
  });
});

