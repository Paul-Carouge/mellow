// A QR code encoder (ISO/IEC 18004), byte mode at error correction level M, for the two-factor
// enrolment's `otpauth://` URI (ADR-0049). Drawn here so the secret never leaves the browser;
// small enough not to need a dependency.

/** Error correction codewords per block at level M, by version (index 0 unused). */
const ECC_CODEWORDS_PER_BLOCK = [
  -1, 10, 16, 26, 18, 24, 16, 18, 22, 22, 26, 30, 22, 22, 24, 24, 28, 28, 26, 26, 26, 26, 28, 28, 28, 28, 28, 28, 28,
  28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28,
] as const;
/** Error correction blocks at level M, by version (index 0 unused). */
const ERROR_CORRECTION_BLOCKS = [
  -1, 1, 1, 1, 2, 2, 4, 4, 4, 5, 5, 5, 8, 9, 9, 10, 10, 11, 13, 14, 16, 17, 17, 18, 20, 21, 23, 25, 26, 28, 29, 31, 33,
  35, 37, 38, 40, 43, 45, 47, 49,
] as const;
const MINIMUM_VERSION = 1;
const MAXIMUM_VERSION = 40;
/** Level M in the format information. */
const LEVEL_M_FORMAT_BITS = 0;
const BYTE_MODE_INDICATOR = 0b0100;
const PAD_CODEWORDS = [0xec, 0x11] as const;
/** The light border scanners need around the code, in modules. */
export const QR_QUIET_ZONE_MODULES = 4;

/** Dark modules, row by row. */
export type QrMatrix = readonly (readonly boolean[])[];

function rawDataModules(version: number): number {
  let modules = (16 * version + 128) * version + 64;
  if (version >= 2) {
    const alignments = Math.floor(version / 7) + 2;
    modules -= (25 * alignments - 10) * alignments - 55;
    if (version >= 7) {
      modules -= 36;
    }
  }
  return modules;
}

function eccPerBlock(version: number): number {
  return ECC_CODEWORDS_PER_BLOCK[version] ?? 0;
}

function blockCount(version: number): number {
  return ERROR_CORRECTION_BLOCKS[version] ?? 1;
}

function dataCodewords(version: number): number {
  return Math.floor(rawDataModules(version) / 8) - eccPerBlock(version) * blockCount(version);
}

function countBits(version: number): number {
  return version <= 9 ? 8 : 16;
}

/** Product in GF(2^8) with the QR polynomial x^8 + x^4 + x^3 + x^2 + 1. */
function multiply(x: number, y: number): number {
  let product = 0;
  for (let bit = 7; bit >= 0; bit--) {
    product = (product << 1) ^ ((product >>> 7) * 0x11d);
    product ^= ((y >>> bit) & 1) * x;
  }
  return product;
}

function reedSolomonDivisor(degree: number): number[] {
  const divisor = new Array<number>(degree).fill(0);
  divisor[degree - 1] = 1;
  let root = 1;
  for (let i = 0; i < degree; i++) {
    for (let j = 0; j < degree; j++) {
      divisor[j] = multiply(divisor[j] ?? 0, root);
      if (j + 1 < degree) {
        divisor[j] = (divisor[j] ?? 0) ^ (divisor[j + 1] ?? 0);
      }
    }
    root = multiply(root, 0x02);
  }
  return divisor;
}

function reedSolomonRemainder(data: readonly number[], divisor: readonly number[]): number[] {
  const remainder = new Array<number>(divisor.length).fill(0);
  for (const byte of data) {
    const factor = byte ^ (remainder.shift() ?? 0);
    remainder.push(0);
    divisor.forEach((coefficient, index) => {
      remainder[index] = (remainder[index] ?? 0) ^ multiply(coefficient, factor);
    });
  }
  return remainder;
}

