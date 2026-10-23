/**
 * Wire framing.
 *
 * Every message is a fixed 18 byte header followed by a payload:
 *
 * ```text
 * 0      2   magic "KM"
 * 2      1   protocol version
 * 3      1   message type
 * 4      2   flags
 * 6      8   sequence number
 * 14     4   payload length
 * ```
 *
 * Handshake messages carry a plaintext payload. Every other message carries
 * `ciphertext || tag`, produced by the session AEAD over the header as
 * associated data.
 */

export const PROTOCOL_VERSION = 1;
export const HEADER_LEN = 18;
export const MAGIC = 0x4b; // "K"

export const MessageType = {
    Hello: 0,
    Challenge: 1,
    Auth: 2,
    AuthOk: 3,
    Error: 4,
    DisplayInfo: 5,
    Frame: 6,
    FrameAck: 7,
    InputMouse: 8,
    InputKey: 9,
    InputTouch: 10,
    Heartbeat: 11,
    HeartbeatAck: 12,
    Close: 13,
} as const;

export type MessageType = (typeof MessageType)[keyof typeof MessageType];

const MESSAGE_NAMES: Record<number, string> = {
    0: "HELLO",
    1: "CHALLENGE",
    2: "AUTH",
    3: "AUTH_OK",
    4: "ERROR",
    5: "DISPLAY_INFO",
    6: "FRAME",
    7: "FRAME_ACK",
    8: "INPUT_MOUSE",
    9: "INPUT_KEY",
    10: "INPUT_TOUCH",
    11: "HEARTBEAT",
    12: "HEARTBEAT_ACK",
    13: "CLOSE",
};

export function messageTypeName(type: number): string {
    return MESSAGE_NAMES[type] ?? `UNKNOWN(${type})`;
}

export interface Header {
    version: number;
    type: MessageType;
    flags: number;
    seq: number;
    payloadLen: number;
}

export interface Message {
    header: Header;
    payload: Uint8Array;
}

export class ProtocolError extends Error {
    readonly code: string;

    constructor(code: string, message: string) {
        super(message);
        this.name = "ProtocolError";
        this.code = code;
    }
}

export function encodeHeader(header: Header): Uint8Array {
    const out = new Uint8Array(HEADER_LEN);
    const view = new DataView(out.buffer);
    out[0] = MAGIC;
    out[1] = 0x4d; // "M"
    out[2] = header.version;
    out[3] = header.type;
    view.setUint16(4, header.flags, false);
    view.setBigUint64(6, BigInt(header.seq), false);
    view.setUint32(14, header.payloadLen, false);
    return out;
}

export function decodeHeader(bytes: Uint8Array): Header {
    if (bytes.length < HEADER_LEN) {
        throw new ProtocolError("short_header", "header is shorter than 18 bytes");
    }
    if (bytes[0] !== MAGIC || bytes[1] !== 0x4d) {
        throw new ProtocolError("bad_magic", `bad magic: ${bytes[0]}, ${bytes[1]}`);
    }
    const version = bytes[2] ?? 0;
    if (version !== PROTOCOL_VERSION) {
        throw new ProtocolError("bad_version", `unsupported protocol version ${version}`);
    }
    const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    return {
        version,
        type: bytes[3] as MessageType,
        flags: view.getUint16(4, false),
        seq: Number(view.getBigUint64(6, false)),
        payloadLen: view.getUint32(14, false),
    };
}

/**
 * Splits a byte stream into messages.
 *
 * The payload length is checked against the limit before the payload is
 * copied out, so a hostile peer cannot make the client allocate.
 */
export class MessageReader {
    private buffer = new Uint8Array(0);

    push(chunk: Uint8Array, maxPayload: number): Message[] {
        const merged = new Uint8Array(this.buffer.length + chunk.length);
        merged.set(this.buffer, 0);
        merged.set(chunk, this.buffer.length);
        this.buffer = merged;

        const messages: Message[] = [];
        let offset = 0;
        for (; ;) {
            if (this.buffer.length - offset < HEADER_LEN) {
                break;
            }
            const header = decodeHeader(this.buffer.subarray(offset, offset + HEADER_LEN));
            if (header.payloadLen > maxPayload) {
                throw new ProtocolError(
                    "payload_too_large",
                    `payload of ${header.payloadLen} bytes exceeds the limit of ${maxPayload}`,
                );
            }
            const total = HEADER_LEN + header.payloadLen;
            if (this.buffer.length - offset < total) {
                break;
            }
            const payload = this.buffer.slice(offset + HEADER_LEN, offset + total);
            messages.push({ header, payload });
            offset += total;
        }
        this.buffer = this.buffer.slice(offset);
        return messages;
    }

    get pending(): number {
        return this.buffer.length;
    }
}

/** Listener signature shared by node:net and by a React Native TCP socket. */
export type SocketListener = (payload?: unknown) => void;

/** The part of a socket this package needs. */
export interface ChannelSocket {
    write(data: Uint8Array): void;
    on(event: string, listener: SocketListener): void;
}

/**
 * A message oriented view over a socket.
 *
 * Incoming messages are queued, so the handshake and the session can await
 * them one at a time.
 */
export class MessageChannel {
    private readonly reader = new MessageReader();
    private readonly queue: Message[] = [];
    private readonly waiters: Array<(message: Message) => void> = [];
    private failure: ProtocolError | null = null;

    constructor(
        private readonly socket: ChannelSocket,
        private readonly maxPayload: number,
    ) {
        socket.on("data", (payload) => {
            const chunk = payload as Uint8Array;
            let messages: Message[];
            try {
                messages = this.reader.push(chunk, this.maxPayload);
            } catch (error) {
                this.fail(error instanceof ProtocolError ? error : new ProtocolError("parse", String(error)));
                return;
            }
            for (const message of messages) {
                this.deliver(message);
            }
        });
        socket.on("error", (payload) => {
            const message = payload instanceof Error ? payload.message : "socket error";
            this.fail(new ProtocolError("socket", message));
        });
        socket.on("close", () => {
            this.fail(new ProtocolError("closed", "socket closed"));
        });
    }

    private deliver(message: Message): void {
        const waiter = this.waiters.shift();
        if (waiter) {
            waiter(message);
        } else {
            this.queue.push(message);
        }
    }

    private fail(error: ProtocolError): void {
        this.failure = error;
        while (this.waiters.length > 0) {
            const waiter = this.waiters.shift();
            waiter?.(null as unknown as Message);
        }
    }

    /** Resolves with the next message, or rejects when the socket fails. */
    next(): Promise<Message> {
        const queued = this.queue.shift();
        if (queued) {
            return Promise.resolve(queued);
        }
        if (this.failure) {
            return Promise.reject(this.failure);
        }
        return new Promise<Message>((resolve, reject) => {
            this.waiters.push((message) => {
                if (message) {
                    resolve(message);
                } else {
                    reject(this.failure ?? new ProtocolError("closed", "socket closed"));
                }
            });
        });
    }

    send(header: Header, payload: Uint8Array): void {
        const out = new Uint8Array(HEADER_LEN + payload.length);
        out.set(encodeHeader(header), 0);
        out.set(payload, HEADER_LEN);
        this.socket.write(out);
    }
}
