'use strict';

// Run with: node --test
//
// The extension driven end to end against a stubbed editor API (test/vscode-stub.js)
// and the real DC-10 examples, which stand in for a workspace.

const test = require('node:test');
const assert = require('node:assert');
const path = require('node:path');
const fs = require('node:fs');

const { makeVscode, loadExtension, readWorkspace } = require('./vscode-stub');

const EXAMPLES = path.join(__dirname, '..', '..', '..', 'examples', 'dc10');
const EXTENSION = path.join(__dirname, '..');

const HEAD = 'procedure P {\n    name "P"\n    category normal\n';

async function activated() {
  const { vscode, registered, makeDocument } = makeVscode(readWorkspace(EXAMPLES));
  const extension = loadExtension(vscode);
  const api = extension.activate({ subscriptions: [], extensionPath: EXTENSION });
  await api.ready;

  const items = source => {
    const document = makeDocument(source);
    return registered.completion.provideCompletionItems(document, document.endPosition());
  };

  /** What is offered with the cursor at the end of `source`, in the order shown. */
  const complete = source =>
    items(source)
      .slice()
      .sort((a, b) => (a.sortText < b.sortText ? -1 : a.sortText > b.sortText ? 1 : 0))
      .map(item => item.label);

  return { registered, makeDocument, complete, items };
}

test('completion follows the cursor', async () => {
  const { complete } = await activated();

  assert.deepStrictEqual(complete(''), ['procedure']);

  assert.deepStrictEqual(complete('procedure P {\n    category ').sort(), [
    'abnormal',
    'emergency',
    'normal',
    'reference',
  ]);

  // Metadata first, but the steps are there for a procedure that goes straight
  // to work.
  const metadata = complete('procedure P {\n    ');
  assert.strictEqual(metadata[0], 'name');
  assert.ok(metadata.includes('check'));

  // Once a step has been written, metadata is no longer legal and not offered.
  const steps = complete(`${HEAD}    check X\n    `);
  assert.ok(steps.includes('check'));
  assert.strictEqual(steps.includes('category'), false);

  assert.deepStrictEqual(complete(`${HEAD}    wait fuel.x > 1 timeout `), [
    '500ms',
    '1s',
    '5s',
    '10s',
    '30s',
    '1m',
    '5m',
  ]);

  // Nothing at all inside a message.
  assert.deepStrictEqual(complete(`${HEAD}    notify "hold `), []);
});

test('completion offers names from the whole workspace', async () => {
  const { complete } = await activated();

  assert.ok(complete(`${HEAD}    open `).includes('FUEL_CROSSFEED_VALVE'), 'a control');
  assert.ok(complete(`${HEAD}    call `).includes('ELEC_BUS_2_RESTORE'), 'a procedure');
  assert.ok(complete(`${HEAD}    wait `).includes('hydraulic.2.pressure'), 'a state path');

  // Controls are not state, and state is not a control. Confusing the two is
  // the mistake this language is shaped to prevent.
  assert.strictEqual(complete(`${HEAD}    wait `).includes('FUEL_CROSSFEED_VALVE'), false);
  assert.strictEqual(complete(`${HEAD}    open `).includes('hydraulic.2.pressure'), false);

  // Positions the workspace uses come before the ones we guessed at.
  const positions = complete(`${HEAD}    set FUEL_XFEED_SELECTOR = `);
  assert.ok(positions.includes('TANK_3_TO_1'));
  assert.ok(positions.indexOf('TANK_3_TO_1') < positions.indexOf('AUTO'));
});

test('a name written a moment ago is offered on the next line', async () => {
  const { complete } = await activated();

  const source = `${HEAD}    check BRAND_NEW_CONTROL\n    open `;
  assert.ok(complete(source).includes('BRAND_NEW_CONTROL'), 'the open file is re-read, not cached');
});

test('a step completion inserts a statement, not just a word', async () => {
  const { registered, makeDocument } = await activated();

  const document = makeDocument(`${HEAD}    `);
  const items = registered.completion.provideCompletionItems(document, document.endPosition());
  const set = items.find(item => item.label === 'set');

  assert.strictEqual(set.insertText.value, 'set ${1:CONTROL} = ${2:ON}');
  assert.match(set.documentation.value, /set CONTROL = ON/);
});

test('the outline lists procedures by identifier, titled by name', async () => {
  const { registered, makeDocument } = await activated();

  const source = fs.readFileSync(path.join(EXAMPLES, 'pressurization.fe'), 'utf8');
  const symbols = registered.symbols.provideDocumentSymbols(makeDocument(source));

  assert.deepStrictEqual(
    symbols.map(s => [s.name, s.detail]),
    [
      ['CABIN_RAPID_DEPRESSURIZATION', 'Cabin Rapid Depressurization'],
      ['CABIN_EMERGENCY_DESCENT_SUPPORT', "Emergency Descent - Engineer's Panel"],
    ],
  );
  // The range covers the whole procedure, closing brace included.
  const first = symbols[0];
  assert.ok(first.range.end.line > first.range.start.line);
  assert.strictEqual(source.split('\n')[first.range.end.line].trim(), '}');
});

test('go to definition follows a call across files', async () => {
  const { registered, makeDocument } = await activated();

  const source = fs.readFileSync(path.join(EXAMPLES, 'hydraulic.fe'), 'utf8');
  const document = makeDocument(source, path.join(EXAMPLES, 'hydraulic.fe'));
  const lines = source.split('\n');
  const line = lines.findIndex(l => l.trim().startsWith('call ELEC_BUS_2_RESTORE'));
  assert.notStrictEqual(line, -1, 'the example should still call across files');

  const definition = await registered.definition.provideDefinition(document, {
    line,
    character: lines[line].indexOf('ELEC_BUS_2_RESTORE') + 2,
  });

  assert.ok(definition, 'the call target resolved');
  assert.match(definition.uri.toString(), /electrical\.fe$/);
  const target = fs.readFileSync(path.join(EXAMPLES, 'electrical.fe'), 'utf8').split('\n');
  assert.match(target[definition.position.line], /^procedure ELEC_BUS_2_RESTORE/);
});
