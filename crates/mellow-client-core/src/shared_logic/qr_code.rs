//! A QR code encoder (ISO/IEC 18004), byte mode at error correction level M, for the two-factor
//! enrolment's `otpauth://` URI (ADR-0049): `web/src/lib/qr-code.ts`, module for module, so both
//! clients draw the same symbol and the secret never leaves the device.

/// Error correction codewords per block at level M, by version (index 0 unused).
const ECC_CODEWORDS_PER_BLOCK: [usize; 41] = [
    0, 10, 16, 26, 18, 24, 16, 18, 22, 22, 26, 30, 22, 22, 24, 24, 28, 28, 26, 26, 26, 26, 28, 28,
    28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28,
];
/// Error correction blocks at level M, by version (index 0 unused).
const ERROR_CORRECTION_BLOCKS: [usize; 41] = [
    1, 1, 1, 1, 2, 2, 4, 4, 4, 5, 5, 5, 8, 9, 9, 10, 10, 11, 13, 14, 16, 17, 17, 18, 20, 21, 23,
    25, 26, 28, 29, 31, 33, 35, 37, 38, 40, 43, 45, 47, 49,
];
const MINIMUM_VERSION: usize = 1;
const MAXIMUM_VERSION: usize = 40;
/// Level M in the format information.
const LEVEL_M_FORMAT_BITS: u32 = 0;
const BYTE_MODE_INDICATOR: u32 = 0b0100;
const PAD_CODEWORDS: [u8; 2] = [0xec, 0x11];
const MASK_COUNT: usize = 8;
/// The light border scanners need around the code, in modules.
pub const QR_QUIET_ZONE_MODULES: usize = 4;

/// Dark modules, row by row; square.
pub type QrMatrix = Vec<Vec<bool>>;

fn raw_data_modules(version: usize) -> usize {
    let mut modules = (16 * version + 128) * version + 64;
    if version >= 2 {
        let alignments = version / 7 + 2;
        modules -= (25 * alignments - 10) * alignments - 55;
        if version >= 7 {
            modules -= 36;
        }
    }
    modules
}

fn ecc_per_block(version: usize) -> usize {
    ECC_CODEWORDS_PER_BLOCK.get(version).copied().unwrap_or(0)
}

fn block_count(version: usize) -> usize {
    ERROR_CORRECTION_BLOCKS.get(version).copied().unwrap_or(1)
}

fn data_codewords(version: usize) -> usize {
    raw_data_modules(version) / 8 - ecc_per_block(version) * block_count(version)
}

fn count_bits(version: usize) -> usize {
    if version <= 9 { 8 } else { 16 }
}

/// Product in GF(2^8) with the QR polynomial x^8 + x^4 + x^3 + x^2 + 1.
fn multiply(x: u8, y: u8) -> u8 {
    let mut product: u32 = 0;
    for bit in (0..8).rev() {
        product = (product << 1) ^ ((product >> 7) * 0x11d);
        product ^= u32::from((y >> bit) & 1) * u32::from(x);
    }
    // The reduction keeps it under 256.
    u8::try_from(product & 0xff).unwrap_or(0)
}

fn reed_solomon_divisor(degree: usize) -> Vec<u8> {
    let mut divisor = vec![0_u8; degree];
    if let Some(last) = divisor.last_mut() {
        *last = 1;
    }
    let mut root: u8 = 1;
    for _ in 0..degree {
        for j in 0..degree {
            divisor[j] = multiply(divisor[j], root);
            if j + 1 < degree {
                divisor[j] ^= divisor[j + 1];
            }
        }
        root = multiply(root, 0x02);
    }
    divisor
}

fn reed_solomon_remainder(data: &[u8], divisor: &[u8]) -> Vec<u8> {
    let mut remainder = vec![0_u8; divisor.len()];
    for byte in data {
        let factor = byte ^ remainder.first().copied().unwrap_or(0);
        if !remainder.is_empty() {
            remainder.remove(0);
        }
        remainder.push(0);
        for (slot, coefficient) in remainder.iter_mut().zip(divisor) {
            *slot ^= multiply(*coefficient, factor);
        }
    }
    remainder
}

