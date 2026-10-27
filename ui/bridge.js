// Pont entre l'interface et le programme Tauri.
(function () {
  const tauri = window.__TAURI__;
  const invoke = tauri.core.invoke;

  async function copyText(text) {
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      const area = document.createElement('textarea');
      area.value = text;
      document.body.appendChild(area);
      area.select();
      document.execCommand('copy');
      area.remove();
    }
    return true;
  }

  window.app = {
    getState: () => invoke('get_state'),
    save: (patch) => invoke('save_settings', { patch }),
    reset: () => invoke('reset_session_cmd'),
    setMmr: (mmr) => invoke('set_mmr_cmd', { mmr }),
    statsApiStatus: () => invoke('statsapi_status'),
    detectInstall: () => invoke('statsapi_detect'),
    selectInstallPlatform: (platform) => invoke('statsapi_select_platform', { platform }),
    detectPlayerNow: () => invoke('detect_player_now'),
    chooseInstall: () => invoke('statsapi_choose'),
    enableStatsApi: () => invoke('statsapi_enable'),
    openObsFolder: () => invoke('open_obs_folder'),
    openBrowserUrl: (url) => invoke('open_browser_url', { url }),
    syncTrackerMmr: () => invoke('sync_tracker_mmr'),
    deleteHistorySession: (id) => invoke('delete_history_session', { id }),
    getAutostart: () => invoke('get_autostart'),
    setAutostart: (enabled) => invoke('set_autostart', { enabled }),
    getOverlayPreview: () => invoke('get_overlay_preview'),
    setOverlayPreview: (preview) => invoke('set_overlay_preview', { preview }),
    getVersion: () => invoke('app_version'),
    copy: copyText,
    quit: () => invoke('quit_app'),
    onState: (callback) => tauri.event.listen('state', (event) => callback(event.payload)),
    // LobbyTracker
    ltState: () => invoke('lt_state'),
    ltSaveConfig: (config) => invoke('lt_save_config', { config }),
    ltSetPreview: (preview) => invoke('lt_set_preview', { preview }),
    onLtState: (callback) => tauri.event.listen('lt-state', (event) => callback(event.payload)),
    onLtPreviewEnded: (callback) => tauri.event.listen('lt-preview-ended', () => callback()),
  };
})();
