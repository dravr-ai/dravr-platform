// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Fails on any runtime import cycle in the app's own modules (src/ and app/)
// ABOUTME: Metro warns "Require cycle" once per bundle load; this makes the cycle a red test instead of log noise

import fs from 'fs';
import os from 'os';
import path from 'path';
import ts from 'typescript';

/**
 * carnet#356: `ui/index.ts -> FloatingSearchBar -> ExpandableTabBar ->
 * ChatPlusFlows -> ui/index.ts` logged 126 "Require cycle" warnings across two
 * Android runs, through the module that drew the tab bar, and nobody had
 * recorded it. React Native's own warning is that such a cycle "can result in
 * uninitialized values". The layout that produced it was refactored away, but
 * nothing stopped the next barrel import from a module the barrel re-exports.
 *
 * Only edges that exist at runtime count: `import type`, `export type` and
 * imports whose every binding is `type` are erased by Babel and cannot leave a
 * module half-initialised. Dynamic `import()` and a `require` inside a function
 * body are lazy, so they are not edges either.
 */
const MOBILE_ROOT = path.resolve(__dirname, '..');
const ROOTS = ['src', 'app'];
const EXTENSIONS = ['.ts', '.tsx', '.js', '.jsx'];
const PLATFORMS = ['', '.native', '.ios', '.android'];

const isTest = (file: string): boolean =>
  file.split(path.sep).includes('__tests__') || /\.(test|spec)\.[jt]sx?$/.test(file);

function sourceFiles(root: string, dirs: string[]): string[] {
  const out: string[] = [];
  const walk = (dir: string): void => {
    if (!fs.existsSync(dir)) return;
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const full = path.join(dir, entry.name);
      if (entry.isDirectory()) {
        if (entry.name !== 'node_modules') walk(full);
      } else if (EXTENSIONS.includes(path.extname(entry.name)) && !entry.name.endsWith('.d.ts') && !isTest(full)) {
        out.push(full);
      }
    }
  };
  for (const dir of dirs) walk(path.join(root, dir));
  return out;
}

/** Every file a specifier can land on, across the platform variants Metro picks between. */
function resolve(root: string, fromFile: string, specifier: string): string[] {
  let base: string;
  if (specifier.startsWith('.')) base = path.resolve(path.dirname(fromFile), specifier);
  else if (specifier.startsWith('@/')) base = path.join(root, 'src', specifier.slice(2));
  else return [];
  const stems = [base, path.join(base, 'index')];
  for (const stem of stems) {
    const hits = PLATFORMS.flatMap((platform) => EXTENSIONS.map((ext) => `${stem}${platform}${ext}`)).filter(
      (candidate) => fs.existsSync(candidate) && fs.statSync(candidate).isFile()
    );
    if (hits.length > 0) return hits;
  }
  return [];
}

function allTypeOnly(elements: ts.NodeArray<ts.ImportSpecifier | ts.ExportSpecifier>): boolean {
  return elements.length > 0 && elements.every((element) => element.isTypeOnly);
}

/** The specifiers a module evaluates when it is first required. */
function runtimeSpecifiers(file: string): string[] {
  const source = ts.createSourceFile(file, fs.readFileSync(file, 'utf8'), ts.ScriptTarget.Latest, false);
  const specifiers: string[] = [];
  for (const statement of source.statements) {
    if (ts.isImportDeclaration(statement) && ts.isStringLiteral(statement.moduleSpecifier)) {
      const clause = statement.importClause;
      const typeOnly =
        clause !== undefined &&
        (clause.isTypeOnly ||
          (clause.name === undefined &&
            clause.namedBindings !== undefined &&
            ts.isNamedImports(clause.namedBindings) &&
            allTypeOnly(clause.namedBindings.elements)));
      if (!typeOnly) specifiers.push(statement.moduleSpecifier.text);
    } else if (
      ts.isExportDeclaration(statement) &&
      statement.moduleSpecifier !== undefined &&
      ts.isStringLiteral(statement.moduleSpecifier)
    ) {
      const clause = statement.exportClause;
      const typeOnly =
        statement.isTypeOnly || (clause !== undefined && ts.isNamedExports(clause) && allTypeOnly(clause.elements));
      if (!typeOnly) specifiers.push(statement.moduleSpecifier.text);
    } else if (ts.isVariableStatement(statement)) {
      for (const declaration of statement.declarationList.declarations) {
        const required = eagerRequire(declaration.initializer);
        if (required !== undefined) specifiers.push(required);
      }
    } else if (ts.isExpressionStatement(statement)) {
      const required = eagerRequire(statement.expression);
      if (required !== undefined) specifiers.push(required);
    }
  }
  return specifiers;
}

