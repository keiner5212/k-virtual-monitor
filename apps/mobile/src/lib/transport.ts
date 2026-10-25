/**
 * The socket of the phone.
 *
 * It is the only part of the session that knows about React Native, so the
 * client can be tested with another socket.
 */

import TcpSocket from "react-native-tcp-socket";

import type { ConnectOptions, Transport, TransportFactory } from "./client";

/** How long the phone waits for the desktop to answer the connection. */
const CONNECT_TIMEOUT_MS = 5000;

/** Opens the socket to the desktop. */
export const phoneTransport: TransportFactory = (options: ConnectOptions) =>
    new Promise<Transport>((resolve, reject) => {
        const socket = TcpSocket.createConnection(
            {
                host: options.host,
                port: options.port,
                connectTimeout: CONNECT_TIMEOUT_MS,
            },
            () => resolve(adapt(socket)),
        );
        socket.on("error", reject);
    });

/** The socket of the phone behind the one of the session. */
function adapt(socket: TcpSocket.Socket): Transport {
    return {
        write: (data) => {
            socket.write(data);
        },
        onData: (listener) => {
            socket.on("data", (data) => listener(toBytes(data)));
        },
        onError: (listener) => {
            socket.on("error", (error) => listener(error));
        },
        onClose: (listener) => {
            socket.on("close", () => listener());
        },
        setNoDelay: (noDelay) => {
            socket.setNoDelay(noDelay);
        },
        destroy: () => {
            socket.destroy();
        },
    };
}

/**
 * The bytes of a chunk.
 *
 * The library hands a buffer over, but a chunk that is not bytes would leave
 * the protocol waiting for data that never comes.
 */
function toBytes(data: Buffer | string): Uint8Array {
    if (data instanceof Uint8Array) {
        return data;
    }
    if (typeof data === "string") {
        const bytes = new Uint8Array(data.length);
        for (let index = 0; index < data.length; index += 1) {
            bytes[index] = data.charCodeAt(index) & 0xff;
        }
        return bytes;
    }
    return new Uint8Array(0);
}
