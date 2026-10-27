/* global app */
// Onglet LobbyTracker : analyse du lobby (rangs, smurfs, badges) et réglages de son overlay.
// Il réutilise les composants visuels de RL Tracker Suite (cartes de thème, grille d'emplacement,
// curseurs, couleurs) et sa Stats API, qui se règle dans l'onglet « Session Stats ».

(function () {
  const $ = (id) => document.getElementById(id);
  const esc = (s) => String(s).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));

  const TIERS = ['Non classé', 'Bronze 1', 'Bronze 2', 'Bronze 3', 'Argent 1', 'Argent 2', 'Argent 3', 'Or 1', 'Or 2', 'Or 3',
    'Platine 1', 'Platine 2', 'Platine 3', 'Diamant 1', 'Diamant 2', 'Diamant 3', 'Champion 1', 'Champion 2', 'Champion 3',
    'Grand Champion 1', 'Grand Champion 2', 'Grand Champion 3', 'Supersonic Legend'];

  const FILTERS = [
    ['smurf_suspect', 'Potentiel smurf', 'Compte à peu de victoires à haut rang (Champion 2+), ou récent et en montée rapide.'],
    ['one_v_one_specialist', 'Joueur 1v1', 'Rang 1v1 anormalement élevé par rapport au lobby.'],
    ['defensive_anchor', 'Gardien', 'Beaucoup d\'arrêts, peu de buts.'],
    ['on_tilt', 'Lose streak', 'Affiche le nombre de défaites consécutives, à partir de 3.'],
    ['peak_disparity', 'Ancien haut rang', 'A atteint Champion 2 ou plus, et son rang actuel est au moins 2 tiers en dessous.'],
    ['only_opponents', 'Afficher uniquement les adversaires', 'Masque le profilage des coéquipiers.'],
  ];

  const LAYOUTS = [
    ['card_horizontal', 'Carte horizontale', 'Cartes côte à côte avec bandeau translucide'],
    ['column', 'Colonne', 'Lignes compactes avec séparateurs fins'],
    ['card_vertical', 'Carte verticale', 'Cartes empilées, détail complet par joueur'],
    ['row', 'Ligne', 'Bande horizontale discrète, une ligne par lobby'],
  ];

  const POSITIONS = ['top-left', 'top-center', 'top-right', 'bottom-left', 'bottom-center', 'bottom-right'];

  const COLORS = [
    ['background', 'Fond', '#0c0e14'], ['text', 'Texte', '#ffffff'], ['muted', 'Texte secondaire', '#9aa4b8'],
    ['opponent', 'Adversaire', '#ff7a2b'], ['teammate', 'Coéquipier', '#3b9dff'], ['unknown', 'Équipe inconnue', '#888888'],
    ['badge_smurf', 'Badge potentiel smurf', '#e03a3a'], ['badge_one_v_one', 'Badge joueur 1v1', '#7b2cff'],
    ['badge_anchor', 'Badge gardien', '#7b2cff'], ['badge_tilt', 'Badge lose streak', '#6b7280'],
    ['badge_peak', 'Badge ancien haut rang', '#b7791f'],
  ];

  let lt = null;          // configuration LobbyTracker
  let snap = null;        // dernier lobby analysé
  let me = null;
  let previewing = false;
  let saveTimer = null;

  // ---------------------------------------------------------------------------
  // Onglets
  // ---------------------------------------------------------------------------

  function selectTab(name) {
    document.querySelectorAll('.tab').forEach((t) => t.setAttribute('aria-selected', String(t.dataset.tab === name)));
    $('tab-session').hidden = name !== 'session';
    $('tab-lt').hidden = name !== 'lt';
    try { localStorage.setItem('rlss-tab', name); } catch { /* stockage indisponible */ }
  }
  document.querySelectorAll('.tab').forEach((t) => t.addEventListener('click', () => selectTab(t.dataset.tab)));
  // « L'activer » (bannière Stats API) mène à la section de l'onglet Session Stats.
  $('goToApi')?.addEventListener('click', () => selectTab('session'), true);

  // ---------------------------------------------------------------------------
  // Structure de l'onglet
  // ---------------------------------------------------------------------------

  const toggle = (id, label, hint) =>
    `<label class="toggle"><input type="checkbox" id="${id}"><span>${label}${hint ? `<small style="display:block;color:var(--muted)">${hint}</small>` : ''}</span></label>`;

  function build() {
    $('tab-lt').innerHTML = `
      <h2>LobbyTracker</h2>
      <p class="hint">Rang réel, smurfs, joueurs en lose streak : sachez à qui vous avez affaire pendant le décompte. Les joueurs viennent de la Stats API (réglage dans l'onglet Session Stats).</p>

      <div class="settings-card" style="margin-top:16px">
        <div class="card-header"><div class="card-title">Lobby en direct</div><div class="card-badge" id="ltMe">Compte : en attente du jeu</div></div>
        <div class="card-body"><div class="lt-players" id="ltPlayers"></div></div>
      </div>

      <div class="settings-card">
        <div class="card-header"><div class="card-title">Badges et alertes</div></div>
        <div class="card-body" id="ltFilters">${FILTERS.map(([k, label, hint]) => toggle('ltf_' + k, label, hint)).join('')}</div>
      </div>

      <div class="settings-card">
        <div class="card-header"><div class="card-title">Overlay du lobby</div><div class="card-badge">Raccourci : <kbd>Ctrl</kbd> + <kbd>Maj</kbd> + <kbd>L</kbd></div></div>
        <div class="card-body">
          ${toggle('ltEnabled', 'Afficher l\'overlay du lobby par-dessus Rocket League', 'Il apparaît quand les joueurs du lobby arrivent (Stats API), au même titre que l\'overlay de session.')}
          <div class="preview-banner" style="margin-top:16px;padding:12px 14px;background:rgba(54,169,255,.08);border:1px solid rgba(54,169,255,.25);border-radius:var(--radius);display:flex;align-items:center;justify-content:space-between;gap:14px">
            <div style="font-size:13px;line-height:1.4"><strong style="display:block;font-weight:600">Prévisualisation en direct</strong>
              <span class="hint" style="margin:0;font-size:12px">Affiche un lobby fictif 30 secondes ; vous pouvez le déplacer à la souris.</span></div>
            <button type="button" class="secondary" id="ltPreviewBtn" style="white-space:nowrap;flex-shrink:0;font-weight:600;padding:7px 14px">Prévisualiser l'overlay</button>
          </div>
          <div style="margin-top:18px"><div class="group-label">Disposition</div>
            <div class="theme-grid" id="ltLayout">${LAYOUTS.map(([v, n, d]) =>
              `<button type="button" class="theme-card" data-value="${v}"><span class="theme-name">${n}</span><span class="theme-desc">${d}</span></button>`).join('')}</div></div>
          <div style="margin-top:18px"><div class="group-label">Masquage automatique</div>
            <label class="lt-radio"><input type="radio" name="ltAutoHide" value="off"><span>Ne pas masquer<small>L'overlay reste affiché tant que le lobby est rempli.</small></span></label>
            <label class="lt-radio"><input type="radio" name="ltAutoHide" value="kickoff"><span>À la fin du décompte 3-2-1<small>Il disparaît au coup d'envoi (Stats API activée).</small></span></label>
            <label class="lt-radio"><input type="radio" name="ltAutoHide" value="timer"><span>Après un délai<small>Il disparaît après la durée choisie.</small></span></label>
            <div class="lt-field-row"><label>Durée d'affichage<select id="ltAutoHideSeconds">${[10, 15, 20, 30, 45, 60, 90].map((v) => `<option value="${v}">${v} secondes</option>`).join('')}</select></label></div>
          </div>
        </div>
      </div>

      <div class="settings-card">
        <div class="card-header"><div class="card-title">Position & Dimensions</div></div>
        <div class="card-body"><div class="grid-two-col">
          <div class="sub-col">
            <div class="group-label">Emplacement sur l'écran</div>
            <div class="screen" id="ltPosition">${POSITIONS.map((p) => `<button type="button" data-pos="${p}" aria-label="${p}"></button>`).join('')}</div>
            <div style="margin-top:8px"><button type="button" class="custom-pos-btn" id="ltCustomPos"><span>Position libre (glisser-déposer)</span></button></div>
            <div class="margin-inputs">
              <label class="margin-field"><span id="ltLabelX">Marge X</span><div class="input-unit"><input type="number" id="ltOffsetX" step="4"><small>px</small></div></label>
              <label class="margin-field"><span id="ltLabelY">Marge Y</span><div class="input-unit"><input type="number" id="ltOffsetY" step="4"><small>px</small></div></label>
            </div>
          </div>
          <div class="sub-col">
            <div class="slider-group"><div class="slider-row"><span class="slider-label">Taille (Échelle)</span><output id="ltScaleOut" class="slider-value">100 %</output></div>
              <input type="range" id="ltScale" min="50" max="200" step="5"></div>
            <div class="slider-group" style="margin-top:14px"><div class="slider-row"><span class="slider-label">Opacité du fond</span><output id="ltOpacityOut" class="slider-value">82 %</output></div>
              <input type="range" id="ltOpacity" min="10" max="100" step="5"></div>
          </div>
        </div></div>
      </div>

      <div class="settings-card">
        <div class="card-header"><div class="card-title">Couleurs personnalisées</div>
          <div class="card-actions"><button type="button" class="btn-subtle" id="ltResetColors">Réinitialiser les couleurs</button></div></div>
        <div class="card-body"><div class="color-grid">${COLORS.map(([k, label, d]) =>
          `<div class="color-item"><label for="ltc_${k}">${label}</label><div class="color-input-wrap"><input type="color" id="ltc_${k}" value="${d}"><span class="color-hex" id="ltc_${k}Hex">${d}</span></div></div>`).join('')}</div></div>
      </div>

      <div class="settings-card">
        <div class="card-header"><div class="card-title">Sources et seuils</div></div>
        <div class="card-body">
          ${toggle('ltWebview', 'Scraper WebView2 invisible (contourne Cloudflare)', 'Récupère les profils sur Tracker Network, comme la synchronisation du MMR.')}
          <div class="lt-field-row">
            <label>Jeton API Tracker Network (optionnel, repli)<input type="password" id="ltTrnKey" placeholder="optionnel"></label>
            <label>Durée du cache (heures)<input type="number" id="ltCacheTtl" min="1"></label>
          </div>
          <div class="lt-field-row">
            <label>Smurf : victoires maximum<input type="number" id="ltSmurfWins" min="1"></label>
            <label>Smurf : rang minimum pour l'alerte<select id="ltSmurfTier">${TIERS.map((n, i) => (i ? `<option value="${i}">${n}</option>` : '')).join('')}</select></label>
          </div>
        </div>
      </div>

      <div class="settings-card">
        <div class="card-header"><div class="card-title">Source OBS</div></div>
        <div class="card-body">
          <div class="lt-obs-url"><code id="ltObsUrl"></code><button type="button" class="btn-subtle" id="ltObsCopy">Copier</button></div>
          <p class="hint">Source navigateur OBS : mêmes disposition, couleurs et échelle que l'overlay en jeu. Le port est celui de la section OBS de l'onglet Session Stats.</p>
        </div>
      </div>

      <p class="message" id="ltMsg" role="status"></p>`;
  }

  // ---------------------------------------------------------------------------
  // État <-> formulaire
  // ---------------------------------------------------------------------------

  function setPressed(container, attr, value) {
    container.querySelectorAll('button').forEach((b) => b.setAttribute('aria-pressed', String(b.dataset[attr] === value)));
  }

  function fill() {
    if (!lt) return;
    for (const [k] of FILTERS) $('ltf_' + k).checked = !!lt.filters[k];
    $('ltEnabled').checked = !!lt.overlay_enabled;
    setPressed($('ltLayout'), 'value', lt.overlay_layout);
    const mode = document.querySelector(`input[name="ltAutoHide"][value="${lt.auto_hide_mode || 'off'}"]`);
    if (mode) mode.checked = true;
    $('ltAutoHideSeconds').value = lt.auto_hide_seconds;
    $('ltAutoHideSeconds').disabled = lt.auto_hide_mode !== 'timer';

    const custom = lt.overlay_position === 'custom';
    setPressed($('ltPosition'), 'pos', lt.overlay_position);
    $('ltCustomPos').setAttribute('aria-pressed', String(custom));
    $('ltLabelX').textContent = custom ? 'Position X' : 'Marge X';
    $('ltLabelY').textContent = custom ? 'Position Y' : 'Marge Y';
    $('ltOffsetX').value = custom ? lt.overlay_custom_x ?? lt.overlay_offset_x : lt.overlay_offset_x;
    $('ltOffsetY').value = custom ? lt.overlay_custom_y ?? lt.overlay_offset_y : lt.overlay_offset_y;
    $('ltScale').value = lt.overlay_scale;
    $('ltScaleOut').textContent = `${lt.overlay_scale} %`;
    $('ltOpacity').value = lt.overlay_opacity;
    $('ltOpacityOut').textContent = `${lt.overlay_opacity} %`;

    for (const [k, , d] of COLORS) {
      const v = lt.overlay_colors[k] || d;
      $('ltc_' + k).value = v;
      $('ltc_' + k + 'Hex').textContent = v.toUpperCase();
    }
    $('ltWebview').checked = !!lt.use_webview_scraper;
    $('ltTrnKey').value = lt.trn_api_key || '';
    $('ltCacheTtl').value = lt.cache_ttl_hours;
    $('ltSmurfWins').value = lt.smurf_max_wins;
    $('ltSmurfTier').value = lt.smurf_min_tier;
  }

  // Enregistre sans attendre l'aller-retour : l'overlay suit en direct, le formulaire n'est pas réécrit.
  function save(mutate) {
    mutate(lt);
    fill();
    clearTimeout(saveTimer);
    saveTimer = setTimeout(async () => {
      try { lt = await app.ltSaveConfig(lt); } catch (e) { $('ltMsg').textContent = String(e); }
      saveTimer = null;
    }, 150);
  }

  function renderPlayers() {
    $('ltMe').textContent = me ? `Compte : ${me}` : 'Compte : en attente du jeu';
    const players = snap?.players || [];
    $('ltPlayers').innerHTML = players.length
      ? players.map((p) => {
          const right = p.status === 'ready'
            ? `${esc(p.best_tier_name)}${p.badge_labels.map((l) => `<span class="lt-tag">${esc(l)}</span>`).join('')}`
            : p.status === 'loading' ? 'chargement…' : 'indisponible';
          return `<div class="lt-player ${p.relation}"><span>${esc(p.name)} <small>${p.relation}</small></span><span>${right}</span></div>`;
        }).join('')
      : '<span class="hint">En attente d\'un lobby (il se remplit pendant le décompte d\'un match).</span>';
  }

  function updateObsUrl(state) {
    $('ltObsUrl').textContent = state?.lobbyObsUrl || '';
  }

  // ---------------------------------------------------------------------------
  // Évènements
  // ---------------------------------------------------------------------------

  function bind() {
    for (const [k] of FILTERS) $('ltf_' + k).addEventListener('change', (e) => save((c) => { c.filters[k] = e.target.checked; }));
    $('ltEnabled').addEventListener('change', (e) => save((c) => { c.overlay_enabled = e.target.checked; }));

    $('ltLayout').addEventListener('click', (e) => {
      const v = e.target.closest('button')?.dataset.value;
      if (v) save((c) => { c.overlay_layout = v; });
    });
    document.querySelectorAll('input[name="ltAutoHide"]').forEach((r) =>
      r.addEventListener('change', () => save((c) => { c.auto_hide_mode = r.value; })));
    $('ltAutoHideSeconds').addEventListener('change', (e) => save((c) => { c.auto_hide_seconds = Number(e.target.value); }));

    $('ltPosition').addEventListener('click', (e) => {
      const pos = e.target.closest('button')?.dataset.pos;
      if (pos) save((c) => { c.overlay_position = pos; });
    });
    $('ltCustomPos').addEventListener('click', () => save((c) => { c.overlay_position = 'custom'; }));
    const offset = (axis) => (e) => {
      const v = Number(e.target.value) || 0;
      save((c) => { c[c.overlay_position === 'custom' ? `overlay_custom_${axis}` : `overlay_offset_${axis}`] = v; });
    };
    $('ltOffsetX').addEventListener('change', offset('x'));
    $('ltOffsetY').addEventListener('change', offset('y'));
    $('ltScale').addEventListener('input', (e) => { $('ltScaleOut').textContent = `${e.target.value} %`; });
    $('ltScale').addEventListener('change', (e) => save((c) => { c.overlay_scale = Number(e.target.value); }));
    $('ltOpacity').addEventListener('input', (e) => { $('ltOpacityOut').textContent = `${e.target.value} %`; });
    $('ltOpacity').addEventListener('change', (e) => save((c) => { c.overlay_opacity = Number(e.target.value); }));

    for (const [k] of COLORS) {
      $('ltc_' + k).addEventListener('input', (e) => save((c) => { c.overlay_colors[k] = e.target.value; }));
    }
    $('ltResetColors').addEventListener('click', () => save((c) => { COLORS.forEach(([k, , d]) => { c.overlay_colors[k] = d; }); }));

    $('ltWebview').addEventListener('change', (e) => save((c) => { c.use_webview_scraper = e.target.checked; }));
    $('ltTrnKey').addEventListener('change', (e) => save((c) => { c.trn_api_key = e.target.value.trim() || null; }));
    $('ltCacheTtl').addEventListener('change', (e) => save((c) => { c.cache_ttl_hours = Math.max(1, Number(e.target.value) || 24); }));
    $('ltSmurfWins').addEventListener('change', (e) => save((c) => { c.smurf_max_wins = Math.max(1, Number(e.target.value) || 250); }));
    $('ltSmurfTier').addEventListener('change', (e) => save((c) => { c.smurf_min_tier = Number(e.target.value); }));

    $('ltObsCopy').addEventListener('click', () => app.copy($('ltObsUrl').textContent));

    const setPreviewUi = (on) => {
      previewing = on;
      $('ltPreviewBtn').textContent = on ? 'Arrêter la prévisualisation' : 'Prévisualiser l\'overlay';
    };
    $('ltPreviewBtn').addEventListener('click', async () => setPreviewUi(await app.ltSetPreview(!previewing)));
    app.onLtPreviewEnded(() => setPreviewUi(false));
  }

  // ---------------------------------------------------------------------------
  // Démarrage
  // ---------------------------------------------------------------------------

  build();
  bind();

  const apply = (state) => {
    snap = state.snapshot;
    me = snap.me;
    // Un enregistrement en cours est la source de vérité : on ne réécrit pas le formulaire dessus.
    if (saveTimer === null) { lt = state.display; fill(); }
    renderPlayers();
  };
  app.ltState().then(apply);
  app.onLtState(apply);
  app.getState().then(updateObsUrl);
  app.onState(updateObsUrl);

  try { selectTab(localStorage.getItem('rlss-tab') === 'lt' ? 'lt' : 'session'); } catch { selectTab('session'); }
})();
