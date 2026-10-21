import { MediaQuery } from "svelte/reactivity";

// Animated pictures and Melli never play under reduced motion (DESIGN.md §2.8).
export const prefersReducedMotion = new MediaQuery("prefers-reduced-motion: reduce");
