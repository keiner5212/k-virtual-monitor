import { describe, expect, it } from "vitest";

import {
    decodePairingBlob,
    encodePairingBlob,
    MessageChannel,
    MessageReader,
    MessageType,
    ProtocolError,
    type ChannelSocket,
} from "../src/index.js";
import { encodeHeader } from "../src/frame.js";
import { clientHandshake, serverHandshake } from "../src/handshake.js";
import { SendWindow, Session } from "../src/session.js";
import { deriveSessionKeys, fromHex, toHex } from "../src/crypto.js";
import type { Hello } from "../src/messages.js";

class FakeSocket implements ChannelSocket {
    peer: FakeSocket | null = null;
    private readonly listeners = new Map<string, Array<(payload?: unknown) => void>>();

    write(data: Uint8Array): void {
        const peer = this.peer;
        if (!peer) {
            return;
        }
        const copy = data.slice();
        queueMicrotask(() => {
            peer.emit("data", copy);
        });
    }

    on(event: string, listener: (payload?: unknown) => void): void {
        const existing = this.listeners.get(event) ?? [];
        existing.push(listener);
        this.listeners.set(event, existing);
    }

    emit(event: string, payload?: unknown): void {
        for (const listener of this.listeners.get(event) ?? []) {
            listener(payload);
        }
    }
}

function socketPair(): [FakeSocket, FakeSocket] {
    const a = new FakeSocket();
    const b = new FakeSocket();
    a.peer = b;
    b.peer = a;
    return [a, b];
}

describe("message reader", () => {
    it("splits a stream into messages", () => {
        const reader = new MessageReader();
        const first = new Uint8Array([...encodeHeader({ version: 1, type: MessageType.Hello, flags: 0, seq: 1, payloadLen: 2 }), 1, 2]);
        const second = new Uint8Array([...encodeHeader({ version: 1, type: MessageType.Frame, flags: 0, seq: 2, payloadLen: 1 }), 9]);
        const messages = reader.push(new Uint8Array([...first, ...second]), 1024);
        expect(messages).toHaveLength(2);
        expect(messages[0]?.payload).toEqual(Uint8Array.of(1, 2));
        expect(messages[1]?.payload).toEqual(Uint8Array.of(9));
    });

    it("keeps a partial message until the rest arrives", () => {
        const reader = new MessageReader();
        const message = new Uint8Array([...encodeHeader({ version: 1, type: MessageType.Frame, flags: 0, seq: 1, payloadLen: 4 }), 1, 2]);
        expect(reader.push(message, 1024)).toHaveLength(0);
        const messages = reader.push(Uint8Array.of(3, 4), 1024);
        expect(messages).toHaveLength(1);
        expect(messages[0]?.payload).toEqual(Uint8Array.of(1, 2, 3, 4));
    });

    it("refuses a payload above the limit before allocating it", () => {
        const reader = new MessageReader();
        const header = encodeHeader({ version: 1, type: MessageType.Frame, flags: 0, seq: 1, payloadLen: 5000 });
        expect(() => reader.push(header, 1024)).toThrow(ProtocolError);
    });
});

describe("session", () => {
    const keys = deriveSessionKeys(fromHex("11".repeat(32)), fromHex("22".repeat(32)), fromHex("33".repeat(32)));

    it("opens a message sealed by the peer", () => {
        const client = new Session(keys, "client");
        const server = new Session(keys, "server");
        const sealed = client.seal(MessageType.Frame, 0, new TextEncoder().encode("frame"));
        const opened = server.open(sealed.header, sealed.payload);
        expect(new TextDecoder().decode(opened)).toBe("frame");
    });

    it("rejects a replayed sequence number", () => {
        const client = new Session(keys, "client");
        const server = new Session(keys, "server");
        const sealed = client.seal(MessageType.Frame, 0, new TextEncoder().encode("frame"));
        server.open(sealed.header, sealed.payload);
        expect(() => server.open(sealed.header, sealed.payload)).toThrow(/sequence/);
    });

    it("rejects a tampered payload", () => {
        const client = new Session(keys, "client");
        const server = new Session(keys, "server");
        const sealed = client.seal(MessageType.Frame, 0, new TextEncoder().encode("frame"));
        sealed.payload[0] = (sealed.payload[0] as number) ^ 0xff;
        expect(() => server.open(sealed.header, sealed.payload)).toThrow(ProtocolError);
    });
});

