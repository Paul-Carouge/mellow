<script lang="ts">
  import { strings } from "../i18n";
  import type { IconCrop } from "../lib/api";
  import { prefersReducedMotion } from "../lib/motion.svelte";
  import Button from "./Button.svelte";
  import {
    type CropSquare,
    type PictureSize,
    centredSquare,
    cropMovedBy,
    cropToSend,
    cropZoomedTo,
    maximumZoom,
    placementInPercent,
    zoomOf,
  } from "./icon-crop";
  import ModalDialog from "./ModalDialog.svelte";

  // DESIGN.md §6.54: the chosen picture on a stage, the kept square framed and the rest dimmed
  // around it. It opens on the centred square, which is what the instance keeps when nothing is
  // moved; dragging, the wheel, a pinch, the arrow keys or the zoom slider choose another one.
  // Under the stage, the result at its real size, where it will live: the server strip or a
  // channel row. The file is sent as it is (an animated GIF stays animated), with the square.
  interface Props {
    file: File;
    kind: "server" | "channel";
    /** The server's or channel's name, for the samples. */
    name: string;
    isBusy: boolean;
    errorMessage: string;
    onclose: () => void;
    onsave: (crop: IconCrop | null) => void;
  }

  let { file, kind, name, isBusy, errorMessage, onclose, onsave }: Props = $props();

  /** Each arrow key moves the square by this share of its side; with Shift, by the larger one. */
  const KEYBOARD_STEP_SHARE = 0.02;
  const KEYBOARD_LARGE_STEP_SHARE = 0.1;
  const KEYBOARD_ZOOM_FACTOR = 1.1;
  /** How much one wheel pixel zooms: about 10 % per notch of a mouse wheel. */
  const WHEEL_ZOOM_SENSITIVITY = 0.001;
  const ZOOM_SLIDER_STEP = 0.01;

  const id = $props.id();
  let pictureUrl = $state<string | null>(null);
  let picture = $state<PictureSize | null>(null);
  let isUnreadable = $state(false);
  let crop = $state<CropSquare>({ x: 0, y: 0, size: 1 });
  let frameSide = $state(0);
  let frame = $state<HTMLElement | null>(null);
  let stage = $state<HTMLElement | null>(null);
  const pointers = new Map<number, { x: number; y: number }>();

  // Channel icons are always stored still, and nothing plays under reduced motion: such pictures
  // are shown from their first frame. The file sent stays the one chosen.
  const isStillWanted = $derived(kind === "channel" || prefersReducedMotion.current);

  $effect(() => {
    let url: string | null = null;
    let isCancelled = false;
    const mayMove = file.type === "image/gif" || file.type === "image/webp";
    void (isStillWanted && mayMove ? stillPictureUrl(file) : Promise.resolve(URL.createObjectURL(file)))
      .then((created) => {
        url = created;
        if (isCancelled) {
          URL.revokeObjectURL(created);
        } else {
          pictureUrl = created;
        }
      })
      .catch(() => {
        // createImageBitmap refuses what it cannot decode: the picture is unreadable here too.
        isUnreadable = true;
      });
    return () => {
      isCancelled = true;
      if (url) {
        URL.revokeObjectURL(url);
      }
    };
  });

  async function stillPictureUrl(source: File): Promise<string> {
    const bitmap = await createImageBitmap(source);
    const canvas = document.createElement("canvas");
    canvas.width = bitmap.width;
    canvas.height = bitmap.height;
    canvas.getContext("2d")?.drawImage(bitmap, 0, 0);
    bitmap.close();
    const blob = await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve));
    if (!blob) {
      throw new Error("the first frame could not be drawn");
    }
    return URL.createObjectURL(blob);
  }

  /** The wheel zooms around the pointer, instead of scrolling the dialog. */
  function zoomWithWheel(event: WheelEvent) {
    if (!picture) {
      return;
    }
    event.preventDefault();
    const zoom = zoomOf(crop, picture) * Math.exp(-event.deltaY * WHEEL_ZOOM_SENSITIVITY);
    crop = cropZoomedTo(crop, picture, zoom, pointInPicture(event.clientX, event.clientY));
  }

  /** CSS pixels per picture pixel inside the frame. */
  const scale = $derived(frameSide > 0 ? frameSide / crop.size : 1);

  const placement = $derived(picture ? placementInPercent(crop, picture) : null);
  const zoom = $derived(picture ? zoomOf(crop, picture) : 1);
  const zoomLimit = $derived(picture ? maximumZoom(picture) : 1);

  function pointInPicture(clientX: number, clientY: number): { x: number; y: number } {
    const bounds = frame?.getBoundingClientRect();
    if (!bounds) {
      return { x: crop.x + crop.size / 2, y: crop.y + crop.size / 2 };
    }
    return { x: crop.x + (clientX - bounds.left) / scale, y: crop.y + (clientY - bounds.top) / scale };
  }

  function startDrag(event: PointerEvent) {
    if (!picture) {
      return;
    }
    stage?.setPointerCapture(event.pointerId);
    pointers.set(event.pointerId, { x: event.clientX, y: event.clientY });
  }

  function drag(event: PointerEvent) {
    const previous = pointers.get(event.pointerId);
    if (!picture || !previous) {
      return;
    }
    const others = [...pointers.entries()].filter(([pointerId]) => pointerId !== event.pointerId);
    const other = others[0]?.[1];
    if (other) {
      // A pinch: the distance between the two fingers scales the zoom, around their midpoint.
      const before = Math.hypot(previous.x - other.x, previous.y - other.y);
      const after = Math.hypot(event.clientX - other.x, event.clientY - other.y);
      if (before > 0) {
        const midpoint = pointInPicture((event.clientX + other.x) / 2, (event.clientY + other.y) / 2);
        crop = cropZoomedTo(crop, picture, zoom * (after / before), midpoint);
      }
    } else {
      // Dragging the picture right shows more of its left: the square moves the other way.
      crop = cropMovedBy(crop, picture, -(event.clientX - previous.x) / scale, -(event.clientY - previous.y) / scale);
    }
    pointers.set(event.pointerId, { x: event.clientX, y: event.clientY });
  }

  function endDrag(event: PointerEvent) {
    pointers.delete(event.pointerId);
  }

  function moveWithKeys(event: KeyboardEvent) {
    if (!picture) {
      return;
    }
    const step = crop.size * (event.shiftKey ? KEYBOARD_LARGE_STEP_SHARE : KEYBOARD_STEP_SHARE);
    // The keys move the picture, as dragging does.
    const moves: Record<string, [number, number]> = {
      ArrowLeft: [step, 0],
      ArrowRight: [-step, 0],
      ArrowUp: [0, step],
      ArrowDown: [0, -step],
    };
    const move = moves[event.key];
    if (move) {
      event.preventDefault();
      crop = cropMovedBy(crop, picture, move[0], move[1]);
    } else if (event.key === "+" || event.key === "=") {
      event.preventDefault();
      crop = cropZoomedTo(crop, picture, zoom * KEYBOARD_ZOOM_FACTOR);
    } else if (event.key === "-" || event.key === "_") {
      event.preventDefault();
      crop = cropZoomedTo(crop, picture, zoom / KEYBOARD_ZOOM_FACTOR);
    }
  }
