// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';
import { horaDark, horaLight } from './src/code-themes.mjs';

const site = 'https://uplg.github.io';
const base = '/hora';

// Deployed as a GitHub Pages project site: https://uplg.github.io/hora/
export default defineConfig({
	site,
	base,
	integrations: [
		starlight({
			title: 'Hora',
			description:
				'Hora keeps the hours, so you can sleep. A small self-hosted uptime monitor: one binary, a calm status page, and alerts only for real outages.',
			logo: {
				light: './src/assets/wordmark-light.svg',
				dark: './src/assets/wordmark-dark.svg',
				alt: 'Hora',
				replacesTitle: true,
			},
			favicon: '/favicon.svg',
			head: [
				{ tag: 'meta', attrs: { property: 'og:image', content: `${site}${base}/og.png` } },
				{ tag: 'meta', attrs: { property: 'og:image:width', content: '1200' } },
				{ tag: 'meta', attrs: { property: 'og:image:height', content: '630' } },
				{
					tag: 'meta',
					attrs: {
						property: 'og:image:alt',
						content: 'Hora, the night-watch owl asleep inside the twelve hours: Hora keeps the hours, so you can sleep.',
					},
				},
				{ tag: 'meta', attrs: { name: 'twitter:card', content: 'summary_large_image' } },
				{ tag: 'meta', attrs: { name: 'theme-color', content: '#0E5E68' } },
				// The two faces read first on every page; the others load on demand.
				{
					tag: 'link',
					attrs: { rel: 'preload', href: `${base}/fonts/CalSansVF.woff2`, as: 'font', type: 'font/woff2', crossorigin: '' },
				},
				{
					tag: 'link',
					attrs: { rel: 'preload', href: `${base}/fonts/Fraunces-VF.woff2`, as: 'font', type: 'font/woff2', crossorigin: '' },
				},
			],
			customCss: ['./src/styles/custom.css'],
			components: {
				Hero: './src/components/Hero.astro',
			},
			expressiveCode: {
				themes: [horaDark, horaLight],
				useStarlightUiThemeColors: true,
				styleOverrides: {
					borderRadius: '10px',
					frames: { frameBoxShadowCssValue: 'none' },
				},
			},
			social: [
				{ icon: 'github', label: 'GitHub', href: 'https://github.com/uplg/hora' },
			],
			editLink: {
				baseUrl: 'https://github.com/uplg/hora/edit/main/docs/',
			},
			sidebar: [
				{
					label: 'Start here',
					items: [
						{ label: 'Getting started', slug: 'getting-started' },
						{ label: 'When Hora fits', slug: 'when-hora-fits' },
						{ label: 'Concepts', slug: 'concepts' },
						{ label: 'Configuration', slug: 'configuration' },
						{ label: 'Upgrading', slug: 'upgrading' },
					],
				},
				{
					label: 'Guides',
					items: [
						{ label: 'Monitors', slug: 'guides/monitors' },
						{ label: 'Alerting & notifications', slug: 'guides/alerting' },
						{ label: 'The status page', slug: 'guides/status-page' },
						{ label: 'SLOs & error budgets', slug: 'guides/slo' },
						{ label: 'Incidents & history', slug: 'guides/incidents' },
						{ label: 'Per-group pages & SLA reports', slug: 'guides/multi-tenant' },
						{ label: 'Mutual surveillance (peers)', slug: 'guides/peers' },
						{ label: 'Importing from Uptime Kuma', slug: 'guides/import' },
					],
				},
				{
					label: 'Reference',
					items: [
						{ label: 'CLI', slug: 'reference/cli' },
						{ label: 'HTTP API', slug: 'reference/api' },
					],
				},
				{
					label: 'Project',
					items: [
						{ label: 'Roadmap', slug: 'roadmap' },
						{ label: 'Changelog', link: '/changelog/' },
						{ label: 'Development', slug: 'development' },
						{ label: 'Brand', slug: 'brand' },
					],
				},
			],
		}),
	],
});
