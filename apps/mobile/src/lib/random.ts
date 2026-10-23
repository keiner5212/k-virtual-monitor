/**
 * The random source of the phone.
 *
 * The nonces of the handshake and the pairing secrets depend on the numbers
 * being unpredictable, so the platform CSPRNG is used and never
 * `Math.random`. React Native does not always expose
 * `crypto.getRandomValues`, so it is filled in from the native module of
 * Expo the first time it is missing.
 */

import * as Crypto from "expo-crypto";

type Csprng = { getRandomValues?: (array: Uint8Array) => Uint8Array };

let checked = false;

export function ensureCsprng(): void {
    if (checked) {
        return;
    }
    checked = true;
    const scope = globalThis as unknown as { crypto?: Csprng };
    if (scope.crypto?.getRandomValues) {
        return;
    }
    const fill = (array: Uint8Array): Uint8Array => {
        const bytes = Crypto.getRandomBytes(array.length);
        array.set(bytes);
        return array;
    };
    if (scope.crypto) {
        scope.crypto.getRandomValues = fill;
    } else {
        scope.crypto = { getRandomValues: fill };
    }
}
