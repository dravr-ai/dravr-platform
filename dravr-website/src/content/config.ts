// ABOUTME: Astro Content Collections schema for alpha user documentation
// ABOUTME: Defines frontmatter types for the /docs (EN) and /docs-fr (FR) guide collections

import { defineCollection, z } from 'astro:content';

const guideSchema = defineCollection({
  type: 'content',
  schema: z.object({
    title: z.string(),
    description: z.string(),
    order: z.number(),
    platform: z.string().optional(),
  }),
});

export const collections = {
  docs: guideSchema,
  'docs-fr': guideSchema,
};
