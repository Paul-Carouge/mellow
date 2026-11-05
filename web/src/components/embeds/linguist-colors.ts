// The colours GitHub shows beside a repository's language, for 60 widely used languages, taken
// from GitHub Linguist v9.3.0 (`lib/linguist/languages.yml`, https://github.com/github-linguist/linguist).
// Linguist is released under the MIT licence:
//
// Copyright (c) 2017 GitHub, Inc.
//
// Permission is hereby granted, free of charge, to any person obtaining a copy of this software and
// associated documentation files (the "Software"), to deal in the Software without restriction,
// including without limitation the rights to use, copy, modify, merge, publish, distribute,
// sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all copies or
// substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT
// NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
// NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM,
// DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT
// OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
//
// These are facts about the languages, not Mellow's colours: they appear only as the small dot
// beside the language's name on a repository card (docs/design/web-provider-embeds.md §4.3).

/** `#rrggbb` by language name, as GitHub's API names the language. */
export const LINGUIST_LANGUAGE_COLORS: Readonly<Record<string, string>> = {
  JavaScript: "#f1e05a",
  TypeScript: "#3178c6",
  Python: "#3572a5",
  Java: "#b07219",
  "C#": "#178600",
  "C++": "#f34b7d",
  C: "#555555",
  PHP: "#4f5d95",
  Shell: "#89e051",
  Go: "#00add8",
  Rust: "#dea584",
  Kotlin: "#a97bff",
  Swift: "#f05138",
  Ruby: "#701516",
  Dart: "#00b4ab",
  Scala: "#c22d40",
  Lua: "#000080",
  R: "#198ce7",
  "Objective-C": "#438eff",
  Perl: "#0298c3",
  Haskell: "#5e5086",
  Elixir: "#6e4a7e",
  Erlang: "#b83998",
  Clojure: "#db5855",
  "F#": "#b845fc",
  OCaml: "#ef7a08",
  Julia: "#a270ba",
  Zig: "#ec915c",
  Nim: "#ffc200",
  Crystal: "#000100",
  Groovy: "#4298b8",
  PowerShell: "#012456",
  "Vim Script": "#199f4b",
  "Emacs Lisp": "#c065db",
  Assembly: "#6e4c13",
  Fortran: "#4d41b1",
  MATLAB: "#e16737",
  "Jupyter Notebook": "#da5b0b",
  HTML: "#e34c26",
  CSS: "#663399",
  SCSS: "#c6538c",
  Less: "#1d365d",
  Vue: "#41b883",
  Svelte: "#ff3e00",
  Astro: "#ff5a03",
  TeX: "#3d6117",
  Nix: "#7e7eff",
  Dockerfile: "#384d54",
  Makefile: "#427819",
  CMake: "#da3434",
  HCL: "#844fba",
  Solidity: "#aa6746",
  GDScript: "#355570",
  "Visual Basic .NET": "#945db7",
  Elm: "#60b5cc",
  "Common Lisp": "#3fb68b",
  Scheme: "#1e4aec",
  Racket: "#3c5caa",
  Gleam: "#ffaff3",
  D: "#ba595e",
};
