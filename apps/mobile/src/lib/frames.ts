/**
 * The pictures of the session.
 *
 * The client must never queue pictures: a picture that is late is worth less
 * than the one that replaced it. The feed keeps the newest picture and tells
 * every listener about each one, so the decoder takes them all, in order.
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

export class FrameFeed {
    private current: Frame | null = null;
    private listeners = new Set<(frame: Frame) => void>();

    push(frame: Frame): void {
        this.current = frame;
        for (const listener of this.listeners) {
            listener(frame);
        }
    }

    get(): Frame | null {
        return this.current;
    }

    subscribe(listener: (frame: Frame) => void): () => void {
        this.listeners.add(listener);
        const current = this.current;
        if (current) {
            listener(current);
        }
        return () => {
            this.listeners.delete(listener);
        };
    }
}
