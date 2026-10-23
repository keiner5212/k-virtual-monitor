import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import {
    deriveSessionKeys,
    fromHex,
    open,
    proof,
    randomBytes,
    safetyCode,
    seal,
    toHex,
    type SessionKeys,
} from "../src/crypto.js";
import { clientTranscript, serverTranscript } from "../src/handshake.js";
import { encodeHeader, MessageType } from "../src/frame.js";
import {
    Caps,
    Codec,
    decodeDisplayInfo,
    decodeFrameAck,
    decodeFrameHeader,
    decodeHello,
    decodeKey,
    decodeMouse,
    decodeTouch,
    encodeDisplayInfo,
    encodeError,
    encodeFrameAck,
    encodeFrameHeader,
    encodeHeartbeat,
    encodeHello,
    encodeKey,
    encodeMouse,
    encodeTouch,
    negotiateCodec,
    CODEC_PREFERENCE,
    FRAME_HEADER_LEN,
    type Close,
    type ErrorInfo,
} from "../src/messages.js";
import { decodePairingBlob, encodePairingBlob } from "../src/pairing.js";
import { Session } from "../src/session.js";

const vectorsPath = fileURLToPath(new URL("../../testdata/vectors.json", import.meta.url));
const vectors = JSON.parse(readFileSync(vectorsPath, "utf8")) as {
    psk: string;
    clientNonce: string;
    serverNonce: string;
    deviceId: string;
    clientProof: string;
    serverProof: string;
    sessionKeyC2s: string;
    sessionKeyS2c: string;
    safetyCode: number;
    pairingBlob: string;
    codecPreference: number[];
    sealedFrame: { seq: number; header: string; payload: string; plaintext: string };
    sealedFrame2: { seq: number; header: string; payload: string; plaintext: string };
    sealedFrameServerToClient: { seq: number; header: string; payload: string; plaintext: string };
    payloads: Record<string, string>;
};

function payload(name: string): string {
    const value = vectors.payloads[name];
    if (value === undefined) {
        throw new Error(`the vectors file has no payload named ${name}`);
    }
    return value;
}

function sessionKeys(): SessionKeys {
    return {
        clientToServer: fromHex(vectors.sessionKeyC2s),
        serverToClient: fromHex(vectors.sessionKeyS2c),
    };
}

