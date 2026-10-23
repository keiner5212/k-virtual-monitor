import { StatusBar } from "expo-status-bar";
import { useEffect, useState } from "react";
import { Image, StyleSheet, Text, View } from "react-native";

import { SessionClient, type ConnectionState } from "./src/lib/client";
import type { Frame } from "./src/lib/frames";

export default function App() {
    const [state, setState] = useState<ConnectionState>("idle");
    const [detail, setDetail] = useState<string | null>(null);
    const [frame, setFrame] = useState<Frame | null>(null);

    useEffect(() => {
        const client = new SessionClient({
            onState: (next, message) => {
                setState(next);
                setDetail(message ?? null);
            },
            onDisplayInfo: () => undefined,
        });
        const unsubscribe = client.latestFrame.subscribe(setFrame);
        // The pairing flow lands here: read the stored pairing, then connect.
        return () => {
            unsubscribe();
            client.close();
        };
    }, []);

    return (
        <View style={styles.root}>
            <StatusBar style="light" />
            <Text style={styles.state}>State: {state}</Text>
            {detail ? <Text style={styles.detail}>{detail}</Text> : null}
            <View style={styles.screen}>
                {frame ? (
                    <Image
                        style={styles.image}
                        source={{ uri: `data:image/jpeg;base64,${toBase64(frame.data)}` }}
                        resizeMode="contain"
                    />
                ) : (
                    <Text style={styles.detail}>No frame yet</Text>
                )}
            </View>
        </View>
    );
}

function toBase64(bytes: Uint8Array): string {
    let binary = "";
    for (const byte of bytes) {
        binary += String.fromCharCode(byte);
    }
    return globalThis.btoa(binary);
}

const styles = StyleSheet.create({
    root: {
        flex: 1,
        backgroundColor: "#101014",
        padding: 16,
    },
    state: {
        color: "#e6e6ea",
        fontSize: 16,
    },
    detail: {
        color: "#9a9aa4",
        fontSize: 13,
    },
    screen: {
        flex: 1,
        marginTop: 12,
        alignItems: "center",
        justifyContent: "center",
    },
    image: {
        width: "100%",
        height: "100%",
    },
});
