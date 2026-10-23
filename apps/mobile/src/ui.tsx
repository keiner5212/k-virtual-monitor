/**
 * Small building blocks shared by the screens of the phone app.
 */

import { Pressable, StyleSheet, Text, View } from "react-native";

export const colors = {
    surface: "#0b0d12",
    raised: "#12151d",
    border: "#222836",
    ink: "#e8ecf5",
    muted: "#8b93a7",
    accent: "#6d8bff",
    accentSoft: "#2a3557",
    ok: "#4ade80",
    warn: "#fbbf24",
    danger: "#f87171",
};

export function Section({
    title,
    hint,
    children,
}: {
    title: string;
    hint?: string;
    children: React.ReactNode;
}) {
    return (
        <View style={styles.section}>
            <Text style={styles.sectionTitle}>{title}</Text>
            {hint ? <Text style={styles.sectionHint}>{hint}</Text> : null}
            <View style={styles.sectionBody}>{children}</View>
        </View>
    );
}

export function Row({
    label,
    value,
    children,
}: {
    label: string;
    value?: string;
    children?: React.ReactNode;
}) {
    return (
        <View style={styles.row}>
            <Text style={styles.rowLabel}>{label}</Text>
            {value ? <Text style={styles.rowValue}>{value}</Text> : null}
            {children}
        </View>
    );
}

export function Segmented<T extends string | number>({
    options,
    value,
    onChange,
}: {
    options: Array<{ value: T; label: string }>;
    value: T;
    onChange: (value: T) => void;
}) {
    return (
        <View style={styles.segmented}>
            {options.map((option) => {
                const active = option.value === value;
                return (
                    <Pressable
                        key={option.value}
                        accessibilityRole="radio"
                        accessibilityState={{ selected: active }}
                        onPress={() => onChange(option.value)}
                        style={[styles.segment, active && styles.segmentActive]}
                    >
                        <Text style={[styles.segmentLabel, active && styles.segmentLabelActive]}>
                            {option.label}
                        </Text>
                    </Pressable>
                );
            })}
        </View>
    );
}

export function Stepper({
    value,
    options,
    onChange,
    format,
}: {
    value: number;
    options: number[];
    onChange: (value: number) => void;
    format?: (value: number) => string;
}) {
    return (
        <Segmented
            options={options.map((option) => ({ value: option, label: format ? format(option) : String(option) }))}
            value={value}
            onChange={onChange}
        />
    );
}

const styles = StyleSheet.create({
    section: {
        marginBottom: 20,
    },
    sectionTitle: {
        color: colors.ink,
        fontSize: 15,
        fontWeight: "600",
    },
    sectionHint: {
        color: colors.muted,
        fontSize: 12,
        marginTop: 2,
    },
    sectionBody: {
        marginTop: 10,
        borderRadius: 14,
        borderWidth: 1,
        borderColor: colors.border,
        backgroundColor: colors.raised,
        overflow: "hidden",
    },
    row: {
        flexDirection: "row",
        alignItems: "center",
        justifyContent: "space-between",
        paddingHorizontal: 14,
        paddingVertical: 12,
        gap: 12,
    },
    rowLabel: {
        color: colors.ink,
        fontSize: 14,
        flexShrink: 1,
    },
    rowValue: {
        color: colors.muted,
        fontSize: 13,
    },
    segmented: {
        flexDirection: "row",
        flexWrap: "wrap",
        gap: 6,
        padding: 10,
    },
    segment: {
        paddingHorizontal: 12,
        paddingVertical: 7,
        borderRadius: 8,
        borderWidth: 1,
        borderColor: colors.border,
        backgroundColor: colors.surface,
    },
    segmentActive: {
        borderColor: colors.accent,
        backgroundColor: colors.accentSoft,
    },
    segmentLabel: {
        color: colors.muted,
        fontSize: 13,
    },
    segmentLabelActive: {
        color: colors.ink,
        fontWeight: "600",
    },
});
