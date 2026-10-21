<script lang="ts">
  import { prefersReducedMotion } from "./Avatar.svelte";
  import type { MelliState } from "./melli";

  // Melli, the Mellow mascot and logo (DESIGN.md §2.10). One named part per element: a state only
  // shows, hides or restyles parts, so adding one is a few CSS rules. `motion` parts loop; poses
  // and colours ease between states. Decorative: the host names what Melli stands for.
  interface Props {
    state?: MelliState;
    /** A `--mw-*` length token, for example "--mw-component-rail-item-size". */
    sizeToken: string;
    /** Off when the host paints the tile itself (the Home button morphs its own). */
    hasTile?: boolean;
    /** The still pose of the state even when motion is allowed (the gallery); reduced motion always gets it. */
    isStill?: boolean;
  }

  let { state = "idle", sizeToken, hasTile = true, isStill = false }: Props = $props();
</script>

<svg
  class={["melli", (isStill || prefersReducedMotion.current) && "is-still"]}
  data-state={state}
  viewBox="0 0 256 256"
  aria-hidden="true"
  focusable="false"
  style:width="var({sizeToken})"
  style:height="var({sizeToken})"
>
  {#if hasTile}
    <rect class="tile" width="256" height="256" rx="56" />
  {/if}
  <ellipse class="shadow" cx="128" cy="222" rx="54" ry="7" />
  <!-- Behind the cloud, so each drop falls out of it. -->
  <g class="mark drops">
    <path class="drop" d="M104 194C108 200 111 204 111 207A7 7 0 0 1 97 207C97 204 100 200 104 194Z" />
    <path class="drop" d="M128 194C132 200 135 204 135 207A7 7 0 0 1 121 207C121 204 124 200 128 194Z" />
    <path class="drop" d="M152 194C156 200 159 204 159 207A7 7 0 0 1 145 207C145 204 148 200 152 194Z" />
  </g>
  <g class="motion">
    <g class="puff">
      <circle cx="128" cy="124" r="62" />
      <circle cx="128" cy="78" r="30" />
      <circle cx="167.8" cy="101" r="30" />
      <circle cx="167.8" cy="147" r="30" />
      <circle cx="128" cy="170" r="30" />
      <circle cx="88.2" cy="147" r="30" />
      <circle cx="88.2" cy="101" r="30" />
    </g>
    <g class="face">
      <g class="blush">
        <ellipse cx="100" cy="144" rx="10" ry="6" />
        <ellipse cx="156" cy="144" rx="10" ry="6" />
      </g>
      <g class="eyes">
        <g class="eyes-blink">
          <rect x="110" y="112" width="12" height="24" rx="6" />
          <rect x="134" y="112" width="12" height="24" rx="6" />
          <circle class="glint" cx="118" cy="118" r="3" />
          <circle class="glint" cx="142" cy="118" r="3" />
        </g>
      </g>
      <g class="stroke eyes-happy">
        <path d="M109 129Q116 114 123 129" />
        <path d="M133 129Q140 114 147 129" />
      </g>
      <g class="stroke eyes-closed">
        <path d="M109 124Q116 132 123 124" />
        <path d="M133 124Q140 132 147 124" />
      </g>
      <ellipse class="mouth" cx="128" cy="150" rx="6" ry="5" />
    </g>
  </g>
  <g class="mark dots">
    <circle class="dot" cx="156" cy="44" r="8" />
    <circle class="dot" cx="180" cy="44" r="8" />
    <circle class="dot" cx="204" cy="44" r="8" />
  </g>
  <g class="mark sparkle">
    <path class="star" d="M194 36Q197 55 216 58Q197 61 194 80Q191 61 172 58Q191 55 194 36Z" />
    <path class="star star-small" d="M58 34Q59.5 42.5 68 44Q59.5 45.5 58 54Q56.5 45.5 48 44Q56.5 42.5 58 34Z" />
  </g>
  <g class="stroke mark rings">
    <path d="M46 100Q34 124 46 148" />
    <path d="M30 88Q14 124 30 160" />
    <path d="M210 100Q222 124 210 148" />
    <path d="M226 88Q242 124 226 160" />
  </g>
  <g class="stroke mark waves">
    <path class="wave" d="M210 106Q221 124 210 142" />
    <path class="wave" d="M226 94Q241 124 226 154" />
  </g>
  <g class="stroke mark arrow">
    <path d="M200 84V44M186 58L200 44L214 58" />
  </g>
  <g class="stroke mark zzz">
    <path class="z" d="M164 58H180L164 74H180" />
    <path class="z z-small" d="M188 34H200L188 46H200" />
  </g>
</svg>

<style>
  /* Keyframe amplitudes are in viewBox units: they are part of the drawing, like the paths.
     Colours and timings are tokens (DESIGN.md §2.10). */
  .melli {
    --change: var(--mw-component-melli-state-change-duration) var(--mw-component-melli-state-change-easing);
    display: block;
    flex-shrink: 0;
  }
  /* Small parts scale and turn around their own centre, as in the logo's source. */
  .shadow,
  .eyes,
  .eyes-blink,
  .mouth,
  .mark,
  .mark > * {
    transform-box: fill-box;
    transform-origin: center;
  }

  /* Parts */
  .tile {
    fill: var(--mw-color-mascot-tile);
  }
  .shadow {
    fill: var(--mw-color-mascot-shadow);
    animation: shadow-float var(--mw-component-melli-float-duration) ease-in-out infinite;
  }
  .motion {
    /* Squash, stretch and rocking pivot on the cloud's base. */
    transform-box: view-box;
    transform-origin: 128px 200px;
    animation: float var(--mw-component-melli-float-duration) ease-in-out infinite;
  }
  .puff {
    fill: var(--mw-color-mascot-body);
    transform-box: view-box;
    transform-origin: 128px 124px;
    transition: fill var(--change);
    animation: spin var(--mw-component-melli-puff-spin-duration) linear infinite;
  }
  .face {
    transition: transform var(--change);
  }
  .eyes {
    fill: var(--mw-color-mascot-ink);
    transform-origin: center bottom;
    transition:
      transform var(--change),
      opacity var(--change);
  }
  .eyes-blink {
    animation: blink var(--mw-component-melli-blink-duration) infinite;
  }
  .glint {
    fill: var(--mw-color-mascot-body);
    opacity: 0;
    transition: opacity var(--change);
  }
  .blush {
    fill: var(--mw-color-mascot-blush);
  }
  .stroke {
    fill: none;
    stroke: var(--mw-color-mascot-ink);
    stroke-width: 8;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .mouth,
  .dots,
  .sparkle,
  .drops {
    fill: var(--mw-color-mascot-ink);
  }
  .blush,
  .eyes-happy,
  .eyes-closed,
  .mouth,
  .mark {
    opacity: 0;
    transition: opacity var(--change);
  }

  /* connecting: looks up at the dots while it waits. */
  [data-state="connecting"] .face {
    transform: translate(3px, -5px);
  }
  [data-state="connecting"] .dots {
    opacity: 1;
  }
  [data-state="connecting"] .dot {
    animation: wait var(--mw-component-melli-wait-duration) ease-in-out infinite;
  }
  [data-state="connecting"] .dot:nth-child(2) {
    animation-delay: calc(var(--mw-component-melli-wait-duration) / 6);
  }
  [data-state="connecting"] .dot:nth-child(3) {
    animation-delay: calc(var(--mw-component-melli-wait-duration) / 3);
  }

  /* connection-lost: grey, heavy-lidded, stalled, raining. */
  [data-state="connection-lost"] .puff {
    fill: var(--mw-color-mascot-body-overcast);
    animation-play-state: paused;
  }
  [data-state="connection-lost"] .eyes {
    transform: scaleY(0.45);
  }
  [data-state="connection-lost"] .motion {
    animation-name: sag;
  }
  [data-state="connection-lost"] .shadow {
    animation: none;
  }
  [data-state="connection-lost"] .drops {
    opacity: 1;
  }
  [data-state="connection-lost"] .drop {
    animation: rain var(--mw-component-melli-rain-duration) ease-in infinite;
  }
  [data-state="connection-lost"] .drop:nth-child(2) {
    animation-delay: calc(var(--mw-component-melli-rain-duration) / -3);
  }
  [data-state="connection-lost"] .drop:nth-child(3) {
    animation-delay: calc(var(--mw-component-melli-rain-duration) / -1.5);
  }

  /* notification: happy eyes, blush, a hop and a sparkle. */
  [data-state="notification"] .eyes {
    opacity: 0;
  }
  [data-state="notification"] :is(.eyes-happy, .blush, .sparkle) {
    opacity: 1;
  }
  [data-state="notification"] .motion {
    animation: hop var(--mw-component-melli-hop-duration) ease-in-out infinite;
  }
  [data-state="notification"] .shadow {
    animation: shadow-hop var(--mw-component-melli-hop-duration) ease-in-out infinite;
  }
  [data-state="notification"] .star {
    animation: twinkle var(--mw-component-melli-hop-duration) ease-in-out infinite;
  }
  [data-state="notification"] .star-small {
    animation-delay: calc(var(--mw-component-melli-hop-duration) / -2);
  }

  /* incoming-call: wide eyes, rocking like a ringing phone. */
  [data-state="incoming-call"] .eyes {
    transform: scaleY(1.15);
  }
  [data-state="incoming-call"] .rings {
    opacity: 1;
    animation: ring-flash var(--mw-component-melli-ring-duration) linear infinite;
  }
  [data-state="incoming-call"] .motion {
    animation: ring var(--mw-component-melli-ring-duration) ease-in-out infinite;
  }
  [data-state="incoming-call"] .shadow {
    animation: none;
  }

  /* in-call: a calm bob, talking, sound waves. */
  [data-state="in-call"] :is(.mouth, .waves) {
    opacity: 1;
  }
  [data-state="in-call"] .motion {
    animation: bob var(--mw-component-melli-talk-duration) ease-in-out infinite;
  }
  [data-state="in-call"] .shadow {
    animation: none;
  }
  [data-state="in-call"] .mouth {
    animation: talk calc(var(--mw-component-melli-talk-duration) / 2) ease-in-out infinite;
  }
  [data-state="in-call"] .wave {
    animation: wave var(--mw-component-melli-talk-duration) ease-in-out infinite;
  }
  [data-state="in-call"] .wave:nth-child(2) {
    animation-delay: calc(var(--mw-component-melli-talk-duration) / 6);
  }

  /* update-available: sparkly eyes, a double bounce, an arrow pointing up. */
  [data-state="update-available"] :is(.glint, .blush, .arrow) {
    opacity: 1;
  }
  [data-state="update-available"] .motion {
    animation: bounce var(--mw-component-melli-excited-duration) ease-out infinite;
  }
  [data-state="update-available"] .shadow {
    animation: none;
  }
  [data-state="update-available"] .arrow {
    animation: rise var(--mw-component-melli-excited-duration) ease-in-out infinite;
  }

  /* sleeping: eyes closed, slow breath, drifting Zs. */
  [data-state="sleeping"] .eyes {
    opacity: 0;
  }
  [data-state="sleeping"] :is(.eyes-closed, .zzz) {
    opacity: 1;
  }
  [data-state="sleeping"] .motion {
    animation: breathe var(--mw-component-melli-sleep-duration) ease-in-out infinite;
  }
  [data-state="sleeping"] .shadow {
    animation: shadow-breathe var(--mw-component-melli-sleep-duration) ease-in-out infinite;
  }
  [data-state="sleeping"] .z {
    animation: drift var(--mw-component-melli-sleep-duration) ease-out infinite;
  }
  [data-state="sleeping"] .z-small {
    animation-delay: calc(var(--mw-component-melli-sleep-duration) / -2);
  }

  /* Still: no motion, the pose alone shows the state (reduced motion, DESIGN.md §2.8). */
  .is-still,
  .is-still * {
    animation: none !important;
    transition: none !important;
  }
  .is-still[data-state="incoming-call"] .motion {
    transform: rotate(-10deg);
  }
  .is-still[data-state="notification"] .motion {
    transform: translateY(-6px);
  }

  @keyframes float {
    50% {
      transform: translateY(-8px);
    }
  }
  @keyframes shadow-float {
    50% {
      transform: scaleX(0.82);
      opacity: 0.7;
    }
  }
  @keyframes blink {
    0%,
    92%,
    100% {
      transform: scaleY(1);
    }
    95% {
      transform: scaleY(0.1);
    }
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  @keyframes wait {
    0%,
    60%,
    100% {
      opacity: 0.3;
      transform: translateY(0);
    }
    30% {
      opacity: 1;
      transform: translateY(-6px);
    }
  }
  @keyframes sag {
    50% {
      transform: translateY(4px) scale(1.02, 0.98);
    }
  }
  @keyframes rain {
    0% {
      opacity: 0;
      transform: translateY(-24px);
    }
    25%,
    75% {
      opacity: 1;
    }
    100% {
      opacity: 0;
      transform: translateY(16px);
    }
  }
  @keyframes hop {
    0%,
    60%,
    100% {
      transform: none;
    }
    12% {
      transform: scale(1.08, 0.9);
    }
    28% {
      transform: translateY(-22px) scale(0.94, 1.08);
    }
    44% {
      transform: scale(1.06, 0.92);
    }
  }
  @keyframes shadow-hop {
    0%,
    60%,
    100% {
      transform: none;
    }
    12% {
      transform: scaleX(1.08);
    }
    28% {
      transform: scaleX(0.7);
      opacity: 0.5;
    }
  }
  @keyframes twinkle {
    0%,
    60%,
    100% {
      transform: scale(0.8);
    }
    28% {
      transform: scale(1.2) rotate(20deg);
    }
  }
  @keyframes ring {
    0%,
    34%,
    100% {
      transform: none;
    }
    4% {
      transform: rotate(-9deg);
    }
    9% {
      transform: rotate(9deg);
    }
    14% {
      transform: rotate(-9deg);
    }
    19% {
      transform: rotate(9deg);
    }
    24% {
      transform: rotate(-5deg);
    }
    29% {
      transform: rotate(3deg);
    }
  }
  @keyframes ring-flash {
    0%,
    34% {
      opacity: 1;
    }
    45%,
    100% {
      opacity: 0.3;
    }
  }
  @keyframes bob {
    50% {
      transform: translateY(-4px);
    }
  }
  @keyframes talk {
    0%,
    100% {
      transform: scaleY(0.4);
    }
    25% {
      transform: scaleY(1.1);
    }
    50% {
      transform: scaleY(0.6);
    }
    75% {
      transform: scaleY(1);
    }
  }
  @keyframes wave {
    0%,
    100% {
      opacity: 0.25;
    }
    40% {
      opacity: 1;
    }
  }
  @keyframes bounce {
    0%,
    40%,
    100% {
      transform: none;
    }
    10% {
      transform: translateY(-12px) scale(0.96, 1.05);
    }
    20% {
      transform: scale(1.04, 0.96);
    }
    30% {
      transform: translateY(-8px);
    }
  }
  @keyframes rise {
    0%,
    100% {
      transform: none;
    }
    50% {
      transform: translateY(-8px);
    }
  }
  @keyframes breathe {
    50% {
      transform: scale(1.04, 0.96);
    }
  }
  @keyframes shadow-breathe {
    50% {
      transform: scaleX(1.06);
    }
  }
  @keyframes drift {
    0% {
      opacity: 0;
      transform: translate(-4px, 8px);
    }
    30%,
    60% {
      opacity: 1;
    }
    100% {
      opacity: 0;
      transform: translate(6px, -12px);
    }
  }
</style>
