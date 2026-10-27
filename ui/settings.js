/* global app */
const FIELD_GROUPS = [
  {
    category: 'Rang & MMR',
    items: [
      ['rank', 'Rang & Division'],
      ['mmr', 'MMR Actuel'],
      ['mmrChange', 'MMR (Dernier match)'],
      ['mmrSession', 'MMR (Session)'],
      ['gamemode', 'Mode de jeu'],
    ],
  },
  {
    category: 'Bilan de Session',
    items: [
      ['wins', 'Victoires'],
      ['losses', 'Défaites'],
      ['streak', 'Série (Streak)'],
      ['winrate', 'Winrate (%)'],
      ['games', 'Parties jouées'],
      ['best', 'Meilleure série'],
    ],
  },
  {
    category: 'Événements en Match',
    items: [
      ['goals', 'Buts'],
      ['assists', 'Passes décisives'],
      ['saves', 'Arrêts'],
      ['shots', 'Tirs'],
      ['demos', 'Démolitions'],
      ['deaths', 'Morts subies'],
    ],
  },
];

const FIELDS = FIELD_GROUPS.flatMap((g) => g.items);

const COLOR_KEYS = [
  ['colorBg', '#0e192d'],
  ['colorText', '#ffffff'],
  ['colorMuted', '#8ea3c6'],
  ['colorWin', '#2ecc71'],
  ['colorLoss', '#ff4757'],
  ['colorAccent', '#36a9ff'],
  ['colorGoal', '#ffc107'],
];

const RANK_FILES = {
  0: 'unranked.png',
  1: 'bronze-1.png', 2: 'bronze-2.png', 3: 'bronze-3.png',
  4: 'silver-1.png', 5: 'silver-2.png', 6: 'silver-3.png',
  7: 'gold-1.png', 8: 'gold-2.png', 9: 'gold-3.png',
  10: 'platinum-1.png', 11: 'platinum-2.png', 12: 'platinum-3.png',
  13: 'diamond-1.png', 14: 'diamond-2.png', 15: 'diamond-3.png',
  16: 'champion-1.png', 17: 'champion-2.png', 18: 'champion-3.png',
  19: 'grand-champion-1.png', 20: 'grand-champion-2.png', 21: 'grand-champion-3.png',
  22: 'supersonic-legend.png',
};

function getRankImg(tier, size = 36) {
  const file = RANK_FILES[Number(tier)] || 'unranked.png';
  return `<img src="ranks/${file}" alt="Rank" width="${size}" height="${size}" style="object-fit: contain;">`;
}

const $ = (id) => document.getElementById(id);
let S = null;          // état complet reçu du processus principal
let api = null;        // état de la Stats API (fichiers .ini)
let formReady = false;

// ---------------------------------------------------------------------------
// Utilitaires
// ---------------------------------------------------------------------------

const fmtStreak = (n) => (n > 0 ? `+${n}` : String(n));

function calculateRankFrontend(mmr, mode = '2v2') {
  mmr = Number(mmr) || 0;
  if (mmr <= 0) return { name: 'Unranked', div: 1, tier: 0 };

  if (mode === '1v1') {
    const brackets1v1 = [
      ['Bronze I', 0, 155, 1],
      ['Bronze II', 155, 215, 2],
      ['Bronze III', 215, 275, 3],
      ['Silver I', 275, 335, 4],
      ['Silver II', 335, 395, 5],
      ['Silver III', 395, 455, 6],
      ['Gold I', 455, 515, 7],
      ['Gold II', 515, 575, 8],
      ['Gold III', 575, 635, 9],
      ['Platinum I', 635, 695, 10],
      ['Platinum II', 695, 755, 11],
      ['Platinum III', 755, 815, 12],
      ['Diamond I', 815, 875, 13],
      ['Diamond II', 875, 935, 14],
      ['Diamond III', 935, 995, 15],
      ['Champion I', 995, 1055, 16],
      ['Champion II', 1055, 1115, 17],
      ['Champion III', 1115, 1175, 18],
      ['Grand Champion I', 1175, 1235, 19],
      ['Grand Champion II', 1235, 1295, 20],
      ['Grand Champion III', 1295, 1355, 21],
    ];
    if (mmr >= 1355) return { name: 'Supersonic Legend', div: 1, tier: 22 };
    for (const [name, min, max, tier] of brackets1v1) {
      if (mmr < max) {
        const width = (max - min) / 4;
        const div = 1 + Math.min(3, Math.max(0, Math.floor((mmr - min) / width)));
        return { name, div, tier };
      }
    }
    return { name: 'Supersonic Legend', div: 1, tier: 22 };
  }

  // Grille officielle 2v2 / 3v3 (1656 MMR = GC2 Div 3)
  const brackets = [
    ['Bronze I', 0, 175, 1],
    ['Bronze II', 175, 235, 2],
    ['Bronze III', 235, 295, 3],
    ['Silver I', 295, 355, 4],
    ['Silver II', 355, 415, 5],
    ['Silver III', 415, 475, 6],
    ['Gold I', 475, 535, 7],
    ['Gold II', 535, 595, 8],
    ['Gold III', 595, 655, 9],
    ['Platinum I', 655, 715, 10],
    ['Platinum II', 715, 775, 11],
    ['Platinum III', 775, 835, 12],
    ['Diamond I', 835, 915, 13],
    ['Diamond II', 915, 995, 14],
    ['Diamond III', 995, 1075, 15],
    ['Champion I', 1075, 1195, 16],
    ['Champion II', 1195, 1315, 17],
    ['Champion III', 1315, 1435, 18],
    ['Grand Champion I', 1435, 1575, 19],
    ['Grand Champion II', 1575, 1715, 20],
    ['Grand Champion III', 1715, 1875, 21],
  ];
  if (mmr >= 1875) return { name: 'Supersonic Legend', div: 1, tier: 22 };
  for (const [name, min, max, tier] of brackets) {
    if (mmr < max) {
      const width = (max - min) / 4;
      const div = 1 + Math.min(3, Math.max(0, Math.floor((mmr - min) / width)));
      return { name, div, tier };
    }
  }
  return { name: 'Supersonic Legend', div: 1, tier: 22 };
}

