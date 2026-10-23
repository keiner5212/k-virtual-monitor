/**
 * The authenticated session.
 *
 * After the handshake every message is sealed with ChaCha20-Poly1305. The
 * header is the associated data, so the type, the sequence number and the
 * length are authenticated together with the payload.
 *
 * The sequence number starts at 1 and increases by exactly one per message
 * in each direction. A gap means the peer is broken or an attacker is
 * injecting, so the session is closed instead of resynchronising.
 */

import { open, seal, type SessionKeys, TAG_LEN } from "./crypto";
import { encodeHeader, ProtocolError, type Header, type MessageType } from "./frame";

export const MAX_WINDOW_FRAMES = 8;
export const MAX_WINDOW_BYTES = 2_000_000;

export class Session {
    private readonly sendKey: Uint8Array;
    private readonly recvKey: Uint8Array;
    private readonly outboundClientToServer: boolean;
    private readonly inboundClientToServer: boolean;
    private nextSendSeq = 1;
    private nextRecvSeq = 1;

    constructor(keys: SessionKeys, role: "client" | "server") {
        if (role === "client") {
            this.sendKey = keys.clientToServer;
            this.recvKey = keys.serverToClient;
            this.outboundClientToServer = true;
            this.inboundClientToServer = false;
        } else {
            this.sendKey = keys.serverToClient;
            this.recvKey = keys.clientToServer;
            this.outboundClientToServer = false;
            this.inboundClientToServer = true;
        }
    }

    /** Seals a message. The returned payload is `ciphertext || tag`. */
    seal(type: MessageType, flags: number, plaintext: Uint8Array): { header: Header; payload: Uint8Array } {
        const seq = this.nextSendSeq;
        const header: Header = {
            version: 1,
            type,
            flags,
            seq,
            payloadLen: plaintext.length + TAG_LEN,
        };
        const payload = seal(
            this.sendKey,
            this.outboundClientToServer,
            seq,
            encodeHeader(header),
            plaintext,
        );
        this.nextSendSeq += 1;
        return { header, payload };
    }

    /** Opens a sealed message and checks the sequence number. */
    open(header: Header, payload: Uint8Array): Uint8Array {
        if (header.seq !== this.nextRecvSeq) {
            throw new ProtocolError(
                "bad_sequence",
                `sequence ${header.seq} is not the expected ${this.nextRecvSeq}`,
            );
        }
        const plaintext = open(
            this.recvKey,
            this.inboundClientToServer,
            header.seq,
            encodeHeader(header),
            payload,
        );
        this.nextRecvSeq += 1;
        return plaintext;
    }

    get sendSeq(): number {
        return this.nextSendSeq;
    }
}

/**
 * Flow control window for the frame stream.
 *
 * The window is the number of frames and the number of bytes the sender may
 * have outstanding. It is large enough that the frame rate is not bounded by
 * the round trip time, and small enough that a stalled client cannot make the
 * server buffer an unbounded backlog.
 */
export class SendWindow {
    private readonly inFlight = new Map<number, number>();
    private inFlightBytes = 0;

    constructor(
        private readonly maxFrames: number = MAX_WINDOW_FRAMES,
        private readonly maxBytes: number = MAX_WINDOW_BYTES,
    ) { }

    get frames(): number {
        return this.inFlight.size;
    }

    get bytes(): number {
        return this.inFlightBytes;
    }

    /** True when a frame of `bytes` fits in the window. */
    canSend(bytes: number): boolean {
        return this.inFlight.size < this.maxFrames && this.inFlightBytes + bytes <= this.maxBytes;
    }

    /** Records a frame as sent. */
    push(seq: number, bytes: number): void {
        this.inFlight.set(seq, bytes);
        this.inFlightBytes += bytes;
    }

    /** Releases every frame up to and including `ackedSeq`. */
    ack(ackedSeq: number): number {
        let released = 0;
        for (const [seq, bytes] of [...this.inFlight.entries()].sort((a, b) => a[0] - b[0])) {
            if (seq > ackedSeq) {
                break;
            }
            this.inFlight.delete(seq);
            this.inFlightBytes -= bytes;
            released += bytes;
        }
        return released;
    }

    /** Drops the whole backlog, used when the link is considered dead. */
    clear(): void {
        this.inFlight.clear();
        this.inFlightBytes = 0;
    }
}
