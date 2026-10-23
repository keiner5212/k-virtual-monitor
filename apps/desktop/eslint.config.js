import js from "@eslint/js";
import globals from "globals";
import reactHooks from "eslint-plugin-react-hooks";
import tseslint from "typescript-eslint";

export default tseslint.config(
    { ignores: ["dist", "src-tauri"] },
    {
        extends: [js.configs.recommended, ...tseslint.configs.recommended],
        files: ["**/*.{ts,tsx}"],
        languageOptions: {
            ecmaVersion: 2022,
            globals: globals.browser,
        },
        plugins: {
            "react-hooks": reactHooks,
        },
        rules: {
            ...reactHooks.configs.recommended.rules,
        },
    },
    {
        // The React Bits components are vendored upstream code. They are kept
        // close to the original so they can be refreshed, so the React Compiler
        // rules that flag their internal ref usage do not apply to them.
        files: ["src/components/react-bits/**/*.{ts,tsx}"],
        rules: {
            "react-hooks/refs": "off",
            "react-hooks/set-state-in-effect": "off",
            "no-useless-assignment": "off",
        },
    },
);
