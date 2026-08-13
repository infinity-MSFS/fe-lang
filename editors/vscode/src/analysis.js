'use strict';

const fs = require('fs');
const path = require('path');

// Everything in this file is deliberately free of `vscode` imports so that it
// can be run — and tested — with plain node. `extension.js` is the only part
// that touches the editor API.
//
// None of this is a substitute for `fe-compiler`. There is no type checking, no
// symbol registry and no idea whether a control exists; the completion list is
// assembled from what the .fe files in the workspace already say. The compiler
// remains the only thing that decides whether a procedure is legal.

const CATEGORIES = ['normal', 'abnormal', 'emergency', 'reference'];

// Positions every aircraft tends to register, offered when the file itself has
// not yet used anything better.
const COMMON_POSITIONS = ['ON', 'OFF', 'OPEN', 'CLOSED', 'AUTO', 'NORMAL', 'HIGH', 'LOW'];

const COMMON_DURATIONS = ['500ms', '1s', '5s', '10s', '30s', '1m', '5m'];

const METADATA_KEYWORDS = [
  'name',
  'description',
  'category',
  'priority',
  'revision',
  'trigger',
  'require',
];

const STEP_KEYWORDS = [
  'check',
  'set',
  'start',
  'stop',
  'open',
  'close',
  'notify',
  'call',
  'wait',
  'if',
  'complete',
  'fail',
];

const CONTROL_VERBS = ['check', 'set', 'start', 'stop', 'open', 'close'];

/**
 * Blank out comments and string contents, keeping every offset and newline
 * where it was, so that brace counting and keyword matching cannot be fooled by
 * a `{` inside a `notify` message.
 *
 * Also reports whether the text *ends* inside a string or a comment, which is
 * how the completion provider knows to stay quiet.
 */
function stripTrivia(source) {
  const out = new Array(source.length);
  let i = 0;
  let inString = false;
  let inLineComment = false;
  let inBlockComment = false;

  while (i < source.length) {
    const c = source[i];
    const next = source[i + 1];

    if (inLineComment) {
      if (c === '\n') {
        inLineComment = false;
        out[i] = '\n';
      } else {
        out[i] = ' ';
      }
      i += 1;
      continue;
    }

    if (inBlockComment) {
      if (c === '*' && next === '/') {
        inBlockComment = false;
        out[i] = ' ';
        out[i + 1] = ' ';
        i += 2;
        continue;
      }
      out[i] = c === '\n' ? '\n' : ' ';
      i += 1;
      continue;
    }

    if (inString) {
      if (c === '\\' && next !== undefined && next !== '\n') {
        out[i] = ' ';
        out[i + 1] = ' ';
        i += 2;
        continue;
      }
      if (c === '"') {
        inString = false;
        out[i] = ' ';
        i += 1;
        continue;
      }
      if (c === '\n') {
        // The lexer does not let a string literal span lines, so neither do we.
        inString = false;
        out[i] = '\n';
        i += 1;
        continue;
      }
      out[i] = ' ';
      i += 1;
      continue;
    }

    if (c === '/' && next === '/') {
      inLineComment = true;
      out[i] = ' ';
      i += 1;
      continue;
    }
    if (c === '/' && next === '*') {
      inBlockComment = true;
      out[i] = ' ';
      i += 1;
      continue;
    }
    if (c === '"') {
      inString = true;
      out[i] = ' ';
      i += 1;
      continue;
    }

    out[i] = c;
    i += 1;
  }

  return { text: out.join(''), inString, inComment: inLineComment || inBlockComment };
}

/** Offset of the `{` that opens the procedure we are inside, or -1. */
function openProcedureBrace(text) {
  let depth = 0;
  let start = -1;
  for (let i = 0; i < text.length; i += 1) {
    if (text[i] === '{') {
      if (depth === 0) start = i;
      depth += 1;
    } else if (text[i] === '}') {
      depth -= 1;
      if (depth <= 0) {
        depth = 0;
        start = -1;
      }
    }
  }
  return start;
}

function braceDepth(text) {
  let depth = 0;
  for (let i = 0; i < text.length; i += 1) {
    if (text[i] === '{') depth += 1;
    else if (text[i] === '}') depth = Math.max(0, depth - 1);
  }
  return depth;
}

const STEP_START = new RegExp(`^\\s*(?:${STEP_KEYWORDS.join('|')})\\b`, 'm');

/**
 * Offset just past the `}` that closes the procedure declared at `from`. An
 * unclosed procedure — one still being typed — ends at the end of the text.
 */
function procedureEnd(text, from) {
  const open = text.indexOf('{', from);
  if (open < 0) return text.length;
  let depth = 0;
  for (let i = open; i < text.length; i += 1) {
    if (text[i] === '{') depth += 1;
    else if (text[i] === '}') {
      depth -= 1;
      if (depth === 0) return i + 1;
    }
  }
  return text.length;
}

/**
 * Work out what the cursor is in the middle of writing, given everything in the
 * document before it.
 *
 * Returns one of:
 *   none        inside a string or a comment — suggest nothing
 *   top         between procedures
 *   metadata    inside a procedure that has not started its steps yet
 *   step        inside a procedure body or an `if` block
 *   category    after `category`
 *   position    after `set CONTROL =`
 *   duration    after `timeout`
 *   control     after a verb that names a control
 *   procedure   after `call`
 *   expression  inside a condition
 */
