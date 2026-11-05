import { describe, expect, it } from "vitest";

import { groupedSecret, isRecoveryCode, isSixDigitCode, otpauthUri, recoveryCodesFileText } from "./two-factor";

describe("otpauthUri", () => {
  it("names the instance in the label and the issuer, percent-encoded", () => {
    expect(otpauthUri("JBSWY3DPEHPK3PXP", "ana.b", "chat.example.org:8443")).toBe(
      "otpauth://totp/Mellow%20(chat.example.org%3A8443):ana.b?secret=JBSWY3DPEHPK3PXP&issuer=Mellow%20%28chat.example.org%3A8443%29&algorithm=SHA1&digits=6&period=30",
    );
  });
});

describe("groupedSecret", () => {
  it("splits the key in groups of four", () => {
    expect(groupedSecret("JBSWY3DPEHPK3PXPJB")).toBe("JBSW Y3DP EHPK 3PXP JB");
    expect(groupedSecret("")).toBe("");
  });
});

describe("codes", () => {
  it("tells six digits from a recovery code", () => {
    expect(isSixDigitCode("123 456")).toBe(true);
    expect(isSixDigitCode("12345")).toBe(false);
    expect(isRecoveryCode("abcd-efgh-ijkl-mnop")).toBe(true);
    expect(isRecoveryCode("ABCD EFGH IJKL MNOP")).toBe(true);
    expect(isRecoveryCode("abcd-efgh-ijkl-mno0")).toBe(false);
  });

  it("writes one recovery code per line under a heading", () => {
    expect(recoveryCodesFileText("Codes", ["a", "b"])).toBe("Codes\n\na\nb\n");
  });
});
