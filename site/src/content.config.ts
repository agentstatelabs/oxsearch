import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { defineCollection } from 'astro:content';
import type { Loader } from 'astro/loaders';
import { z } from 'astro/zod';
import { docsLoader } from '@astrojs/starlight/loaders';
import { docsSchema } from '@astrojs/starlight/schema';

/** The repository root. The site is built from `site/`. */
const root = resolve(process.cwd(), '..');

/**
 * Markdown the repository keeps for itself, shown on the site as it is
 * when the site is built, so the two never disagree: the coverage report
 * `cargo run -p xtask -- coverage` writes, and the README's version and
 * license tables, between `<!-- ANCHOR: name -->` and
 * `<!-- ANCHOR_END: name -->`. A part that's missing fails the build.
 */
const PARTS: Record<string, { file: string; region?: string }> = {
	coverage: { file: 'docs/coverage.md' },
	licenses: { file: 'README.md', region: 'licenses' },
	versions: { file: 'README.md', region: 'versions' },
};

function part(file: string, region?: string): string {
	const text = readFileSync(resolve(root, file), 'utf8');
	if (!region) {
		// The page has a title of its own.
		return text.replace(/^# .*\n+/, '');
	}
	const open = `<!-- ANCHOR: ${region} -->`;
	const start = text.indexOf(open);
	const end = text.indexOf(`<!-- ANCHOR_END: ${region} -->`);
	if (start < 0 || end < start) {
		throw new Error(`${file} has no region "${region}" (<!-- ANCHOR: ${region} --> ... <!-- ANCHOR_END: ${region} -->)`);
	}
	return text.slice(start + open.length, end).trim();
}

const repository: Loader = {
	name: 'repository',
	async load({ store, renderMarkdown, watcher }) {
		store.clear();
		for (const [id, { file, region }] of Object.entries(PARTS)) {
			watcher?.add(resolve(root, file));
			const body = part(file, region);
			store.set({ id, data: { file }, body, rendered: await renderMarkdown(body) });
		}
	},
};

export const collections = {
	docs: defineCollection({ loader: docsLoader(), schema: docsSchema() }),
	repository: defineCollection({ loader: repository, schema: z.object({ file: z.string() }) }),
};
