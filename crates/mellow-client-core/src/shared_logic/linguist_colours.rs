//! The colours GitHub shows beside a repository's language, for 60 widely used languages, taken
//! from GitHub Linguist v9.3.0 (`lib/linguist/languages.yml`, <https://github.com/github-linguist/linguist>).
//! Linguist is released under the MIT licence:
//!
//! Copyright (c) 2017 GitHub, Inc.
//!
//! Permission is hereby granted, free of charge, to any person obtaining a copy of this software and
//! associated documentation files (the "Software"), to deal in the Software without restriction,
//! including without limitation the rights to use, copy, modify, merge, publish, distribute,
//! sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is
//! furnished to do so, subject to the following conditions:
//!
//! The above copyright notice and this permission notice shall be included in all copies or
//! substantial portions of the Software.
//!
//! THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT
//! NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
//! NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM,
//! DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT
//! OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
//!
//! These are facts about the languages, not Mellow's colours: they appear only as the small dot
//! beside the language's name on a repository card (`docs/design/web-provider-embeds.md` §4.3).
//! Ported from `web/src/components/embeds/linguist-colors.ts`.

/// `0xRRGGBB` by language name, as GitHub's API names the language.
pub(super) const LINGUIST_LANGUAGE_COLOURS: [(&str, u32); 60] = [
    ("JavaScript", 0x00f1_e05a),
    ("TypeScript", 0x0031_78c6),
    ("Python", 0x0035_72a5),
    ("Java", 0x00b0_7219),
    ("C#", 0x0017_8600),
    ("C++", 0x00f3_4b7d),
    ("C", 0x0055_5555),
    ("PHP", 0x004f_5d95),
    ("Shell", 0x0089_e051),
    ("Go", 0x0000_add8),
    ("Rust", 0x00de_a584),
    ("Kotlin", 0x00a9_7bff),
    ("Swift", 0x00f0_5138),
    ("Ruby", 0x0070_1516),
    ("Dart", 0x0000_b4ab),
    ("Scala", 0x00c2_2d40),
    ("Lua", 0x0000_0080),
    ("R", 0x0019_8ce7),
    ("Objective-C", 0x0043_8eff),
    ("Perl", 0x0002_98c3),
    ("Haskell", 0x005e_5086),
    ("Elixir", 0x006e_4a7e),
    ("Erlang", 0x00b8_3998),
    ("Clojure", 0x00db_5855),
    ("F#", 0x00b8_45fc),
    ("OCaml", 0x00ef_7a08),
    ("Julia", 0x00a2_70ba),
    ("Zig", 0x00ec_915c),
    ("Nim", 0x00ff_c200),
    ("Crystal", 0x0000_0100),
    ("Groovy", 0x0042_98b8),
    ("PowerShell", 0x0001_2456),
    ("Vim Script", 0x0019_9f4b),
    ("Emacs Lisp", 0x00c0_65db),
    ("Assembly", 0x006e_4c13),
    ("Fortran", 0x004d_41b1),
    ("MATLAB", 0x00e1_6737),
    ("Jupyter Notebook", 0x00da_5b0b),
    ("HTML", 0x00e3_4c26),
    ("CSS", 0x0066_3399),
    ("SCSS", 0x00c6_538c),
    ("Less", 0x001d_365d),
    ("Vue", 0x0041_b883),
    ("Svelte", 0x00ff_3e00),
    ("Astro", 0x00ff_5a03),
    ("TeX", 0x003d_6117),
    ("Nix", 0x007e_7eff),
    ("Dockerfile", 0x0038_4d54),
    ("Makefile", 0x0042_7819),
    ("CMake", 0x00da_3434),
    ("HCL", 0x0084_4fba),
    ("Solidity", 0x00aa_6746),
    ("GDScript", 0x0035_5570),
    ("Visual Basic .NET", 0x0094_5db7),
    ("Elm", 0x0060_b5cc),
    ("Common Lisp", 0x003f_b68b),
    ("Scheme", 0x001e_4aec),
    ("Racket", 0x003c_5caa),
    ("Gleam", 0x00ff_aff3),
    ("D", 0x00ba_595e),
];