/**
 * The specifier of a `require('…')` evaluated where it stands, seen through
 * what wraps its result without deferring it: `.default`, `['x']`, `as T`, `!`
 * and parentheses. A `require` inside a function body never reaches here.
 */
function eagerRequire(expression: ts.Expression | undefined): string | undefined {
  let node = expression;
  while (
    node !== undefined &&
    (ts.isPropertyAccessExpression(node) ||
      ts.isElementAccessExpression(node) ||
      ts.isAsExpression(node) ||
      ts.isNonNullExpression(node) ||
      ts.isParenthesizedExpression(node))
  ) {
    node = node.expression;
  }
  if (
    node !== undefined &&
    ts.isCallExpression(node) &&
    ts.isIdentifier(node.expression) &&
    node.expression.text === 'require' &&
    node.arguments.length === 1 &&
    ts.isStringLiteral(node.arguments[0])
  ) {
    return node.arguments[0].text;
  }
  return undefined;
}

/** Strongly connected components of size > 1, or a module that imports itself — each one a require cycle. */
interface ImportGraph {
  /** Each module and the modules it evaluates on load. */
  edges: Map<string, string[]>;
  /**
   * `file: specifier` for every local specifier (relative or `@/`) that lands
   * on no module and no file on disk. Each one is an edge the cycle search
   * cannot see, so a resolver gap reads as a cycle-free app; an asset import
   * (`./logo.png`) exists on disk and is not counted.
   */
  unresolved: string[];
}

function importGraph(root: string, dirs: string[]): ImportGraph {
  const edges = new Map<string, string[]>();
  const unresolved: string[] = [];
  for (const file of sourceFiles(root, dirs)) {
    const targets: string[] = [];
    for (const specifier of runtimeSpecifiers(file)) {
      const hits = resolve(root, file, specifier);
      targets.push(...hits);
      const local = specifier.startsWith('.') || specifier.startsWith('@/');
      const onDisk = specifier.startsWith('.') && fs.existsSync(path.resolve(path.dirname(file), specifier));
      if (local && hits.length === 0 && !onDisk) unresolved.push(`${path.relative(root, file)}: ${specifier}`);
    }
    edges.set(file, targets);
  }
  return { edges, unresolved };
}

function findImportCycles(root: string, dirs: string[]): string[][] {
  const { edges } = importGraph(root, dirs);
  const files = [...edges.keys()];

  // Tarjan, iteratively: the app is deep enough that recursion is not worth the risk.
  let counter = 0;
  const index = new Map<string, number>();
  const low = new Map<string, number>();
  const onStack = new Set<string>();
  const stack: string[] = [];
  const cycles: string[][] = [];
  for (const start of files) {
    if (index.has(start)) continue;
    const work: Array<{ node: string; next: number }> = [{ node: start, next: 0 }];
    index.set(start, counter);
    low.set(start, counter);
    counter += 1;
    stack.push(start);
    onStack.add(start);
    while (work.length > 0) {
      const frame = work[work.length - 1];
      const targets = edges.get(frame.node) ?? [];
      if (frame.next < targets.length) {
        const target = targets[frame.next];
        frame.next += 1;
        if (!edges.has(target)) continue;
        if (!index.has(target)) {
          index.set(target, counter);
          low.set(target, counter);
          counter += 1;
          stack.push(target);
          onStack.add(target);
          work.push({ node: target, next: 0 });
        } else if (onStack.has(target)) {
          low.set(frame.node, Math.min(low.get(frame.node) as number, index.get(target) as number));
        }
        continue;
      }
      work.pop();
      if (work.length > 0) {
        const parent = work[work.length - 1].node;
        low.set(parent, Math.min(low.get(parent) as number, low.get(frame.node) as number));
      }
      if (low.get(frame.node) === index.get(frame.node)) {
        const component: string[] = [];
        let member: string;
        do {
          member = stack.pop() as string;
          onStack.delete(member);
          component.push(member);
        } while (member !== frame.node);
        const selfLoop = component.length === 1 && (edges.get(frame.node) ?? []).includes(frame.node);
        if (component.length > 1 || selfLoop) {
          cycles.push(component.map((file) => path.relative(root, file)).sort());
        }
      }
    }
  }
  return cycles;
}