function getDivisionProgress(mmr, mode) {
  const is1v1 = (mode || '').toLowerCase() === '1v1';
  const brackets = is1v1 ? [
    ['Bronze I', 0, 155], ['Bronze II', 155, 215], ['Bronze III', 215, 275],
    ['Silver I', 275, 335], ['Silver II', 335, 395], ['Silver III', 395, 455],
    ['Gold I', 455, 515], ['Gold II', 515, 575], ['Gold III', 575, 635],
    ['Platinum I', 635, 695], ['Platinum II', 695, 755], ['Platinum III', 755, 815],
    ['Diamond I', 815, 875], ['Diamond II', 875, 935], ['Diamond III', 935, 995],
    ['Champion I', 995, 1055], ['Champion II', 1055, 1115], ['Champion III', 1115, 1175],
    ['Grand Champion I', 1175, 1235], ['Grand Champion II', 1235, 1295], ['Grand Champion III', 1295, 1355]
  ] : [
    ['Bronze I', 0, 175], ['Bronze II', 175, 235], ['Bronze III', 235, 295],
    ['Silver I', 295, 355], ['Silver II', 355, 415], ['Silver III', 415, 475],
    ['Gold I', 475, 535], ['Gold II', 535, 595], ['Gold III', 595, 655],
    ['Platinum I', 655, 715], ['Platinum II', 715, 775], ['Platinum III', 775, 835],
    ['Diamond I', 835, 915], ['Diamond II', 915, 995], ['Diamond III', 995, 1075],
    ['Champion I', 1075, 1195], ['Champion II', 1195, 1315], ['Champion III', 1315, 1435],
    ['Grand Champion I', 1435, 1575], ['Grand Champion II', 1575, 1715], ['Grand Champion III', 1715, 1875]
  ];
  const maxMmr = is1v1 ? 1355 : 1875;
  if (mmr >= maxMmr) return { percent: 100, neededNext: 0 };

  for (const [name, min, max] of brackets) {
    if (mmr < max) {
      const width = (max - min) / 4;
      const divIndex = Math.min(3, Math.max(0, Math.floor((mmr - min) / width)));
      const divMin = min + divIndex * width;
      const divMax = divMin + width;
      const percent = Math.min(100, Math.max(0, Math.round(((mmr - divMin) / width) * 100)));
      const needed = Math.max(0, Math.ceil(divMax - mmr));
      return { percent, neededNext: needed };
    }
  }
  return { percent: 100, neededNext: 0 };
}

let savedTimer;
function flashSaved(text = 'Enregistré') {
  $('saved').textContent = text;
  clearTimeout(savedTimer);
  savedTimer = setTimeout(() => { $('saved').textContent = ''; }, 1600);
}

let isPreviewing = false;

function updatePreviewButton() {
  const btn = $('previewOverlayBtn');
  if (!btn) return;
  if (isPreviewing) {
    btn.textContent = "Arrêter l'aperçu";
    btn.classList.remove('secondary');
    btn.classList.add('primary');
    btn.setAttribute('aria-pressed', 'true');
  } else {
    btn.textContent = "Prévisualiser l'overlay";
    btn.classList.remove('primary');
    btn.classList.add('secondary');
    btn.setAttribute('aria-pressed', 'false');
  }
}

let appVersion = '1.3.1';
// Dépôt GitHub dont les releases servent à la vérification des mises à jour.
const RELEASES_REPO = 'NeyorDEV/rl-tracker-suite';
let latestReleaseInfo = null;

function parseSemver(v) {
  if (!v) return [0, 0, 0];
  const clean = String(v).replace(/^v/i, '').trim();
  const parts = clean.split('.').map(n => parseInt(n, 10) || 0);
  while (parts.length < 3) parts.push(0);
  return parts;
}

function isNewerVersion(remote, current) {
  const [rMaj, rMin, rPat] = parseSemver(remote);
  const [cMaj, cMin, cPat] = parseSemver(current);
  if (rMaj !== cMaj) return rMaj > cMaj;
  if (rMin !== cMin) return rMin > cMin;
  return rPat > cPat;
}

async function checkAppUpdates(silent = false) {
  const statusEl = $('updateStatusText');
  const banner = $('updateBanner');
  const checkBtn = $('checkUpdateBtn');

  if (!silent && statusEl) {
    statusEl.textContent = 'Vérification en cours...';
    statusEl.className = 'update-status-text';
    if (checkBtn) checkBtn.disabled = true;
  }

  try {
    const res = await fetch(`https://api.github.com/repos/${RELEASES_REPO}/releases/latest`, {
      headers: { 'Accept': 'application/vnd.github.v3+json' },
    });
    // 404 = aucune release publiée pour l'instant : on est forcément à jour.
    const data = res.status === 404 ? { tag_name: '' } : res.ok ? await res.json() : null;
    if (!data) throw new Error('HTTP ' + res.status);
    latestReleaseInfo = res.status === 404 ? null : data;

    const remoteTag = data.tag_name || data.name || '';
    if (isNewerVersion(remoteTag, appVersion)) {
      if (banner) {
        $('updateVersion').textContent = remoteTag.startsWith('v') ? remoteTag : 'v' + remoteTag;
        if ($('updateCurrentHint')) $('updateCurrentHint').textContent = `(actuelle : v${appVersion})`;
        const notes = (data.body || '').split('\n').map(l => l.trim()).filter(l => l && !l.startsWith('#') && !l.startsWith('---')).slice(0, 2).join(' — ');
        $('updateNotes').textContent = notes || data.name || 'Une nouvelle mise à jour est disponible sur GitHub.';
        banner.hidden = false;
      }
      if (statusEl) {
        statusEl.textContent = `Mise à jour disponible (${remoteTag})`;
        statusEl.className = 'update-status-text found';
      }
    } else {
      if (!silent && statusEl) {
        statusEl.textContent = `Vous êtes à jour (v${appVersion})`;
        statusEl.className = 'update-status-text ok';
        setTimeout(() => { if (statusEl) statusEl.textContent = ''; }, 4500);
      }
    }
  } catch (err) {
    console.warn('Vérification des mises à jour impossible:', err);
    if (!silent && statusEl) {
      statusEl.textContent = 'Impossible de vérifier (hors ligne)';
      statusEl.className = 'update-status-text error';
      setTimeout(() => { if (statusEl) statusEl.textContent = ''; }, 4500);
    }
  } finally {
    if (!silent && checkBtn) checkBtn.disabled = false;
  }
}

async function save(patch) {
  S = await app.save(patch);
  renderForm();
  renderSession();
  flashSaved();
}

function setPressed(container, value) {
  for (const b of container.querySelectorAll('button')) {
    b.setAttribute('aria-pressed', String(b.dataset.value === String(value) || b.dataset.pos === value));
  }
}

// ---------------------------------------------------------------------------
// Colonne session
// ---------------------------------------------------------------------------

