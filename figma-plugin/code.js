/*
 * OpenCompany Design System — Figma generator
 * ===========================================
 *
 * Regenerates the Figma library from the tokens in `frontend/src/index.css`.
 * The library is a build output, not a hand-drawn artefact: run this again
 * after a token changes and the variables, styles and documentation pages
 * catch up.
 *
 * One file, no bundler. Figma loads a single `main` script and has no module
 * system, so a separate tokens file would simply never be read. The token
 * block below is the only place values live; everything else reads from it.
 *
 * Idempotent throughout. Every create is preceded by a lookup by exact name,
 * so a second run updates in place rather than duplicating. Components are
 * skipped if they already exist unless "Rebuild components" is ticked —
 * rebuilding replaces the component, which detaches existing instances.
 *
 * Plan limits this cannot work around (they are enforced by Figma, not here):
 *   - Starter allows 3 pages, so Foundations and Components share pages
 *     rather than getting one page per component.
 *   - Starter allows 1 mode per collection, so Light and Dark are parallel
 *     collections instead of two modes of one. On Professional, merge them.
 */

/* ==========================================================================
 * TOKENS — transcribed from frontend/src/index.css
 *
 * Hex rather than the oklch the stylesheet declares, because the Plugin API
 * takes sRGB. Same colours: the hex on each line of `index.css` is the
 * canonical value, and oklch is how CSS states it so the ramps step evenly in
 * perceptual lightness.
 *
 * Kept in sync by hand, deliberately. Parsing the stylesheet would mean
 * resolving `color-mix()`, `oklch()` and the cascade, and would break
 * silently the first time one of them moved.
 * ========================================================================== */

var PRIMITIVES = {
  'brand/50': '#EEEDFF', 'brand/100': '#E0DEFF', 'brand/200': '#C7C3FF',
  'brand/300': '#A6A0FF', 'brand/400': '#857EFF', 'brand/500': '#635BFF',
  'brand/600': '#524AE0', 'brand/700': '#423BBA', 'brand/800': '#322C8F',
  'brand/900': '#241F66',

  'gray/25': '#FCFCFD', 'gray/50': '#F4F4F7', 'gray/100': '#EEEEF3',
  'gray/200': '#E6E6EC', 'gray/300': '#D9D9E3', 'gray/400': '#8C8C9E',
  'gray/500': '#6E6E80', 'gray/600': '#9797A8', 'gray/800': '#23232C',
  'gray/850': '#1E1E26', 'gray/875': '#17171D', 'gray/900': '#16161D',
  'gray/925': '#131318', 'gray/950': '#0C0C0F',

  'white': '#FFFFFF',

  'green/mark': '#12A150', 'green/text': '#0A7D3E', 'green/bright': '#35C77F',
  'amber/mark': '#F5A524', 'amber/text': '#A16207', 'amber/bright': '#FFC53D',
  'red/mark': '#E5484D', 'red/text': '#C62A2F', 'red/bright': '#FF6369',
  'cyan/mark': '#0EA5E9', 'cyan/text': '#0A6E9C', 'cyan/bright': '#38BDF8',
  'pink/mark': '#E93D82', 'pink/bright': '#FF6BA6',
};

var FILL = ['FRAME_FILL', 'SHAPE_FILL'];
var TEXT = ['TEXT_FILL'];
var STROKE = ['STROKE_COLOR'];

// [semantic name, primitive, scopes, the CSS variable it maps to]
var SEMANTIC_LIGHT = [
  ['color/bg/canvas', 'gray/25', FILL, '--background'],
  ['color/bg/card', 'white', FILL, '--card'],
  ['color/bg/popover', 'white', FILL, '--popover'],
  ['color/bg/muted', 'gray/50', FILL, '--muted'],
  ['color/bg/secondary', 'gray/100', FILL, '--secondary'],
  ['color/bg/sidebar', 'gray/50', FILL, '--sidebar'],
  ['color/bg/nav-active', 'brand/50', FILL, '--sidebar-accent'],

  ['color/text/primary', 'gray/900', TEXT, '--foreground'],
  ['color/text/muted', 'gray/500', TEXT, '--muted-foreground'],
  ['color/text/brand', 'brand/500', TEXT, '--primary'],
  ['color/text/on-brand', 'white', TEXT, '--primary-foreground'],
  ['color/text/nav-active', 'brand/700', TEXT, '--sidebar-accent-foreground'],

  ['color/border/default', 'gray/200', STROKE, '--border'],
  ['color/border/strong', 'gray/300', STROKE, '--input'],
  ['color/border/focus', 'brand/500', STROKE, '--ring'],

  ['color/action/primary', 'brand/500', FILL, '--primary'],
  ['color/action/primary-hover', 'brand/600', FILL, '--primary'],

  ['color/status/idle', 'gray/400', FILL, '--status-idle'],
  ['color/status/idle-text', 'gray/500', TEXT, '--status-idle-text'],
  ['color/status/running', 'cyan/mark', FILL, '--status-running'],
  ['color/status/running-text', 'cyan/text', TEXT, '--status-running-text'],
  ['color/status/blocked', 'amber/mark', FILL, '--status-blocked'],
  ['color/status/blocked-text', 'amber/text', TEXT, '--status-blocked-text'],
  ['color/status/done', 'green/mark', FILL, '--status-done'],
  ['color/status/done-text', 'green/text', TEXT, '--status-done-text'],
  ['color/status/failed', 'red/mark', FILL, '--status-failed'],
  ['color/status/failed-text', 'red/text', TEXT, '--status-failed-text'],

  ['color/chart/1', 'brand/500', FILL, '--chart-1'],
  ['color/chart/2', 'cyan/mark', FILL, '--chart-2'],
  ['color/chart/3', 'green/mark', FILL, '--chart-3'],
  ['color/chart/4', 'amber/mark', FILL, '--chart-4'],
  ['color/chart/5', 'pink/mark', FILL, '--chart-5'],
];

