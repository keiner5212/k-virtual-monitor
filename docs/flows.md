# The flows

The two moments that matter: pairing a phone, and a live session.

## Pairing

Pairing happens once per phone. The desktop creates a code, the phone scans
it, and the user confirms with a six digit safety code that both screens show
the same thing.

```mermaid
sequenceDiagram
    autonumber
    participant User as User
    participant Desktop as Linux Desktop App
    participant Store as Device Store File
    participant Phone as Android Phone
    participant SecureStore as Secure Store
    User->>Desktop: Press Create pairing code
    Desktop->>Desktop: Generate 16 byte device id and 32 byte pairing secret with CSPRNG
    Desktop->>Store: Store device as pending with mode 0600
    Desktop->>Desktop: Build pairing blob with prefix kvm1 device id secret port server name host address
    Desktop-->>User: Show pairing blob as QR code and six digit safety code
    User->>Phone: Scan QR code with camera
    Phone->>Phone: Decode pairing blob
    Phone->>SecureStore: Store secret
    Phone->>Phone: Compute six digit safety code from secret
    Phone-->>User: Show safety code
    User->>User: Compare safety codes on both screens
    User->>Phone: Confirm codes match
    Phone->>Desktop: Open TCP connection and send HELLO with fresh 32 byte nonce device id and decodable codecs
    Desktop->>Store: Look up device id
    alt device revoked
        Desktop-->>Phone: Refuse device
    else device not revoked
        Desktop-->>Phone: Send CHALLENGE with fresh 32 byte nonce
        Phone->>Desktop: Send AUTH with HMAC-SHA256 proof over both nonces and device id
        Desktop->>Desktop: Verify proof in constant time
        Desktop-->>Phone: Send AUTH_OK with own proof
        Desktop->>Desktop: Derive session keys with HKDF-SHA256 using secret both nonces and direction-specific labels
        Phone->>Phone: Derive session keys with HKDF-SHA256 using secret both nonces and direction-specific labels
        Desktop->>Store: Mark device as paired
        Note over Desktop,Phone: Session starts
    end
```

## A live session

The frame path, the window that keeps the sender busy, and the input path in
the other direction.

```mermaid
sequenceDiagram
    title Live session frame and input paths
    participant Desktop as Desktop Server
    participant Phone as Phone
    Desktop->>Desktop: Capture screen
    Desktop->>Desktop: Encode picture in negotiated codec
    Desktop->>Desktop: Check send window
    alt fewer than 8 frames and fewer than 2 MB in flight
        Desktop->>Desktop: Seal frame with ChaCha20-Poly1305
        Note right of Desktop: Header as associated data
        Note right of Desktop: Nonce is direction bit plus message sequence number
        Desktop->>Phone: Send sealed bytes on socket
    else window full
        Desktop->>Desktop: Wait for acknowledgement
    end
    Phone->>Phone: Read bytes from socket
    Phone->>Phone: Verify authentication tag
    Phone->>Phone: Check sequence number
    Phone->>Phone: Parse payload
    Phone->>Phone: Decode picture
    Phone->>Phone: Show picture
    Phone-->>Desktop: Acknowledge frame on receipt
    Note over Desktop,Phone: Acknowledgement precedes decode
    Desktop->>Desktop: Release send window
    loop every 30 seconds
        Phone->>Desktop: Heartbeat
        Desktop-->>Phone: Heartbeat response
        Phone->>Phone: Measure round trip
    end
    Phone->>Phone: Seal touch, mouse or key event
    Phone->>Desktop: Send sealed input event
    Desktop->>Desktop: Verify authentication tag
    Desktop->>Desktop: Check control capability of device
    Desktop->>Desktop: Inject event through XDG remote desktop portal
    alt client acknowledges within three measured round trips
        Desktop->>Desktop: Continue session
    else client stops acknowledging
        Desktop->>Desktop: Close session instead of growing backlog
    end
```

## The parts

```mermaid
flowchart TD
    subgraph Shared [Shared Protocol]
        RustCrate["Rust crate"]
        TSPackage["TypeScript package"]
        Golden["Golden vectors file"]
        Spec["Wire protocol specification"]
        RustCrate -->|checked against| Golden
        TSPackage -->|checked against| Golden
        Spec --> RustCrate
        Spec --> TSPackage
    end

    subgraph Desktop [Desktop App]
        Tauri["Tauri v2 application"]
        RustBackend["Rust backend"]
        TCPServer["TCP server"]
        Handshake["Handshake state machine"]
        Session["Session"]
        Sealed["Sealed channel"]
        Flow["Flow control window"]
        DeviceStore["Device store"]
        FrameSource["Frame source"]
        React["React interface with Vite"]
        ServerState["Server state"]
        PairingCode["Pairing code"]
        SafetyCode["Safety code"]
        Settings["Settings"]
        Tauri --> RustBackend
        RustBackend --> TCPServer
        RustBackend --> Handshake
        RustBackend --> Session
        Session --> Sealed
        Session --> Flow
        RustBackend --> DeviceStore
        RustBackend --> FrameSource
        Tauri --> React
        React --> ServerState
        React --> PairingCode
        React --> SafetyCode
        React --> Settings
        RustBackend -->|uses Rust implementation| RustCrate
    end

    subgraph Phone [Phone App]
        Expo["Expo React Native application"]
        ProtoClient["Protocol client"]
        Pairing["Pairing screen with camera"]
        Viewer["Viewer"]
        PhoneSettings["Settings screen"]
        Compression["Compression detection"]
        Reconnect["Reconnect policy"]
        Secret["Secret in secure store"]
        Expo --> ProtoClient
        Expo --> Pairing
        Expo --> Viewer
        Expo --> PhoneSettings
        PhoneSettings --> Compression
        PhoneSettings --> Reconnect
        ProtoClient --> Secret
        ProtoClient -->|uses TypeScript implementation| TSPackage
    end

    TCPServer -->|talks over TCP using shared protocol| ProtoClient
    Golden -->|ties implementations together| RustCrate
    Golden -->|ties implementations together| TSPackage
```