/** Splits the data into blocks, adds each block's error correction, and interleaves them. */
function withErrorCorrection(data: readonly number[], version: number): number[] {
  const blocks = blockCount(version);
  const eccLength = eccPerBlock(version);
  const rawCodewords = Math.floor(rawDataModules(version) / 8);
  const shortBlocks = blocks - (rawCodewords % blocks);
  const shortBlockLength = Math.floor(rawCodewords / blocks);
  const divisor = reedSolomonDivisor(eccLength);
  const allBlocks: number[][] = [];
  let offset = 0;
  for (let i = 0; i < blocks; i++) {
    const length = shortBlockLength - eccLength + (i < shortBlocks ? 0 : 1);
    const blockData = data.slice(offset, offset + length);
    offset += length;
    const ecc = reedSolomonRemainder(blockData, divisor);
    // Short blocks get a placeholder where long ones have their extra data codeword.
    allBlocks.push([...blockData, ...(i < shortBlocks ? [0] : []), ...ecc]);
  }
  const interleaved: number[] = [];
  for (let i = 0; i < shortBlockLength + 1; i++) {
    allBlocks.forEach((block, j) => {
      if (i !== shortBlockLength - eccLength || j >= shortBlocks) {
        interleaved.push(block[i] ?? 0);
      }
    });
  }
  return interleaved;
}

/** Mode, count, bytes, terminator and padding: the data codewords of `version`. */
function dataCodewordsFor(bytes: Uint8Array, version: number): number[] {
  const bits: number[] = [];
  const append = (value: number, length: number) => {
    for (let bit = length - 1; bit >= 0; bit--) {
      bits.push((value >>> bit) & 1);
    }
  };
  append(BYTE_MODE_INDICATOR, 4);
  append(bytes.length, countBits(version));
  bytes.forEach((byte) => append(byte, 8));
  const capacityBits = dataCodewords(version) * 8;
  append(0, Math.min(4, capacityBits - bits.length));
  append(0, (8 - (bits.length % 8)) % 8);
  const codewords: number[] = [];
  for (let i = 0; i < bits.length; i += 8) {
    codewords.push(bits.slice(i, i + 8).reduce((byte, bit) => (byte << 1) | bit, 0));
  }
  for (let pad = 0; codewords.length < dataCodewords(version); pad++) {
    codewords.push(PAD_CODEWORDS[pad % 2] ?? 0);
  }
  return codewords;
}

function smallestVersionFor(byteCount: number): number | null {
  for (let version = MINIMUM_VERSION; version <= MAXIMUM_VERSION; version++) {
    if (4 + countBits(version) + byteCount * 8 <= dataCodewords(version) * 8) {
      return version;
    }
  }
  return null;
}

function alignmentPositions(version: number, size: number): number[] {
  if (version === 1) {
    return [];
  }
  const count = Math.floor(version / 7) + 2;
  const step = version === 32 ? 26 : Math.ceil((version * 4 + 4) / (count * 2 - 2)) * 2;
  const positions = [6];
  for (let position = size - 7; positions.length < count; position -= step) {
    positions.splice(1, 0, position);
  }
  return positions;
}

const MASKS: readonly ((x: number, y: number) => boolean)[] = [
  (x, y) => (x + y) % 2 === 0,
  (_x, y) => y % 2 === 0,
  (x) => x % 3 === 0,
  (x, y) => (x + y) % 3 === 0,
  (x, y) => (Math.floor(x / 3) + Math.floor(y / 2)) % 2 === 0,
  (x, y) => ((x * y) % 2) + ((x * y) % 3) === 0,
  (x, y) => (((x * y) % 2) + ((x * y) % 3)) % 2 === 0,
  (x, y) => (((x + y) % 2) + ((x * y) % 3)) % 2 === 0,
];

class QrSymbol {
  readonly modules: boolean[][];
  private readonly isFunction: boolean[][];

  readonly size: number;

  constructor(size: number) {
    this.size = size;
    this.modules = Array.from({ length: size }, () => new Array<boolean>(size).fill(false));
    this.isFunction = Array.from({ length: size }, () => new Array<boolean>(size).fill(false));
  }

