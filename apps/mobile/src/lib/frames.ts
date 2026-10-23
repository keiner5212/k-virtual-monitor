/**
 * The latest frame buffer.
 *
 * The client must never queue frames: a frame that is late is worth less than
 * the one that replaced it. The buffer keeps the newest frame and drops the
 * rest, so a slow decoder or a slow network cannot build a backlog.
 */

export interface Frame {
  /** Milliseconds since the session started, as sent by the server. */
  ptsMs: number;
  width: number;
  height: number;
  codec: number;
  keyframe: boolean;
  /** The encoded picture, still in the codec of the stream. */
  data: Uint8Array;
}

export class LatestFrame {
  private current: Frame | null = null;
  private listeners = new Set<(frame: Frame | null) => void>();

  push(frame: Frame): void {
    this.current = frame;
    for (const listener of this.listeners) {
      listener(frame);
    }
  }

  get(): Frame | null {
    return this.current;
  }

  subscribe(listener: (frame: Frame | null) => void): () => void {
    this.listeners.add(listener);
    listener(this.current);
    return () => {
      this.listeners.delete(listener);
    };
  }
}
