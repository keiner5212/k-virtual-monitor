import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const version = JSON.parse(readFileSync(join(root, "apps/desktop/package.json"), "utf8")).version;
const changelog = readFileSync(join(root, "CHANGELOG.md"), "utf8");
const section = sectionFor(changelog, version);

if (!section) {
    console.error(`CHANGELOG.md has no section for ${version}. Add ## [${version}] before releasing.`);
    process.exit(1);
}

const notes = section.split("\n").slice(1).join("\n").trim();
if (!notes) {
    console.error(`CHANGELOG.md section for ${version} is empty.`);
    process.exit(1);
}

if (process.argv[2] === "body") {
    process.stdout.write(`${section}\n`);
} else if (process.argv[2] !== undefined && process.argv[2] !== "check") {
    console.error("Use: node scripts/changelog.mjs [check|body]");
    process.exit(1);
}

function sectionFor(markdown, appVersion) {
    const lines = markdown.split(/\r?\n/);
    const heading = new RegExp(`^## \\[${escapeRegExp(appVersion)}\\](?:\\s|$)`);
    const start = lines.findIndex((line) => heading.test(line));
    if (start === -1) {
        return null;
    }
    let end = lines.length;
    for (let index = start + 1; index < lines.length; index += 1) {
        if (lines[index].startsWith("## ")) {
            end = index;
            break;
        }
    }
    return lines.slice(start, end).join("\n").trim();
}

function escapeRegExp(value) {
    return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
