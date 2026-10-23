# Security model

The server streams a live screen and accepts remote input. A device on the
network that can do that can read everything on the desktop and press any
key, so authentication is not optional.

## The trust boundary

Everything past the socket is controlled by the peer. The server must not
send a frame, must not create a capture session and must not inject an input
event before the peer proved it knows the pairing secret.

There is exactly one gate: the handshake. There is no anonymous mode, no
discovery mode and no debug switch. A release build refuses to start with
authentication disabled.

## Pairing

1. The desktop creates a pairing code. It contains a 32 byte secret and the
   device id the phone will use.
2. The phone scans the code, or types it.
3. Both devices show the same six digit safety code, derived from the
   secret. The user compares them. The code proves that the two devices
   share the same secret; it is not a secret by itself.
4. The desktop stores the phone as a pending device. The first successful
   handshake marks it as paired.

The secret is written to `~/.local/state/kvm/devices.json` with mode `0600`
inside a `0700` directory, written atomically.

## The session

- The handshake is mutual. The client proves it knows the secret, and the
  server proves it too, so the phone cannot be tricked into streaming to a
  stranger.
- Every message after the handshake is sealed with ChaCha20-Poly1305, with
  the header as associated data. The type, the length and the sequence
  number are authenticated, so nothing on the wire can be swapped, resized
  or reordered.
- The session keys are derived with HKDF-SHA256 from the pairing secret and
  the two nonces of the connection. The keys are ephemeral and never written
  to disk.
- Every connection uses fresh nonces, so a reconnect produces fresh keys and
  the nonce space may safely restart.

There is no forward secrecy against a later compromise of the pairing
secret: a secret that leaks lets an attacker derive the keys of a future
session. Adding an ephemeral X25519 exchange to the handshake would fix that
and is a planned change.

## Capabilities

A paired device holds a capability set: `view` to receive the screen and
`control` to send input. The server checks the capability per message
class, not per session. A device that may only watch cannot inject input,
and the attempt is counted as a protocol violation.

## Resource limits

A peer must not be able to degrade the service for the legitimate user.

| Limit | Value |
| --- | --- |
| Payload length, checked before allocating | 4 MiB, and 1 KiB for the handshake |
| Concurrent sessions | 4 |
| Pending handshakes | 8, one per source address in practice |
| Handshake deadline | 10 s |
| Session idle timeout | 120 s |
| Frame window | 8 frames and 2 MB in flight |

The frame window is the one that matters for the stream: a client that stops
acknowledging cannot make the server buffer an unbounded backlog, because
the sender drops frames instead of queueing them.

## What the server never does

- It never binds a privileged port, and it never runs as root. The XDG
  portal session is opened in the user session.
- It never kills other processes to free a port. It fails with a clear error
  instead.
- It never logs key material, pairing secrets, payloads or peer controlled
  text.
- It never serves a frame to a connection that did not complete the
  handshake.

## Checklist for every change

- [ ] No frame, no capture session and no input injection before the
      handshake completes.
- [ ] The authentication tag is verified before the payload is parsed.
- [ ] Every payload length is bounded before anything is allocated.
- [ ] Every blocking read has a deadline.
- [ ] Concurrency is bounded, and the bound is enforced before the work
      starts, not after.
- [ ] No secret is committed, logged or embedded in a binary.
- [ ] `Cargo.lock` and `pnpm-lock.yaml` are committed, and the CI installs
      with `--locked`.
- [ ] `cargo audit` and `pnpm audit` pass.

## Accepted advisories

Two advisories are accepted on purpose, with the date they are reviewed.

| Advisory | Package | Why it is accepted |
| --- | --- | --- |
| GHSA-86w9-cpqp-85rv | node-forge 1.4.0 | It is in the build tooling of the Expo CLI, not in the shipped application, and there is no patched release yet. |
| GHSA-mrhx-c7xc-m3w5 | zod 3.25.76 | Same, in the build tooling of the Expo CLI. The fix is in the 4.x line, which the CLI does not accept. |

They are listed in `pnpm-workspace.yaml` under `auditConfig.ignoreGhsas`, and
the audit fails on anything else. Review on 2027-01-01.

`cargo audit` reports two informational warnings and no vulnerabilities: the
`glib` crate that Tauri pulls in on Linux has an unsoundness advisory that is
fixed only in a version Tauri 2 does not use yet, and `proc-macro-error` is
unmaintained. Neither is used by our code directly.