function contextAt(before) {
  const { text, inString, inComment } = stripTrivia(before);
  if (inString || inComment) return { kind: 'none' };

  const line = text.slice(text.lastIndexOf('\n') + 1);

  if (/\bcategory\s+[A-Za-z_][A-Za-z0-9_]*$|\bcategory\s+$/.test(line)) {
    return { kind: 'category' };
  }
  if (/\bset\s+[A-Za-z_][A-Za-z0-9_.]*\s*=\s*[A-Za-z0-9_.]*$/.test(line)) {
    return { kind: 'position' };
  }
  if (/\btimeout\s+[A-Za-z0-9_.]*$/.test(line)) {
    return { kind: 'duration' };
  }
  if (/\bcall\s+[A-Za-z0-9_]*$/.test(line)) {
    return { kind: 'procedure' };
  }
  if (new RegExp(`\\b(?:${CONTROL_VERBS.join('|')})\\s+[A-Za-z0-9_.]*$`).test(line)) {
    return { kind: 'control' };
  }

  // A condition runs from `trigger`/`require`/`wait`/`if`/`when` to the end of
  // the line — unless an `{` has already closed it off, in which case we are in
  // the block that follows.
  const conditionKeyword = /\b(?:trigger|require|wait|if|when)\b/g;
  let conditionStart = -1;
  let match;
  while ((match = conditionKeyword.exec(line)) !== null) conditionStart = match.index;
  if (conditionStart >= 0 && !line.slice(conditionStart).includes('{')) {
    return { kind: 'expression' };
  }

  if (braceDepth(text) === 0) return { kind: 'top' };

  const bodyStart = openProcedureBrace(text);
  const body = bodyStart >= 0 ? text.slice(bodyStart + 1) : '';
  const started = braceDepth(text) > 1 || STEP_START.test(body);
  return { kind: started ? 'step' : 'metadata' };
}

/**
 * Harvest the names a .fe file already uses. Comments and string contents are
 * blanked first, so a control named in a `notify` message is not mistaken for a
 * control that is actually moved.
 */
function indexSource(source) {
  const { text } = stripTrivia(source);

  const procedures = [];
  const seenProcedures = new Set();
  const procedureDecl = /\bprocedure\s+([A-Za-z_][A-Za-z0-9_]*)/g;
  let match;
  while ((match = procedureDecl.exec(text)) !== null) {
    if (seenProcedures.has(match[1])) continue;
    seenProcedures.add(match[1]);
    // The crew-facing title comes from the raw source: `stripTrivia` blanked it.
    const window = source.slice(match.index, match.index + 600);
    const title = /\bname\s+"((?:[^"\\\n]|\\.)*)"/.exec(window);
    procedures.push({
      name: match[1],
      title: title ? title[1] : '',
      index: match.index,
      end: procedureEnd(text, match.index),
    });
  }

  const controls = new Set();
  const controlUse = new RegExp(
    `(?:^|[\\s{}])(?:${CONTROL_VERBS.join('|')})\\s+([A-Za-z_][A-Za-z0-9_.]*)`,
    'gm',
  );
  while ((match = controlUse.exec(text)) !== null) controls.add(match[1]);

  const positions = new Set();
  const positionUse = /\bset\s+[A-Za-z_][A-Za-z0-9_.]*\s*=\s*([A-Za-z_][A-Za-z0-9_]*)/g;
  while ((match = positionUse.exec(text)) !== null) positions.add(match[1]);

  // State symbols: dotted, and lower case by convention — `hydraulic.2.pressure`
  // rather than `HYD_2_ELECTRIC_PUMP`.
  const paths = new Set();
  const pathUse = /\b[a-z_][A-Za-z0-9_]*(?:\.[A-Za-z0-9_]+)+/g;
  while ((match = pathUse.exec(text)) !== null) {
    if (!controls.has(match[0])) paths.add(match[0]);
  }

  return {
    procedures,
    controls: [...controls].sort(),
    positions: [...positions].sort(),
    paths: [...paths].sort(),
  };
}

/** A snippet body with its placeholders resolved, for the documentation popup. */
function preview(body) {
  return body
    .replace(/\$\{\d+\|([^|]*)\|\}/g, (_, choices) => choices.split(',')[0])
    .replace(/\$\{\d+:([^}]*)\}/g, '$1')
    .replace(/\$\{\d+\}/g, '')
    .replace(/\$\d+/g, '');
}

/**
 * Read `snippets/fe.json` and work out where each snippet belongs.
 *
 * They are served through the completion provider rather than
 * `contributes.snippets` so that they can be filtered by where the cursor is —
 * `category` is not a step, and `check` is not metadata. Contributing them as
 * well would offer every one of them twice.
 */
function loadSnippets(extensionPath) {
  const file = path.join(extensionPath, 'snippets', 'fe.json');
  const parsed = JSON.parse(fs.readFileSync(file, 'utf8'));

  return Object.values(parsed).map(entry => {
    const body = Array.isArray(entry.body) ? entry.body.join('\n') : entry.body;
    let context = 'step';
    if (entry.prefix === 'procedure') context = 'top';
    else if (METADATA_KEYWORDS.includes(entry.prefix)) context = 'metadata';
    return { prefix: entry.prefix, body, description: entry.description || '', context };
  });
}

module.exports = {
  CATEGORIES,
  COMMON_POSITIONS,
  COMMON_DURATIONS,
  METADATA_KEYWORDS,
  STEP_KEYWORDS,
  CONTROL_VERBS,
  stripTrivia,
  contextAt,
  indexSource,
  loadSnippets,
  preview,
};
