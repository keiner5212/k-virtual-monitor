/**
 * Payload structures for every message type.
 *
 * Payloads are little endian, like the Rust implementation. Every decoder
 * validates lengths and ranges, so a malformed payload is rejected at the
 * boundary instead of reaching the application logic.
 */

import { ProtocolError } from "./frame.js";

export const MAX_NAME_LEN = 64;
export const MAX_ERROR_TEXT = 256;

/** Video codecs the protocol can carry, as a bit mask. */
export const Codec = {
    JPEG: 1 << 0,
    H264: 1 << 1,
    H265: 1 << 2,
    VP9: 1 << 3,
    AV1: 1 << 4,
} as const;

/** Codecs in the order the server prefers them. Higher compression first. */
export const CODEC_PREFERENCE: number[] = [Codec.AV1, Codec.H265, Codec.H264, Codec.VP9, Codec.JPEG];

export function codecName(bit: number): string {
    switch (bit) {
        case Codec.JPEG:
            return "jpeg";
        case Codec.H264:
            return "h264";
        case Codec.H265:
            return "h265";
        case Codec.VP9:
            return "vp9";
        case Codec.AV1:
            return "av1";
        default:
            return "unknown";
    }
}

/**
 * Picks the codec both ends support, the most compressed first.
 *
 * The client sends the codecs it can decode, the server answers with the
 * one it will use. Both sides run the same order, so the answer does not
 * depend on who asks.
 */
export function negotiateCodec(clientMask: number, serverMask: number): number | null {
    for (const bit of CODEC_PREFERENCE) {
        if ((clientMask & bit) !== 0 && (serverMask & bit) !== 0) {
            return bit;
        }
    }
    return null;
}

/** Capabilities a paired device may hold. */
export const Caps = {
    /** May receive the screen. */
    VIEW: 1 << 0,
    /** May send mouse, key and touch input. */
    CONTROL: 1 << 1,
} as const;

class Writer {
    private chunks: Uint8Array[] = [];
    private length = 0;

    u8(value: number): void {
        this.push(Uint8Array.of(value & 0xff));
    }

    u16(value: number): void {
        const out = new Uint8Array(2);
        new DataView(out.buffer).setUint16(0, value, true);
        this.push(out);
    }

    i16(value: number): void {
        const out = new Uint8Array(2);
        new DataView(out.buffer).setInt16(0, value, true);
        this.push(out);
    }

    u64(value: number): void {
        const out = new Uint8Array(8);
        new DataView(out.buffer).setBigUint64(0, BigInt(value), true);
        this.push(out);
    }

    bytes(value: Uint8Array): void {
        this.push(value);
    }

    string(value: string): void {
        const raw = new TextEncoder().encode(value);
        const trimmed = raw.subarray(0, MAX_NAME_LEN);
        this.u16(trimmed.length);
        this.push(trimmed);
    }

    private push(chunk: Uint8Array): void {
        this.chunks.push(chunk);
        this.length += chunk.length;
    }

    finish(): Uint8Array {
        const out = new Uint8Array(this.length);
        let offset = 0;
        for (const chunk of this.chunks) {
            out.set(chunk, offset);
            offset += chunk.length;
        }
        return out;
    }
}

class Reader {
    private offset = 0;

    constructor(private readonly bytes: Uint8Array) { }

    private take(n: number): Uint8Array {
        if (this.offset + n > this.bytes.length) {
            throw new ProtocolError(
                "short_payload",
                `payload is ${this.bytes.length - this.offset} bytes, expected ${n}`,
            );
        }
        const slice = this.bytes.subarray(this.offset, this.offset + n);
        this.offset += n;
        return slice;
    }

    u8(): number {
        return this.take(1)[0] as number;
    }

    u16(): number {
        const bytes = this.take(2);
        return new DataView(bytes.buffer, bytes.byteOffset, 2).getUint16(0, true);
    }

    i16(): number {
        const bytes = this.take(2);
        return new DataView(bytes.buffer, bytes.byteOffset, 2).getInt16(0, true);
    }

    u64(): number {
        const bytes = this.take(8);
        return Number(new DataView(bytes.buffer, bytes.byteOffset, 8).getBigUint64(0, true));
    }

    fixed(n: number): Uint8Array {
        return this.take(n).slice();
    }

    string(): string {
        const len = this.u16();
        if (len > MAX_NAME_LEN) {
            throw new ProtocolError("name_too_long", "name is too long");
        }
        return new TextDecoder().decode(this.take(len));
    }
}

export interface Hello {
    clientNonce: Uint8Array;
    deviceId: Uint8Array;
    deviceName: string;
    maxWidth: number;
    maxHeight: number;
    maxFps: number;
    codecMask: number;
    capsRequested: number;
}

export function encodeHello(hello: Hello): Uint8Array {
    const w = new Writer();
    w.bytes(hello.clientNonce);
    w.bytes(hello.deviceId);
    w.string(hello.deviceName);
    w.u16(hello.maxWidth);
    w.u16(hello.maxHeight);
    w.u16(hello.maxFps);
    w.u16(hello.codecMask);
    w.u8(hello.capsRequested);
    return w.finish();
}

export function decodeHello(bytes: Uint8Array): Hello {
    const r = new Reader(bytes);
    return {
        clientNonce: r.fixed(32),
        deviceId: r.fixed(16),
        deviceName: r.string(),
        maxWidth: r.u16(),
        maxHeight: r.u16(),
        maxFps: r.u16(),
        codecMask: r.u16(),
        capsRequested: r.u8(),
    };
}

export interface Challenge {
    serverNonce: Uint8Array;
    serverName: string;
}

export function encodeChallenge(challenge: Challenge): Uint8Array {
    const w = new Writer();
    w.bytes(challenge.serverNonce);
    w.string(challenge.serverName);
    return w.finish();
}

