/** Game-flow states emitted by Aura's native live-client service. */
export type GameStatus = 'IN_LOBBY' | 'CHAMP_SELECT' | 'IN_GAME' | 'ENDED';

/** High-frequency (1-2 Hz) snapshot used by the live-game HUD. */
export interface LiveGameTickPayload {
  gameTime: number; // in seconds
  activePlayer: {
    summonerName: string;
    championName: string;
    currentGold: number;
    kda: { kills: number; deaths: number; assists: number };
    dpm: number;
    level: number;
    creepScore: number;
    creepScorePerMinute: number;
    killParticipationPercent: number;
    observableHeldValue: number;
    observableValuePerMinute: number;
    earnedGoldPerMinute: number;
    xpProgressPercent: number | null;
  };
  teamGoldDelta: number; // Positive = Blue ahead, Negative = Red ahead
  objectives: {
    dragonType: string | null;
    dragonTimer: number;
    baronTimer: number;
  };
}

/** Medium-frequency player inventory/build update. */
export interface PlayerStatsPayload {
  summonerName: string;
  championName: string;
  team: 'ORDER' | 'HARMONY'; // Blue / Red
  level: number;
  creepScore: number;
  items: number[]; // Item IDs
}

/**
 * Aura extension for metrics the official Live Client API cannot always expose.
 * Required numeric fields remain stable for IPC compatibility, while the UI uses
 * these flags to avoid presenting a zero sentinel as measured data.
 */
export interface LiveMetricAvailabilityPayload {
  currentGold: boolean;
  kda: boolean;
  dpm: boolean;
  teamGoldDelta: boolean;
  level: boolean;
  creepScore: boolean;
  creepScorePerMinute: boolean;
  killParticipationPercent: boolean;
  observableHeldValue: boolean;
  observableValuePerMinute: boolean;
  earnedGoldPerMinute: boolean;
  xpProgressPercent: boolean;
}

export interface LiveMetricSourcesPayload {
  observableHeldValue: 'CURRENT_GOLD_PLUS_CURRENT_INVENTORY_LISTED_VALUE' | null;
  observableValuePerMinute: 'CURRENT_GOLD_PLUS_CURRENT_INVENTORY_LISTED_VALUE' | null;
}

export type LiveGameTickEventPayload = LiveGameTickPayload & {
  metricAvailability: LiveMetricAvailabilityPayload;
  metricSources: LiveMetricSourcesPayload;
};

/** Volatile native overlay layout shared by dashboard, tray, and HUD WebView. */
export type OverlayMode = 'standby' | 'compact' | 'expanded';
export type OverlayScalePercent = 75 | 90 | 100;

export interface OverlayLayoutConfig {
  mode: OverlayMode;
  scalePercent: OverlayScalePercent;
  opacityPercent: number; // Native command clamps this to 40..100.
  locked: boolean; // Locked = non-focusable and mouse input passes through.
}

export interface OverlayStatusPayload {
  visible: boolean;
  layout: OverlayLayoutConfig;
}
