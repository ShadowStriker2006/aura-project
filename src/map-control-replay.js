const BLUE_TEAM = 100;
const RED_TEAM = 200;
const BLUE = '#38bdf8';
const RED = '#fb7185';
const GRID_SIZE = 22;
const MAX_DPR = 1.5;
const FRAME_BUDGET_MS = 1000 / 24;
const RECALL_DISTANCE = 4200;
const BASE_PROXIMITY = 0.18;
// BEGIN GENERATED MAP ASSET MANIFEST - DO NOT EDIT
const MAP_ASSET_MANIFEST = Object.freeze({
  ddragonVersion: '16.15.1',
  maps: Object.freeze({
    11: Object.freeze({ name: 'Summoner\'s Rift', filename: 'map11-16.15.1.png' }),
    12: Object.freeze({ name: 'Howling Abyss', filename: 'map12-16.15.1.png' }),
  }),
});
// END GENERATED MAP ASSET MANIFEST
const MAJOR_EVENT_KINDS = new Set([
  'dragon', 'baron', 'void_grubs', 'rift_herald', 'atakhan', 'turret', 'inhibitor', 'objective',
]);

const clamp = (value, min, max) => Math.min(max, Math.max(min, value));
const finite = (value, fallback = 0) => Number.isFinite(Number(value)) ? Number(value) : fallback;
const lerp = (start, end, amount) => start + ((end - start) * amount);

function normalizedPoint(player, coordinates) {
  const minX = finite(coordinates?.min_x, 0);
  const maxX = finite(coordinates?.max_x, 15000);
  const minY = finite(coordinates?.min_y, 0);
  const maxY = finite(coordinates?.max_y, 15000);
  let x = clamp((finite(player?.x) - minX) / Math.max(1, maxX - minX), 0, 1);
  const rawY = clamp((finite(player?.y) - minY) / Math.max(1, maxY - minY), 0, 1);
  let y = coordinates?.invert_y_for_canvas === false ? rawY : 1 - rawY;
  if (coordinates?.invert_x_for_canvas === true) x = 1 - x;
  if (coordinates?.swap_axes_for_canvas === true) [x, y] = [y, x];
  return { x, y };
}

function mapAsset(mapId) {
  return MAP_ASSET_MANIFEST.maps[finite(mapId)] || null;
}

function mapAssetUrl(asset) {
  if (!asset?.filename) return null;
  try {
    return new URL(`./assets/maps/${asset.filename}`, import.meta.url).href;
  } catch {
    return `./assets/maps/${asset.filename}`;
  }
}

function patchFamily(version) {
  const match = String(version || '').match(/^(\d+)\.(\d+)/);
  return match ? `${match[1]}.${match[2]}` : null;
}

function terrainPresentation(replay, options, asset, terrainState, controlEnabled, linearControl) {
  const mapName = asset?.name || `Map ${finite(replay?.map_id)}`;
  const matchPatch = patchFamily(replay?.game_version);
  const catalogPatch = patchFamily(options?.currentDdragonVersion);
  const assetPatch = patchFamily(MAP_ASSET_MANIFEST.ddragonVersion);
  const referenceDifferences = [];
  if (matchPatch && assetPatch && matchPatch !== assetPatch) {
    referenceDifferences.push(`match ${matchPatch}`);
  }
  if (catalogPatch && assetPatch && catalogPatch !== assetPatch && catalogPatch !== matchPatch) {
    referenceDifferences.push(`current game data ${catalogPatch}`);
  }
  const mismatch = terrainState === 'ready' && referenceDifferences.length > 0;
  const baseCanvasLabel = controlEnabled
    ? `Animated champion movement and ${linearControl ? 'estimated lane control' : 'estimated map control'}`
    : 'Animated champion movement';

  if (terrainState === 'ready') {
    return {
      label: `${mapName} terrain · Data Dragon ${MAP_ASSET_MANIFEST.ddragonVersion}${mismatch ? ` · reference only for ${referenceDifferences.join(' / ')}` : ''}`,
      canvasLabel: `${baseCanvasLabel} on bundled ${mapName} terrain`,
      mismatch,
    };
  }
  if (terrainState === 'loading') {
    return {
      label: `${mapName} terrain · loading bundled Data Dragon ${MAP_ASSET_MANIFEST.ddragonVersion}…`,
      canvasLabel: `${baseCanvasLabel}; bundled terrain is loading`,
      mismatch: false,
    };
  }
  if (terrainState === 'error') {
    return {
      label: `${mapName} terrain failed to load · neutral coordinate grid`,
      canvasLabel: `${baseCanvasLabel} on a neutral coordinate grid because bundled terrain failed to load`,
      mismatch: false,
    };
  }
  return {
    label: `${mapName} · neutral coordinate grid · no bundled terrain`,
    canvasLabel: `${baseCanvasLabel} on a neutral coordinate grid`,
    mismatch: false,
  };
}

function participantLookup(replay) {
  return new Map((Array.isArray(replay?.participants) ? replay.participants : [])
    .map((participant) => [finite(participant.participant_id), participant]));
}

function isNearOwnBase(player, participant, replay) {
  const teamId = finite(participant?.team_id);
  if (teamId !== BLUE_TEAM && teamId !== RED_TEAM) return false;
  const point = normalizedPoint(player, replay?.coordinates);
  const modelBase = teamId === BLUE_TEAM
    ? replay?.control_model?.blue_base
    : replay?.control_model?.red_base;
  const base = modelBase
    ? normalizedPoint(modelBase, replay?.coordinates)
    : teamId === BLUE_TEAM ? { x: 0.06, y: 0.94 } : { x: 0.94, y: 0.06 };
  return Math.hypot(point.x - base.x, point.y - base.y) <= BASE_PROXIMITY;
}