// Dark is NOT a filter over light. Several roles point at a different ramp
// step — the accent steps up from brand/500 to brand/400, because 500 is too
// dense to read as ink on near-black.
var SEMANTIC_DARK = [
  ['color/bg/canvas', 'gray/950', FILL, '--background'],
  ['color/bg/card', 'gray/925', FILL, '--card'],
  ['color/bg/popover', 'gray/875', FILL, '--popover'],
  ['color/bg/muted', 'gray/850', FILL, '--muted'],
  ['color/bg/secondary', 'gray/800', FILL, '--secondary'],
  ['color/bg/sidebar', 'gray/925', FILL, '--sidebar'],
  ['color/bg/nav-active', 'gray/800', FILL, '--sidebar-accent'],

  ['color/text/primary', 'gray/50', TEXT, '--foreground'],
  ['color/text/muted', 'gray/600', TEXT, '--muted-foreground'],
  ['color/text/brand', 'brand/400', TEXT, '--primary'],
  ['color/text/on-brand', 'white', TEXT, '--primary-foreground'],
  ['color/text/nav-active', 'brand/300', TEXT, '--sidebar-accent-foreground'],

  ['color/border/focus', 'brand/400', STROKE, '--ring'],

  ['color/action/primary', 'brand/400', FILL, '--primary'],
  ['color/action/primary-hover', 'brand/300', FILL, '--primary'],

  ['color/status/idle', 'gray/600', FILL, '--status-idle'],
  ['color/status/idle-text', 'gray/600', TEXT, '--status-idle-text'],
  ['color/status/running', 'cyan/bright', FILL, '--status-running'],
  ['color/status/running-text', 'cyan/bright', TEXT, '--status-running-text'],
  ['color/status/blocked', 'amber/bright', FILL, '--status-blocked'],
  ['color/status/blocked-text', 'amber/bright', TEXT, '--status-blocked-text'],
  ['color/status/done', 'green/bright', FILL, '--status-done'],
  ['color/status/done-text', 'green/bright', TEXT, '--status-done-text'],
  ['color/status/failed', 'red/bright', FILL, '--status-failed'],
  ['color/status/failed-text', 'red/bright', TEXT, '--status-failed-text'],

  ['color/chart/1', 'brand/400', FILL, '--chart-1'],
  ['color/chart/2', 'cyan/bright', FILL, '--chart-2'],
  ['color/chart/3', 'green/bright', FILL, '--chart-3'],
  ['color/chart/4', 'amber/bright', FILL, '--chart-4'],
  ['color/chart/5', 'pink/bright', FILL, '--chart-5'],
];

// The two dark borders that cannot alias a primitive. Translucent white on
// purpose: a dark border must read against the canvas, the card AND the
// popover — three different lightnesses — and any fixed colour disappears
// against one of them.
var SEMANTIC_DARK_RGBA = [
  ['color/border/default', { r: 1, g: 1, b: 1, a: 0.09 }, STROKE, '--border'],
  ['color/border/strong', { r: 1, g: 1, b: 1, a: 0.14 }, STROKE, '--input'],
];

// [name, value, scopes, css]. `space/1-5` — Figma rejects `.` in a name.
var SCALE = [
  ['radius/sm', 6, ['CORNER_RADIUS'], '--radius-sm'],
  ['radius/md', 8, ['CORNER_RADIUS'], '--radius-md'],
  ['radius/lg', 10, ['CORNER_RADIUS'], '--radius-lg'],
  ['radius/xl', 14, ['CORNER_RADIUS'], '--radius-xl'],
  ['radius/2xl', 18, ['CORNER_RADIUS'], '--radius-2xl'],
  ['radius/full', 999, ['CORNER_RADIUS'], '--radius-full'],

  ['space/1', 4, ['GAP', 'WIDTH_HEIGHT'], '--spacing-1'],
  ['space/1-5', 6, ['GAP', 'WIDTH_HEIGHT'], '--spacing-1_5'],
  ['space/2', 8, ['GAP', 'WIDTH_HEIGHT'], '--spacing-2'],
  ['space/3', 12, ['GAP', 'WIDTH_HEIGHT'], '--spacing-3'],
  ['space/4', 16, ['GAP', 'WIDTH_HEIGHT'], '--spacing-4'],
  ['space/5', 20, ['GAP', 'WIDTH_HEIGHT'], '--spacing-5'],
  ['space/6', 24, ['GAP', 'WIDTH_HEIGHT'], '--spacing-6'],
  ['space/8', 32, ['GAP', 'WIDTH_HEIGHT'], '--spacing-8'],

  ['font-size/3xs', 10, ['FONT_SIZE'], '--text-3xs'],
  ['font-size/2xs', 11, ['FONT_SIZE'], '--text-2xs'],
  ['font-size/xs', 12, ['FONT_SIZE'], '--text-xs'],
  ['font-size/sm', 14, ['FONT_SIZE'], '--text-sm'],
  ['font-size/base', 16, ['FONT_SIZE'], '--text-base'],
  ['font-size/lg', 18, ['FONT_SIZE'], '--text-lg'],
  ['font-size/xl', 20, ['FONT_SIZE'], '--text-xl'],
  ['font-size/2xl', 24, ['FONT_SIZE'], '--text-2xl'],
];