function renderSession() {
  const s = S.session;

  const status = $('status');
  status.classList.toggle('connected', S.connected);
  status.querySelector('span').textContent = S.inMatch
    ? 'Match en cours'
    : S.connected ? 'Connecté à Rocket League' : 'En attente de Rocket League';

  // Bloc Rang & MMR
  if ($('sideRankImg')) $('sideRankImg').innerHTML = getRankImg(s.rankTier, 36);
  $('sideRankName').textContent = s.rankName || 'Unranked';
  $('sideRankDiv').textContent = `Division ${s.division || 1}`;
  $('sideMmr').textContent = Number(s.currentMmr || 0).toFixed(1);
  const delta = Number(s.sessionMmrChange || 0);
  $('sideMmrChange').textContent = (delta >= 0 ? '+' : '') + delta.toFixed(1);
  $('sideMmrChange').style.color = delta > 0 ? 'var(--good)' : delta < 0 ? 'var(--orange)' : 'var(--muted)';

  const prog = getDivisionProgress(Number(s.currentMmr || 0), s.focusedPlaylist || S?.settings?.counting?.focusedPlaylist);
  const nextTarget = (s.division || 1) < 4 ? `Div. ${(s.division || 1) + 1}` : 'Rank Up';
  if ($('sideDivProgPct')) $('sideDivProgPct').textContent = `${prog.percent}%`;
  if ($('sideDivProgNeeded')) $('sideDivProgNeeded').textContent = prog.neededNext > 0 ? `+${prog.neededNext} MMR (${nextTarget})` : 'Division max';
  if ($('sideDivProgFill')) $('sideDivProgFill').style.width = `${prog.percent}%`;

  $('streak').textContent = fmtStreak(s.streak);
  $('streakBox').classList.toggle('up', s.streak > 0);
  $('streakBox').classList.toggle('down', s.streak < 0);
  $('wins').textContent = s.wins;
  $('losses').textContent = s.losses;
  $('winrate').textContent = s.games ? `${Math.round((100 * s.wins) / s.games)}%` : '–';

  const recent = $('recent');
  recent.innerHTML = '';
  for (const r of s.recent.slice(-15)) {
    const el = document.createElement('span');
    el.className = r.won ? 'w' : r.abandoned ? 'a' : 'l';
    el.title = `${r.won ? 'Victoire' : r.abandoned ? 'Abandon' : 'Défaite'} - ${r.playlist} (${r.mmrChange >= 0 ? '+' : ''}${r.mmrChange.toFixed(1)} MMR)`;
    recent.appendChild(el);
  }

  const t = s.totals;
  const rows = [
    ['Parties', s.games],
    ['MMR session', (delta >= 0 ? '+' : '') + delta.toFixed(1)],
    ['Buts', t.goals],
    ['Passes décisives', t.assists],
    ['Arrêts', t.saves],
    ['Tirs', t.shots],
    ['Démolitions', t.demos],
    ['Morts', t.deaths],
    ['Meilleure streak', fmtStreak(s.bestStreak)],
    ['Pire série', fmtStreak(s.worstStreak)],
  ];
  $('totals').innerHTML = '';
  for (const [label, value] of rows) {
    const dt = document.createElement('dt');
    const dd = document.createElement('dd');
    dt.textContent = label;
    dd.textContent = value;
    $('totals').append(dt, dd);
  }

  const per = $('perPlaylist');
  per.innerHTML = '';
  for (const pl of Object.values(s.playlists)) {
    const row = document.createElement('div');
    const name = document.createElement('span');
    const record = document.createElement('em');
    const streak = document.createElement('b');
    name.textContent = pl.name;
    record.textContent = `${pl.wins}V – ${pl.losses}D`;
    streak.textContent = fmtStreak(pl.streak);
    streak.style.color = pl.streak > 0 ? 'var(--blue)' : pl.streak < 0 ? 'var(--orange)' : '';
    row.append(name, record, streak);
    per.appendChild(row);
  }

  const since = new Date(s.startedAt);
  $('since').textContent = `Session commencée le ${since.toLocaleDateString('fr-FR')} à ${since.toLocaleTimeString('fr-FR', { hour: '2-digit', minute: '2-digit' })}`;

  const p = S.settings.player || {};
  const activePseudo = p.customName || p.detectedName || '';
  const platStr = p.platform ? (p.platform === 'steam' ? 'Steam' : 'Epic Games') : '';
  if ($('autoDetectTitle')) {
    $('autoDetectTitle').textContent = activePseudo
      ? `Profil actif : ${activePseudo}${platStr ? ` (${platStr})` : ''}`
      : 'Détection automatique du profil';
  }
  if ($('autoDetectDesc')) {
    $('autoDetectDesc').textContent = activePseudo
      ? 'Synchronisé automatiquement avec le client Rocket League.'
      : 'Votre pseudo et plateforme s\'adaptent automatiquement au lancement de Rocket League.';
  }
  if ($('detected')) {
    $('detected').textContent = activePseudo ? `Joueur actif : ${activePseudo}` : '';
  }

  updateHistoryView();
}

let selectedSessionId = 'current';
let selectedAccountFilter = 'all';
let selectedPlaylistFilter = 'all';

function renderSessionSelector() {
  const sel = $('sessionHistorySelect');
  const filterSel = $('sessionAccountFilter');
  if (!sel) return;

  const history = S.history || [];
  const currentVal = selectedSessionId;
  const currentFilter = selectedAccountFilter;

  // Récupération des comptes uniques présents dans l'historique + compte actuel
  if (filterSel) {
    const accounts = new Set();
    const curP = S.settings?.player?.customName || S.settings?.player?.detectedName;
    if (curP) accounts.add(curP);
    for (const h of history) {
      if (h.playerName) accounts.add(h.playerName);
    }

    if (accounts.size > 0) {
      filterSel.style.display = 'inline-block';
      filterSel.innerHTML = '<option value="all">Tous les comptes</option>';
      for (const acc of Array.from(accounts).sort()) {
        const opt = document.createElement('option');
        opt.value = acc;
        opt.textContent = acc;
        filterSel.appendChild(opt);
      }
      filterSel.value = accounts.has(currentFilter) ? currentFilter : 'all';
      selectedAccountFilter = filterSel.value;
    } else {
      filterSel.style.display = 'none';
    }
  }

  sel.innerHTML = '';

  const cS = S.session || {};
  const curPName = S.settings?.player?.customName || S.settings?.player?.detectedName || '';
  if (selectedAccountFilter === 'all' || selectedAccountFilter === curPName) {
    const optCur = document.createElement('option');
    optCur.value = 'current';
    const cDelta = Number(cS.sessionMmrChange || 0);
    const cDeltaStr = (cDelta >= 0 ? '+' : '') + cDelta.toFixed(1);
    const badge = curPName ? ` [${curPName}]` : '';
    optCur.textContent = `Session en cours${badge} (Aujourd'hui) • ${cDeltaStr} MMR (${cS.wins || 0}V - ${cS.losses || 0}D)`;
    sel.appendChild(optCur);
  }

  for (const item of history) {
    if (String(item.id) === String(cS.startedAt)) continue;
    if (selectedAccountFilter !== 'all' && item.playerName && item.playerName !== selectedAccountFilter) continue;
    const opt = document.createElement('option');
    opt.value = item.id;
    const d = new Date(item.startedAt);
    const dateStr = d.toLocaleDateString('fr-FR', { weekday: 'short', day: 'numeric', month: 'short' });
    const timeStr = d.toLocaleTimeString('fr-FR', { hour: '2-digit', minute: '2-digit' });
    const delta = Number(item.sessionMmrChange || 0);
    const deltaStr = (delta >= 0 ? '+' : '') + delta.toFixed(1);
    const pTag = item.playerName ? ` • ${item.playerName}` : '';
    opt.textContent = `${dateStr} à ${timeStr}${pTag} • ${deltaStr} MMR (${item.wins || 0}V - ${item.losses || 0}D)`;
    sel.appendChild(opt);
  }

  const exists = Array.from(sel.options).some(o => o.value === currentVal);
  sel.value = exists ? currentVal : (sel.options[0]?.value || 'current');
  selectedSessionId = sel.value;
}

