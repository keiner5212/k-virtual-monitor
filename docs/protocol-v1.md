# The kvm protocol, version 1

This is the contract between the desktop server and the phone client. Both
implementations live in `packages/protocol`, one in Rust and one in
TypeScript, and both are checked against the same golden vectors, so they
cannot drift apart silently.

The protocol is ours. It does not implement the spacedesk wire format.

## Transport

- TCP. One session per connection.
- The port is 28252 by default.
- `TCP_NODELAY` is set on both ends. Nagle would add up to 40 ms to every
  small message, and this protocol is a request and answer protocol.
- There is no discovery in version 1. The phone learns the address from the
  pairing code.

## Framing

Every message is an 18 byte header followed by a payload.

```text
offset size field
0      2    magic, "KM"
2      1    protocol version, 1
3      1    message type
4      2    flags, big endian
6      8    sequence number, big endian
14     4    payload length, big endian
```

The payload length is checked against the limit of the message type before any
buffer is reserved, so a peer cannot make the other end allocate.

Handshake messages carry a plaintext payload. Every other message carries
`ciphertext || tag`, where the tag is the 16 byte Poly1305 tag.

## Message types

| Value | Name | Direction | Purpose |
| --- | --- | --- | --- |
| 0 | HELLO | client to server | greeting |
| 1 | CHALLENGE | server to client | server nonce |
| 2 | AUTH | client to server | proof of the pairing secret |
| 3 | AUTH_OK | server to client | proof of the pairing secret |
| 4 | ERROR | both | error code and text |
| 5 | DISPLAY_INFO | server to client | geometry, codec, granted capabilities |
| 6 | FRAME | server to client | one encoded picture |
| 7 | FRAME_ACK | client to server | acknowledgement of the frames received |
| 8 | INPUT_MOUSE | client to server | pointer |
| 9 | INPUT_KEY | client to server | keyboard |
| 10 | INPUT_TOUCH | client to server | touchscreen |
| 11 | HEARTBEAT | client to server | keep alive and round trip measurement |
| 12 | HEARTBEAT_ACK | server to client | answer to a heartbeat |
| 13 | CLOSE | both | the session is over |

## Handshake

```text
client -> server   HELLO      client_nonce, device_id, device_name, limits
server -> client   CHALLENGE  server_nonce, server_name
client -> server   AUTH       proof
server -> client   AUTH_OK    proof
```

- `client_nonce` and `server_nonce` are 32 random bytes, fresh per
  connection.
- The client proof is
  `HMAC-SHA256(PSK, "kvm-auth-v1|c2s" || client_nonce || server_nonce || device_id)`.
- The server proof is
  `HMAC-SHA256(PSK, "kvm-auth-v1|s2c" || client_nonce || server_nonce)`.
- `PSK` is the 32 byte pairing secret, shared during pairing.

Both nonces are in the transcript, so a captured handshake cannot be replayed
on another connection, and the server proof binds the session to the server
that answered.

The handshake must finish within 10 seconds and before any frame is sent.

## Session keys

```text
salt        = client_nonce || server_nonce
session_c2s = HKDF-SHA256(PSK, salt, "kvm-session-v1|c2s", 32)
session_s2c = HKDF-SHA256(PSK, salt, "kvm-session-v1|s2c", 32)
```

The two directions use different labels, so a key is never used for two
purposes. The salt contains both nonces, so a reconnect produces a different
key pair.

## Sealed messages

Every message after the handshake is sealed with ChaCha20-Poly1305.

```text
nonce = direction_bit(1) || sequence_number(8) || zeros(3)
aad   = the 18 header bytes
```

- The header is the associated data, so the type, the sequence number and the
  length cannot be altered on the wire.
- The sequence number starts at 1 and increases by exactly one per message in
  each direction. A gap closes the session: on a reliable ordered transport a
  gap means the peer is broken or someone is injecting.
- The nonce is unique per key, because the sequence number never repeats
  inside a session and every session has fresh keys.

## Frames

A FRAME payload is a 14 byte frame header followed by the picture.

```text
offset size field
0      8    presentation timestamp in milliseconds
8      1    flags, bit 0 is the key frame flag
9      2    width
11     2    height
13     1    codec
```

The picture is encoded in the codec of the stream. The JPEG codec is a plain
JPEG file. The H.264 codec is a sequence of length prefixed access units,
each one a NAL unit without its start code.

The client acknowledges frames on receipt, before decoding, so a slow decoder
never blocks the server.

## Flow control

The sender keeps a window of frames in flight:

- at most 8 frames,
- at most 2,000,000 bytes.

When the window is full the sender waits for an acknowledgement. This is what
makes the frame rate independent of the round trip time: the previous
implementation waited for an acknowledgement before every single frame, so it
could not go faster than one frame per round trip. On a 70 ms link that is 14
frames per second, which is exactly what the old server managed.

When the window is full and the client stops acknowledging, the sender drops
the new frames instead of queueing them. A late frame is worth less than a
fresh one.

## Pairing

Pairing happens once per phone. The desktop creates a pairing code, which
carries the pairing secret, the device id and the address of the server. The
phone scans the code, or types it.

```text
kvm1:<hex>
```

The hex payload is:

```text
version(1) device_id(16) psk(32) port(2) server_name_len(1) server_name host_len(1) host
```

Both devices show a six digit safety code, derived from the pairing secret.
The code only confirms that the two devices share the same secret, it is not
a secret on its own.

## Limits

| Limit | Value |
| --- | --- |
| Maximum payload | 4 MiB |
| Maximum handshake payload | 1 KiB |
| Maximum input payload | 4 KiB |
| Maximum name length | 64 bytes |
| Maximum sessions | 4 |
| Maximum pending handshakes | 8 |
| Handshake deadline | 10 s |
| Session idle timeout | 120 s |
