import type { Attachment } from "../lib/protocol";

// The picture viewer is rendered once, by the shell, outside any message: inside a message (or a
// search result) its clicks, keys and context menus would reach that row's own handlers, and a
// deleted message would take the open viewer with it.

export interface PictureViewerRequest {
  /** The pictures and videos; copied when opened: the viewer keeps them while the message behind it changes. */
  media: readonly Attachment[];
  index: number;
  /** Where the video opened at `index` goes on from, when it was playing in the list. */
  videoStartSeconds?: number;
  authorName: string;
  sentAtUnixMilliseconds: number;
  /** The picture link or "Enlarge" button that opened the viewer; focus goes back to it when it closes. */
  opener: HTMLElement;
}

const viewer = $state<{ request: PictureViewerRequest | null }>({ request: null });

export function openPictureViewer(request: PictureViewerRequest): void {
  viewer.request = { ...request, media: [...request.media] };
}

export function closePictureViewer(): void {
  viewer.request = null;
}

export function pictureViewerRequest(): PictureViewerRequest | null {
  return viewer.request;
}