function updateHistoryView() {
  renderSessionSelector();

  const isCurrent = selectedSessionId === 'current';
  const delBtn = $('deleteSessionBtn');
  if (delBtn) delBtn.style.display = isCurrent ? 'none' : 'inline-block';

  let activeSession = S.session || {};
  if (!isCurrent) {
    const found = (S.history || []).find(h => String(h.id) === String(selectedSessionId));
    if (found) activeSession = found;
  }

  const matches = activeSession.recent || [];
  const wins = Number(activeSession.wins || 0);
  const losses = Number(activeSession.losses || 0);
  const games = Number(activeSession.games || matches.length);
  const deltaNet = Number(activeSession.sessionMmrChange || 0);
  const winrate = games > 0 ? Math.round((100 * wins) / games) + '%' : '–';
  const streak = activeSession.bestStreak != null ? activeSession.bestStreak : activeSession.streak || 0;

  if ($('histCardRecord')) $('histCardRecord').textContent = `${wins}V - ${losses}D`;
  if ($('histCardMmr')) {
    $('histCardMmr').textContent = (deltaNet >= 0 ? '+' : '') + deltaNet.toFixed(1) + ' MMR';
    $('histCardMmr').style.color = deltaNet > 0 ? '#38ef7d' : deltaNet < 0 ? '#ff6b6b' : 'var(--blue)';
  }
  if ($('histCardWinrate')) $('histCardWinrate').textContent = winrate;
  if ($('histCardStreak')) $('histCardStreak').textContent = (streak > 0 ? '+' : '') + streak;

  if ($('chartHeaderTitle')) {
    $('chartHeaderTitle').textContent = isCurrent
      ? "COURBE D'ÉVOLUTION DU MMR (SESSION EN COURS)"
      : `COURBE DE LA SESSION DU ${new Date(activeSession.startedAt).toLocaleDateString('fr-FR')}`;
  }
  if ($('matchesHeaderTitle')) {
    $('matchesHeaderTitle').textContent = isCurrent
      ? "Matchs de la session en cours"
      : `Matchs enregistrés (${new Date(activeSession.startedAt).toLocaleDateString('fr-FR')})`;
  }

  renderPlaylistsBreakdown(activeSession);
  renderHistoryAndChart(activeSession, selectedPlaylistFilter);
}

function renderPlaylistsBreakdown(s) {
  const container = $('sessionPlaylistsBreakdown');
  if (!container) return;

  const matches = s.recent || [];
  const statsMap = new Map();
  for (const m of matches) {
    const pl = m.playlist || 'Match';
    if (!statsMap.has(pl)) {
      statsMap.set(pl, { name: pl, games: 0, wins: 0, losses: 0, netMmr: 0.0 });
    }
    const item = statsMap.get(pl);
    item.games += 1;
    if (m.won) item.wins += 1; else item.losses += 1;
    item.netMmr += Number(m.mmrChange || 0);
  }

  if (s.playlists && typeof s.playlists === 'object') {
    for (const key of Object.keys(s.playlists)) {
      const p = s.playlists[key];
      if (p && p.games > 0 && !statsMap.has(p.name)) {
        statsMap.set(p.name, {
          name: p.name,
          games: Number(p.games || 0),
          wins: Number(p.wins || 0),
          losses: Number(p.losses || 0),
          netMmr: Number(p.sessionMmrChange || 0),
        });
      }
    }
  }

  if (statsMap.size === 0) {
    container.style.display = 'none';
    container.innerHTML = '';
    return;
  }

  container.style.display = 'grid';
  container.innerHTML = '';

  const totalGames = Number(s.games || matches.length);
  const totalWins = Number(s.wins || 0);
  const totalLosses = Number(s.losses || 0);
  const totalMmr = Number(s.sessionMmrChange || 0);
  const allCard = document.createElement('div');
  allCard.className = `hist-playlist-pill ${selectedPlaylistFilter === 'all' ? 'active' : ''}`;
  allCard.innerHTML = `
    <div class="hpp-head">
      <span>Tous les modes</span>
      <span class="hpp-games">${totalGames} match${totalGames > 1 ? 's' : ''}</span>
    </div>
    <div class="hpp-body">
      <span class="hpp-record">${totalWins}V - ${totalLosses}D</span>
      <span class="hpp-mmr ${totalMmr > 0 ? 'pos' : totalMmr < 0 ? 'neg' : 'nul'}">${(totalMmr >= 0 ? '+' : '') + totalMmr.toFixed(1)} MMR</span>
    </div>
  `;
  allCard.addEventListener('click', () => {
    selectedPlaylistFilter = 'all';
    updateHistoryView();
  });
  container.appendChild(allCard);

  for (const [plName, p] of statsMap.entries()) {
    const card = document.createElement('div');
    const isAct = selectedPlaylistFilter === plName;
    card.className = `hist-playlist-pill ${isAct ? 'active' : ''}`;
    const winrate = p.games > 0 ? Math.round((100 * p.wins) / p.games) : 0;
    const deltaStr = (p.netMmr >= 0 ? '+' : '') + p.netMmr.toFixed(1);
    const mmrClass = p.netMmr > 0 ? 'pos' : p.netMmr < 0 ? 'neg' : 'nul';

    card.innerHTML = `
      <div class="hpp-head">
        <span>${plName}</span>
        <span class="hpp-games">${p.games} match${p.games > 1 ? 's' : ''}</span>
      </div>
      <div class="hpp-body">
        <span class="hpp-record">${p.wins}V - ${p.losses}D <small>(${winrate}%)</small></span>
        <span class="hpp-mmr ${mmrClass}">${deltaStr} MMR</span>
      </div>
    `;
    card.addEventListener('click', () => {
      selectedPlaylistFilter = isAct ? 'all' : plName;
      updateHistoryView();
    });
    container.appendChild(card);
  }
}