function shouldSnapBaseTransition(current, next, participant, replay) {
  const distance = Math.hypot(finite(next?.x) - finite(current?.x), finite(next?.y) - finite(current?.y));
  return distance > RECALL_DISTANCE
    && (isNearOwnBase(current, participant, replay) || isNearOwnBase(next, participant, replay));
}

function playerWeight(player) {
  const level = clamp(finite(player?.level, 1), 1, 18);
  const gold = clamp(finite(player?.total_gold, 500), 0, 25000);
  return 0.78 + ((level - 1) / 17) * 0.22 + (gold / 25000) * 0.2;
}

function pressureAt(x, y, players, participantById, coordinates) {
  let pressure = 0;
  for (const player of players) {
    const participant = participantById.get(finite(player.participant_id));
    const sign = finite(participant?.team_id) === BLUE_TEAM
      ? 1
      : finite(participant?.team_id) === RED_TEAM ? -1 : 0;
    if (!sign) continue;
    const point = normalizedPoint(player, coordinates);
    const distanceSquared = ((x - point.x) ** 2) + ((y - point.y) ** 2);
    pressure += sign * playerWeight(player) * Math.exp(-distanceSquared / 0.055);
  }
  const blueBaseDistance = ((x - 0.06) ** 2) + ((y - 0.94) ** 2);
  const redBaseDistance = ((x - 0.94) ** 2) + ((y - 0.06) ** 2);
  pressure += 0.52 * Math.exp(-blueBaseDistance / 0.19);
  pressure -= 0.52 * Math.exp(-redBaseDistance / 0.19);
  return pressure;
}

function linearModelGeometry(replay) {
  const model = replay?.control_model;
  if (model?.topology !== 'linear_lane' || !model.blue_base || !model.red_base) return null;
  const blue = normalizedPoint(model.blue_base, replay?.coordinates);
  const red = normalizedPoint(model.red_base, replay?.coordinates);
  const dx = red.x - blue.x;
  const dy = red.y - blue.y;
  const lengthSquared = (dx * dx) + (dy * dy);
  if (lengthSquared < 0.2) return null;
  return { blue, red, dx, dy, lengthSquared };
}

function laneProjection(point, geometry) {
  return clamp(
    (((point.x - geometry.blue.x) * geometry.dx)
      + ((point.y - geometry.blue.y) * geometry.dy)) / geometry.lengthSquared,
    0,
    1,
  );
}

function linearPressureAt(position, projectedPlayers) {
  let pressure = 0;
  for (const player of projectedPlayers) {
    const distance = position - player.position;
    pressure += player.sign * player.weight * Math.exp(-(distance * distance) / 0.028);
  }
  pressure += 0.68 * Math.exp(-(position * position) / 0.11);
  const redDistance = 1 - position;
  pressure -= 0.68 * Math.exp(-(redDistance * redDistance) / 0.11);
  return pressure;
}

function lineSquareSegment(origin, direction) {
  const candidates = [];
  const add = (x, y) => {
    if (x < -0.0001 || x > 1.0001 || y < -0.0001 || y > 1.0001) return;
    const point = { x: clamp(x, 0, 1), y: clamp(y, 0, 1) };
    if (!candidates.some((item) => Math.hypot(item.x - point.x, item.y - point.y) < 0.0001)) {
      candidates.push(point);
    }
  };
  if (Math.abs(direction.x) > 0.0001) {
    add(0, origin.y + ((0 - origin.x) / direction.x) * direction.y);
    add(1, origin.y + ((1 - origin.x) / direction.x) * direction.y);
  }
  if (Math.abs(direction.y) > 0.0001) {
    add(origin.x + ((0 - origin.y) / direction.y) * direction.x, 0);
    add(origin.x + ((1 - origin.y) / direction.y) * direction.x, 1);
  }
  return candidates.slice(0, 2);
}

function computeLinearControlSample(frame, replay) {
  const geometry = linearModelGeometry(replay);
  const participantById = participantLookup(replay);
  if (!geometry) {
    return {
      timestamp_ms: Math.max(0, finite(frame?.timestamp_ms)),
      blue: 50,
      red: 50,
      frontier: [],
      frontier_segment: [],
      topology: 'linear_lane',
      split: true,
    };
  }
  const projectedPlayers = (Array.isArray(frame?.players) ? frame.players : []).flatMap((player) => {
    const participant = participantById.get(finite(player?.participant_id));
    const teamId = finite(participant?.team_id);
    const sign = teamId === BLUE_TEAM ? 1 : teamId === RED_TEAM ? -1 : 0;
    if (!sign) return [];
    return [{
      position: laneProjection(normalizedPoint(player, replay?.coordinates), geometry),
      sign,
      weight: playerWeight(player),
    }];
  });
  const crossings = [];
  const sampleCount = 64;
  let previousPosition = 0;
  let previousPressure = linearPressureAt(0, projectedPlayers);
  let logisticShare = 1 / (1 + Math.exp(-previousPressure * 3.2));
  for (let index = 1; index <= sampleCount; index += 1) {
    const position = index / sampleCount;
    const pressure = linearPressureAt(position, projectedPlayers);
    logisticShare += 1 / (1 + Math.exp(-pressure * 3.2));
    if ((previousPressure >= 0 && pressure < 0) || (previousPressure < 0 && pressure >= 0)) {
      const amount = Math.abs(previousPressure)
        / Math.max(0.0001, Math.abs(previousPressure) + Math.abs(pressure));
      crossings.push(lerp(previousPosition, position, amount));
    }
    previousPosition = position;
    previousPressure = pressure;
  }
  const split = crossings.length !== 1;
  const blue = split
    ? clamp(Math.round(((logisticShare / (sampleCount + 1)) * 100) * 1000) / 1000, 0, 100)
    : clamp(Math.round(crossings[0] * 100000) / 1000, 0, 100);
  const frontierPosition = split ? null : crossings[0];
  const origin = frontierPosition === null ? null : {
    x: lerp(geometry.blue.x, geometry.red.x, frontierPosition),
    y: lerp(geometry.blue.y, geometry.red.y, frontierPosition),
  };
  return {
    timestamp_ms: Math.max(0, finite(frame?.timestamp_ms)),
    blue,
    red: 100 - blue,
    frontier: [],
    frontier_segment: origin
      ? lineSquareSegment(origin, { x: -geometry.dy, y: geometry.dx })
      : [],
    frontier_position: frontierPosition,
    axis: { x: geometry.dx, y: geometry.dy },
    topology: 'linear_lane',
    split,
  };
}

