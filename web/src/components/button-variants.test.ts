import { describe, expect, it } from "vitest";

import { type ButtonVariant, buttonVariantClasses } from "./button-variants";

const DISABLED_WASH = "bg-(--mw-component-button-disabled-background)";

describe("buttonVariantClasses", () => {
  it.each<ButtonVariant>(["primary", "secondary", "danger"])("gives a disabled %s button the disabled wash, never a dimmed fill", (variant) => {
    const classes = buttonVariantClasses(variant, true);
    expect(classes).toContain(DISABLED_WASH);
    expect(classes).toContain("text-(--mw-component-button-disabled-text)");
    expect(classes).not.toContain("opacity-");
    expect(classes).not.toContain(`button-${variant}-background`);
  });

  it("dims a disabled ghost button, which has no fill to swap", () => {
    const classes = buttonVariantClasses("ghost", true);
    expect(classes).toContain("opacity-(--mw-component-button-disabled-opacity)");
    expect(classes).not.toContain(DISABLED_WASH);
  });

  it.each<ButtonVariant>(["primary", "secondary", "danger", "ghost"])("keeps an enabled %s button undimmed", (variant) => {
    const classes = buttonVariantClasses(variant, false);
    expect(classes).not.toContain("opacity-");
    expect(classes).not.toContain(DISABLED_WASH);
  });

  it("fills an enabled primary button with its own background", () => {
    expect(buttonVariantClasses("primary", false)).toContain("bg-(--mw-component-button-primary-background)");
  });
});