function renderHistoryAndChart(s, filterPl) {
  const list = $('historyMatchList');
  const svg = $('mmrSvg');
  const summary = $('chartSummary');
  if (!list || !svg) return;

  const allMatches = s.recent || [];
  const matches = (filterPl && filterPl !== 'all')
    ? allMatches.filter(m => (m.playlist || 'Match') === filterPl)
    : allMatches;

  const startMmr = Number(s.startingMmr || (s.currentMmr - (s.sessionMmrChange || 0)) || 855);

  const mmrPoints = [startMmr];
  let cur = startMmr;
  for (const m of matches) {
    cur += Number(m.mmrChange || 0);
    mmrPoints.push(cur);
  }

  const minMmr = Math.min(...mmrPoints);
  const maxMmr = Math.max(...mmrPoints);
  const deltaNet = matches.reduce((acc, m) => acc + Number(m.mmrChange || 0), 0);
  const filterWins = matches.filter(m => m.won).length;
  const filterLosses = matches.filter(m => !m.won).length;

  const filterText = (filterPl && filterPl !== 'all') ? ` en ${filterPl}` : '';
  summary.textContent = `${matches.length} match${matches.length > 1 ? 's' : ''}${filterText} • Net : ${(deltaNet >= 0 ? '+' : '') + deltaNet.toFixed(1)} MMR (${filterWins}V - ${filterLosses}D)`;

  const isCurrent = selectedSessionId === 'current';
  if ($('matchesHeaderTitle')) {
    const filterTag = (filterPl && filterPl !== 'all') ? ` [${filterPl}]` : '';
    $('matchesHeaderTitle').textContent = isCurrent
      ? `Matchs de la session en cours${filterTag}`
      : `Matchs enregistrés${filterTag} (${new Date(s.startedAt).toLocaleDateString('fr-FR')})`;
  }

  const W = 580;
  const H = 130;
  const padX = 24;
  const padY = 20;

  if (mmrPoints.length <= 1) {
    svg.innerHTML = `
      <text x="${W / 2}" y="${H / 2}" fill="rgba(255,255,255,0.3)" font-size="12" text-anchor="middle" font-family="inherit">
        Jouez quelques parties pour afficher la courbe d'évolution du MMR
      </text>
    `;
  } else {
    const rangeY = (maxMmr - minMmr) || 20;
    const stepX = (W - 2 * padX) / (mmrPoints.length - 1);
    const coords = mmrPoints.map((val, idx) => {
      const x = padX + idx * stepX;
      const y = H - padY - ((val - minMmr) / rangeY) * (H - 2 * padY);
      return [x, y, val];
    });

    const ptsStr = coords.map(([x, y]) => `${x.toFixed(1)},${y.toFixed(1)}`).join(' ');
    const polyStr = `${padX},${H - padY} ` + ptsStr + ` ${coords[coords.length - 1][0].toFixed(1)},${H - padY}`;

    let dotsHtml = '';
    for (const [x, y, val] of coords) {
      dotsHtml += `
        <circle cx="${x.toFixed(1)}" cy="${y.toFixed(1)}" r="3.5" fill="#2db0ff" stroke="#ffffff" stroke-width="1.5">
          <title>${val.toFixed(1)} MMR</title>
        </circle>
      `;
    }

    svg.innerHTML = `
      <defs>
        <linearGradient id="chartGrad" x1="0%" y1="0%" x2="0%" y2="100%">
          <stop offset="0%" stop-color="#2db0ff" stop-opacity="0.35"/>
          <stop offset="100%" stop-color="#2db0ff" stop-opacity="0.0"/>
        </linearGradient>
      </defs>
      <line x1="${padX}" y1="${H - padY}" x2="${W - padX}" y2="${H - padY}" stroke="rgba(255,255,255,0.1)" stroke-width="1"/>
      <polygon points="${polyStr}" fill="url(#chartGrad)"/>
      <polyline points="${ptsStr}" fill="none" stroke="#2db0ff" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"/>
      ${dotsHtml}
    `;
  }

  list.innerHTML = '';
  if (matches.length === 0) {
    list.innerHTML = '<p class="hint" style="margin: 0; padding: 10px; text-align: center;">Aucun match joué dans cette session pour l\'instant.</p>';
  } else {
    for (const m of matches.slice().reverse()) {
      const won = !!m.won;
      const delta = Number(m.mmrChange || 0);
      const row = document.createElement('div');
      row.style.display = 'flex';
      row.style.alignItems = 'center';
      row.style.justifyContent = 'space-between';
      row.style.padding = '8px 12px';
      row.style.background = won ? 'rgba(46, 204, 113, 0.08)' : 'rgba(255, 71, 87, 0.08)';
      row.style.border = `1px solid ${won ? 'rgba(46, 204, 113, 0.25)' : 'rgba(255, 71, 87, 0.25)'}`;
      row.style.borderRadius = '6px';

      const left = document.createElement('div');
      left.style.display = 'flex';
      left.style.alignItems = 'center';
      left.style.gap = '10px';

      const badge = document.createElement('span');
      badge.textContent = won ? 'VICTOIRE' : m.abandoned ? 'ABANDON' : 'DÉFAITE';
      badge.style.fontSize = '11px';
      badge.style.fontWeight = '800';
      badge.style.padding = '2px 6px';
      badge.style.borderRadius = '4px';
      badge.style.color = won ? '#a3ffc2' : '#ffad82';
      badge.style.background = won ? 'rgba(46, 204, 113, 0.2)' : 'rgba(255, 71, 87, 0.2)';

      const plName = document.createElement('span');
      plName.textContent = m.playlist || 'Match';
      plName.style.fontWeight = '600';
      plName.style.fontSize = '13px';

      left.append(badge, plName);

      const right = document.createElement('div');
      right.style.display = 'flex';
      right.style.alignItems = 'center';
      right.style.gap = '12px';

      const deltaSpan = document.createElement('span');
      deltaSpan.textContent = (delta >= 0 ? '+' : '') + delta.toFixed(1) + ' MMR';
      deltaSpan.style.fontWeight = '800';
      deltaSpan.style.fontSize = '13px';
      deltaSpan.style.color = delta > 0 ? '#2ecc71' : delta < 0 ? '#ff4757' : 'var(--muted)';

      right.append(deltaSpan);
      row.append(left, right);
      list.appendChild(row);
    }
  }
}

// ---------------------------------------------------------------------------
// Formulaire
// ---------------------------------------------------------------------------

function renderForm() {
  const o = S.settings.overlay;
  const c = S.settings.counting;
  const active = document.activeElement;
  const setValue = (el, value) => { if (el !== active) el.value = value; };

  $('overlayEnabled').checked = !!o.enabled;
  if ($('onlyInGame')) $('onlyInGame').checked = o.onlyInGame !== false;
  setPressed($('theme'), o.theme || 'rocketstats');

  for (const b of $('fields').querySelectorAll('button')) {
    b.setAttribute('aria-pressed', String(o.fields.includes(b.dataset.key)));
  }
  const isCustomPos = o.position === 'custom';
  setPressed($('position'), isCustomPos ? '' : o.position);
  if ($('customPosBtn')) $('customPosBtn').setAttribute('aria-pressed', String(isCustomPos));
  if ($('labelOffsetX')) $('labelOffsetX').textContent = isCustomPos ? 'Position X (px)' : 'Marge X';
  if ($('labelOffsetY')) $('labelOffsetY').textContent = isCustomPos ? 'Position Y (px)' : 'Marge Y';
  setValue($('offsetX'), isCustomPos ? (o.customX != null ? o.customX : o.offsetX) : o.offsetX);
  setValue($('offsetY'), isCustomPos ? (o.customY != null ? o.customY : o.offsetY) : o.offsetY);
  setValue($('scale'), o.scale);
  $('scaleOut').textContent = `${Math.round(o.scale * 100)} %`;
  setValue($('opacity'), o.opacity != null ? o.opacity : 0.92);
  $('opacityOut').textContent = `${Math.round((o.opacity != null ? o.opacity : 0.92) * 100)} %`;

  if ($('sessionGoalMmr')) setValue($('sessionGoalMmr'), o.sessionGoalMmr || '');
  if ($('showGoal')) $('showGoal').checked = !!o.showGoal;

  const goalBox = $('goalPreviewBox');
  if (goalBox) {
    const target = Number(o.sessionGoalMmr || 0);
    const show = !!o.showGoal && target > 0;
    goalBox.style.display = show ? 'block' : 'none';
    if (show) {
      const cur = Number(S?.session?.currentMmr || 0);
      const rem = Math.max(0, target - cur);
      const pct = Math.min(100, Math.max(0, Math.round((cur / target) * 100)));
      if ($('goalPreviewTitle')) $('goalPreviewTitle').textContent = `OBJECTIF : ${target.toFixed(0)} MMR`;
      if ($('goalPreviewDiff')) {
        $('goalPreviewDiff').textContent = rem <= 0 ? 'ATTEINT !' : `${rem.toFixed(1)} MMR restant (${pct}% accompli)`;
        $('goalPreviewDiff').style.color = rem <= 0 ? 'var(--good)' : 'var(--gold)';
      }
      if ($('goalPreviewFill')) $('goalPreviewFill').style.width = `${pct}%`;
    }
  }

  for (const [key, defaultVal] of COLOR_KEYS) {
    const val = o[key] || defaultVal;
    if ($(key)) $(key).value = val;
    if ($(`${key}Hex`)) $(`${key}Hex`).textContent = val.toUpperCase();
    if (key === 'colorGoal') document.documentElement.style.setProperty('--gold', val);
  }

  setPressed($('countMode'), c.mode);
  $('customPlaylists').hidden = c.mode !== 'custom';
  renderPlaylistChecklist();
  if ($('autoSwitchPlaylist')) $('autoSwitchPlaylist').checked = c.autoSwitchPlaylist !== false;
  $('onlineOnly').checked = !!c.onlineOnly;
  $('countLeave').checked = !!c.countLeaveAsLoss;
  $('resetOnStart').checked = !!c.resetOnStart;
  if ($('resetOnExit')) $('resetOnExit').checked = !!c.resetOnExit;

  setPressed($('rate'), S.settings.statsApi.packetSendRate);

  const cPl = S.settings.counting || {};
  setPressed($('focusedPlaylist'), cPl.focusedPlaylist || '2v2');

  const pl = S.settings.player || {};
  if ($('playerPlatformSelect')) $('playerPlatformSelect').value = pl.platform || '';
  if ($('playerPlatform')) setPressed($('playerPlatform'), pl.platform || '');
  $('customName').disabled = false;
  $('customName').placeholder = 'Détecté automatiquement ou saisissez votre pseudo...';
  setValue($('customName'), pl.customName || '');
  if ($('autoSyncMmr')) $('autoSyncMmr').checked = !!pl.autoSyncMmr;

  // Mise à jour de la carte de profil joueur en direct
  if ($('ppRankImg')) $('ppRankImg').innerHTML = getRankImg(S.session.rankTier, 48);
  if ($('ppPseudo')) $('ppPseudo').textContent = pl.customName || 'Joueur Rocket League';
  if ($('ppRankTitle')) $('ppRankTitle').textContent = `${S.session.rankName || 'Unranked'} Div. ${S.session.division || 1}`;
  if ($('ppMmrVal')) $('ppMmrVal').textContent = Number(S.session.currentMmr || 0).toFixed(1);
  if ($('ppPlatBadge')) $('ppPlatBadge').textContent = pl.platform ? (pl.platform === 'epic' ? 'Epic Games' : 'Steam') : 'Détection auto';

  const formProg = getDivisionProgress(Number(S.session.currentMmr || 0), S.session.focusedPlaylist || S?.settings?.counting?.focusedPlaylist);
  if ($('ppDivProgPct')) $('ppDivProgPct').textContent = `${formProg.percent}% de la division`;
  if ($('ppDivProgNeeded')) $('ppDivProgNeeded').textContent = formProg.neededNext > 0 ? `+${formProg.neededNext} MMR pour monter` : 'Division max';
  if ($('ppDivProgFill')) $('ppDivProgFill').style.width = `${formProg.percent}%`;

  $('obsUrl').textContent = S.obsUrl;
  setValue($('serverPort'), S.settings.server.port);
  $('serverError').hidden = !S.serverError;
  $('serverError').textContent = S.serverError || '';
}

