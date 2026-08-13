import assert from 'node:assert/strict';
import { readFile, readdir } from 'node:fs/promises';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const projectRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

const read = (relativePath) => readFile(path.join(projectRoot, relativePath), 'utf8');

async function sourceFiles(relativeDirectory) {
  const directory = path.join(projectRoot, relativeDirectory);
  const entries = await readdir(directory, { withFileTypes: true });
  const nested = await Promise.all(entries.map(async (entry) => {
    const absolute = path.join(directory, entry.name);
    if (entry.isDirectory()) {
      return sourceFiles(path.relative(projectRoot, absolute));
    }
    return entry.isFile() ? [absolute] : [];
  }));
  return nested.flat();
}

test('Riot profile and ranked endpoints remain separately registered and implemented', async () => {
  const [main, riotApi] = await Promise.all([
    read('src-tauri/src/main.rs'),
    read('src-tauri/src/riotapi.rs'),
  ]);

  assert.match(main, /riotapi::get_summoner_profile\s*,/);
  assert.match(main, /riotapi::get_league_entries\s*,/);
  assert.match(riotApi, /pub async fn get_summoner_profile\s*\(/);
  assert.match(riotApi, /pub async fn get_league_entries\s*\(/);

  const profileBody = riotApi.slice(
    riotApi.indexOf('pub async fn get_summoner_profile'),
    riotApi.indexOf('pub async fn get_league_entries'),
  );
  assert.match(profileBody, /"summoner"[\s\S]*"v4"/);
  assert.doesNotMatch(profileBody, /ranked_entries|league\/v4|fetch_league_entries/);
});

test('removed preferences, legacy timer channel, and hard-coded map version stay removed', async () => {
  const files = [
    ...await sourceFiles('src'),
    ...await sourceFiles('src-tauri/src'),
  ];
  const combined = (await Promise.all(files.map((file) => readFile(file, 'utf8')))).join('\n');

  for (const symbol of [
    'load_riot_id_prefs',
    'save_riot_id_prefs',
    'RiotIdPrefs',
    'MAP_ASSET_VERSION',
    'aura-objective-timers',
  ]) {
    assert.equal(combined.includes(symbol), false, `${symbol} must remain removed`);
  }
});

test('overlay supports compact geometry and a reversible click-through lock', async () => {
  const overlay = await read('src-tauri/src/overlay.rs');
  assert.match(overlay, /const STANDBY_WIDTH: f64 = 32\.0;/);
  assert.match(overlay, /const STANDBY_HEIGHT: f64 = 32\.0;/);
  assert.match(overlay, /const COMPACT_WIDTH: f64 = 432\.0;/);
  assert.match(overlay, /const COMPACT_HEIGHT: f64 = 52\.0;/);
  assert.match(overlay, /const EXPANDED_WIDTH: f64 = 520\.0;/);
  assert.match(overlay, /const EXPANDED_HEIGHT: f64 = 150\.0;/);
  assert.match(overlay, /set_focusable\(false\)/);
  assert.match(overlay, /set_focusable\(true\)/);
  assert.match(overlay, /\.focused\(false\)/);
  assert.match(overlay, /set_ignore_cursor_events\(true\)/);
  assert.match(overlay, /set_ignore_cursor_events\(false\)/);
  assert.match(overlay, /overlay:layout-changed/);
});

test('runtime source contains no UUID-shaped Riot development key', async () => {
  const files = [
    ...await sourceFiles('src'),
    ...await sourceFiles('src-tauri/src'),
  ];
  const riotKey = /RGAPI-[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/i;
  for (const file of files) {
    const content = await readFile(file, 'utf8');
    assert.doesNotMatch(content, riotKey, `development key found in ${file}`);
  }
});
