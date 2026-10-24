/**
 * The pairing, stored on the phone.
 *
 * The pairing secret goes to the secure store, the rest of the pairing
 * (the name and the address of the desktop) goes to the regular storage.
 * The id of the phone goes to the secure store as well: the desktop knows
 * the phone by that id, so it has to survive a new pairing.
 */

import AsyncStorage from "@react-native-async-storage/async-storage";
import * as SecureStore from "expo-secure-store";

import { decodePairingBlob, randomBytes } from "@kvm/protocol";

const SECRET_KEY = "kvm.pairing.secret";
const META_KEY = "kvm.pairing.meta";
const ID_KEY = "kvm.device.id";

interface PairingMeta {
    /** The shape of the saved pairing. An old one is dropped. */
    version: number;
    serverName: string;
    host: string;
    port: number;
}

/** The version of the saved pairing. */
const PAIRING_VERSION = 2;

export interface StoredPairing extends PairingMeta {
    /** The id of this phone, as the desktop knows it. */
    deviceId: string;
    /** The bytes of the device id, as they travel in the handshake. */
    deviceIdBytes: Uint8Array;
    psk: Uint8Array;
}

export async function savePairing(blob: string): Promise<StoredPairing> {
    const parsed = decodePairingBlob(blob.trim());
    const meta: PairingMeta = {
        version: PAIRING_VERSION,
        serverName: parsed.serverName,
        host: parsed.host,
        port: parsed.port,
    };
    await SecureStore.setItemAsync(SECRET_KEY, hex(parsed.psk));
    await AsyncStorage.setItem(META_KEY, JSON.stringify(meta));
    const id = await deviceId();
    return { ...meta, deviceId: hex(id), deviceIdBytes: id, psk: parsed.psk };
}

export async function loadPairing(): Promise<StoredPairing | null> {
    const raw = await AsyncStorage.getItem(META_KEY);
    if (!raw) {
        return null;
    }
    const secret = await SecureStore.getItemAsync(SECRET_KEY);
    if (!secret) {
        return null;
    }
    const meta = JSON.parse(raw) as PairingMeta;
    if (meta.version !== PAIRING_VERSION) {
        // A pairing of an older build: the phone did not own its id yet, so
        // the desktop would not know it. It has to pair again.
        await clearPairing();
        return null;
    }
    const id = await deviceId();
    return { ...meta, deviceId: hex(id), deviceIdBytes: id, psk: unhex(secret) };
}

export async function clearPairing(): Promise<void> {
    await SecureStore.deleteItemAsync(SECRET_KEY);
    await AsyncStorage.removeItem(META_KEY);
}

/**
 * The id of this phone.
 *
 * The phone owns it: it is made once and kept in the secure store, so the
 * desktop always knows the same phone, even when it pairs again.
 */
export async function deviceId(): Promise<Uint8Array> {
    const stored = await SecureStore.getItemAsync(ID_KEY);
    if (stored) {
        return unhex(stored);
    }
    const bytes = randomBytes(16);
    await SecureStore.setItemAsync(ID_KEY, hex(bytes));
    return bytes;
}

function hex(bytes: Uint8Array): string {
    let out = "";
    for (const byte of bytes) {
        out += byte.toString(16).padStart(2, "0");
    }
    return out;
}

function unhex(text: string): Uint8Array {
    const out = new Uint8Array(text.length / 2);
    for (let i = 0; i < out.length; i += 1) {
        out[i] = Number.parseInt(text.slice(i * 2, i * 2 + 2), 16);
    }
    return out;
}
