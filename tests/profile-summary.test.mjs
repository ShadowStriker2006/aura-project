import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const sourceUrl = new URL('../src/profile-summary.js', import.meta.url);
const source = await readFile(sourceUrl, 'utf8');
const profileModule = await import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`);
const {
  buildHomeProfileSummary,
  championNameForId,
  compactMasteryPoints,
  normalizeMasteries,
  normalizeRankedEntry,
  rankBadges,
  rankDetail,
  rankLabel,
  selectRankedEntries,
} = profileModule;

test('rank selection is order-independent and normalizes camel and snake case with zero LP', () => {
  const { solo, flex } = selectRankedEntries([
    {
      queueType: 'RANKED_FLEX_SR',
      tier: 'GOLD',
      rank: 'IV',
      leaguePoints: 0,
      wins: 12,
      losses: 8,
      hotStreak: true,
      freshBlood: true,
    },
    {
      queue_type: 'RANKED_SOLO_5x5',
      tier: 'EMERALD',
      rank: 'II',
      league_points: 0,
      wins: 8,
      losses: 2,
      hot_streak: false,
      fresh_blood: false,
    },
  ]);

  assert.equal(solo.queue_type, 'RANKED_SOLO_5x5');
  assert.equal(solo.league_points, 0);
  assert.equal(rankLabel(solo), 'EMERALD II');
  assert.equal(rankDetail(solo), '0 LP · 8W 2L · 80% WR');
  assert.equal(flex.queue_type, 'RANKED_FLEX_SR');
  assert.equal(flex.league_points, 0);
  assert.equal(flex.hot_streak, true);
  assert.equal(flex.fresh_blood, true);
});

test('rank badges normalize flags and sanitize mini-series progress', () => {
  const ranked = normalizeRankedEntry({
    queueType: 'RANKED_SOLO_5x5',
    hotStreak: true,
    veteran: true,
    freshBlood: true,
    miniSeries: {
      wins: 1,
      losses: 0,
      target: 3,
      progress: 'w-l?n<script>',
    },
  });

  assert.deepEqual(rankBadges(ranked), [
    { key: 'promos', label: 'Promos 1W 0L · first to 3 · WLN' },
    { key: 'hot-streak', label: 'Hot streak' },
    { key: 'veteran', label: 'Veteran' },
    { key: 'fresh-blood', label: 'New ranked entrant' },
  ]);

  const missing = normalizeRankedEntry({ queue_type: 'RANKED_FLEX_SR' });
  assert.equal(missing.hot_streak, false);
  assert.equal(missing.veteran, false);
  assert.equal(missing.fresh_blood, false);
  assert.equal(missing.mini_series, null);
  assert.deepEqual(rankBadges(missing), []);
  assert.deepEqual(rankBadges(null), []);
});

test('mastery normalization filters invalid rows, deduplicates, sorts deterministically, and caps output', () => {
  const raw = [
    { champion_id: 62, champion_level: 6, champion_points: 400_000 },
    {
      championId: 62,
      championLevel: 7,
      championPoints: 500_000,
      lastPlayTime: 1_720_000_000_000,
      championPointsUntilNextLevel: 0,
      championPointsSinceLastLevel: 120_000,
      tokensEarned: 2,
      chestGranted: true,
    },
    { champion_id: 1, champion_level: 7, champion_points: 500_000 },
    { championId: 266, championLevel: 5, championPoints: 450_000 },
    { championId: 0, championLevel: 7, championPoints: 999_999 },
    { championId: 'not-a-number', championLevel: 7, championPoints: 999_999 },
    null,
  ];

  const normalized = normalizeMasteries(raw, 10);
  assert.deepEqual(normalized.map((entry) => entry.champion_id), [1, 62, 266]);
  assert.equal(normalized.filter((entry) => entry.champion_id === 62).length, 1);
  assert.deepEqual(normalized.find((entry) => entry.champion_id === 62), {
    champion_id: 62,
    champion_level: 7,
    champion_points: 500_000,
    last_play_time: 1_720_000_000_000,
    champion_points_until_next_level: 0,
    champion_points_since_last_level: 120_000,
    tokens_earned: 2,
    chest_granted: true,
  });
  assert.deepEqual(
    normalizeMasteries(raw, 2).map((entry) => entry.champion_id),
    [1, 62],
  );
});

test('champion lookup uses the supplied numeric map and has an explicit unknown fallback', () => {
  const championMap = { 1: 'Annie', 62: 'Wukong' };
  assert.equal(championNameForId(championMap, 62), 'Wukong');
  assert.equal(championNameForId(championMap, 999), 'Champion 999');
  assert.equal(championNameForId(null, 62), 'Champion 62');
});

test('mastery point labels stay compact at each magnitude boundary', () => {
  assert.equal(compactMasteryPoints(-1), '0');
  assert.equal(compactMasteryPoints(999), '999');
  assert.equal(compactMasteryPoints(1_000), '1.0k');
  assert.equal(compactMasteryPoints(99_999), '100.0k');
  assert.equal(compactMasteryPoints(100_000), '100k');
  assert.equal(compactMasteryPoints(1_000_000), '1.0m');
  assert.equal(compactMasteryPoints(10_000_000), '10m');
});

test('home summary combines Solo rank, top mastery, and summoner level', () => {
  const profile = {
    summoner_level: 321,
    ranked_entries: [
      { queue_type: 'RANKED_FLEX_SR', tier: 'GOLD', rank: 'I', league_points: 55 },
      {
        queueType: 'RANKED_SOLO_5x5',
        tier: 'EMERALD',
        rank: 'II',
        leaguePoints: 0,
        wins: 8,
        losses: 2,
      },
    ],
  };
  const masteries = normalizeMasteries([
    { championId: 62, championLevel: 7, championPoints: 1_250_000 },
    { championId: 1, championLevel: 6, championPoints: 500_000 },
  ]);

  assert.deepEqual(buildHomeProfileSummary(profile, masteries, { 62: 'Wukong' }), {
    rank_value: 'EMERALD II',
    rank_detail: '0 LP · 8W 2L · 80% WR',
    mastery_value: 'Wukong',
    mastery_detail: 'Mastery 7 · 1.3m points',
    level_value: '321',
    level_detail: 'Riot summoner level',
  });
});

test('home summary exposes honest empty states instead of fabricated values', () => {
  assert.deepEqual(buildHomeProfileSummary(null, [], {}), {
    rank_value: 'Unranked',
    rank_detail: 'No Solo/Duo rank',
    mastery_value: '—',
    mastery_detail: 'No mastery loaded',
    level_value: '—',
    level_detail: 'Waiting for League',
  });
});
