import {
  createIpcClient,
  subscribeLiveClientEvents,
} from './services/ipc.js';
import {
  buildLiveOverlayViewModel,
  emptyLiveOverlayViewModel,
  normalizeOverlayLayout,
  renderLiveOverlayView,
} from './components/overlay/live-overlay.js';

const LAYOUT_EVENT = 'overlay:layout-changed';
const DEFAULT_LAYOUT = Object.freeze({
  mode: 'standby',
  scalePercent: 100,
  opacityPercent: 55,
  locked: true,
});

window.addEventListener('load', () => {
  const { invoke: tauriInvoke } = window.__TAURI__.core;
  const { listen } = window.__TAURI__.event;
  const { invoke } = createIpcClient(tauriInvoke);
  const cleanupHandles = [];
  let disposed = false;
  let gameStatus = null;
  let preferredMatchMode = 'compact';
  let layout = { ...DEFAULT_LAYOUT };
  let layoutRevision = 0;

  const elements = {
    status: document.getElementById('overlay-status'),
    liveDot: document.getElementById('overlay-live-dot'),
    gameTime: document.getElementById('overlay-game-time'),
    champion: document.getElementById('overlay-champion'),
    summoner: document.getElementById('overlay-summoner'),
    level: document.getElementById('overlay-level'),
    kda: document.getElementById('overlay-kda'),
    currentGold: document.getElementById('overlay-current-gold'),
    csCombined: document.getElementById('overlay-cs-combined'),
    killParticipation: document.getElementById('overlay-kp'),
    observableValuePerMinute: document.getElementById('overlay-held-value-minute'),
    earnedGoldPerMinute: document.getElementById('overlay-earned-gpm'),
    dpm: document.getElementById('overlay-dpm'),
    goldDelta: document.getElementById('overlay-team-gold'),
    xpProgress: document.getElementById('overlay-xp'),
    xpProgressBar: document.getElementById('overlay-xp-progress'),
    dragonType: document.getElementById('overlay-dragon-type'),
    dragonTimer: document.getElementById('overlay-dragon'),
    baronTimer: document.getElementById('overlay-baron'),
    modeToggle: document.getElementById('overlay-mode-toggle'),
    opacity: document.getElementById('overlay-opacity'),
    opacityValue: document.getElementById('overlay-opacity-value'),
    scaleButtons: [...document.querySelectorAll('.overlay-scale-button')],
    lockButton: document.getElementById('overlay-lock-button'),
  };

  const statusLabels = Object.freeze({
    IN_LOBBY: 'In Lobby',
    CHAMP_SELECT: 'Champion Select',
    IN_GAME: 'In Game - syncing telemetry',
    ENDED: 'Game Ended',
  });

  const setAttributeChanged = (element, name, value) => {
    const text = String(value);
    if (element?.getAttribute(name) !== text) element?.setAttribute(name, text);
  };

  const setPropertyChanged = (style, name, value) => {
    if (style.getPropertyValue(name) !== value) style.setProperty(name, value);
  };

  function applyTheme(theme) {
    const nextTheme = typeof theme === 'string' ? theme : 'default';
    if (!nextTheme || nextTheme === 'default') {
      if (document.documentElement.hasAttribute('data-theme')) {
        document.documentElement.removeAttribute('data-theme');
      }
    } else if (document.documentElement.getAttribute('data-theme') !== nextTheme) {
      document.documentElement.setAttribute('data-theme', nextTheme);
    }
  }

  function applyLayout(nextLayout) {
    const normalized = normalizeOverlayLayout(nextLayout, layout);
    layout = normalized;
    if (normalized.mode === 'compact' || normalized.mode === 'expanded') {
      preferredMatchMode = normalized.mode;
    }

    if (document.body.dataset.overlayMode !== normalized.mode) {
      document.body.dataset.overlayMode = normalized.mode;
    }
    const lockedText = String(normalized.locked);
    if (document.body.dataset.overlayLocked !== lockedText) {
      document.body.dataset.overlayLocked = lockedText;
    }
    setPropertyChanged(document.body.style, '--overlay-scale', String(normalized.scalePercent / 100));
    setPropertyChanged(document.body.style, '--overlay-opacity', String(normalized.opacityPercent / 100));

    const expanded = normalized.mode === 'expanded';
    setAttributeChanged(elements.modeToggle, 'aria-expanded', expanded);
    setAttributeChanged(
      elements.modeToggle,
      'aria-label',
      expanded ? 'Show compact live statistics' : 'Show full live statistics',
    );
    setAttributeChanged(
      elements.modeToggle,
      'title',
      normalized.locked
        ? 'Unlock the HUD from Aura or the tray before changing its layout'
        : (expanded ? 'Show compact live statistics' : 'Show full live statistics'),
    );
    if (elements.modeToggle && elements.modeToggle.disabled !== normalized.locked) {
      elements.modeToggle.disabled = normalized.locked;
    }

    const opacityText = String(normalized.opacityPercent);
    if (elements.opacity?.value !== opacityText) elements.opacity.value = opacityText;
    if (elements.opacityValue?.textContent !== `${opacityText}%`) {
      elements.opacityValue.textContent = `${opacityText}%`;
    }
    elements.scaleButtons.forEach((button) => {
      setAttributeChanged(button, 'aria-pressed', Number(button.dataset.scale) === normalized.scalePercent);
    });
  }

  async function commitLayout(patch) {
    const previous = layout;
    const requested = normalizeOverlayLayout({ ...layout, ...patch }, layout);
    const revision = ++layoutRevision;
    applyLayout(requested);
    try {
      const result = await invoke('set_overlay_layout', { config: requested });
      if (revision === layoutRevision) applyLayout(normalizeOverlayLayout(result, requested));
    } catch (error) {
      if (revision === layoutRevision) applyLayout(previous);
      throw error;
    }
  }

  function commitLayoutQuietly(patch, context) {
    commitLayout(patch).catch((error) => {
      console.error(`[AURA::OVERLAY][ERR] ${context}:`, error);
    });
  }

  async function toggleInteraction() {
    const previous = layout;
    const revision = ++layoutRevision;
    applyLayout({ ...layout, locked: !layout.locked });
    try {
      const result = await invoke('toggle_overlay_interaction');
      if (revision === layoutRevision) applyLayout(normalizeOverlayLayout(result, layout));
    } catch (error) {
      if (revision === layoutRevision) applyLayout(previous);
      throw error;
    }
  }

  function setLiveDot(active) {
    if (!elements.liveDot?.classList) return;
    if (elements.liveDot.classList.contains('is-active') !== active) {
      elements.liveDot.classList.toggle('is-active', active);
    }
  }

  function enterMatchLayout() {
    if (layout.mode === 'standby') {
      commitLayoutQuietly({ mode: preferredMatchMode }, 'match layout activation failed');
    }
  }

  function enterStandbyLayout() {
    if (layout.mode !== 'standby') {
      commitLayoutQuietly({ mode: 'standby' }, 'standby layout activation failed');
    }
  }

  function renderStatus(status) {
    gameStatus = status;
    const inGame = status === 'IN_GAME';
    setLiveDot(inGame);
    if (!inGame) {
      renderLiveOverlayView(
        elements,
        emptyLiveOverlayViewModel(statusLabels[status] || 'Awaiting League Client'),
      );
      enterStandbyLayout();
      return;
    }
    renderLiveOverlayView(elements, emptyLiveOverlayViewModel(statusLabels.IN_GAME));
    enterMatchLayout();
  }

  function renderTick(tick) {
    if (gameStatus !== 'IN_GAME') gameStatus = 'IN_GAME';
    setLiveDot(true);
    enterMatchLayout();
    renderLiveOverlayView(elements, buildLiveOverlayViewModel(tick, gameStatus));
  }

  async function registerCleanup(handle) {
    if (disposed) {
      await handle();
      return;
    }
    cleanupHandles.push(handle);
  }

  function bindControls() {
    elements.modeToggle?.addEventListener('click', () => {
      if (layout.locked || layout.mode === 'standby') return;
      const mode = layout.mode === 'expanded' ? 'compact' : 'expanded';
      commitLayoutQuietly({ mode }, 'layout toggle failed');
    });

    elements.opacity?.addEventListener('input', (event) => {
      const preview = normalizeOverlayLayout(
        { ...layout, opacityPercent: event.currentTarget.value },
        layout,
      );
      setPropertyChanged(document.body.style, '--overlay-opacity', String(preview.opacityPercent / 100));
      if (elements.opacityValue?.textContent !== `${preview.opacityPercent}%`) {
        elements.opacityValue.textContent = `${preview.opacityPercent}%`;
      }
    });

    elements.opacity?.addEventListener('change', (event) => {
      commitLayoutQuietly(
        { opacityPercent: event.currentTarget.value },
        'opacity update failed',
      );
    });

    elements.scaleButtons.forEach((button) => {
      button.addEventListener('click', () => {
        if (layout.locked) return;
        commitLayoutQuietly({ scalePercent: button.dataset.scale }, 'scale update failed');
      });
    });

    elements.lockButton?.addEventListener('click', () => {
      if (layout.locked) return;
      toggleInteraction().catch((error) => {
        console.error('[AURA::OVERLAY][ERR] click-through lock failed:', error);
      });
    });
  }

  function bindKeyboardSafety() {
    const onKeyDown = (event) => {
      if (event.key !== 'Escape' || event.repeat || layout.locked) return;
      event.preventDefault();
      toggleInteraction().catch((error) => {
        console.error('[AURA::OVERLAY][ERR] Escape-to-lock failed:', error);
      });
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }

  async function initializeOverlay() {
    bindControls();
    await registerCleanup(bindKeyboardSafety());
    applyLayout(layout);

    const layoutUnlisten = await listen(LAYOUT_EVENT, (event) => {
      layoutRevision += 1;
      applyLayout(normalizeOverlayLayout(event?.payload, layout));
    });
    if (typeof layoutUnlisten !== 'function') {
      throw new TypeError('Overlay layout listener did not return an unlisten function');
    }
    await registerCleanup(layoutUnlisten);

    const themeUnlisten = await listen('aura-theme-changed', (event) => {
      applyTheme(event.payload);
    });
    if (typeof themeUnlisten !== 'function') {
      throw new TypeError('Theme listener did not return an unlisten function');
    }
    await registerCleanup(themeUnlisten);

    const disposeLiveEvents = await subscribeLiveClientEvents(listen, {
      onGameStatus: renderStatus,
      onGameTick: renderTick,
      onError: (error) => console.error('[AURA::OVERLAY][ERR] live event:', error),
    });
    await registerCleanup(disposeLiveEvents);

    const initializationRevision = layoutRevision;
    const [themeResult, layoutResult] = await Promise.allSettled([
      invoke('get_theme'),
      invoke('get_overlay_layout'),
    ]);
    if (themeResult.status === 'fulfilled') {
      applyTheme(themeResult.value);
    } else {
      console.error('[AURA::OVERLAY][ERR] theme sync failed:', themeResult.reason);
    }
    if (layoutResult.status === 'fulfilled' && initializationRevision === layoutRevision) {
      applyLayout(normalizeOverlayLayout(layoutResult.value, layout));
    } else if (layoutResult.status === 'rejected') {
      console.error('[AURA::OVERLAY][ERR] layout sync failed:', layoutResult.reason);
    }
  }

  async function disposeOverlay() {
    if (disposed) return;
    disposed = true;
    const handles = cleanupHandles.splice(0).reverse();
    await Promise.allSettled(handles.map((cleanup) => Promise.resolve().then(cleanup)));
  }

  window.addEventListener('beforeunload', () => {
    disposeOverlay().catch((error) => {
      console.error('[AURA::OVERLAY][ERR] cleanup failed:', error);
    });
  }, { once: true });

  initializeOverlay().catch((error) => {
    console.error('[AURA::OVERLAY][ERR] initialization failed:', error);
  });
});
