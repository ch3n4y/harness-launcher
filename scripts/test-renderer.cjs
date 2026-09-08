const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const html = fs.readFileSync('src/index.html', 'utf8');
const elements = new Map([...html.matchAll(/id="([^"]+)"/g)].map(([, id]) => [id, {
  value: '', checked: false, dataset: {}, textContent: '', hidden: false,
  setAttribute() {}, replaceChildren(...options) { this.options = options; },
}]));
const calls = [];
const nodePath = '/Users/test/.nvm/versions/node/v24.19.0/bin/node';
const state = {status: 'stopped', runtime: {node: nodePath, version: '24.19.0', entry: 'dsh'}, candidates: [nodePath, '/opt/node/bin/node'], logs: [], busy: false};
const context = vm.createContext({
  document: {getElementById: id => elements.get(id), querySelectorAll: () => [], addEventListener() {}},
  window: {__TAURI__: {core: {invoke: async (command, args) => {
    calls.push({command, ...args});
    return {config: {node: '', cwd: '/tmp', autoOpen: false}, state};
  }}}},
  Option: function(text, value) { return {text, value}; },
  setInterval() {},
});
(async () => {
  vm.runInContext(fs.readFileSync('src/renderer.js', 'utf8'), context);
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(elements.get('node').value, nodePath, 'auto detection must display the actual path');
  await elements.get('save').onclick();
  assert.equal(calls.at(-1).data.node, '', 'displaying an automatic path must not pin it');
  elements.get('node').value = '/opt/node/bin/node';
  elements.get('node').oninput();
  await elements.get('save').onclick();
  assert.equal(calls.at(-1).data.node, '/opt/node/bin/node');
  elements.get('autoNode').onclick();
  assert.equal(elements.get('node').value, nodePath);
  elements.get('node').value = '/custom/node';
  elements.get('node').oninput();
  await elements.get('save').onclick();
  assert.equal(calls.at(-1).data.node, '/custom/node');
  assert.equal(elements.has('homeNodePath'), false, 'the homepage must not display the Node path');
  console.log('Renderer Node selection regression checks passed');
})().catch(error => { console.error(error); process.exitCode = 1; });