/// Splits the data into blocks, adds each block's error correction, and interleaves them.
fn with_error_correction(data: &[u8], version: usize) -> Vec<u8> {
    let blocks = block_count(version);
    let ecc_length = ecc_per_block(version);
    let raw_codewords = raw_data_modules(version) / 8;
    let short_blocks = blocks - raw_codewords % blocks;
    let short_block_length = raw_codewords / blocks;
    let divisor = reed_solomon_divisor(ecc_length);
    let mut all_blocks: Vec<Vec<u8>> = Vec::with_capacity(blocks);
    let mut offset = 0;
    for i in 0..blocks {
        let length = short_block_length - ecc_length + usize::from(i >= short_blocks);
        let end = (offset + length).min(data.len());
        let block_data = data.get(offset..end).unwrap_or_default();
        offset += length;
        let mut block = block_data.to_vec();
        // Short blocks get a placeholder where long ones have their extra data codeword.
        if i < short_blocks {
            block.push(0);
        }
        block.extend(reed_solomon_remainder(block_data, &divisor));
        all_blocks.push(block);
    }
    let mut interleaved = Vec::with_capacity(raw_codewords);
    for i in 0..=short_block_length {
        for (j, block) in all_blocks.iter().enumerate() {
            if i != short_block_length - ecc_length || j >= short_blocks {
                interleaved.push(block.get(i).copied().unwrap_or(0));
            }
        }
    }
    interleaved
}

/// Mode, count, bytes, terminator and padding: the data codewords of `version`.
fn data_codewords_for(bytes: &[u8], version: usize) -> Vec<u8> {
    let mut bits: Vec<u8> = Vec::new();
    let append = |bits: &mut Vec<u8>, value: u32, length: usize| {
        for bit in (0..length).rev() {
            bits.push(u8::from((value >> bit) & 1 == 1));
        }
    };
    append(&mut bits, BYTE_MODE_INDICATOR, 4);
    append(
        &mut bits,
        u32::try_from(bytes.len()).unwrap_or(u32::MAX),
        count_bits(version),
    );
    for byte in bytes {
        append(&mut bits, u32::from(*byte), 8);
    }
    let capacity_bits = data_codewords(version) * 8;
    let terminator_length = capacity_bits.saturating_sub(bits.len()).min(4);
    append(&mut bits, 0, terminator_length);
    let to_byte_boundary = (8 - bits.len() % 8) % 8;
    append(&mut bits, 0, to_byte_boundary);
    let mut codewords: Vec<u8> = bits
        .chunks(8)
        .map(|chunk| chunk.iter().fold(0_u8, |byte, bit| (byte << 1) | bit))
        .collect();
    let mut pad = 0;
    while codewords.len() < data_codewords(version) {
        codewords.push(PAD_CODEWORDS[pad % 2]);
        pad += 1;
    }
    codewords
}

fn smallest_version_for(byte_count: usize) -> Option<usize> {
    (MINIMUM_VERSION..=MAXIMUM_VERSION)
        .find(|&version| 4 + count_bits(version) + byte_count * 8 <= data_codewords(version) * 8)
}

fn alignment_positions(version: usize, size: usize) -> Vec<usize> {
    if version == 1 {
        return Vec::new();
    }
    let count = version / 7 + 2;
    let step = if version == 32 {
        26
    } else {
        (version * 4 + 4).div_ceil(count * 2 - 2) * 2
    };
    let mut positions = vec![6];
    let mut position = size - 7;
    while positions.len() < count {
        positions.insert(1, position);
        position -= step;
    }
    positions
}

fn is_masked(mask: usize, x: usize, y: usize) -> bool {
    match mask {
        0 => (x + y).is_multiple_of(2),
        1 => y.is_multiple_of(2),
        2 => x.is_multiple_of(3),
        3 => (x + y).is_multiple_of(3),
        4 => (x / 3 + y / 2).is_multiple_of(2),
        5 => (x * y) % 2 + (x * y) % 3 == 0,
        6 => ((x * y) % 2 + (x * y) % 3).is_multiple_of(2),
        _ => ((x + y) % 2 + (x * y) % 3).is_multiple_of(2),
    }
}

struct QrSymbol {
    size: usize,
    modules: QrMatrix,
    is_function: Vec<Vec<bool>>,
}

impl QrSymbol {
    fn new(size: usize) -> Self {
        Self {
            size,
            modules: vec![vec![false; size]; size],
            is_function: vec![vec![false; size]; size],
        }
    }

    /// Out-of-range modules are ignored, as the web's optional chaining does.
    fn set_function(&mut self, x: isize, y: isize, is_dark: bool) {
        let (Ok(x), Ok(y)) = (usize::try_from(x), usize::try_from(y)) else {
            return;
        };
        if x < self.size && y < self.size {
            self.modules[y][x] = is_dark;
            self.is_function[y][x] = true;
        }
    }

    fn is_function_module(&self, x: usize, y: usize) -> bool {
        self.is_function
            .get(y)
            .and_then(|row| row.get(x))
            .copied()
            .unwrap_or(true)
    }