export function decodeChallenge(bytes: Uint8Array): Challenge {
    const r = new Reader(bytes);
    return { serverNonce: r.fixed(32), serverName: r.string() };
}

export function encodeProof(proofBytes: Uint8Array): Uint8Array {
    return proofBytes.slice();
}

export function decodeProof(bytes: Uint8Array): Uint8Array {
    if (bytes.length !== 32) {
        throw new ProtocolError("bad_proof", "proof must be 32 bytes");
    }
    return bytes.slice();
}

export interface ErrorInfo {
    code: number;
    text: string;
}

export function encodeError(error: ErrorInfo): Uint8Array {
    const w = new Writer();
    w.u16(error.code);
    const raw = new TextEncoder().encode(error.text).subarray(0, MAX_ERROR_TEXT);
    w.u8(raw.length);
    w.bytes(raw);
    return w.finish();
}

export function decodeError(bytes: Uint8Array): ErrorInfo {
    const r = new Reader(bytes);
    const code = r.u16();
    const len = r.u8();
    return { code, text: new TextDecoder().decode(r.fixed(len)) };
}

export interface DisplayInfo {
    width: number;
    height: number;
    fps: number;
    codec: number;
    grantedCaps: number;
    serverName: string;
}

export function encodeDisplayInfo(info: DisplayInfo): Uint8Array {
    const w = new Writer();
    w.u16(info.width);
    w.u16(info.height);
    w.u16(info.fps);
    w.u8(info.codec);
    w.u8(info.grantedCaps);
    w.string(info.serverName);
    return w.finish();
}

export function decodeDisplayInfo(bytes: Uint8Array): DisplayInfo {
    const r = new Reader(bytes);
    return {
        width: r.u16(),
        height: r.u16(),
        fps: r.u16(),
        codec: r.u8(),
        grantedCaps: r.u8(),
        serverName: r.string(),
    };
}

export const FRAME_HEADER_LEN = 14;

export interface FrameHeader {
    ptsMs: number;
    keyframe: boolean;
    width: number;
    height: number;
    codec: number;
}

export function encodeFrameHeader(header: FrameHeader): Uint8Array {
    const w = new Writer();
    w.u64(header.ptsMs);
    w.u8(header.keyframe ? 1 : 0);
    w.u16(header.width);
    w.u16(header.height);
    w.u8(header.codec);
    return w.finish();
}

export function decodeFrameHeader(bytes: Uint8Array): FrameHeader {
    const r = new Reader(bytes);
    return {
        ptsMs: r.u64(),
        keyframe: r.u8() !== 0,
        width: r.u16(),
        height: r.u16(),
        codec: r.u8(),
    };
}

export interface FrameAck {
    ackedSeq: number;
    ackedBytes: number;
}

export function encodeFrameAck(ack: FrameAck): Uint8Array {
    const w = new Writer();
    w.u64(ack.ackedSeq);
    w.u64(ack.ackedBytes);
    return w.finish();
}

export function decodeFrameAck(bytes: Uint8Array): FrameAck {
    const r = new Reader(bytes);
    return { ackedSeq: r.u64(), ackedBytes: r.u64() };
}

export interface Mouse {
    x: number;
    y: number;
    buttons: number;
    pressed: boolean;
    wheel: number;
}

export function encodeMouse(mouse: Mouse): Uint8Array {
    const w = new Writer();
    w.u16(mouse.x);
    w.u16(mouse.y);
    w.u8(mouse.buttons);
    w.u8(mouse.pressed ? 1 : 0);
    w.i16(mouse.wheel);
    return w.finish();
}

export function decodeMouse(bytes: Uint8Array): Mouse {
    const r = new Reader(bytes);
    return {
        x: r.u16(),
        y: r.u16(),
        buttons: r.u8(),
        pressed: r.u8() !== 0,
        wheel: r.i16(),
    };
}

export interface Key {
    keycode: number;
    pressed: boolean;
    modifiers: number;
}

export function encodeKey(key: Key): Uint8Array {
    const w = new Writer();
    w.u16(key.keycode);
    w.u8(key.pressed ? 1 : 0);
    w.u8(key.modifiers);
    return w.finish();
}

export function decodeKey(bytes: Uint8Array): Key {
    const r = new Reader(bytes);
    return { keycode: r.u16(), pressed: r.u8() !== 0, modifiers: r.u8() };
}

export interface Touch {
    pointerId: number;
    action: number;
    x: number;
    y: number;
    pressure: number;
}

export function encodeTouch(touch: Touch): Uint8Array {
    const w = new Writer();
    w.u8(touch.pointerId);
    w.u8(touch.action);
    w.u16(touch.x);
    w.u16(touch.y);
    w.u16(touch.pressure);
    return w.finish();
}

export function decodeTouch(bytes: Uint8Array): Touch {
    const r = new Reader(bytes);
    return {
        pointerId: r.u8(),
        action: r.u8(),
        x: r.u16(),
        y: r.u16(),
        pressure: r.u16(),
    };
}

export interface Heartbeat {
    clientTimeMs: number;
}

export function encodeHeartbeat(heartbeat: Heartbeat): Uint8Array {
    const w = new Writer();
    w.u64(heartbeat.clientTimeMs);
    return w.finish();
}

export function decodeHeartbeat(bytes: Uint8Array): Heartbeat {
    return { clientTimeMs: new Reader(bytes).u64() };
}

export interface Close {
    reason: number;
}

export function encodeClose(close: Close): Uint8Array {
    const w = new Writer();
    w.u16(close.reason);
    return w.finish();
}

export function decodeClose(bytes: Uint8Array): Close {
    return { reason: new Reader(bytes).u16() };
}
