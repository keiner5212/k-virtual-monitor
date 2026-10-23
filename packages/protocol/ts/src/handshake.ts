/**
 * The pairing handshake.
 *
 * Four messages, two round trips, mutual authentication with a pre-shared
 * key. Both nonces are fresh per connection, so the transcript is unique and
 * a captured handshake cannot be replayed.
 *
 * ```text
 * client -> server   HELLO      client_nonce, device_id, capabilities
 * server -> client   CHALLENGE  server_nonce
 * client -> server   AUTH       HMAC(PSK, "kvm-auth-v1|c2s", transcript)
 * server -> client   AUTH_OK    HMAC(PSK, "kvm-auth-v1|s2c", transcript)
 * ```
 */

import { concat, deriveSessionKeys, proof, proofsEqual, type Psk, type SessionKeys } from "./crypto.js";
import { MessageType, type MessageChannel } from "./frame.js";
import {
    decodeChallenge,
    decodeHello,
    decodeProof,
    encodeChallenge,
    encodeHello,
    encodeProof,
    type Hello,
} from "./messages.js";

const CLIENT_PROOF_LABEL = new TextEncoder().encode("kvm-auth-v1|c2s");
const SERVER_PROOF_LABEL = new TextEncoder().encode("kvm-auth-v1|s2c");

export function clientTranscript(
    clientNonce: Uint8Array,
    serverNonce: Uint8Array,
    deviceId: Uint8Array,
): Uint8Array {
    return concat(clientNonce, serverNonce, deviceId);
}

export function serverTranscript(clientNonce: Uint8Array, serverNonce: Uint8Array): Uint8Array {
    return concat(clientNonce, serverNonce);
}

/** Server side of the handshake. */
export async function serverHandshake(
    channel: MessageChannel,
    psk: Psk,
    serverName: string,
): Promise<{ keys: SessionKeys; hello: Hello }> {
    const first = await channel.next();
    if (first.header.type !== MessageType.Hello) {
        throw new Error(`expected HELLO, got ${first.header.type}`);
    }
    const hello = decodeHello(first.payload);

    const serverNonce = crypto.getRandomValues(new Uint8Array(32));
    const challenge = encodeChallenge({ serverNonce, serverName });
    channel.send(
        { version: 1, type: MessageType.Challenge, flags: 0, seq: 1, payloadLen: challenge.length },
        challenge,
    );

    const second = await channel.next();
    if (second.header.type !== MessageType.Auth) {
        throw new Error(`expected AUTH, got ${second.header.type}`);
    }
    const clientProof = decodeProof(second.payload);
    const expected = proof(
        psk,
        CLIENT_PROOF_LABEL,
        clientTranscript(hello.clientNonce, serverNonce, hello.deviceId),
    );
    if (!proofsEqual(expected, clientProof)) {
        throw new Error("client proof mismatch");
    }

    const keys = deriveSessionKeys(psk, hello.clientNonce, serverNonce);
    const serverProof = proof(psk, SERVER_PROOF_LABEL, serverTranscript(hello.clientNonce, serverNonce));
    const payload = encodeProof(serverProof);
    channel.send(
        { version: 1, type: MessageType.AuthOk, flags: 0, seq: 2, payloadLen: payload.length },
        payload,
    );

    return { keys, hello };
}

/** Client side of the handshake. */
export async function clientHandshake(
    channel: MessageChannel,
    psk: Psk,
    hello: Hello,
): Promise<SessionKeys> {
    const payload = encodeHello(hello);
    channel.send(
        { version: 1, type: MessageType.Hello, flags: 0, seq: 1, payloadLen: payload.length },
        payload,
    );

    const challengeMessage = await channel.next();
    if (challengeMessage.header.type !== MessageType.Challenge) {
        throw new Error(`expected CHALLENGE, got ${challengeMessage.header.type}`);
    }
    const challenge = decodeChallenge(challengeMessage.payload);

    const clientProof = proof(
        psk,
        CLIENT_PROOF_LABEL,
        clientTranscript(hello.clientNonce, challenge.serverNonce, hello.deviceId),
    );
    const authPayload = encodeProof(clientProof);
    channel.send(
        { version: 1, type: MessageType.Auth, flags: 0, seq: 2, payloadLen: authPayload.length },
        authPayload,
    );

    const authOkMessage = await channel.next();
    if (authOkMessage.header.type !== MessageType.AuthOk) {
        throw new Error(`expected AUTH_OK, got ${authOkMessage.header.type}`);
    }
    const serverProof = decodeProof(authOkMessage.payload);
    const expected = proof(
        psk,
        SERVER_PROOF_LABEL,
        serverTranscript(hello.clientNonce, challenge.serverNonce),
    );
    if (!proofsEqual(expected, serverProof)) {
        throw new Error("server proof mismatch");
    }

    return deriveSessionKeys(psk, hello.clientNonce, challenge.serverNonce);
}