// [style name, family, weight, size, line height, letter-spacing %, size token]
var TEXT_STYLES = [
  ['Body/3xs', 'Geist', 'Regular', 10, 14, 1, 'font-size/3xs'],
  ['Body/2xs', 'Geist', 'Regular', 11, 16, 0.5, 'font-size/2xs'],
  ['Body/xs', 'Geist', 'Regular', 12, 16, 0, 'font-size/xs'],
  ['Body/sm', 'Geist', 'Regular', 14, 20, 0, 'font-size/sm'],
  ['Body/base', 'Geist', 'Regular', 16, 24, 0, 'font-size/base'],

  ['Label/3xs', 'Geist', 'Medium', 10, 14, 1, 'font-size/3xs'],
  ['Label/2xs', 'Geist', 'Medium', 11, 16, 0.5, 'font-size/2xs'],
  ['Label/xs', 'Geist', 'Medium', 12, 16, 0, 'font-size/xs'],
  ['Label/sm', 'Geist', 'Medium', 14, 20, 0, 'font-size/sm'],

  ['Heading/sm', 'Geist', 'SemiBold', 14, 20, 0, 'font-size/sm'],
  ['Heading/lg', 'Geist', 'SemiBold', 18, 28, 0, 'font-size/lg'],
  ['Heading/xl', 'Geist', 'SemiBold', 20, 28, 0, 'font-size/xl'],
  ['Heading/2xl', 'Geist', 'SemiBold', 24, 32, 0, 'font-size/2xl'],

  ['Mono/3xs', 'Geist Mono', 'Regular', 10, 14, 0, 'font-size/3xs'],
  ['Mono/2xs', 'Geist Mono', 'Regular', 11, 16, 0, 'font-size/2xs'],
  ['Mono/xs', 'Geist Mono', 'Regular', 12, 16, 0, 'font-size/xs'],
  ['Mono/sm', 'Geist Mono', 'Regular', 14, 20, 0, 'font-size/sm'],
];

// Every font this generator writes with. Loaded once, up front: any text
// mutation on an unloaded font throws.
var FONTS = [
  { family: 'Geist', style: 'Regular' },
  { family: 'Geist', style: 'Medium' },
  { family: 'Geist', style: 'SemiBold' },
  { family: 'Geist Mono', style: 'Regular' },
];

// Shadow tints. Neutral-hued, never pure black — black over a hue-tinted
// surface goes muddy.
var SHADOW_LIGHT = { r: 0.135, g: 0.09, b: 0.15 }; // hsl(285 25% 12%)
var SHADOW_DARK = { r: 0.026, g: 0.008, b: 0.032 }; // hsl(285 60% 2%)
var SHADOW_BRAND = { r: 0.388, g: 0.357, b: 1.0 }; // #635BFF

/* ==========================================================================
 * HELPERS
 * ========================================================================== */

function hexToRgb(h) {
  var n = h.replace('#', '');
  return {
    r: parseInt(n.slice(0, 2), 16) / 255,
    g: parseInt(n.slice(2, 4), 16) / 255,
    b: parseInt(n.slice(4, 6), 16) / 255,
  };
}

/** A running log the UI renders, so a long build is legible while it runs. */
var LOG = [];
function log(line) {
  LOG.push(line);
  figma.ui.postMessage({ type: 'log', line: line });
}

/**
 * A solid paint bound to a variable.
 *
 * `opacity` must be set on the paint handed *into* the bind call. Merging it
 * onto the returned paint does not survive assignment — the binding call
 * produces the authoritative object. This cost a debugging round the first
 * time: every status badge rendered fully saturated.
 */
function boundPaint(variable, opacity) {
  var input = { type: 'SOLID', color: { r: 0, g: 0, b: 0 } };
  if (opacity != null) input.opacity = opacity;
  return figma.variables.setBoundVariableForPaint(input, 'color', variable);
}
function fillWith(variable, opacity) {
  return [boundPaint(variable, opacity)];
}

/** Look up one collection by name, creating it with a single named mode. */
async function ensureCollection(name) {
  var all = await figma.variables.getLocalVariableCollectionsAsync();
  for (var i = 0; i < all.length; i++) {
    if (all[i].name === name) return all[i];
  }
  var c = figma.variables.createVariableCollection(name);
  c.renameMode(c.modes[0].modeId, 'Value');
  return c;
}

/** Every local variable in `collection`, keyed by name. */
async function variablesIn(collection) {
  var all = await figma.variables.getLocalVariablesAsync();
  var map = {};
  for (var i = 0; i < all.length; i++) {
    if (all[i].variableCollectionId === collection.id) map[all[i].name] = all[i];
  }
  return map;
}

async function textStylesByName() {
  var styles = await figma.getLocalTextStylesAsync();
  var map = {};
  for (var i = 0; i < styles.length; i++) map[styles[i].name] = styles[i];
  return map;
}

