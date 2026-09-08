const fs = require('node:fs');
const version = require('../package.json').version;
const config = require('../src-tauri/tauri.conf.json');
const cargo = fs.readFileSync('src-tauri/Cargo.toml', 'utf8').match(/^version = "([^"]+)"/m)?.[1];
if (config.version !== version || cargo !== version || process.env.GITHUB_REF_NAME !== `v${version}`) {
  throw new Error('Tag, package.json, Cargo.toml and tauri.conf.json versions must match');
}
