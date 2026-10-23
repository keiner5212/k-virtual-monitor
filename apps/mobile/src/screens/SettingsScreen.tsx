/**
 * The settings screen.
 *
 * The compression section shows what the phone asks for and what the server
 * actually chose, so the user can tell whether the stream is compressed or
 * not before the picture appears.
 */

import { Pressable, StyleSheet, Switch, Text, View } from "react-native";

import { codecName } from "@kvm/protocol";

import { type StoredPairing } from "../lib/pairing";
import {
    CODEC_PREFERENCES,
    FRAME_RATES,
    RESOLUTIONS,
    type AppSettings,
} from "../lib/settings";
import { colors, Row, Section, Segmented, Stepper } from "../ui";

interface SettingsScreenProps {
    settings: AppSettings;
    onChange: (settings: AppSettings) => void;
    pairing: StoredPairing | null;
    negotiatedCodec: number | null;
    onUnpair: () => void;
}

const RECONNECT_DELAYS = [500, 1000, 2000, 5000];
const RECONNECT_MAX_DELAYS = [5000, 10000, 30000, 60000];

export default function SettingsScreen({
    settings,
    onChange,
    pairing,
    negotiatedCodec,
    onUnpair,
}: SettingsScreenProps) {
    const detected =
        negotiatedCodec === null
            ? "Not connected"
            : `${codecName(negotiatedCodec).toUpperCase()} (${describeDecoder(negotiatedCodec)})`;

    return (
        <View style={styles.root}>
            <Section
                title="Compression"
                hint="The server picks the most compressed codec both ends support."
            >
                <Segmented
                    options={CODEC_PREFERENCES.map((option) => ({ value: option.value, label: option.label }))}
                    value={settings.codecPreference}
                    onChange={(codecPreference) => onChange({ ...settings, codecPreference })}
                />
                <Row label="Detected" value={detected} />
                <View style={styles.hintBox}>
                    <Text style={styles.hintText}>
                        {CODEC_PREFERENCES.find((option) => option.value === settings.codecPreference)?.hint}
                    </Text>
                </View>
            </Section>

            <Section title="Stream" hint="The largest frame and frame rate the phone accepts.">
                <Stepper
                    value={settings.maxWidth}
                    options={RESOLUTIONS.map((resolution) => resolution.width)}
                    onChange={(maxWidth) => {
                        const resolution = RESOLUTIONS.find((option) => option.width === maxWidth);
                        onChange({
                            ...settings,
                            maxWidth,
                            maxHeight: resolution?.height ?? settings.maxHeight,
                        });
                    }}
                    format={(width) => RESOLUTIONS.find((option) => option.width === width)?.label ?? String(width)}
                />
                <Stepper
                    value={settings.maxFps}
                    options={FRAME_RATES}
                    onChange={(maxFps) => onChange({ ...settings, maxFps })}
                    format={(fps) => `${fps} fps`}
                />
            </Section>

            <Section
                title="Connection"
                hint="How fast the phone comes back when the desktop goes away."
            >
                <Stepper
                    value={settings.reconnectDelayMs}
                    options={RECONNECT_DELAYS}
                    onChange={(reconnectDelayMs) => onChange({ ...settings, reconnectDelayMs })}
                    format={(delay) => `${delay / 1000} s`}
                />
                <Row label="Longest delay" />
                <Stepper
                    value={settings.reconnectMaxDelayMs}
                    options={RECONNECT_MAX_DELAYS}
                    onChange={(reconnectMaxDelayMs) => onChange({ ...settings, reconnectMaxDelayMs })}
                    format={(delay) => `${delay / 1000} s`}
                />
            </Section>

            <Section title="Screen">
                <Row label="Keep the screen on">
                    <Switch
                        value={settings.keepAwake}
                        onValueChange={(keepAwake) => onChange({ ...settings, keepAwake })}
                        trackColor={{ false: colors.border, true: colors.accentSoft }}
                        thumbColor={settings.keepAwake ? colors.accent : colors.muted}
                    />
                </Row>
                <Row label="Show the stream statistics">
                    <Switch
                        value={settings.showStats}
                        onValueChange={(showStats) => onChange({ ...settings, showStats })}
                        trackColor={{ false: colors.border, true: colors.accentSoft }}
                        thumbColor={settings.showStats ? colors.accent : colors.muted}
                    />
                </Row>
            </Section>

            <Section title="Pairing">
                {pairing ? (
                    <>
                        <Row label="Desktop" value={pairing.serverName} />
                        <Row label="Address" value={`${pairing.host}:${pairing.port}`} />
                        <Pressable style={styles.dangerButton} onPress={() => void onUnpair()}>
                            <Text style={styles.dangerLabel}>Forget this desktop</Text>
                        </Pressable>
                    </>
                ) : (
                    <Text style={styles.empty}>No desktop paired yet.</Text>
                )}
            </Section>
        </View>
    );
}

function describeDecoder(codec: number): string {
    switch (codec) {
        case 1:
            return "decoded in software, the most bandwidth";
        case 2:
            return "decoded in hardware, the usual choice";
        case 4:
            return "decoded in hardware, less bandwidth";
        default:
            return "decoded in hardware";
    }
}

const styles = StyleSheet.create({
    root: {
        padding: 16,
        paddingBottom: 40,
    },
    hintBox: {
        paddingHorizontal: 14,
        paddingBottom: 12,
    },
    hintText: {
        color: colors.muted,
        fontSize: 12,
    },
    empty: {
        color: colors.muted,
        fontSize: 13,
        padding: 14,
    },
    dangerButton: {
        margin: 12,
        paddingVertical: 11,
        borderRadius: 10,
        borderWidth: 1,
        borderColor: colors.danger,
        alignItems: "center",
    },
    dangerLabel: {
        color: colors.danger,
        fontSize: 14,
        fontWeight: "600",
    },
});
