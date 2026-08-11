/*
 * A mock of the Figma Plugin API, faithful enough to execute the generator
 * end to end and catch runtime errors: typos, missing lookups, wrong call
 * order, unbound variables.
 *
 * It does NOT validate Figma's semantics (whether a sizing mode is legal in a
 * given structural context, whether a scope name is real). It catches the
 * class of bug that would otherwise surface only after loading the plugin.
 */
const fs = require('fs');
const path = require('path');
const vm = require('vm');

let idSeq = 1;
const nextId = (p) => `${p}:${idSeq++}`;

const calls = { boundVariables: 0, textStyleBinds: 0 };

function baseNode(type, name) {
  const node = {
    id: nextId('n'),
    type,
    name: name || type,
    children: [],
    x: 0, y: 0, width: 100, height: 32,
    fills: [], strokes: [], effects: [],
    boundVars: {},
    appendChild(child) {
      if (!child) throw new Error(`${this.name}.appendChild(undefined)`);
      child.parent = this;
      this.children.push(child);
    },
    insertChild(i, child) {
      const at = this.children.indexOf(child);
      if (at >= 0) this.children.splice(at, 1);
      this.children.splice(i, 0, child);
      child.parent = this;
    },
    resize(w, h) {
      if (typeof w !== 'number' || Number.isNaN(w)) throw new Error(`${this.name}.resize bad width: ${w}`);
      if (typeof h !== 'number' || Number.isNaN(h)) throw new Error(`${this.name}.resize bad height: ${h}`);
      this.width = w; this.height = h;
      // Faithful to the real API: resize clears both sizing modes.
      this.primaryAxisSizingMode = 'FIXED';
      this.counterAxisSizingMode = 'FIXED';
    },
    setBoundVariable(field, variable) {
      if (!variable) throw new Error(`${this.name}.setBoundVariable('${field}', undefined) — token missing`);
      this.boundVars[field] = variable.id;
      calls.boundVariables++;
    },
    remove() {
      if (this.parent) {
        const at = this.parent.children.indexOf(this);
        if (at >= 0) this.parent.children.splice(at, 1);
      }
      this.removed = true;
    },
    async loadAsync() {},
    async setTextStyleIdAsync() {},
    async setEffectStyleIdAsync() {},
  };
  return node;
}

function makeTextNode() {
  const t = baseNode('TEXT', 'Text');
  let chars = '';
  Object.defineProperty(t, 'characters', {
    configurable: true,
    get: () => chars,
    set(v) {
      if (!t.fontName) throw new Error('wrote characters before fontName was set');
      // The real rule: the node's font must be loaded before any text write.
      const key = t.fontName.family + ' ' + t.fontName.style;
      if (!loadedFonts.has(key)) throw new Error(`font not loaded: ${key}`);
      chars = v;
      t.width = Math.max(10, v.length * 7);
      t.height = 16;
    },
  });
  return t;
}

const root = baseNode('DOCUMENT', 'root');
const page1 = baseNode('PAGE', 'Page 1');
root.appendChild(page1);

const collections = [];
const variables = [];
const textStyles = [];
const effectStyles = [];
const loadedFonts = new Set();

