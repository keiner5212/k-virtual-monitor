/**
 * The phone opens here.
 *
 * A card is a saved desktop. Opening it connects. If the PC app is closed,
 * the session screen says so instead of retrying on its own.
 */

import { Pressable, StyleSheet, Text, View } from "react-native";

import type { StoredPairing } from "../lib/pairing";
import { colors, radius, space, type } from "../ui";

interface HomeScreenProps {
    pairing: StoredPairing | null;
    onOpen: () => void;
    onAdd: () => void;
}

export default function HomeScreen({ pairing, onOpen, onAdd }: HomeScreenProps) {
    return (
        <View style={styles.root}>
            <Text style={type.title}>Desktops</Text>
            <Text style={[type.body, styles.lead]}>
                Open a computer to use it as a second screen.
            </Text>
            {pairing ? (
                <Pressable
                    accessibilityRole="button"
                    accessibilityLabel={`Connect to ${pairing.serverName}`}
                    accessibilityHint="The PC app has to be open"
                    style={styles.card}
                    onPress={onOpen}
                >
                    <Text style={type.uppercase}>Saved computer</Text>
                    <Text style={type.title}>{pairing.serverName}</Text>
                    <Text style={type.mono}>
                        {pairing.host}:{pairing.port}
                    </Text>
                    <Text style={type.caption}>Connect</Text>
                </Pressable>
            ) : (
                <View style={styles.card}>
                    <Text style={type.title}>No computer yet</Text>
                    <Text style={type.caption}>Add the desktop that shows the pairing code.</Text>
                </View>
            )}
            <Pressable
                accessibilityRole="button"
                accessibilityLabel="Add a desktop"
                style={styles.card}
                onPress={onAdd}
            >
                <Text style={type.title}>Add a desktop</Text>
                <Text style={type.caption}>Scan the code from the PC app, or paste it.</Text>
            </Pressable>
        </View>
    );
}

const styles = StyleSheet.create({
    root: {
        flex: 1,
        padding: space[4],
        gap: space[3],
    },
    lead: {
        color: colors.muted,
    },
    card: {
        backgroundColor: colors.raised,
        borderWidth: 1,
        borderColor: colors.borderStrong,
        borderRadius: radius.panel,
        padding: space[4],
        gap: space[2],
    },
});
