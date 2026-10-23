/**
 * The pairing blob.
 *
 * Pairing happens once per phone. The desktop shows a blob as a QR code and
 * the phone scans it. The blob carries the pairing secret, the device id of
 * the server and the address to connect to. The six digit safety code derived
 * from the same secret is shown on both devices, so the user can confirm they
 * paired with the right peer.
 */

import { fromHex, safetyCode, toHex, PSK_LEN } from "./crypto";
import { ProtocolError } from "./frame";

const PREFIX = "kvm1:";
const VERSION = 1;
const MAX_NAME_LEN = 64;

export interface PairingBlob {
    deviceId: Uint8Array;
    psk: Uint8Array;
    serverName: string;
    host: string;
    port: number;
}

const encoder = new TextEncoder();
const decoder = new TextDecoder();

export function encodePairingBlob(blob: PairingBlob): string {
    const parts: Uint8Array[] = [];
    parts.push(Uint8Array.of(VERSION));
    parts.push(blob.deviceId);
    parts.push(blob.psk);
    const port = new Uint8Array(2);
    new DataView(port.buffer).setUint16(0, blob.port, false);
    parts.push(port);
    for (const value of [blob.serverName, blob.host]) {
        const raw = encoder.encode(value).subarray(0, MAX_NAME_LEN);
        parts.push(Uint8Array.of(raw.length));
        parts.push(raw);
    }
    const total = parts.reduce((sum, part) => sum + part.length, 0);
    const bytes = new Uint8Array(total);
    let offset = 0;
    for (const part of parts) {
        bytes.set(part, offset);
        offset += part.length;
    }
    return `${PREFIX}${toHex(bytes)}`;
}

export function decodePairingBlob(text: string): PairingBlob {
    if (!text.startsWith(PREFIX)) {
        throw new ProtocolError("bad_blob", "pairing blob has a wrong prefix");
    }
    const bytes = fromHex(text.slice(PREFIX.length));
    let offset = 0;
    const version = bytes[offset];
    offset += 1;
    if (version !== VERSION) {
        throw new ProtocolError("bad_blob", `unsupported pairing blob version ${version}`);
    }
    const deviceId = bytes.slice(offset, offset + 16);
    offset += 16;
    const psk = bytes.slice(offset, offset + PSK_LEN);
    offset += PSK_LEN;
    const port = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength).getUint16(offset, false);
    offset += 2;
    const readString = (): string => {
        const len = bytes[offset] as number;
        offset += 1;
        const value = decoder.decode(bytes.subarray(offset, offset + len));
        offset += len;
        return value;
    };
    const serverName = readString();
    const host = readString();
    return { deviceId, psk, serverName, host, port };
}

export function pairingSafetyCode(blob: PairingBlob): number {
    return safetyCode(blob.psk);
}