</script>

{#snippet croppedPicture(alt: string)}
  {#if pictureUrl && placement}
    <img
      src={pictureUrl}
      {alt}
      draggable="false"
      class="absolute h-auto max-w-none select-none"
      style:left="{placement.left}%"
      style:top="{placement.top}%"
      style:width="{placement.width}%"
    />
  {/if}
{/snippet}

{#snippet sample(label: string, shapeClass: string)}
  <figure class="flex flex-col items-center gap-2xs">
    <span
      class={["relative size-(--mw-component-rail-item-size) overflow-hidden bg-(--mw-component-icon-picker-stage-background)", shapeClass]}
    >
      {@render croppedPicture("")}
    </span>
    <figcaption class="type-body-sm text-(--mw-component-icon-picker-sample-label-text)">{label}</figcaption>
  </figure>
{/snippet}

<ModalDialog
  title={strings.iconEditor.crop.title}
  subtitle={strings.iconEditor.crop.subtitle}
  errorMessage={isUnreadable
    ? strings.errors.withReason(strings.iconEditor.changeFailed, strings.profile.unreadable)
    : errorMessage}
  {isBusy}
  submitLabel={isBusy ? strings.iconEditor.crop.saving : strings.iconEditor.crop.save}
  isSubmitEnabled={picture !== null}
  {onclose}
  onsubmit={() => {
    if (picture) {
      onsave(cropToSend(crop, picture));
    }
  }}
>
  <div class="flex flex-col gap-(--mw-component-dialog-section-gap)" aria-busy={isBusy}>
    <!-- An application, so screen readers hand it the arrow keys, + and −; the slider and
         "Centre it" do the same for everyone else. Svelte counts `application` as non-interactive. -->
    <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
    <div
      bind:this={stage}
      role="application"
      tabindex="0"
      data-initial-focus
      aria-label={strings.iconEditor.crop.stage}
      aria-describedby="{id}-hint"
      class={[
        "relative aspect-square w-full touch-none overflow-hidden rounded-(--mw-component-icon-picker-stage-radius) bg-(--mw-component-icon-picker-stage-background) focus-visible:focus-ring",
        picture ? "cursor-grab active:cursor-grabbing" : "",
      ]}
      onpointerdown={startDrag}
      onpointermove={drag}
      onpointerup={endDrag}
      onpointercancel={endDrag}
      onkeydown={moveWithKeys}
      onwheel={zoomWithWheel}
    >
      <div bind:this={frame} bind:clientWidth={frameSide} class="absolute inset-(--mw-component-icon-picker-frame-inset)">
        {@render croppedPicture("")}
      </div>
      {#if picture}
        <!-- The spread shadow dims everything outside the square, whatever the stage's size. -->
        <div
          aria-hidden="true"
          class="pointer-events-none absolute inset-(--mw-component-icon-picker-frame-inset) border-(length:--mw-component-icon-picker-frame-border-width) border-(--mw-component-icon-picker-frame-border) shadow-[0_0_0_100vmax_var(--mw-component-icon-picker-outside-scrim)]"
        ></div>
      {/if}
      {#if pictureUrl}
        <!-- Only measured: the visible copies above are placed from its natural size. -->
        <img
          src={pictureUrl}
          alt=""
          aria-hidden="true"
          class="invisible absolute size-px"
          onload={(event) => {
            const image = event.currentTarget;
            if (image instanceof HTMLImageElement) {
              picture = { width: image.naturalWidth, height: image.naturalHeight };
              crop = centredSquare(picture);
            }
          }}
          onerror={() => (isUnreadable = true)}
        />
      {/if}
      {#if !picture && !isUnreadable}
        <p class="absolute inset-0 grid place-items-center type-body text-(--mw-component-icon-picker-sample-label-text)">
          {strings.iconEditor.crop.opening}
        </p>
      {/if}
    </div>
    <p id="{id}-hint" class="-mt-xs type-body-sm text-(--mw-component-icon-picker-sample-label-text)">
      {strings.iconEditor.crop.stageHint}
    </p>

    <div class="flex flex-wrap items-center gap-(--mw-component-dialog-button-gap)">
      <label for="{id}-zoom" class="type-category text-(--mw-component-text-field-label-text)">{strings.iconEditor.crop.zoom}</label>
      <input
        id="{id}-zoom"
        type="range"
        min="1"
        max={zoomLimit}
        step={ZOOM_SLIDER_STEP}
        value={zoom}
        disabled={!picture || zoomLimit === 1}
        aria-valuetext="{Math.round(zoom * 100)} %"
        class="h-(--mw-size-hit-web-min) min-w-0 flex-1 accent-text-heading"
        oninput={(event) => {
          if (picture) {
            crop = cropZoomedTo(crop, picture, Number(event.currentTarget.value));
          }
        }}
      />
      <Button
        variant="ghost"
        isDisabled={!picture}
        onclick={() => {
          if (picture) {
            crop = centredSquare(picture);
          }
        }}
      >
        {strings.iconEditor.crop.centre}
      </Button>
    </div>

    {#if picture}
      <!-- What sighted people check by eye; the stage already holds the picture for everyone. -->
      <section aria-hidden="true" class="flex flex-col gap-(--mw-component-settings-section-label-gap)">
        <h3 class="type-category text-(--mw-component-settings-section-label-text)">
          {strings.iconEditor.crop.samples}
        </h3>
        {#if kind === "server"}
          <!-- The strip's two shapes: a circle at rest, the squircle when hovered or selected. -->
          <div class="flex gap-(--mw-component-icon-picker-sample-gap)">
            {@render sample(strings.iconEditor.crop.sampleAtRest, "rounded-(--mw-component-rail-icon-radius-rest)")}
            {@render sample(strings.iconEditor.crop.sampleSelected, "rounded-(--mw-component-rail-icon-radius-active)")}
          </div>
        {:else}
          <!-- A channel row on the navigator, read and selected, at the list's own size. -->
          <div class="flex flex-col gap-(--mw-component-channel-row-gap) rounded-(--mw-component-icon-picker-stage-radius) bg-(--mw-component-navigator-background) p-xs">
            {#each [false, true] as isSelected (isSelected)}
              <div
                class={[
                  "flex h-(--mw-component-channel-row-height) items-center gap-(--mw-component-channel-row-icon-gap) rounded-(--mw-component-channel-row-radius) px-(--mw-component-channel-row-padding-inline)",
                  isSelected
                    ? "bg-(--mw-component-channel-row-background-selected) text-(--mw-component-channel-row-text-selected)"
                    : "text-(--mw-component-channel-row-text-read)",
                ]}
              >
                <span class="relative size-(--mw-component-channel-row-icon-size) shrink-0 overflow-hidden rounded-(--mw-component-channel-row-image-radius)">
                  {@render croppedPicture("")}
                </span>
                <span class="min-w-0 truncate type-channel-name">{name}</span>
              </div>
            {/each}
          </div>
        {/if}
      </section>
    {/if}
  </div>
</ModalDialog>
