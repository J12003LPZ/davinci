const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

// Exercise the template's pure data layer without a browser or duplicated logic.
const source = fs.readFileSync(path.join(__dirname, 'template.js'), 'utf8');
const boundary = source.indexOf('      // TREE DISPLAY TEXT (pure data -> string)');
assert.ok(boundary > 0, 'template data layer boundary must exist');

function fixture(entries, leafId = entries.at(-1)?.id) {
  const data = Buffer.from(JSON.stringify({ header: {}, entries, leafId })).toString('base64');
  const context = vm.createContext({
    document: {
      getElementById: () => ({ textContent: data }),
      querySelector: () => null,
    },
    window: { location: { search: '' } },
    URLSearchParams, atob, TextDecoder,
  });
  vm.runInContext(`${source.slice(0, boundary)}
    globalThis.tree = { buildTree, buildActivePathIds, getPath, findNewestLeaf,
      flattenTree, filterNodes };
  })();`, context);
  return expression => vm.runInContext(expression, context, { timeout: 2000 });
}

function entry(id, parentId = null, timestamp = 0) {
  return { id, parentId, timestamp, type: 'message', message: { role: 'user', content: id } };
}

test('branch order, newest leaf and filtered indentation stay compatible', () => {
  const run = fixture([
    entry('root'), entry('late', 'root', 20), entry('early', 'root', 10),
    entry('tip', 'early', 30), entry('orphan', 'missing', 40),
  ], 'tip');
  assert.deepEqual(Array.from(run("tree.getPath('tip').map(e => e.id)")), ['root', 'early', 'tip']);
  assert.equal(run("tree.findNewestLeaf('root')"), 'late');
  assert.deepEqual(Array.from(run("tree.flattenTree(tree.buildTree(), tree.buildActivePathIds('tip')).map(n => n.node.entry.id)")),
    ['root', 'early', 'tip', 'late', 'orphan']);
  assert.equal(run("tree.filterNodes(tree.flattenTree(tree.buildTree(), new Set()), 'tip').length"), 5);
});

test('a long linear session renders without recursive stack exhaustion', () => {
  const entries = Array.from({ length: 20000 }, (_, i) => entry(String(i), i ? String(i - 1) : null));
  const run = fixture(entries);
  assert.equal(run('tree.buildTree().length'), 1);
  assert.equal(run("tree.findNewestLeaf('0')"), '19999');
  assert.equal(run("tree.getPath('19999').length"), entries.length);
  assert.equal(run("tree.filterNodes(tree.flattenTree(tree.buildTree(), tree.buildActivePathIds('19999')), '19999').length"), entries.length);
});

test('cyclic parent links terminate and keep each reachable entry once', () => {
  const run = fixture([entry('a', 'b'), entry('b', 'a'), entry('tail', 'a')]);
  assert.deepEqual(Array.from(run("tree.getPath('tail').map(e => e.id)")), ['b', 'a', 'tail']);
  assert.deepEqual(Array.from(run("tree.buildActivePathIds('tail')")), ['tail', 'a', 'b']);
});

test('hidden self-parented roots do not trap filtered descendant traversal', () => {
  const run = fixture([
    { id: 'root', parentId: 'root', timestamp: 0, type: 'model_change' },
    entry('child', 'root'),
  ]);
  assert.deepEqual(Array.from(run("tree.filterNodes(tree.flattenTree(tree.buildTree(), new Set()), 'child').map(n => n.node.entry.id)")), ['child']);
});