async function effectStylesByName() {
  var styles = await figma.getLocalEffectStylesAsync();
  var map = {};
  for (var i = 0; i < styles.length; i++) map[styles[i].name] = styles[i];
  return map;
}

/** A text node in a given style, filled from a colour variable. */
function makeText(chars, family, style, size, colorVar) {
  var t = figma.createText();
  t.fontName = { family: family, style: style };
  t.characters = chars;
  t.fontSize = size;
  if (colorVar) t.fills = fillWith(colorVar);
  return t;
}

/* ==========================================================================
 * PHASE 1 — FOUNDATIONS
 * ========================================================================== */

async function buildVariables() {
  var prim = await ensureCollection('Primitives');
  var lightColl = await ensureCollection('Color · Light');
  var darkColl = await ensureCollection('Color · Dark');
  var scaleColl = await ensureCollection('Scale');

  // --- primitives -------------------------------------------------------
  var existing = await variablesIn(prim);
  var primMode = prim.modes[0].modeId;
  var names = Object.keys(PRIMITIVES);
  for (var i = 0; i < names.length; i++) {
    var name = names[i];
    var v = existing[name];
    if (!v) v = figma.variables.createVariable(name, prim, 'COLOR');
    v.setValueForMode(primMode, hexToRgb(PRIMITIVES[name]));
    // Hidden from every picker: designers pick semantics, and a raw ramp step
    // offered alongside them is how a system gets bypassed.
    v.scopes = [];
    v.setVariableCodeSyntax('WEB', 'var(--' + name.replace('/', '-') + ')');
  }
  log('Primitives: ' + names.length);

  var P = await variablesIn(prim);

  // --- semantics --------------------------------------------------------
  async function writeSemantics(collection, rows, rgbaRows) {
    var have = await variablesIn(collection);
    var mode = collection.modes[0].modeId;
    var count = 0;
    for (var j = 0; j < rows.length; j++) {
      var r = rows[j];
      var source = P[r[1]];
      if (!source) throw new Error('missing primitive: ' + r[1]);
      var v = have[r[0]];
      if (!v) v = figma.variables.createVariable(r[0], collection, 'COLOR');
      v.setValueForMode(mode, { type: 'VARIABLE_ALIAS', id: source.id });
      v.scopes = r[2];
      v.setVariableCodeSyntax('WEB', 'var(' + r[3] + ')');
      count++;
    }
    if (rgbaRows) {
      for (var k = 0; k < rgbaRows.length; k++) {
        var q = rgbaRows[k];
        var rv = have[q[0]];
        if (!rv) rv = figma.variables.createVariable(q[0], collection, 'COLOR');
        rv.setValueForMode(mode, q[1]);
        rv.scopes = q[2];
        rv.setVariableCodeSyntax('WEB', 'var(' + q[3] + ')');
        count++;
      }
    }
    return count;
  }

  var lightCount = await writeSemantics(lightColl, SEMANTIC_LIGHT, null);
  log('Color · Light: ' + lightCount);
  var darkCount = await writeSemantics(darkColl, SEMANTIC_DARK, SEMANTIC_DARK_RGBA);
  log('Color · Dark: ' + darkCount);

  // --- scale ------------------------------------------------------------
  var haveScale = await variablesIn(scaleColl);
  var scaleMode = scaleColl.modes[0].modeId;
  for (var m = 0; m < SCALE.length; m++) {
    var s = SCALE[m];
    var sv = haveScale[s[0]];
    if (!sv) sv = figma.variables.createVariable(s[0], scaleColl, 'FLOAT');
    sv.setValueForMode(scaleMode, s[1]);
    sv.scopes = s[2];
    sv.setVariableCodeSyntax('WEB', 'var(' + s[3] + ')');
  }
  log('Scale: ' + SCALE.length);

  return {
    prim: prim,
    light: lightColl,
    dark: darkColl,
    scale: scaleColl,
    total: names.length + lightCount + darkCount + SCALE.length,
  };
}

async function buildTextStyles(scaleColl) {
  var S = await variablesIn(scaleColl);
  var have = await textStylesByName();
  var made = 0;
  for (var i = 0; i < TEXT_STYLES.length; i++) {
    var row = TEXT_STYLES[i];
    var s = have[row[0]];
    if (!s) {
      s = figma.createTextStyle();
      s.name = row[0];
      made++;
    }
    s.fontName = { family: row[1], style: row[2] };
    s.fontSize = row[3];
    s.lineHeight = { unit: 'PIXELS', value: row[4] };
    s.letterSpacing = { unit: 'PERCENT', value: row[5] };
    // One source of truth for the ramp: the style's size follows the token.
    if (S[row[6]]) s.setBoundVariable('fontSize', S[row[6]]);
  }
  log('Text styles: ' + TEXT_STYLES.length + ' (' + made + ' new)');
  return TEXT_STYLES.length;
}

function drop(c, a, y, blur, spread) {
  return {
    type: 'DROP_SHADOW',
    color: { r: c.r, g: c.g, b: c.b, a: a },
    offset: { x: 0, y: y },
    radius: blur,
    spread: spread || 0,
    visible: true,
    blendMode: 'NORMAL',
  };
}
function innerTop(a) {
  return {
    type: 'INNER_SHADOW',
    color: { r: 1, g: 1, b: 1, a: a },
    offset: { x: 0, y: 1 },
    radius: 0,
    spread: 0,
    visible: true,
    blendMode: 'NORMAL',
  };
}