export function computeControlSample(frame, replay) {
  if (replay?.control_model?.topology === 'linear_lane') {
    return computeLinearControlSample(frame, replay);
  }
  const players = Array.isArray(frame?.players) ? frame.players : [];
  const participantById = participantLookup(replay);
  let blueShare = 0;
  for (let row = 0; row < GRID_SIZE; row += 1) {
    const y = (row + 0.5) / GRID_SIZE;
    for (let column = 0; column < GRID_SIZE; column += 1) {
      const x = (column + 0.5) / GRID_SIZE;
      const pressure = pressureAt(x, y, players, participantById, replay?.coordinates);
      blueShare += 1 / (1 + Math.exp(-pressure * 3.2));
    }
  }
  const blue = clamp(
    Math.round(((blueShare / (GRID_SIZE * GRID_SIZE)) * 100) * 1000) / 1000,
    0,
    100,
  );
  const frontier = [];
  for (let row = 0; row < GRID_SIZE; row += 1) {
    const y = (row + 0.5) / GRID_SIZE;
    let crossing = null;
    let previous = pressureAt(0, y, players, participantById, replay?.coordinates);
    for (let column = 1; column <= GRID_SIZE; column += 1) {
      const x = column / GRID_SIZE;
      const next = pressureAt(x, y, players, participantById, replay?.coordinates);
      if ((previous >= 0 && next < 0) || (previous < 0 && next >= 0)) {
        const previousX = (column - 1) / GRID_SIZE;
        const amount = Math.abs(previous) / Math.max(0.0001, Math.abs(previous) + Math.abs(next));
        crossing = lerp(previousX, x, amount);
        break;
      }
      previous = next;
    }
    if (crossing === null) crossing = previous >= 0 ? 1 : 0;
    frontier.push({ x: clamp(crossing, 0, 1), y });
  }
  return {
    timestamp_ms: Math.max(0, finite(frame?.timestamp_ms)),
    blue,
    red: 100 - blue,
    frontier,
    frontier_segment: [],
    topology: 'two_dimensional',
    split: false,
  };
}

export function prepareControlFrames(frames, replay) {
  const participantById = participantLookup(replay);
  const lastKnown = new Map();
  return (Array.isArray(frames) ? frames : []).map((frame) => {
    for (const player of Array.isArray(frame?.players) ? frame.players : []) {
      const participantId = finite(player?.participant_id);
      if (participantId > 0 && participantById.has(participantId)) {
        lastKnown.set(participantId, { ...lastKnown.get(participantId), ...player });
      }
    }
    const players = [...lastKnown.values()]
      .sort((left, right) => finite(left.participant_id) - finite(right.participant_id));
    const teams = new Set(players.map((player) =>
      finite(participantById.get(finite(player.participant_id))?.team_id)));
    return {
      ...frame,
      // Do not turn a missing team into apparent map dominance. Once both
      // teams have been observed, keep the latest valid coordinate until Riot
      // supplies that participant's next positional sample.
      players: teams.has(BLUE_TEAM) && teams.has(RED_TEAM) ? players : [],
    };
  });
}

function surroundingFrames(frames, timestampMs) {
  if (!frames.length) return { before: null, after: null, amount: 0 };
  if (timestampMs <= finite(frames[0].timestamp_ms)) {
    return { before: frames[0], after: frames[0], amount: 0 };
  }
  for (let index = 1; index < frames.length; index += 1) {
    const after = frames[index];
    if (timestampMs <= finite(after.timestamp_ms)) {
      const before = frames[index - 1];
      const span = Math.max(1, finite(after.timestamp_ms) - finite(before.timestamp_ms));
      return { before, after, amount: clamp((timestampMs - finite(before.timestamp_ms)) / span, 0, 1) };
    }
  }
  return { before: frames.at(-1), after: frames.at(-1), amount: 0 };
}

export function interpolateReplayState(replay, timestampMs) {
  const frames = Array.isArray(replay?.frames) ? replay.frames : [];
  const { before, after, amount } = surroundingFrames(frames, timestampMs);
  if (!before || !after) return { players: [], teams: [], amount: 0 };
  const participantById = participantLookup(replay);
  const beforePlayers = new Map((before.players || []).map((player) => [finite(player.participant_id), player]));
  const afterPlayers = new Map((after.players || []).map((player) => [finite(player.participant_id), player]));
  const playerIds = new Set([...beforePlayers.keys(), ...afterPlayers.keys()]);
  const hasModelBases = replay?.control_model?.blue_base && replay?.control_model?.red_base;
  const usesCalibratedBaseModel = replay?.availability?.control_estimate === true
    && (hasModelBases || finite(replay?.map_id) === 11);
  const players = [...playerIds].flatMap((participantId) => {
    const current = beforePlayers.get(participantId);
    const next = afterPlayers.get(participantId);
    if (!current && !next) return [];
    if (!current) return amount >= 0.5 ? [{ ...next }] : [];
    if (!next) return amount < 0.5 ? [{ ...current }] : [];
    const position = usesCalibratedBaseModel && shouldSnapBaseTransition(
      current,
      next,
      participantById.get(participantId),
      replay,
    )
      ? (amount < 0.5 ? current : next)
      : {
          x: lerp(finite(current.x), finite(next.x), amount),
          y: lerp(finite(current.y), finite(next.y), amount),
        };
    return [{
      ...current,
      x: finite(position.x),
      y: finite(position.y),
      level: amount < 1 ? finite(current.level) : finite(next.level),
      total_gold: lerp(finite(current.total_gold), finite(next.total_gold), amount),
    }];
  });
  const beforeTeams = new Map((before.teams || []).map((team) => [finite(team.team_id), team]));
  const afterTeams = new Map((after.teams || []).map((team) => [finite(team.team_id), team]));
  const events = (Array.isArray(replay?.events) ? replay.events : [])
    .filter((event) => finite(event.timestamp_ms) <= timestampMs);
  const teams = [BLUE_TEAM, RED_TEAM].map((teamId) => {
    const current = beforeTeams.get(teamId) || {};
    const next = afterTeams.get(teamId) || current;
    return {
      team_id: teamId,
      gold: lerp(finite(current.gold), finite(next.gold), amount),
      kills: events.filter((event) => event.kind === 'champion_kill' && finite(event.team_id) === teamId).length,
      turrets: events.filter((event) => event.kind === 'turret' && finite(event.team_id) === teamId).length,
    };
  });
  return { players, teams, before, after, amount };
}