const figma = {
  root,
  currentPage: page1,
  createPage() {
    // The real Starter cap. Left generous here so the generator's own reuse
    // logic is what gets exercised, not the throw.
    const p = baseNode('PAGE', 'Page ' + (root.children.length + 1));
    root.appendChild(p);
    return p;
  },
  async setCurrentPageAsync(p) {
    if (!p) throw new Error('setCurrentPageAsync(undefined)');
    figma.currentPage = p;
  },
  async loadAllPagesAsync() {},
  async loadFontAsync(f) {
    if (!f || !f.family || !f.style) throw new Error('loadFontAsync bad font: ' + JSON.stringify(f));
    loadedFonts.add(f.family + ' ' + f.style);
  },
  createText() { return makeTextNode(); },
  createFrame() { return baseNode('FRAME'); },
  createEllipse() { return baseNode('ELLIPSE'); },
  createComponent() { return baseNode('COMPONENT'); },
  createTextStyle() { const s = { id: nextId('ts'), name: '' }; s.setBoundVariable = (f, v) => { if (!v) throw new Error(`text style ${s.name}.setBoundVariable('${f}', undefined)`); calls.textStyleBinds++; }; textStyles.push(s); return s; },
  createEffectStyle() { const s = { id: nextId('es'), name: '', effects: [] }; effectStyles.push(s); return s; },
  async getLocalTextStylesAsync() { return textStyles.slice(); },
  async getLocalEffectStylesAsync() { return effectStyles.slice(); },
  combineAsVariants(nodes, parent) {
    if (!nodes || !nodes.length) throw new Error('combineAsVariants with no nodes');
    const set = baseNode('COMPONENT_SET', 'Set');
    for (const n of nodes) { n.remove(); set.appendChild(n); }
    parent.appendChild(set);
    return set;
  },
  variables: {
    async getLocalVariableCollectionsAsync() { return collections.slice(); },
    async getLocalVariablesAsync() { return variables.slice(); },
    createVariableCollection(name) {
      const modeId = nextId('m');
      const c = {
        id: nextId('VariableCollectionId'),
        name,
        modes: [{ modeId, name: 'Mode 1' }],
        variableIds: [],
        renameMode(id, n) { this.modes.find((m) => m.modeId === id).name = n; },
        addMode() { throw new Error('Limited to 1 modes only'); },
      };
      collections.push(c);
      return c;
    },
    createVariable(name, collection, type) {
      if (!collection) throw new Error(`createVariable('${name}') with no collection`);
      if (/\./.test(name)) throw new Error(`invalid variable name: ${name}`);
      const v = {
        id: nextId('VariableID'),
        name,
        resolvedType: type,
        variableCollectionId: collection.id,
        scopes: ['ALL_SCOPES'],
        values: {},
        setValueForMode(mode, value) {
          if (value === undefined) throw new Error(`${name}.setValueForMode undefined`);
          if (value && value.type === 'VARIABLE_ALIAS' && !value.id) {
            throw new Error(`${name} aliased to a variable with no id`);
          }
          this.values[mode] = value;
        },
        setVariableCodeSyntax(platform, syntax) {
          if (platform === 'WEB' && !/^var\(--/.test(syntax)) {
            throw new Error(`${name} WEB code syntax must be var(--…), got: ${syntax}`);
          }
          this.codeSyntax = syntax;
        },
      };
      collection.variableIds.push(v.id);
      variables.push(v);
      return v;
    },
    setBoundVariableForPaint(paint, field, variable) {
      if (!variable) throw new Error(`setBoundVariableForPaint('${field}', undefined) — token missing`);
      return Object.assign({}, paint, { boundVariables: { [field]: { id: variable.id } } });
    },
  },
  ui: { postMessage() {}, onmessage: null },
  showUI() {},
  notify(msg) { mockNotices.push(msg); },
  closePlugin() {},
};

const mockNotices = [];

// ---- run ------------------------------------------------------------------
const codePath = path.join(__dirname, 'code.js');
const source = fs.readFileSync(codePath, 'utf8');

const sandbox = { figma, __html__: '<html></html>', console, Date, Math, JSON, Object, Array, String, Number, parseInt, RegExp, Error };
vm.createContext(sandbox);

const logs = [];
figma.ui.postMessage = (m) => { if (m.type === 'log') logs.push(m.line); if (m.type === 'error') logs.push('ERROR: ' + m.message); };

try {
  new vm.Script(source, { filename: 'code.js' }).runInContext(sandbox);
} catch (e) {
  console.error('LOAD FAILED:', e.message);
  process.exit(1);
}

if (typeof figma.ui.onmessage !== 'function') {
  console.error('FAILED: plugin did not register figma.ui.onmessage');
  process.exit(1);
}

(async () => {
  let failed = false;
  const originalPost = figma.ui.postMessage;
  figma.ui.postMessage = (m) => {
    originalPost(m);
    if (m.type === 'error') { failed = true; }
    if (m.type === 'done') { sandbox.__summary = m.summary; }
  };

  await figma.ui.onmessage({ type: 'run', what: 'all', rebuild: false });

  console.log('--- generator log ---');
  for (const l of logs) console.log('  ' + l);

  if (failed) { console.error('\nRUN FAILED'); process.exit(1); }

  const s = sandbox.__summary;
  console.log('\n--- results ---');
  console.log('  collections      :', collections.map((c) => `${c.name}(${c.variableIds.length})`).join(', '));
  console.log('  variables total  :', variables.length);
  console.log('  text styles      :', textStyles.length);
  console.log('  effect styles    :', effectStyles.length);
  console.log('  pages            :', root.children.map((p) => p.name).join(', '));
  console.log('  components built :', (s.components || []).join(', ') || '(none)');
  console.log('  variable binds   :', calls.boundVariables, '| text-style binds:', calls.textStyleBinds);

  // Assertions.
  const problems = [];
  const allScopes = variables.filter((v) => v.scopes.indexOf('ALL_SCOPES') >= 0);
  if (allScopes.length) problems.push(`${allScopes.length} variables left at ALL_SCOPES`);
  const noSyntax = variables.filter((v) => !v.codeSyntax);
  if (noSyntax.length) problems.push(`${noSyntax.length} variables have no code syntax`);
  const noValue = variables.filter((v) => Object.keys(v.values).length === 0);
  if (noValue.length) problems.push(`${noValue.length} variables have no value`);
  if (root.children.length !== 3) problems.push(`expected 3 pages, got ${root.children.length}`);

  // Idempotency: a second run must not duplicate anything.
  const before = { v: variables.length, t: textStyles.length, e: effectStyles.length };
  await figma.ui.onmessage({ type: 'run', what: 'all', rebuild: false });
  if (variables.length !== before.v) problems.push(`second run added ${variables.length - before.v} variables`);
  if (textStyles.length !== before.t) problems.push(`second run added ${textStyles.length - before.t} text styles`);
  if (effectStyles.length !== before.e) problems.push(`second run added ${effectStyles.length - before.e} effect styles`);
  if (root.children.length !== 3) problems.push(`second run changed page count to ${root.children.length}`);

  console.log('\n--- idempotency (second run) ---');
  console.log('  variables:', variables.length, '| text styles:', textStyles.length,
              '| effect styles:', effectStyles.length, '| pages:', root.children.length);

  if (problems.length) {
    console.error('\nPROBLEMS:');
    for (const p of problems) console.error('  ✗ ' + p);
    process.exit(1);
  }
  console.log('\nALL CHECKS PASSED');
})();
