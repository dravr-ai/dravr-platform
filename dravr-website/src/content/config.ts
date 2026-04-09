// ABOUTME: Astro Content Collections schema for alpha user documentation
// ABOUTME: Defines frontmatter types for the /docs guides

import { defineCollection, z } from 'astro:content';

const docs = defineCollection({
  type: 'content',
  schema: z.object({
    title: z.string(),
    description: z.string(),
    order: z.number(),
    platform: z.string().optional(),
  }),
});

export const collections = { docs };
