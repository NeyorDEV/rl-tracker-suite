//! Smurf Index et attribution des badges, en respectant les filtres de l'utilisateur.
//!
//! Un badge dont la case est décochée n'est jamais calculé ni exposé : l'overlay ne reçoit
//! aucune trace de la catégorie masquée.

use tracker_core::{
    tier_name, Badge, Config, PlayerId, PlayerProfile, PlayerStatus, PlayerView, PlaylistKind,
    RankRole, RankView, Relation, Team,
};

/// Écart de tiers entre le 1v1 et le reste du lobby pour déclencher « Joueur 1v1 ».
const DUEL_GAP: f32 = 2.0;
const DUEL_MIN_MATCHES: u32 = 10;
const ANCHOR_SAVES: f32 = 1.6;
const ANCHOR_SAVES_TO_GOALS: f32 = 2.5;
const TILT_STREAK: u32 = 3;
/// Montée rapide en classé : assez de matchs récents, très bon taux de victoire et MMR en hausse.
const RAPID_MIN_MATCHES: u32 = 15;
const RAPID_WIN_RATE: f32 = 0.7;
const RAPID_MMR_GAIN: i32 = 150;
/// Points ajoutés à l'indice smurf pour une montée rapide.
const RAPID_INDEX_BONUS: u32 = 20;
/// Une montée rapide est suspecte à partir de ce nombre de tiers sous le seuil smurf.
const RAPID_TIER_MARGIN: u32 = 4;
/// Écart minimal avec le pic, à condition que ce pic soit au moins Champion 2.
const PEAK_GAP: u32 = 2;
const PEAK_MIN_TIER: u32 = tracker_core::TIER_C2;

#[derive(Debug, Clone)]
pub enum EntryState {
    Loading,
    Failed,
    Ready(Box<PlayerProfile>),
}

#[derive(Debug, Clone)]
pub struct LobbyEntry {
    pub id: PlayerId,
    pub name: String,
    pub team: Team,
    pub relation: Relation,
    pub state: EntryState,
}

/// Statistiques du lobby servant de référence « relative ».
#[derive(Debug, Clone, Copy, Default)]
pub struct LobbyContext {
    pub median_team_tier: Option<f32>,
}

