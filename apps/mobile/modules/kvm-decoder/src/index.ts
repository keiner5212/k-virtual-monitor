import type { Ref } from "react";
import { requireNativeModule, requireNativeViewManager } from "expo-modules-core";
import type { StyleProp, ViewStyle } from "react-native";

type NativeDecoder = {
    isAvailable?: () => boolean;
};

export type KvmVideoRef = {
    pushFrame: (payload: string, keyframe: boolean, width: number, height: number) => Promise<void>;
};

type VideoProps = {
    ref?: Ref<KvmVideoRef>;
    style?: StyleProp<ViewStyle>;
    onDecodeError?: () => void;
};

export const KvmVideoView = requireNativeViewManager<VideoProps>("KvmDecoder");

export function decoderAvailable(): boolean {
    try {
        const native = requireNativeModule("KvmDecoder") as NativeDecoder;
        return typeof native.isAvailable === "function" ? native.isAvailable() : true;
    } catch {
        return false;
    }
}