function renderPlaylistChecklist() {
  const box = $('customPlaylists');
  const selected = S.settings.counting.playlists;
  const ids = new Set(Object.keys(S.playlists).map(Number));
  for (const pl of Object.values(S.session.playlists)) if (pl.id != null) ids.add(pl.id);
  box.innerHTML = '';
  for (const id of [...ids].sort((a, b) => a - b)) {
    const label = document.createElement('label');
    const input = document.createElement('input');
    input.type = 'checkbox';
    input.checked = selected.includes(id);
    input.addEventListener('change', () => {
      const next = input.checked ? [...selected, id] : selected.filter((x) => x !== id);
      save({ counting: { playlists: next } });
    });
    const text = document.createElement('span');
    text.textContent = S.playlists[id] || `Playlist ${id}`;
    label.append(input, text);
    box.appendChild(label);
  }
}

function renderApi(result) {
  if (result) api = result;
  if (!api) return;

  const state = $('apiState');
  state.classList.toggle('on', api.enabled);
  if (!api.valid) state.textContent = 'Dossier de Rocket League introuvable';
  else if (api.enabled) {
    const rate = Math.max(...api.files.filter((f) => f.exists).map((f) => f.packetSendRate));
    state.textContent = `Activée (${rate} envois par seconde)`;
  } else state.textContent = 'Désactivée';

  const dir = $('installDir');
  if (dir) {
    dir.textContent = api.installDir || 'Aucun dossier sélectionné';
    dir.classList.toggle('missing', !api.valid);
  }

  // Détection automatique BakkesMod (Epic / Steam)
  const installs = api.installs || {};
  const running = api.runningGame;
  const isRunning = !!api.rlRunning;

  const dot = $('gameRunningDot');
  const statusText = $('gameRunningStatus');
  const details = $('autoPlatformDetails');

  if (dot) dot.classList.toggle('running', isRunning);

  if (statusText) {
    if (running && running.platform) {
      const platName = running.platform === 'steam' ? 'Steam' : (running.platform === 'epic' ? 'Epic Games' : 'Processus');
      statusText.textContent = `Rocket League actif via ${platName}`;
      if (details) details.textContent = `Emplacement détecté : ${running.installDir || running.exePath}`;
    } else if (isRunning) {
      statusText.textContent = 'Rocket League en cours d\'exécution';
      if (details) details.textContent = 'L\'overlay est connecté et synchronise les données en direct.';
    } else {
      statusText.textContent = 'En attente du lancement de Rocket League…';
      if (details) details.textContent = 'L\'overlay détectera automatiquement si vous lancez Rocket League depuis Epic Games ou Steam.';
    }
  }

  // Résumé Epic
  const epicPath = installs.epic;
  if ($('summaryPathEpic')) $('summaryPathEpic').textContent = epicPath || 'Non détecté sur cette machine';
  if ($('summaryBadgeEpic')) {
    $('summaryBadgeEpic').textContent = epicPath ? 'Prêt' : 'Non détecté';
    $('summaryBadgeEpic').className = epicPath ? 'badge badge-detected' : 'badge badge-missing';
  }
  $('summaryEpic')?.classList.toggle('active', running?.platform === 'epic');

  // Résumé Steam
  const steamPath = installs.steam;
  if ($('summaryPathSteam')) $('summaryPathSteam').textContent = steamPath || 'Non détecté sur cette machine';
  if ($('summaryBadgeSteam')) {
    $('summaryBadgeSteam').textContent = steamPath ? 'Prêt' : 'Non détecté';
    $('summaryBadgeSteam').className = steamPath ? 'badge badge-detected' : 'badge badge-missing';
  }
  $('summarySteam')?.classList.toggle('active', running?.platform === 'steam');

  // Notice pour dossier personnalisé
  const customNotice = $('customInstallNotice');
  if (customNotice) {
    customNotice.style.display = api.activePlatform === 'custom' ? 'block' : 'none';
  }

  $('enableApi').textContent = api.enabled ? 'Réappliquer le réglage' : 'Activer la Stats API';
  $('apiBanner').hidden = api.enabled;

  const msg = $('apiMsg');
  msg.className = 'message';
  msg.textContent = '';
  if (result && result.error) { msg.classList.add('error'); msg.textContent = result.error; }
  else if (result && result.success) { msg.classList.add('ok'); msg.textContent = result.success; }
  else if (result && result.notFound) { msg.classList.add('error'); msg.textContent = "Rocket League n'a pas été trouvé automatiquement. Utilise « Choisir le dossier… »."; }
  else if (api.rlRunning && !api.enabled) { msg.classList.add('error'); msg.textContent = 'Rocket League est ouvert : ferme-le avant d\'activer la Stats API.'; }
}

