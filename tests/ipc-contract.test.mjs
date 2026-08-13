import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

const projectRoot = dirname(dirname(fileURLToPath(import.meta.url)));
const rustMain = readFileSync(join(projectRoot, 'src-tauri', 'src', 'main.rs'), 'utf8');
const rustLiveClient = readFileSync(
  join(projectRoot, 'src-tauri', 'src', 'live_client', 'mod.rs'),
  'utf8',
);
const frontendIpc = readFileSync(join(projectRoot, 'src', 'services', 'ipc.js'), 'utf8');
function readJavaScriptSources(directory) {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) return readJavaScriptSources(path);
    return entry.isFile() && entry.name.endsWith('.js')
      ? [readFileSync(path, 'utf8')]
      : [];
  });
}

const frontendSource = readJavaScriptSources(join(projectRoot, 'src')).join('\n');

function sorted(values) {
  return [...new Set(values)].sort();
}

function registeredCommands() {
  const handler = rustMain.match(/generate_handler!\s*\[([\s\S]*?)\]\)/);
  assert.ok(handler, 'main.rs must contain a tauri::generate_handler! command list');
  return sorted(
    [...handler[1].matchAll(/^\s*(?:[A-Za-z0-9_]+::)+([A-Za-z0-9_]+),\s*$/gm)]
      .map((match) => match[1]),
  );
}

function literalInvokeCommands() {
  return sorted(
    [...frontendSource.matchAll(/\b(?:invoke|invokeWithTimeout)\s*\(\s*['"]([a-z0-9_]+)['"]/g)]
      .map((match) => match[1]),
  );
}

function dynamicDispatchCommands() {
  const commands = [];
  const patterns = [
    /\b(?:draft|live|post)\s*:\s*\{\s*command:\s*['"]([a-z0-9_]+)['"]/g,
    /\bspotifyControl\s*\(\s*['"]([a-z0-9_]+)['"]/g,
    /\boverlayCommand\s*\(\s*['"]([a-z0-9_]+)['"]/g,
  ];
  for (const pattern of patterns) {
    commands.push(...[...frontendSource.matchAll(pattern)].map((match) => match[1]));
  }
  return sorted(commands);
}

test('every Tauri command and frontend invocation has a matching contract endpoint', () => {
  const registered = registeredCommands();
  const literal = literalInvokeCommands();
  const dynamic = dynamicDispatchCommands();
  const invoked = sorted([...literal, ...dynamic]);

  assert.deepEqual(
    registered.filter((command) => !invoked.includes(command)),
    [],
    'registered handlers without a frontend call site',
  );
  assert.deepEqual(
    invoked.filter((command) => !registered.includes(command)),
    [],
    'frontend calls without a registered handler',
  );
});

test('computed dispatch is limited to the reviewed hard-coded command sets', () => {
  assert.deepEqual(dynamicDispatchCommands(), [
    'advisor_draft_mandate',
    'advisor_live_orders',
    'advisor_post_game',
    'hide_overlay',
    'show_overlay',
    'spotify_pause',
    'spotify_play',
    'spotify_previous',
    'spotify_skip',
  ]);

  assert.match(frontendSource, /invokeWithTimeout\s*\(\s*config\.command\s*,/);
  assert.match(frontendSource, /async function spotifyControl\s*\(command,[\s\S]*?invokeWithTimeout\s*\(\s*command\s*,/);
  assert.match(frontendSource, /async function overlayCommand\s*\(command\)[\s\S]*?invokeWithTimeout\s*\(\s*command\s*,/);
});

test('typed live-client event channel literals match across Rust and frontend', () => {
  const channels = [
    'game:state-changed',
    'live:game-tick',
    'live:player-update',
    'draft:update',
  ];

  for (const channel of channels) {
    assert.equal(
      rustLiveClient.split(`\"${channel}\"`).length - 1,
      1,
      `Rust should define ${channel} exactly once`,
    );
    assert.equal(
      frontendIpc.split(`'${channel}'`).length - 1,
      1,
      `frontend should define ${channel} exactly once`,
    );
  }
});