describe("golden vectors", () => {
    it("derives the same session keys as the Rust implementation", () => {
        const keys = deriveSessionKeys(
            fromHex(vectors.psk),
            fromHex(vectors.clientNonce),
            fromHex(vectors.serverNonce),
        );
        expect(toHex(keys.clientToServer)).toBe(vectors.sessionKeyC2s);
        expect(toHex(keys.serverToClient)).toBe(vectors.sessionKeyS2c);
    });

    it("computes the same handshake proofs as the Rust implementation", () => {
        const psk = fromHex(vectors.psk);
        const clientNonce = fromHex(vectors.clientNonce);
        const serverNonce = fromHex(vectors.serverNonce);
        const deviceId = fromHex(vectors.deviceId);

        const clientProof = proof(
            psk,
            new TextEncoder().encode("kvm-auth-v1|c2s"),
            clientTranscript(clientNonce, serverNonce, deviceId),
        );
        expect(toHex(clientProof)).toBe(vectors.clientProof);

        const serverProof = proof(
            psk,
            new TextEncoder().encode("kvm-auth-v1|s2c"),
            serverTranscript(clientNonce, serverNonce),
        );
        expect(toHex(serverProof)).toBe(vectors.serverProof);
    });

    it("derives the same safety code as the Rust implementation", () => {
        expect(safetyCode(fromHex(vectors.psk))).toBe(vectors.safetyCode);
    });

    it("seals the same frames as the Rust implementation", () => {
        const client = new Session(sessionKeys(), "client");
        for (const name of ["sealedFrame", "sealedFrame2"] as const) {
            const expected = vectors[name];
            const sealed = client.seal(
                MessageType.Frame,
                0,
                fromHex(expected.plaintext),
            );
            expect(toHex(encodeHeader(sealed.header))).toBe(expected.header);
            expect(toHex(sealed.payload)).toBe(expected.payload);
        }

        const server = new Session(sessionKeys(), "server");
        const expected = vectors.sealedFrameServerToClient;
        const sealed = server.seal(MessageType.Frame, 0, fromHex(expected.plaintext));
        expect(toHex(encodeHeader(sealed.header))).toBe(expected.header);
        expect(toHex(sealed.payload)).toBe(expected.payload);
    });

    it("encodes the same payload layouts as the Rust implementation", () => {
        const hello = {
            clientNonce: fromHex(vectors.clientNonce),
            deviceId: fromHex(vectors.deviceId),
            deviceName: "phone",
            maxWidth: 1408,
            maxHeight: 792,
            maxFps: 30,
            codecMask: Codec.H264 | Codec.JPEG,
            capsRequested: Caps.VIEW | Caps.CONTROL,
        };
        expect(toHex(encodeHello(hello))).toBe(payload("hello"));

        const displayInfo = {
            width: 1408,
            height: 792,
            fps: 30,
            codec: 1,
            grantedCaps: Caps.VIEW | Caps.CONTROL,
            serverName: "desktop",
        };
        expect(toHex(encodeDisplayInfo(displayInfo))).toBe(payload("displayInfo"));

        const frameHeader = {
            ptsMs: 1234,
            keyframe: true,
            width: 1408,
            height: 792,
            codec: 1,
        };
        expect(toHex(encodeFrameHeader(frameHeader))).toBe(payload("frameHeader"));
        expect(encodeFrameHeader(frameHeader).length).toBe(FRAME_HEADER_LEN);

        expect(toHex(encodeFrameAck({ ackedSeq: 12, ackedBytes: 4096 }))).toBe(payload("frameAck"));
        expect(
            toHex(encodeMouse({ x: 100, y: 200, buttons: 1, pressed: true, wheel: -1 })),
        ).toBe(payload("mouse"));
        expect(toHex(encodeKey({ keycode: 42, pressed: true, modifiers: 3 }))).toBe(payload("key"));
        expect(
            toHex(encodeTouch({ pointerId: 1, action: 2, x: 100, y: 200, pressure: 1024 })),
        ).toBe(payload("touch"));
        expect(toHex(encodeHeartbeat({ clientTimeMs: 123456789 }))).toBe(payload("heartbeat"));
        expect(toHex(encodeError({ code: 5, text: "boom" } as ErrorInfo))).toBe(payload("error"));
    });

    it("decodes the payloads of the Rust implementation", () => {
        const hello = decodeHello(fromHex(payload("hello")));
        expect(hello.deviceName).toBe("phone");
        expect(hello.maxWidth).toBe(1408);
        expect(hello.codecMask).toBe(Codec.H264 | Codec.JPEG);

        const info = decodeDisplayInfo(fromHex(payload("displayInfo")));
        expect(info.width).toBe(1408);
        expect(info.serverName).toBe("desktop");

        const header = decodeFrameHeader(fromHex(payload("frameHeader")));
        expect(header.ptsMs).toBe(1234);
        expect(header.keyframe).toBe(true);

        expect(decodeFrameAck(fromHex(payload("frameAck")))).toEqual({
            ackedSeq: 12,
            ackedBytes: 4096,
        });
        expect(decodeMouse(fromHex(payload("mouse"))).wheel).toBe(-1);
        expect(decodeKey(fromHex(payload("key"))).keycode).toBe(42);
        expect(decodeTouch(fromHex(payload("touch"))).pressure).toBe(1024);
    });

    it("builds the same pairing blob as the Rust implementation", () => {
        const blob = encodePairingBlob({
            deviceId: fromHex(vectors.deviceId),
            psk: fromHex(vectors.psk),
            serverName: "desktop",
            host: "192.168.1.10",
            port: 28252,
        });
        expect(blob).toBe(vectors.pairingBlob);

        const decoded = decodePairingBlob(vectors.pairingBlob);
        expect(decoded.serverName).toBe("desktop");
        expect(decoded.host).toBe("192.168.1.10");
        expect(decoded.port).toBe(28252);
    });

    it("negotiates the codec in the same order as the Rust implementation", () => {
        expect(CODEC_PREFERENCE).toEqual(vectors.codecPreference);
        expect(negotiateCodec(Codec.H264 | Codec.JPEG, Codec.H264 | Codec.JPEG)).toBe(Codec.H264);
        expect(negotiateCodec(Codec.AV1, Codec.JPEG)).toBeNull();
    });

    it("uses the platform random source", () => {
        const bytes = randomBytes(32);
        expect(bytes.length).toBe(32);
        expect(bytes.some((byte) => byte !== 0)).toBe(true);
    });

    it("seals and opens a payload", () => {
        const key = fromHex("07".repeat(32));
        const aad = new TextEncoder().encode("header");
        const plaintext = new TextEncoder().encode("payload");
        const sealed = seal(key, true, 9, aad, plaintext);
        expect(sealed.length).toBe(plaintext.length + 16);
        const opened = open(key, true, 9, aad, sealed);
        expect(new TextDecoder().decode(opened)).toBe("payload");
    });
});

export type { Close };
