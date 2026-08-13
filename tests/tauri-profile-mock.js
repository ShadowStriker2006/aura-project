(() => {
  const wait = (value, delay = 0) => new Promise((resolve) => setTimeout(() => resolve(value), delay));
  const fail = (message, delay = 0) => new Promise((_, reject) => setTimeout(() => reject(new Error(message)), delay));
  const calls = [];
  const listeners = new Map();
  const params = new URLSearchParams(location.search);
  let selectedProfile = { gameName: 'Aura Fixture', tagLine: 'EUNE', platform: 'eun1' };
  let overlayLayout = {
    mode: params.has('liveTick')
      ? (params.has('overlayExpanded') ? 'expanded' : 'compact')
      : 'standby',
    scalePercent: params.has('scale75') ? 75 : (params.has('scale90') ? 90 : 100),
    opacityPercent: 55,
    locked: !params.has('overlayUnlocked'),
  };

  const profileFixture = () => {
    const selected = { ...selectedProfile };
    const isSlow = selected.gameName === 'Slow A';
    const isFast = selected.gameName === 'Fast B';
    return wait({
      profile_icon_id: isFast ? 103 : (isSlow ? 1 : 29),
      summoner_level: isFast ? 222 : (isSlow ? 111 : 321),
    }, isSlow ? 500 : (isFast ? 30 : 90));
  };

  const leagueEntriesFixture = () => {
    if (params.has('rankError')) return fail('Fixture League-V4 rank service unavailable', 35);
    const selected = { ...selectedProfile };
    const isSlow = selected.gameName === 'Slow A';
    const isFast = selected.gameName === 'Fast B';
    return wait([
        {
          queue_type: 'RANKED_FLEX_SR', tier: 'GOLD', rank: 'I', league_points: 55,
          wins: 22, losses: 18, hot_streak: false, veteran: true, fresh_blood: false,
          mini_series: null,
        },
        {
          queue_type: 'RANKED_SOLO_5x5', tier: isFast ? 'DIAMOND' : (isSlow ? 'SILVER' : 'EMERALD'),
          rank: isFast ? 'IV' : (isSlow ? 'I' : 'II'), league_points: isFast ? 73 : 0,
          wins: 48, losses: 32, hot_streak: true, veteran: false, fresh_blood: false,
          mini_series: { wins: 1, losses: 0, target: 2, progress: 'WNN' },
        },
      ], isSlow ? 490 : (isFast ? 25 : 70));
  };

  const masteryFixture = () => {
    if (params.has('masteryError')) return fail('Fixture mastery service unavailable', 30);
    const selected = { ...selectedProfile };
    const isSlow = selected.gameName === 'Slow A';
    const isFast = selected.gameName === 'Fast B';
    const primary = isFast ? 103 : (isSlow ? 1 : 62);
    return wait([
      {
        champion_id: primary, champion_level: 7, champion_points: isFast ? 2_000_000 : 1_250_000,
        last_play_time: 1_775_000_000_000, champion_points_until_next_level: 0,
        champion_points_since_last_level: 550_000, tokens_earned: 0, chest_granted: true,
      },
      {
        champion_id: 103, champion_level: 6, champion_points: 430_500,
        last_play_time: 1_774_000_000_000, champion_points_until_next_level: 19_500,
        champion_points_since_last_level: 130_500, tokens_earned: 1, chest_granted: false,
      },
      {
        champion_id: 1, champion_level: 5, champion_points: 98_750,
        last_play_time: 1_773_000_000_000, champion_points_until_next_level: 21_250,
        champion_points_since_last_level: 48_750, tokens_earned: 0, chest_granted: false,
      },
    ], isSlow ? 480 : (isFast ? 20 : 30));
  };

  const responses = {
    get_integration_config: () => ({
      riot_api_configured: !params.has('noKey'),
      riot_api_source: 'test fixture',
      spotify_configured: true,
      spotify_redirect_uri: 'http://127.0.0.1:8888/callback',
      spotify_scopes: ['streaming', 'user-read-playback-state'],
      spotify_error: '',
    }),
    get_local_riot_account: () => ({
      puuid: 'fixture-puuid-00000000000000000001',
      game_name: 'Aura Fixture',
      tag_line: 'EUNE',
      platform: 'eun1',
      profile_icon_id: 29,
      summoner_level: 321,
    }),
    select_riot_profile: (args) => {
      if (params.has('identityError')) return fail('Fixture Riot ID was not found', 30);
      selectedProfile = {
        gameName: args.fallbackGameName || 'Aura Fixture',
        tagLine: args.fallbackTagLine || 'EUNE',
        platform: args.platform || 'eun1',
      };
      return {
        puuid: args.puuid || 'fixture-puuid-00000000000000000001',
        game_name: selectedProfile.gameName,
        tag_line: selectedProfile.tagLine,
        platform: selectedProfile.platform,
      };
    },
    set_riot_id: (args) => {
      selectedProfile = { gameName: args.gameName, tagLine: args.tagLine, platform: args.platform };
      if (params.has('raceAuto') && args.gameName === 'Slow A') {
        setTimeout(() => {
          document.documentElement.dataset.mockRaceQueued = 'true';
          document.getElementById('btn-my-profile')?.click();
        }, 50);
      }
      return null;
    },
    get_summoner_profile: profileFixture,
    get_league_entries: leagueEntriesFixture,
    get_champion_masteries: masteryFixture,
    fetch_recent_matches: () => wait([], 120),
    get_champion_map: () => wait({ 1: 'Annie', 62: 'Wukong', 103: 'Ahri' }, 220),
    get_champion_image_id_map: () => wait({ 1: 'Annie', 62: 'MonkeyKing', 103: 'Ahri' }, 220),
    get_champion_details: ({ championImageId }) => ({
      id: championImageId,
      name: championImageId === 'MonkeyKing' ? 'Wukong' : championImageId,
      title: 'the Monkey King',
      tags: ['Fighter', 'Tank'],
      lore: 'Fixture champion details for browser validation.',
      passive: { name: 'Stone Skin', description: 'A defensive fixture passive.' },
      spells: [{ name: 'Crushing Blow', description: 'A fixture ability.' }],
      stats: { hp: 610, armor: 31, attackdamage: 68, movespeed: 345 },
    }),
    get_item_map: () => wait({}, 220),
    get_ddragon_version: () => wait('16.15.1', 220),
    get_rune_trees: () => wait([], 220),
    overlay_status: () => ({ visible: false }),
    get_overlay_layout: () => ({ ...overlayLayout }),
    set_overlay_layout: ({ config }) => {
      overlayLayout = { ...overlayLayout, ...config };
      return { ...overlayLayout };
    },
    toggle_overlay_interaction: () => {
      overlayLayout = { ...overlayLayout, locked: !overlayLayout.locked };
      setTimeout(() => window.__AURA_TEST_EMIT__('overlay:layout-changed', overlayLayout), 0);
      return { ...overlayLayout };
    },
    spotify_embedded_status: () => ({ running: false, ready: false, message: '' }),
    advisor_status: () => ({ configured: false, ready: true, mode: 'local' }),
    get_theme: () => 'default',
  };

  const liveTickFixture = () => ({
    gameTime: 754.8,
    activePlayer: {
      summonerName: 'Aura Fixture#EUNE',
      championName: 'Briar',
      currentGold: 1475,
      kda: { kills: 7, deaths: 2, assists: 5 },
      dpm: 0,
      level: 11,
      creepScore: 142,
      creepScorePerMinute: 11.3,
      killParticipationPercent: 63.2,
      observableHeldValue: 5200,
      observableValuePerMinute: 413.4,
      earnedGoldPerMinute: 0,
      xpProgressPercent: null,
    },
    teamGoldDelta: 0,
    objectives: { dragonType: 'Infernal', dragonTimer: 28, baronTimer: 94 },
    metricAvailability: {
      currentGold: true,
      kda: true,
      dpm: false,
      teamGoldDelta: false,
      level: true,
      creepScore: true,
      creepScorePerMinute: true,
      killParticipationPercent: true,
      observableHeldValue: true,
      observableValuePerMinute: true,
      earnedGoldPerMinute: false,
      xpProgressPercent: false,
    },
    metricSources: {
      observableHeldValue: 'CURRENT_GOLD_PLUS_CURRENT_INVENTORY_LISTED_VALUE',
      observableValuePerMinute: 'CURRENT_GOLD_PLUS_CURRENT_INVENTORY_LISTED_VALUE',
    },
  });

  window.__AURA_TEST_CALLS__ = calls;
  window.__AURA_LIVE_TICK_FIXTURE__ = liveTickFixture;
  window.__AURA_TEST_EMIT__ = (eventName, payload) => {
    const callbacks = listeners.get(eventName);
    if (!callbacks) return 0;
    callbacks.forEach((callback) => callback({ event: eventName, payload }));
    return callbacks.size;
  };
  window.__TAURI__ = {
    core: {
      invoke: async (command, args = {}) => {
        calls.push({ command, args });
        if (!(command in responses)) throw new Error(`No browser-fixture response for ${command}`);
        return responses[command](args);
      },
    },
    event: {
      listen: async (eventName, callback) => {
        const callbacks = listeners.get(eventName) || new Set();
        callbacks.add(callback);
        listeners.set(eventName, callbacks);
        return () => {
          callbacks.delete(callback);
          if (!callbacks.size) listeners.delete(eventName);
        };
      },
    },
  };

  if (params.has('liveTick')) {
    window.addEventListener('load', () => {
      setTimeout(() => {
        window.__AURA_TEST_EMIT__('game:state-changed', 'IN_GAME');
        window.__AURA_TEST_EMIT__('live:game-tick', liveTickFixture());
        document.documentElement.dataset.mockLiveFixture = 'ready';
      }, 0);
    }, { once: true });
  }
})();