  setFunction(x: number, y: number, isDark: boolean): void {
    const row = this.modules[y];
    const functionRow = this.isFunction[y];
    if (row && functionRow) {
      row[x] = isDark;
      functionRow[x] = true;
    }
  }

  isFunctionModule(x: number, y: number): boolean {
    return this.isFunction[y]?.[x] ?? true;
  }

  flip(x: number, y: number): void {
    const row = this.modules[y];
    if (row) {
      row[x] = !row[x];
    }
  }
}

function drawFunctionPatterns(symbol: QrSymbol, version: number): void {
  const { size } = symbol;
  for (let i = 0; i < size; i++) {
    symbol.setFunction(6, i, i % 2 === 0);
    symbol.setFunction(i, 6, i % 2 === 0);
  }
  for (const [centreX, centreY] of [
    [3, 3],
    [size - 4, 3],
    [3, size - 4],
  ] as const) {
    for (let dy = -4; dy <= 4; dy++) {
      for (let dx = -4; dx <= 4; dx++) {
        const distance = Math.max(Math.abs(dx), Math.abs(dy));
        const x = centreX + dx;
        const y = centreY + dy;
        if (x >= 0 && x < size && y >= 0 && y < size) {
          symbol.setFunction(x, y, distance !== 2 && distance !== 4);
        }
      }
    }
  }
  const positions = alignmentPositions(version, size);
  const last = positions.length - 1;
  positions.forEach((x, i) => {
    positions.forEach((y, j) => {
      // The three corners hold finder patterns.
      if ((i === 0 && j === 0) || (i === 0 && j === last) || (i === last && j === 0)) {
        return;
      }
      for (let dy = -2; dy <= 2; dy++) {
        for (let dx = -2; dx <= 2; dx++) {
          symbol.setFunction(x + dx, y + dy, Math.max(Math.abs(dx), Math.abs(dy)) !== 1);
        }
      }
    });
  });
  drawFormatBits(symbol, 0);
  if (version >= 7) {
    let remainder = version;
    for (let i = 0; i < 12; i++) {
      remainder = (remainder << 1) ^ ((remainder >>> 11) * 0x1f25);
    }
    const bits = (version << 12) | remainder;
    for (let i = 0; i < 18; i++) {
      const isDark = ((bits >>> i) & 1) !== 0;
      const a = size - 11 + (i % 3);
      const b = Math.floor(i / 3);
      symbol.setFunction(a, b, isDark);
      symbol.setFunction(b, a, isDark);
    }
  }
}

/** The 15 format bits (level and mask) in their two copies, plus the dark module. */
function drawFormatBits(symbol: QrSymbol, mask: number): void {
  const data = (LEVEL_M_FORMAT_BITS << 3) | mask;
  let remainder = data;
  for (let i = 0; i < 10; i++) {
    remainder = (remainder << 1) ^ ((remainder >>> 9) * 0x537);
  }
  const bits = ((data << 10) | remainder) ^ 0x5412;
  const bit = (index: number) => ((bits >>> index) & 1) !== 0;
  const { size } = symbol;
  for (let i = 0; i <= 5; i++) {
    symbol.setFunction(8, i, bit(i));
  }
  symbol.setFunction(8, 7, bit(6));
  symbol.setFunction(8, 8, bit(7));
  symbol.setFunction(7, 8, bit(8));
  for (let i = 9; i < 15; i++) {
    symbol.setFunction(14 - i, 8, bit(i));
  }
  for (let i = 0; i < 8; i++) {
    symbol.setFunction(size - 1 - i, 8, bit(i));
  }
  for (let i = 8; i < 15; i++) {
    symbol.setFunction(8, size - 15 + i, bit(i));
  }
  symbol.setFunction(8, size - 8, true);
}

