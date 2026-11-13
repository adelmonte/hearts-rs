//! Tiny INI/.desktop reader and the persistent settings.

use std::collections::BTreeMap;
use crate::game::Rules;
use std::path::{Path, PathBuf};

pub type Ini = BTreeMap<String, BTreeMap<String, String>>;

pub fn parse_ini(text: &str) -> Ini {
    let mut ini = Ini::new();
    let mut group = String::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            group = line[1..line.len() - 1].to_string();
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            ini.entry(group.clone())
                .or_default()
                .insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    ini
}

pub fn read_ini(path: &Path) -> Option<Ini> {
    std::fs::read_to_string(path).ok().map(|t| parse_ini(&t))
}

pub fn get<'a>(ini: &'a Ini, group: &str, key: &str) -> Option<&'a str> {
    ini.get(group)?.get(key).map(String::as_str)
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
}

fn env_dir(var: &str, fallback: &str) -> PathBuf {
    match std::env::var_os(var) {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => home().join(fallback),
    }
}

pub fn config_dir() -> PathBuf {
    env_dir("XDG_CONFIG_HOME", ".config")
}

/// XDG data directories, user dir first (that's where KNewStuff installs
/// downloaded decks and themes).
pub fn data_dirs() -> Vec<PathBuf> {
    let mut v = vec![env_dir("XDG_DATA_HOME", ".local/share")];
    let sys = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|s| !s.is_empty());
    for d in sys
        .as_deref()
        .unwrap_or("/usr/local/share:/usr/share")
        .split(':')
    {
        if !d.is_empty() {
            let p = PathBuf::from(d);
            if !v.contains(&p) {
                v.push(p);
            }
        }
    }
    v
}

#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub played: u32,
    pub won: u32,
    pub total_score: i64,
    pub best: Option<i32>,
    pub moons: u32,
}

#[derive(Clone, Debug)]
pub struct Settings {
    pub deck: String,
    pub theme: String,
    pub card_width: i32,
    pub status_bar: bool,
    pub difficulty: u8,
    /// 0 fast, 1 normal, 2 slow.
    pub speed: u8,
    pub rules: Rules,
    pub stats: Stats,
    pub win_w: i32,
    pub win_h: i32,
}

const GENERAL: &str = "General";

impl Settings {
    fn path() -> PathBuf {
        config_dir().join("hearts-rsrc")
    }

    /// Our own rc file; on first run, seed it from qt-hearts' settings so
    /// the deck, difficulty, rules and statistics carry over.
    pub fn load() -> Settings {
        let mut s = Settings {
            deck: crate::assets::BUILTIN.into(),
            theme: "greenblaze".into(),
            card_width: 0,
            status_bar: true,
            difficulty: 1,
            speed: 1,
            rules: Rules::default(),
            stats: Stats::default(),
            win_w: 0,
            win_h: 0,
        };
        if let Some(ini) = read_ini(&Self::path()) {
            s.read(&ini);
        } else if let Some(ini) = read_ini(&config_dir().join("Hearts/Hearts.conf")) {
            s.read_qt(&ini);
        }
        s
    }

    fn read(&mut self, ini: &Ini) {
        let b = |g: &str, k: &str, d: bool| get(ini, g, k).map_or(d, |v| v == "true" || v == "1");
        let i = |g: &str, k: &str| get(ini, g, k).and_then(|v| v.parse::<i64>().ok());
        if let Some(v) = get(ini, GENERAL, "Deck") {
            self.deck = v.to_string();
        }
        if let Some(v) = get(ini, GENERAL, "Background") {
            self.theme = v.to_string();
        }
        self.card_width = i(GENERAL, "CardWidth").unwrap_or(0).clamp(0, 2000) as i32;
        self.status_bar = b(GENERAL, "StatusBar", true);
        self.difficulty = i(GENERAL, "Difficulty").unwrap_or(1).clamp(0, 2) as u8;
        self.speed = i(GENERAL, "Speed").unwrap_or(1).clamp(0, 2) as u8;
        self.read_rules(ini, "Rules", "EndScore", "ExactReset", "QueenBreaksHearts", "MoonProtection", "FullPolish");
        let st = "Statistics";
        self.stats = Stats {
            played: i(st, "Played").unwrap_or(0).max(0) as u32,
            won: i(st, "Won").unwrap_or(0).max(0) as u32,
            total_score: i(st, "TotalScore").unwrap_or(0),
            best: i(st, "Best").map(|v| v as i32),
            moons: i(st, "Moons").unwrap_or(0).max(0) as u32,
        };
        self.win_w = i("Window", "Width").unwrap_or(0) as i32;
        self.win_h = i("Window", "Height").unwrap_or(0) as i32;
    }