    fn flip(&mut self, x: usize, y: usize) {
        if let Some(module) = self.modules.get_mut(y).and_then(|row| row.get_mut(x)) {
            *module = !*module;
        }
    }
}

#[allow(clippy::cast_possible_wrap)] // Sizes stay under 200.
fn draw_function_patterns(symbol: &mut QrSymbol, version: usize) {
    let size = symbol.size as isize;
    for i in 0..size {
        symbol.set_function(6, i, i % 2 == 0);
        symbol.set_function(i, 6, i % 2 == 0);
    }
    for (centre_x, centre_y) in [(3, 3), (size - 4, 3), (3, size - 4)] {
        for dy in -4_isize..=4 {
            for dx in -4_isize..=4 {
                let distance = dx.abs().max(dy.abs());
                symbol.set_function(centre_x + dx, centre_y + dy, distance != 2 && distance != 4);
            }
        }
    }
    let positions = alignment_positions(version, symbol.size);
    let last = positions.len().saturating_sub(1);
    for (i, &x) in positions.iter().enumerate() {
        for (j, &y) in positions.iter().enumerate() {
            // The three corners hold finder patterns.
            if (i == 0 && (j == 0 || j == last)) || (i == last && j == 0) {
                continue;
            }
            for dy in -2_isize..=2 {
                for dx in -2_isize..=2 {
                    symbol.set_function(
                        x as isize + dx,
                        y as isize + dy,
                        dx.abs().max(dy.abs()) != 1,
                    );
                }
            }
        }
    }
    draw_format_bits(symbol, 0);
    if version >= 7 {
        let version_bits = u32::try_from(version).unwrap_or(0);
        let mut remainder = version_bits;
        for _ in 0..12 {
            remainder = (remainder << 1) ^ ((remainder >> 11) * 0x1f25);
        }
        let bits = (version_bits << 12) | remainder;
        for i in 0..18_isize {
            let is_dark = (bits >> i) & 1 != 0;
            let a = size - 11 + i % 3;
            let b = i / 3;
            symbol.set_function(a, b, is_dark);
            symbol.set_function(b, a, is_dark);
        }
    }
}

/// The 15 format bits (level and mask) in their two copies, plus the dark module.
#[allow(clippy::cast_possible_wrap)] // Sizes stay under 200.
fn draw_format_bits(symbol: &mut QrSymbol, mask: usize) {
    let data = (LEVEL_M_FORMAT_BITS << 3) | u32::try_from(mask).unwrap_or(0);
    let mut remainder = data;
    for _ in 0..10 {
        remainder = (remainder << 1) ^ ((remainder >> 9) * 0x537);
    }
    let bits = ((data << 10) | remainder) ^ 0x5412;
    let bit = |index: isize| (bits >> index) & 1 != 0;
    let size = symbol.size as isize;
    for i in 0..=5 {
        symbol.set_function(8, i, bit(i));
    }
    symbol.set_function(8, 7, bit(6));
    symbol.set_function(8, 8, bit(7));
    symbol.set_function(7, 8, bit(8));
    for i in 9..15 {
        symbol.set_function(14 - i, 8, bit(i));
    }
    for i in 0..8 {
        symbol.set_function(size - 1 - i, 8, bit(i));
    }
    for i in 8..15 {
        symbol.set_function(8, size - 15 + i, bit(i));
    }
    symbol.set_function(8, size - 8, true);
}

/// Codewords in the zigzag order of the standard, two columns at a time from the right.
fn draw_codewords(symbol: &mut QrSymbol, codewords: &[u8]) {
    let size = symbol.size;
    let total_bits = codewords.len() * 8;
    let mut index = 0;
    let mut right = size - 1;
    while right >= 1 {
        if right == 6 {
            right = 5;
        }
        for vertical in 0..size {
            for j in 0..2 {
                let x = right - j;
                let is_upward = (right + 1) & 2 == 0;
                let y = if is_upward {
                    size - 1 - vertical
                } else {
                    vertical
                };
                if !symbol.is_function_module(x, y) && index < total_bits {
                    let codeword = codewords.get(index >> 3).copied().unwrap_or(0);
                    if (codeword >> (7 - (index & 7))) & 1 != 0 {
                        symbol.flip(x, y);
                    }
                    index += 1;
                }
            }
        }
        if right < 2 {
            break;
        }
        right -= 2;
    }
}

fn apply_mask(symbol: &mut QrSymbol, mask: usize) {
    for y in 0..symbol.size {
        for x in 0..symbol.size {
            if !symbol.is_function_module(x, y) && is_masked(mask, x, y) {
                symbol.flip(x, y);
            }
        }
    }
}

