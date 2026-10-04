//! Match rotation: the playlist (`--playlist <file>`) and the cursor that
//! picks the next entry for the current player count.
//!
//! A playlist file is RON:
//!
//! ```ron
//! Playlist(
//!     entries: [
//!         // A built-in rule set by id, a random world, 4 to 32 players.
//!         (rules: "team_brawl", min_players: 4),
//!         // A rule set file (relative to the playlist file) on a fixed world.
//!         (rules: "my_rules.ron", world: Code("LS08-0000-0000-0000-8006-8001-8F9G")),
//!         // An LR rule set code on a random world with slider ranges.
//!         (rules: "LR8V-6097-G000-TMBG", world: RandomSliders(enclosure: (20, 60), density: (30, 70))),
//!     ],
//! )
//! ```
//!
//! `rules` is a built-in rule set id (the file name in `data/rulesets/`
//! without `.ron`), a rule set file or an `LR` code; `world` is `Random` (the
//! default: default sliders), `RandomSliders(enclosure: (lo, hi), density:
//! (lo, hi))` (each range optional) or `Code("LS..")`; `min_players` (default 1) and `max_players` (default
//! 32) bound the player count the entry is played with. The server-wide bans
//! and balance list apply to every entry.
//!
//! [`Rotation::pick`] walks the entries in order and skips those whose player
//! range does not fit the current player count. If none fits it falls back
//! to the first `team_brawl` entry, else to the next entry in order (see
//! [`Rotation::pick`]).

use std::path::{Path, PathBuf};

use ls_game::MatchConfig;
use ls_sim::ruleset::BUILTIN_RULESETS;
use ls_worldgen::{ShareCode, Sliders};
use serde::Deserialize;

use crate::rules::ServerRules;
use crate::server::{random_seed, ServerError};

/// Built-in rule set ids of the default playlist in rotation order (plan
/// §3.1), each with the alternative file names it may land under, and its
/// minimum player count. Ids without an embedded rule set yet are left out,
/// so the default playlist grows as the mode rule sets land.
pub const DEFAULT_ROTATION: [(&[&str], u8); 6] = [
    (&["team_brawl"], 4),
    (&["the_squeeze", "squeeze"], 2),
    (&["paint_the_void"], 1),
    (&["hot_reactor"], 2),
    (&["heist"], 2),
    (&["last_color_standing"], 2),
];

/// Rule set id of the fallback entry when no entry fits the player count.
pub const FALLBACK_RULES: &str = "team_brawl";

/// Largest player range an entry can name.
pub const MAX_ENTRY_PLAYERS: u8 = 32;

/// The world of a playlist entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorldChoice {
    /// Always this world.
    Fixed(ShareCode),
    /// A new random world for every match, sliders drawn from the ranges
    /// (inclusive).
    Random {
        enclosure: (u8, u8),
        density: (u8, u8),
    },
}

impl WorldChoice {
    /// Default sliders, random seed.
    pub fn random() -> Self {
        let d = Sliders::default();
        WorldChoice::Random {
            enclosure: (d.enclosure, d.enclosure),
            density: (d.density, d.density),
        }
    }

    /// The share code of the next match on this entry.
    pub fn share_code(&self) -> ShareCode {
        match self {
            WorldChoice::Fixed(code) => *code,
            WorldChoice::Random { enclosure, density } => {
                let seed = random_seed();
                let pick = |(lo, hi): (u8, u8), salt: u64| {
                    let span = u64::from(hi.saturating_sub(lo)) + 1;
                    lo + (ls_math::rng::mix64(seed ^ salt) % span) as u8
                };
                let sliders = Sliders::new(pick(*enclosure, 0x51ed), pick(*density, 0xde45));
                ShareCode::new(seed, sliders)
            }
        }
    }
}

/// One entry of the rotation.
#[derive(Debug, Clone, PartialEq)]
pub struct PlaylistEntry {
    /// Where the rules came from (built-in id, file or code), for logs.
    pub source: String,
    /// The rule set plus the server-wide bans and balance list.
    pub rules: ServerRules,
    pub world: WorldChoice,
    pub min_players: u8,
    pub max_players: u8,
}

impl PlaylistEntry {
    /// Whether `players` is inside the entry's range.
    pub fn fits(&self, players: usize) -> bool {
        (usize::from(self.min_players)..=usize::from(self.max_players)).contains(&players)
    }

    /// The configuration of the next match on this entry: a world (fixed or
    /// a new random one) and the effective rules (the balance list is
    /// re-read here). Tables come from `template`.
    pub fn match_config(&self, template: &MatchConfig) -> Result<MatchConfig, ServerError> {
        let mut cfg = template.clone();
        cfg.code = self.world.share_code();
        cfg.rules = self.rules.effective(&cfg.weapons, &cfg.gadgets)?;
        Ok(cfg)
    }
}

