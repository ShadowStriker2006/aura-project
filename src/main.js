import {
  buildHomeProfileSummary,
  championNameForId,
  compactMasteryPoints,
  masteryForChampion,
  normalizeMasteries,
  normalizeRankedEntries,
  rankBadges,
  rankDetail,
  rankLabel,
  selectRankedEntries,
} from './profile-summary.js';
import {
  buildLiveGameViewModel,
  renderLiveGameView,
} from './components/analytics/live-game-metrics.js';
import { normalizeOverlayLayout } from './components/overlay/live-overlay.js';
import {
  createIpcClient,
  subscribeLiveClientEvents,
} from './services/ipc.js';

window.addEventListener('load', () => {
  const { invoke: tauriInvoke } = window.__TAURI__.core;
  const { listen } = window.__TAURI__.event;
  const { invoke, invokeWithTimeout } = createIpcClient(tauriInvoke);
  const MAX_TIMELINE_CACHE = 3;

  const state = {
    championMap: {},
    imageIdByNumericId: {},
    itemMap: {},
    runeTrees: [],
    ddragonVersion: null,
    selectedChampion: null,
    selectedChampionDetails: null,
    runePage: {
      primaryTreeId: 8000,
      secondaryTreeId: 8400,
      primary: new Map(),
      secondary: new Map(),
      secondaryOrder: [],
      shards: new Map(),
    },
    build: [],
    championDetails: new Map(),
    allyChampionIds: [],
    enemyChampionIds: [],
    localChampionId: null,
    liveAllyChampionIds: [],
    liveEnemyChampionIds: [],
    liveChampionId: null,
    currentQueueId: null,
    gameflowPhase: 'NONE',
    gameStatus: null,
    profileMatches: [],
    matchDetails: new Map(),
    matchDetailRequests: new Map(),
    matchTimelines: new Map(),
    matchTimelineRequests: new Map(),
    mapReplayControllers: new Map(),
    mapReplayModule: null,
    mapReplayModuleRequest: null,
    summonerProfile: null,
    rankedEntries: [],
    rankStatus: 'idle',
    rankError: '',
    championMasteries: [],
    masteryStatus: 'idle',
    masteryProfileKey: '',
    masteryError: '',
    localRiotAccount: null,
    profileTarget: null,
    profileLoadGeneration: 0,
    profileLoadChain: Promise.resolve(),
    autoProfileLoadedKey: '',
    telemetry: { gameTime: 0, dragonRespawnAt: null, baronRespawnAt: null, receivedAt: 0 },
    spotifyConnected: false,
    currentPage: 'home',
    integration: null,
    advisorStatus: null,
    advisorResults: { draft: null, live: null, post: null },
    advisorBusy: { draft: false, live: false, post: false },
    advisorDraftSignature: '',
    advisorDraftTimer: null,
    advisorDetectedRole: '',
    overlayLayout: {
      mode: 'standby',
      scalePercent: 100,
      opacityPercent: 55,
      locked: true,
    },
    overlayPreferredMode: 'compact',
    overlayVisible: false,
  };

  const $ = (id) => document.getElementById(id);
  const escapeHtml = (value) => String(value ?? '')
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
    .replaceAll("'", '&#039;');

  function logOk(scope, message) { console.log(`[AURA::${scope}][OK]`, message); }
  function logErr(scope, error) { console.error(`[AURA::${scope}][ERR]`, error); }

  let toastTimer = null;
  function toast(message, type = 'info') {
    const element = $('toast');
    if (!element) return;
    element.textContent = String(message);
    element.className = `toast show ${type}`;
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => { element.className = 'toast'; }, 4200);
  }

  function setMessage(id, message, isError = false) {
    const element = $(id);
    if (!element) return;
    element.textContent = message;
    element.classList.toggle('error', isError);
    element.classList.toggle('success', !isError && Boolean(message));
  }

  function setIntegrationStatus(id, text, mode = 'pending', title = '') {
    const element = $(id);
    if (!element) return;
    element.textContent = text;
    element.title = title;
    element.classList.remove('pending', 'ready', 'missing');
    element.classList.add(mode);
  }

  // Navigation ---------------------------------------------------------------
  function navigate(page, updateHash = true) {
    if (!document.querySelector(`[data-page-view="${page}"]`)) page = 'home';
    if (page !== 'profile') closeExpandedMatches();
    state.currentPage = page;
    document.querySelectorAll('.page-view').forEach((view) => {
      view.classList.toggle('active', view.dataset.pageView === page);
    });
    document.querySelectorAll('.nav-item').forEach((button) => {
      button.classList.toggle('active', button.dataset.page === page);
    });
    if (updateHash) history.replaceState(null, '', `#${page}`);
    const scroller = document.querySelector('.main-content-scroller');
    if (scroller) scroller.scrollTop = 0;
    hideGlobalResults();
    if (page === 'overlays') refreshOverlayStatus();
    if (page === 'intelligence') refreshAdvisorStatus(false);
    if (page === 'home') renderHomeProfileSummary();
    if (page === 'profile') renderProfileMasteryList();
    if (page === 'champions' && state.ddragonVersion) {
      renderChampionCatalog($('champion-page-search')?.value || '');
    }
    if (page === 'builds') {
      renderBuild();
      renderItemCatalog($('item-search')?.value || '');
    }
    if (page === 'runes') renderRuneTrees();
  }

  document.querySelectorAll('.nav-item').forEach((button) => {
    button.addEventListener('click', () => navigate(button.dataset.page));
  });
  document.querySelectorAll('[data-go-page]').forEach((button) => {
    button.addEventListener('click', () => navigate(button.dataset.goPage));
  });
  $('hero-profile')?.addEventListener('click', () => navigate('profile'));
  $('hero-overlays')?.addEventListener('click', () => navigate('overlays'));
  navigate(location.hash.slice(1) || 'home', false);

  // Data Dragon and champion search -----------------------------------------
  function ddragonImg(path) {
    return state.ddragonVersion
      ? `https://ddragon.leagueoflegends.com/cdn/${state.ddragonVersion}/${path}`
      : '';
  }

  function runeImg(path) {
    return `https://ddragon.leagueoflegends.com/cdn/img/${path}`;
  }

  function championEntries() {
    const byName = new Map();
    Object.entries(state.championMap)
      .map(([numericId, name]) => ({
        numericId,
        name,
        imageId: state.imageIdByNumericId[numericId] || name.replace(/[^a-z0-9]/gi, ''),
      }))
      .forEach((champion) => {
        const key = champion.name.toLocaleLowerCase();
        const existing = byName.get(key);
        if (!existing || Number(champion.numericId) < Number(existing.numericId)) {
          byName.set(key, champion);
        }
      });
    return [...byName.values()].sort((a, b) => a.name.localeCompare(b.name));
  }

  function matchingChampions(query, limit = Infinity) {
    const normalized = query.trim().toLocaleLowerCase();
    const all = championEntries();
    if (!normalized) return all.slice(0, limit);
    return all
      .filter((champion) => champion.name.toLocaleLowerCase().includes(normalized))
      .sort((a, b) => {
        const aStarts = a.name.toLocaleLowerCase().startsWith(normalized) ? 0 : 1;
        const bStarts = b.name.toLocaleLowerCase().startsWith(normalized) ? 0 : 1;
        return aStarts - bStarts || a.name.localeCompare(b.name);
      })
      .slice(0, limit);
  }

  async function initDDragon(attempt = 1) {
    try {
      const [champions, items, version, images, runes] = await Promise.all([
        invokeWithTimeout('get_champion_map', {}, 12000),
        invokeWithTimeout('get_item_map', {}, 12000),
        invokeWithTimeout('get_ddragon_version', {}, 12000),
        invokeWithTimeout('get_champion_image_id_map', {}, 12000),
        invokeWithTimeout('get_rune_trees', {}, 12000),
      ]);
      state.championMap = champions;
      state.itemMap = items;
      state.ddragonVersion = version;
      state.imageIdByNumericId = images;
      state.runeTrees = runes;
      $('patch-label').textContent = `Patch ${version}`;
      $('ddragon-state').textContent = `${championEntries().length} champions ready`;
      $('profile-icon').src = ddragonImg('img/profileicon/588.png');
      applyKnownProfileIcon();
      renderMasteryViews();
      if (state.currentPage === 'champions') renderChampionCatalog($('champion-page-search')?.value || '');
      if (state.currentPage === 'builds') {
        renderBuild();
        renderItemCatalog($('item-search')?.value || '');
      }
      if (state.currentPage === 'runes') renderRuneTrees();
      logOk('DDRAGON', `patch ${version} loaded`);
    } catch (error) {
      if (attempt < 8) {
        $('ddragon-state').textContent = `Game data retry ${attempt}/8`;
        setTimeout(() => initDDragon(attempt + 1), 2500);
      } else {
        $('ddragon-state').textContent = 'Game data unavailable';
        toast(`Could not load League game data: ${error}`, 'error');
        logErr('DDRAGON', error);
      }
    }
  }

  function renderChampionCatalog(query) {
    const results = matchingChampions(query);
    $('champion-result-count').textContent = state.ddragonVersion
      ? `${results.length} shown`
      : '';
    if (!results.length) {
      $('champion-catalog').innerHTML = '<div class="empty-state">No champion matches that search.</div>';
      return;
    }
    $('champion-catalog').innerHTML = results.map((champion) => `
      <button class="champion-tile${state.selectedChampion?.numericId === champion.numericId ? ' selected' : ''}" data-champion-id="${escapeHtml(champion.numericId)}">
        <img src="${ddragonImg(`img/champion/${champion.imageId}.png`)}" alt="" loading="lazy">
        <span>${escapeHtml(champion.name)}</span>
      </button>
    `).join('');
  }

  async function selectChampion(numericId) {
    const champion = championEntries().find((entry) => entry.numericId === String(numericId));
    if (!champion) return;
    state.selectedChampion = champion;
    state.selectedChampionDetails = null;
    $('build-champion-label').textContent = champion.name;
    $('rune-champion-label').textContent = champion.name;
    renderChampionCatalog($('champion-page-search').value);
    $('champion-detail').innerHTML = '<div class="empty-state">Loading champion details…</div>';
    navigate('champions');

    try {
      const details = await getChampionDetails(champion);
      if (state.selectedChampion?.numericId !== champion.numericId) return;
      state.selectedChampionDetails = details;
      renderChampionDetail(champion, details);
      applyRecommendedBuild(false).catch((error) => logErr('BUILD', error));
      applyRecommendedRunes(false);
    } catch (error) {
      $('champion-detail').innerHTML = `<div class="empty-state error-text">${escapeHtml(error)}</div>`;
      logErr('CHAMPION', error);
    }
  }

  async function getChampionDetails(champion) {
    let details = state.championDetails.get(champion.imageId);
    if (!details) {
      details = await invokeWithTimeout(
        'get_champion_details',
        { championImageId: champion.imageId },
        12000
      );
      if (!details || typeof details !== 'object') {
        throw new Error('Champion details are temporarily unavailable. Check your connection and try again.');
      }
      state.championDetails.set(champion.imageId, details);
    }
    return details;
  }

  function plainText(markup) {
    const element = document.createElement('div');
    element.innerHTML = String(markup || '');
    return element.textContent || '';
  }

  function renderRankFlags(id, entry) {
    const container = $(id);
    if (!container) return;
    container.replaceChildren();
    for (const badge of rankBadges(entry)) {
      const element = document.createElement('span');
      element.className = `rank-flag ${badge.key}`;
      element.textContent = badge.label;
      container.append(element);
    }
  }

  function formatMasteryDate(timestamp) {
    const numeric = Number(timestamp);
    if (!Number.isFinite(numeric) || numeric <= 0) return 'Last played date unavailable';
    const date = new Date(numeric);
    if (Number.isNaN(date.getTime())) return 'Last played date unavailable';
    return `Last played ${new Intl.DateTimeFormat(undefined, { dateStyle: 'medium' }).format(date)}`;
  }

  function renderHomeProfileSummary() {
    const localLevel = Number(state.localRiotAccount?.summoner_level) || 0;
    const canUseLocalFallback = !state.profileTarget || state.profileTarget.source === 'local';
    const profileBase = state.summonerProfile || (canUseLocalFallback && localLevel
      ? { summoner_level: localLevel }
      : null);
    const summaryProfile = profileBase
      ? { ...profileBase, ranked_entries: state.rankedEntries }
      : null;
    const summary = buildHomeProfileSummary(
      summaryProfile,
      state.championMasteries,
      state.championMap,
    );
    $('home-rank-value').textContent = state.rankStatus === 'loading' || state.rankStatus === 'error'
      ? '—'
      : summary.rank_value;
    $('home-rank-detail').textContent = state.rankStatus === 'loading'
      ? 'Loading official rank…'
      : state.rankStatus === 'error'
        ? 'Rank temporarily unavailable'
        : summary.rank_detail;
    $('home-mastery-value').textContent = summary.mastery_value;
    $('home-mastery-detail').textContent = state.masteryStatus === 'loading'
      ? 'Loading official mastery…'
      : state.masteryStatus === 'error'
        ? 'Mastery temporarily unavailable'
        : summary.mastery_detail;
    $('home-level-value').textContent = summary.level_value;
    const target = state.profileTarget || state.localRiotAccount;
    $('home-level-detail').textContent = target?.gameName && target?.tagLine
      ? `${target.gameName}#${target.tagLine}`
      : summary.level_detail;

    const { solo } = selectRankedEntries(state.rankedEntries);
    const badges = rankBadges(solo);
    if (badges.length && solo) {
      $('home-rank-detail').textContent = `${rankDetail(solo)} · ${badges.map((badge) => badge.label).join(' · ')}`;
    }
  }

  function renderSelectedChampionMastery() {
    const container = $('champion-detail-mastery');
    if (!container || !state.selectedChampion) return;
    container.className = 'champion-detail-mastery';
    if (state.masteryStatus === 'loading') {
      container.textContent = 'Loading mastery for this profile…';
      return;
    }
    if (state.masteryStatus === 'error') {
      container.classList.add('unavailable');
      container.textContent = 'Mastery temporarily unavailable';
      return;
    }
    if (state.masteryStatus !== 'ready') {
      container.textContent = 'Load a Riot profile to see champion mastery';
      return;
    }
    const mastery = masteryForChampion(state.championMasteries, state.selectedChampion.numericId);
    if (!mastery) {
      container.textContent = 'No mastery points on this profile';
      return;
    }
    const progression = mastery.champion_points_until_next_level > 0
      ? `${mastery.champion_points_until_next_level.toLocaleString()} points to next level`
      : 'Current mastery tier reported by Riot';
    container.innerHTML = `
      <span class="mastery-star" aria-hidden="true">★</span>
      <strong>Mastery ${mastery.champion_level}</strong>
      <span>${mastery.champion_points.toLocaleString()} points</span>
      <span>${escapeHtml(progression)}</span>
      <span>${escapeHtml(formatMasteryDate(mastery.last_play_time))}</span>`;
  }

  function renderProfileMasteryList() {
    const list = $('profile-mastery-list');
    const status = $('profile-mastery-status');
    if (!list || !status) return;
    if (state.masteryStatus === 'loading') {
      status.textContent = 'Loading…';
      list.innerHTML = '<div class="empty-state">Loading official champion mastery…</div>';
      return;
    }
    if (state.masteryStatus === 'error') {
      status.textContent = 'Unavailable';
      list.innerHTML = `<div class="empty-state error-text">${escapeHtml(state.masteryError || 'Champion mastery is temporarily unavailable.')}</div>`;
      return;
    }
    if (state.masteryStatus !== 'ready') {
      status.textContent = 'Profile required';
      list.innerHTML = '<div class="empty-state">Load a Riot profile to see champion mastery.</div>';
      return;
    }
    const visible = state.championMasteries.slice(0, 5);
    const countLabel = visible.length
      ? `Top ${visible.length} of ${state.championMasteries.length}`
      : 'No mastery entries';
    status.textContent = state.ddragonVersion ? countLabel : `${countLabel} · game data loading`;
    if (!visible.length) {
      list.innerHTML = '<div class="empty-state">Riot returned no champion mastery for this profile.</div>';
      return;
    }
    list.innerHTML = visible.map((mastery) => {
      const id = String(mastery.champion_id);
      const name = championNameForId(state.championMap, id);
      const imageId = state.imageIdByNumericId[id];
      const championReady = Boolean(state.ddragonVersion && imageId && state.championMap[id]);
      const portrait = championReady
        ? `<img class="champion-avatar" src="${ddragonImg(`img/champion/${imageId}.png`)}" alt="">`
        : '<span class="champion-avatar mastery-avatar-fallback" aria-hidden="true">?</span>';
      return `
        <button type="button" class="champion-card" data-mastery-champion-id="${escapeHtml(id)}"${championReady ? '' : ' disabled'} aria-label="${championReady ? `Open ${escapeHtml(name)} champion details` : `Game data loading for champion ${escapeHtml(id)}`}">
          <span class="champion-left">${portrait}<span class="champion-info"><strong class="champion-name">${escapeHtml(name)}</strong><span class="mastery"><span class="mastery-star" aria-hidden="true">★</span><span class="mastery-level">Mastery ${mastery.champion_level}</span></span><span class="champion-games">${escapeHtml(formatMasteryDate(mastery.last_play_time))}</span></span></span>
          <span class="champion-right"><strong class="kda-number">${escapeHtml(compactMasteryPoints(mastery.champion_points))}</strong><span class="kda-label">points</span></span>
        </button>`;
    }).join('');
  }

  function renderMasteryViews() {
    renderHomeProfileSummary();
    renderProfileMasteryList();
    renderSelectedChampionMastery();
  }

  function resetProfilePresentation() {
    state.summonerProfile = null;
    state.rankedEntries = [];
    state.rankStatus = 'idle';
    state.rankError = '';
    state.profileMatches = [];
    state.championMasteries = [];
    state.masteryStatus = 'idle';
    state.masteryProfileKey = '';
    state.masteryError = '';
    $('profile-level').textContent = 'Level —';
    $('profile-solo-rank').textContent = 'Solo/Duo rank —';
    $('profile-flex-rank').textContent = 'Flex rank —';
    if (state.ddragonVersion) {
      const fallbackIcon = ddragonImg('img/profileicon/588.png');
      $('profile-icon').src = fallbackIcon;
      $('profile-page-icon').src = fallbackIcon;
    }
    for (const id of ['profile-winrate', 'profile-kda', 'profile-cs', 'profile-vision', 'profile-kp', 'profile-dpm']) {
      $(id).textContent = '—';
    }
    destroyAllMapReplays();
    state.matchDetails.clear();
    state.matchDetailRequests.clear();
    state.matchTimelines.clear();
    state.matchTimelineRequests.clear();
    renderRankFlags('profile-solo-flags', null);
    renderRankFlags('profile-flex-flags', null);
    renderMasteryViews();
  }

  async function loadChampionMasteries(target, generation) {
    const requestedKey = profileTargetKey(target);
    state.masteryStatus = 'loading';
    state.masteryProfileKey = requestedKey;
    state.masteryError = '';
    renderMasteryViews();
    try {
      const payload = await invokeWithTimeout('get_champion_masteries', {}, 24000);
      if (generation !== state.profileLoadGeneration
          || requestedKey !== profileTargetKey(state.profileTarget)) return;
      state.championMasteries = normalizeMasteries(payload);
      state.masteryStatus = 'ready';
      renderMasteryViews();
    } catch (error) {
      if (generation !== state.profileLoadGeneration
          || requestedKey !== profileTargetKey(state.profileTarget)) return;
      state.championMasteries = [];
      state.masteryStatus = 'error';
      state.masteryError = String(error);
      renderMasteryViews();
      logErr('MASTERY', error);
    }
  }

  function applyKnownProfileIcon() {
    if (!state.ddragonVersion) return;
    const canUseLocalFallback = !state.profileTarget || state.profileTarget.source === 'local';
    const iconId = Number(state.summonerProfile?.profile_icon_id
      || (canUseLocalFallback ? state.localRiotAccount?.profile_icon_id : 0)) || 0;
    if (!iconId) return;
    const source = ddragonImg(`img/profileicon/${iconId}.png`);
    $('profile-icon').src = source;
    $('profile-page-icon').src = source;
  }

  function renderChampionDetail(champion, details) {
    const spells = [
      details.passive,
      ...(Array.isArray(details.spells) ? details.spells : []),
    ].filter((spell) => spell && typeof spell === 'object');
    const stat = (key) => Number(details.stats?.[key] || 0).toFixed(key.includes('perlevel') ? 1 : 0);
    $('champion-detail').innerHTML = `
      <div class="detail-hero">
        <img src="${ddragonImg(`img/champion/${champion.imageId}.png`)}" alt="">
        <div><h2>${escapeHtml(details.name || champion.name)}</h2><p>${escapeHtml(details.title || '')}</p><div class="tag-row">${(Array.isArray(details.tags) ? details.tags : []).map((tag) => `<span>${escapeHtml(tag)}</span>`).join('')}</div><div id="champion-detail-mastery" class="champion-detail-mastery" aria-live="polite"></div></div>
      </div>
      <p class="detail-lore">${escapeHtml(details.lore || 'Champion information loaded without lore text.')}</p>
      <div class="stat-chips">
        <span>HP ${stat('hp')}</span><span>Armor ${stat('armor')}</span><span>AD ${stat('attackdamage')}</span><span>Move ${stat('movespeed')}</span>
      </div>
      <div class="ability-list">
        ${spells.map((spell, index) => `<article><strong>${index === 0 ? 'Passive · ' : ''}${escapeHtml(spell.name)}</strong><p>${escapeHtml(plainText(spell.description))}</p></article>`).join('')}
      </div>
      <div class="button-row">
        <button id="detail-open-build" class="btn-secondary">Plan Build</button>
        <button id="detail-open-runes" class="btn-secondary">Plan Runes</button>
      </div>
    `;
    renderSelectedChampionMastery();
    $('detail-open-build')?.addEventListener('click', () => navigate('builds'));
    $('detail-open-runes')?.addEventListener('click', () => navigate('runes'));
  }

  $('champion-catalog')?.addEventListener('click', (event) => {
    const tile = event.target.closest('[data-champion-id]');
    if (tile) selectChampion(tile.dataset.championId);
  });
  $('profile-mastery-list')?.addEventListener('click', (event) => {
    const row = event.target.closest('[data-mastery-champion-id]');
    if (row) selectChampion(row.dataset.masteryChampionId);
  });
  $('champion-page-search')?.addEventListener('input', (event) => renderChampionCatalog(event.target.value));

  function renderGlobalResults(query) {
    const container = $('global-search-results');
    const matches = matchingChampions(query, 7);
    if (!query.trim() || !matches.length) {
      container.hidden = true;
      container.innerHTML = '';
      return;
    }
    container.innerHTML = matches.map((champion) => `
      <button data-global-champion="${escapeHtml(champion.numericId)}">
        <img src="${ddragonImg(`img/champion/${champion.imageId}.png`)}" alt="">
        <span>${escapeHtml(champion.name)}</span>
      </button>
    `).join('');
    container.hidden = false;
  }

  function hideGlobalResults() {
    const container = $('global-search-results');
    if (container) container.hidden = true;
  }

  $('global-champion-search')?.addEventListener('input', (event) => renderGlobalResults(event.target.value));
  $('global-champion-search')?.addEventListener('keydown', (event) => {
    if (event.key !== 'Enter') return;
    const champion = matchingChampions(event.target.value, 1)[0];
    if (champion) {
      $('champion-page-search').value = event.target.value;
      selectChampion(champion.numericId);
    } else {
      toast('No champion found for that search.', 'error');
    }
  });
  $('global-search-results')?.addEventListener('click', (event) => {
    const result = event.target.closest('[data-global-champion]');
    if (!result) return;
    const champion = championEntries().find((entry) => entry.numericId === result.dataset.globalChampion);
    $('champion-page-search').value = champion?.name || '';
    $('global-champion-search').value = champion?.name || '';
    selectChampion(result.dataset.globalChampion);
  });
  document.addEventListener('click', (event) => {
    if (!event.target.closest('.search-container')) hideGlobalResults();
  });

  // Builds and runes ---------------------------------------------------------
  function itemIdByName(name) {
    const matches = Object.entries(state.itemMap)
      .filter(([, itemName]) => itemName.toLocaleLowerCase() === name.toLocaleLowerCase())
      .sort((a, b) => Number(a[0]) - Number(b[0]));
    return matches[0]?.[0] || null;
  }

  function baseBuildNames(details) {
    const name = details?.name || '';
    const tags = details?.tags || [];
    const info = details?.info || {};
    const overrides = {
      Briar: ["Plated Steelcaps", "Stridebreaker", "Sundered Sky", "Black Cleaver", "Sterak's Gage", "Spirit Visage"],
      Garen: ["Plated Steelcaps", "Stridebreaker", "Trinity Force", "Sterak's Gage", "Dead Man's Plate", "Force of Nature"],
      Lux: ["Sorcerer's Shoes", "Luden's Echo", "Shadowflame", "Rabadon's Deathcap", "Void Staff", "Zhonya's Hourglass"],
      Ashe: ["Berserker's Greaves", "Kraken Slayer", "Infinity Edge", "Runaan's Hurricane", "Lord Dominik's Regards", "Bloodthirster"],
    };
    if (overrides[name]) return [...overrides[name]];
    if (tags.includes('Marksman')) {
      return ["Berserker's Greaves", "Kraken Slayer", "Infinity Edge", "Rapid Firecannon", "Lord Dominik's Regards", "Bloodthirster"];
    }
    if (Number(info.magic || 0) >= 7 || tags[0] === 'Mage') {
      return ["Sorcerer's Shoes", "Luden's Echo", "Shadowflame", "Rabadon's Deathcap", "Void Staff", "Zhonya's Hourglass"];
    }
    if (tags[0] === 'Tank' || (tags.includes('Tank') && Number(info.defense || 0) >= 7)) {
      return ["Plated Steelcaps", "Sunfire Aegis", "Jak'Sho, The Protean", "Thornmail", "Kaenic Rookern", "Warmog's Armor"];
    }
    if (tags.includes('Support')) {
      return ["Ionian Boots of Lucidity", "Shurelya's Battlesong", "Redemption", "Locket of the Iron Solari", "Mikael's Blessing", "Dawncore"];
    }
    if (tags.includes('Assassin')) {
      return ["Ionian Boots of Lucidity", "Youmuu's Ghostblade", "Edge of Night", "Black Cleaver", "Serylda's Grudge", "Guardian Angel"];
    }
    return ["Plated Steelcaps", "Trinity Force", "Sundered Sky", "Black Cleaver", "Sterak's Gage", "Death's Dance"];
  }

  function adaptBuildNames(names, details, enemyDetails) {
    if (!enemyDetails.length) {
      return { names, reason: 'No enemy picks are visible yet, so Aura used the champion template.' };
    }
    const magicThreat = enemyDetails.reduce((sum, entry) => sum + Number(entry.info?.magic || 0), 0);
    const physicalThreat = enemyDetails.reduce((sum, entry) => sum + Number(entry.info?.attack || 0), 0);
    const tankCount = enemyDetails.filter((entry) => entry.tags?.includes('Tank')).length;
    const isMagicBuild = Number(details.info?.magic || 0) > Number(details.info?.attack || 0);
    const isTankBuild = details.tags?.includes('Tank') && Number(details.info?.defense || 0) >= 7;
    const adapted = [...names];
    const reasons = [];

    if (magicThreat > physicalThreat + 4) {
      adapted[0] = "Mercury's Treads";
      adapted[5] = isTankBuild ? "Kaenic Rookern" : (isMagicBuild ? "Banshee's Veil" : "Maw of Malmortius");
      reasons.push('extra magic resistance');
    } else if (physicalThreat > magicThreat + 4) {
      adapted[0] = "Plated Steelcaps";
      adapted[5] = isTankBuild ? "Randuin's Omen" : (isMagicBuild ? "Zhonya's Hourglass" : "Guardian Angel");
      reasons.push('extra armor');
    }
    if (tankCount >= 2) {
      adapted[4] = isMagicBuild ? "Void Staff" : (isTankBuild ? "Abyssal Mask" : "Black Cleaver");
      reasons.push('tank penetration');
    }
    return {
      names: adapted,
      reason: reasons.length
        ? `Adapted for ${reasons.join(' and ')} from ${enemyDetails.length} visible enemy pick${enemyDetails.length === 1 ? '' : 's'}.`
        : `Enemy picks checked; the standard build remains the best fit for their current damage mix.`,
    };
  }

  function renderBuild() {
    $('build-slots').innerHTML = Array.from({ length: 6 }, (_, index) => {
      const itemId = state.build[index];
      return itemId
        ? `<button class="build-slot filled" data-remove-item="${index}" title="Remove ${escapeHtml(state.itemMap[itemId])}"><img src="${ddragonImg(`img/item/${itemId}.png`)}" alt=""><span>${escapeHtml(state.itemMap[itemId])}</span></button>`
        : '<div class="build-slot"><span>Empty slot</span></div>';
    }).join('');
  }

  async function applyRecommendedBuild(showToast = true) {
    if (!state.selectedChampion || !state.selectedChampionDetails) {
      setMessage('build-recommendation-status', 'Choose a champion first.', true);
      return;
    }
    setMessage('build-recommendation-status', `Preparing ${state.selectedChampion.name}'s Aura build…`);
    let recommendation = {
      names: baseBuildNames(state.selectedChampionDetails),
      reason: 'Aura champion template loaded.',
    };
    if ($('adapt-enemy-build')?.checked && state.enemyChampionIds.length) {
      const enemies = state.enemyChampionIds
        .map((id) => championEntries().find((entry) => entry.numericId === String(id)))
        .filter(Boolean);
      const settled = await Promise.allSettled(enemies.map((champion) => getChampionDetails(champion)));
      const enemyDetails = settled
        .filter((entry) => entry.status === 'fulfilled')
        .map((entry) => entry.value);
      recommendation = adaptBuildNames(recommendation.names, state.selectedChampionDetails, enemyDetails);
    } else if ($('adapt-enemy-build')?.checked) {
      recommendation.reason = 'Enemy adaptation is on. Aura will update after enemy picks become visible in Champion Select.';
    }

    const itemIds = recommendation.names.map(itemIdByName).filter(Boolean).slice(0, 6);
    if (!itemIds.length) {
      setMessage('build-recommendation-status', 'Item data is not ready yet. Try again in a few seconds.', true);
      return;
    }
    state.build = itemIds;
    renderBuild();
    setMessage(
      'build-recommendation-status',
      `${state.selectedChampion.name}: ${itemIds.length}/6 items ready. ${recommendation.reason}`
    );
    if (showToast) toast(`${state.selectedChampion.name} build applied.`, 'success');
  }

  function renderItemCatalog(query) {
    if (!$('item-catalog')) return;
    const normalized = query.trim().toLocaleLowerCase();
    const uniqueByName = new Map();
    Object.entries(state.itemMap).forEach(([id, name]) => {
      const key = name.trim().toLocaleLowerCase();
      const existing = uniqueByName.get(key);
      if (key && (!existing || Number(id) < Number(existing[0]))) uniqueByName.set(key, [id, name]);
    });
    const entries = [...uniqueByName.values()]
      .filter(([, name]) => !normalized || name.toLocaleLowerCase().includes(normalized))
      .sort((a, b) => a[1].localeCompare(b[1]));
    $('item-result-count').textContent = `${entries.length} shown`;
    $('item-catalog').innerHTML = entries.length ? entries.map(([id, name]) => `
      <button class="item-tile" data-item-id="${escapeHtml(id)}" title="Add ${escapeHtml(name)}">
        <img src="${ddragonImg(`img/item/${id}.png`)}" alt="" loading="lazy"><span>${escapeHtml(name)}</span>
      </button>
    `).join('') : '<div class="empty-state">No item matches that search.</div>';
  }

  $('item-search')?.addEventListener('input', (event) => renderItemCatalog(event.target.value));
  $('item-catalog')?.addEventListener('click', (event) => {
    const tile = event.target.closest('[data-item-id]');
    if (!tile) return;
    if (state.build.length >= 6) return toast('Your build already has six items.', 'error');
    state.build.push(tile.dataset.itemId);
    renderBuild();
  });
  $('build-slots')?.addEventListener('click', (event) => {
    const slot = event.target.closest('[data-remove-item]');
    if (!slot) return;
    state.build.splice(Number(slot.dataset.removeItem), 1);
    renderBuild();
  });
  $('clear-build')?.addEventListener('click', () => { state.build = []; renderBuild(); });
  $('apply-recommended-build')?.addEventListener('click', () => {
    applyRecommendedBuild(true).catch((error) => setMessage('build-recommendation-status', String(error), true));
  });
  $('adapt-enemy-build')?.addEventListener('change', () => {
    applyRecommendedBuild(false).catch((error) => setMessage('build-recommendation-status', String(error), true));
  });

  const RUNE_TREE_COLORS = {
    8000: '#d8b45b',
    8100: '#ef4d5b',
    8200: '#7797ff',
    8300: '#45bfd3',
    8400: '#79bf72',
  };
  const RUNE_TREE_ORDER = [8000, 8100, 8200, 8400, 8300];

  const STAT_SHARD_ROWS = [
    {
      label: 'Offense',
      options: [
        { id: 5008, name: 'Adaptive Force', icon: 'perk-images/StatMods/StatModsAdaptiveForceIcon.png' },
        { id: 5005, name: 'Attack Speed', icon: 'perk-images/StatMods/StatModsAttackSpeedIcon.png' },
        { id: 5007, name: 'Ability Haste', icon: 'perk-images/StatMods/StatModsCDRScalingIcon.png' },
      ],
    },
    {
      label: 'Flex',
      options: [
        { id: 5008, name: 'Adaptive Force', icon: 'perk-images/StatMods/StatModsAdaptiveForceIcon.png' },
        { id: 5010, name: 'Move Speed', icon: 'perk-images/StatMods/StatModsMovementSpeedIcon.png' },
        { id: 5001, name: 'Health Scaling', icon: 'perk-images/StatMods/StatModsHealthPlusIcon.png' },
      ],
    },
    {
      label: 'Defense',
      options: [
        { id: 5011, name: 'Health', icon: 'perk-images/StatMods/StatModsHealthScalingIcon.png' },
        { id: 5013, name: 'Tenacity and Slow Resist', icon: 'perk-images/StatMods/StatModsTenacityIcon.png' },
        { id: 5001, name: 'Health Scaling', icon: 'perk-images/StatMods/StatModsHealthPlusIcon.png' },
      ],
    },
  ];

  function recommendedRunePage(details) {
    const tags = details?.tags || [];
    const info = details?.info || {};
    if (tags[0] === 'Tank' || (tags.includes('Tank') && Number(info.defense || 0) >= 7)) {
      return {
        primaryTreeId: 8400,
        primary: [8437, 8446, 8473, 8451],
        secondaryTreeId: 8300,
        secondary: [8304, 8345],
        shards: [5007, 5001, 5001],
      };
    }
    if (tags.includes('Support')) {
      return {
        primaryTreeId: 8200,
        primary: [8214, 8226, 8210, 8236],
        secondaryTreeId: 8400,
        secondary: [8463, 8453],
        shards: [5007, 5010, 5011],
      };
    }
    if (tags.includes('Marksman')) {
      return {
        primaryTreeId: 8000,
        primary: [8005, 9111, 9104, 8017],
        secondaryTreeId: 8300,
        secondary: [8304, 8345],
        shards: [5005, 5008, 5011],
      };
    }
    if (Number(info.magic || 0) >= 7 || tags[0] === 'Mage') {
      return {
        primaryTreeId: 8200,
        primary: [8229, 8226, 8210, 8237],
        secondaryTreeId: 8300,
        secondary: [8304, 8345],
        shards: [5008, 5008, 5011],
      };
    }
    if (tags.includes('Assassin') && !tags.includes('Fighter')) {
      return {
        primaryTreeId: 8100,
        primary: [8112, 8143, 8140, 8106],
        secondaryTreeId: 8200,
        secondary: [8233, 8237],
        shards: [5008, 5008, 5011],
      };
    }
    return {
      primaryTreeId: 8000,
      primary: [8010, 9111, 9104, 8299],
      secondaryTreeId: 8400,
      secondary: [8473, 8453],
      shards: [5005, 5008, 5011],
    };
  }

  function treeById(treeId) {
    return state.runeTrees.find((tree) => tree.id === Number(treeId));
  }

  function firstTreeExcept(treeId) {
    return orderedRuneTrees().find((tree) => tree.id !== Number(treeId));
  }

  function orderedRuneTrees() {
    return [...state.runeTrees].sort(
      (left, right) => RUNE_TREE_ORDER.indexOf(left.id) - RUNE_TREE_ORDER.indexOf(right.id)
    );
  }

  function ensureRunePagePaths() {
    let primary = treeById(state.runePage.primaryTreeId);
    if (!primary) {
      primary = treeById(8000) || state.runeTrees[0];
      state.runePage.primaryTreeId = primary?.id ?? null;
      state.runePage.primary.clear();
    }
    let secondary = treeById(state.runePage.secondaryTreeId);
    if (!secondary || secondary.id === primary?.id) {
      secondary = (primary?.id !== 8400 ? treeById(8400) : null) || firstTreeExcept(primary?.id);
      state.runePage.secondaryTreeId = secondary?.id ?? null;
      state.runePage.secondary.clear();
      state.runePage.secondaryOrder = [];
    }
  }

  function runeFromSlot(tree, slotIndex, runeId) {
    return tree?.slots?.[slotIndex]?.runes?.find((rune) => rune.id === Number(runeId));
  }

  function defaultSecondarySelections(tree) {
    return [1, 2]
      .map((slotIndex) => ({ slotIndex, rune: tree?.slots?.[slotIndex]?.runes?.[0] }))
      .filter((selection) => selection.rune);
  }

  function clearRuneSelections() {
    state.runePage.primary.clear();
    state.runePage.secondary.clear();
    state.runePage.secondaryOrder = [];
    state.runePage.shards.clear();
  }

  function applyRecommendedRunes(showToast = true) {
    if (!state.selectedChampionDetails) {
      setMessage('rune-recommendation-status', 'Choose a champion first.', true);
      return;
    }
    const recommendation = recommendedRunePage(state.selectedChampionDetails);
    state.runePage.primaryTreeId = recommendation.primaryTreeId;
    state.runePage.secondaryTreeId = recommendation.secondaryTreeId;
    clearRuneSelections();
    ensureRunePagePaths();

    const primaryTree = treeById(state.runePage.primaryTreeId);
    primaryTree?.slots?.slice(0, 4).forEach((slot, slotIndex) => {
      const rune = runeFromSlot(primaryTree, slotIndex, recommendation.primary[slotIndex])
        || slot.runes?.[0];
      if (rune) state.runePage.primary.set(slotIndex, rune);
    });

    const secondaryTree = treeById(state.runePage.secondaryTreeId);
    const secondarySelections = recommendation.secondary
      .map((runeId) => {
        const slotIndex = secondaryTree?.slots
          ?.slice(1, 4)
          .findIndex((slot) => slot.runes.some((rune) => rune.id === runeId));
        const normalizedSlot = Number(slotIndex) + 1;
        return {
          slotIndex: normalizedSlot,
          rune: runeFromSlot(secondaryTree, normalizedSlot, runeId),
        };
      })
      .filter((selection) => selection.rune && selection.slotIndex > 0);
    const validSecondary = secondarySelections.length === 2
      && secondarySelections[0].slotIndex !== secondarySelections[1].slotIndex
      ? secondarySelections
      : defaultSecondarySelections(secondaryTree);
    validSecondary.slice(0, 2).forEach(({ slotIndex, rune }) => {
      state.runePage.secondary.set(slotIndex, rune);
      state.runePage.secondaryOrder.push(slotIndex);
    });

    STAT_SHARD_ROWS.forEach((row, rowIndex) => {
      const shard = row.options.find((option) => option.id === recommendation.shards[rowIndex])
        || row.options[0];
      state.runePage.shards.set(rowIndex, shard);
    });

    renderRuneTrees();
    setMessage(
      'rune-recommendation-status',
      `${state.selectedChampion.name}'s Aura rune page is ready. Rates below use only your loaded recent matches.`
    );
    if (showToast) toast(`${state.selectedChampion.name} runes applied.`, 'success');
  }

  function runeRate(runeId) {
    if (!state.selectedChampion) return 'WR — · Pick —';
    const games = state.profileMatches.filter(
      (match) => match.champion_name === state.selectedChampion.imageId
    );
    if (!games.length) return 'WR — · Pick —';
    const used = games.filter((match) => match.perk_ids?.includes(runeId));
    const pickRate = Math.round(used.length / games.length * 100);
    const winRate = used.length
      ? `${Math.round(used.filter((match) => match.win).length / used.length * 100)}%`
      : '—';
    return `WR ${winRate} · Pick ${pickRate}%`;
  }

  function renderPathPicker(role, selectedTreeId) {
    return `
      <div class="rune-path-picker" role="tablist" aria-label="${role === 'primary' ? 'Primary' : 'Secondary'} rune path">
        ${orderedRuneTrees().map((tree) => {
          const selected = tree.id === Number(selectedTreeId);
          const disabled = role === 'secondary' && tree.id === Number(state.runePage.primaryTreeId);
          return `<button class="rune-path-button${selected ? ' selected' : ''}" data-rune-path-role="${role}" data-tree-id="${tree.id}" aria-label="${escapeHtml(tree.name)}" aria-selected="${selected}" title="${escapeHtml(tree.name)}" ${disabled ? 'disabled' : ''} style="--path-color:${RUNE_TREE_COLORS[tree.id] || '#a855f7'}">
            <img src="${runeImg(tree.icon)}" alt="">
          </button>`;
        }).join('')}
      </div>
    `;
  }

  function renderRuneOption(rune, tree, slotIndex, role, selected, isKeystone = false) {
    return `<button class="rune-option${selected ? ' selected' : ''}${isKeystone ? ' keystone' : ''}" data-rune-role="${role}" data-tree-id="${tree.id}" data-slot-index="${slotIndex}" data-rune-id="${rune.id}" aria-pressed="${selected}" title="${escapeHtml(`${rune.name}: ${plainText(rune.short_desc)}`)}">
      <span class="rune-icon-ring"><img src="${runeImg(rune.icon)}" alt=""></span>
      <span class="rune-option-name">${escapeHtml(rune.name)}</span>
      <small>${runeRate(rune.id)}</small>
    </button>`;
  }

  function renderPrimaryTree(tree) {
    return `
      <section class="rune-column primary-runes" style="--path-color:${RUNE_TREE_COLORS[tree.id] || '#a855f7'}">
        <div class="rune-column-heading">
          <img src="${runeImg(tree.icon)}" alt="">
          <div><span>Primary path</span><h3>${escapeHtml(tree.name)}</h3></div>
        </div>
        ${tree.slots.slice(0, 4).map((slot, slotIndex) => `
          <div class="rune-row${slotIndex === 0 ? ' keystone-row' : ''}">
            <span class="rune-row-label">${slotIndex === 0 ? 'Keystones' : `Row ${slotIndex}`}</span>
            <div class="rune-row-options">
              ${slot.runes.map((rune) => renderRuneOption(
                rune,
                tree,
                slotIndex,
                'primary',
                state.runePage.primary.get(slotIndex)?.id === rune.id,
                slotIndex === 0
              )).join('')}
            </div>
          </div>
        `).join('')}
      </section>
    `;
  }

  function renderStatShards() {
    return `
      <section class="stat-shards">
        <div class="secondary-instructions"><strong>Stat shards</strong><span>Choose one from each row</span></div>
        ${STAT_SHARD_ROWS.map((row, rowIndex) => `
          <div class="shard-row">
            <span>${escapeHtml(row.label)}</span>
            <div>
              ${row.options.map((shard) => {
                const selected = state.runePage.shards.get(rowIndex)?.id === shard.id;
                return `<button class="shard-option${selected ? ' selected' : ''}" data-shard-row="${rowIndex}" data-shard-id="${shard.id}" aria-pressed="${selected}" title="${escapeHtml(`${shard.name} · ${runeRate(shard.id)}`)}">
                  <img src="${runeImg(shard.icon)}" alt=""><span>${escapeHtml(shard.name)}</span>
                </button>`;
              }).join('')}
            </div>
          </div>
        `).join('')}
      </section>
    `;
  }

  function renderSecondaryTree(tree) {
    return `
      <section class="rune-column secondary-runes" style="--path-color:${RUNE_TREE_COLORS[tree.id] || '#a855f7'}">
        <div class="rune-column-heading">
          <img src="${runeImg(tree.icon)}" alt="">
          <div><span>Secondary path</span><h3>${escapeHtml(tree.name)}</h3></div>
        </div>
        <div class="secondary-instructions"><strong>Secondary</strong><span>Choose two runes from different rows</span></div>
        ${tree.slots.slice(1, 4).map((slot, offset) => {
          const slotIndex = offset + 1;
          return `<div class="rune-row secondary-row">
            <span class="rune-row-label">Row ${slotIndex}</span>
            <div class="rune-row-options">
              ${slot.runes.map((rune) => renderRuneOption(
                rune,
                tree,
                slotIndex,
                'secondary',
                state.runePage.secondary.get(slotIndex)?.id === rune.id
              )).join('')}
            </div>
          </div>`;
        }).join('')}
        ${renderStatShards()}
      </section>
    `;
  }

  function renderRuneTrees() {
    if (!state.runeTrees.length) return;
    ensureRunePagePaths();
    const primaryTree = treeById(state.runePage.primaryTreeId);
    const secondaryTree = treeById(state.runePage.secondaryTreeId);
    if (!primaryTree || !secondaryTree) return;
    $('rune-trees').innerHTML = `
      <section class="card rune-builder">
        <div class="rune-picker-grid">
          <div>
            <span class="rune-picker-label">Choose primary path</span>
            ${renderPathPicker('primary', primaryTree.id)}
          </div>
          <div>
            <span class="rune-picker-label">Choose secondary path</span>
            ${renderPathPicker('secondary', secondaryTree.id)}
          </div>
        </div>
        <div class="rune-page-grid">
          ${renderPrimaryTree(primaryTree)}
          ${renderSecondaryTree(secondaryTree)}
        </div>
      </section>
    `;
    const championGames = state.selectedChampion
      ? state.profileMatches.filter((match) => match.champion_name === state.selectedChampion.imageId).length
      : 0;
    $('rune-rate-sample').textContent = championGames
      ? `Personal sample: ${championGames} recent ${state.selectedChampion.name} game${championGames === 1 ? '' : 's'}.`
      : 'Load Profile matches to calculate honest personal win and pick rates. Global rates are not supplied by Riot Data Dragon.';
    renderRuneSummary();
  }

  function renderRuneSummary() {
    const primaryTree = treeById(state.runePage.primaryTreeId);
    const secondaryTree = treeById(state.runePage.secondaryTreeId);
    const primaryNames = [...state.runePage.primary.entries()]
      .sort(([left], [right]) => left - right)
      .map(([, rune]) => rune.name);
    const secondaryNames = [...state.runePage.secondary.entries()]
      .sort(([left], [right]) => left - right)
      .map(([, rune]) => rune.name);
    const shardNames = [...state.runePage.shards.entries()]
      .sort(([left], [right]) => left - right)
      .map(([, shard]) => shard.name);
    const selectedCount = primaryNames.length + secondaryNames.length + shardNames.length;
    if (!selectedCount) {
      $('rune-summary').textContent = 'No runes selected. A complete page has 4 primary runes, 2 secondary runes, and 3 stat shards.';
      return;
    }
    $('rune-summary').textContent = `${selectedCount}/9 selected · ${primaryTree?.name || 'Primary'}: ${primaryNames.join(', ') || 'incomplete'} · ${secondaryTree?.name || 'Secondary'}: ${secondaryNames.join(', ') || 'incomplete'} · Shards: ${shardNames.join(', ') || 'incomplete'}`;
  }

  $('rune-trees')?.addEventListener('click', (event) => {
    const pathButton = event.target.closest('[data-rune-path-role]');
    if (pathButton) {
      const role = pathButton.dataset.runePathRole;
      const treeId = Number(pathButton.dataset.treeId);
      if (role === 'primary' && treeId !== state.runePage.primaryTreeId) {
        state.runePage.primaryTreeId = treeId;
        state.runePage.primary.clear();
        if (treeId === state.runePage.secondaryTreeId) {
          state.runePage.secondaryTreeId = firstTreeExcept(treeId)?.id ?? null;
          state.runePage.secondary.clear();
          state.runePage.secondaryOrder = [];
        }
      } else if (role === 'secondary' && treeId !== state.runePage.primaryTreeId) {
        state.runePage.secondaryTreeId = treeId;
        state.runePage.secondary.clear();
        state.runePage.secondaryOrder = [];
      }
      renderRuneTrees();
      return;
    }

    const shardButton = event.target.closest('[data-shard-id]');
    if (shardButton) {
      const rowIndex = Number(shardButton.dataset.shardRow);
      const shard = STAT_SHARD_ROWS[rowIndex]?.options
        .find((option) => option.id === Number(shardButton.dataset.shardId));
      if (shard) state.runePage.shards.set(rowIndex, shard);
      renderRuneTrees();
      return;
    }

    const button = event.target.closest('[data-rune-id]');
    if (!button) return;
    const tree = state.runeTrees.find((entry) => entry.id === Number(button.dataset.treeId));
    const slotIndex = Number(button.dataset.slotIndex);
    const rune = runeFromSlot(tree, slotIndex, button.dataset.runeId);
    if (!rune) return;
    if (button.dataset.runeRole === 'primary') {
      state.runePage.primary.set(slotIndex, rune);
    } else if (button.dataset.runeRole === 'secondary') {
      const existing = state.runePage.secondary.get(slotIndex);
      if (existing?.id === rune.id) {
        state.runePage.secondary.delete(slotIndex);
        state.runePage.secondaryOrder = state.runePage.secondaryOrder
          .filter((row) => row !== slotIndex);
      } else {
        if (!state.runePage.secondary.has(slotIndex) && state.runePage.secondary.size >= 2) {
          const oldestRow = state.runePage.secondaryOrder.shift();
          state.runePage.secondary.delete(oldestRow);
          toast('Secondary runes must use two different rows. Aura replaced the oldest row.', 'info');
        }
        state.runePage.secondary.set(slotIndex, rune);
        state.runePage.secondaryOrder = state.runePage.secondaryOrder
          .filter((row) => row !== slotIndex);
        state.runePage.secondaryOrder.push(slotIndex);
      }
    }
    renderRuneTrees();
  });
  $('clear-runes')?.addEventListener('click', () => { clearRuneSelections(); renderRuneTrees(); });
  $('apply-recommended-runes')?.addEventListener('click', () => applyRecommendedRunes(true));

  // Integration configuration ------------------------------------------------
  function applyIntegrationConfig(config) {
    state.integration = config;
    const riotText = config.riot_api_configured
      ? `Configured via ${config.riot_api_source}`
      : 'No Riot API key saved';
    setIntegrationStatus('riot-config-status', riotText, config.riot_api_configured ? 'ready' : 'missing');
    setIntegrationStatus('settings-riot-status', riotText, config.riot_api_configured ? 'ready' : 'missing');
    $('btn-load-matches').dataset.configReady = String(config.riot_api_configured);

    const spotifyText = config.spotify_configured
      ? `OAuth ready · ${config.spotify_redirect_uri}`
      : 'Spotify OAuth configuration needs attention';
    setIntegrationStatus('spotify-config-status', spotifyText, config.spotify_configured ? 'ready' : 'missing', config.spotify_error || '');
    setIntegrationStatus('settings-spotify-status', spotifyText, config.spotify_configured ? 'ready' : 'missing', config.spotify_error || '');
    $('spotify-redirect-value').textContent = config.spotify_redirect_uri || 'Unavailable';
    $('spotify-scopes-value').textContent = config.spotify_scopes.join(' ') || 'Unavailable';
    [$('btn-login'), $('settings-connect-spotify')].forEach((button) => {
      if (button) button.disabled = !config.spotify_configured;
    });
    maybeAutoLoadLocalProfile();
  }

  async function refreshIntegrationConfig() {
    try {
      applyIntegrationConfig(await invoke('get_integration_config'));
    } catch (error) {
      logErr('CONFIG', error);
      setIntegrationStatus('settings-riot-status', 'Could not read configuration', 'missing');
      setIntegrationStatus('settings-spotify-status', 'Could not read configuration', 'missing');
    }
  }

  $('save-riot-key')?.addEventListener('click', async () => {
    const keyInput = $('riot-api-key');
    const apiKey = keyInput.value.trim();
    if (!apiKey) return toast('Paste a fresh Riot API key first.', 'error');
    const button = $('save-riot-key');
    button.disabled = true;
    try {
      const config = await invoke('save_riot_api_key', { apiKey });
      keyInput.value = '';
      applyIntegrationConfig(config);
      toast('Riot key saved securely. No restart is needed.', 'success');
    } catch (error) {
      toast(error, 'error');
      logErr('RIOT-CONFIG', error);
    } finally {
      button.disabled = false;
    }
  });

  $('clear-riot-key')?.addEventListener('click', async () => {
    try {
      const config = await invoke('clear_riot_api_key');
      applyIntegrationConfig(config);
      toast('Saved Riot credential cleared.', 'success');
    } catch (error) {
      toast(error, 'error');
    }
  });

  // Spotify ------------------------------------------------------------------
  function spotifyStatus(message, isError = false) {
    setMessage('spotify-action-status', message, isError);
    if (isError) toast(message, 'error');
  }

  async function refreshSpotifyDevices(showMessages = true) {
    try {
      const devices = await invoke('spotify_devices');
      const select = $('spotify-device-select');
      select.innerHTML = devices.length
        ? devices.map((device) => `<option value="${escapeHtml(device.id || '')}"${device.is_active ? ' selected' : ''}>${escapeHtml(device.name)} · ${escapeHtml(device.device_type)}${device.is_active ? ' (active)' : ''}</option>`).join('')
        : '<option value="">No active devices found</option>';
      if (!devices.length && showMessages) {
        spotifyStatus('No playback device found. Start Aura Player to play on this PC without Spotify Desktop, or activate an external Spotify client.', true);
      } else if (showMessages) {
        spotifyStatus(`${devices.length} Spotify device${devices.length === 1 ? '' : 's'} found.`);
      }
      return devices;
    } catch (error) {
      if (showMessages) spotifyStatus(String(error), true);
      throw error;
    }
  }

  async function refreshSpotifyPlayback(showErrors = false) {
    try {
      const playback = await invoke('spotify_playback_status');
      state.spotifyConnected = true;
      $('spotify-track').textContent = playback.track_name || 'No track is active';
      $('spotify-artist').textContent = playback.artists.length
        ? `${playback.artists.join(', ')} · ${playback.device?.name || 'No device'}`
        : 'Choose a device and start playback.';
      if (playback.device?.id) $('spotify-device-select').value = playback.device.id;
      return playback;
    } catch (error) {
      if (showErrors) spotifyStatus(String(error), true);
      throw error;
    }
  }

  async function connectSpotify() {
    const buttons = [$('btn-login'), $('settings-connect-spotify')].filter(Boolean);
    buttons.forEach((button) => { button.disabled = true; button.textContent = 'Waiting for browser…'; });
    spotifyStatus('Waiting for Spotify authorization. Keep Aura open until the browser says it connected.');
    try {
      await invoke('spotify_login');
      state.spotifyConnected = true;
      buttons.forEach((button) => { button.textContent = 'Connected'; });
      spotifyStatus('Spotify connected. Start Aura Player to use this PC, or choose an existing device.');
      await refreshSpotifyDevices(false);
      await refreshSpotifyPlayback(false);
      toast('Spotify account connected.', 'success');
    } catch (error) {
      buttons.forEach((button) => { button.disabled = false; button.textContent = 'Retry Connect'; });
      spotifyStatus(String(error), true);
      logErr('SPOTIFY-LOGIN', error);
    }
  }

  function renderEmbeddedSpotifyStatus(status, announce = false) {
    if (!status) return;
    [$('btn-aura-player'), $('settings-aura-player')].filter(Boolean).forEach((button) => {
      button.disabled = Boolean(status.running);
      button.textContent = status.running
        ? (status.mode === 'compatible_browser' ? 'Browser Player Running' : 'Aura Player Running')
        : 'Start Aura Player';
    });
    [$('btn-browser-player'), $('settings-browser-player')].filter(Boolean).forEach((button) => {
      button.hidden = !status.fallback_available;
      button.disabled = status.mode === 'compatible_browser';
    });
    if ($('btn-stop-aura-player')) $('btn-stop-aura-player').disabled = !status.running;
    if (status.device_id) {
      refreshSpotifyDevices(false).then(() => {
        if ($('spotify-device-select').querySelector(`option[value="${CSS.escape(status.device_id)}"]`)) {
          $('spotify-device-select').value = status.device_id;
        }
      }).catch(() => {});
    }
    if (announce || status.activated || status.error) {
      spotifyStatus(status.message || 'Aura Player status updated.', Boolean(status.error));
    }
  }

  async function startAuraPlayer() {
    spotifyStatus('Starting Aura Player in an isolated window…');
    try {
      const status = await invokeWithTimeout('spotify_start_embedded_player', {}, 15000);
      state.spotifyConnected = true;
      renderEmbeddedSpotifyStatus(status, true);
    } catch (error) {
      spotifyStatus(String(error), true);
      logErr('SPOTIFY-PLAYER', error);
    }
  }

  async function startBrowserAuraPlayer() {
    spotifyStatus('Opening Aura Player in your default browser…');
    try {
      const status = await invokeWithTimeout('spotify_start_browser_player', {}, 15000);
      state.spotifyConnected = true;
      renderEmbeddedSpotifyStatus(status, true);
    } catch (error) {
      spotifyStatus(String(error), true);
      logErr('SPOTIFY-BROWSER-PLAYER', error);
    }
  }

  async function stopAuraPlayer() {
    try {
      const status = await invoke('spotify_stop_embedded_player');
      renderEmbeddedSpotifyStatus(status, true);
      await refreshSpotifyDevices(false).catch(() => []);
    } catch (error) {
      spotifyStatus(String(error), true);
    }
  }

  async function spotifyControl(command, successMessage) {
    try {
      const deviceId = $('spotify-device-select').value || null;
      await invokeWithTimeout(command, { deviceId }, 15000);
      spotifyStatus(successMessage);
      state.spotifyConnected = true;
      setTimeout(() => refreshSpotifyPlayback(false).catch(() => {}), 400);
    } catch (error) {
      spotifyStatus(String(error), true);
      logErr('SPOTIFY', error);
    }
  }

  $('btn-login')?.addEventListener('click', connectSpotify);
  $('settings-connect-spotify')?.addEventListener('click', connectSpotify);
  $('btn-play')?.addEventListener('click', () => spotifyControl('spotify_play', 'Playback started.'));
  $('btn-pause')?.addEventListener('click', () => spotifyControl('spotify_pause', 'Playback paused.'));
  $('btn-prev')?.addEventListener('click', () => spotifyControl('spotify_previous', 'Previous track.'));
  $('btn-skip')?.addEventListener('click', () => spotifyControl('spotify_skip', 'Skipped to next track.'));
  $('btn-refresh-devices')?.addEventListener('click', () => refreshSpotifyDevices(true).catch(() => {}));
  $('btn-aura-player')?.addEventListener('click', startAuraPlayer);
  $('settings-aura-player')?.addEventListener('click', startAuraPlayer);
  $('btn-browser-player')?.addEventListener('click', startBrowserAuraPlayer);
  $('settings-browser-player')?.addEventListener('click', startBrowserAuraPlayer);
  $('btn-stop-aura-player')?.addEventListener('click', stopAuraPlayer);
  $('btn-transfer-device')?.addEventListener('click', async () => {
    const deviceId = $('spotify-device-select').value;
    if (!deviceId) return spotifyStatus('Choose a Spotify device first.', true);
    try {
      await invoke('spotify_transfer', { deviceId });
      spotifyStatus('Playback moved to the selected device.');
      setTimeout(() => refreshSpotifyPlayback(false).catch(() => {}), 400);
    } catch (error) {
      spotifyStatus(String(error), true);
    }
  });
  function openSpotifyWeb() {
    invoke('spotify_open_web_player')
      .then(() => spotifyStatus('External Spotify Web Player opened. Start playback there, then refresh devices.'))
      .catch((error) => spotifyStatus(String(error), true));
  }
  $('btn-open-spotify')?.addEventListener('click', openSpotifyWeb);
  $('settings-open-spotify')?.addEventListener('click', openSpotifyWeb);
  listen('spotify-embedded-status', (event) => renderEmbeddedSpotifyStatus(event.payload, true))
    .catch((error) => logErr('SPOTIFY-PLAYER', error));
  invoke('spotify_embedded_status')
    .then((status) => renderEmbeddedSpotifyStatus(status, false))
    .catch((error) => logErr('SPOTIFY-PLAYER', error));
  setInterval(() => {
    if (state.spotifyConnected && !document.hidden) refreshSpotifyPlayback(false).catch(() => {});
  }, 5000);

  // Riot match history -------------------------------------------------------
  const queueNames = {
    420: 'Ranked Solo/Duo', 440: 'Ranked Flex', 400: 'Normal Draft',
    430: 'Normal Blind', 450: 'ARAM', 490: 'Quickplay',
  };

  function renderResolvedRiotId(gameName, tagLine, rankText = 'Loading profile…') {
    $('profile-name').textContent = `${gameName}#${tagLine}`;
    $('profile-rank').textContent = rankText;
  }

  function renderSummonerProfile(profile, gameName, tagLine) {
    const normalizedProfile = { ...profile };
    state.summonerProfile = normalizedProfile;
    renderResolvedRiotId(
      gameName,
      tagLine,
      state.rankStatus === 'loading' ? 'Loading rank…' : $('profile-rank').textContent,
    );
    $('profile-level').textContent = `Level ${Number(normalizedProfile.summoner_level) || 0}`;
    applyKnownProfileIcon();
    renderHomeProfileSummary();
  }

  function renderLeagueEntries(entries) {
    state.rankedEntries = normalizeRankedEntries(entries);
    state.rankStatus = 'ready';
    state.rankError = '';
    const { solo, flex } = selectRankedEntries(state.rankedEntries);
    const rankText = solo ? `${rankLabel(solo)} · ${rankDetail(solo)}` : 'Unranked Solo/Duo';
    $('profile-rank').textContent = rankText;
    $('profile-solo-rank').textContent = rankText;
    $('profile-flex-rank').textContent = flex ? `${rankLabel(flex)} · ${rankDetail(flex)}` : 'Unranked Flex';
    renderRankFlags('profile-solo-flags', solo);
    renderRankFlags('profile-flex-flags', flex);
    renderHomeProfileSummary();
  }

  function renderLeagueEntriesError(error) {
    state.rankedEntries = [];
    state.rankStatus = 'error';
    state.rankError = String(error);
    $('profile-rank').textContent = 'Rank temporarily unavailable';
    $('profile-solo-rank').textContent = 'Solo/Duo rank unavailable';
    $('profile-flex-rank').textContent = 'Flex rank unavailable';
    renderRankFlags('profile-solo-flags', null);
    renderRankFlags('profile-flex-flags', null);
    renderHomeProfileSummary();
  }

  function profileTargetKey(target) {
    return [target.puuid || '', target.gameName || '', target.tagLine || '', target.platform || '']
      .map((value) => String(value).trim().toLocaleLowerCase())
      .join('|');
  }

  function fillRiotIdForm(target) {
    if (!target) return;
    if (target.gameName) $('riot-game-name').value = target.gameName;
    if (target.tagLine) $('riot-tag-line').value = target.tagLine;
    if (target.platform) $('riot-platform').value = String(target.platform).toLocaleLowerCase();
  }

  function targetFromForm(source = 'manual') {
    return {
      gameName: $('riot-game-name').value.trim(),
      tagLine: $('riot-tag-line').value.trim(),
      platform: $('riot-platform').value,
      puuid: '',
      source,
    };
  }

  function queueProfileLoad(target, options = {}) {
    if (!target?.platform || (!target.puuid && (!target.gameName || !target.tagLine))) {
      setMessage('match-status', 'Enter a Riot ID and server, or start the League Client for automatic detection.', true);
      return Promise.resolve();
    }
    const waitingOnPrevious = $('btn-load-matches').disabled
      && $('btn-load-matches').textContent === 'Loading…';
    const generation = ++state.profileLoadGeneration;
    const queuedTarget = { ...target };
    resetProfilePresentation();
    state.profileTarget = queuedTarget;
    if (queuedTarget.gameName && queuedTarget.tagLine) {
      renderResolvedRiotId(
        queuedTarget.gameName,
        queuedTarget.tagLine,
        waitingOnPrevious ? 'Waiting for previous Riot request…' : 'Loading profile…',
      );
    } else {
      $('profile-name').textContent = 'Resolving Riot profile…';
      $('profile-rank').textContent = waitingOnPrevious
        ? 'Waiting for previous Riot request…'
        : 'Loading profile…';
    }
    renderHomeProfileSummary();
    fillRiotIdForm(queuedTarget);
    if ($('btn-load-matches').dataset.configReady !== 'true') {
      $('profile-rank').textContent = 'Riot API key required';
      $('match-list').className = 'empty-state error-text';
      $('match-list').innerHTML = '<div>Save a fresh Riot API key in Settings to load this profile.</div>';
      setMessage('match-status', 'Account selected. Save a fresh Riot API key in Settings to load rank, mastery, and matches.', true);
      $('btn-load-matches').disabled = false;
      $('btn-load-matches').textContent = 'Load Profile';
      if (options.navigateToSettings) navigate('settings');
      return Promise.resolve();
    }
    if (waitingOnPrevious) {
      setMessage('match-status', 'Waiting for the previous Riot request to finish before loading this account.');
    }
    $('btn-load-matches').disabled = true;
    $('btn-load-matches').textContent = 'Loading…';
    state.profileLoadChain = state.profileLoadChain
      .catch((error) => logErr('RIOT QUEUE', error))
      .then(() => runProfileLoad(queuedTarget, generation));
    return state.profileLoadChain;
  }

  async function runProfileLoad(target, generation) {
    if (generation !== state.profileLoadGeneration) return;
    const isCurrent = () => generation === state.profileLoadGeneration;
    const requestedCount = Math.min(20, Math.max(10, Number($('match-count')?.value) || 10));
    let gameName = String(target.gameName || '').trim();
    let tagLine = String(target.tagLine || '').trim();
    const platform = String(target.platform || '').toLocaleLowerCase();

    fillRiotIdForm({ gameName, tagLine, platform });
    if ($('match-count')) $('match-count').disabled = true;
    $('match-list').className = 'match-list loading';
    $('match-list').innerHTML = `
      <div class="match-loading-state" role="status">
        <span class="match-loading-spinner" aria-hidden="true"></span>
        <div><strong>Loading recent matches</strong><span>Resolving match data from Riot…</span></div>
      </div>`;
    setMessage('match-status', target.source === 'local'
      ? 'Step 1/2: League account detected. Verifying the Riot ID…'
      : 'Step 1/2: Resolving the selected Riot ID…');

    try {
      if (target.puuid) {
        const selected = await invokeWithTimeout('select_riot_profile', {
          puuid: target.puuid,
          platform,
          fallbackGameName: gameName,
          fallbackTagLine: tagLine,
        }, 18000);
        gameName = selected.game_name || gameName;
        tagLine = selected.tag_line || tagLine;
      } else {
        await invokeWithTimeout('set_riot_id', { gameName, tagLine, platform }, 15000);
      }
      if (!isCurrent()) return;
      const resolvedTarget = {
        puuid: target.puuid || '', gameName, tagLine, platform, source: target.source || 'manual',
      };
      state.profileTarget = resolvedTarget;
      fillRiotIdForm(resolvedTarget);
      state.rankStatus = 'loading';
      state.rankError = '';
      renderResolvedRiotId(gameName, tagLine, 'Loading rank…');
      $('profile-solo-rank').textContent = 'Loading Solo/Duo rank…';
      $('profile-flex-rank').textContent = 'Loading Flex rank…';
      renderHomeProfileSummary();
      void loadChampionMasteries(resolvedTarget, generation);
      setMessage(
        'match-status',
        `Step 2/2: Loading level, rank, mastery, and ${requestedCount} recent matches independently…`,
      );

      const profileTask = invokeWithTimeout('get_summoner_profile', {}, 24000)
        .then((profile) => {
          if (!isCurrent()) return { stale: true };
          renderSummonerProfile(profile, gameName, tagLine);
          return { ok: true };
        })
        .catch((error) => {
          if (!isCurrent()) return { stale: true };
          $('profile-level').textContent = 'Level unavailable';
          renderHomeProfileSummary();
          logErr('RIOT PROFILE', error);
          return { ok: false, error };
        });

      const rankTask = invokeWithTimeout('get_league_entries', {}, 24000)
        .then((entries) => {
          if (!isCurrent()) return { stale: true };
          renderLeagueEntries(entries);
          return { ok: true };
        })
        .catch((error) => {
          if (!isCurrent()) return { stale: true };
          renderLeagueEntriesError(error);
          logErr('RIOT RANK', error);
          return { ok: false, error };
        });

      const matchesTask = invokeWithTimeout(
        'fetch_recent_matches',
        { count: requestedCount },
        40000,
      )
        .then((payload) => {
          if (!isCurrent()) return { stale: true };
          const matches = Array.isArray(payload) ? payload : [];
          state.profileMatches = matches;
          destroyAllMapReplays();
          state.matchDetails.clear();
          state.matchDetailRequests.clear();
          state.matchTimelines.clear();
          state.matchTimelineRequests.clear();
          if (!matches.length) {
            $('match-list').className = 'empty-state';
            $('match-list').innerHTML = '<div>No recent matches were found for this account and queue selection.</div>';
          } else {
            renderMatches(matches);
            renderPerformance(matches);
          }
          if (state.currentPage === 'runes') renderRuneTrees();
          return { ok: true, count: matches.length };
        })
        .catch((error) => {
          if (!isCurrent()) return { stale: true };
          $('match-list').className = 'empty-state error-text';
          $('match-list').innerHTML = '<div>Match history is temporarily unavailable. Level, rank, and mastery can still load above.</div>';
          logErr('RIOT MATCHES', error);
          return { ok: false, error };
        });

      const [profileResult, rankResult, matchesResult] = await Promise.all([
        profileTask,
        rankTask,
        matchesTask,
      ]);
      if (!isCurrent()) return;

      const failures = [
        ['level', profileResult],
        ['rank', rankResult],
        ['match history', matchesResult],
      ].filter(([, result]) => !result.ok);
      const rejectedKey = failures.some(([, result]) => {
        const message = String(result.error || '').toLocaleLowerCase();
        return message.includes('expired') || message.includes('401') || message.includes('403');
      });
      if (rejectedKey) {
        setIntegrationStatus('riot-config-status', 'Riot rejected this key — replace it in Settings', 'missing');
        setIntegrationStatus('settings-riot-status', 'Riot rejected this key — replace it', 'missing');
      }

      const loadedText = matchesResult.ok
        ? matchesResult.count
          ? `Loaded ${matchesResult.count} real matches for ${gameName}#${tagLine}.`
          : 'Riot returned no recent matches.'
        : 'Match history could not be loaded.';
      const failureText = failures.length
        ? ` Unavailable right now: ${failures.map(([label]) => label).join(', ')}.`
        : '';
      setMessage('match-status', `${loadedText}${failureText}`, failures.length > 0);
    } catch (error) {
      if (!isCurrent()) return;
      const rawMessage = String(error);
      const message = rawMessage.toLowerCase().includes('timed out')
        ? `${rawMessage}. Aura stopped waiting; check your connection and try again.`
        : rawMessage;
      setMessage('match-status', message, true);
      $('profile-rank').textContent = 'Profile unavailable';
      if (message.toLowerCase().includes('expired') || message.includes('401') || message.includes('403')) {
        setIntegrationStatus('riot-config-status', 'Riot rejected this key — replace it in Settings', 'missing');
        setIntegrationStatus('settings-riot-status', 'Riot rejected this key — replace it', 'missing');
      }
      logErr('RIOT', error);
      if (!state.profileMatches.length) {
        $('match-list').className = 'empty-state error-text';
        $('match-list').innerHTML = '<div>Match history could not be loaded. Check the message above, then try again.</div>';
      }
    } finally {
      if (isCurrent()) {
        $('btn-load-matches').disabled = false;
        $('btn-load-matches').textContent = 'Load Profile';
        if ($('match-count')) $('match-count').disabled = false;
      }
    }
  }

  function loadMatches() {
    return queueProfileLoad(targetFromForm('manual'), { navigateToSettings: true });
  }

  function maybeAutoLoadLocalProfile() {
    const target = state.localRiotAccount;
    if (!target || !state.integration) return;
    fillRiotIdForm(target);
    const key = profileTargetKey(target);
    if (state.autoProfileLoadedKey === key || state.profileTarget?.source === 'match') return;
    if (!state.integration.riot_api_configured) {
      setMessage('match-status', `Detected ${target.gameName}#${target.tagLine}. Save a Riot API key in Settings to load the profile.`);
      return;
    }
    state.autoProfileLoadedKey = key;
    queueProfileLoad({ ...target, source: 'local' });
  }

  function handleLocalRiotAccount(payload) {
    if (!payload || !payload.puuid || !payload.platform) return;
    const target = {
      puuid: String(payload.puuid),
      gameName: String(payload.game_name || ''),
      tagLine: String(payload.tag_line || ''),
      platform: String(payload.platform).toLocaleLowerCase(),
      profile_icon_id: Number(payload.profile_icon_id) || 0,
      summoner_level: Number(payload.summoner_level) || 0,
      source: 'local',
    };
    state.localRiotAccount = target;
    $('btn-my-profile').disabled = false;
    if (!state.profileTarget || state.profileTarget.source !== 'match') {
      fillRiotIdForm(target);
      if (target.gameName && target.tagLine && !state.profileTarget) {
        renderResolvedRiotId(target.gameName, target.tagLine, 'League account detected');
      }
    }
    if (!state.summonerProfile && (!state.profileTarget || state.profileTarget.source === 'local')) {
      applyKnownProfileIcon();
      if (target.summoner_level) $('profile-level').textContent = `Level ${target.summoner_level}`;
      renderHomeProfileSummary();
    }
    maybeAutoLoadLocalProfile();
  }

  function renderPerformance(matches) {
    if (!matches.length) return;
    const wins = matches.filter((match) => match.win).length;
    const winRate = Math.round(wins / matches.length * 100);
    const avgKda = matches.reduce((sum, match) => sum + (match.kills + match.assists) / Math.max(1, match.deaths), 0) / matches.length;
    const avgCs = matches.reduce((sum, match) => sum + match.cs / Math.max(1, match.game_duration_secs / 60), 0) / matches.length;
    const avgVision = matches.reduce((sum, match) => sum + match.vision_score, 0) / matches.length;
    const kpValues = matches
      .map((match) => percentValue(match.kill_participation))
      .filter((value) => value !== null);
    const dpmValues = matches
      .map((match) => {
        const damage = metricNumber(match, [
          'total_damage_dealt_to_champions', 'damage_to_champions', 'champion_damage',
        ], null);
        const minutes = metricNumber(match, ['game_duration_secs'], 0) / 60;
        return damage !== null && minutes > 0 ? damage / minutes : null;
      })
      .filter((value) => value !== null);
    $('profile-winrate').textContent = `${winRate}%`;
    $('profile-kda').textContent = avgKda.toFixed(1);
    $('profile-cs').textContent = avgCs.toFixed(1);
    $('profile-vision').textContent = Math.round(avgVision);
    $('profile-kp').textContent = kpValues.length
      ? `${Math.round(kpValues.reduce((sum, value) => sum + value, 0) / kpValues.length)}%`
      : '—';
    $('profile-dpm').textContent = dpmValues.length
      ? Math.round(dpmValues.reduce((sum, value) => sum + value, 0) / dpmValues.length).toLocaleString()
      : '—';
  }

  function metricNumber(source, keys, fallback = 0) {
    for (const key of keys) {
      const value = source?.[key];
      if (value === null || value === undefined || value === '') continue;
      const parsed = Number(value);
      if (Number.isFinite(parsed)) return parsed;
    }
    return fallback;
  }

  function percentValue(value) {
    if (value === null || value === undefined || value === '') return null;
    const parsed = Number(value);
    if (!Number.isFinite(parsed)) return null;
    // Aura's Rust match model serializes percentages in the 0..=100 range.
    // Treating values <= 1 as fractions turns a legitimate 1% KP into 100%.
    return Math.max(0, Math.min(100, parsed));
  }

  function compactNumber(value) {
    if (value === null || value === undefined || value === '') return '—';
    const parsed = Number(value);
    if (!Number.isFinite(parsed)) return '—';
    return new Intl.NumberFormat(undefined, {
      notation: parsed >= 10000 ? 'compact' : 'standard',
      maximumFractionDigits: parsed >= 10000 ? 1 : 0,
    }).format(parsed);
  }

  function formatDuration(seconds) {
    const total = Math.max(0, Math.round(Number(seconds) || 0));
    const minutes = Math.floor(total / 60);
    const remainder = total % 60;
    return `${minutes}:${String(remainder).padStart(2, '0')}`;
  }

  function championDisplayName(imageId) {
    const numericId = Object.keys(state.imageIdByNumericId)
      .find((id) => state.imageIdByNumericId[id] === imageId);
    return numericId ? state.championMap[numericId] : (imageId || 'Unknown champion');
  }

  function championImage(imageId) {
    return ddragonImg(`img/champion/${encodeURIComponent(String(imageId || ''))}.png`);
  }

  function championImageIdFromNumericId(numericId) {
    return state.imageIdByNumericId[String(numericId)] || '';
  }

  function queueCategory(match) {
    if ([420, 440].includes(Number(match.queue_id))) return 'ranked';
    if (Number(match.queue_id) === 450 || String(match.game_mode).toUpperCase() === 'ARAM') return 'aram';
    return 'normal';
  }

  function visibleMatchEntries(matches) {
    const queueFilter = $('match-queue-filter')?.value || 'all';
    const resultFilter = $('match-result-filter')?.value || 'all';
    return matches
      .map((match, index) => ({ match, index }))
      .filter(({ match }) => queueFilter === 'all' || queueCategory(match) === queueFilter)
      .filter(({ match }) => resultFilter === 'all'
        || (resultFilter === 'win' ? Boolean(match.win) : !match.win));
  }

  function matchItems(items, className = '') {
    const normalized = Array.isArray(items) ? items.filter((id) => Number(id) > 0).slice(0, 7) : [];
    const icons = normalized.map((id) => {
      const itemId = Math.max(0, Math.trunc(Number(id) || 0));
      const name = state.itemMap[itemId] || `Item ${itemId}`;
      return `<img src="${ddragonImg(`img/item/${itemId}.png`)}" alt="${escapeHtml(name)}" title="${escapeHtml(name)}" loading="lazy">`;
    });
    while (icons.length < 7) icons.push('<span class="match-item-empty" aria-hidden="true"></span>');
    return `<div class="match-item-strip ${className}">${icons.join('')}</div>`;
  }

  function renderMatches(matches) {
    const list = $('match-list');
    destroyAllMapReplays();
    const entries = visibleMatchEntries(matches);
    list.className = 'match-list';
    if (!entries.length) {
      list.className = 'empty-state';
      list.innerHTML = '<div>No matches match these filters. Change Queue or Result to see more games.</div>';
      return;
    }
    list.innerHTML = entries.map(({ match, index }) => {
      const displayName = championDisplayName(match.champion_name);
      const duration = metricNumber(match, ['game_duration_secs'], 0);
      const kdaRatio = metricNumber(match, ['kda'], null)
        ?? ((metricNumber(match, ['kills']) + metricNumber(match, ['assists']))
          / Math.max(1, metricNumber(match, ['deaths'])));
      const csm = metricNumber(match, ['csm'], null)
        ?? (duration > 0 ? metricNumber(match, ['cs']) / (duration / 60) : null);
      const kp = percentValue(match.kill_participation);
      const damage = metricNumber(match, [
        'total_damage_dealt_to_champions', 'damage_to_champions', 'champion_damage',
      ], null);
      const matchId = String(match.match_id || `match-${index}`);
      return `
        <article class="match-card ${match.win ? 'victory' : 'defeat'}">
          <button class="match-summary-toggle" type="button" data-match-toggle="${index}"
            aria-expanded="false" aria-controls="match-detail-${index}">
            <span class="match-result-block">
              <strong class="match-result ${match.win ? 'victory' : 'defeat'}">${match.win ? 'Victory' : 'Defeat'}</strong>
              <span>${escapeHtml(queueNames[match.queue_id] || match.game_mode || 'League match')}</span>
              <span>${timeAgo(match.game_creation_ms)} · ${formatDuration(duration)}</span>
            </span>
            <span class="match-champion-block">
              <img class="match-champion" src="${championImage(match.champion_name)}" alt="${escapeHtml(displayName)}">
              <span><strong>${escapeHtml(displayName)}</strong><small>${escapeHtml(match.team_position || match.position || 'Role unavailable')}</small></span>
            </span>
            <span class="match-summary-stat">
              <strong>${metricNumber(match, ['kills'])} / ${metricNumber(match, ['deaths'])} / ${metricNumber(match, ['assists'])}</strong>
              <span>${kdaRatio.toFixed(2)} KDA</span>
            </span>
            <span class="match-summary-stat">
              <strong>${metricNumber(match, ['cs'])} CS</strong>
              <span>${csm === null ? '—' : csm.toFixed(1)} / min</span>
            </span>
            <span class="match-summary-stat">
              <strong>${kp === null ? '—' : `${Math.round(kp)}%`}</strong>
              <span>Kill participation</span>
            </span>
            <span class="match-summary-stat match-damage-summary">
              <strong>${damage === null ? '—' : compactNumber(damage)}</strong>
              <span>Champion damage</span>
            </span>
            ${matchItems(match.items, 'match-summary-items')}
            <span class="match-expand-indicator" aria-hidden="true">⌄</span>
            <span class="sr-only">Open full details for ${escapeHtml(matchId)}</span>
          </button>
          <div id="match-detail-${index}" class="match-detail-panel" data-match-detail="${index}" hidden></div>
        </article>`;
    }).join('');
  }

  const SUMMONER_SPELLS = {
    1: ['Cleanse', 'SummonerBoost.png'],
    3: ['Exhaust', 'SummonerExhaust.png'],
    4: ['Flash', 'SummonerFlash.png'],
    6: ['Ghost', 'SummonerHaste.png'],
    7: ['Heal', 'SummonerHeal.png'],
    11: ['Smite', 'SummonerSmite.png'],
    12: ['Teleport', 'SummonerTeleport.png'],
    13: ['Clarity', 'SummonerMana.png'],
    14: ['Ignite', 'SummonerDot.png'],
    21: ['Barrier', 'SummonerBarrier.png'],
    32: ['Mark', 'SummonerSnowball.png'],
  };

  function summonerSpellStrip(ids) {
    const values = Array.isArray(ids) ? ids.slice(0, 2) : [];
    if (!values.length) return '<span class="match-data-unavailable">Spells —</span>';
    return `<span class="match-spell-strip">${values.map((rawId) => {
      const id = Math.trunc(Number(rawId) || 0);
      const [name, image] = SUMMONER_SPELLS[id] || [`Spell ${id}`, ''];
      return image
        ? `<img src="${ddragonImg(`img/spell/${image}`)}" alt="${escapeHtml(name)}" title="${escapeHtml(name)}" loading="lazy">`
        : `<span title="${escapeHtml(name)}">${id}</span>`;
    }).join('')}</span>`;
  }

  function findRune(runeId) {
    for (const tree of state.runeTrees) {
      for (const slot of tree.slots || []) {
        const rune = (slot.runes || []).find((entry) => Number(entry.id) === Number(runeId));
        if (rune) return rune;
      }
    }
    return STAT_SHARD_ROWS
      .flatMap((row) => row.options)
      .find((entry) => Number(entry.id) === Number(runeId));
  }

  function runeStrip(ids) {
    const runes = (Array.isArray(ids) ? ids : [])
      .map((id) => findRune(id))
      .filter(Boolean)
      .slice(0, 9);
    if (!runes.length) return '<span class="match-data-unavailable">Rune data unavailable</span>';
    return `<div class="match-rune-strip">${runes.map((rune) => `
      <img src="${runeImg(rune.icon)}" alt="${escapeHtml(rune.name)}" title="${escapeHtml(rune.name)}" loading="lazy">
    `).join('')}</div>`;
  }

  function participantName(player) {
    if (player.riot_id) return String(player.riot_id);
    const gameName = player.riot_id_game_name || player.game_name || player.summoner_name;
    const tagLine = player.riot_id_tagline || player.tag_line;
    if (gameName) return `${gameName}${tagLine ? `#${tagLine}` : ''}`;
    return 'Unknown player';
  }

  function participantKp(player, teamKills) {
    const reported = percentValue(player.kill_participation);
    if (reported !== null) return reported;
    return teamKills > 0
      ? Math.min(100, ((metricNumber(player, ['kills']) + metricNumber(player, ['assists'])) / teamKills) * 100)
      : 0;
  }

  function normalizedParticipants(detail) {
    const players = detail?.all_participants || detail?.participants || [];
    return Array.isArray(players) ? players : [];
  }

  function participantTeamId(player) {
    return Math.trunc(metricNumber(player, ['team_id', 'teamId'], 0));
  }

  function renderParticipantRow(player, teamKills, selectedPuuid, platform) {
    const championName = player.champion_name || player.championName || '';
    const displayName = championDisplayName(championName);
    const kills = metricNumber(player, ['kills']);
    const deaths = metricNumber(player, ['deaths']);
    const assists = metricNumber(player, ['assists']);
    const cs = metricNumber(player, ['cs'], null)
      ?? (metricNumber(player, ['total_minions_killed', 'totalMinionsKilled'])
        + metricNumber(player, ['neutral_minions_killed', 'neutralMinionsKilled']));
    const duration = metricNumber(player, ['game_duration_secs'], 0);
    const csm = metricNumber(player, ['csm'], null);
    const damage = metricNumber(player, [
      'total_damage_dealt_to_champions', 'damage_to_champions', 'champion_damage',
    ], null);
    const vision = metricNumber(player, ['vision_score', 'visionScore'], null);
    const kp = participantKp(player, teamKills);
    const isMe = Boolean(player.is_me)
      || (selectedPuuid && String(player.puuid || '') === String(selectedPuuid));
    const spellIds = player.summoner_spell_ids
      || [player.summoner1_id, player.summoner2_id].filter(Boolean);
    const playerRunes = Array.isArray(player.perk_ids) ? player.perk_ids : [];
    const puuid = String(player.puuid || '');
    const gameName = String(player.game_name || player.riot_id_game_name || '');
    const tagLine = String(player.tag_line || player.riot_id_tag_line || '');
    const canOpenProfile = Boolean(puuid && platform);
    const profileAttributes = canOpenProfile
      ? `data-player-profile data-player-puuid="${escapeHtml(puuid)}" data-player-game-name="${escapeHtml(gameName)}" data-player-tag-line="${escapeHtml(tagLine)}" data-player-platform="${escapeHtml(platform)}"`
      : 'disabled';
    return `
      <tr class="${isMe ? 'is-me' : ''}">
        <th scope="row">
          <span class="scoreboard-player">
            <button class="scoreboard-profile-link" type="button" ${profileAttributes}
              aria-label="Open ${escapeHtml(participantName(player))} profile">
              <img src="${championImage(championName)}" alt="${escapeHtml(displayName)}" loading="lazy">
              <span><strong>${escapeHtml(participantName(player))}${isMe ? ' (You)' : ''}</strong><small>${escapeHtml(player.team_position || player.position || displayName)}</small></span>
            </button>
            <span class="scoreboard-loadout-icons">
              ${summonerSpellStrip(spellIds)}
              ${runeStrip(playerRunes.slice(0, 2))}
            </span>
          </span>
        </th>
        <td><strong>${kills} / ${deaths} / ${assists}</strong><small>${((kills + assists) / Math.max(1, deaths)).toFixed(2)} KDA</small></td>
        <td><strong>${Math.round(kp)}%</strong><small>KP</small></td>
        <td><strong>${cs}</strong><small>${csm !== null ? csm.toFixed(1) : (duration > 0 ? (cs / (duration / 60)).toFixed(1) : '—')} / min</small></td>
        <td><strong>${compactNumber(metricNumber(player, ['gold', 'gold_earned', 'goldEarned'], null))}</strong><small>Gold</small></td>
        <td><strong>${compactNumber(damage)}</strong><small>Damage</small></td>
        <td><strong>${compactNumber(vision)}</strong><small>Vision</small></td>
        <td>${matchItems(player.items, 'scoreboard-items')}</td>
      </tr>`;
  }

  function renderTeamScoreboard(detail, participants, teamId, fallbackLabel) {
    const teamPlayers = participants.filter((player) => participantTeamId(player) === teamId);
    if (!teamPlayers.length) return '';
    const teamKills = teamPlayers.reduce((sum, player) => sum + metricNumber(player, ['kills']), 0);
    const team = (Array.isArray(detail.teams) ? detail.teams : [])
      .find((entry) => Math.trunc(metricNumber(entry, ['team_id', 'teamId'], 0)) === teamId);
    const won = typeof team?.win === 'boolean'
      ? team.win
      : String(team?.win || '').toLowerCase() === 'win';
    const label = teamId === 100 ? 'Blue team' : teamId === 200 ? 'Red team' : fallbackLabel;
    return `
      <section class="match-scoreboard-team ${won ? 'winner' : ''}" aria-label="${escapeHtml(label)} scoreboard">
        <div class="match-team-heading">
          <h4>${escapeHtml(label)}</h4>
          <span>${teamKills} kills · ${won ? 'Victory' : 'Defeat'}</span>
        </div>
        <div class="scoreboard-scroll" tabindex="0" aria-label="Scrollable ${escapeHtml(label)} statistics">
          <table class="match-scoreboard">
            <caption class="sr-only">${escapeHtml(label)} player statistics</caption>
            <thead><tr><th>Player</th><th>KDA</th><th>KP</th><th>CS</th><th>Gold</th><th>Damage</th><th>Vision</th><th>Items</th></tr></thead>
            <tbody>${teamPlayers.map((player) => renderParticipantRow(
              player,
              teamKills,
              detail.selected_puuid || detail.puuid,
              String(detail.platform_id || state.profileTarget?.platform || '')
            )).join('')}</tbody>
          </table>
        </div>
      </section>`;
  }

  function objectiveCount(team, names) {
    for (const name of names) {
      const value = team?.objectives?.[name] ?? team?.[name];
      if (value && typeof value === 'object') {
        const count = metricNumber(value, ['kills', 'count'], null);
        if (count !== null) return count;
      }
      if (value !== null && value !== undefined && Number.isFinite(Number(value))) return Number(value);
    }
    return 0;
  }

  function renderObjectives(detail) {
    const teams = Array.isArray(detail.teams) ? detail.teams : [];
    if (!teams.length) return '<div class="match-data-unavailable">Team objective data is unavailable for this match.</div>';
    const definitions = [
      ['Champions', ['champion', 'champions']],
      ['Towers', ['tower', 'towers']],
      ['Dragons', ['dragon', 'dragons']],
      ['Barons', ['baron', 'barons']],
      ['Heralds', ['riftHerald', 'rift_herald', 'herald']],
      ['Voidgrubs', ['horde', 'void_grubs', 'voidGrubs']],
      ['Atakhan', ['atakhan']],
      ['Inhibitors', ['inhibitor', 'inhibitors']],
    ];
    return `<div class="match-objective-grid">${teams.map((team) => {
      const teamId = Math.trunc(metricNumber(team, ['team_id', 'teamId'], 0));
      const label = teamId === 100 ? 'Blue team' : teamId === 200 ? 'Red team' : `Team ${teamId}`;
      const bans = Array.isArray(team.bans) ? team.bans : [];
      return `
        <section class="match-objective-team">
          <h4>${escapeHtml(label)} objectives</h4>
          <dl>${definitions.map(([name, keys]) => `<div><dt>${name}</dt><dd>${objectiveCount(team, keys)}</dd></div>`).join('')}</dl>
          <div class="match-bans"><span>Bans</span>${bans.length ? bans.map((ban) => {
            const champion = ban.champion_name || ban.championName;
            const championId = metricNumber(ban, ['champion_id', 'championId'], null);
            const imageId = champion || championImageIdFromNumericId(championId);
            return imageId
              ? `<img src="${championImage(imageId)}" alt="${escapeHtml(championDisplayName(imageId))}" title="${escapeHtml(championDisplayName(imageId))}" loading="lazy">`
              : `<span>${championId ?? '—'}</span>`;
          }).join('') : '<small>None reported</small>'}</div>
        </section>`;
    }).join('')}</div>`;
  }

  function renderSelectedMatchReport(detail, summary) {
    const participants = normalizedParticipants(detail);
    const selected = detail.selected_player
      || detail.participant
      || participants.find((player) => player.is_me)
      || summary;
    const duration = metricNumber(detail, ['game_duration_secs'], metricNumber(summary, ['game_duration_secs']));
    const kills = metricNumber(selected, ['kills']);
    const deaths = metricNumber(selected, ['deaths']);
    const assists = metricNumber(selected, ['assists']);
    const cs = metricNumber(selected, ['cs'], metricNumber(summary, ['cs']));
    const teamPlayers = participants.filter((player) => participantTeamId(player) === participantTeamId(selected));
    const teamKills = teamPlayers.reduce((sum, player) => sum + metricNumber(player, ['kills']), 0);
    const kp = participantKp(selected, teamKills);
    const damage = metricNumber(selected, [
      'total_damage_dealt_to_champions', 'damage_to_champions', 'champion_damage',
    ], null);
    const damageTaken = metricNumber(selected, ['total_damage_taken', 'damage_taken'], null);
    const objectiveDamage = metricNumber(selected, [
      'damage_dealt_to_objectives', 'objective_damage', 'damage_to_objectives',
    ], null);
    const controlWards = metricNumber(selected, [
      'control_wards', 'vision_wards_bought_in_game', 'detector_wards_placed',
    ], null);
    const wardLine = [
      metricNumber(selected, ['wards_placed'], null),
      metricNumber(selected, ['wards_killed'], null),
    ];
    const reportMetrics = [
      ['KDA', `${kills} / ${deaths} / ${assists}`, `${((kills + assists) / Math.max(1, deaths)).toFixed(2)} ratio`],
      ['CS / minute', duration > 0 ? (cs / (duration / 60)).toFixed(1) : '—', `${cs} total CS`],
      ['Kill participation', participants.length ? `${Math.round(kp)}%` : (percentValue(summary.kill_participation) === null ? '—' : `${Math.round(percentValue(summary.kill_participation))}%`), 'Team takedowns'],
      ['Champion damage', compactNumber(damage), damage !== null && duration > 0 ? `${compactNumber(damage / (duration / 60))} / min` : 'Unavailable'],
      ['Gold earned', compactNumber(metricNumber(selected, ['gold', 'gold_earned', 'goldEarned'], metricNumber(summary, ['gold'], null))), 'Economy'],
      ['Vision score', compactNumber(metricNumber(selected, ['vision_score', 'visionScore'], metricNumber(summary, ['vision_score'], null))), controlWards === null ? 'Control wards —' : `${controlWards} control wards`],
      ['Damage taken', compactNumber(damageTaken), 'Durability'],
      ['Objective damage', compactNumber(objectiveDamage), 'Structures and monsters'],
      ['Wards', wardLine.every((value) => value === null) ? '—' : `${wardLine[0] ?? '—'} / ${wardLine[1] ?? '—'}`, 'Placed / cleared'],
    ];
    const participantTeams = [...new Set(participants.map(participantTeamId).filter(Boolean))];
    const teamIds = participantTeams.length ? participantTeams : [100, 200];
    return `
      <div class="match-detail-content">
        <div class="match-detail-header">
          <div><span class="eyebrow">Complete match report</span><h3>${escapeHtml(queueNames[summary.queue_id] || summary.game_mode || 'League match')} · ${formatDuration(duration)}</h3></div>
          <span class="match-id-label">${escapeHtml(detail.match_id || summary.match_id || '')}</span>
        </div>
        <div class="match-detail-metrics">${reportMetrics.map(([label, value, note]) => `
          <div><span>${escapeHtml(label)}</span><strong>${escapeHtml(value)}</strong><small>${escapeHtml(note)}</small></div>
        `).join('')}</div>
        <div class="match-loadout-grid">
          <section><h4>Final items</h4>${matchItems(selected.items || summary.items, 'match-detail-items')}</section>
          <section><h4>Runes</h4>${runeStrip(selected.perk_ids || summary.perk_ids)}</section>
          <section><h4>Summoner spells</h4>${summonerSpellStrip(
            selected.summoner_spell_ids
              || [selected.summoner1_id, selected.summoner2_id].filter(Boolean)
          )}</section>
        </div>
        ${participants.length
          ? `<div class="match-scoreboards">${teamIds.map((teamId, index) =>
            renderTeamScoreboard(detail, participants, teamId, `Team ${index + 1}`)).join('')}</div>`
          : '<div class="match-data-unavailable">The compact summary is available, but Riot did not return the ten-player scoreboard for this match.</div>'}
        <section class="match-map-replay">
          <div class="match-detail-section-heading">
            <div><span class="eyebrow">Post-match timeline</span><h3>Dynamic Map Control Replay</h3></div>
            <span class="map-replay-source">Match-V5 Timeline · RAM only</span>
          </div>
          <div class="map-replay-root" data-map-replay-root data-replay-match="${escapeHtml(detail.match_id || summary.match_id || '')}">
            <div class="map-replay-loading" role="status"><span class="match-loading-spinner" aria-hidden="true"></span><span>Waiting for positional timeline…</span></div>
          </div>
        </section>
        <section class="match-objectives">
          <div class="match-detail-section-heading"><span class="eyebrow">Final totals</span><h3>Team objectives</h3></div>
          ${renderObjectives(detail)}
        </section>
      </div>`;
  }

  function destroyMapReplay(matchId) {
    const key = String(matchId || '');
    const controller = state.mapReplayControllers.get(key);
    if (!controller) return;
    try { controller.destroy(); } catch (error) { logErr('MAP REPLAY', error); }
    state.mapReplayControllers.delete(key);
  }

  function destroyAllMapReplays() {
    [...state.mapReplayControllers.keys()].forEach(destroyMapReplay);
  }

  function cachedMapTimeline(matchId) {
    const replay = state.matchTimelines.get(matchId);
    if (!replay) return null;
    state.matchTimelines.delete(matchId);
    state.matchTimelines.set(matchId, replay);
    return replay;
  }

  function cacheMapTimeline(matchId, replay) {
    state.matchTimelines.delete(matchId);
    state.matchTimelines.set(matchId, replay);
    while (state.matchTimelines.size > MAX_TIMELINE_CACHE) {
      const oldest = state.matchTimelines.keys().next().value;
      if (!oldest) break;
      state.matchTimelines.delete(oldest);
    }
  }

  function renderReplayError(root, index, error) {
    if (!root) return;
    root.innerHTML = `
      <div class="map-replay-error" role="alert">
        <div><strong>Could not load the positional timeline</strong><span>${escapeHtml(String(error))}</span></div>
        <button class="btn-secondary compact" type="button" data-retry-replay="${index}">Retry replay</button>
      </div>`;
  }

  async function loadMapReplayModule() {
    if (state.mapReplayModule) return state.mapReplayModule;
    if (!state.mapReplayModuleRequest) {
      state.mapReplayModuleRequest = import('./map-control-replay.js')
        .then((module) => {
          state.mapReplayModule = module;
          return module;
        })
        .finally(() => { state.mapReplayModuleRequest = null; });
    }
    return state.mapReplayModuleRequest;
  }

  async function mountReplay(root, replay, matchId, isCurrent) {
    const { mountMapControlReplay } = await loadMapReplayModule();
    if (!root.isConnected || !isCurrent()) return;
    destroyMapReplay(matchId);
    const controller = mountMapControlReplay(root, replay, {
      championImage,
      currentDdragonVersion: state.ddragonVersion,
    });
    if (!isCurrent()) {
      controller.destroy();
      return;
    }
    state.mapReplayControllers.set(matchId, controller);
  }

  async function loadMapReplay(
    index,
    detail,
    summary,
    force = false,
    expectedPanel = null,
    profileGeneration = state.profileLoadGeneration,
  ) {
    const panel = $(`match-detail-${index}`);
    const root = panel?.querySelector('[data-map-replay-root]');
    const matchId = String(detail?.match_id || summary?.match_id || '');
    if (!panel || !root || (expectedPanel && panel !== expectedPanel)) return;
    if (!matchId) {
      root.innerHTML = '<div class="map-replay-unavailable"><strong>Replay unavailable</strong><span>This match has no Riot match identifier.</span></div>';
      return;
    }
    const sameProfile = () => profileGeneration === state.profileLoadGeneration
      && String(state.profileMatches[index]?.match_id || '') === matchId
      && $(`match-detail-${index}`) === panel
      && root.dataset.replayMatch === matchId;
    if (!sameProfile()) return;
    if (force) {
      destroyMapReplay(matchId);
      state.matchTimelines.delete(matchId);
      state.matchTimelineRequests.delete(matchId);
    }
    const stillCurrent = () => sameProfile()
      && panel.isConnected
      && !panel.hidden
      && panel.querySelector('[data-map-replay-root]') === root
      && document.querySelector(`[data-match-toggle="${index}"]`)?.getAttribute('aria-expanded') === 'true';
    const cached = cachedMapTimeline(matchId);
    if (cached) {
      try {
        if (stillCurrent()) await mountReplay(root, cached, matchId, stillCurrent);
      } catch (error) {
        state.matchTimelines.delete(matchId);
        if (stillCurrent()) renderReplayError(root, index, error);
        logErr('MAP REPLAY', error);
      }
      return;
    }
    root.innerHTML = '<div class="map-replay-loading" role="status"><span class="match-loading-spinner" aria-hidden="true"></span><span>Loading Riot positional frames and objective events…</span></div>';
    let request;
    try {
      request = state.matchTimelineRequests.get(matchId);
      if (!request) {
        request = invokeWithTimeout('get_match_timeline', { matchId }, 16000);
        state.matchTimelineRequests.set(matchId, request);
      }
      const replay = await request;
      if (state.matchTimelineRequests.get(matchId) === request) {
        state.matchTimelineRequests.delete(matchId);
      }
      if (!sameProfile()) return;
      if (!replay || typeof replay !== 'object') throw new Error('Riot returned an empty timeline.');
      cacheMapTimeline(matchId, replay);
      if (stillCurrent()) await mountReplay(root, replay, matchId, stillCurrent);
    } catch (error) {
      const activeRequest = state.matchTimelineRequests.get(matchId);
      if (!activeRequest || activeRequest === request) state.matchTimelineRequests.delete(matchId);
      if (stillCurrent()) renderReplayError(root, index, error);
      logErr('MAP REPLAY', error);
    }
  }

  function hydrateMatchReport(index, detail, summary, panel, profileGeneration) {
    loadMapReplay(index, detail, summary, false, panel, profileGeneration)
      .catch((error) => logErr('MAP REPLAY', error));
  }

  function closeExpandedMatches(exceptIndex = null) {
    document.querySelectorAll('[data-match-toggle][aria-expanded="true"]').forEach((button) => {
      if (exceptIndex !== null && Number(button.dataset.matchToggle) === exceptIndex) return;
      button.setAttribute('aria-expanded', 'false');
      button.closest('.match-card')?.classList.remove('expanded');
      const panel = $(`match-detail-${button.dataset.matchToggle}`);
      if (panel) {
        const matchId = panel.querySelector('[data-map-replay-root]')?.dataset.replayMatch;
        if (matchId) destroyMapReplay(matchId);
        panel.hidden = true;
      }
    });
  }

  async function loadMatchDetail(index, force = false) {
    const match = state.profileMatches[index];
    const panel = $(`match-detail-${index}`);
    if (!match || !panel) return;
    const matchId = String(match.match_id || '');
    const profileGeneration = state.profileLoadGeneration;
    const stillCurrent = () => profileGeneration === state.profileLoadGeneration
      && panel.isConnected
      && $(`match-detail-${index}`) === panel
      && String(state.profileMatches[index]?.match_id || '') === matchId;
    if (!matchId) {
      panel.innerHTML = renderSelectedMatchReport(match, match);
      hydrateMatchReport(index, match, match, panel, profileGeneration);
      return;
    }
    if (force) {
      state.matchDetails.delete(matchId);
      state.matchDetailRequests.delete(matchId);
    }
    const cached = state.matchDetails.get(matchId);
    if (cached) {
      if (!stillCurrent()) return;
      panel.innerHTML = renderSelectedMatchReport(cached, match);
      hydrateMatchReport(index, cached, match, panel, profileGeneration);
      return;
    }
    panel.innerHTML = `
      <div class="match-loading-state" role="status">
        <span class="match-loading-spinner" aria-hidden="true"></span>
        <div><strong>Opening complete match report</strong><span>Reading the ten-player scoreboard and objectives from Aura's RAM cache…</span></div>
      </div>`;
    let request;
    try {
      request = state.matchDetailRequests.get(matchId);
      if (!request) {
        request = invokeWithTimeout('get_match_detail', { matchId }, 5000);
        state.matchDetailRequests.set(matchId, request);
      }
      const detail = await request;
      if (state.matchDetailRequests.get(matchId) === request) {
        state.matchDetailRequests.delete(matchId);
      }
      if (!stillCurrent()) return;
      if (!detail || typeof detail !== 'object') throw new Error('Riot returned an empty match report.');
      state.matchDetails.set(matchId, detail);
      panel.innerHTML = renderSelectedMatchReport(detail, match);
      hydrateMatchReport(index, detail, match, panel, profileGeneration);
    } catch (error) {
      const activeRequest = state.matchDetailRequests.get(matchId);
      if (!activeRequest || activeRequest === request) state.matchDetailRequests.delete(matchId);
      if (!stillCurrent()) return;
      panel.innerHTML = `
        <div class="match-detail-error" role="alert">
          <div><strong>Could not load the complete report</strong><span>${escapeHtml(String(error))}</span></div>
          <button class="btn-secondary compact" type="button" data-retry-match="${index}">Retry</button>
        </div>`;
      logErr('MATCH DETAIL', error);
    }
  }

  function toggleMatchDetail(index) {
    const button = document.querySelector(`[data-match-toggle="${index}"]`);
    const panel = $(`match-detail-${index}`);
    if (!button || !panel) return;
    const willOpen = button.getAttribute('aria-expanded') !== 'true';
    closeExpandedMatches(willOpen ? index : null);
    button.setAttribute('aria-expanded', String(willOpen));
    button.closest('.match-card')?.classList.toggle('expanded', willOpen);
    panel.hidden = !willOpen;
    if (willOpen) loadMatchDetail(index).catch((error) => logErr('MATCH DETAIL', error));
  }

  function timeAgo(epochMs) {
    const timestamp = Number(epochMs);
    if (!Number.isFinite(timestamp) || timestamp <= 0) return 'Date unavailable';
    const minutes = Math.max(1, Math.round((Date.now() - timestamp) / 60000));
    if (minutes < 60) return `${minutes}m ago`;
    if (minutes < 1440) return `${Math.round(minutes / 60)}h ago`;
    return `${Math.round(minutes / 1440)}d ago`;
  }

  $('btn-load-matches')?.addEventListener('click', loadMatches);
  $('btn-my-profile')?.addEventListener('click', () => {
    if (!state.localRiotAccount) {
      setMessage('match-status', 'Start the League Client and sign in so Aura can detect your account.', true);
      return;
    }
    navigate('profile');
    queueProfileLoad({ ...state.localRiotAccount, source: 'local' });
  });
  $('match-list')?.addEventListener('click', (event) => {
    const profileLink = event.target.closest('[data-player-profile]');
    if (profileLink) {
      event.preventDefault();
      event.stopPropagation();
      navigate('profile');
      queueProfileLoad({
        puuid: profileLink.dataset.playerPuuid || '',
        gameName: profileLink.dataset.playerGameName || '',
        tagLine: profileLink.dataset.playerTagLine || '',
        platform: profileLink.dataset.playerPlatform || state.profileTarget?.platform || $('riot-platform').value,
        source: 'match',
      });
      return;
    }
    const retry = event.target.closest('[data-retry-match]');
    if (retry) {
      loadMatchDetail(Number(retry.dataset.retryMatch), true).catch((error) => logErr('MATCH DETAIL', error));
      return;
    }
    const replayRetry = event.target.closest('[data-retry-replay]');
    if (replayRetry) {
      event.preventDefault();
      event.stopPropagation();
      const index = Number(replayRetry.dataset.retryReplay);
      const match = state.profileMatches[index];
      const detail = state.matchDetails.get(String(match?.match_id || '')) || match;
      loadMapReplay(index, detail, match, true).catch((error) => logErr('MAP REPLAY', error));
      return;
    }
    const toggle = event.target.closest('[data-match-toggle]');
    if (toggle) toggleMatchDetail(Number(toggle.dataset.matchToggle));
  });
  ['match-queue-filter', 'match-result-filter'].forEach((id) => {
    $(id)?.addEventListener('change', () => {
      closeExpandedMatches();
      if (state.profileMatches.length) renderMatches(state.profileMatches);
    });
  });
  $('match-count')?.addEventListener('change', () => {
    if (state.profileMatches.length) {
      queueProfileLoad(state.profileTarget || targetFromForm('manual'))
        .catch((error) => logErr('RIOT', error));
    }
  });

  // LCU, telemetry, and timers ----------------------------------------------
  function updateGameflow(phase) {
    const label = typeof phase === 'string' && phase ? phase : 'NONE';
    state.gameflowPhase = label;
    if (['none', 'lobby', 'matchmaking', 'readycheck', 'endofgame', 'preendofgame', 'waitingforstats']
      .includes(label.toLowerCase())) {
      state.allyChampionIds = [];
      state.enemyChampionIds = [];
      state.localChampionId = null;
      state.liveAllyChampionIds = [];
      state.liveEnemyChampionIds = [];
      state.liveChampionId = null;
      state.currentQueueId = null;
      state.advisorDetectedRole = '';
      const autoRoleOption = $('advisor-role')?.querySelector('option[value="auto"]');
      if (autoRoleOption) autoRoleOption.textContent = 'Auto-detect';
      state.telemetry = {
        gameTime: 0,
        dragonRespawnAt: null,
        baronRespawnAt: null,
        receivedAt: 0,
      };
    }
    ['gameflow-phase', 'live-gameflow-phase'].forEach((id) => {
      const element = $(id);
      element.textContent = label;
      element.className = `status-badge phase-${label.toLowerCase()}`;
    });
  }

  const gameStatusLabels = Object.freeze({
    IN_LOBBY: 'In Lobby',
    CHAMP_SELECT: 'Champion Select',
    IN_GAME: 'In Game',
    ENDED: 'Game Ended',
  });
  const gameStatusPhases = Object.freeze({
    IN_LOBBY: 'Lobby',
    CHAMP_SELECT: 'ChampSelect',
    IN_GAME: 'InProgress',
    ENDED: 'EndOfGame',
  });

  function liveMetricElements() {
    return {
      status: $('live-ipc-status'),
      gameTime: $('live-ipc-game-time'),
      summonerName: $('live-ipc-summoner'),
      championName: $('live-ipc-champion'),
      level: $('live-ipc-level'),
      kda: $('live-ipc-kda'),
      creepScore: $('live-ipc-cs'),
      creepScorePerMinute: $('live-ipc-cs-minute'),
      killParticipation: $('live-ipc-kp'),
      observableHeldValue: $('live-ipc-held-value'),
      observableValuePerMinute: $('live-ipc-held-value-minute'),
      earnedGoldPerMinute: $('live-ipc-earned-gpm'),
      dpm: $('live-ipc-dpm'),
      currentGold: $('live-ipc-current-gold'),
      goldDelta: $('live-ipc-gold-delta'),
      dragonType: $('live-objective-dragon-type'),
      dragonTimer: $('live-timer-dragon'),
      baronTimer: $('live-timer-baron'),
      xpProgress: $('live-ipc-xp-progress'),
      xpProgressBar: $('live-ipc-xp-bar'),
      integrityNote: $('live-metric-source'),
    };
  }

  function renderTypedGameStatus(status) {
    state.gameStatus = status;
    const phase = gameStatusPhases[status];
    if (phase) updateGameflow(phase);
    const liveHeaderStatus = $('live-gameflow-phase');
    if (liveHeaderStatus) {
      liveHeaderStatus.textContent = gameStatusLabels[status] || phase || 'Awaiting League Client';
      liveHeaderStatus.className = `status-badge live-status-${String(status).toLowerCase()}`;
    }
    const element = $('live-ipc-status');
    if (element) {
      const label = gameStatusLabels[status] || 'Awaiting League Client';
      if (element.textContent !== label) element.textContent = label;
      element.className = `status-badge live-status-${String(status).toLowerCase()}`;
    }
    if (status !== 'IN_GAME') clearLiveMetrics();
  }

  function clearLiveMetrics() {
    const placeholders = {
      'live-ipc-game-time': '00:00',
      'live-ipc-summoner': '—',
      'live-ipc-champion': '—',
      'live-ipc-level': 'Level unavailable',
      'live-ipc-kda': 'Unavailable',
      'live-ipc-cs': 'Unavailable',
      'live-ipc-cs-minute': 'Unavailable',
      'live-ipc-kp': 'Unavailable',
      'live-ipc-held-value': 'Unavailable',
      'live-ipc-held-value-minute': 'Unavailable',
      'live-ipc-earned-gpm': 'Unavailable',
      'live-ipc-dpm': 'Unavailable',
      'live-ipc-current-gold': 'Unavailable',
      'live-ipc-gold-delta': 'Unavailable',
      'live-ipc-xp-progress': 'Unavailable from Live Client API',
      'live-objective-dragon-type': 'DRAGON',
      'live-timer-dragon': 'READY',
      'live-timer-baron': 'READY',
      'live-metric-source': 'Waiting for Aura’s native live-client stream.',
    };
    Object.entries(placeholders).forEach(([id, value]) => {
      const target = $(id);
      if (target && target.textContent !== value) target.textContent = value;
    });
    const xpBar = $('live-ipc-xp-bar');
    if (xpBar && !xpBar.hidden) xpBar.hidden = true;
    if (xpBar && xpBar.value !== 0) xpBar.value = 0;
    if (xpBar?.getAttribute('aria-label') !== 'XP progress unavailable') {
      xpBar?.setAttribute('aria-label', 'XP progress unavailable');
    }
    const goldDelta = $('live-ipc-gold-delta');
    ['blue', 'red', 'even', 'unavailable'].forEach((tone) => {
      goldDelta?.classList.toggle(`live-tone-${tone}`, tone === 'unavailable');
    });
    [
      'live-ipc-level',
      'live-ipc-kda',
      'live-ipc-cs',
      'live-ipc-cs-minute',
      'live-ipc-kp',
      'live-ipc-held-value',
      'live-ipc-held-value-minute',
      'live-ipc-earned-gpm',
      'live-ipc-dpm',
      'live-ipc-current-gold',
      'live-ipc-xp-progress',
    ].forEach((id) => $(id)?.classList.add('live-metric-unavailable'));
  }

  function renderLiveGameTick(tick) {
    const effectiveStatus = state.gameStatus || 'IN_GAME';
    const viewModel = buildLiveGameViewModel(tick, effectiveStatus);
    renderLiveGameView(liveMetricElements(), viewModel);

    const dragonTimer = Number(tick.objectives.dragonTimer) || 0;
    const baronTimer = Number(tick.objectives.baronTimer) || 0;
    state.telemetry = {
      gameTime: tick.gameTime,
      dragonRespawnAt: dragonTimer > 0 ? tick.gameTime + dragonTimer : null,
      baronRespawnAt: baronTimer > 0 ? tick.gameTime + baronTimer : null,
      receivedAt: Date.now(),
    };

  }

  function championName(id) {
    return state.championMap[id] || (id ? `Champion ${id}` : null);
  }

  function renderTeam(listId, players, enemy = false) {
    const list = $(listId);
    if (!Array.isArray(players) || !players.length) {
      list.innerHTML = `<li class="muted">${enemy ? 'Enemy picks are hidden.' : 'Awaiting Champion Select…'}</li>`;
      return;
    }
    list.innerHTML = players.map((player) => {
      const id = player.championId || player.championPickIntent || 0;
      const name = championName(id) || (enemy ? 'Hidden' : 'Choosing…');
      return `<li class="${enemy ? 'enemy' : (player.championId ? 'locked' : 'hovering')}">${escapeHtml(name)}</li>`;
    }).join('');
  }

  function parseChampSelect(data) {
    if (!data || !Array.isArray(data.myTeam)) {
      state.allyChampionIds = [];
      state.enemyChampionIds = [];
      state.localChampionId = null;
      state.advisorDetectedRole = '';
      const autoRoleOption = $('advisor-role')?.querySelector('option[value="auto"]');
      if (autoRoleOption) autoRoleOption.textContent = 'Auto-detect';
      return;
    }
    renderTeam('my-team-list', data.myTeam);
    renderTeam('their-team-list', data.theirTeam || [], true);
    state.allyChampionIds = data.myTeam
      .map((player) => player.championId || player.championPickIntent || 0)
      .filter(Boolean);
    state.enemyChampionIds = (data.theirTeam || [])
      .map((player) => player.championId || player.championPickIntent || 0)
      .filter(Boolean);
    state.liveAllyChampionIds = [...state.allyChampionIds];
    state.liveEnemyChampionIds = [...state.enemyChampionIds];
    const localPlayer = data.myTeam.find((player) => player.cellId === data.localPlayerCellId);
    state.localChampionId = localPlayer
      ? (localPlayer.championId || localPlayer.championPickIntent || null)
      : null;
    state.liveChampionId = state.localChampionId || state.liveChampionId;
    const detectedRole = String(localPlayer?.assignedPosition || '').toLowerCase();
    const supportedRoles = new Set(['top', 'jungle', 'middle', 'bottom', 'utility']);
    state.advisorDetectedRole = supportedRoles.has(detectedRole) ? detectedRole : '';
    const autoRoleOption = $('advisor-role')?.querySelector('option[value="auto"]');
    if (autoRoleOption) {
      const labels = { top: 'Top', jungle: 'Jungle', middle: 'Mid', bottom: 'ADC', utility: 'Support' };
      autoRoleOption.textContent = state.advisorDetectedRole
        ? `Auto-detect (${labels[state.advisorDetectedRole]})`
        : 'Auto-detect';
    }
    const visibleQueueId = Number(data.queueId || data.gameData?.queueId);
    if (visibleQueueId) state.currentQueueId = visibleQueueId;
    const draftSignature = JSON.stringify({
      allies: state.allyChampionIds,
      enemies: state.enemyChampionIds,
      local: state.localChampionId,
      role: state.advisorDetectedRole,
    });
    if (draftSignature !== state.advisorDraftSignature) {
      state.advisorDraftSignature = draftSignature;
      scheduleAdvisorDraftRefresh();
    }
    if ($('adapt-enemy-build')?.checked && state.selectedChampionDetails) {
      applyRecommendedBuild(false).catch((error) => logErr('BUILD', error));
    }
  }

  listen('lcu-initial-phase', (event) => updateGameflow(event.payload)).catch((error) => logErr('LCU', error));
  listen('lcu-current-summoner', (event) => handleLocalRiotAccount(event.payload))
    .catch((error) => logErr('LCU ACCOUNT', error));
  listen('lcu-event', (event) => {
    const packet = event.payload;
    if (packet?.uri === '/lol-gameflow/v1/gameflow-phase') updateGameflow(packet.data);
    if (packet?.uri === '/lol-champ-select/v1/session') parseChampSelect(packet.data);
  }).catch((error) => logErr('LCU', error));

  let disposeLiveClientEvents = null;
  subscribeLiveClientEvents(listen, {
    onGameStatus: renderTypedGameStatus,
    onGameTick: renderLiveGameTick,
    onError: (error) => logErr('LIVE IPC', error),
  })
    .then((dispose) => { disposeLiveClientEvents = dispose; })
    .catch((error) => logErr('LIVE IPC', error));

  window.addEventListener('beforeunload', () => {
    if (disposeLiveClientEvents) {
      disposeLiveClientEvents().catch((error) => logErr('LIVE IPC CLEANUP', error));
      disposeLiveClientEvents = null;
    }
  }, { once: true });

  function updateTimer(ids, respawnAt, currentGameTime) {
    const ready = !respawnAt || currentGameTime >= respawnAt;
    const remaining = Math.max(0, respawnAt - currentGameTime);
    const text = ready ? 'READY' : `${Math.floor(remaining / 60)}:${Math.floor(remaining % 60).toString().padStart(2, '0')}`;
    ids.forEach((id) => {
      const element = $(id);
      if (!element) return;
      if (element.textContent !== text) element.textContent = text;
      const color = ready ? '#22c55e' : 'var(--aura-cyan)';
      if (element.style.color !== color) element.style.color = color;
    });
  }

  let timerInterval = null;
  function startTimerUpdates(delay) {
    clearInterval(timerInterval);
    timerInterval = setInterval(() => {
      if (!state.telemetry.receivedAt) return;
      const gameTime = state.telemetry.gameTime + (Date.now() - state.telemetry.receivedAt) / 1000;
      updateTimer(['timer-dragon', 'live-timer-dragon'], state.telemetry.dragonRespawnAt, gameTime);
      updateTimer(['timer-baron', 'live-timer-baron'], state.telemetry.baronRespawnAt, gameTime);
    }, delay);
  }
  startTimerUpdates(1000);
  const handleTimerBlur = () => startTimerUpdates(5000);
  const handleTimerFocus = () => startTimerUpdates(1000);
  window.addEventListener('blur', handleTimerBlur);
  window.addEventListener('focus', handleTimerFocus);
  window.addEventListener('beforeunload', () => {
    clearInterval(timerInterval);
    window.removeEventListener('blur', handleTimerBlur);
    window.removeEventListener('focus', handleTimerFocus);
  }, { once: true });

  function manualTimer(id, seconds) {
    const button = $(id);
    let interval = null;
    let end = 0;
    const original = button.textContent;
    button.addEventListener('click', () => {
      if (interval) {
        clearInterval(interval);
        interval = null;
        button.textContent = original;
        button.classList.remove('active-timer');
        return;
      }
      end = Date.now() + seconds * 1000;
      button.classList.add('active-timer');
      interval = setInterval(() => {
        const remaining = Math.max(0, Math.ceil((end - Date.now()) / 1000));
        if (!remaining) {
          clearInterval(interval);
          interval = null;
          button.textContent = 'RESPAWNED';
          button.classList.remove('active-timer');
          return;
        }
        button.textContent = `${Math.floor(remaining / 60)}:${(remaining % 60).toString().padStart(2, '0')}`;
      }, 500);
    });
  }
  manualTimer('btn-blue-buff', 300);
  manualTimer('btn-red-buff', 300);

  // Aura Intelligence -------------------------------------------------------
  function advisorRankRange() {
    const { solo } = selectRankedEntries(state.rankedEntries);
    return solo ? rankLabel(solo) : null;
  }

  function advisorQueueContext() {
    const latest = state.profileMatches[0];
    const queueId = latest?.queue_id ?? null;
    return {
      queue_id: queueId,
      queue: queueId ? (queueNames[queueId] || latest.game_mode || null) : null,
    };
  }

  function advisorMatchSnapshot(match) {
    if (!match || typeof match !== 'object') return null;
    return {
      match_id: match.match_id ?? '',
      queue_id: match.queue_id ?? null,
      game_mode: match.game_mode ?? '',
      game_creation_ms: match.game_creation_ms ?? null,
      game_duration_secs: match.game_duration_secs ?? 0,
      champion_name: match.champion_name ?? '',
      win: Boolean(match.win),
      kills: match.kills ?? 0,
      deaths: match.deaths ?? 0,
      assists: match.assists ?? 0,
      cs: match.cs ?? 0,
      gold: match.gold ?? 0,
      vision_score: match.vision_score ?? 0,
      items: Array.isArray(match.items) ? match.items.slice(0, 7) : [],
    };
  }

  function advisorCommonRequest(kind) {
    const queueId = state.currentQueueId;
    const selectedRole = $('advisor-role')?.value || 'auto';
    const isLive = kind === 'live';
    const isPost = kind === 'post';
    return {
      role: selectedRole === 'auto' ? (state.advisorDetectedRole || 'auto') : selectedRole,
      patch: state.ddragonVersion || '',
      region: $('riot-platform')?.value || '',
      queue_id: queueId,
      queue: queueId ? (queueNames[queueId] || '') : '',
      rank_range: advisorRankRange() || '',
      gameflow_phase: state.gameflowPhase,
      selected_champion_id: isPost
        ? null
        : (Number(isLive
          ? state.liveChampionId
          : (state.localChampionId || state.selectedChampion?.numericId)) || null),
      ally_champion_ids: [...(isLive ? state.liveAllyChampionIds : state.allyChampionIds)],
      enemy_champion_ids: [...(isLive ? state.liveEnemyChampionIds : state.enemyChampionIds)],
      context_captured_at: new Date().toISOString(),
    };
  }

  function advisorRequest(kind) {
    const common = advisorCommonRequest(kind);
    if (kind === 'draft') {
      return {
        ...common,
        champion_catalog: championEntries().map((champion) => ({
          id: Number(champion.numericId),
          name: champion.name,
          image_id: champion.imageId,
        })),
      };
    }
    if (kind === 'live') {
      const telemetryAge = state.telemetry.receivedAt ? Date.now() - state.telemetry.receivedAt : null;
      return {
        ...common,
        game_time: state.telemetry.gameTime,
        dragon_respawn_at: state.telemetry.dragonRespawnAt,
        baron_respawn_at: state.telemetry.baronRespawnAt,
        telemetry_received_at_ms: state.telemetry.receivedAt || null,
        telemetry_age_ms: telemetryAge,
        kills: null,
        deaths: null,
        assists: null,
        cs: null,
        vision_score: null,
        current_gold: null,
        telemetry: {
          game_time: state.telemetry.gameTime,
          dragon_respawn_at: state.telemetry.dragonRespawnAt,
          baron_respawn_at: state.telemetry.baronRespawnAt,
          received_at_ms: state.telemetry.receivedAt || null,
          age_ms: telemetryAge,
        },
      };
    }
    const latestQueue = advisorQueueContext();
    return {
      ...common,
      queue_id: latestQueue.queue_id,
      queue: latestQueue.queue,
      latest_match: advisorMatchSnapshot(state.profileMatches[0]),
      recent_matches: state.profileMatches.slice(0, 10).map(advisorMatchSnapshot),
    };
  }

  function advisorList(value) {
    if (Array.isArray(value)) return value.filter((entry) => entry !== null && entry !== undefined);
    if (typeof value === 'string' && value.trim()) return [value.trim()];
    return [];
  }

  function advisorText(value, fallback = 'Not reported') {
    if (value === null || value === undefined || value === '') return fallback;
    return String(value);
  }

  function policySafeAdvisorCopy(value, fallback = 'No analysis was returned.') {
    return advisorText(value, fallback)
      .replace(/\block(?:\s+in)?\b/gi, 'prioritize')
      .replace(/\bguaranteed?\b/gi, 'strongly supported')
      .replace(/\babsolutely?\b/gi, 'with high confidence')
      .replace(/\bmust\b/gi, 'should');
  }

  function safeAdvisorUrl(value) {
    try {
      const url = new URL(String(value));
      return url.protocol === 'https:' || url.protocol === 'http:' ? url.href : null;
    } catch {
      return null;
    }
  }

  function advisorGeneratedTime(value) {
    if (!value) return 'Not reported';
    const date = new Date(value);
    if (Number.isNaN(date.getTime())) return advisorText(value);
    return new Intl.DateTimeFormat(undefined, {
      dateStyle: 'medium',
      timeStyle: 'short',
    }).format(date);
  }

  function advisorMode(result) {
    const provenance = result?.provenance || {};
    if (provenance.mode) return String(provenance.mode);
    if (result?.mode) return String(result.mode);
    if (result?.used_fallback) return 'local fallback';
    if (state.advisorStatus?.provenance?.mode) return String(state.advisorStatus.provenance.mode);
    if (state.advisorStatus?.mode) return String(state.advisorStatus.mode);
    const source = String(
      provenance.source ||
      state.advisorStatus?.provenance?.source ||
      state.advisorStatus?.source ||
      ''
    ).toLowerCase();
    if (source.includes('local') || source.includes('bundled')) return 'local';
    if (source.includes('cloud') || source.includes('provider')) return 'cloud';
    return 'not reported';
  }

  function renderAdvisorEvidence(result) {
    const provenance = result?.provenance || {};
    const mode = advisorMode(result);
    const providerMode = /(?:aggregate|cloud|provider|development_feed)/i.test(mode);
    const sourceUrl = safeAdvisorUrl(provenance.source_url || provenance.sourceUrl);
    const methodologyUrl = safeAdvisorUrl(provenance.methodology_url || provenance.methodologyUrl);
    const sourceValue = escapeHtml(advisorText(provenance.source || result?.source));
    const source = sourceUrl
      ? `<a href="${escapeHtml(sourceUrl)}" target="_blank" rel="noreferrer">${sourceValue}</a>`
      : sourceValue;
    const methodology = escapeHtml(advisorText(provenance.methodology || provenance.method));
    const methodologyDetails = methodologyUrl
      ? ` - <a href="${escapeHtml(methodologyUrl)}" target="_blank" rel="noreferrer">Method details</a>`
      : '';
    return `
      <dl class="advisor-evidence">
        <div><dt>Source</dt><dd>${source}</dd></div>
        <div><dt>Mode</dt><dd>${escapeHtml(mode)}</dd></div>
        <div><dt>Patch</dt><dd>${escapeHtml(advisorText(provenance.patch))}</dd></div>
        <div><dt>Queue</dt><dd>${escapeHtml(advisorText(provenance.queue))}</dd></div>
        <div><dt>Rank range</dt><dd>${escapeHtml(advisorText(provenance.rank_range || provenance.rankRange))}</dd></div>
        <div><dt>Region</dt><dd>${escapeHtml(advisorText(provenance.region))}</dd></div>
        <div><dt>${providerMode ? 'Provider-reported sample' : 'Local evidence records'}</dt><dd>${escapeHtml(advisorText(provenance.sample_size ?? provenance.sampleSize))}</dd></div>
        <div><dt>Generated</dt><dd>${escapeHtml(advisorGeneratedTime(provenance.generated_at || provenance.generatedAt))}</dd></div>
        <div class="advisor-evidence-wide"><dt>Methodology</dt><dd>${methodology}${methodologyDetails}</dd></div>
      </dl>
      <p class="advisor-evidence-note">${escapeHtml(advisorText(
        provenance.sample_size_note || provenance.sampleSizeNote,
        providerMode
          ? 'Provider claims and sample sizes are displayed as reported and are not independently verified by Aura.'
          : 'Local analysis does not claim an aggregate match sample.'
      ))}</p>`;
  }

  function normalizeAdvisorAlternative(entry, index, providerMode) {
    if (typeof entry === 'string') {
      return {
        title: entry,
        reason: 'No supporting reason was supplied.',
        tradeoff: 'No separate tradeoff was supplied.',
        metric: null,
        index,
      };
    }
    const value = entry && typeof entry === 'object' ? entry : {};
    const title = value.champion || value.champion_name || value.name || value.title || value.mandate;
    const reason = value.reason || value.reasoning || value.summary;
    const tradeoff = value.tradeoff || value.limit || value.limitation;
    const winRate = Number(value.win_rate ?? value.winRate);
    const confidence = Number(value.confidence);
    const score = Number(value.score);
    const sampleSize = Number(value.sample_size ?? value.sampleSize);
    let metric = null;
    if (Number.isFinite(winRate)) {
      metric = `${providerMode ? 'Provider-reported win rate' : 'Observed win rate'}: ${(winRate * 100).toFixed(1)}%`;
    } else if (Number.isFinite(confidence)) {
      metric = `${providerMode ? 'Provider confidence' : 'Local heuristic confidence'}: ${Math.round(Math.max(0, Math.min(1, confidence)) * 100)}%`;
    } else if (Number.isFinite(score)) {
      metric = `${providerMode ? 'Provider ranking score' : 'Local heuristic score'}: ${score.toFixed(2)}`;
    }
    if (metric && Number.isFinite(sampleSize) && sampleSize > 0) {
      metric += ` - ${Math.round(sampleSize).toLocaleString()} games`;
    }
    return {
      title: policySafeAdvisorCopy(title, 'No evidence-backed alternative supplied'),
      reason: policySafeAdvisorCopy(reason, 'Aura will not invent a pick or action to fill this slot.'),
      tradeoff: policySafeAdvisorCopy(tradeoff, 'No separate tradeoff was supplied.'),
      metric,
      index,
    };
  }

  function renderAdvisorResult(kind, result) {
    const target = $(`advisor-${kind}-result`);
    if (!target) return;
    const recommendedChampion = result?.recommended_champion || result?.recommendedChampion;
    const rawAnalysis = result?.mandate || result?.headline || 'No recommendation was returned.';
    const primary = kind === 'draft' && recommendedChampion
      ? `BEST STATISTICAL FIT: ${recommendedChampion}`
      : policySafeAdvisorCopy(result?.headline || rawAnalysis);
    const analysisCopy = policySafeAdvisorCopy(rawAnalysis);
    const supportingCopy = analysisCopy !== primary ? analysisCopy : null;
    const reasons = advisorList(result?.reasoning || result?.reasons);
    const actions = advisorList(result?.actions || result?.orders);
    const warnings = advisorList(result?.warnings);
    const confidenceNumber = Number(result?.confidence);
    const confidence = Number.isFinite(confidenceNumber)
      ? `${Math.round(Math.max(0, Math.min(1, confidenceNumber)) * 100)}% confidence`
      : 'Confidence not reported';
    const providerMode = /(?:aggregate|cloud|provider|development_feed)/i.test(advisorMode(result));
    const alternatives = advisorList(result?.alternatives).slice(0, 2);
    while (alternatives.length < 2) alternatives.push(null);
    const alternativeCards = alternatives
      .map((entry, index) => normalizeAdvisorAlternative(entry, index, providerMode))
      .map((alternative) => `
        <article class="advisor-alternative${alternative.title.startsWith('No evidence-') ? ' unavailable' : ''}">
          <span>Alternative ${alternative.index + 1}</span>
          <strong>${escapeHtml(alternative.title)}</strong>
          <p>${escapeHtml(alternative.reason)}</p>
          <p class="advisor-alternative-tradeoff"><strong>Tradeoff:</strong> ${escapeHtml(alternative.tradeoff)}</p>
          ${alternative.metric ? `<small>${escapeHtml(alternative.metric)}</small>` : ''}
        </article>`)
      .join('');
    const renderItems = (items) => items.length
      ? `<ul>${items.map((item) => {
        const text = typeof item === 'object' ? (item.text || item.action || JSON.stringify(item)) : item;
        return `<li>${escapeHtml(policySafeAdvisorCopy(text))}</li>`;
      }).join('')}</ul>`
      : '<p class="advisor-empty-copy">None supplied.</p>';
    const primaryLabel = kind === 'draft'
      ? 'Top-ranked option'
      : (kind === 'live' ? 'Highest current priority' : 'Main review finding');
    target.innerHTML = `
      <div class="advisor-command-block">
        <div class="advisor-command-meta">
          <span>${primaryLabel}</span>
          <span>${escapeHtml(confidence)}</span>
        </div>
        <p class="advisor-command">${escapeHtml(primary)}</p>
        ${supportingCopy ? `<p class="advisor-supporting-copy">${escapeHtml(supportingCopy)}</p>` : ''}
        ${kind === 'draft' && recommendedChampion ? `<div class="advisor-pick">Top-ranked champion: <strong>${escapeHtml(recommendedChampion)}</strong></div>` : ''}
      </div>
      <div class="advisor-detail-grid">
        <div><h4>Cold evidence</h4>${renderItems(reasons)}</div>
        <div><h4>${kind === 'live' ? 'Priorities' : 'Execution'}</h4>${renderItems(actions)}</div>
      </div>
      ${warnings.length ? `<div class="advisor-warnings"><strong>Limits</strong>${renderItems(warnings)}</div>` : ''}
      <div class="advisor-alternatives">
        <h4>Three-option decision set: primary plus two alternatives</h4>
        <div>${alternativeCards}</div>
      </div>
      ${renderAdvisorEvidence(result)}`;
  }

  function renderAdvisorSystemStatus(status) {
    state.advisorStatus = status && typeof status === 'object' ? status : {};
    const mode = advisorMode(null);
    const badge = $('advisor-mode-badge');
    badge.textContent = mode.toUpperCase();
    badge.className = `status-badge advisor-mode-${mode.toLowerCase().replace(/[^a-z0-9]+/g, '-')}`;
    let message = 'Advisor status loaded.';
    let isError = false;
    const statusError = state.advisorStatus.last_error || state.advisorStatus.error;
    if (state.advisorStatus.refreshing) message = 'Advisor data is refreshing.';
    else if (statusError) {
      message = advisorText(statusError);
      isError = true;
    } else if (state.advisorStatus.stale) {
      message = 'Advisor data is available but stale. Refresh before relying on it.';
    } else if (state.advisorStatus.ready === false || state.advisorStatus.configured === false) {
      message = 'Aggregate dataset not connected. Aura will label any local fallback explicitly.';
    } else if (state.advisorStatus.source) {
      message = `Ready: ${state.advisorStatus.source}.`;
    }
    setMessage('advisor-system-status', message, isError);
  }

  async function refreshAdvisorStatus(showErrors = true) {
    try {
      const status = await invokeWithTimeout('advisor_status', {}, 8000);
      renderAdvisorSystemStatus(status);
      return status;
    } catch (error) {
      const message = `Advisor status failed within the 8-second deadline: ${error}`;
      setMessage('advisor-system-status', message, true);
      const badge = $('advisor-mode-badge');
      badge.textContent = 'UNAVAILABLE';
      badge.className = 'status-badge advisor-mode-unavailable';
      if (showErrors) toast(message, 'error');
      logErr('ADVISOR', error);
      return null;
    }
  }

  async function refreshAdvisorData() {
    const button = $('advisor-refresh');
    button.disabled = true;
    button.textContent = 'Refreshing...';
    setMessage('advisor-system-status', 'Refreshing advisor data (30-second deadline)...');
    try {
      const status = await invokeWithTimeout('advisor_refresh', {}, 30000);
      if (status && typeof status === 'object') renderAdvisorSystemStatus(status);
      await refreshAdvisorStatus(false);
      toast(
        status?.configured === false
          ? 'Local advisor rules are ready; no aggregate feed is configured.'
          : 'Advisor data refreshed.',
        status?.configured === false ? 'info' : 'success'
      );
    } catch (error) {
      const message = `Advisor refresh failed or exceeded 30 seconds: ${error}`;
      setMessage('advisor-system-status', message, true);
      toast(message, 'error');
      logErr('ADVISOR', error);
    } finally {
      button.disabled = false;
      button.textContent = 'Refresh Data';
    }
  }

  async function runAdvisor(kind, automatic = false) {
    const config = {
      draft: { command: 'advisor_draft_mandate', label: 'Draft ranking', deadline: 18000, loading: 'Ranking options from visible draft evidence' },
      live: { command: 'advisor_live_orders', label: 'Live-priority analysis', deadline: 15000, loading: 'Ranking priorities from visible live signals' },
      post: { command: 'advisor_post_game', label: 'Post-game analysis', deadline: 20000, loading: 'Analyzing the latest loaded match' },
    }[kind];
    if (!config || state.advisorBusy[kind]) return;
    if (kind === 'post' && !state.profileMatches.length) {
      setMessage('advisor-post-status', 'Load Profile matches before requesting a post-game verdict.', true);
      return;
    }
    state.advisorBusy[kind] = true;
    const button = $(`advisor-run-${kind}`);
    button.disabled = true;
    setMessage(
      `advisor-${kind}-status`,
      `${automatic ? 'Champion Select changed. ' : ''}${config.loading} (${Math.ceil(config.deadline / 1000)}-second deadline)...`
    );
    try {
      const result = await invokeWithTimeout(
        config.command,
        { request: advisorRequest(kind) },
        config.deadline
      );
      state.advisorResults[kind] = result;
      renderAdvisorResult(kind, result);
      setMessage(
        `advisor-${kind}-status`,
        `${automatic ? 'Updated automatically from the Champion Select event. ' : ''}Analysis ranked with evidence and tradeoffs.`
      );
    } catch (error) {
      const message = `${config.label} failed or exceeded its deadline: ${error}`;
      setMessage(`advisor-${kind}-status`, message, true);
      $(`advisor-${kind}-result`).innerHTML = `<div class="empty-state error-text">${escapeHtml(message)}</div>`;
      if (!automatic) toast(message, 'error');
      logErr('ADVISOR', error);
    } finally {
      state.advisorBusy[kind] = false;
      button.disabled = false;
    }
  }

  function scheduleAdvisorDraftRefresh() {
    clearTimeout(state.advisorDraftTimer);
    if (state.currentPage !== 'intelligence' || document.hidden) return;
    setMessage('advisor-draft-status', 'Visible Champion Select changed. Updating after the event settles...');
    state.advisorDraftTimer = setTimeout(() => {
      runAdvisor('draft', true).catch((error) => logErr('ADVISOR', error));
    }, 750);
  }

  $('advisor-refresh')?.addEventListener('click', refreshAdvisorData);
  $('advisor-run-draft')?.addEventListener('click', () => runAdvisor('draft'));
  $('advisor-run-live')?.addEventListener('click', () => runAdvisor('live'));
  $('advisor-run-post')?.addEventListener('click', () => runAdvisor('post'));
  $('advisor-role')?.addEventListener('change', () => {
    setMessage('advisor-draft-status', 'Role changed. Rank options again to apply it.');
  });

  // Overlay ------------------------------------------------------------------
  function overlayVisibility(status) {
    if (!status || typeof status.visible !== 'boolean') {
      throw new Error('The overlay window did not return a valid status. Restart Aura and try again.');
    }
    return status.visible;
  }

  function renderOverlayLayout(value) {
    const layout = normalizeOverlayLayout(value, state.overlayLayout);
    state.overlayLayout = layout;
    if (layout.mode === 'compact' || layout.mode === 'expanded') {
      state.overlayPreferredMode = layout.mode;
    }

    const mode = $('overlay-mode');
    const scale = $('overlay-scale');
    const opacity = $('overlay-opacity');
    const opacityValue = $('overlay-opacity-value');
    const lockButton = $('toggle-overlay-lock');
    const lockState = $('overlay-lock-state');

    if (mode) mode.value = state.overlayPreferredMode;
    if (scale) scale.value = String(layout.scalePercent);
    if (opacity) opacity.value = String(layout.opacityPercent);
    if (opacityValue) opacityValue.textContent = `${layout.opacityPercent}%`;
    if (lockButton) {
      lockButton.textContent = layout.locked ? 'Unlock HUD Editing' : 'Lock & Pass Through';
      lockButton.disabled = !state.overlayVisible;
    }
    if (lockState) {
      lockState.textContent = layout.locked ? 'Locked' : 'Editing';
      lockState.classList.toggle('active', !layout.locked);
    }

    return layout;
  }

  function renderOverlayStatus(visible, layout = state.overlayLayout) {
    const modeLabels = {
      standby: 'Standby pill',
      compact: 'Compact ribbon',
      expanded: 'Expanded telemetry',
    };
    const mode = modeLabels[layout.mode] || 'Adaptive HUD';
    const interaction = layout.locked ? 'click-through locked' : 'editing enabled';
    setIntegrationStatus(
      'overlay-status',
      visible
        ? `Overlay visible · ${mode} · ${layout.scalePercent}% · ${interaction}`
        : `Overlay hidden · ${state.overlayPreferredMode} mode ready for the next match`,
      visible ? 'ready' : 'pending',
    );
  }

  function applyOverlayStatus(status) {
    const visible = overlayVisibility(status);
    state.overlayVisible = visible;
    const layout = renderOverlayLayout(status.layout || state.overlayLayout);
    renderOverlayStatus(visible, layout);
    return visible;
  }

  async function refreshOverlayStatus() {
    try {
      const status = await invokeWithTimeout('overlay_status', {}, 5000);
      applyOverlayStatus(status);
    } catch (error) {
      setIntegrationStatus('overlay-status', `Overlay check failed: ${error}`, 'missing');
    }
  }

  async function overlayCommand(command) {
    const buttons = [$('show-overlay'), $('hide-overlay')].filter(Boolean);
    buttons.forEach((button) => { button.disabled = true; });
    setIntegrationStatus(
      'overlay-status',
      command === 'show_overlay' ? 'Opening overlay…' : 'Hiding overlay…',
      'pending'
    );
    try {
      const status = await invokeWithTimeout(command, {}, 8000);
      applyOverlayStatus(status);
    } catch (error) {
      setIntegrationStatus('overlay-status', `Overlay action failed: ${error}`, 'missing');
      toast(error, 'error');
    } finally {
      buttons.forEach((button) => { button.disabled = false; });
    }
  }

  async function updateOverlayLayout(patch) {
    if (patch.mode === 'compact' || patch.mode === 'expanded') {
      state.overlayPreferredMode = patch.mode;
    }
    const requested = normalizeOverlayLayout(
      { ...state.overlayLayout, ...patch },
      state.overlayLayout,
    );
    try {
      const layout = await invokeWithTimeout(
        'set_overlay_layout',
        { config: requested },
        5000,
      );
      renderOverlayLayout(layout);
      renderOverlayStatus(state.overlayVisible);
    } catch (error) {
      renderOverlayLayout(state.overlayLayout);
      toast(`Overlay layout update failed: ${error}`, 'error');
    }
  }

  $('show-overlay')?.addEventListener('click', () => overlayCommand('show_overlay'));
  $('hide-overlay')?.addEventListener('click', () => overlayCommand('hide_overlay'));
  $('toggle-overlay-lock')?.addEventListener('click', async () => {
    try {
      const layout = await invokeWithTimeout('toggle_overlay_interaction', {}, 5000);
      renderOverlayLayout(layout);
      renderOverlayStatus(state.overlayVisible);
    } catch (error) {
      toast(`Overlay interaction update failed: ${error}`, 'error');
    }
  });
  $('overlay-mode')?.addEventListener('change', (event) => {
    updateOverlayLayout({ mode: event.target.value });
  });
  $('overlay-scale')?.addEventListener('change', (event) => {
    updateOverlayLayout({ scalePercent: Number(event.target.value) });
  });
  $('overlay-opacity')?.addEventListener('input', (event) => {
    const value = Number(event.target.value);
    if ($('overlay-opacity-value')) $('overlay-opacity-value').textContent = `${value}%`;
  });
  $('overlay-opacity')?.addEventListener('change', (event) => {
    updateOverlayLayout({ opacityPercent: Number(event.target.value) });
  });

  let overlayLayoutUnlisten = null;
  listen('overlay:layout-changed', (event) => {
    renderOverlayLayout(event.payload);
    // Tray and telemetry actions can change native visibility without going
    // through a dashboard button. Refresh the full status after each rare
    // layout transition so lock controls never display stale availability.
    refreshOverlayStatus();
  })
    .then((unlisten) => { overlayLayoutUnlisten = unlisten; })
    .catch((error) => logErr('OVERLAY IPC', error));
  window.addEventListener('beforeunload', () => {
    if (typeof overlayLayoutUnlisten === 'function') {
      overlayLayoutUnlisten();
      overlayLayoutUnlisten = null;
    }
  }, { once: true });

  // Appearance ---------------------------------------------------------------
  document.querySelectorAll('.theme-card').forEach((card) => {
    card.addEventListener('click', () => {
      document.querySelectorAll('.theme-card').forEach((entry) => entry.classList.remove('active'));
      card.classList.add('active');
      const theme = card.dataset.theme;
      if (theme === 'default') document.documentElement.removeAttribute('data-theme');
      else document.documentElement.dataset.theme = theme;
      invoke('set_theme', { theme }).catch((error) => logErr('THEME', error));
    });
  });
  $('glow-slider')?.addEventListener('input', (event) => {
    const value = Number(event.target.value);
    $('glow-val').textContent = `${value}%`;
    document.documentElement.style.setProperty('--border-glow', `rgba(168,85,247,${value / 170})`);
  });

  function updateClock() {
    $('home-clock').textContent = new Intl.DateTimeFormat(undefined, { hour: '2-digit', minute: '2-digit' }).format(new Date());
  }
  updateClock();
  setInterval(updateClock, 60000);

  refreshIntegrationConfig();
  refreshOverlayStatus();
  initDDragon();
  invoke('get_local_riot_account')
    .then((account) => { if (account) handleLocalRiotAccount(account); })
    .catch((error) => logErr('LCU ACCOUNT', error));
});