/// Penalty rules 1 (runs of five or more), 2 (2×2 blocks) and 4 (dark/light balance) of the
/// standard. shortcut: rule 3 (finder-like patterns) is left out, as on the web; every mask is
/// valid and any one scans, the rule only nudges the choice. Add it if a phone ever struggles.
fn penalty(modules: &QrMatrix) -> usize {
    let size = modules.len();
    let at = |x: usize, y: usize| modules[y][x];
    let mut score = 0;
    let mut run_penalty = |line: &dyn Fn(usize) -> bool| {
        let mut run_length = 1;
        for i in 1..=size {
            if i < size && line(i) == line(i - 1) {
                run_length += 1;
            } else {
                if run_length >= 5 {
                    score += run_length - 2;
                }
                run_length = 1;
            }
        }
    };
    for y in 0..size {
        run_penalty(&|x| at(x, y));
        run_penalty(&|x| at(y, x));
    }
    let mut dark: usize = 0;
    for y in 0..size {
        for x in 0..size {
            let is_dark = at(x, y);
            if is_dark {
                dark += 1;
            }
            if x + 1 < size
                && y + 1 < size
                && is_dark == at(x + 1, y)
                && is_dark == at(x, y + 1)
                && is_dark == at(x + 1, y + 1)
            {
                score += 3;
            }
        }
    }
    let total = size * size;
    let imbalance = (dark * 20).abs_diff(total * 10);
    score + (imbalance.div_ceil(total).saturating_sub(1)) * 10
}

/// The smallest level-M symbol holding `text` as UTF-8, or `None` past version 40 (2 331 bytes).
#[must_use]
pub fn encode_qr_code(text: &str) -> Option<QrMatrix> {
    let bytes = text.as_bytes();
    let version = smallest_version_for(bytes.len())?;
    let mut symbol = QrSymbol::new(version * 4 + 17);
    draw_function_patterns(&mut symbol, version);
    draw_codewords(
        &mut symbol,
        &with_error_correction(&data_codewords_for(bytes, version), version),
    );
    let mut best_mask = 0;
    let mut best_penalty = usize::MAX;
    for mask in 0..MASK_COUNT {
        apply_mask(&mut symbol, mask);
        draw_format_bits(&mut symbol, mask);
        let score = penalty(&symbol.modules);
        if score < best_penalty {
            best_mask = mask;
            best_penalty = score;
        }
        // Masking twice restores the modules.
        apply_mask(&mut symbol, mask);
    }
    apply_mask(&mut symbol, best_mask);
    draw_format_bits(&mut symbol, best_mask);
    Some(symbol.modules)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(text: &str) -> Vec<String> {
        encode_qr_code(text)
            .unwrap_or_default()
            .iter()
            .map(|row| {
                row.iter()
                    .map(|&dark| if dark { '1' } else { '0' })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn draws_the_symbol_a_reader_decodes_back_to_the_text() {
        // The web's test vector, read back as "MELLOW" by a scanner: a change of output is a bug.
        assert_eq!(
            rows("MELLOW"),
            [
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
            ]
        );
    }

    #[test]
    fn takes_the_smallest_version_that_holds_the_bytes_at_level_m() {
        assert_eq!(rows(&"a".repeat(14)).len(), 21);
        assert_eq!(rows(&"a".repeat(15)).len(), 25);
        let uri = "otpauth://totp/Mellow%20(chat.example.org):ana?secret=JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP&issuer=Mellow%20(chat.example.org)&algorithm=SHA1&digits=6&period=30";
        assert_eq!(rows(uri).len(), 53);
        assert_eq!(
            encode_qr_code(&"a".repeat(2331)).map(|matrix| matrix.len()),
            Some(177)
        );
        assert_eq!(encode_qr_code(&"a".repeat(2332)), None);
        // Version 7 and up carry version bits: still square and full size.
        assert_eq!(rows(&"a".repeat(200)).len(), 4 * 10 + 17);
    }

    #[test]
    fn puts_the_three_finder_patterns_in_the_corners() {
        let symbol = rows("finder patterns");
        let size = symbol.len();
        let finder = [
            "1111111", "1000001", "1011101", "1011101", "1011101", "1000001", "1111111",
        ];
        for (left, top) in [(0, 0), (size - 7, 0), (0, size - 7)] {
            let corner: Vec<&str> = symbol[top..top + 7]
                .iter()
                .map(|row| &row[left..left + 7])
                .collect();
            assert_eq!(corner, finder);
        }
    }
}