/// A validated playlist (at least one entry).
#[derive(Debug, Clone, PartialEq)]
pub struct Playlist {
    entries: Vec<PlaylistEntry>,
}

/// RON form of a playlist file.
#[derive(Debug, Deserialize)]
#[serde(rename = "Playlist", deny_unknown_fields)]
struct PlaylistFile {
    entries: Vec<EntryFile>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EntryFile {
    rules: String,
    #[serde(default)]
    world: WorldFile,
    #[serde(default = "one")]
    min_players: u8,
    #[serde(default = "max_players")]
    max_players: u8,
}

fn one() -> u8 {
    1
}

fn max_players() -> u8 {
    MAX_ENTRY_PLAYERS
}

#[derive(Debug, Default, Deserialize)]
enum WorldFile {
    #[default]
    Random,
    RandomSliders {
        #[serde(default = "default_enclosure")]
        enclosure: (u8, u8),
        #[serde(default = "default_density")]
        density: (u8, u8),
    },
    Code(String),
}

fn default_enclosure() -> (u8, u8) {
    let e = Sliders::default().enclosure;
    (e, e)
}

fn default_density() -> (u8, u8) {
    let d = Sliders::default().density;
    (d, d)
}

fn invalid(msg: impl Into<String>) -> ServerError {
    ServerError::Playlist(msg.into())
}

impl Playlist {
    /// A playlist from entries (at least one).
    pub fn new(entries: Vec<PlaylistEntry>) -> Result<Playlist, ServerError> {
        if entries.is_empty() {
            return Err(invalid("the playlist has no entries"));
        }
        for e in &entries {
            if e.min_players == 0 || e.min_players > e.max_players {
                return Err(invalid(format!(
                    "{}: player range {}..={} is empty",
                    e.source, e.min_players, e.max_players
                )));
            }
            if let WorldChoice::Random { enclosure, density } = e.world {
                for (lo, hi) in [enclosure, density] {
                    if lo > hi || hi > ls_worldgen::sliders::SLIDER_MAX {
                        return Err(invalid(format!(
                            "{}: slider range ({lo}, {hi}) is invalid",
                            e.source
                        )));
                    }
                }
            }
        }
        Ok(Playlist { entries })
    }

    /// The one-entry playlist of `--ruleset` / `--code` (and of
    /// [`crate::ServerConfig::new`]): `rules` (or the template's rules
    /// unchanged) on `world`, any player count.
    pub fn single(source: &str, rules: ServerRules, world: WorldChoice) -> Playlist {
        Playlist {
            entries: vec![PlaylistEntry {
                source: source.to_string(),
                rules,
                world,
                min_players: 1,
                max_players: MAX_ENTRY_PLAYERS,
            }],
        }
    }

    /// The default rotation ([`DEFAULT_ROTATION`], built-in rule sets that
    /// exist) on random worlds, or on `world` if given; `bans` and
    /// `balance_list` apply to every entry.
    pub fn default_rotation(
        world: Option<ShareCode>,
        bans: &[String],
        balance_list: Option<&Path>,
    ) -> Result<Playlist, ServerError> {
        let world = world.map_or_else(WorldChoice::random, WorldChoice::Fixed);
        let mut entries = Vec::new();
        for (ids, min) in DEFAULT_ROTATION {
            let Some(id) = ids
                .iter()
                .copied()
                .find(|id| builtin_rule_set(id).is_some())
            else {
                continue;
            };
            entries.push(PlaylistEntry {
                source: id.to_string(),
                rules: ServerRules::new(
                    Some(id),
                    bans.to_vec(),
                    balance_list.map(Path::to_path_buf),
                )?,
                world: world.clone(),
                min_players: min,
                max_players: MAX_ENTRY_PLAYERS,
            });
        }
        Playlist::new(entries)
    }

