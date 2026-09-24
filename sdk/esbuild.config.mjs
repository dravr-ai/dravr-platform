// esbuild configuration for bundling the SDK with Node 24 target
// This bundles all TypeScript and resolves ESM/CJS module issues
import * as esbuild from 'esbuild';
import { readFileSync, mkdirSync, writeFileSync, existsSync } from 'fs';
import { resolve, dirname, join } from 'path';
import { fileURLToPath } from 'url';

const __dirname = dirname(fileURLToPath(import.meta.url));

// Common build options
const commonOptions = {
  bundle: true,
  platform: 'node',
  target: 'node24', // Target Node 24 LTS (has native fetch)
  format: 'cjs',
  sourcemap: true,
  // Prefer ESM module resolution to handle packages like ajv@8 which are ESM-first
  // This ensures proper default export handling when bundling to CJS
  mainFields: ['module', 'main'],
  external: [
    // Keep native Node modules external - they can't be bundled
    '@napi-rs/keyring', // Native Rust module for OS keychain (replaces deprecated keytar)
  ],
  logLevel: 'info',
};

console.log('🔨 Building SDK with esbuild (Node 24 target)...\n');

// The OAuth callback pages are the MCP transport's own templates, so the SDK
// and the server show the same page. They ask for the shared Boreal stylesheet
// through {{HOSTED_PAGE_CSS}}, which the server fills with
// pierre_core::html::with_hosted_page_css; the bundle fills it here from the
// same generated file. A missing template fails the build: the bundle reads
// them when it loads, so shipping without one breaks every SDK command.
const templatesDir = join(__dirname, 'dist', 'templates');
const sourceTemplatesDir = join(__dirname, '..', 'crates', 'pierre-mcp-transport', 'templates');
const hostedPageCss = join(__dirname, '..', 'crates', 'pierre-core', 'src', 'hosted_page.css');
mkdirSync(templatesDir, { recursive: true });

if (!existsSync(hostedPageCss)) {
  throw new Error(`Hosted page stylesheet not found: ${hostedPageCss}`);
}
const css = readFileSync(hostedPageCss, 'utf8');

const templateFiles = ['oauth_success.html', 'oauth_error.html'];
for (const file of templateFiles) {
  const src = join(sourceTemplatesDir, file);
  if (!existsSync(src)) {
    throw new Error(`OAuth template not found: ${src}`);
  }
  const html = readFileSync(src, 'utf8');
  if (!html.includes('{{HOSTED_PAGE_CSS}}')) {
    throw new Error(`${src} no longer asks for {{HOSTED_PAGE_CSS}}`);
  }
  writeFileSync(join(templatesDir, file), html.replaceAll('{{HOSTED_PAGE_CSS}}', css));
  console.log(`📋 Wrote ${file} to dist/templates/ with the hosted page stylesheet`);
}

// Build CLI entry point (shebang already in source file)
await esbuild.build({
  ...commonOptions,
  entryPoints: ['src/cli.ts'],
  outfile: 'dist/cli.js',
});
console.log('✅ Built dist/cli.js');

// Build main library entry point
await esbuild.build({
  ...commonOptions,
  entryPoints: ['src/index.ts'],
  outfile: 'dist/index.js',
});
console.log('✅ Built dist/index.js');

console.log('\n✨ Build completed successfully with esbuild');
