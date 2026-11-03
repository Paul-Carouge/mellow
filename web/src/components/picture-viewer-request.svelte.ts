import type { Attachment } from "../lib/protocol";

// The picture viewer is rendered once, by the shell, outside any message: inside a message (or a
// search result) its clicks, keys and context menus would reach that row's own handlers, and a
// deleted message would take the open viewer with it.

export interface PictureViewerRequest {
  /** Copied when opened: the viewer keeps them while the message behind it changes. */
  pictures: readonly Attachment[];
  index: number;
  authorName: string;
  sentAtUnixMilliseconds: number;
  /** The picture link that opened the viewer; focus goes back to it when it closes. */
  opener: HTMLElement;
}

const viewer = $state<{ request: PictureViewerRequest | null }>({ request: null });

export function openPictureViewer(request: PictureViewerRequest): void {
  viewer.request = { ...request, pictures: [...request.pictures] };
}

export function closePictureViewer(): void {
  viewer.request = null;
}

export function pictureViewerRequest(): PictureViewerRequest | null {
  return viewer.request;
}