    /// Loads a playlist file; relative rule set paths are relative to the
    /// file. Every entry's rules are validated now.
    pub fn load(
        path: &Path,
        bans: &[String],
        balance_list: Option<&Path>,
    ) -> Result<Playlist, ServerError> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| invalid(format!("{}: {e}", path.display())))?;
        Playlist::parse(&text, path.parent(), bans, balance_list)
            .map_err(|e| invalid(format!("{}: {e}", path.display())))
    }

    /// Parses playlist RON (see the module docs).
    pub fn parse(
        text: &str,
        base_dir: Option<&Path>,
        bans: &[String],
        balance_list: Option<&Path>,
    ) -> Result<Playlist, ServerError> {
        let file: PlaylistFile = ron::from_str(text).map_err(|e| invalid(e.to_string()))?;
        let mut entries = Vec::new();
        for (i, e) in file.entries.into_iter().enumerate() {
            let arg = resolve_rules_arg(&e.rules, base_dir);
            let rules = ServerRules::new(
                Some(&arg),
                bans.to_vec(),
                balance_list.map(Path::to_path_buf),
            )
            .map_err(|err| invalid(format!("entry {}: {err}", i + 1)))?;
            let world = match e.world {
                WorldFile::Random => WorldChoice::random(),
                WorldFile::RandomSliders { enclosure, density } => {
                    WorldChoice::Random { enclosure, density }
                }
                WorldFile::Code(code) => WorldChoice::Fixed(
                    ShareCode::decode(&code)
                        .map_err(|err| invalid(format!("entry {}: {err}", i + 1)))?,
                ),
            };
            entries.push(PlaylistEntry {
                source: e.rules,
                rules,
                world,
                min_players: e.min_players,
                max_players: e.max_players,
            });
        }
        Playlist::new(entries)
    }

    pub fn entries(&self) -> &[PlaylistEntry] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Never true (a playlist has at least one entry).
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// A playlist `rules` value: a relative file path is taken relative to the
/// playlist's directory when that file exists; ids and codes stay.
fn resolve_rules_arg(arg: &str, base_dir: Option<&Path>) -> String {
    let p = Path::new(arg);
    if let Some(dir) = base_dir {
        if p.is_relative() {
            let joined: PathBuf = dir.join(p);
            if joined.is_file() {
                return joined.to_string_lossy().into_owned();
            }
        }
    }
    arg.to_string()
}

/// The embedded rule set RON of a built-in id (`paint_the_void`, ...).
pub fn builtin_rule_set(id: &str) -> Option<&'static str> {
    BUILTIN_RULESETS
        .iter()
        .find(|(path, _)| {
            Path::new(path)
                .file_stem()
                .is_some_and(|stem| stem.eq_ignore_ascii_case(id))
        })
        .map(|(_, text)| *text)
}

/// The rotation cursor over a playlist.
#[derive(Debug, Clone, PartialEq)]
pub struct Rotation {
    playlist: Playlist,
    /// Index of the entry to try first on the next pick.
    next: usize,
}

impl Rotation {
    pub fn new(playlist: Playlist) -> Self {
        Rotation { playlist, next: 0 }
    }

    pub fn playlist(&self) -> &Playlist {
        &self.playlist
    }

    pub fn entry(&self, index: usize) -> &PlaylistEntry {
        &self.playlist.entries[index]
    }