function interpolateControlSample(samples, timestampMs) {
  const { before, after, amount } = surroundingFrames(samples, timestampMs);
  if (!before || !after) {
    return { blue: 50, red: 50, frontier: [], frontier_segment: [], split: false };
  }
  const frontier = before.frontier.map((point, index) => ({
    x: lerp(point.x, after.frontier[index]?.x ?? point.x, amount),
    y: point.y,
  }));
  const beforeSegment = Array.isArray(before.frontier_segment) ? before.frontier_segment : [];
  const afterSegment = Array.isArray(after.frontier_segment) ? after.frontier_segment : [];
  const frontierSegment = beforeSegment.length === 2 && afterSegment.length === 2
    ? beforeSegment.map((point, index) => ({
        x: lerp(point.x, afterSegment[index]?.x ?? point.x, amount),
        y: lerp(point.y, afterSegment[index]?.y ?? point.y, amount),
      }))
    : [];
  const blue = lerp(before.blue, after.blue, amount);
  return {
    blue,
    red: 100 - blue,
    frontier,
    frontier_segment: frontierSegment,
    axis: before.axis || after.axis,
    topology: before.topology || after.topology,
    split: before.split === true || after.split === true,
  };
}

function formatClock(timestampMs) {
  const totalSeconds = Math.max(0, Math.floor(finite(timestampMs) / 1000));
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}:${String(seconds).padStart(2, '0')}`;
}

function compactNumber(value) {
  const number = Math.max(0, finite(value));
  if (number >= 1000) return `${(number / 1000).toFixed(number >= 10000 ? 1 : 2)}k`;
  return String(Math.round(number));
}

function eventLabel(event) {
  let detail = String(event?.detail || '')
    .replaceAll('_', ' ')
    .toLowerCase();
  if (detail.startsWith('sru dragon ')) detail = `${detail.slice('sru dragon '.length)} dragon`;
  const labels = {
    dragon: detail ? `${detail} secured` : 'Dragon secured',
    baron: 'Baron Nashor secured',
    void_grubs: 'Void Grubs secured',
    rift_herald: 'Rift Herald secured',
    atakhan: 'Atakhan secured',
    turret: detail ? `${detail} destroyed` : 'Turret destroyed',
    inhibitor: 'Inhibitor destroyed',
    objective: detail ? `${detail} secured` : 'Objective secured',
  };
  return labels[event?.kind] || 'Objective secured';
}

function prepareCanvas(canvas, width, height) {
  const dpr = Math.min(MAX_DPR, Math.max(1, finite(globalThis.devicePixelRatio, 1)));
  const pixelWidth = Math.max(1, Math.round(width * dpr));
  const pixelHeight = Math.max(1, Math.round(height * dpr));
  if (canvas.width !== pixelWidth || canvas.height !== pixelHeight) {
    canvas.width = pixelWidth;
    canvas.height = pixelHeight;
  }
  const context = canvas.getContext('2d', { alpha: false });
  context.setTransform(dpr, 0, 0, dpr, 0, 0);
  return context;
}

function curvedPath(context, points, width, height) {
  if (!points.length) return;
  context.moveTo(points[0].x * width, points[0].y * height);
  for (let index = 1; index < points.length - 1; index += 1) {
    const current = points[index];
    const next = points[index + 1];
    context.quadraticCurveTo(
      current.x * width,
      current.y * height,
      ((current.x + next.x) / 2) * width,
      ((current.y + next.y) / 2) * height,
    );
  }
  const last = points.at(-1);
  context.lineTo(last.x * width, last.y * height);
}

function clipRectangleToBlueSide(frontier, axis) {
  const rectangle = [
    { x: 0, y: 0 }, { x: 1, y: 0 }, { x: 1, y: 1 }, { x: 0, y: 1 },
  ];
  const signedDistance = (point) => ((point.x - frontier.x) * axis.x)
    + ((point.y - frontier.y) * axis.y);
  const output = [];
  for (let index = 0; index < rectangle.length; index += 1) {
    const current = rectangle[index];
    const next = rectangle[(index + 1) % rectangle.length];
    const currentDistance = signedDistance(current);
    const nextDistance = signedDistance(next);
    const currentInside = currentDistance <= 0;
    const nextInside = nextDistance <= 0;
    if (currentInside) output.push(current);
    if (currentInside !== nextInside) {
      const amount = currentDistance / (currentDistance - nextDistance);
      output.push({
        x: lerp(current.x, next.x, amount),
        y: lerp(current.y, next.y, amount),
      });
    }
  }
  return output;
}

function drawMapBase(
  context,
  width,
  height,
  control,
  controlEnabled = true,
  mapImage = null,
  allowSchematic = true,
) {
  context.fillStyle = '#070b14';
  context.fillRect(0, 0, width, height);
  const hasTerrain = mapImage?.complete && mapImage.naturalWidth > 0;
  if (hasTerrain) {
    context.drawImage(mapImage, 0, 0, width, height);
    context.fillStyle = 'rgba(3, 7, 18, .24)';
    context.fillRect(0, 0, width, height);
  }

  if (controlEnabled) {
    context.fillStyle = 'rgba(251, 113, 133, .10)';
    context.fillRect(0, 0, width, height);
    if (control.topology === 'linear_lane'
      && Array.isArray(control.frontier_segment)
      && control.frontier_segment.length === 2
      && control.axis) {
      const midpoint = {
        x: (control.frontier_segment[0].x + control.frontier_segment[1].x) / 2,
        y: (control.frontier_segment[0].y + control.frontier_segment[1].y) / 2,
      };
      const polygon = clipRectangleToBlueSide(midpoint, control.axis);
      if (polygon.length >= 3) {
        context.beginPath();
        polygon.forEach((point, index) => index
          ? context.lineTo(point.x * width, point.y * height)
          : context.moveTo(point.x * width, point.y * height));
        context.closePath();
        context.fillStyle = 'rgba(56, 189, 248, .12)';
        context.fill();
      }
    } else if (Array.isArray(control.frontier) && control.frontier.length) {
      context.beginPath();
      context.moveTo(0, 0);
      context.lineTo(control.frontier[0].x * width, 0);
      for (const point of control.frontier) context.lineTo(point.x * width, point.y * height);
      context.lineTo(0, height);
      context.closePath();
      context.fillStyle = 'rgba(56, 189, 248, .13)';
      context.fill();
    }
  }

  context.strokeStyle = 'rgba(148, 163, 184, .12)';
  context.lineWidth = 1;
  for (let index = 1; index < 6; index += 1) {
    const position = (index / 6) * width;
    context.beginPath(); context.moveTo(position, 0); context.lineTo(position, height); context.stroke();
    context.beginPath(); context.moveTo(0, position); context.lineTo(width, position); context.stroke();
  }

  if (controlEnabled && !hasTerrain && allowSchematic && control.topology !== 'linear_lane') {
    context.strokeStyle = 'rgba(34, 211, 238, .20)';
    context.lineWidth = width * 0.065;
    context.beginPath();
    context.moveTo(width * 0.16, 0);
    context.bezierCurveTo(width * 0.34, height * 0.28, width * 0.66, height * 0.72, width * 0.84, height);
    context.stroke();

    context.strokeStyle = 'rgba(226, 232, 240, .18)';
    context.lineWidth = Math.max(2, width * 0.012);
    const lanes = [
      [[0.06, 0.94], [0.94, 0.06]],
      [[0.06, 0.94], [0.12, 0.16], [0.94, 0.06]],
      [[0.06, 0.94], [0.84, 0.88], [0.94, 0.06]],
    ];
    for (const lane of lanes) {
      context.beginPath();
      lane.forEach(([x, y], index) => index
        ? context.lineTo(x * width, y * height)
        : context.moveTo(x * width, y * height));
      context.stroke();
    }

    context.fillStyle = 'rgba(56, 189, 248, .45)';
    context.fillRect(width * 0.025, height * 0.91, width * 0.085, height * 0.065);
    context.fillStyle = 'rgba(251, 113, 133, .45)';
    context.fillRect(width * 0.89, height * 0.025, width * 0.085, height * 0.065);
  }

  if (controlEnabled && control.topology === 'linear_lane'
    && Array.isArray(control.frontier_segment)
    && control.frontier_segment.length === 2) {
    context.beginPath();
    context.moveTo(control.frontier_segment[0].x * width, control.frontier_segment[0].y * height);
    context.lineTo(control.frontier_segment[1].x * width, control.frontier_segment[1].y * height);
    context.strokeStyle = control.blue >= 50
      ? 'rgba(56, 189, 248, .48)'
      : 'rgba(251, 113, 133, .48)';
    context.lineWidth = Math.max(8, width * 0.018);
    context.stroke();
    context.strokeStyle = 'rgba(248, 250, 252, .95)';
    context.lineWidth = Math.max(2, width * 0.006);
    context.stroke();
  } else if (controlEnabled && Array.isArray(control.frontier) && control.frontier.length) {
    context.beginPath();
    curvedPath(context, control.frontier, width, height);
    context.strokeStyle = control.blue >= 50
      ? 'rgba(56, 189, 248, .42)'
      : 'rgba(251, 113, 133, .42)';
    context.lineWidth = Math.max(5, width * 0.012);
    context.stroke();
    context.strokeStyle = 'rgba(248, 250, 252, .92)';
    context.lineWidth = Math.max(2, width * 0.006);
    context.stroke();
  }
}

function drawChampion(context, point, participant, image, width) {
  const radius = clamp(width * 0.026, 11, 19);
  const x = point.x * width;
  const y = point.y * width;
  context.save();
  context.beginPath();
  context.arc(x, y, radius, 0, Math.PI * 2);
  context.clip();
  if (image?.complete && image.naturalWidth > 0) {
    context.drawImage(image, x - radius, y - radius, radius * 2, radius * 2);
  } else {
    context.fillStyle = '#111827';
    context.fillRect(x - radius, y - radius, radius * 2, radius * 2);
    context.fillStyle = '#f8fafc';
    context.font = `800 ${Math.max(8, radius * 0.8)}px system-ui`;
    context.textAlign = 'center';
    context.textBaseline = 'middle';
    context.fillText(String(participant?.champion_name || '?').slice(0, 2).toUpperCase(), x, y);
  }
  context.restore();
  context.beginPath();
  context.arc(x, y, radius + 1, 0, Math.PI * 2);
  context.strokeStyle = finite(participant?.team_id) === BLUE_TEAM ? BLUE : RED;
  context.lineWidth = 3;
  context.stroke();
}

function drawGraph(canvas, samples, cursorMs, durationMs) {
  const width = Math.max(260, canvas.clientWidth || 620);
  const height = Math.max(92, canvas.clientHeight || 112);
  const context = prepareCanvas(canvas, width, height);
  context.fillStyle = '#090d17';
  context.fillRect(0, 0, width, height);
  const padX = 13;
  const padY = 12;
  context.strokeStyle = 'rgba(148, 163, 184, .18)';
  context.setLineDash([4, 5]);
  context.beginPath();
  context.moveTo(padX, height / 2);
  context.lineTo(width - padX, height / 2);
  context.stroke();
  context.setLineDash([]);
  if (samples.length) {
    context.beginPath();
    samples.forEach((sample, index) => {
      const x = padX + (finite(sample.timestamp_ms) / Math.max(1, durationMs)) * (width - (padX * 2));
      const y = padY + (1 - (sample.blue / 100)) * (height - (padY * 2));
      if (index) context.lineTo(x, y); else context.moveTo(x, y);
    });
    context.strokeStyle = BLUE;
    context.lineWidth = 2;
    context.stroke();
  }
  const control = interpolateControlSample(samples, cursorMs);
  const scrubX = padX + (cursorMs / Math.max(1, durationMs)) * (width - (padX * 2));
  const scrubY = padY + (1 - (control.blue / 100)) * (height - (padY * 2));
  context.strokeStyle = 'rgba(248, 250, 252, .8)';
  context.lineWidth = 1;
  context.beginPath(); context.moveTo(scrubX, 0); context.lineTo(scrubX, height); context.stroke();
  context.fillStyle = BLUE;
  context.beginPath(); context.arc(scrubX, scrubY, 4, 0, Math.PI * 2); context.fill();
}

export function replayMode(replay) {
  const frames = Array.isArray(replay?.frames) ? replay.frames : [];
  if (!frames.length) return 'empty';
  const hasPositions = frames.some((frame) => Array.isArray(frame?.players)
    && frame.players.some((player) => Number.isFinite(Number(player?.x))
      && Number.isFinite(Number(player?.y))));
  if (!hasPositions || replay?.availability?.positions === false) return 'unavailable';
  return replay?.availability?.control_estimate === true ? 'control' : 'movement';
}

function replayMarkup(controlEnabled, linearControl = false, hasMapAsset = false) {
  const estimateName = linearControl ? 'Estimated Lane Control' : 'Estimated Map Control';
  return `
    ${controlEnabled ? `
      <div class="map-replay-control-readout" aria-label="${estimateName}">
        <strong class="map-replay-blue"><span data-control-blue>50%</span> Blue</strong>
        <div class="map-control-center"><span data-control-label>${estimateName}</span><div class="map-control-bar" aria-hidden="true"><span data-control-bar></span></div><small data-control-state></small></div>
        <strong class="map-replay-red">Red <span data-control-red>50%</span></strong>
      </div>` : `
      <div class="map-replay-calibration-note" role="note">
        <strong>Movement replay</strong>
        <span data-control-reason>Territorial pressure is not calibrated for this map.</span>
      </div>`}
    <div class="map-replay-stats">
      <div><span>Kills</span><strong data-stat-kills>0 - 0</strong></div>
      <div><span>Gold</span><strong data-stat-gold>0 - 0</strong></div>
      <div><span>Turrets</span><strong data-stat-turrets>0 - 0</strong></div>
      <div><span>Game clock</span><strong data-stat-clock>0:00</strong></div>
    </div>
    <div class="map-replay-canvas-wrap">
      <canvas class="map-replay-canvas" data-map-canvas role="img" aria-label="${controlEnabled
        ? `Animated champion movement and ${estimateName.toLowerCase()}${hasMapAsset ? ' with bundled terrain when available' : ' on a neutral coordinate grid'}`
        : `Animated champion movement${hasMapAsset ? ' with bundled terrain when available' : ' on a neutral coordinate grid'}`}"></canvas>
      <div class="map-replay-terrain-label" data-terrain-label role="status" aria-live="polite"></div>
      <div class="map-replay-event" data-replay-event role="status" aria-live="polite" hidden></div>
    </div>
    <div class="map-replay-graph-block${controlEnabled ? '' : ' map-replay-timeline-block'}">
      <div class="map-replay-graph-heading"><strong>${controlEnabled ? `${linearControl ? 'Lane' : 'Map'} Control Over Time` : 'Replay Timeline'}</strong><span>0:00 <i data-graph-end>0:00</i></span></div>
      ${controlEnabled ? `<canvas class="map-replay-graph" data-graph-canvas role="img" aria-label="Estimated blue ${linearControl ? 'lane' : 'map'} control over game time"></canvas>` : ''}
      <input class="map-replay-scrubber" data-replay-scrubber type="range" min="0" max="1000" value="0" aria-label="Replay position">
    </div>
    <div class="map-replay-actions">
      <button class="btn-secondary compact" type="button" data-replay-back aria-label="Back 15 seconds">-15s</button>
      <button class="btn-cta compact" type="button" data-replay-play>Play</button>
      <button class="btn-secondary compact" type="button" data-replay-forward aria-label="Forward 15 seconds">+15s</button>
      <label>Speed <select data-replay-speed aria-label="Replay speed"><option value="1">1x</option><option value="4">4x</option><option value="8" selected>8x</option><option value="16">16x</option></select></label>
      <span data-replay-time>0:00 / 0:00</span>
    </div>
    <p class="map-replay-disclaimer">${controlEnabled
      ? linearControl
        ? 'Estimated one-dimensional lane pressure from interpolated Riot Timeline champion positions, levels, and gold. The straight frontier is not measured vision, pathing, or brush occupancy.'
        : 'Estimated territorial pressure from interpolated Riot Timeline champion positions, levels, and gold. This is not measured fog-of-war or exact vision coverage.'
      : `Champion positions are interpolated from Riot Timeline samples${hasMapAsset ? '. Bundled terrain is a visual reference when available; a neutral coordinate grid is used if it cannot load' : ' on a neutral coordinate grid'}. Territorial pressure is not calibrated for this map.`}</p>`;
}

export function mountMapControlReplay(root, replay, options = {}) {
  if (!root) throw new Error('Map replay root is missing.');
  const mode = replayMode(replay);
  if (mode === 'empty') {
    throw new Error(
      replay?.availability?.positions_reason
        || replay?.availability?.reason
        || 'Riot returned no replay frames for this match.',
    );
  }
  if (mode === 'unavailable') {
    root.innerHTML = '<div class="map-replay-unavailable"><strong>Movement replay unavailable</strong><span></span></div>';
    root.querySelector('span').textContent = replay?.availability?.positions_reason
      || replay?.availability?.reason
      || 'No usable positional timeline was returned.';
    return { pause() {}, resume() {}, destroy() { root.replaceChildren(); } };
  }

  const controlEnabled = mode === 'control';
  const linearControl = replay?.control_model?.topology === 'linear_lane';
  const asset = mapAsset(replay?.map_id);
  const assetUrl = mapAssetUrl(asset);
  root.innerHTML = replayMarkup(controlEnabled, linearControl, Boolean(assetUrl));
  const controlReason = root.querySelector('[data-control-reason]');
  if (controlReason) {
    controlReason.textContent = replay?.availability?.control_reason
      || replay?.availability?.reason
      || 'Territorial pressure is not calibrated for this map.';
  }
  const mapCanvas = root.querySelector('[data-map-canvas]');
  const graphCanvas = root.querySelector('[data-graph-canvas]');
  const scrubber = root.querySelector('[data-replay-scrubber]');
  const playButton = root.querySelector('[data-replay-play]');
  const eventBanner = root.querySelector('[data-replay-event]');
  const terrainLabel = root.querySelector('[data-terrain-label]');
  const durationMs = Math.max(1, finite(replay.duration_ms, replay.frames.at(-1)?.timestamp_ms || 1));
  const participantById = participantLookup(replay);
  const frames = [...replay.frames].sort((a, b) => finite(a.timestamp_ms) - finite(b.timestamp_ms));
  const controlSamples = controlEnabled
    ? prepareControlFrames(frames, replay).map((frame) => computeControlSample(frame, replay))
    : [];
  let mapImage = null;
  let terrainState = assetUrl ? 'loading' : 'neutral';

  function updateTerrainPresentation() {
    if (!terrainLabel) return;
    const presentation = terrainPresentation(
      replay,
      options,
      asset,
      terrainState,
      controlEnabled,
      linearControl,
    );
    terrainLabel.textContent = presentation.label;
    mapCanvas.setAttribute('aria-label', presentation.canvasLabel);
    terrainLabel.dataset.state = terrainState;
    terrainLabel.dataset.mismatch = String(presentation.mismatch);
    mapCanvas.dataset.terrainState = terrainState;
  }

  updateTerrainPresentation();
  if (assetUrl && typeof Image !== 'undefined') {
    mapImage = new Image();
    mapImage.decoding = 'async';
    mapImage.addEventListener('load', () => {
      terrainState = 'ready';
      updateTerrainPresentation();
      draw(true);
    }, { once: true });
    mapImage.addEventListener('error', () => {
      terrainState = 'error';
      mapImage = null;
      updateTerrainPresentation();
      draw(true);
    }, { once: true });
    mapImage.src = assetUrl;
  } else if (assetUrl) {
    terrainState = 'error';
    updateTerrainPresentation();
  }
  const images = new Map();
  if (typeof Image !== 'undefined' && typeof options.championImage === 'function') {
    for (const participant of replay.participants || []) {
      const image = new Image();
      image.decoding = 'async';
      image.addEventListener('load', () => draw(true), { once: true });
      image.src = options.championImage(participant.champion_name);
      images.set(finite(participant.participant_id), image);
    }
  }

  const abortController = new AbortController();
  const signal = abortController.signal;
  const reducedMotion = globalThis.matchMedia?.('(prefers-reduced-motion: reduce)').matches === true;
  let cursorMs = 0;
  let playing = false;
  let speed = 8;
  let frameRequest = 0;
  let destroyed = false;
  let lastTick = 0;
  let lastDraw = 0;
  let lastDomUpdate = -Infinity;
  let announcedEventId = null;

  root.querySelector('[data-graph-end]').textContent = formatClock(durationMs);

  function draw(forceDom = false) {
    if (destroyed || !root.isConnected) return;
    const state = interpolateReplayState({ ...replay, frames }, cursorMs);
    const control = controlEnabled
      ? interpolateControlSample(controlSamples, cursorMs)
      : { blue: 50, red: 50, frontier: [] };
    const width = Math.max(280, mapCanvas.clientWidth || root.clientWidth || 560);
    const context = prepareCanvas(mapCanvas, width, width);
    drawMapBase(
      context,
      width,
      width,
      control,
      controlEnabled,
      mapImage,
      terrainState !== 'error',
    );
    for (const player of state.players) {
      const participant = participantById.get(finite(player.participant_id));
      const point = normalizedPoint(player, replay.coordinates);
      drawChampion(context, point, participant, images.get(finite(player.participant_id)), width);
    }
    if (graphCanvas) drawGraph(graphCanvas, controlSamples, cursorMs, durationMs);
    scrubber.value = String(Math.round((cursorMs / durationMs) * 1000));

    if (forceDom || performance.now() - lastDomUpdate >= 240) {
      lastDomUpdate = performance.now();
      if (controlEnabled) {
        const bluePercent = clamp(Math.round(control.blue), 0, 100);
        root.querySelector('[data-control-blue]').textContent = `${bluePercent}%`;
        root.querySelector('[data-control-red]').textContent = `${100 - bluePercent}%`;
        root.querySelector('[data-control-bar]').style.width = `${bluePercent}%`;
        const stateLabel = root.querySelector('[data-control-state]');
        if (stateLabel) {
          stateLabel.textContent = linearControl && control.split
            ? 'Split pressure · no single front line'
            : linearControl ? 'Single-lane frontier' : 'Territorial pressure frontier';
        }
      }
      const blue = state.teams.find((team) => finite(team.team_id) === BLUE_TEAM) || {};
      const red = state.teams.find((team) => finite(team.team_id) === RED_TEAM) || {};
      root.querySelector('[data-stat-kills]').textContent = `${finite(blue.kills)} - ${finite(red.kills)}`;
      root.querySelector('[data-stat-gold]').textContent = `${compactNumber(blue.gold)} - ${compactNumber(red.gold)}`;
      root.querySelector('[data-stat-turrets]').textContent = `${finite(blue.turrets)} - ${finite(red.turrets)}`;
      root.querySelector('[data-stat-clock]').textContent = formatClock(cursorMs);
      root.querySelector('[data-replay-time]').textContent = `${formatClock(cursorMs)} / ${formatClock(durationMs)}`;
      scrubber.setAttribute(
        'aria-valuetext',
        `${formatClock(cursorMs)} of ${formatClock(durationMs)}`,
      );
      const activeEvent = [...(replay.events || [])].reverse().find((event) =>
        MAJOR_EVENT_KINDS.has(event.kind)
        && finite(event.timestamp_ms) <= cursorMs
        && cursorMs - finite(event.timestamp_ms) <= 12000);
      if (activeEvent) {
        const eventId = String(activeEvent.id || `${activeEvent.kind}-${activeEvent.timestamp_ms}`);
        if (announcedEventId !== eventId) {
          const team = finite(activeEvent.team_id) === BLUE_TEAM ? 'Blue' : finite(activeEvent.team_id) === RED_TEAM ? 'Red' : 'A team';
          eventBanner.textContent = `${formatClock(activeEvent.timestamp_ms)} · ${team} · ${eventLabel(activeEvent)}`;
          eventBanner.dataset.team = String(finite(activeEvent.team_id));
          eventBanner.hidden = false;
          announcedEventId = eventId;
        }
      } else if (announcedEventId !== null) {
        eventBanner.hidden = true;
        eventBanner.textContent = '';
        delete eventBanner.dataset.team;
        announcedEventId = null;
      }
    }
  }

  function schedule() {
    if (!playing || destroyed || frameRequest) return;
    frameRequest = requestAnimationFrame(tick);
  }

  function setPlaying(next) {
    playing = Boolean(next) && cursorMs < durationMs && !destroyed;
    playButton.textContent = playing ? 'Pause' : cursorMs >= durationMs ? 'Replay' : 'Play';
    playButton.setAttribute('aria-pressed', String(playing));
    lastTick = 0;
    if (playing) schedule();
    else if (frameRequest) {
      cancelAnimationFrame(frameRequest);
      frameRequest = 0;
    }
  }

  function seek(nextMs) {
    cursorMs = clamp(finite(nextMs), 0, durationMs);
    if (cursorMs >= durationMs) setPlaying(false);
    draw(true);
  }

  function tick(now) {
    frameRequest = 0;
    if (!playing || destroyed) return;
    if (!lastTick) lastTick = now;
    const elapsed = Math.min(250, now - lastTick);
    lastTick = now;
    cursorMs = Math.min(durationMs, cursorMs + (elapsed * speed));
    if (now - lastDraw >= FRAME_BUDGET_MS || cursorMs >= durationMs) {
      lastDraw = now;
      draw();
    }
    if (cursorMs >= durationMs) setPlaying(false); else schedule();
  }

  playButton.addEventListener('click', () => {
    if (cursorMs >= durationMs) cursorMs = 0;
    setPlaying(!playing);
    draw(true);
  }, { signal });
  root.querySelector('[data-replay-back]').addEventListener('click', () => seek(cursorMs - 15000), { signal });
  root.querySelector('[data-replay-forward]').addEventListener('click', () => seek(cursorMs + 15000), { signal });
  root.querySelector('[data-replay-speed]').addEventListener('change', (event) => {
    speed = clamp(finite(event.target.value, 8), 1, 16);
  }, { signal });
  scrubber.addEventListener('input', () => seek((finite(scrubber.value) / 1000) * durationMs), { signal });
  document.addEventListener('visibilitychange', () => {
    if (document.hidden) setPlaying(false);
  }, { signal });

  const resizeObserver = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(() => draw(true));
  resizeObserver?.observe(root);
  const visibilityObserver = typeof IntersectionObserver === 'undefined' ? null : new IntersectionObserver(
    (entries) => {
      if (!entries.some((entry) => entry.isIntersecting)) setPlaying(false);
    },
    { threshold: 0.05 },
  );
  visibilityObserver?.observe(root);
  draw(true);
  setPlaying(playing);

  return {
    pause() { setPlaying(false); },
    resume() { if (!reducedMotion) setPlaying(true); },
    seek,
    destroy() {
      destroyed = true;
      setPlaying(false);
      abortController.abort();
      resizeObserver?.disconnect();
      visibilityObserver?.disconnect();
      images.clear();
      mapImage = null;
      root.replaceChildren();
    },
  };
}

export const mapControlReplayTestHooks = Object.freeze({
  normalizedPoint,
  pressureAt,
  interpolateControlSample,
  surroundingFrames,
  shouldSnapBaseTransition,
  replayMode,
  mapAsset,
  terrainPresentation,
});