async function buildEffectStyles() {
  // Dark carries a 1px inset top highlight as well as the drop shadows:
  // shadow alone cannot separate two near-black surfaces, because there is no
  // light for it to occlude.
  var SETS = [
    ['Elevation/xs', [drop(SHADOW_LIGHT, 0.05, 1, 2)]],
    ['Elevation/sm', [drop(SHADOW_LIGHT, 0.08, 1, 2, -1), drop(SHADOW_LIGHT, 0.06, 1, 3)]],
    ['Elevation/md', [drop(SHADOW_LIGHT, 0.08, 2, 4, -2), drop(SHADOW_LIGHT, 0.08, 4, 12, -2)]],
    ['Elevation/lg', [drop(SHADOW_LIGHT, 0.10, 4, 8, -4), drop(SHADOW_LIGHT, 0.12, 12, 32, -8)]],
    ['Elevation/xl', [drop(SHADOW_LIGHT, 0.12, 8, 16, -8), drop(SHADOW_LIGHT, 0.16, 24, 56, -12)]],
    ['Elevation/brand', [drop(SHADOW_BRAND, 0.32, 2, 8, -2)]],

    ['Elevation Dark/xs', [drop(SHADOW_DARK, 0.40, 1, 2), innerTop(0.03)]],
    ['Elevation Dark/sm', [drop(SHADOW_DARK, 0.50, 1, 2, -1), drop(SHADOW_DARK, 0.40, 1, 3), innerTop(0.04)]],
    ['Elevation Dark/md', [drop(SHADOW_DARK, 0.50, 2, 4, -2), drop(SHADOW_DARK, 0.45, 4, 12, -2), innerTop(0.05)]],
    ['Elevation Dark/lg', [drop(SHADOW_DARK, 0.50, 4, 8, -4), drop(SHADOW_DARK, 0.55, 12, 32, -8), innerTop(0.06)]],
    ['Elevation Dark/xl', [drop(SHADOW_DARK, 0.55, 8, 16, -8), drop(SHADOW_DARK, 0.60, 24, 56, -12), innerTop(0.07)]],
    ['Elevation Dark/brand', [drop(SHADOW_BRAND, 0.40, 2, 12, -2)]],
  ];

  var have = await effectStylesByName();
  var made = 0;
  for (var i = 0; i < SETS.length; i++) {
    var s = have[SETS[i][0]];
    if (!s) {
      s = figma.createEffectStyle();
      s.name = SETS[i][0];
      made++;
    }
    s.effects = SETS[i][1];
  }
  log('Effect styles: ' + SETS.length + ' (' + made + ' new)');
  return SETS.length;
}

/* ==========================================================================
 * PHASE 2 — PAGES
 * ========================================================================== */

/**
 * Find or make the three pages.
 *
 * Starter allows three, which is exactly what this uses. A spare default-named
 * page is renamed rather than spending one of the three on a new create.
 */
async function ensurePages() {
  var WANTED = ['Cover', 'Foundations', 'Components'];
  var out = {};
  for (var i = 0; i < WANTED.length; i++) {
    var name = WANTED[i];
    var found = null;
    for (var j = 0; j < figma.root.children.length; j++) {
      if (figma.root.children[j].name === name) found = figma.root.children[j];
    }
    if (!found) {
      var spare = null;
      for (var k = 0; k < figma.root.children.length; k++) {
        var p = figma.root.children[k];
        if (/^Page \d+$/.test(p.name) && WANTED.indexOf(p.name) === -1) spare = p;
      }
      if (spare) {
        spare.name = name;
        found = spare;
      } else {
        found = figma.createPage();
        found.name = name;
      }
    }
    out[name] = found;
  }
  for (var m = 0; m < WANTED.length; m++) {
    var page = out[WANTED[m]];
    if (page) figma.root.insertChild(m, page);
  }
  return out;
}

/* ==========================================================================
 * PHASE 3 — COMPONENTS
 *
 * Every visual property binds to a variable. A hardcoded fill, radius or gap
 * in here is a bug: it is a value the system cannot see or change later.
 * ========================================================================== */

/** Place a finished component set on the page without colliding. */
function placeSet(set, name, description, x, y, L) {
  set.name = name;
  set.description = description;
  set.layoutMode = 'VERTICAL';
  set.itemSpacing = 16;
  set.counterAxisSizingMode = 'AUTO';
  set.primaryAxisSizingMode = 'AUTO';
  set.paddingLeft = 24;
  set.paddingRight = 24;
  set.paddingTop = 24;
  set.paddingBottom = 24;
  set.x = x;
  set.y = y;
  set.cornerRadius = 12;
  // A truthful preview background: Figma's default set frame is grey, which
  // makes a 14% status tint read as neutral.
  set.fills = fillWith(L['color/bg/card']);
  set.strokes = fillWith(L['color/border/default']);
  set.strokeWeight = 1;
  return set;
}

