/**
 * The pairing, stored on the phone.
 *
 * The pairing secret goes to the secure store, the rest of the pairing
 * (the name and the address of the desktop) goes to the regular storage.
 */

import AsyncStorage from "@react-native-async-storage/async-storage";
import * as SecureStore from "expo-secure-store";

import { decodePairingBlob } from "@kvm/protocol";

const SECRET_KEY = "kvm.pairing.secret";
const META_KEY = "kvm.pairing.meta";

interface PairingMeta {
    deviceId: string;
    serverName: string;
    host: string;
    port: number;
}

export interface StoredPairing extends PairingMeta {
    /** The bytes of the device id, as they travel in the handshake. */
    deviceIdBytes: Uint8Array;
    psk: Uint8Array;
}

export async function savePairing(blob: string): Promise<StoredPairing> {
    const parsed = decodePairingBlob(blob.trim());
    const meta: PairingMeta = {
        deviceId: hex(parsed.deviceId),
        serverName: parsed.serverName,
        host: parsed.host,
        port: parsed.port,
    };
    await SecureStore.setItemAsync(SECRET_KEY, hex(parsed.psk));
    await AsyncStorage.setItem(META_KEY, JSON.stringify(meta));
    return { ...meta, deviceIdBytes: parsed.deviceId, psk: parsed.psk };
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
    return {
        ...meta,
        deviceIdBytes: unhex(meta.deviceId),
        psk: unhex(secret),
    };
}

export async function clearPairing(): Promise<void> {
    await SecureStore.deleteItemAsync(SECRET_KEY);
    await AsyncStorage.removeItem(META_KEY);
}

/**
 * The id of this phone.
 *
 * The desktop generates it during the pairing and puts it in the code, so
 * both ends agree on the id that selects the pairing secret. The phone
 * never invents one.
 */
export function deviceIdBytes(pairing: StoredPairing): Uint8Array {
    return pairing.deviceIdBytes;
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