    #[allow(clippy::too_many_arguments)]
    fn read_rules(&mut self, ini: &Ini, g: &str, end: &str, exact: &str, queen: &str, moon: &str, polish: &str) {
        let b = |k: &str, d: bool| get(ini, g, k).map_or(d, |v| v == "true" || v == "1");
        let end_score = get(ini, g, end)
            .and_then(|v| v.parse::<i32>().ok())
            .filter(|v| END_SCORES.contains(v))
            .unwrap_or(100);
        self.rules = Rules {
            end_score,
            exact_reset: b(exact, false),
            queen_breaks_hearts: b(queen, true),
            moon_protection: b(moon, false),
            full_polish: b(polish, false),
        };
    }

    /// qt-hearts' QSettings file (~/.config/Hearts/Hearts.conf).
    fn read_qt(&mut self, ini: &Ini) {
        let i = |g: &str, k: &str| get(ini, g, k).and_then(|v| v.parse::<i64>().ok());
        if let Some(t) = get(ini, GENERAL, "theme").filter(|t| !t.is_empty()) {
            // Either the deck directory or the SVG inside it.
            let p = Path::new(t);
            let dir = if p.extension().is_some_and(|e| e == "svg" || e == "svgz") {
                p.parent().unwrap_or(p)
            } else {
                p
            };
            if let Some(n) = dir.file_name() {
                self.deck = n.to_string_lossy().into_owned();
            }
        }
        self.difficulty = i(GENERAL, "aiDifficulty").unwrap_or(1).clamp(0, 2) as u8;
        self.speed = i(GENERAL, "gameSpeed").unwrap_or(1).clamp(0, 2) as u8;
        self.read_rules(ini, "rules", "endScore", "exactResetTo50", "queenBreaksHearts", "moonProtection", "fullPolish");
        let st = "stats";
        self.stats = Stats {
            played: i(st, "gamesPlayed").unwrap_or(0).max(0) as u32,
            won: i(st, "gamesWon").unwrap_or(0).max(0) as u32,
            total_score: i(st, "totalScore").unwrap_or(0),
            best: i(st, "bestScore").filter(|&v| v != 999).map(|v| v as i32),
            moons: i(st, "shootTheMoon").unwrap_or(0).max(0) as u32,
        };
    }

    pub fn save(&self) {
        let r = &self.rules;
        let st = &self.stats;
        let best = st.best.map_or(String::new(), |b| format!("Best={b}\n"));
        let t = format!(
            "[{GENERAL}]\nDeck={}\nBackground={}\nCardWidth={}\nStatusBar={}\nDifficulty={}\nSpeed={}\n\n\
             [Rules]\nEndScore={}\nExactReset={}\nQueenBreaksHearts={}\nMoonProtection={}\nFullPolish={}\n\n\
             [Statistics]\nPlayed={}\nWon={}\nTotalScore={}\n{best}Moons={}\n\n\
             [Window]\nWidth={}\nHeight={}\n",
            self.deck,
            self.theme,
            self.card_width,
            self.status_bar,
            self.difficulty,
            self.speed,
            r.end_score,
            r.exact_reset,
            r.queen_breaks_hearts,
            r.moon_protection,
            r.full_polish,
            st.played,
            st.won,
            st.total_score,
            st.moons,
            self.win_w,
            self.win_h
        );
        let p = Self::path();
        let _ = std::fs::create_dir_all(config_dir());
        let tmp = p.with_extension("tmp");
        if std::fs::write(&tmp, t).is_ok() {
            let _ = std::fs::rename(&tmp, &p);
        }
    }
}

pub const END_SCORES: [i32; 4] = [50, 75, 100, 150];