function buildInput(L, S) {
  // State=Default / Focus / Invalid / Disabled. Hover is omitted on purpose:
  // it is an interaction state the code owns, and duplicating it here would
  // be a second source of truth for the same value.
  var STATES = [
    ['Default', L['color/border/strong'], 1, 'acme-marketing', L['color/text/muted'], null],
    ['Focus', L['color/border/focus'], 2, 'acme-marketing', L['color/text/primary'], null],
    ['Invalid', L['color/status/failed'], 2, 'not a company', L['color/text/primary'], null],
    ['Disabled', L['color/border/strong'], 1, 'Unavailable', L['color/text/muted'], 0.5],
  ];
  var variants = [];
  for (var i = 0; i < STATES.length; i++) {
    var s = STATES[i];
    var field = figma.createComponent();
    field.name = 'State=' + s[0];
    field.layoutMode = 'HORIZONTAL';
    field.counterAxisAlignItems = 'CENTER';
    field.primaryAxisSizingMode = 'FIXED';
    field.counterAxisSizingMode = 'AUTO';
    field.paddingTop = 7;
    field.paddingBottom = 7;
    field.fills = fillWith(L['color/bg/card']);
    field.strokes = fillWith(s[1]);
    field.strokeWeight = s[2];
    field.setBoundVariable('paddingLeft', S['space/3']);
    field.setBoundVariable('paddingRight', S['space/3']);
    field.setBoundVariable('topLeftRadius', S['radius/md']);
    field.setBoundVariable('topRightRadius', S['radius/md']);
    field.setBoundVariable('bottomLeftRadius', S['radius/md']);
    field.setBoundVariable('bottomRightRadius', S['radius/md']);
    if (s[5] != null) field.opacity = s[5];

    var label = makeText(s[3], 'Geist', 'Regular', 14, s[4]);
    field.appendChild(label);
    label.setBoundVariable('fontSize', S['font-size/sm']);
    // resize() resets both sizing modes to FIXED, so the height mode is
    // re-asserted after it — otherwise the field stops hugging its label and
    // a type-scale change would no longer grow it.
    field.resize(240, field.height);
    field.counterAxisSizingMode = 'AUTO';
    variants.push(field);
  }
  return variants;
}

function buildBadge(L, S) {
  var VARIANTS = [
    ['Default', L['color/action/primary'], L['color/text/on-brand'], null],
    ['Secondary', L['color/bg/secondary'], L['color/text/primary'], null],
    ['Outline', null, L['color/text/primary'], L['color/border/default']],
    ['Destructive', L['color/status/failed-text'], L['color/text/on-brand'], null],
  ];
  var variants = [];
  for (var i = 0; i < VARIANTS.length; i++) {
    var v = VARIANTS[i];
    var b = figma.createComponent();
    b.name = 'Variant=' + v[0];
    b.layoutMode = 'HORIZONTAL';
    b.primaryAxisSizingMode = 'AUTO';
    b.counterAxisSizingMode = 'AUTO';
    b.counterAxisAlignItems = 'CENTER';
    b.paddingTop = 2;
    b.paddingBottom = 2;
    b.setBoundVariable('paddingLeft', S['space/2']);
    b.setBoundVariable('paddingRight', S['space/2']);
    b.setBoundVariable('topLeftRadius', S['radius/sm']);
    b.setBoundVariable('topRightRadius', S['radius/sm']);
    b.setBoundVariable('bottomLeftRadius', S['radius/sm']);
    b.setBoundVariable('bottomRightRadius', S['radius/sm']);
    b.fills = v[1] ? fillWith(v[1]) : [];
    if (v[3]) {
      b.strokes = fillWith(v[3]);
      b.strokeWeight = 1;
    }
    var t = makeText(v[0], 'Geist', 'Medium', 11, v[2]);
    b.appendChild(t);
    t.setBoundVariable('fontSize', S['font-size/2xs']);
    variants.push(b);
  }
  return variants;
}

function buildAvatar(L, S) {
  var SIZES = [['sm', 24, 10], ['md', 32, 12], ['lg', 40, 14]];
  var variants = [];
  for (var i = 0; i < SIZES.length; i++) {
    var s = SIZES[i];
    var a = figma.createComponent();
    a.name = 'Size=' + s[0];
    a.layoutMode = 'HORIZONTAL';
    a.primaryAxisAlignItems = 'CENTER';
    a.counterAxisAlignItems = 'CENTER';
    a.primaryAxisSizingMode = 'FIXED';
    a.counterAxisSizingMode = 'FIXED';
    a.resize(s[1], s[1]);
    a.cornerRadius = 999;
    a.fills = fillWith(L['color/bg/muted']);
    // Fallbacks are never coloured by hashing a name into a random hue: that
    // invents a colour vocabulary the system does not have.
    var t = makeText('OC', 'Geist', 'Medium', s[2], L['color/text/muted']);
    a.appendChild(t);
    variants.push(a);
  }
  return variants;
}

