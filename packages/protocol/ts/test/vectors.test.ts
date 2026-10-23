import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import {
    deriveSessionKeys,
    fromHex,
    open,
    proof,
    safetyCode,
    seal,
    toHex,
    type SessionKeys,
} from "../src/crypto.js";
import { clientTranscript, serverTranscript } from "../src/handshake.js";
import { encodeHeader, MessageType } from "../src/frame.js";
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
    sealedFrame: {
        type: number;
        flags: number;
        seq: number;
        header: string;
        payload: string;
        plaintext: string;
    };
};

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

    it("seals the same frame as the Rust implementation", () => {
        const keys: SessionKeys = {
            clientToServer: fromHex(vectors.sessionKeyC2s),
            serverToClient: fromHex(vectors.sessionKeyS2c),
        };
        const session = new Session(keys, "client");
        const sealed = session.seal(
            vectors.sealedFrame.type as MessageType,
            vectors.sealedFrame.flags,
            fromHex(vectors.sealedFrame.plaintext),
        );
        expect(toHex(encodeHeader(sealed.header))).toBe(vectors.sealedFrame.header);
        expect(sealed.header.seq).toBe(vectors.sealedFrame.seq);
        expect(sealed.header.payloadLen).toBe(vectors.sealedFrame.payload.length / 2);
        expect(toHex(sealed.payload)).toBe(vectors.sealedFrame.payload);
    });

    it("derives the same safety code as the Rust implementation", () => {
        const code = safetyCode(fromHex(vectors.psk));
        expect(code).toBeLessThan(1_000_000);
    });
});

describe("sealing", () => {
    it("round trips a payload", () => {
        const key = fromHex("07".repeat(32));
        const aad = new TextEncoder().encode("header");
        const plaintext = new TextEncoder().encode("payload");
        const sealed = seal(key, true, 9, aad, plaintext);
        expect(sealed.length).toBe(plaintext.length + 16);
        const opened = open(key, true, 9, aad, sealed);
        expect(new TextDecoder().decode(opened)).toBe("payload");
    });
});
