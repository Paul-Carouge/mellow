import { describe, expect, it } from "vitest";

import { QR_QUIET_ZONE_MODULES, encodeQrCode, qrCodeSvgPath } from "./qr-code";

const rows = (text: string) =>
  (encodeQrCode(text) ?? []).map((row) => row.map((isDark) => (isDark ? "1" : "0")).join(""));

describe("encodeQrCode", () => {
  it("draws the symbol a reader decodes back to the text", () => {
    // Read back as "MELLOW" by Chrome's BarcodeDetector (macOS Vision) when this test was written,
    // as were symbols of versions 1 to 40 and an `otpauth://` URI: a change of output is a bug.
    expect(rows("MELLOW")).toEqual([
      "111111100111001111111",
      "100000101000101000001",
      "101110100111001011101",
      "101110100100101011101",
      "101110101111101011101",
      "100000100001001000001",
      "111111101010101111111",
      "000000000100000000000",
      "101010100000100010010",
      "110101000111010100010",
      "000110100101011101111",
      "001000001101110110000",
      "001110100101011100110",
      "000000001000001100110",
      "111111100010100010111",
      "100000100110001010011",
      "101110101000101110111",
      "101110100001010000110",
      "101110101101011110101",
      "100000100001110010010",
      "111111101111011100111",
    ]);
  });

  it("takes the smallest version that holds the bytes at level M", () => {
    expect(rows("a".repeat(14))).toHaveLength(21);
    expect(rows("a".repeat(15))).toHaveLength(25);
    // A typical enrolment URI: version 9.
    const uri =
      "otpauth://totp/Mellow%20(chat.example.org):ana?secret=JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP&issuer=Mellow%20(chat.example.org)&algorithm=SHA1&digits=6&period=30";
    expect(rows(uri)).toHaveLength(53);
    expect(encodeQrCode("a".repeat(2331))).toHaveLength(177);
    expect(encodeQrCode("a".repeat(2332))).toBeNull();
  });

  it("puts the three finder patterns in the corners", () => {
    const symbol = rows("finder patterns");
    const size = symbol.length;
    const finder = ["1111111", "1000001", "1011101", "1011101", "1011101", "1000001", "1111111"];
    for (const [left, top] of [
      [0, 0],
      [size - 7, 0],
      [0, size - 7],
    ] as const) {
      expect(symbol.slice(top, top + 7).map((row) => row.slice(left, left + 7))).toEqual(finder);
    }
  });
});

describe("qrCodeSvgPath", () => {
  it("draws one unit square per dark module, inside the quiet zone", () => {
    expect(qrCodeSvgPath([[true, false], [false, true]])).toBe(
      `M${QR_QUIET_ZONE_MODULES} ${QR_QUIET_ZONE_MODULES}h1v1h-1zM${QR_QUIET_ZONE_MODULES + 1} ${QR_QUIET_ZONE_MODULES + 1}h1v1h-1z`,
    );
  });
});
