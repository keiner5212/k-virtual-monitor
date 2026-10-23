/**
 * Cryptographic primitives for the session.
 *
 * The pairing secret is a 32 byte PSK shared out of band. From it we derive
 * one key per direction. Every message after the handshake is sealed with
 * ChaCha20-Poly1305, using the header as associated data, so the type, the
 * sequence number and the length cannot be altered on the wire.
 *
 * The implementation is pure JavaScript so the same code runs on the phone
 * and in Node. `@noble/ciphers` and `@noble/hashes` are audited libraries.
 */

import { chacha20poly1305 } from "@noble/ciphers/chacha.js";
import { hkdf } from "@noble/hashes/hkdf.js";
import { hmac } from "@noble/hashes/hmac.js";
import { sha256 } from "@noble/hashes/sha2.js";

import { ProtocolError } from "./frame.js";

export const PSK_LEN = 32;
export const NONCE_LEN = 32;
export const KEY_LEN = 32;
export const TAG_LEN = 16;
const AEAD_NONCE_LEN = 12;

export type Psk = Uint8Array;

export interface SessionKeys {
    clientToServer: Uint8Array;
    serverToClient: Uint8Array;
}

const encoder = new TextEncoder();

/**
 * Derives the session keys from the pairing secret and both nonces.
 *
 * The two directions use different `info` labels, so a key is never reused
 * for two purposes. The nonces are per connection, so a reconnect produces a
 * different key pair and the AEAD nonce space may restart.
 */
export function deriveSessionKeys(
    psk: Psk,
    clientNonce: Uint8Array,
    serverNonce: Uint8Array,
): SessionKeys {
    const salt = new Uint8Array(NONCE_LEN * 2);
    salt.set(clientNonce, 0);
    salt.set(serverNonce, NONCE_LEN);
    return {
        clientToServer: hkdf(sha256, psk, salt, encoder.encode("kvm-session-v1|c2s"), KEY_LEN),
        serverToClient: hkdf(sha256, psk, salt, encoder.encode("kvm-session-v1|s2c"), KEY_LEN),
    };
}

/** Computes the handshake proof for one direction. */
export function proof(psk: Psk, label: Uint8Array, transcript: Uint8Array): Uint8Array {
    return hmac(sha256, psk, concat(label, transcript));
}

/** Compares two proofs in constant time. */
export function proofsEqual(a: Uint8Array, b: Uint8Array): boolean {
    if (a.length !== b.length) {
        return false;
    }
    let diff = 0;
    for (let i = 0; i < a.length; i += 1) {
        diff |= (a[i] as number) ^ (b[i] as number);
    }
    return diff === 0;
}

/**
 * Builds the AEAD nonce for a message: one direction bit plus the message
 * sequence number. The sequence number is strictly monotonic inside a
 * session, and every session has fresh keys, so a nonce is never reused.
 */
export function aeadNonce(clientToServer: boolean, seq: number): Uint8Array {
    const nonce = new Uint8Array(AEAD_NONCE_LEN);
    nonce[0] = clientToServer ? 1 : 0;
    const view = new DataView(nonce.buffer);
    view.setBigUint64(1, BigInt(seq), false);
    return nonce;
}

/** Seals a payload. The output is `ciphertext || tag`. */
export function seal(
    key: Uint8Array,
    clientToServer: boolean,
    seq: number,
    aad: Uint8Array,
    plaintext: Uint8Array,
): Uint8Array {
    const cipher = chacha20poly1305(key, aeadNonce(clientToServer, seq), aad);
    return cipher.encrypt(plaintext);
}

/** Opens a sealed payload. */
export function open(
    key: Uint8Array,
    clientToServer: boolean,
    seq: number,
    aad: Uint8Array,
    sealed: Uint8Array,
): Uint8Array {
    if (sealed.length < TAG_LEN) {
        throw new ProtocolError("short_ciphertext", "sealed payload is shorter than the tag");
    }
    const cipher = chacha20poly1305(key, aeadNonce(clientToServer, seq), aad);
    try {
        return cipher.decrypt(sealed);
    } catch {
        throw new ProtocolError("tag_mismatch", "authentication tag mismatch");
    }
}

/** Six digit safety code derived from the pairing secret. */
export function safetyCode(psk: Psk): number {
    const digest = sha256(psk);
    const view = new DataView(digest.buffer, digest.byteOffset, digest.byteLength);
    return view.getUint32(0, false) % 1_000_000;
}

/** Formats a safety code as two groups of three digits. */
export function formatSafetyCode(code: number): string {
    const digits = code.toString().padStart(6, "0");
    return `${digits.slice(0, 3)} ${digits.slice(3)}`;
}

export function concat(...parts: Uint8Array[]): Uint8Array {
    const total = parts.reduce((sum, part) => sum + part.length, 0);
    const out = new Uint8Array(total);
    let offset = 0;
    for (const part of parts) {
        out.set(part, offset);
        offset += part.length;
    }
    return out;
}

/**
 * Random bytes from the platform CSPRNG.
 *
 * The nonces of the handshake and the session keys depend on them being
 * unpredictable, so there is no fallback to `Math.random`.
 */
export function randomBytes(length: number): Uint8Array {
    const source = globalThis.crypto;
    if (!source?.getRandomValues) {
        throw new ProtocolError(
            "no_csprng",
            "the platform has no crypto.getRandomValues, the nonces cannot be generated",
        );
    }
    return source.getRandomValues(new Uint8Array(length));
}

export function toHex(bytes: Uint8Array): string {
    let out = "";
    for (const byte of bytes) {
        out += byte.toString(16).padStart(2, "0");
    }
    return out;
}

export function fromHex(text: string): Uint8Array {
    if (text.length % 2 !== 0) {
        throw new ProtocolError("bad_hex", "hex string has an odd length");
    }
    const out = new Uint8Array(text.length / 2);
    for (let i = 0; i < out.length; i += 1) {
        const pair = text.slice(i * 2, i * 2 + 2);
        if (!/^[0-9a-fA-F]{2}$/.test(pair)) {
            // A mistyped code must be an error, never a zero byte: a pairing
            // secret of zeros is still a secret.
            throw new ProtocolError("bad_hex", `not a hex byte: ${pair}`);
        }
        out[i] = Number.parseInt(pair, 16);
    }
    return out;
}