// ---------------------------------------------------------------------------
// Événements
// ---------------------------------------------------------------------------

function bind() {
  // Puces des stats affichées réparties par catégories
  const fieldsContainer = $('fields');
  fieldsContainer.innerHTML = '';
  for (const group of FIELD_GROUPS) {
    const catDiv = document.createElement('div');
    catDiv.className = 'field-category';

    const titleDiv = document.createElement('div');
    titleDiv.className = 'field-category-title';
    titleDiv.textContent = group.category;
    catDiv.appendChild(titleDiv);

    const chipsDiv = document.createElement('div');
    chipsDiv.className = 'chips-grid';

    for (const [key, label] of group.items) {
      const b = document.createElement('button');
      b.type = 'button';
      b.dataset.key = key;
      b.textContent = label;
      b.addEventListener('click', () => {
        const current = S.settings.overlay.fields;
        const next = current.includes(key)
          ? current.filter((k) => k !== key)
          : FIELDS.map(([k]) => k).filter((k) => k === key || current.includes(k));
        save({ overlay: { fields: next } });
      });
      chipsDiv.appendChild(b);
    }

    catDiv.appendChild(chipsDiv);
    fieldsContainer.appendChild(catDiv);
  }

  // Raccourcis de sélection pour les statistiques
  $('selectAllFields')?.addEventListener('click', () => {
    save({ overlay: { fields: FIELDS.map(([k]) => k) } });
  });
  $('selectDefaultFields')?.addEventListener('click', () => {
    save({ overlay: { fields: ['rank', 'mmr', 'mmrChange', 'wins', 'losses', 'streak'] } });
  });
  $('clearAllFields')?.addEventListener('click', () => {
    save({ overlay: { fields: [] } });
  });

  $('overlayEnabled').addEventListener('change', (e) => save({ overlay: { enabled: e.target.checked } }));
  $('onlyInGame')?.addEventListener('change', (e) => save({ overlay: { onlyInGame: e.target.checked } }));

  $('previewOverlayBtn')?.addEventListener('click', async () => {
    try {
      const next = !isPreviewing;
      const res = await app.setOverlayPreview(next);
      isPreviewing = !!res;
      updatePreviewButton();
    } catch (e) {
      console.error('Erreur bascule prévisualisation overlay:', e);
    }
  });

  $('theme').addEventListener('click', (e) => {
    const v = e.target.closest('button')?.dataset.value;
    if (v) save({ overlay: { theme: v } });
  });

  $('position').addEventListener('click', (e) => {
    const pos = e.target.closest('button')?.dataset.pos;
    if (pos) save({ overlay: { position: pos } });
  });

  $('customPosBtn')?.addEventListener('click', () => {
    save({ overlay: { position: 'custom' } });
  });

  $('offsetX')?.addEventListener('change', (e) => {
    const val = Number(e.target.value) || 0;
    if (S?.settings?.overlay?.position === 'custom') {
      save({ overlay: { customX: val } });
    } else {
      save({ overlay: { offsetX: val } });
    }
  });

  $('offsetY')?.addEventListener('change', (e) => {
    const val = Number(e.target.value) || 0;
    if (S?.settings?.overlay?.position === 'custom') {
      save({ overlay: { customY: val } });
    } else {
      save({ overlay: { offsetY: val } });
    }
  });
  $('scale').addEventListener('input', (e) => { $('scaleOut').textContent = `${Math.round(e.target.value * 100)} %`; });
  $('scale').addEventListener('change', (e) => save({ overlay: { scale: Number(e.target.value) } }));

  $('opacity').addEventListener('input', (e) => { $('opacityOut').textContent = `${Math.round(e.target.value * 100)} %`; });
  $('opacity').addEventListener('change', (e) => save({ overlay: { opacity: Number(e.target.value) } }));

  // Couleurs personnalisées de l'overlay
  let colorSaveTimer = null;
  for (const [key] of COLOR_KEYS) {
    const el = $(key);
    if (!el) continue;
    el.addEventListener('input', (e) => {
      const hexEl = $(`${key}Hex`);
      if (hexEl) hexEl.textContent = e.target.value.toUpperCase();
      if (key === 'colorGoal') document.documentElement.style.setProperty('--gold', e.target.value);
      clearTimeout(colorSaveTimer);
      colorSaveTimer = setTimeout(() => {
        save({ overlay: { [key]: e.target.value } });
      }, 100);
    });
    el.addEventListener('change', (e) => {
      clearTimeout(colorSaveTimer);
      if (key === 'colorGoal') document.documentElement.style.setProperty('--gold', e.target.value);
      save({ overlay: { [key]: e.target.value } });
    });
  }

  $('resetColorsBtn')?.addEventListener('click', () => {
    const resetObj = {};
    for (const [key, defaultVal] of COLOR_KEYS) {
      resetObj[key] = defaultVal;
      if ($(key)) $(key).value = defaultVal;
      if ($(`${key}Hex`)) $(`${key}Hex`).textContent = defaultVal.toUpperCase();
    }
    document.documentElement.style.setProperty('--gold', '#ffc107');
    save({ overlay: resetObj });
    flashSaved('Couleurs réinitialisées');
  });

  $('focusedPlaylist').addEventListener('click', (e) => {
    const v = e.target.closest('button')?.dataset.value;
    if (v) {
      save({ counting: { focusedPlaylist: v } });
      const val = Number(S?.session?.currentMmr || 855);
      if (!isNaN(val) && val >= 0) {
        const rk = calculateRankFrontend(val, v);
        if ($('ppRankTitle')) $('ppRankTitle').textContent = `${rk.name} Div. ${rk.div}`;
        if ($('ppRankImg')) $('ppRankImg').innerHTML = getRankImg(rk.tier, 48);
      }
    }
  });

  $('checkTrackerBtn').addEventListener('click', () => {
    const pl = S?.settings?.player || {};
    const plat = pl.platform || 'epic';
    const name = pl.customName || $('customName').value.trim();
    if (name) {
      const url = `https://rocketleague.tracker.network/rocket-league/profile/${encodeURIComponent(plat)}/${encodeURIComponent(name)}/overview`;
      app.openBrowserUrl(url);
    } else {
      alert("Veuillez d'abord saisir votre pseudo dans le champ 'Pseudo en jeu'.");
    }
  });

  if ($('autoSyncMmr')) {
    $('autoSyncMmr').addEventListener('change', (e) => save({ player: { autoSyncMmr: e.target.checked } }));
  }

  if ($('syncTrackerBtn')) {
    $('syncTrackerBtn').addEventListener('click', async () => {
      const btn = $('syncTrackerBtn');
      const status = $('syncStatusMsg');
      btn.disabled = true;
      if (status) {
        status.style.display = 'block';
        status.style.color = '#58a6ff';
        status.textContent = 'Connexion à Rocket League Tracker en arrière-plan (analyse en cours, ~3s)...';
      }
      try {
        const res = await app.syncTrackerMmr();
        if (res && res.success) {
          if (status) {
            status.style.color = '#3fb950';
            status.textContent = `Succès : MMR officiel synchronisé : ${res.mmr} (${res.rank} ${res.division})`;
          }
          flashSaved(`MMR synchronisé : ${res.mmr}`);
        } else {
          if (status) {
            status.style.color = '#d29922';
            status.textContent = 'Erreur lors de la synchronisation.';
          }
        }
      } catch (err) {
        if (status) {
          status.style.color = '#f85149';
          status.textContent = `Échec : ${err}`;
        }
      } finally {
        btn.disabled = false;
      }
    });
  }

  $('customName').addEventListener('input', (e) => {
    if ($('ppPseudo')) $('ppPseudo').textContent = e.target.value.trim() || 'Joueur Rocket League';
  });

  $('countMode').addEventListener('click', (e) => {
    const v = e.target.closest('button')?.dataset.value;
    if (v) save({ counting: { mode: v } });
  });
  $('autoSwitchPlaylist')?.addEventListener('change', (e) => save({ counting: { autoSwitchPlaylist: e.target.checked } }));
  $('onlineOnly').addEventListener('change', (e) => save({ counting: { onlineOnly: e.target.checked } }));
  $('countLeave').addEventListener('change', (e) => save({ counting: { countLeaveAsLoss: e.target.checked } }));
  $('resetOnStart').addEventListener('change', (e) => save({ counting: { resetOnStart: e.target.checked } }));
  if ($('resetOnExit')) $('resetOnExit').addEventListener('change', (e) => save({ counting: { resetOnExit: e.target.checked } }));

  $('rate').addEventListener('click', (e) => {
    const v = e.target.closest('button')?.dataset.value;
    if (v) save({ statsApi: { packetSendRate: Number(v) } });
  });
  $('detect').addEventListener('click', async () => renderApi(await app.detectInstall()));
  $('choose').addEventListener('click', async () => renderApi(await app.chooseInstall()));

  $('enableApi').addEventListener('click', async () => {
    const btn = $('enableApi');
    btn.disabled = true;
    try { renderApi(await app.enableStatsApi()); } finally { btn.disabled = false; }
  });
  $('goToApi').addEventListener('click', () => $('sec-api').scrollIntoView({ behavior: 'smooth' }));

  $('detectPlayerBtn')?.addEventListener('click', async () => {
    const btn = $('detectPlayerBtn');
    btn.disabled = true;
    const oldText = btn.textContent;
    btn.textContent = 'Détection…';
    try {
      const res = await app.detectPlayerNow();
      if (res && res.success) {
        flashSaved();
        if ($('customName')) $('customName').value = res.name;
        if ($('ppPseudo')) $('ppPseudo').textContent = res.name;
      } else {
        alert(res?.error || 'Aucun profil trouvé dans les logs du jeu.');
      }
    } finally {
      btn.textContent = oldText;
      btn.disabled = false;
    }
  });

  $('playerPlatformSelect')?.addEventListener('change', (e) => {
    save({ player: { platform: e.target.value } });
  });

  $('playerPlatform')?.addEventListener('click', (e) => {
    const v = e.target.closest('button')?.dataset.value;
    if (v !== undefined) {
      save({ player: { platform: v } });
      if (v) $('customName').focus();
    }
  });

  $('customName').addEventListener('change', (e) => {
    save({ player: { customName: e.target.value.trim() } });
  });

  $('copyUrl').addEventListener('click', async () => { await app.copy(S.obsUrl); flashSaved('Copié'); });
  $('openObs').addEventListener('click', () => app.openObsFolder());
  $('serverPort').addEventListener('change', (e) => {
    const port = Number(e.target.value);
    if (port >= 1024 && port <= 65535) save({ server: { port } });
  });

  // Réinitialisation en deux temps pour éviter les clics accidentels
  let confirmTimer;
  $('reset').addEventListener('click', async () => {
    const btn = $('reset');
    if (!btn.classList.contains('confirm')) {
      btn.classList.add('confirm');
      btn.textContent = 'Cliquer à nouveau pour confirmer';
      confirmTimer = setTimeout(() => {
        btn.classList.remove('confirm');
        btn.textContent = 'Réinitialiser la session';
      }, 3000);
      return;
    }
    clearTimeout(confirmTimer);
    btn.classList.remove('confirm');
    btn.textContent = 'Réinitialiser la session';
    S = await app.reset();
    renderSession();
    flashSaved('Session remise à zéro');
  });

  if ($('sessionGoalMmr')) {
    $('sessionGoalMmr').addEventListener('change', (e) => save({ overlay: { sessionGoalMmr: Number(e.target.value) || 0 } }));
  }
  if ($('showGoal')) {
    $('showGoal').addEventListener('change', (e) => save({ overlay: { showGoal: e.target.checked } }));
  }
  if ($('autostartWindows')) {
    $('autostartWindows').addEventListener('change', async (e) => {
      await app.setAutostart(e.target.checked);
      flashSaved('Démarrage Windows mis à jour');
    });
  }

  if ($('sessionAccountFilter')) {
    $('sessionAccountFilter').addEventListener('change', (e) => {
      selectedAccountFilter = e.target.value;
      updateHistoryView();
    });
  }

  if ($('sessionHistorySelect')) {
    $('sessionHistorySelect').addEventListener('change', (e) => {
      selectedSessionId = e.target.value;
      updateHistoryView();
    });
  }

  if ($('deleteSessionBtn')) {
    $('deleteSessionBtn').addEventListener('click', async () => {
      if (selectedSessionId !== 'current') {
        const idToDelete = selectedSessionId;
        selectedSessionId = 'current';
        S = await app.deleteHistorySession(idToDelete);
        renderSession();
        flashSaved('Session supprimée de l\'historique');
      }
    });
  }

  $('updateDownloadBtn')?.addEventListener('click', () => {
    const url = latestReleaseInfo?.html_url || `https://github.com/${RELEASES_REPO}/releases/latest`;
    app.openBrowserUrl(url);
  });
  $('updateDismissBtn')?.addEventListener('click', () => {
    const banner = $('updateBanner');
    if (banner) banner.hidden = true;
  });
  $('checkUpdateBtn')?.addEventListener('click', () => checkAppUpdates(false));

  $('quit').addEventListener('click', () => app.quit());

  app.onState((state) => {
    S = state;
    renderSession();
    if (formReady) renderForm();
  });

  window.addEventListener('focus', async () => {
    try {
      if (app.getOverlayPreview) {
        isPreviewing = await app.getOverlayPreview();
        updatePreviewButton();
      }
    } catch (e) {}
    renderApi(await app.statsApiStatus());
  });
}

(async function init() {
  bind();
  try {
    if (app.getVersion) {
      const v = await app.getVersion();
      if (v) {
        appVersion = v;
        if ($('appVersion')) $('appVersion').textContent = `v${v}`;
        if ($('updateCurrentHint')) $('updateCurrentHint').textContent = `(actuelle : v${v})`;
      }
    }
  } catch (e) {}

  S = await app.getState();
  if ($('autostartWindows')) {
    try {
      const isAuto = await app.getAutostart();
      $('autostartWindows').checked = !!isAuto;
    } catch(e) {}
  }
  try {
    if (app.getOverlayPreview) {
      isPreviewing = await app.getOverlayPreview();
      updatePreviewButton();
    }
  } catch (e) {}
  renderSession();
  renderForm();
  formReady = true;
  renderApi(await app.statsApiStatus());

  setTimeout(() => checkAppUpdates(true), 3000);
})();
