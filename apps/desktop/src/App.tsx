import { useCallback, useEffect, useState } from "react";

import { createPairing, pairedDevices, serverStatus, type PairingInfo, type ServerStatus } from "./lib/api";

export default function App() {
    const [status, setStatus] = useState<ServerStatus | null>(null);
    const [pairing, setPairing] = useState<PairingInfo | null>(null);
    const [devices, setDevices] = useState<string[]>([]);
    const [error, setError] = useState<string | null>(null);

    const refresh = useCallback(async () => {
        try {
            setStatus(await serverStatus());
            setDevices(await pairedDevices());
            setError(null);
        } catch (cause) {
            setError(cause instanceof Error ? cause.message : String(cause));
        }
    }, []);

    useEffect(() => {
        const initial = setTimeout(() => void refresh(), 0);
        const timer = setInterval(() => void refresh(), 2000);
        return () => {
            clearTimeout(initial);
            clearInterval(timer);
        };
    }, [refresh]);

    const pair = useCallback(async () => {
        try {
            setPairing(await createPairing());
        } catch (cause) {
            setError(cause instanceof Error ? cause.message : String(cause));
        }
    }, []);

    return (
        <main className="app">
            <header>
                <h1>k-virtual-monitor</h1>
                <p className="subtitle">Desktop server</p>
            </header>

            {error ? <p className="error">{error}</p> : null}

            <section>
                <h2>Server</h2>
                <dl>
                    <dt>State</dt>
                    <dd>{status?.running ? "running" : "stopped"}</dd>
                    <dt>Address</dt>
                    <dd>{status ? `${status.address}:${status.port}` : "-"}</dd>
                    <dt>Sessions</dt>
                    <dd>{status?.sessions ?? 0}</dd>
                    <dt>Paired devices</dt>
                    <dd>{status?.pairedDevices ?? devices.length}</dd>
                </dl>
            </section>

            <section>
                <h2>Pair a phone</h2>
                <p>
                    The phone scans the pairing code, then both devices show the same safety code. The
                    safety code only confirms that the two devices share the same secret.
                </p>
                <button type="button" onClick={() => void pair()}>
                    Create pairing code
                </button>
                {pairing ? (
                    <div className="pairing">
                        <p className="blob">{pairing.blob}</p>
                        <p className="safety">
                            Safety code <strong>{pairing.safetyCode}</strong>
                        </p>
                    </div>
                ) : null}
            </section>
        </main>
    );
}
