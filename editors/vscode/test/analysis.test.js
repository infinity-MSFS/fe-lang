'use strict';

// Run with: node --test
//
// These cover the part of the extension that has opinions. Everything else is
// wiring to the editor API, which cannot be tested without one.

const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');

const {
  STEP_KEYWORDS,
  METADATA_KEYWORDS,
  contextAt,
  indexSource,
  loadSnippets,
  preview,
  stripTrivia,
} = require('../src/analysis');

const EXAMPLES = path.join(__dirname, '..', '..', '..', 'examples', 'dc10');

/** The text of a procedure up to `|`, which marks the cursor. */
function before(source) {
  const cursor = source.indexOf('|');
  assert.notStrictEqual(cursor, -1, 'the fixture must mark the cursor with |');
  return source.slice(0, cursor);
}

test('stripTrivia keeps offsets and hides comments and strings', () => {
  const source = 'notify "a { b" // { c\nfail';
  const { text } = stripTrivia(source);

  assert.strictEqual(text.length, source.length);
  assert.strictEqual(text.includes('{'), false, 'braces inside trivia must not count');
  assert.strictEqual(text.startsWith('notify'), true);
  assert.strictEqual(text.endsWith('fail'), true);
});

test('stripTrivia reports a cursor inside a string or comment', () => {
  assert.strictEqual(stripTrivia('notify "half a mess').inString, true);
  assert.strictEqual(stripTrivia('notify "done"').inString, false);
  assert.strictEqual(stripTrivia('// thinking').inComment, true);
  assert.strictEqual(stripTrivia('/* thinking */').inComment, false);
  assert.strictEqual(stripTrivia('// thinking\n').inComment, false);
});

test('a string may not span lines, as in the lexer', () => {
  assert.strictEqual(stripTrivia('notify "unterminated\ncheck X').inString, false);
});

const CONTEXTS = [
  ['between procedures', '|', 'top'],
  ['after a procedure', 'procedure A {\n  name "x"\n}\n|', 'top'],
  ['metadata zone', 'procedure A {\n  |', 'metadata'],
  ['metadata zone after metadata', 'procedure A {\n  name "x"\n  |', 'metadata'],
  ['body after a step', 'procedure A {\n  name "x"\n  check X\n  |', 'step'],
  ['inside an if block', 'procedure A {\n  if a.b {\n    |', 'step'],
  ['after category', 'procedure A {\n  category |', 'category'],
  ['halfway through a category', 'procedure A {\n  category abn|', 'category'],
  ['after check', 'procedure A {\n  check |', 'control'],
  ['halfway through a control', 'procedure A {\n  check FUEL_|', 'control'],
  ['after open', 'procedure A {\n  open |', 'control'],
  ['after set', 'procedure A {\n  set |', 'control'],
  ['after set =', 'procedure A {\n  set X = |', 'position'],
  ['halfway through a position', 'procedure A {\n  set X = O|', 'position'],
  ['after call', 'procedure A {\n  call |', 'procedure'],
  ['after timeout', 'procedure A {\n  wait a.b > 1 timeout |', 'duration'],
  ['a trigger condition', 'procedure A {\n  trigger |', 'expression'],
  ['a require condition', 'procedure A {\n  require !|', 'expression'],
  ['a wait condition', 'procedure A {\n  wait fuel.|', 'expression'],
  ['after an operator', 'procedure A {\n  wait fuel.x > 1 && |', 'expression'],
  ['an if condition', 'procedure A {\n  if |', 'expression'],
  ['a complete criterion', 'procedure A {\n  complete when |', 'expression'],
  ['past the brace of an if', 'procedure A {\n  if a.b {\n    check X\n    |', 'step'],
  ['inside a string', 'procedure A {\n  notify "hold |', 'none'],
  ['inside a comment', 'procedure A {\n  // hold |', 'none'],
];

for (const [what, fixture, expected] of CONTEXTS) {
  test(`context: ${what}`, () => {
    assert.strictEqual(contextAt(before(`${fixture}|`)).kind, expected);
  });
}

