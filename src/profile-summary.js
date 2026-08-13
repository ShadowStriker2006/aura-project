const SOLO_QUEUE = 'RANKED_SOLO_5x5';
const FLEX_QUEUE = 'RANKED_FLEX_SR';

const finiteInteger = (value, fallback = 0) => {
  const parsed = Number(value);
  return Number.isFinite(parsed) ? Math.max(0, Math.trunc(parsed)) : fallback;
};

export function normalizeRankedEntry(raw) {
  if (!raw || typeof raw !== 'object') return null;
  return {
    queue_type: String(raw.queue_type ?? raw.queueType ?? ''),
    tier: String(raw.tier ?? ''),
    rank: String(raw.rank ?? ''),
    league_points: finiteInteger(raw.league_points ?? raw.leaguePoints),
    wins: finiteInteger(raw.wins),
    losses: finiteInteger(raw.losses),
    hot_streak: (raw.hot_streak ?? raw.hotStreak) === true,
    veteran: raw.veteran === true,
    fresh_blood: (raw.fresh_blood ?? raw.freshBlood) === true,
    mini_series: raw.mini_series ?? raw.miniSeries ?? null,
  };
}

export function normalizeRankedEntries(entries) {
  return (Array.isArray(entries) ? entries : [])
    .map(normalizeRankedEntry)
    .filter((entry) => entry?.queue_type);
}

export function selectRankedEntries(entries) {
  const safe = normalizeRankedEntries(entries);
  return {
    solo: safe.find((entry) => entry?.queue_type === SOLO_QUEUE) || null,
    flex: safe.find((entry) => entry?.queue_type === FLEX_QUEUE) || null,
  };
}

export function rankLabel(entry, unrankedLabel = 'Unranked') {
  const tier = String(entry?.tier || '').trim();
  const division = String(entry?.rank || '').trim();
  return tier ? `${tier}${division ? ` ${division}` : ''}` : unrankedLabel;
}

export function rankDetail(entry) {
  if (!entry) return 'No ranked entry';
  const wins = finiteInteger(entry.wins);
  const losses = finiteInteger(entry.losses);
  const games = wins + losses;
  const winRate = games ? Math.round((wins / games) * 100) : 0;
  return `${finiteInteger(entry.league_points)} LP · ${wins}W ${losses}L${games ? ` · ${winRate}% WR` : ''}`;
}

export function rankBadges(entry) {
  if (!entry || typeof entry !== 'object') return [];
  const badges = [];
  const series = entry.mini_series;
  if (series && typeof series === 'object') {
    const wins = finiteInteger(series.wins);
    const losses = finiteInteger(series.losses);
    const target = finiteInteger(series.target);
    const progress = String(series.progress || '')
      .toUpperCase()
      .replace(/[^WLN]/g, '')
      .slice(0, 10);
    const record = `${wins}W ${losses}L`;
    badges.push({
      key: 'promos',
      label: `Promos ${record}${target ? ` · first to ${target}` : ''}${progress ? ` · ${progress}` : ''}`,
    });
  }
  if (entry.hot_streak === true) badges.push({ key: 'hot-streak', label: 'Hot streak' });
  if (entry.veteran === true) badges.push({ key: 'veteran', label: 'Veteran' });
  if (entry.fresh_blood === true) badges.push({ key: 'fresh-blood', label: 'New ranked entrant' });
  return badges;
}

export function normalizeMasteries(entries, limit = 200) {
  const boundedLimit = Math.min(200, Math.max(0, finiteInteger(limit, 200)));
  const byChampion = new Map();
  for (const raw of Array.isArray(entries) ? entries : []) {
    const championId = finiteInteger(raw?.champion_id ?? raw?.championId);
    if (!championId) continue;
    const mastery = {
      champion_id: championId,
      champion_level: finiteInteger(raw.champion_level ?? raw.championLevel),
      champion_points: finiteInteger(raw.champion_points ?? raw.championPoints),
      last_play_time: Number.isFinite(Number(raw.last_play_time ?? raw.lastPlayTime))
        ? Number(raw.last_play_time ?? raw.lastPlayTime) : 0,
      champion_points_until_next_level: Math.trunc(Number(
        raw.champion_points_until_next_level ?? raw.championPointsUntilNextLevel,
      ) || 0),
      champion_points_since_last_level: Math.trunc(Number(
        raw.champion_points_since_last_level ?? raw.championPointsSinceLastLevel,
      ) || 0),
      tokens_earned: finiteInteger(raw.tokens_earned ?? raw.tokensEarned),
      chest_granted: (raw.chest_granted ?? raw.chestGranted) === true,
    };
    const current = byChampion.get(championId);
    if (!current || mastery.champion_points > current.champion_points) {
      byChampion.set(championId, mastery);
    }
  }
  return [...byChampion.values()]
    .sort((left, right) => right.champion_points - left.champion_points
      || left.champion_id - right.champion_id)
    .slice(0, boundedLimit);
}

export function masteryForChampion(masteries, championId) {
  const requested = finiteInteger(championId);
  return (Array.isArray(masteries) ? masteries : [])
    .find((entry) => finiteInteger(entry?.champion_id) === requested) || null;
}

export function championNameForId(championMap, championId) {
  const id = String(finiteInteger(championId));
  const value = championMap && typeof championMap === 'object' ? championMap[id] : null;
  return String(value || '').trim() || `Champion ${id}`;
}

export function compactMasteryPoints(value) {
  const points = finiteInteger(value);
  if (points >= 1_000_000) return `${(points / 1_000_000).toFixed(points >= 10_000_000 ? 0 : 1)}m`;
  if (points >= 1_000) return `${(points / 1_000).toFixed(points >= 100_000 ? 0 : 1)}k`;
  return String(points);
}

export function buildHomeProfileSummary(profile, masteries, championMap) {
  const { solo } = selectRankedEntries(profile?.ranked_entries);
  const top = Array.isArray(masteries) && masteries.length ? masteries[0] : null;
  return {
    rank_value: rankLabel(solo, 'Unranked'),
    rank_detail: solo ? rankDetail(solo) : 'No Solo/Duo rank',
    mastery_value: top ? championNameForId(championMap, top.champion_id) : '—',
    mastery_detail: top
      ? `Mastery ${finiteInteger(top.champion_level)} · ${compactMasteryPoints(top.champion_points)} points`
      : 'No mastery loaded',
    level_value: profile ? String(finiteInteger(profile.summoner_level)) : '—',
    level_detail: profile ? 'Riot summoner level' : 'Waiting for League',
  };
}

export const profileSummaryTestHooks = Object.freeze({ finiteInteger });