describe("send window", () => {
    it("blocks when full and drains on ack", () => {
        const window = new SendWindow(2, 1000);
        expect(window.canSend(100)).toBe(true);
        window.push(1, 100);
        window.push(2, 100);
        expect(window.canSend(100)).toBe(false);
        window.ack(1);
        expect(window.frames).toBe(1);
        expect(window.bytes).toBe(100);
        expect(window.canSend(100)).toBe(true);
    });

    it("respects the byte cap", () => {
        const window = new SendWindow(8, 1000);
        window.push(1, 900);
        expect(window.canSend(200)).toBe(false);
        expect(window.canSend(50)).toBe(true);
    });
});

describe("pairing blob", () => {
    it("round trips", () => {
        const blob = {
            deviceId: fromHex("09".repeat(16)),
            psk: fromHex("04".repeat(32)),
            serverName: "desktop",
            host: "192.168.1.10",
            port: 28252,
        };
        const encoded = encodePairingBlob(blob);
        expect(encoded.startsWith("kvm1:")).toBe(true);
        const decoded = decodePairingBlob(encoded);
        expect(toHex(decoded.psk)).toBe(toHex(blob.psk));
        expect(decoded.serverName).toBe("desktop");
        expect(decoded.port).toBe(28252);
    });

    it("rejects a wrong prefix", () => {
        expect(() => decodePairingBlob("kvm2:00")).toThrow(ProtocolError);
    });
});

describe("handshake", () => {
    it("authenticates both sides and derives the same keys", async () => {
        const psk = fromHex("77".repeat(32));
        const [clientSocket, serverSocket] = socketPair();
        const clientChannel = new MessageChannel(clientSocket, 1024);
        const serverChannel = new MessageChannel(serverSocket, 1024);

        const hello: Hello = {
            clientNonce: fromHex("aa".repeat(32)),
            deviceId: fromHex("bb".repeat(16)),
            deviceName: "phone",
            maxWidth: 1408,
            maxHeight: 792,
            maxFps: 30,
            codecMask: 0b110,
            capsRequested: 0b11,
        };

        const serverSide = serverHandshake(serverChannel, psk, "desktop");
        const clientSide = clientHandshake(clientChannel, psk, hello);
        const [serverResult, clientKeys] = await Promise.all([serverSide, clientSide]);

        expect(serverResult.hello.deviceName).toBe("phone");
        expect(toHex(clientKeys.clientToServer)).toBe(toHex(serverResult.keys.clientToServer));
    });

    it("fails when the client does not know the pairing secret", async () => {
        const [clientSocket, serverSocket] = socketPair();
        const clientChannel = new MessageChannel(clientSocket, 1024);
        const serverChannel = new MessageChannel(serverSocket, 1024);

        const hello: Hello = {
            clientNonce: fromHex("aa".repeat(32)),
            deviceId: fromHex("bb".repeat(16)),
            deviceName: "phone",
            maxWidth: 1408,
            maxHeight: 792,
            maxFps: 30,
            codecMask: 0b110,
            capsRequested: 0b11,
        };

        const serverSide = serverHandshake(serverChannel, fromHex("01".repeat(32)), "desktop");
        const clientSide = clientHandshake(clientChannel, fromHex("02".repeat(32)), hello);
        await expect(Promise.all([serverSide, clientSide])).rejects.toThrow();
    });
});