test('indexSource finds procedures with their crew-facing title', () => {
  const source = [
    'procedure HYD_2_LOW_PRESSURE {',
    '    name "Hydraulic System 2 Low Pressure"',
    '    category abnormal',
    '}',
    '',
    'procedure ELEC_BUS_2_RESTORE {',
    '    name "AC Bus 2 Restore"',
    '    category abnormal',
    '}',
  ].join('\n');

  const { procedures } = indexSource(source);

  assert.deepStrictEqual(
    procedures.map(p => [p.name, p.title]),
    [
      ['HYD_2_LOW_PRESSURE', 'Hydraulic System 2 Low Pressure'],
      ['ELEC_BUS_2_RESTORE', 'AC Bus 2 Restore'],
    ],
  );
  assert.strictEqual(source.slice(procedures[0].index, procedures[0].end).endsWith('}'), true);
});

test('indexSource separates controls from state, and ignores talk about them', () => {
  const source = [
    'procedure A {',
    '    name "A"',
    '    category normal',
    '    trigger hydraulic.2.pressure < 1800',
    '    check HYD_2_ENGINE_PUMP',
    '    set FUEL_XFEED_SELECTOR = TANK_3_TO_1',
    '    open FUEL_CROSSFEED_VALVE',
    '    notify "check IMAGINARY_CONTROL and imaginary.state"',
    '    // check COMMENTED_CONTROL',
    '}',
  ].join('\n');

  const { controls, positions, paths } = indexSource(source);

  assert.deepStrictEqual(controls, [
    'FUEL_CROSSFEED_VALVE',
    'FUEL_XFEED_SELECTOR',
    'HYD_2_ENGINE_PUMP',
  ]);
  assert.deepStrictEqual(positions, ['TANK_3_TO_1']);
  assert.deepStrictEqual(paths, ['hydraulic.2.pressure']);
});

test('every keyword in the language is offered somewhere', () => {
  const snippets = loadSnippets(path.join(__dirname, '..'));
  const prefixes = snippets.map(s => s.prefix);

  for (const keyword of [...METADATA_KEYWORDS, ...STEP_KEYWORDS, 'procedure']) {
    assert.ok(prefixes.includes(keyword), `nothing completes \`${keyword}\``);
  }

  assert.strictEqual(new Set(prefixes).size, prefixes.length, 'two snippets share a prefix');

  const contexts = {};
  for (const snippet of snippets) contexts[snippet.prefix] = snippet.context;
  assert.strictEqual(contexts.procedure, 'top');
  assert.strictEqual(contexts.category, 'metadata');
  assert.strictEqual(contexts.check, 'step');
  assert.strictEqual(contexts.completew, 'step');
});

test('snippet previews read as the language, not as placeholders', () => {
  assert.strictEqual(preview('set ${1:CONTROL} = ${2:ON}'), 'set CONTROL = ON');
  assert.strictEqual(preview('category ${1|abnormal,normal|}'), 'category abnormal');
  assert.strictEqual(preview('if ${1:condition} {\n\t$0\n}'), 'if condition {\n\t\n}');
});

test('the DC-10 examples index the way the language reference says they should', () => {
  const merged = { procedures: [], controls: new Set(), paths: new Set() };

  for (const file of fs.readdirSync(EXAMPLES).filter(f => f.endsWith('.fe'))) {
    const entry = indexSource(fs.readFileSync(path.join(EXAMPLES, file), 'utf8'));
    merged.procedures.push(...entry.procedures.map(p => p.name));
    entry.controls.forEach(c => merged.controls.add(c));
    entry.paths.forEach(p => merged.paths.add(p));
  }

  assert.ok(merged.procedures.includes('HYD_2_LOW_PRESSURE'));
  assert.ok(merged.procedures.includes('CABIN_RAPID_DEPRESSURIZATION'));
  assert.ok(merged.controls.has('FUEL_CROSSFEED_VALVE'));
  assert.ok(merged.paths.has('hydraulic.2.pressure'));

  // A control is never offered as though it were readable state.
  for (const control of merged.controls) {
    assert.strictEqual(merged.paths.has(control), false);
  }
  // Every procedure a `call` names exists — the compiler enforces this, and the
  // examples should not be teaching otherwise.
  for (const file of fs.readdirSync(EXAMPLES).filter(f => f.endsWith('.fe'))) {
    const source = fs.readFileSync(path.join(EXAMPLES, file), 'utf8');
    for (const [, target] of source.matchAll(/\bcall\s+([A-Za-z_][A-Za-z0-9_]*)/g)) {
      assert.ok(merged.procedures.includes(target), `${target} is called but never declared`);
    }
  }
});