    /// Picks the next entry for `players` and advances the cursor past it.
    /// With `players == 0` (nobody connected yet) the next entry in order is
    /// taken. Otherwise entries whose range does not contain `players` are
    /// skipped; if none fits, the first [`FALLBACK_RULES`] entry is taken,
    /// else the next entry in order.
    pub fn pick(&mut self, players: usize) -> usize {
        let n = self.playlist.entries.len();
        let order = (0..n).map(|k| (self.next + k) % n);
        let index = if players == 0 {
            self.next % n
        } else if let Some(i) = order
            .clone()
            .find(|&i| self.playlist.entries[i].fits(players))
        {
            i
        } else {
            self.playlist
                .entries
                .iter()
                .position(|e| e.source.eq_ignore_ascii_case(FALLBACK_RULES))
                .unwrap_or(self.next % n)
        };
        self.next = (index + 1) % n;
        index
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ls_sim::RuleSet;

    fn entry(source: &str, min: u8, max: u8) -> PlaylistEntry {
        PlaylistEntry {
            source: source.into(),
            rules: ServerRules::default(),
            world: WorldChoice::random(),
            min_players: min,
            max_players: max,
        }
    }

    fn rotation(entries: Vec<PlaylistEntry>) -> Rotation {
        Rotation::new(Playlist::new(entries).unwrap())
    }

    #[test]
    fn entries_outside_their_player_range_are_skipped() {
        let mut r = rotation(vec![
            entry("team_brawl", 4, 32),
            entry("squeeze", 2, 32),
            entry("paint_the_void", 1, 8),
        ]);
        // 2 players: Team Brawl (needs 4) is skipped.
        assert_eq!(r.pick(2), 1);
        assert_eq!(r.pick(2), 2);
        assert_eq!(r.pick(2), 1, "wraps around, skipping Team Brawl again");
        // Enough players: back in order.
        assert_eq!(r.pick(6), 2);
        assert_eq!(r.pick(6), 0);
        // 10 players: Paint the Void (max 8) is skipped.
        assert_eq!(r.pick(10), 1);
        assert_eq!(r.pick(10), 0);
        // Nobody connected: plain order.
        assert_eq!(r.pick(0), 1);
    }

    #[test]
    fn nothing_fits_falls_back_to_team_brawl_then_to_the_order() {
        let mut r = rotation(vec![entry("squeeze", 2, 32), entry("team_brawl", 4, 32)]);
        assert_eq!(r.pick(1), 1, "Team Brawl is the fallback");
        assert_eq!(r.pick(1), 1);
        let mut r = rotation(vec![entry("squeeze", 2, 32), entry("heist", 2, 32)]);
        assert_eq!(r.pick(1), 0);
        assert_eq!(r.pick(1), 1, "no Team Brawl: plain order");
    }

    #[test]
    fn playlist_files_parse_every_form() {
        let ptv = RuleSet::paint_the_void();
        let code = ptv.to_code().unwrap();
        let dir = std::env::temp_dir().join(format!("ls-server-playlist-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut custom = ptv.clone();
        custom.name = "Custom".into();
        std::fs::write(dir.join("custom.ron"), custom.to_ron()).unwrap();
        let world = ShareCode::new(7, Sliders::new(40, 30)).encode().unwrap();
        let text = format!(
            r#"Playlist(entries: [
                (rules: "paint_the_void"),
                (rules: "custom.ron", world: Code("{world}"), min_players: 2, max_players: 8),
                (rules: "{code}", world: RandomSliders(enclosure: (10, 20), density: (30, 30))),
                (rules: "sandbox_all_weapons", world: Random),
            ])"#
        );
        let path = dir.join("playlist.ron");
        std::fs::write(&path, &text).unwrap();
        let p = Playlist::load(&path, &["railspike".into()], None).unwrap();
        assert_eq!(p.len(), 4);
        let e = p.entries();
        assert_eq!(e[0].rules.base.name, "Paint the Void");
        assert_eq!(e[0].world, WorldChoice::random());
        assert_eq!((e[0].min_players, e[0].max_players), (1, MAX_ENTRY_PLAYERS));
        assert_eq!(e[1].rules.base.name, "Custom");
        assert_eq!((e[1].min_players, e[1].max_players), (2, 8));
        assert_eq!(
            e[1].world,
            WorldChoice::Fixed(ShareCode::decode(&world).unwrap())
        );
        assert_eq!(
            e[2].world,
            WorldChoice::Random {
                enclosure: (10, 20),
                density: (30, 30)
            }
        );
        assert_eq!(e[3].rules.base, RuleSet::sandbox_all_weapons());
        // Bans apply to every entry.
        let mc = e[2]
            .match_config(&MatchConfig::paint_the_void(ShareCode::new(
                1,
                Sliders::default(),
            )))
            .unwrap();
        assert!(mc.rules.availability("railspike").is_banned());
        let s = mc.code.sliders;
        assert!((10..=20).contains(&s.enclosure) && s.density == 30, "{s:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn bad_playlists_are_rejected_with_the_entry() {
        let bad = [
            ("Playlist(entries: [])", "no entries"),
            (
                r#"Playlist(entries: [(rules: "no_such_rules")])"#,
                "entry 1",
            ),
            (
                r#"Playlist(entries: [(rules: "paint_the_void", min_players: 5, max_players: 4)])"#,
                "player range",
            ),
            (
                r#"Playlist(entries: [(rules: "paint_the_void", world: RandomSliders(enclosure: (60, 50)))])"#,
                "slider range",
            ),
            (
                r#"Playlist(entries: [(rules: "paint_the_void", world: Code("LS00"))])"#,
                "entry 1",
            ),
            (r#"Playlist(entries: [(rulez: "x")])"#, "rulez"),
        ];
        for (text, needle) in bad {
            let e = Playlist::parse(text, None, &[], None)
                .unwrap_err()
                .to_string();
            assert!(e.contains(needle), "{text}: {e}");
        }
    }

    #[test]
    fn default_rotation_holds_the_existing_builtin_mode_rule_sets_in_order() {
        let p = Playlist::default_rotation(None, &[], None).unwrap();
        let ids: Vec<&str> = p.entries().iter().map(|e| e.source.as_str()).collect();
        let expected: Vec<&str> = DEFAULT_ROTATION
            .iter()
            .filter_map(|(ids, _)| {
                ids.iter()
                    .copied()
                    .find(|id| builtin_rule_set(id).is_some())
            })
            .collect();
        assert_eq!(ids, expected);
        assert!(ids.contains(&"paint_the_void"));
        let fixed = ShareCode::new(3, Sliders::default());
        let p = Playlist::default_rotation(Some(fixed), &[], None).unwrap();
        assert!(p
            .entries()
            .iter()
            .all(|e| e.world == WorldChoice::Fixed(fixed)));
    }

    #[test]
    fn random_worlds_differ_and_stay_in_range() {
        let w = WorldChoice::Random {
            enclosure: (0, 100),
            density: (5, 6),
        };
        let codes: Vec<ShareCode> = (0..8).map(|_| w.share_code()).collect();
        assert!(codes.windows(2).any(|p| p[0].seed != p[1].seed));
        assert!(codes.iter().all(|c| (5..=6).contains(&c.sliders.density)));
    }
}