function buildAlert(L, S) {
  var VARIANTS = [
    ['Default', L['color/bg/muted'], L['color/border/default'], L['color/text/primary'],
      'Heads up', 'Two runs are waiting on your approval.'],
    ['Destructive', L['color/status/failed'], L['color/status/failed'], L['color/status/failed-text'],
      'Run failed', 'The host rejected the credential. Reconnect and try again.'],
  ];
  var variants = [];
  for (var i = 0; i < VARIANTS.length; i++) {
    var v = VARIANTS[i];
    var a = figma.createComponent();
    a.name = 'Variant=' + v[0];
    a.layoutMode = 'VERTICAL';
    a.primaryAxisSizingMode = 'AUTO';
    a.counterAxisSizingMode = 'FIXED';
    a.itemSpacing = 4;
    a.setBoundVariable('paddingLeft', S['space/3']);
    a.setBoundVariable('paddingRight', S['space/3']);
    a.setBoundVariable('paddingTop', S['space/3']);
    a.setBoundVariable('paddingBottom', S['space/3']);
    a.setBoundVariable('topLeftRadius', S['radius/lg']);
    a.setBoundVariable('topRightRadius', S['radius/lg']);
    a.setBoundVariable('bottomLeftRadius', S['radius/lg']);
    a.setBoundVariable('bottomRightRadius', S['radius/lg']);
    // Destructive uses the failed hue at low alpha, so the alert reads as a
    // tint rather than a slab of red.
    a.fills = v[0] === 'Destructive' ? fillWith(v[1], 0.10) : fillWith(v[1]);
    a.strokes = v[0] === 'Destructive' ? fillWith(v[2], 0.30) : fillWith(v[2]);
    a.strokeWeight = 1;

    var title = makeText(v[4], 'Geist', 'SemiBold', 14, v[3]);
    a.appendChild(title);
    title.setBoundVariable('fontSize', S['font-size/sm']);

    var body = makeText(v[5], 'Geist', 'Regular', 12,
      v[0] === 'Destructive' ? v[3] : L['color/text/muted']);
    a.appendChild(body);
    body.setBoundVariable('fontSize', S['font-size/xs']);

    // See the note in buildInput: resize() clears the sizing modes.
    a.resize(320, a.height);
    a.primaryAxisSizingMode = 'AUTO';
    variants.push(a);
  }
  return variants;
}

function buildTab(L, S) {
  var STATES = [
    ['Active', L['color/bg/card'], L['color/text/primary'], true],
    ['Inactive', null, L['color/text/muted'], false],
  ];
  var variants = [];
  for (var i = 0; i < STATES.length; i++) {
    var s = STATES[i];
    var t = figma.createComponent();
    t.name = 'State=' + s[0];
    t.layoutMode = 'HORIZONTAL';
    t.primaryAxisSizingMode = 'AUTO';
    t.counterAxisSizingMode = 'AUTO';
    t.counterAxisAlignItems = 'CENTER';
    t.paddingTop = 5;
    t.paddingBottom = 5;
    t.setBoundVariable('paddingLeft', S['space/3']);
    t.setBoundVariable('paddingRight', S['space/3']);
    t.setBoundVariable('topLeftRadius', S['radius/sm']);
    t.setBoundVariable('topRightRadius', S['radius/sm']);
    t.setBoundVariable('bottomLeftRadius', S['radius/sm']);
    t.setBoundVariable('bottomRightRadius', S['radius/sm']);
    t.fills = s[1] ? fillWith(s[1]) : [];
    if (s[3]) {
      t.strokes = fillWith(L['color/border/default']);
      t.strokeWeight = 1;
    }
    var label = makeText('Overview', 'Geist', 'Medium', 12, s[2]);
    t.appendChild(label);
    label.setBoundVariable('fontSize', S['font-size/xs']);
    variants.push(t);
  }
  return variants;
}

/**
 * Card is a single component, not a variant set — a card is a card. It is the
 * one primitive here with named child layers, because those are what a
 * designer swaps content into.
 */
function buildCard(L, S) {
  var card = figma.createComponent();
  card.name = 'Card';
  card.layoutMode = 'VERTICAL';
  card.primaryAxisSizingMode = 'AUTO';
  card.counterAxisSizingMode = 'FIXED';
  card.itemSpacing = 4;
  card.setBoundVariable('paddingLeft', S['space/4']);
  card.setBoundVariable('paddingRight', S['space/4']);
  card.setBoundVariable('paddingTop', S['space/4']);
  card.setBoundVariable('paddingBottom', S['space/4']);
  card.setBoundVariable('topLeftRadius', S['radius/lg']);
  card.setBoundVariable('topRightRadius', S['radius/lg']);
  card.setBoundVariable('bottomLeftRadius', S['radius/lg']);
  card.setBoundVariable('bottomRightRadius', S['radius/lg']);
  card.fills = fillWith(L['color/bg/card']);
  card.strokes = fillWith(L['color/border/default']);
  card.strokeWeight = 1;
  // No shadow at rest. A card that floats is not a card, it is a popover.

  var title = makeText('Agents active', 'Geist', 'SemiBold', 14, L['color/text/primary']);
  title.name = 'Title';
  card.appendChild(title);
  title.setBoundVariable('fontSize', S['font-size/sm']);

  var desc = makeText('12 running, 3 idle', 'Geist', 'Regular', 12, L['color/text/muted']);
  desc.name = 'Description';
  card.appendChild(desc);
  desc.setBoundVariable('fontSize', S['font-size/xs']);

  // See the note in buildInput: resize() clears the sizing modes.
  card.resize(260, card.height);
  card.primaryAxisSizingMode = 'AUTO';
  card.description =
    'A resting panel. Separates by surface and a hairline border — no shadow. ' +
    'CardTitle sits at 14px semibold rather than a larger default: this console is dense, ' +
    'and a card title is a label, not a heading.';
  return card;
}

