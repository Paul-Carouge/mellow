import { describe, expect, it } from "vitest";

import { fieldErrorsBeforeSending } from "./sign-in-form";

describe("fieldErrorsBeforeSending", () => {
  it("names every empty field, in the form's order", () => {
    const errors = fieldErrorsBeforeSending([
      { id: "login", value: "", emptyError: "Enter your username or email." },
      { id: "password", value: "", emptyError: "Enter your password." },
    ]);

    expect([...errors]).toEqual([
      ["login", "Enter your username or email."],
      ["password", "Enter your password."],
    ]);
  });

  it("says nothing about filled fields", () => {
    expect(fieldErrorsBeforeSending([{ id: "password", value: "  ", emptyError: "Enter your password." }]).size).toBe(0);
  });

  it("reports a malformed field only once it is filled", () => {
    const email = { id: "email", emptyError: "Enter your email.", malformedError: "Enter an email address." };

    expect(fieldErrorsBeforeSending([{ ...email, value: "", isMalformed: true }]).get("email")).toBe("Enter your email.");
    expect(fieldErrorsBeforeSending([{ ...email, value: "ana", isMalformed: true }]).get("email")).toBe("Enter an email address.");
    expect(fieldErrorsBeforeSending([{ ...email, value: "ana@example.org", isMalformed: false }]).size).toBe(0);
  });
});
