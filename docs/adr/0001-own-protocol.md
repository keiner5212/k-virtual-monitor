# 1. Use our own protocol

Date: 2026-10-23

## Status

Accepted.

## Context

The project started as a Linux server that spoke the spacedesk wire format,
so the official Android app could be used as the viewer. It works, but three
things were wrong with it.

The connection dropped. The server logs showed the client hanging up after a
period of silence, and the client then took about four and a half minutes to
come back. The server had no instrumentation for the close reason, so the
cause was never pinned down.

The frame rate was low. The spacedesk protocol waits for an acknowledgement
before every single frame, so the frame rate cannot exceed one frame per
round trip. On a 70 ms link that is 14 frames per second, and the server
delivered 7 to 14 while the capture produced 27. The protocol itself was the
bottleneck.

The control was not ours. The client was a black box. Every behaviour we
wanted to change, from the reconnect backoff to the acknowledgement policy,
had to be guessed from a decompiled APK.

## Decision

We define the wire protocol ourselves. Both ends are our own software, so
the protocol can be designed for what we need: a window of frames in flight
instead of a stop and wait, a frame acknowledgement on receipt instead of
after decoding, an explicit reconnect policy, and an authenticated handshake
before a single frame is sent.

The spacedesk wire format is not implemented and there is no compatibility
mode.

## Consequences

- The phone app only works with our own server. The official spacedesk app
  cannot connect to it, and our server does not talk to the official app.
- The reverse engineering work of the previous project is kept in
  `legacy/` for reference, but nothing depends on it.
- The protocol has to be implemented twice, in Rust and in TypeScript. The
  golden vectors keep the two implementations honest.
- We own the security model, so authentication is part of the protocol from
  the first version, not something to add later.
