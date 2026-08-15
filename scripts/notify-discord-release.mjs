#!/usr/bin/env node
/**
 * Post a short ShuvGrok release announcement to a Discord webhook.
 *
 * The webhook URL is only ever read from the environment
 * (DISCORD_RELEASE_WEBHOOK_URL) — never embedded here.
 *
 * Missing webhook is not an error: notifyDiscordRelease() returns
 * { skipped: true } so a release driver can call it unconditionally.
 */

import { pathToFileURL } from "node:url";

const DEFAULT_REPO = "shuv1337/grok-build";
const DEFAULT_PACKAGE = "@shuv1337/shuvgrok";

export function normalizeVersion(version) {
	const value = String(version ?? "").trim();
	if (!value) {
		throw new Error("version is required");
	}
	const tag = value.startsWith("v") ? value : `v${value}`;
	const semver = tag.slice(1);
	if (!/^\d+\.\d+\.\d+$/.test(semver)) {
		throw new Error(`invalid version: ${version}`);
	}
	return { semver, tag };
}

export function buildDiscordReleaseMessage({
	version,
	repo = DEFAULT_REPO,
	packageName = DEFAULT_PACKAGE,
} = {}) {
	const { semver, tag } = normalizeVersion(version);
	const npmUrl = `https://www.npmjs.com/package/${packageName}/v/${semver}`;
	const tagUrl = `https://github.com/${repo}/tree/${tag}`;

	return [`**ShuvGrok ${tag}** is out`, `npm: ${npmUrl}`, `tag: ${tagUrl}`].join("\n");
}

export async function notifyDiscordRelease({
	version,
	webhookUrl = process.env.DISCORD_RELEASE_WEBHOOK_URL,
	repo = process.env.DISCORD_RELEASE_REPO || DEFAULT_REPO,
	packageName = process.env.DISCORD_RELEASE_PACKAGE || DEFAULT_PACKAGE,
	fetchImpl = globalThis.fetch,
} = {}) {
	if (!webhookUrl) {
		return { skipped: true, reason: "DISCORD_RELEASE_WEBHOOK_URL is not set" };
	}
	if (typeof fetchImpl !== "function") {
		throw new Error("fetch is not available");
	}

	const content = buildDiscordReleaseMessage({ version, repo, packageName });
	const response = await fetchImpl(webhookUrl, {
		method: "POST",
		headers: { "Content-Type": "application/json" },
		body: JSON.stringify({ content }),
	});

	if (!response.ok) {
		const body = typeof response.text === "function" ? await response.text() : "";
		throw new Error(`Discord webhook failed: ${response.status}${body ? ` ${body}` : ""}`);
	}

	return { skipped: false, content };
}

async function main(argv) {
	const version = argv[0];
	if (!version || version === "--help" || version === "-h") {
		console.log(`Usage: node scripts/notify-discord-release.mjs <x.y.z|vX.Y.Z>

Posts a short Discord release announcement via DISCORD_RELEASE_WEBHOOK_URL.

Environment:
  DISCORD_RELEASE_WEBHOOK_URL   required Discord incoming webhook URL
  DISCORD_RELEASE_REPO          optional owner/repo (default: ${DEFAULT_REPO})
  DISCORD_RELEASE_PACKAGE       optional npm package (default: ${DEFAULT_PACKAGE})
`);
		process.exit(version ? 0 : 1);
	}

	const result = await notifyDiscordRelease({ version });
	if (result.skipped) {
		console.error(`Skipping Discord announcement: ${result.reason}`);
		process.exit(1);
	}

	console.log("Posted Discord release announcement");
	console.log(result.content);
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
	main(process.argv.slice(2)).catch((error) => {
		console.error(error instanceof Error ? error.message : error);
		process.exit(1);
	});
}
