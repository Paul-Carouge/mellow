import { MediaQuery } from "svelte/reactivity";

// The theme follows the system (DESIGN.md §2): colours computed in script read it to update.
export const prefersDarkColorScheme = new MediaQuery("prefers-color-scheme: dark");
