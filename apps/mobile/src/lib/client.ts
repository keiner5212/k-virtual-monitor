/**
 * The phone side of the session.
 *
 * It opens a TCP connection, completes the pairing handshake and then keeps
 * the frame window fed: it acknowledges every frame on receipt, before
 * decoding, so the server is never blocked by the decoder.
 *
 * The transport is injected, so the same client runs on the phone and in the
 * tests of the repository.
 */

import {
    clientHandshake,
    decodeDisplayInfo,
    decodeFrameHeader,
    encodeFrameAck,
    encodeHeartbeat,
    MessageChannel,
    MessageType,
    randomBytes,
    Session,
    type ChannelSocket,
    type DisplayInfo,
    type Hello,
    type Message,
} from "@kvm/protocol";

import { FrameFeed, type Frame } from "./frames";

/** The biggest payload the phone accepts, the same limit the server uses. */
const MAX_PAYLOAD = 4 * 1024 * 1024;

/** How often the phone says hello, so the desktop does not close the session. */
const HEARTBEAT_MS = 30000;

/** How long the greeting may take before the attempt counts as a failure. */
const HANDSHAKE_TIMEOUT_MS = 8000;

/** How fast the phone comes back when the desktop goes away. */
const RECONNECT_DELAY_MS = 1000;

/** The longest wait between two attempts. */
const RECONNECT_MAX_DELAY_MS = 2000;

/** Bytes of the frame header that precede the picture. */
const FRAME_HEADER_LEN = 14;

export type ConnectionState = "idle" | "connecting" | "connected" | "error" | "closed";

export interface ClientEvents {
    onState: (state: ConnectionState, detail?: string) => void;
    onDisplayInfo: (info: DisplayInfo) => void;
}

/** The socket the session needs. */
export interface Transport {
    write(data: Uint8Array): void;
    onData(listener: (data: Uint8Array) => void): void;
    onError(listener: (error: Error) => void): void;
    onClose(listener: () => void): void;
    setNoDelay(noDelay: boolean): void;
    destroy(): void;
}

/** Opens the socket to the desktop. */
export type TransportFactory = (options: ConnectOptions) => Promise<Transport>;

export interface ConnectOptions {
    host: string;
    port: number;
    psk: Uint8Array;
    deviceId: Uint8Array;
    deviceName?: string;
    codecMask: number;
    caps: number;
    maxWidth: number;
    maxHeight: number;
    maxFps: number;
    /** Opens the socket. The phone has one and the tests have another. */
    transport: TransportFactory;
}

export class SessionClient {
    private socket: Transport | null = null;
    private channel: MessageChannel | null = null;
    private session: Session | null = null;
    private readonly frames: FrameFeed;
    private ackedBytes = 0;
    private highestSeq = 0;
    private closing = false;
    private heartbeatTimer: ReturnType<typeof setInterval> | null = null;
    private heartbeatSentAt = 0;
    private lastRoundTripMs: number | null = null;

    constructor(
        private readonly events: ClientEvents,
        frames: FrameFeed = new FrameFeed(),
    ) {
        this.frames = frames;
    }

    get latestFrame(): FrameFeed {
        return this.frames;
    }

    async connect(options: ConnectOptions): Promise<void> {
        this.events.onState("connecting");
        const socket = await options.transport(options);
        this.socket = socket;
        try {
            socket.setNoDelay(true);
            const channel = new MessageChannel(adapterOf(socket), MAX_PAYLOAD);
            this.channel = channel;

            const hello: Hello = {
                clientNonce: randomBytes(32),
                deviceId: options.deviceId,
                deviceName: options.deviceName ?? "phone",
                maxWidth: options.maxWidth,
                maxHeight: options.maxHeight,
                maxFps: options.maxFps,
                codecMask: options.codecMask,
                capsRequested: options.caps,
            };

            // The greeting must not hang: a desktop that accepts the connection
            // and then says nothing would leave the phone waiting forever.
            const keys = await within(
                clientHandshake(channel, options.psk, hello),
                HANDSHAKE_TIMEOUT_MS,
                "the desktop did not answer the greeting",
            );
            this.session = new Session(keys, "client");
            this.events.onState("connected");
            this.startHeartbeat();
            void this.pump();
        } catch (cause) {
            this.closeSocket();
            throw cause;
        }
    }

    /** The round trip time of the last heartbeat, in milliseconds. */
    get roundTripMs(): number | null {
        return this.lastRoundTripMs;
    }

    private async pump(): Promise<void> {
        const channel = this.channel;
        const session = this.session;
        if (!channel || !session) {
            return;
        }
        for (; ;) {
            let message: Message;
            try {
                message = await channel.next();
            } catch {
                if (!this.closing) {
                    this.events.onState("closed");
                }
                return;
            }
            let plaintext: Uint8Array;
            try {
                plaintext = session.open(message.header, message.payload);
            } catch {
                this.events.onState("error", "the session was rejected by the server");
                return;
            }
            if (message.header.type === MessageType.DisplayInfo) {
                this.events.onDisplayInfo(decodeDisplayInfo(plaintext));
            } else if (message.header.type === MessageType.Frame) {
                this.onFrame(plaintext, message.header.seq);
            } else if (message.header.type === MessageType.HeartbeatAck) {
                this.lastRoundTripMs = Date.now() - this.heartbeatSentAt;
            }
        }
    }