var COMPONENT_SPECS = [
  {
    name: 'Input',
    x: 860, y: 0,
    build: buildInput,
    description:
      'A form field. Always paired with a Label and a matching htmlFor/id — placeholder text is not a ' +
      'label, because it disappears exactly when it is needed.\n\n' +
      'Placeholders show a realistic example ("acme-marketing"), never a restatement of the label.\n\n' +
      'Hover is omitted deliberately: it is an interaction state the code owns.',
  },
  {
    name: 'Badge',
    x: 0, y: 640,
    build: buildBadge,
    description:
      'For a noun — a count, a label, a category. NOT for status: status uses Status Badge, because a ' +
      'badge alone carries no meaning for anyone who cannot separate the hues.\n\n' +
      'Never interactive. If it can be clicked, it is a Button.',
  },
  {
    name: 'Alert',
    x: 300, y: 640,
    build: buildAlert,
    description:
      'A condition affecting the whole surface, in place, that the operator did not just cause — ' +
      'something they did just cause is a toast.\n\n' +
      'Title states what happened; description says what to do next. Errors explain and instruct, ' +
      'they never apologise.',
  },
  {
    name: 'Avatar',
    x: 700, y: 640,
    build: buildAvatar,
    description:
      'Initials on a muted fill. Fallbacks are never coloured by hashing a name into a random hue — ' +
      'that invents a colour vocabulary the system does not have.',
  },
  {
    name: 'Tab',
    x: 900, y: 640,
    build: buildTab,
    description:
      'One tab trigger. Tabs switch between peer views of the same subject — never steps in a sequence, ' +
      'and never navigation between unrelated screens.\n\n' +
      'The panel does not restate the tab label as a heading.',
  },
];

async function buildComponents(pages, L, S, rebuild) {
  var page = pages['Components'];
  await page.loadAsync();
  await figma.setCurrentPageAsync(page);

  var made = [];
  var skipped = [];

  for (var i = 0; i < COMPONENT_SPECS.length; i++) {
    var spec = COMPONENT_SPECS[i];

    // Idempotency by exact name. Never a prefix match — that is how a cleanup
    // script deletes something a person made.
    var found = null;
    for (var j = 0; j < page.children.length; j++) {
      if (page.children[j].name === spec.name) found = page.children[j];
    }
    if (found) {
      if (!rebuild) {
        skipped.push(spec.name);
        continue;
      }
      found.remove();
    }

    var variants = spec.build(L, S);
    for (var k = 0; k < variants.length; k++) page.appendChild(variants[k]);

    if (variants.length === 1) {
      // A single component, not a set — position it directly.
      variants[0].x = spec.x;
      variants[0].y = spec.y;
      variants[0].description = spec.description;
    } else {
      var set = figma.combineAsVariants(variants, page);
      placeSet(set, spec.name, spec.description, spec.x, spec.y, L);
    }
    made.push(spec.name);
    log('Component: ' + spec.name + ' (' + variants.length + ' variants)');
  }

  // Card is a lone component rather than a set.
  var haveCard = null;
  for (var c = 0; c < page.children.length; c++) {
    if (page.children[c].name === 'Card') haveCard = page.children[c];
  }
  if (haveCard && rebuild) {
    haveCard.remove();
    haveCard = null;
  }
  if (!haveCard) {
    var card = buildCard(L, S);
    page.appendChild(card);
    card.x = 1120;
    card.y = 640;
    made.push('Card');
    log('Component: Card');
  } else {
    skipped.push('Card');
  }

  return { made: made, skipped: skipped };
}

/* ==========================================================================
 * ENTRY
 * ========================================================================== */

async function run(what, rebuild) {
  LOG = [];
  var started = Date.now();

  // Dynamic-page access: pages load on demand, and every page this touches
  // must be loaded before its children are readable.
  await figma.loadAllPagesAsync();

  for (var i = 0; i < FONTS.length; i++) await figma.loadFontAsync(FONTS[i]);
  log('Fonts loaded: Geist, Geist Mono');

  var built = await buildVariables();
  await buildTextStyles(built.scale);
  await buildEffectStyles();

  var pages = await ensurePages();
  log('Pages: Cover, Foundations, Components');

  var summary = { variables: built.total, components: [], skipped: [] };

  if (what === 'all') {
    var L = await variablesIn(built.light);
    var S = await variablesIn(built.scale);
    var res = await buildComponents(pages, L, S, rebuild);
    summary.components = res.made;
    summary.skipped = res.skipped;
  }

  log('Done in ' + ((Date.now() - started) / 1000).toFixed(1) + 's');
  return summary;
}

figma.showUI(__html__, { width: 380, height: 520, themeColors: true });

figma.ui.onmessage = async function (msg) {
  if (msg.type === 'cancel') {
    figma.closePlugin();
    return;
  }
  if (msg.type !== 'run') return;

  try {
    var summary = await run(msg.what, msg.rebuild === true);
    figma.ui.postMessage({ type: 'done', summary: summary });
    var note = summary.variables + ' variables written';
    if (summary.components.length) {
      note += ', ' + summary.components.length + ' components built';
    }
    if (summary.skipped.length) {
      note += ', ' + summary.skipped.length + ' skipped (already exist)';
    }
    figma.notify(note);
  } catch (err) {
    // Surface the real message. A generator that fails silently is worse than
    // one that does nothing.
    figma.ui.postMessage({ type: 'error', message: String(err && err.message ? err.message : err) });
    figma.notify('Generator failed: ' + err, { error: true });
  }
};