describe('runtime import cycles', () => {
  it('the app has none, in src/ or app/', () => {
    expect(findImportCycles(MOBILE_ROOT, ROOTS)).toEqual([]);
  });

  // A search over an empty or unresolved graph finds no cycle either, so the
  // pass above means something only if the graph it searched is the app's.
  it('searched the whole app: every root has modules, and every local import resolved', () => {
    const { edges, unresolved } = importGraph(MOBILE_ROOT, ROOTS);
    for (const root of ROOTS) {
      const modules = [...edges.keys()].filter((file) => path.relative(MOBILE_ROOT, file).startsWith(`${root}${path.sep}`));
      expect({ root, scanned: modules.length > 0 }).toEqual({ root, scanned: true });
    }
    const resolvedEdges = [...edges.values()].reduce((total, targets) => total + targets.length, 0);
    expect(resolvedEdges).toBeGreaterThan(edges.size);
    expect(unresolved).toEqual([]);
  });

  describe('the detector', () => {
    let fixture: string;
    const write = (file: string, body: string): void => {
      const full = path.join(fixture, file);
      fs.mkdirSync(path.dirname(full), { recursive: true });
      fs.writeFileSync(full, body);
    };

    beforeEach(() => {
      fixture = fs.mkdtempSync(path.join(os.tmpdir(), 'import-cycles-'));
    });
    afterEach(() => {
      fs.rmSync(fixture, { recursive: true, force: true });
    });

    it("finds carnet#356's shape: a barrel re-export that imports the barrel back", () => {
      write('src/components/ui/index.ts', "export { TabBar } from './TabBar';\n");
      write('src/components/ui/TabBar.tsx', "import { Flows } from '../../screens/Flows';\nexport const TabBar = Flows;\n");
      write('src/screens/Flows.tsx', "import { TabBar } from '@/components/ui';\nexport const Flows = () => TabBar;\n");
      expect(findImportCycles(fixture, ['src'])).toEqual([
        ['src/components/ui/TabBar.tsx', 'src/components/ui/index.ts', 'src/screens/Flows.tsx'],
      ]);
    });

    it('ignores edges Babel erases, and lazy requires', () => {
      write('src/a.ts', "import type { B } from './b';\nexport type A = B;\nexport const a = 1;\n");
      write('src/b.ts', "import { type A } from './a';\nexport { type A as Again } from './a';\nexport type B = A;\n");
      write('src/c.ts', "export const c = () => require('./d');\n");
      write('src/d.ts', "import { c } from './c';\nexport const d = c;\n");
      expect(findImportCycles(fixture, ['src'])).toEqual([]);
    });

    it('follows a top-level require and a platform-specific file', () => {
      write('src/e.ts', "const f = require('./f');\nexport const e = f;\n");
      write('src/f.ios.ts', "import { e } from './e';\nexport const f = e;\n");
      expect(findImportCycles(fixture, ['src'])).toEqual([['src/e.ts', 'src/f.ios.ts']]);
    });

    it("follows a require whose result is unwrapped, and a bare side-effect require", () => {
      write('src/g.ts', "const H = require('./h').default as unknown;\nexport const g = H;\n");
      write('src/h.ts', "require('./g');\nexport default 1;\n");
      expect(findImportCycles(fixture, ['src'])).toEqual([['src/g.ts', 'src/h.ts']]);
    });

    it('reports a local import it cannot resolve, but not an asset on disk', () => {
      write('src/i.ts', "import logo from './logo.png';\nimport { gone } from './gone';\nimport { x } from '@/missing';\nexport const i = [logo, gone, x];\n");
      write('src/logo.png', '');
      expect(importGraph(fixture, ['src']).unresolved).toEqual(['src/i.ts: ./gone', 'src/i.ts: @/missing']);
    });
  });
});
