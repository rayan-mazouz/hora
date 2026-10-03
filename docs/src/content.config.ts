import { defineCollection } from 'astro:content';
import { glob } from 'astro/loaders';
import { docsLoader } from '@astrojs/starlight/loaders';
import { docsSchema } from '@astrojs/starlight/schema';

export const collections = {
	docs: defineCollection({ loader: docsLoader(), schema: docsSchema() }),
	// The repository's CHANGELOG.md, read in place (never copied) and rendered
	// read-only at /changelog/ by src/pages/changelog.astro.
	changelog: defineCollection({ loader: glob({ pattern: 'CHANGELOG.md', base: '..' }) }),
};