/** Codewords in the zigzag order of the standard, two columns at a time from the right. */
function drawCodewords(symbol: QrSymbol, codewords: readonly number[]): void {
  const { size } = symbol;
  let index = 0;
  for (let right = size - 1; right >= 1; right -= 2) {
    if (right === 6) {
      right = 5;
    }
    for (let vertical = 0; vertical < size; vertical++) {
      for (let j = 0; j < 2; j++) {
        const x = right - j;
        const isUpward = ((right + 1) & 2) === 0;
        const y = isUpward ? size - 1 - vertical : vertical;
        if (!symbol.isFunctionModule(x, y) && index < codewords.length * 8) {
          if ((((codewords[index >>> 3] ?? 0) >>> (7 - (index & 7))) & 1) !== 0) {
            symbol.flip(x, y);
          }
          index++;
        }
      }
    }
  }
}

function applyMask(symbol: QrSymbol, mask: number): void {
  const isInverted = MASKS[mask];
  for (let y = 0; y < symbol.size; y++) {
    for (let x = 0; x < symbol.size; x++) {
      if (!symbol.isFunctionModule(x, y) && isInverted?.(x, y)) {
        symbol.flip(x, y);
      }
    }
  }
}

/**
 * Penalty rules 1 (runs of five or more), 2 (2×2 blocks) and 4 (dark/light balance) of the
 * standard. shortcut: rule 3 (finder-like patterns) is left out; every mask is valid and any one
 * scans, the rule only nudges the choice. Add it if a phone ever struggles.
 */
function penalty(modules: QrMatrix): number {
  const size = modules.length;
  let score = 0;
  const runPenalty = (line: (i: number) => boolean) => {
    let runLength = 1;
    for (let i = 1; i <= size; i++) {
      if (i < size && line(i) === line(i - 1)) {
        runLength++;
      } else {
        if (runLength >= 5) {
          score += runLength - 2;
        }
        runLength = 1;
      }
    }
  };
  let dark = 0;
  for (let y = 0; y < size; y++) {
    runPenalty((x) => modules[y]?.[x] ?? false);
    runPenalty((x) => modules[x]?.[y] ?? false);
    for (let x = 0; x < size; x++) {
      const isDark = modules[y]?.[x] ?? false;
      if (isDark) {
        dark++;
      }
      if (
        x < size - 1 &&
        y < size - 1 &&
        isDark === modules[y]?.[x + 1] &&
        isDark === modules[y + 1]?.[x] &&
        isDark === modules[y + 1]?.[x + 1]
      ) {
        score += 3;
      }
    }
  }
  const total = size * size;
  score += (Math.ceil(Math.abs(dark * 20 - total * 10) / total) - 1) * 10;
  return score;
}

/** The smallest level-M symbol holding `text` as UTF-8, or `null` past version 40 (2 331 bytes). */
export function encodeQrCode(text: string): QrMatrix | null {
  const bytes = new TextEncoder().encode(text);
  const version = smallestVersionFor(bytes.length);
  if (version === null) {
    return null;
  }
  const symbol = new QrSymbol(version * 4 + 17);
  drawFunctionPatterns(symbol, version);
  drawCodewords(symbol, withErrorCorrection(dataCodewordsFor(bytes, version), version));
  let bestMask = 0;
  let bestPenalty = Number.POSITIVE_INFINITY;
  for (let mask = 0; mask < MASKS.length; mask++) {
    applyMask(symbol, mask);
    drawFormatBits(symbol, mask);
    const score = penalty(symbol.modules);
    if (score < bestPenalty) {
      bestMask = mask;
      bestPenalty = score;
    }
    // Masking twice restores the modules.
    applyMask(symbol, mask);
  }
  applyMask(symbol, bestMask);
  drawFormatBits(symbol, bestMask);
  return symbol.modules;
}

/** An SVG path drawing every dark module as a unit square, offset by the quiet zone. */
export function qrCodeSvgPath(matrix: QrMatrix): string {
  const parts: string[] = [];
  matrix.forEach((row, y) => {
    row.forEach((isDark, x) => {
      if (isDark) {
        parts.push(`M${x + QR_QUIET_ZONE_MODULES} ${y + QR_QUIET_ZONE_MODULES}h1v1h-1z`);
      }
    });
  });
  return parts.join("");
}
