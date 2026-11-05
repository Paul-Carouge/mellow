import { describe, expect, it } from "vitest";
import { inputModalityAfterKeyDown } from "./input-modality";

describe("inputModalityAfterKeyDown", () => {
  it("counts Tab and F6 as keyboard navigation, even from a text field", () => {
    expect(inputModalityAfterKeyDown("Tab", false)).toBe("keyboard");
    expect(inputModalityAfterKeyDown("Tab", true)).toBe("keyboard");
    expect(inputModalityAfterKeyDown("F6", true)).toBe("keyboard");
  });

  it("counts arrows as navigation outside text fields only", () => {
    expect(inputModalityAfterKeyDown("ArrowDown", false)).toBe("keyboard");
    expect(inputModalityAfterKeyDown("ArrowLeft", true)).toBeNull();
  });

  it("leaves the modality alone while typing", () => {
    expect(inputModalityAfterKeyDown("a", true)).toBeNull();
    expect(inputModalityAfterKeyDown("Enter", false)).toBeNull();
    expect(inputModalityAfterKeyDown("Escape", false)).toBeNull();
  });
});