    /**
     * Keeps the session alive.
     *
     * The server closes a session that stays silent, and a desktop with a
     * static screen sends no frames at all, so the phone says hello every
     * half minute.
     */
    private startHeartbeat(): void {
        this.heartbeatTimer = setInterval(() => {
            const channel = this.channel;
            const session = this.session;
            if (!channel || !session) {
                return;
            }
            this.heartbeatSentAt = Date.now();
            const payload = encodeHeartbeat({ clientTimeMs: this.heartbeatSentAt });
            const sealed = session.seal(MessageType.Heartbeat, 0, payload);
            channel.send(sealed.header, sealed.payload);
        }, HEARTBEAT_MS);
    }

    private onFrame(plaintext: Uint8Array, seq: number): void {
        if (plaintext.length < FRAME_HEADER_LEN) {
            return;
        }
        const header = decodeFrameHeader(plaintext.subarray(0, FRAME_HEADER_LEN));
        const frame: Frame = {
            ptsMs: header.ptsMs,
            width: header.width,
            height: header.height,
            codec: header.codec,
            keyframe: header.keyframe,
            data: plaintext.subarray(FRAME_HEADER_LEN),
        };
        this.frames.push(frame);

        this.ackedBytes += frame.data.length;
        this.highestSeq = seq;
        this.sendAck();
    }

    private sendAck(): void {
        const channel = this.channel;
        const session = this.session;
        if (!channel || !session) {
            return;
        }
        const payload = encodeFrameAck({ ackedSeq: this.highestSeq, ackedBytes: this.ackedBytes });
        const sealed = session.seal(MessageType.FrameAck, 0, payload);
        channel.send(sealed.header, sealed.payload);
    }

    close(): void {
        this.closing = true;
        if (this.heartbeatTimer) {
            clearInterval(this.heartbeatTimer);
            this.heartbeatTimer = null;
        }
        this.closeSocket();
        this.channel = null;
        this.session = null;
        this.events.onState("idle");
    }

    private closeSocket(): void {
        const socket = this.socket;
        this.socket = null;
        socket?.destroy();
    }
}

/**
 * Brings the session back after the desktop drops.
 *
 * The phone keeps trying: the desktop may be restarting, the network may be
 * busy for a moment, and the user should not have to touch anything. The wait
 * between two attempts grows a little and then stays short.
 */
export class ReconnectingClient {
    private client: SessionClient | null = null;
    private timer: ReturnType<typeof setTimeout> | null = null;
    private attempt = 0;
    private stopped = false;
    /** The attempt that is running. An older one is ignored when it reports. */
    private generation = 0;
    private readonly frameBuffer = new FrameFeed();

    constructor(
        private readonly options: ConnectOptions,
        private readonly events: ClientEvents & { onReconnectIn: (seconds: number | null) => void },
    ) { }

    get frames(): FrameFeed {
        return this.frameBuffer;
    }

    start(): void {
        this.stopped = false;
        void this.attemptConnect();
    }

    stop(): void {
        this.stopped = true;
        if (this.timer) {
            clearTimeout(this.timer);
            this.timer = null;
        }
        this.generation += 1;
        this.client?.close();
        this.client = null;
        this.events.onReconnectIn(null);
    }

    private async attemptConnect(): Promise<void> {
        if (this.stopped) {
            return;
        }
        const generation = ++this.generation;
        const client = new SessionClient(
            {
                onState: (state, detail) => this.note(generation, state, detail),
                onDisplayInfo: (info) => {
                    if (generation !== this.generation) {
                        return;
                    }
                    this.attempt = 0;
                    this.events.onDisplayInfo(info);
                },
            },
            this.frameBuffer,
        );
        this.client = client;
        try {
            await client.connect(this.options);
        } catch (cause) {
            if (!this.stopped && generation === this.generation) {
                this.note(generation, "error", cause instanceof Error ? cause.message : String(cause));
            }
        }
    }

    private note(generation: number, state: ConnectionState, detail?: string): void {
        if (this.stopped || generation !== this.generation) {
            return;
        }
        if (state === "connected") {
            this.attempt = 0;
        }
        this.events.onState(state, detail);
        if (state === "error" || state === "closed") {
            this.afterDrop();
        }
    }

    /** The desktop may be gone for a while. The phone keeps trying. */
    private afterDrop(): void {
        if (this.stopped || this.timer) {
            return;
        }
        this.scheduleRetry();
    }

    private scheduleRetry(): void {
        if (this.stopped || this.timer) {
            return;
        }
        const delay = Math.min(
            RECONNECT_DELAY_MS * 2 ** this.attempt,
            RECONNECT_MAX_DELAY_MS,
        );
        this.attempt += 1;
        this.events.onReconnectIn(Math.max(1, Math.round(delay / 1000)));
        this.timer = setTimeout(() => {
            this.timer = null;
            this.events.onReconnectIn(null);
            this.client?.close();
            void this.attemptConnect();
        }, delay);
    }
}

/** The view of the protocol over a socket. */
function adapterOf(socket: Transport): ChannelSocket {
    return {
        write: (data) => {
            socket.write(data);
        },
        on: (event, listener) => {
            if (event === "data") {
                socket.onData((data) => listener(data));
            } else if (event === "error") {
                socket.onError((error) => listener(error));
            } else {
                socket.onClose(() => listener());
            }
        },
    };
}

/**
 * Fails when the work takes longer than the time given.
 */
function within<T>(work: Promise<T>, ms: number, message: string): Promise<T> {
    return new Promise<T>((resolve, reject) => {
        const timer = setTimeout(() => reject(new Error(message)), ms);
        work.then(
            (value) => {
                clearTimeout(timer);
                resolve(value);
            },
            (error) => {
                clearTimeout(timer);
                reject(error);
            },
        );
    });
}