impl LobbyContext {
    pub fn from_profiles<'a>(profiles: impl IntoIterator<Item = &'a PlayerProfile>) -> Self {
        let mut tiers: Vec<u32> = Vec::new();
        for p in profiles {
            if let Some(t) = p.best_team_tier() {
                tiers.push(t);
            }
        }
        tiers.sort_unstable();
        let median_team_tier = match tiers.len() {
            0 | 1 => None,
            n if n % 2 == 1 => Some(tiers[n / 2] as f32),
            n => Some((tiers[n / 2 - 1] + tiers[n / 2]) as f32 / 2.0),
        };
        Self { median_team_tier }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Analysis {
    pub smurf_index: u32,
    pub badges: Vec<Badge>,
}

/// Vrai si les matchs classés récents montrent une ascension anormalement rapide.
pub fn rapid_climb(p: &PlayerProfile) -> bool {
    p.recent.is_some_and(|r| {
        r.matches >= RAPID_MIN_MATCHES && r.win_rate() >= RAPID_WIN_RATE && r.mmr_gain >= RAPID_MMR_GAIN
    })
}

/// Indice de 0 à 100 : plus un compte a peu de victoires pour son rang, plus il est élevé ;
/// une montée rapide récente l'augmente.
pub fn smurf_index(p: &PlayerProfile, cfg: &Config) -> u32 {
    let max_wins = cfg.smurf_max_wins.max(1) as f32;
    let wins_factor = (1.0 - p.total_wins as f32 / (2.0 * max_wins)).clamp(0.0, 1.0);
    let floor = cfg.smurf_min_tier.saturating_sub(3) as f32;
    let tier_factor = ((p.best_tier() as f32 - floor) / 6.0).clamp(0.0, 1.0);
    let base = (100.0 * (wins_factor * tier_factor).sqrt()).round() as u32;
    let bonus = if rapid_climb(p) { RAPID_INDEX_BONUS } else { 0 };
    (base + bonus).min(100)
}

pub fn analyze(p: &PlayerProfile, ctx: &LobbyContext, cfg: &Config) -> Analysis {
    let f = &cfg.filters;
    let mut out = Analysis::default();
    let best = p.best_tier();

    if f.smurf_suspect {
        out.smurf_index = smurf_index(p, cfg);
        let low_wins_high_rank = p.total_wins < cfg.smurf_max_wins && best >= cfg.smurf_min_tier;
        // Compte encore récent qui grimpe très vite, même si son rang n'a pas atteint le seuil.
        let climbing = rapid_climb(p)
            && p.total_wins < cfg.smurf_max_wins.saturating_mul(2)
            && best + RAPID_TIER_MARGIN >= cfg.smurf_min_tier;
        if low_wins_high_rank || climbing {
            out.badges.push(Badge::SmurfSuspect);
        }
    }

    if f.one_v_one_specialist {
        let duel = p.playlists.iter().find(|s| s.kind == PlaylistKind::Duel);
        if let Some(d) = duel.filter(|d| d.tier > 0 && d.matches >= DUEL_MIN_MATCHES) {
            let reference = ctx.median_team_tier.or_else(|| p.best_team_tier().map(|t| t as f32));
            if reference.is_some_and(|r| d.tier as f32 >= r + DUEL_GAP) {
                out.badges.push(Badge::OneVOneSpecialist);
            }
        }
    }

    if f.defensive_anchor {
        if let Some(s) = p.saves_per_match {
            let goals_ok = p.goals_per_match.is_none_or(|g| s >= g * ANCHOR_SAVES_TO_GOALS);
            if s >= ANCHOR_SAVES && goals_ok {
                out.badges.push(Badge::DefensiveAnchor);
            }
        }
    }

    if f.on_tilt && p.loss_streak >= TILT_STREAK {
        out.badges.push(Badge::OnTilt);
    }

    if f.peak_disparity {
        if let Some(peak) = p.peak_tier {
            if peak >= PEAK_MIN_TIER && best > 0 && peak >= best + PEAK_GAP {
                out.badges.push(Badge::PeakDisparity);
            }
        }
    }
    out
}

/// Construit les vues de l'overlay : filtre « adversaires uniquement », badges et tri.
/// `playlist` : mode de la partie en cours, dont le rang est celui qui compte.
pub fn build_views(entries: &[LobbyEntry], cfg: &Config, playlist: Option<PlaylistKind>) -> Vec<PlayerView> {
    let ctx = LobbyContext::from_profiles(entries.iter().filter_map(|e| match &e.state {
        EntryState::Ready(p) => Some(p.as_ref()),
        _ => None,
    }));

    let mut views: Vec<PlayerView> = entries
        .iter()
        .filter(|e| !(cfg.filters.only_opponents && e.relation == Relation::Teammate))
        .map(|e| view_of(e, &ctx, cfg, playlist))
        .collect();

    let rank = |r: Relation| match r {
        Relation::Opponent => 0,
        Relation::Unknown => 1,
        Relation::Teammate => 2,
    };
    views.sort_by(|a, b| {
        rank(a.relation).cmp(&rank(b.relation)).then(b.smurf_index.cmp(&a.smurf_index))
    });
    views
}

fn view_of(e: &LobbyEntry, ctx: &LobbyContext, cfg: &Config, playlist: Option<PlaylistKind>) -> PlayerView {
    let mut v = PlayerView {
        key: e.id.key(),
        name: e.name.clone(),
        team: e.team,
        relation: e.relation,
        status: PlayerStatus::Loading,
        smurf_index: 0,
        badges: Vec::new(),
        badge_labels: Vec::new(),
        ranks: Vec::new(),
        best_tier_name: String::new(),
        total_wins: None,
    };
    match &e.state {
        EntryState::Loading => {}
        EntryState::Failed => v.status = PlayerStatus::Failed,
        EntryState::Ready(p) => {
            let a = analyze(p, ctx, cfg);
            v.status = PlayerStatus::Ready;
            v.smurf_index = a.smurf_index;
            v.badge_labels = a
                .badges
                .iter()
                .map(|b| match b {
                    Badge::OnTilt => format!("{} : {}", b.label_fr(), p.loss_streak),
                    _ => b.label_fr().to_string(),
                })
                .collect();
            let ranks = relevant_ranks(p, &a.badges, playlist);
            v.badges = a.badges;
            if !p.display_name.is_empty() {
                v.name = p.display_name.clone();
            }
            v.total_wins = Some(p.total_wins);
            v.ranks = ranks;
            v.best_tier_name = v.ranks.first().map(|r| r.tier_name.clone()).unwrap_or_default();
        }
    }
    v
}

fn rank_of(kind: PlaylistKind, tier: u32, label: &str, role: RankRole) -> RankView {
    RankView { role, playlist: kind, label: label.to_string(), tier, tier_name: tier_name(tier).to_string() }
}

/// Rang de `kind` si le joueur y est classé.
fn tier_in(p: &PlayerProfile, kind: PlaylistKind) -> Option<u32> {
    p.tier_of(kind).filter(|t| *t > 0)
}

/// Les rangs qui valent la peine d'être affichés, et pas les trois modes pour tout le monde :
/// - le rang dans le mode de la partie en cours (à défaut, le meilleur rang) ;
/// - le 1v1 d'un spécialiste du 1v1 ;
/// - le meilleur rang d'un smurf, s'il n'est pas déjà celui du mode joué ;
/// - pour un ancien haut niveau, son rang actuel et son rang maximum.
fn relevant_ranks(p: &PlayerProfile, badges: &[Badge], playlist: Option<PlaylistKind>) -> Vec<RankView> {
    let mode = playlist.filter(|k| k.is_core_ranked());
    let best = [PlaylistKind::Duel, PlaylistKind::Doubles, PlaylistKind::Standard]
        .into_iter()
        .filter_map(|k| tier_in(p, k).map(|t| (k, t)))
        .max_by_key(|(_, t)| *t);

    // Rang principal : celui du mode joué, sinon le meilleur.
    let main = mode.and_then(|k| tier_in(p, k).map(|t| (k, t))).or(best);
    let Some((main_kind, main_tier)) = main else { return Vec::new() };
    let former_high = badges.contains(&Badge::PeakDisparity);
    let main_role = if former_high { RankRole::Current } else { RankRole::Mode };
    let main_label = if former_high { "Actuel" } else { main_kind.short_label() };
    let mut ranks = vec![rank_of(main_kind, main_tier, main_label, main_role)];

    if badges.contains(&Badge::OneVOneSpecialist) && main_kind != PlaylistKind::Duel {
        if let Some(t) = tier_in(p, PlaylistKind::Duel) {
            ranks.push(rank_of(PlaylistKind::Duel, t, "1v1", RankRole::Extra));
        }
    }
    if badges.contains(&Badge::SmurfSuspect) {
        // Jamais deux fois le même mode (ex. un spécialiste 1v1 dont le 1v1 est déjà affiché).
        if let Some((k, t)) = best.filter(|(k, t)| *t > main_tier && !ranks.iter().any(|r| r.playlist == *k)) {
            ranks.push(rank_of(k, t, k.short_label(), RankRole::Extra));
        }
    }
    if former_high {
        if let Some(peak) = p.peak_tier {
            ranks.push(rank_of(main_kind, peak, "Pic", RankRole::Peak));
        }
    }
    ranks
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracker_core::{Platform, PlaylistStat, RecentForm};

    fn pl(kind: PlaylistKind, tier: u32, matches: u32) -> PlaylistStat {
        PlaylistStat { kind, tier, mmr: None, matches }
    }

    fn base(n: &str) -> PlayerProfile {
        PlayerProfile {
            id: PlayerId::new(Platform::Steam, format!("7656119800000{n}")),
            display_name: n.into(),
            total_wins: 1000,
            peak_tier: None,
            playlists: vec![pl(PlaylistKind::Standard, 14, 100)],
            saves_per_match: None,
            goals_per_match: None,
            shots_per_match: None,
            loss_streak: 0,
            recent: None,
            fetched_at: 0,
        }
    }

    fn badges(p: &PlayerProfile, cfg: &Config) -> Vec<Badge> {
        analyze(p, &LobbyContext::default(), cfg).badges
    }

    #[test]
    fn smurf_flagged_at_high_rank_low_wins() {
        let mut p = base("1");
        p.total_wins = 120;
        p.playlists = vec![pl(PlaylistKind::Standard, 18, 80)];
        let cfg = Config::default();
        assert!(badges(&p, &cfg).contains(&Badge::SmurfSuspect));
        assert!(smurf_index(&p, &cfg) > 50);
    }

    #[test]
    fn smurf_not_flagged_for_low_rank_or_veteran() {
        let cfg = Config::default();
        let mut low = base("1");
        low.total_wins = 50;
        low.playlists = vec![pl(PlaylistKind::Standard, 12, 30)];
        assert!(!badges(&low, &cfg).contains(&Badge::SmurfSuspect));
        let mut vet = base("2");
        vet.playlists = vec![pl(PlaylistKind::Standard, 20, 900)];
        assert!(!badges(&vet, &cfg).contains(&Badge::SmurfSuspect));
    }

    #[test]
    fn smurf_thresholds_are_configurable() {
        let mut p = base("1");
        p.total_wins = 300;
        p.playlists = vec![pl(PlaylistKind::Doubles, 17, 50)];
        let mut cfg = Config::default();
        assert!(!badges(&p, &cfg).contains(&Badge::SmurfSuspect));
        cfg.smurf_max_wins = 400;
        assert!(badges(&p, &cfg).contains(&Badge::SmurfSuspect));
    }

    #[test]
    fn disabled_filter_hides_badge_and_index() {
        let mut p = base("1");
        p.total_wins = 100;
        p.playlists = vec![pl(PlaylistKind::Standard, 19, 50)];
        let mut cfg = Config::default();
        cfg.filters.smurf_suspect = false;
        let a = analyze(&p, &LobbyContext::default(), &cfg);
        assert!(a.badges.is_empty());
        assert_eq!(a.smurf_index, 0);
    }

    #[test]
    fn one_v_one_specialist_relative_to_lobby() {
        let mut p = base("1");
        p.playlists = vec![pl(PlaylistKind::Duel, 18, 60), pl(PlaylistKind::Standard, 15, 60)];
        let ctx = LobbyContext { median_team_tier: Some(14.0) };
        let mut cfg = Config::default();
        assert!(analyze(&p, &ctx, &cfg).badges.contains(&Badge::OneVOneSpecialist));
        cfg.filters.one_v_one_specialist = false;
        assert!(!analyze(&p, &ctx, &cfg).badges.contains(&Badge::OneVOneSpecialist));
    }

    #[test]
    fn one_v_one_requires_enough_matches_and_gap() {
        let cfg = Config::default();
        let mut few = base("1");
        few.playlists = vec![pl(PlaylistKind::Duel, 20, 3), pl(PlaylistKind::Standard, 14, 60)];
        assert!(!badges(&few, &cfg).contains(&Badge::OneVOneSpecialist));
        let mut close = base("2");
        close.playlists = vec![pl(PlaylistKind::Duel, 15, 60), pl(PlaylistKind::Standard, 14, 60)];
        assert!(!badges(&close, &cfg).contains(&Badge::OneVOneSpecialist));
    }

    #[test]
    fn defensive_anchor_needs_saves_over_goals() {
        let cfg = Config::default();
        let mut p = base("1");
        p.saves_per_match = Some(2.2);
        p.goals_per_match = Some(0.5);
        assert!(badges(&p, &cfg).contains(&Badge::DefensiveAnchor));
        p.goals_per_match = Some(1.5);
        assert!(!badges(&p, &cfg).contains(&Badge::DefensiveAnchor));
    }

    #[test]
    fn rapid_climb_flags_a_young_account_below_the_rank_threshold() {
        let cfg = Config::default(); // seuil Champion 2 (17), 250 victoires
        let mut p = base("1");
        p.total_wins = 300;
        p.playlists = vec![pl(PlaylistKind::Doubles, 14, 60)];
        assert!(!badges(&p, &cfg).contains(&Badge::SmurfSuspect));
        p.recent = Some(RecentForm { matches: 24, wins: 19, mmr_gain: 210 });
        assert!(badges(&p, &cfg).contains(&Badge::SmurfSuspect));
        // Vétéran : la même série ne suffit pas.
        p.total_wins = 2000;
        assert!(!badges(&p, &cfg).contains(&Badge::SmurfSuspect));
        // Pas assez de matchs, ou MMR en baisse : aucun signal.
        p.total_wins = 300;
        p.recent = Some(RecentForm { matches: 8, wins: 8, mmr_gain: 90 });
        assert!(!rapid_climb(&p));
        p.recent = Some(RecentForm { matches: 24, wins: 19, mmr_gain: -20 });
        assert!(!rapid_climb(&p));
    }

    #[test]
    fn rapid_climb_raises_the_smurf_index() {
        let cfg = Config::default();
        let mut p = base("1");
        p.total_wins = 300;
        p.playlists = vec![pl(PlaylistKind::Doubles, 16, 60)];
        let before = smurf_index(&p, &cfg);
        p.recent = Some(RecentForm { matches: 30, wins: 24, mmr_gain: 300 });
        assert_eq!(smurf_index(&p, &cfg), (before + 20).min(100));
    }

    #[test]
    fn tilt_triggers_at_three_losses() {
        let cfg = Config::default();
        let mut p = base("1");
        p.loss_streak = 2;
        assert!(!badges(&p, &cfg).contains(&Badge::OnTilt));
        p.loss_streak = 3;
        assert!(badges(&p, &cfg).contains(&Badge::OnTilt));
    }

    #[test]
    fn peak_disparity_needs_two_tiers_from_champion_2_peak() {
        let cfg = Config::default();
        let mut p = base("1");
        let mut check = |peak: u32, now: u32| {
            p.peak_tier = Some(peak);
            p.playlists = vec![pl(PlaylistKind::Standard, now, 100)];
            badges(&p, &cfg).contains(&Badge::PeakDisparity)
        };
        assert!(check(20, 16)); // GC2 -> C1
        assert!(check(20, 18)); // GC2 -> C3 : 2 tiers
        assert!(!check(20, 19)); // 1 tier seulement
        assert!(!check(20, 20)); // au pic
        assert!(check(17, 15)); // C2 -> D3
        assert!(!check(17, 16)); // C2 -> C1 : 1 tier
        assert!(!check(16, 13)); // pic sous Champion 2
    }

    fn entry(n: &str, relation: Relation, p: Option<PlayerProfile>) -> LobbyEntry {
        LobbyEntry {
            id: PlayerId::new(Platform::Steam, format!("7656119800000{n}")),
            name: n.into(),
            team: Team::Unknown,
            relation,
            state: p.map_or(EntryState::Loading, |p| EntryState::Ready(Box::new(p))),
        }
    }

    #[test]
    fn only_opponents_hides_teammates() {
        let entries = vec![
            entry("1", Relation::Teammate, Some(base("1"))),
            entry("2", Relation::Opponent, Some(base("2"))),
            entry("3", Relation::Unknown, None),
        ];
        let mut cfg = Config::default();
        assert_eq!(build_views(&entries, &cfg, None).len(), 3);
        cfg.filters.only_opponents = true;
        let v = build_views(&entries, &cfg, None);
        assert_eq!(v.len(), 2);
        assert!(v.iter().all(|x| x.relation != Relation::Teammate));
    }

    #[test]
    fn views_sorted_opponents_first_then_by_smurf_index() {
        let mut smurf = base("2");
        smurf.total_wins = 50;
        smurf.playlists = vec![pl(PlaylistKind::Standard, 19, 40)];
        let entries = vec![
            entry("1", Relation::Teammate, Some(smurf.clone())),
            entry("3", Relation::Opponent, Some(base("3"))),
            entry("2", Relation::Opponent, Some(smurf)),
        ];
        let v = build_views(&entries, &Config::default(), None);
        assert_eq!(v[0].name, "2");
        assert_eq!(v[1].name, "3");
        assert_eq!(v[2].relation, Relation::Teammate);
        assert!(v[0].badges.contains(&Badge::SmurfSuspect));
    }

    #[test]
    fn loading_entries_have_no_badges() {
        let v = build_views(&[entry("1", Relation::Opponent, None)], &Config::default(), None);
        assert_eq!(v[0].status, PlayerStatus::Loading);
        assert!(v[0].badges.is_empty());
    }

    #[test]
    fn lobby_context_median() {
        let a = base("1");
        let mut b = base("2");
        b.playlists = vec![pl(PlaylistKind::Doubles, 16, 10)];
        let mut c = base("3");
        c.playlists = vec![pl(PlaylistKind::Doubles, 18, 10)];
        let ctx = LobbyContext::from_profiles([&a, &b, &c]);
        assert_eq!(ctx.median_team_tier, Some(16.0));
    }

    fn labels(ranks: &[RankView]) -> Vec<String> {
        ranks.iter().map(|r| format!("{} {}", r.label, r.tier_name)).collect()
    }

    fn full(duel: u32, doubles: u32, standard: u32) -> PlayerProfile {
        let mut p = base("9");
        p.playlists = vec![
            pl(PlaylistKind::Duel, duel, 80),
            pl(PlaylistKind::Doubles, doubles, 80),
            pl(PlaylistKind::Standard, standard, 80),
        ];
        p
    }

    #[test]
    fn a_normal_player_shows_only_the_rank_of_the_mode_being_played() {
        let p = full(10, 13, 11);
        let ranks = relevant_ranks(&p, &[], Some(PlaylistKind::Doubles));
        assert_eq!(labels(&ranks), vec!["2v2 Diamond 1"], "ni le 1v1 ni le 3v3");
        // Mode inconnu (partie privée, début de lobby) : on montre au moins le meilleur rang.
        let ranks = relevant_ranks(&p, &[], None);
        assert_eq!(labels(&ranks), vec!["2v2 Diamond 1"]);
        // Non classé dans le mode joué : repli sur le meilleur rang plutôt que rien.
        let mut q = full(0, 0, 14);
        q.playlists.retain(|s| s.tier > 0);
        assert_eq!(labels(&relevant_ranks(&q, &[], Some(PlaylistKind::Doubles))), vec!["3v3 Diamond 2"]);
    }

    #[test]
    fn a_one_v_one_specialist_also_shows_his_duel_rank() {
        let p = full(18, 12, 12);
        let ranks = relevant_ranks(&p, &[Badge::OneVOneSpecialist], Some(PlaylistKind::Doubles));
        assert_eq!(labels(&ranks), vec!["2v2 Platinum 3", "1v1 Champion 3"]);
        // En 1v1, le rang du mode est déjà le duel : pas de doublon.
        let ranks = relevant_ranks(&p, &[Badge::OneVOneSpecialist], Some(PlaylistKind::Duel));
        assert_eq!(labels(&ranks), vec!["1v1 Champion 3"]);
    }

    #[test]
    fn a_smurf_shows_his_best_rank_when_it_is_in_another_mode() {
        let p = full(12, 13, 19);
        let ranks = relevant_ranks(&p, &[Badge::SmurfSuspect], Some(PlaylistKind::Doubles));
        assert_eq!(labels(&ranks), vec!["2v2 Diamond 1", "3v3 Grand Champion 1"]);
        // S'il est déjà fort dans le mode joué : un seul rang.
        let ranks = relevant_ranks(&p, &[Badge::SmurfSuspect], Some(PlaylistKind::Standard));
        assert_eq!(labels(&ranks), vec!["3v3 Grand Champion 1"]);
    }

    #[test]
    fn a_smurf_who_is_also_a_one_v_one_specialist_shows_each_rank_once() {
        let p = full(19, 12, 12);
        let badges = [Badge::SmurfSuspect, Badge::OneVOneSpecialist];
        let ranks = relevant_ranks(&p, &badges, Some(PlaylistKind::Doubles));
        assert_eq!(labels(&ranks), vec!["2v2 Platinum 3", "1v1 Grand Champion 1"]);
    }

    #[test]
    fn a_former_high_player_shows_current_and_peak_ranks() {
        let mut p = full(10, 14, 14);
        p.peak_tier = Some(20);
        let ranks = relevant_ranks(&p, &[Badge::PeakDisparity], Some(PlaylistKind::Doubles));
        assert_eq!(labels(&ranks), vec!["Actuel Diamond 2", "Pic Grand Champion 2"]);
        assert_eq!((ranks[0].role, ranks[1].role), (RankRole::Current, RankRole::Peak));
    }

    #[test]
    fn views_carry_only_the_relevant_ranks() {
        let entries = vec![entry("1", Relation::Opponent, Some(full(10, 13, 11)))];
        let v = build_views(&entries, &Config::default(), Some(PlaylistKind::Doubles));
        assert_eq!(v[0].ranks.len(), 1);
        assert_eq!(v[0].best_tier_name, "Diamond 1");
    }
}
