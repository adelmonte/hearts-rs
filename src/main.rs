//! hearts-rs: Windows 95 style Hearts, one static binary.
//! Reads KDE card decks and KPat background themes.

mod ai;
mod assets;
mod board;
mod builtin;
mod config;
mod font;
mod game;
mod gfx;
mod rain;
mod ui;
mod wayland;

use ai::Difficulty;
use assets::{Deck, Entry, Theme};
use board::{Layout, Sprite, Target};
use config::{END_SCORES, Settings};
use game::{Game, NPLAYERS, Phase, Played, SOUTH};
use gfx::{Image, Rect, text_width_bold};
use rain::{Rain, Rng};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};
use ui::{Cmd, Dialog, Item, Menu};
use wayland::{ALT, CTRL, Ev, SHIFT, Window, keys};

const NAMES: [&str; NPLAYERS] = ["You", "West", "North", "East"];
const DOUBLE_CLICK_MS: u32 = 400;
const FRAME_MS: f32 = 1000.0 / 60.0;
const BACK: u8 = 52;
const BACK_SIDEWAYS: u8 = 53;

/// Pauses and card moves for Fast, Normal and Slow, in ms. The pauses
/// are qt-hearts' (between computer plays, on a finished trick, between
/// hands).
struct Speed {
    ai: f32,
    trick: f32,
    round: f32,
    mv: f32,
    deal: f32,
}

const SPEEDS: [Speed; 3] = [
    Speed {
        ai: 150.0,
        trick: 700.0,
        round: 700.0,
        mv: 170.0,
        deal: 10.0,
    },
    Speed {
        ai: 500.0,
        trick: 1500.0,
        round: 2000.0,
        mv: 250.0,
        deal: 22.0,
    },
    Speed {
        ai: 900.0,
        trick: 2500.0,
        round: 3000.0,
        mv: 340.0,
        deal: 34.0,
    },
];

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Now,
    Ms(f32),
    Deal,
}

/// What happens next on its own, once the cards have stopped moving.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Step {
    Wait,
    StartPlay,
    Computer,
    Collect,
    EndRound,
    DemoPass,
    DemoAccept,
    DemoPlay,
    DemoNextHand,
}

#[derive(PartialEq)]
enum DialogKind {
    Help,
    Scores,
    HandOver,
    Statistics,
    ConfirmReset,
}

#[derive(Clone, Copy, PartialEq, Default)]
struct Drawn {
    rect: Rect,
    img: u8,
    dim: u8,
    squash: i32,
    focus: bool,
}

#[derive(Clone, PartialEq)]
struct Label {
    rect: Rect,
    text: String,
    color: u32,
}

#[derive(Clone, PartialEq)]
struct Button {
    rect: Rect,
    label: String,
    enabled: bool,
    pressed: bool,
}

struct App {
    win: Window,
    set: Settings,
    decks: Vec<Entry>,
    themes: Vec<Entry>,
    deck: Deck,
    theme: Theme,
    cards: Vec<Image>,
    cw: i32,
    ch: i32,
    bg: Image,
    bg_due: Option<Instant>,
    cards_due: Option<Instant>,
    bg_gen: u64,
    bg_tx: mpsc::Sender<(u64, Image)>,
    bg_rx: mpsc::Receiver<(u64, Image)>,
    fb: Image,
    ui: i32,

    game: Game,
    undo: Vec<Game>,
    redo: Vec<Game>,
    selected: Vec<u8>,
    focus: Option<usize>,
    hint: Option<u8>,
    sprites: [Sprite; 52],
    /// Where each sprite last belonged: a hand (0-3), the trick (4) or
    /// taken (5); a card changing hands moves above the rest.
    owner: [u8; 52],
    lay: Layout,

    step: Step,
    next_step: Instant,
    demo: bool,
    status: String,

    rain: Option<Rain>,
    rain_regrab: Option<Instant>,
    last_frame: Instant,

    menu: Option<Menu>,
    dialog: Option<(DialogKind, Dialog)>,
    button_down: bool,
    dirty: Rect,
    drawn: [Drawn; 52],
    labels: Vec<Label>,
    button: Option<Button>,
    last_press: (u32, i32, i32),
    mouse: (i32, i32),
    rng: Rng,
    quit: bool,
}

fn find_index(list: &[Entry], id: &str) -> Option<usize> {
    list.iter().position(|e| e.id == id)
}

impl App {
    fn view(&self) -> Rect {
        let sb = if self.set.status_bar {
            ui::status_height(self.ui)
        } else {
            0
        };
        Rect::new(0, 0, self.win.w, (self.win.h - sb).max(1))
    }

    fn speed(&self) -> &'static Speed {
        &SPEEDS[self.set.speed.min(2) as usize]
    }

    fn difficulty(&self) -> Difficulty {
        Difficulty::from_index(self.set.difficulty)
    }

    fn label_h(&self) -> i32 {
        (font::HEIGHT + 4) * self.ui
    }

    fn invalidate(&mut self, r: Rect) {
        self.dirty = self.dirty.union(&r);
    }

    fn invalidate_all(&mut self) {
        self.dirty = self.fb.rect();
    }

    // ---------------------------------------------------------------- assets

    fn render_cards(&mut self) {
        self.ch = ((self.cw as f32 / self.deck.aspect).round() as i32).max(1);
        self.cards = self.deck.render_all(self.cw, self.ch);
        let side = self.cards[BACK as usize].rotated();
        self.cards.push(side);
    }

    fn render_bg_now(&mut self) {
        let v = self.view();
        self.bg = self
            .theme
            .svg
            .render("background", v.w, v.h)
            .unwrap_or_else(|| {
                let mut i = Image::new(v.w, v.h);
                i.fill(i.rect(), 0xff008000);
                i
            });
        self.bg_due = None;
        self.bg_gen += 1;
    }

    /// Stretch the old background now, render the real one in the
    /// background once resizing settles.
    fn bg_resized(&mut self) {
        let v = self.view();
        if self.bg.w != v.w || self.bg.h != v.h {
            self.bg = self.bg.scaled(v.w, v.h);
            self.bg_due = Some(Instant::now() + Duration::from_millis(120));
            self.bg_gen += 1;
        }
    }

    fn start_bg_job(&mut self) {
        self.bg_due = None;
        let v = self.view();
        let svg = Arc::clone(&self.theme.svg);
        let tx = self.bg_tx.clone();
        let g = self.bg_gen;
        std::thread::spawn(move || {
            if let Some(img) = svg.render("background", v.w, v.h) {
                let _ = tx.send((g, img));
            }
        });
    }

    fn card_width_for_view(&self) -> i32 {
        let v = self.view();
        if self.set.card_width > 0 {
            self.set.card_width
        } else {
            board::fit_width(v.w, v.h, self.deck.aspect, self.label_h())
        }
    }

    fn relayout(&mut self) {
        let v = self.view();
        self.lay = Layout::new(v.w, v.h, self.cw, self.ch, self.label_h());
        self.sync(Mode::Now);
        self.invalidate_all();
        if self.rain.is_some() {
            self.rain_regrab = Some(Instant::now() + Duration::from_millis(200));
        }
    }

    fn set_card_width(&mut self, w: i32) {
        let w = w.clamp(20, 2000);
        if w != self.cw {
            self.cw = w;
            self.render_cards();
        }
        self.relayout();
    }

    fn zoom(&mut self, factor: f32) {
        let mut w = (self.cw as f32 * factor).round() as i32;
        if w == self.cw {
            w += if factor > 1.0 { 1 } else { -1 };
        }
        let w = w.clamp(20, 2000);
        self.set.card_width = w;
        self.set.save();
        self.set_card_width(w);
    }

    fn load_deck(&mut self, i: usize) {
        match Deck::load(&self.decks[i]) {
            Ok(d) => {
                self.deck = d;
                self.set.deck = self.decks[i].id.clone();
                self.set.save();
                self.ch = 0;
                self.render_cards();
                self.relayout();
            }
            Err(e) => self.set_status(format!("Could not load card deck: {e}")),
        }
    }

    fn load_theme(&mut self, i: usize) {
        match Theme::load(&self.themes[i]) {
            Ok(t) => {
                self.theme = t;
                self.set.theme = self.themes[i].id.clone();
                self.set.save();
                self.render_bg_now();
                self.invalidate_all();
                if self.rain.is_some() {
                    self.rain_regrab = Some(Instant::now());
                }
            }
            Err(e) => self.set_status(format!("Could not load theme: {e}")),
        }
    }

    // ----------------------------------------------------------------- game

    fn set_status(&mut self, s: String) {
        if self.status != s {
            self.status = s;
            self.invalidate_status();
        }
    }

    fn invalidate_status(&mut self) {
        if self.set.status_bar {
            let v = self.view();
            self.invalidate(Rect::new(
                0,
                v.bottom(),
                self.win.w,
                self.win.h - v.bottom(),
            ));
        }
    }

    fn schedule(&mut self, step: Step, ms: f32) {
        self.step = step;
        self.next_step = Instant::now() + Duration::from_millis(ms as u64);
    }

    fn snap(&mut self) {
        for s in self.sprites.iter_mut() {
            if s.anim.is_some() {
                s.finish();
            }
        }
    }

    fn animating(&self) -> bool {
        self.sprites.iter().any(|s| s.anim.is_some())
    }

    fn clear_hint(&mut self) {
        self.hint = None;
    }

    fn new_game(&mut self) {
        self.set.rules.end_score = self.set.rules.end_score.max(1);
        self.game = Game::new(self.set.rules);
        self.undo.clear();
        self.redo.clear();
        self.rain = None;
        self.dialog = None;
        self.invalidate_all();
        self.begin_hand();
    }

    fn begin_hand(&mut self) {
        self.game.deal(&mut self.rng);
        self.selected.clear();
        self.focus = None;
        self.clear_hint();
        let (cx, cy) = (self.lay.cx, self.lay.cy);
        for s in self.sprites.iter_mut() {
            *s = Sprite {
                x: cx,
                y: cy,
                tx: cx,
                ty: cy,
                ..Sprite::default()
            };
        }
        self.sync(Mode::Deal);
        self.invalidate_all();
        if self.game.phase == Phase::Passing {
            let dir = self.game.pass_dir().word().to_lowercase();
            let to = if dir == "across" {
                "across".to_string()
            } else {
                format!("to the {dir}")
            };
            self.set_status(format!("Select three cards to pass {to}."));
            if self.demo {
                self.schedule(Step::DemoPass, self.speed().ai);
            } else {
                self.step = Step::Wait;
            }
        } else {
            self.set_status("There is no passing this hand.".into());
            self.schedule(Step::StartPlay, self.speed().round);
        }
    }

    fn targets(&self) -> [Target; 52] {
        let g = &self.game;
        let raise = self.lay.raise();
        let mut t: [Target; 52] = std::array::from_fn(|_| Target {
            x: self.lay.cx,
            y: self.lay.cy,
            up: false,
            side: false,
            gone: false,
        });
        for (p, hand) in g.hands.iter().enumerate() {
            for (i, &c) in hand.iter().enumerate() {
                let (x, mut y) = self.lay.hand_pos(p, i, hand.len());
                if p == SOUTH
                    && (self.selected.contains(&c)
                        || (g.phase == Phase::Received && g.received[SOUTH].contains(&c)))
                {
                    y -= raise;
                }
                t[c as usize] = Target {
                    x,
                    y,
                    up: p == SOUTH,
                    side: Layout::sideways(p),
                    gone: false,
                };
            }
        }
        for &(p, c) in &g.trick {
            let (x, y) = self.lay.trick_pos(p);
            t[c as usize] = Target {
                x,
                y,
                up: true,
                side: false,
                gone: false,
            };
        }
        for c in 0..52 {
            if let Some(w) = g.taken_by[c] {
                let (x, y) = self.lay.seat(w);
                t[c] = Target {
                    x,
                    y,
                    up: true,
                    side: false,
                    gone: true,
                };
            }
        }
        t
    }

    fn owners(&self) -> [u8; 52] {
        let mut o = [u8::MAX; 52];
        for (p, hand) in self.game.hands.iter().enumerate() {
            for &c in hand {
                o[c as usize] = p as u8;
            }
        }
        for &(_, c) in &self.game.trick {
            o[c as usize] = 4;
        }
        for c in 0..52 {
            if self.game.taken_by[c].is_some() {
                o[c] = 5;
            }
        }
        o
    }

    fn zorder(&self) -> [i32; 52] {
        let mut z = [0; 52];
        for hand in &self.game.hands {
            for (i, &c) in hand.iter().enumerate() {
                z[c as usize] = i as i32;
            }
        }
        for (k, &(_, c)) in self.game.trick.iter().enumerate() {
            z[c as usize] = 100 + k as i32;
        }
        for c in 0..52 {
            if self.game.taken_by[c].is_some() {
                z[c] = -100;
            }
        }
        z
    }

    /// Bring every sprite to where the game says it belongs.
    fn sync(&mut self, mode: Mode) {
        let now = Instant::now();
        let targets = self.targets();
        let z = self.zorder();
        let owners = self.owners();
        let deal_order: [usize; 52] = {
            let mut o = [0; 52];
            for (p, hand) in self.game.hands.iter().enumerate() {
                for (i, &c) in hand.iter().enumerate() {
                    o[c as usize] = i * NPLAYERS + p;
                }
            }
            o
        };
        let sp = self.speed();
        for (c, t) in targets.iter().enumerate() {
            let s = &mut self.sprites[c];
            s.z = z[c];
            let changed_hands = owners[c] != self.owner[c] && owners[c] != 5;
            self.owner[c] = owners[c];
            if s.same(t) && mode != Mode::Now {
                continue;
            }
            let (ms, start) = match mode {
                Mode::Now => (0.0, now),
                Mode::Ms(ms) => (ms, now),
                Mode::Deal => (
                    sp.mv,
                    now + Duration::from_millis((deal_order[c] as f32 * sp.deal) as u64),
                ),
            };
            s.go(t, ms, changed_hands && mode != Mode::Deal, start);
        }
    }

    /// Whose turn it is, and what that means.
    fn next_turn(&mut self) {
        let g = &self.game;
        if g.phase != Phase::Playing {
            return;
        }
        if g.trick_full() {
            self.schedule(Step::Collect, self.speed().trick);
        } else if g.turn == SOUTH {
            if self.demo {
                self.schedule(Step::DemoPlay, self.speed().ai);
            } else {
                self.step = Step::Wait;
                if self.status.is_empty() {
                    self.set_status("Click on the card you want to play.".into());
                }
            }
        } else {
            self.schedule(Step::Computer, self.speed().ai);
        }
    }

    fn play_card(&mut self, c: u8) {
        self.clear_hint();
        let broken = self.game.hearts_broken;
        let r = self.game.play(c);
        if self.focus.is_some() {
            let n = self.game.hands[SOUTH].len();
            self.focus = self.focus.map(|f| f.min(n.saturating_sub(1))).filter(|_| n > 0);
        }
        self.sync(Mode::Ms(self.speed().mv));
        if !broken && self.game.hearts_broken {
            self.set_status("Hearts have been broken.".into());
        } else {
            self.set_status(String::new());
        }
        match r {
            Played::TrickFull | Played::Next => self.next_turn(),
        }
    }

    fn human_play(&mut self, c: u8) {
        let g = &self.game;
        if g.phase != Phase::Playing || g.turn != SOUTH || g.trick_full() || self.step != Step::Wait {
            return;
        }
        if !g.valid(SOUTH).contains(&c) {
            let why = g.why_not(c);
            self.set_status(why.into());
            return;
        }
        self.undo.push(self.game.clone());
        self.redo.clear();
        self.set_status(String::new());
        self.play_card(c);
    }

    fn collect(&mut self) {
        let (w, pts) = self.game.collect();
        let who = if w == SOUTH {
            "You take".to_string()
        } else {
            format!("{} takes", NAMES[w])
        };
        let msg = match pts {
            0 => format!("{who} the trick."),
            1 => format!("{who} the trick: 1 point."),
            n => format!("{who} the trick: {n} points."),
        };
        self.set_status(msg);
        self.sync(Mode::Ms(self.speed().mv * 1.4));
        if self.game.phase == Phase::RoundOver {
            self.schedule(Step::EndRound, self.speed().ai);
        } else {
            self.next_turn();
        }
    }

    fn end_round(&mut self) {
        let over = self.game.end_round();
        let hand = self.game.sheet.last().cloned();
        if let Some(m) = hand.as_ref().and_then(|h| h.moon) {
            if m == SOUTH {
                self.set.stats.moons += 1;
                self.set_status("You shot the moon!".into());
            } else {
                self.set_status(format!("{} shot the moon!", NAMES[m]));
            }
        }
        if over {
            let w = self.game.winner();
            let st = &mut self.set.stats;
            st.played += 1;
            st.won += (w == SOUTH) as u32;
            st.total_score += self.game.total[SOUTH] as i64;
            st.best = Some(st.best.map_or(self.game.total[SOUTH], |b| b.min(self.game.total[SOUTH])));
            self.undo.clear();
            self.redo.clear();
            self.demo = false;
        }
        self.set.save();
        let title = if over { "Game Over" } else { "Score Sheet" };
        let d = self.score_dialog(title);
        self.dialog = Some((DialogKind::HandOver, d));
        self.invalidate_all();
        if self.demo {
            self.schedule(Step::DemoNextHand, self.speed().round);
        } else {
            self.step = Step::Wait;
        }
    }

    /// The hand-over score sheet was dismissed.
    fn hand_over_done(&mut self) {
        self.dialog = None;
        self.invalidate_all();
        if self.game.phase == Phase::GameOver {
            let w = self.game.winner();
            if w == SOUTH {
                self.set_status("You won! Double-click or press F2 for a new game.".into());
                self.start_rain();
            } else {
                self.set_status(format!(
                    "{} won. Double-click or press F2 for a new game.",
                    NAMES[w]
                ));
            }
            self.step = Step::Wait;
        } else {
            self.begin_hand();
        }
    }

    fn score_dialog(&self, title: &str) -> Dialog {
        let g = &self.game;
        let mut lines = vec![format!("\t{}", NAMES.join("\t"))];
        lines.push("---".into());
        let first = g.sheet.len().saturating_sub(20);
        for (i, h) in g.sheet.iter().enumerate().skip(first) {
            let cells: Vec<String> = h.points.iter().map(|p| p.to_string()).collect();
            let mut row = format!("Hand {}\t{}", i + 1, cells.join("\t"));
            if let Some(m) = h.moon {
                row.push_str(&format!("\t{} shot the moon", NAMES[m]));
            }
            lines.push(row);
        }
        if g.phase == Phase::Playing && g.round_pts.iter().any(|&p| p > 0) {
            let cells: Vec<String> = g.round_pts.iter().map(|p| format!("({p})")).collect();
            lines.push(format!("This hand\t{}", cells.join("\t")));
        }
        lines.push("---".into());
        let totals: Vec<String> = g.total.iter().map(|t| t.to_string()).collect();
        lines.push(format!("Total\t{}", totals.join("\t")));
        if g.phase == Phase::GameOver {
            lines.push(String::new());
            let w = g.winner();
            lines.push(if w == SOUTH {
                "You won the game!".into()
            } else {
                format!("{} won the game.", NAMES[w])
            });
        } else {
            lines.push(String::new());
            lines.push(format!("The game ends at {} points.", g.rules.end_score));
        }
        let mut d = Dialog::new(title, lines);
        d.table = true;
        d
    }

    fn do_pass(&mut self) {
        if self.game.phase != Phase::Passing || self.selected.len() != 3 {
            return;
        }
        self.clear_hint();
        let mine = std::mem::take(&mut self.selected);
        let diff = self.difficulty();
        self.game.pass(&mine, diff, &mut self.rng);
        self.focus = None;
        self.sync(Mode::Ms(self.speed().mv * 1.6));
        self.set_status("These are the cards you were passed. Click OK to play.".into());
        if self.demo {
            self.schedule(Step::DemoAccept, self.speed().trick);
        } else {
            self.step = Step::Wait;
        }
    }

    fn accept_received(&mut self) {
        if self.game.phase != Phase::Received {
            return;
        }
        self.game.phase = Phase::Ready;
        self.sync(Mode::Ms(self.speed().mv));
        self.set_status(String::new());
        self.schedule(Step::StartPlay, 0.0);
    }

    fn step(&mut self) {
        let step = std::mem::replace(&mut self.step, Step::Wait);
        match step {
            Step::Wait => {}
            Step::StartPlay => {
                self.game.start_play();
                if self.game.turn != SOUTH && self.status.is_empty() {
                    self.set_status(format!("{} leads.", NAMES[self.game.turn]));
                }
                self.next_turn();
            }
            Step::Computer => {
                let c = self.game.ai_play(self.difficulty(), &mut self.rng);
                self.play_card(c);
            }
            Step::DemoPlay => {
                if self.game.turn == SOUTH && !self.game.trick_full() {
                    let c = self.game.ai_play(Difficulty::Hard, &mut self.rng);
                    self.play_card(c);
                }
            }
            Step::Collect => self.collect(),
            Step::EndRound => self.end_round(),
            Step::DemoPass => {
                if self.game.phase == Phase::Passing {
                    self.selected = self.suggest_pass();
                    self.do_pass();
                }
            }
            Step::DemoAccept => self.accept_received(),
            Step::DemoNextHand => {
                if matches!(self.dialog, Some((DialogKind::HandOver, _))) {
                    self.hand_over_done();
                }
            }
        }
    }

    /// Pick up whatever should happen next after a restore or a demo
    /// toggle.
    fn resume(&mut self) {
        self.step = Step::Wait;
        if matches!(self.dialog, Some((DialogKind::HandOver, _))) {
            if self.demo {
                self.schedule(Step::DemoNextHand, self.speed().round);
            }
            return;
        }
        match self.game.phase {
            Phase::Passing if self.demo => self.schedule(Step::DemoPass, self.speed().ai),
            Phase::Received if self.demo => self.schedule(Step::DemoAccept, self.speed().ai),
            Phase::Ready => self.schedule(Step::StartPlay, 0.0),
            Phase::Playing => self.next_turn(),
            Phase::RoundOver => self.schedule(Step::EndRound, 0.0),
            _ => {}
        }
    }

    fn suggest_pass(&mut self) -> Vec<u8> {
        let ctx = self.game.ctx(Difficulty::Hard);
        ai::choose_pass(&self.game.hands[SOUTH], SOUTH, &ctx, &mut self.rng)
    }

    fn hint(&mut self) {
        self.snap();
        match self.game.phase {
            Phase::Passing => {
                self.selected = self.suggest_pass();
                self.sync(Mode::Ms(self.speed().mv * 0.6));
                self.set_status("Suggested cards to pass are raised.".into());
            }
            Phase::Playing
                if self.game.turn == SOUTH && !self.game.trick_full() && self.step == Step::Wait =>
            {
                let c = self.game.ai_play(Difficulty::Hard, &mut self.rng);
                self.hint = Some(c);
                self.set_status(format!("Hint: play the {}.", game::name(c)));
            }
            _ => {}
        }
    }

    fn can_undo(&self) -> bool {
        !self.undo.is_empty() && self.can_rewind()
    }

    fn can_redo(&self) -> bool {
        !self.redo.is_empty() && self.can_rewind()
    }

    fn can_rewind(&self) -> bool {
        self.dialog.is_none() && self.game.phase != Phase::GameOver
    }

    fn undo(&mut self) {
        if !self.can_undo() {
            return;
        }
        let prev = self.undo.pop().unwrap();
        self.redo.push(std::mem::replace(&mut self.game, prev));
        self.after_history();
    }

    fn redo(&mut self) {
        if !self.can_redo() {
            return;
        }
        let next = self.redo.pop().unwrap();
        self.undo.push(std::mem::replace(&mut self.game, next));
        self.after_history();
    }

    fn after_history(&mut self) {
        self.snap();
        self.demo = false;
        self.clear_hint();
        self.selected.clear();
        self.focus = None;
        self.game.rules = self.set.rules;
        self.set_status(String::new());
        self.sync(Mode::Ms(self.speed().mv));
        self.invalidate_all();
        self.resume();
    }

    fn rules_changed(&mut self) {
        self.set.save();
        self.game.rules = self.set.rules;
    }

    // ------------------------------------------------------------ win rain

    fn rain_queue(&self) -> Vec<(u8, f32, f32)> {
        (0..52u8)
            .map(|c| {
                let (x, y) = self.lay.seat(c as usize % NPLAYERS);
                (
                    c,
                    x - self.cw as f32 / 2.0,
                    (y - self.ch as f32 / 2.0).max(0.0),
                )
            })
            .collect()
    }

    fn board_snapshot(&mut self) -> Image {
        let v = self.view();
        let saved = self.rain.take();
        self.compose_board(v);
        self.rain = saved;
        let mut snap = Image::new(v.w, v.h);
        snap.copy_from(&self.fb, 0, 0, v);
        snap
    }

    fn start_rain(&mut self) {
        let snap = self.board_snapshot();
        let q = self.rain_queue();
        self.rain = Some(Rain::new(snap, q));
        self.last_frame = Instant::now();
        self.invalidate_all();
    }

    // ---------------------------------------------------------------- input

    fn sprite_rect(&self, c: usize) -> Rect {
        let s = &self.sprites[c];
        self.lay.rect_at(s.x, s.y, s.shown_side() && s.flip < 0.5)
    }

    /// South's card under the pointer, as an index into the hand.
    fn south_at(&self, x: i32, y: i32) -> Option<usize> {
        let hand = &self.game.hands[SOUTH];
        (0..hand.len())
            .rev()
            .find(|&i| self.sprite_rect(hand[i] as usize).contains(x, y))
    }

    fn button_state(&self) -> Option<Button> {
        let (label, enabled) = match self.game.phase {
            Phase::Passing => (
                format!("Pass {}", self.game.pass_dir().word()),
                self.selected.len() == 3,
            ),
            Phase::Received => ("OK".to_string(), true),
            _ => return None,
        };
        if self.dialog.is_some() {
            return None;
        }
        let s = self.ui;
        let w = (gfx::text_width(&label, s) + 24 * s).max(75 * s);
        let h = 23 * s;
        let rect = Rect::new(
            self.lay.cx as i32 - w / 2,
            self.lay.cy as i32 - h / 2,
            w,
            h,
        );
        Some(Button {
            rect,
            label,
            enabled,
            pressed: self.button_down,
        })
    }

    fn press_button(&mut self) {
        match self.game.phase {
            Phase::Passing => self.do_pass(),
            Phase::Received => self.accept_received(),
            _ => {}
        }
    }

    fn toggle_select(&mut self, i: usize) {
        let c = self.game.hands[SOUTH][i];
        if let Some(k) = self.selected.iter().position(|&x| x == c) {
            self.selected.remove(k);
        } else if self.selected.len() < 3 {
            self.selected.push(c);
        } else {
            return;
        }
        self.clear_hint();
        self.sync(Mode::Ms(self.speed().mv * 0.6));
    }

    /// Click (or Space) on South's card `i`.
    fn activate(&mut self, i: usize) {
        match self.game.phase {
            Phase::Passing => self.toggle_select(i),
            Phase::Playing => {
                let c = self.game.hands[SOUTH][i];
                self.human_play(c);
            }
            _ => {}
        }
    }

    fn menu_items(&self) -> Vec<Item> {
        let decks = self
            .decks
            .iter()
            .enumerate()
            .map(|(i, e)| Item::check(&e.name, "", Cmd::Deck(i), e.id == self.set.deck))
            .collect();
        let themes = self
            .themes
            .iter()
            .enumerate()
            .map(|(i, e)| Item::check(&e.name, "", Cmd::Theme(i), e.id == self.set.theme))
            .collect();
        let diff = ["Easy", "Medium", "Hard"]
            .iter()
            .enumerate()
            .map(|(i, n)| {
                Item::check(n, "", Cmd::Difficulty(i as u8), self.set.difficulty == i as u8)
            })
            .collect();
        let speed = ["Fast", "Normal", "Slow"]
            .iter()
            .enumerate()
            .map(|(i, n)| Item::check(n, "", Cmd::Speed(i as u8), self.set.speed == i as u8))
            .collect();
        let r = &self.set.rules;
        let mut rules: Vec<Item> = END_SCORES
            .iter()
            .map(|&n| {
                Item::check(
                    &format!("Game Ends at {n}"),
                    "",
                    Cmd::EndScore(n),
                    r.end_score == n,
                )
            })
            .collect();
        rules.extend([
            Item::Sep,
            Item::check(
                "Queen of Spades Breaks Hearts",
                "",
                Cmd::ToggleQueenBreaks,
                r.queen_breaks_hearts,
            ),
            Item::check(
                &format!("Exactly {} Resets to 50", r.end_score),
                "",
                Cmd::ToggleExactReset,
                r.exact_reset,
            ),
            Item::check(
                "Moon Shooter May Take -26",
                "",
                Cmd::ToggleMoonProtection,
                r.moon_protection,
            ),
            Item::check(
                "Full Polish (99 + 25 = 98)",
                "",
                Cmd::ToggleFullPolish,
                r.full_polish,
            ),
        ]);
        let playing = self.game.phase != Phase::GameOver;
        vec![
            Item::cmd("New Game", "Ctrl+N", Cmd::NewGame),
            Item::Sep,
            Item::cmd("Undo", "Ctrl+Z", Cmd::Undo).enabled(self.can_undo()),
            Item::cmd("Redo", "Ctrl+Shift+Z", Cmd::Redo).enabled(self.can_redo()),
            Item::cmd("Hint", "H", Cmd::Hint).enabled(playing),
            Item::check("Demo", "D", Cmd::Demo, self.demo).enabled(playing),
            Item::Sep,
            Item::cmd("Score Sheet...", "Ctrl+Shift+S", Cmd::ScoreSheet),
            Item::cmd("Statistics...", "Ctrl+Shift+T", Cmd::Statistics),
            Item::cmd("Reset Statistics...", "", Cmd::ResetStats),
            Item::Sep,
            Item::Sub {
                label: "Opponents".into(),
                items: diff,
            },
            Item::Sub {
                label: "Speed".into(),
                items: speed,
            },
            Item::Sub {
                label: "Rules".into(),
                items: rules,
            },
            Item::Sub {
                label: "Card Deck".into(),
                items: decks,
            },
            Item::Sub {
                label: "Background".into(),
                items: themes,
            },
            Item::check("Status Bar", "", Cmd::ToggleStatus, self.set.status_bar),
            Item::Sep,
            Item::cmd("Bigger Cards", "Ctrl++", Cmd::ZoomIn),
            Item::cmd("Smaller Cards", "Ctrl+-", Cmd::ZoomOut),
            Item::cmd("Fit Cards to Window", "Ctrl+0", Cmd::ZoomFit),
            Item::cmd("Full Screen", "F11", Cmd::Fullscreen),
            Item::Sep,
            Item::cmd("Rules and Keys...", "F1", Cmd::Help),
            Item::cmd("Quit", "Ctrl+Q", Cmd::Quit),
        ]
    }

    fn open_menu(&mut self, x: i32, y: i32) {
        self.snap();
        let m = Menu::open(self.menu_items(), x, y, self.ui, self.fb.rect());
        self.invalidate(m.bounds());
        self.menu = Some(m);
    }

    fn close_menu(&mut self) {
        if let Some(m) = self.menu.take() {
            self.invalidate(m.bounds());
        }
    }

    fn show_dialog(&mut self, kind: DialogKind, d: Dialog) {
        // The end-of-hand sheet holds up the game; don't lose it.
        if matches!(self.dialog, Some((DialogKind::HandOver, _))) {
            return;
        }
        self.dialog = Some((kind, d));
        self.invalidate_all();
    }

    fn close_dialog(&mut self) {
        match self.dialog.take() {
            Some((DialogKind::HandOver, _)) => self.hand_over_done(),
            Some(_) => self.invalidate_all(),
            None => {}
        }
    }

    fn stats_dialog(&self) -> Dialog {
        let st = &self.set.stats;
        let rate = if st.played > 0 {
            100.0 * st.won as f64 / st.played as f64
        } else {
            0.0
        };
        let avg = if st.played > 0 {
            st.total_score as f64 / st.played as f64
        } else {
            0.0
        };
        let best = st.best.map_or("-".to_string(), |b| b.to_string());
        let lines = vec![
            format!("Games played:\t{}", st.played),
            format!("Games won:\t{}", st.won),
            format!("Win rate:\t{rate:.1}%"),
            format!("Average score:\t{avg:.1}"),
            format!("Best score:\t{best}"),
            format!("Moons shot:\t{}", st.moons),
        ];
        let mut d = Dialog::new("Statistics", lines);
        d.table = true;
        d
    }

    fn run(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::NewGame => {
                self.demo = false;
                self.new_game();
            }
            Cmd::Undo => self.undo(),
            Cmd::Redo => self.redo(),
            Cmd::Hint => self.hint(),
            Cmd::Demo => {
                if self.game.phase != Phase::GameOver {
                    self.demo = !self.demo;
                    self.clear_hint();
                    if self.step == Step::Wait {
                        self.resume();
                    } else if !self.demo && matches!(self.step, Step::DemoPass | Step::DemoAccept | Step::DemoPlay | Step::DemoNextHand) {
                        self.step = Step::Wait;
                        if self.game.phase == Phase::Playing {
                            self.next_turn();
                        }
                    }
                }
            }
            Cmd::ScoreSheet => {
                let d = self.score_dialog("Score Sheet");
                self.show_dialog(DialogKind::Scores, d);
            }
            Cmd::Statistics => {
                let d = self.stats_dialog();
                self.show_dialog(DialogKind::Statistics, d);
            }
            Cmd::ResetStats => {
                let mut d = Dialog::new(
                    "Reset Statistics",
                    vec![
                        "Clear all statistics? This can't be undone.".into(),
                        "Click Reset or press Enter to clear them, Esc to keep them.".into(),
                    ],
                );
                d.button = Some("Reset");
                self.show_dialog(DialogKind::ConfirmReset, d);
            }
            Cmd::Difficulty(i) => {
                self.set.difficulty = i;
                self.set.save();
            }
            Cmd::Speed(i) => {
                self.set.speed = i;
                self.set.save();
            }
            Cmd::EndScore(n) => {
                self.set.rules.end_score = n;
                self.rules_changed();
            }
            Cmd::ToggleQueenBreaks => {
                self.set.rules.queen_breaks_hearts = !self.set.rules.queen_breaks_hearts;
                self.rules_changed();
            }
            Cmd::ToggleExactReset => {
                self.set.rules.exact_reset = !self.set.rules.exact_reset;
                self.rules_changed();
            }
            Cmd::ToggleMoonProtection => {
                self.set.rules.moon_protection = !self.set.rules.moon_protection;
                self.rules_changed();
            }
            Cmd::ToggleFullPolish => {
                self.set.rules.full_polish = !self.set.rules.full_polish;
                self.rules_changed();
            }
            Cmd::ToggleStatus => {
                self.set.status_bar = !self.set.status_bar;
                self.set.save();
                self.bg_resized();
                self.relayout();
            }
            Cmd::Deck(i) => self.load_deck(i),
            Cmd::Theme(i) => self.load_theme(i),
            Cmd::ZoomIn => self.zoom(1.15),
            Cmd::ZoomOut => self.zoom(1.0 / 1.15),
            Cmd::ZoomFit => {
                self.set.card_width = 0;
                self.set.save();
                let w = self.card_width_for_view();
                self.set_card_width(w);
            }
            Cmd::Fullscreen => {
                let _ = self.win.toggle_fullscreen();
            }
            Cmd::Help => {
                let end = self.set.rules.end_score;
                let lines = vec![
                    "Avoid taking hearts (1 point each) and the queen of spades (13).".to_string(),
                    "Take all 26 points to shoot the moon: everyone else gets 26.".into(),
                    format!("The game ends when someone reaches {end}; the lowest score wins."),
                    "The two of clubs leads; follow suit if you can. Hearts can't be".into(),
                    "led until a heart (or the queen of spades) has been played.".into(),
                    String::new(),
                    "Click three cards, then the Pass button. Click a card to play it.".into(),
                    "Right-click the table for the menu.".into(),
                    String::new(),
                    "Ctrl+N, F2\tNew game".into(),
                    "Ctrl+Z\tUndo".into(),
                    "Ctrl+Shift+Z, Ctrl+Y\tRedo".into(),
                    "H\tHint".into(),
                    "D\tDemo".into(),
                    "Left, Right, Home, End\tChoose a card".into(),
                    "Space\tSelect or play it".into(),
                    "Enter\tPass / OK".into(),
                    "Ctrl+Shift+S\tScore sheet".into(),
                    "Ctrl+Shift+T\tStatistics".into(),
                    "Ctrl++, Ctrl+-\tBigger / smaller cards".into(),
                    "Ctrl+0\tFit cards to window".into(),
                    "F10, Menu\tMenu".into(),
                    "F11\tFull screen".into(),
                    "Ctrl+Q\tQuit".into(),
                ];
                self.show_dialog(DialogKind::Help, Dialog::new("Hearts", lines));
            }
            Cmd::Quit => self.quit = true,
        }
    }

    fn on_press(&mut self, button: u8, x: i32, y: i32, time: u32, mods: u16) {
        let dbl = button == 1
            && time.wrapping_sub(self.last_press.0) < DOUBLE_CLICK_MS
            && (x - self.last_press.1).abs() < 6
            && (y - self.last_press.2).abs() < 6;
        if button == 1 {
            self.last_press = if dbl {
                (time.wrapping_sub(DOUBLE_CLICK_MS), x, y)
            } else {
                (time, x, y)
            };
        }

        if let Some((kind, d)) = &self.dialog {
            if button == 1 || button == 3 {
                let v = self.view();
                let on_button = d.button_rect(self.ui, v).is_some_and(|b| b.contains(x, y));
                let inside = d.rect(self.ui, v).contains(x, y);
                if *kind == DialogKind::ConfirmReset {
                    if on_button {
                        self.set.stats = Default::default();
                        self.set.save();
                        self.set_status("Statistics cleared.".into());
                    }
                    if on_button || !inside {
                        self.close_dialog();
                    }
                } else {
                    self.close_dialog();
                }
            }
            return;
        }
        if let Some(menu) = &mut self.menu {
            if button == 1 || button == 3 {
                let r = menu.click(x, y, self.fb.rect());
                match r {
                    Some(Some(cmd)) => {
                        self.close_menu();
                        self.run(cmd);
                    }
                    Some(None) => self.invalidate(self.menu.as_ref().unwrap().bounds()),
                    None => self.close_menu(),
                }
            }
            return;
        }
        if (button == 4 || button == 5) && mods & CTRL != 0 {
            self.zoom(if button == 4 { 1.15 } else { 1.0 / 1.15 });
            return;
        }
        if self.game.phase == Phase::GameOver {
            if button == 1 && dbl {
                self.run(Cmd::NewGame);
            } else if button == 1 && self.rain.is_some() {
                self.rain = None;
                self.invalidate_all();
            } else if button == 3 {
                self.open_menu(x, y);
            }
            return;
        }
        match button {
            1 => {
                // Clicking while cards move puts them in place and takes
                // the click.
                self.snap();
                if let Some(b) = self.button_state()
                    && b.rect.contains(x, y)
                {
                    if b.enabled {
                        self.button_down = true;
                    }
                    return;
                }
                if let Some(i) = self.south_at(x, y) {
                    self.focus = None;
                    self.activate(i);
                } else if self.step == Step::Collect {
                    // Don't make anyone wait for a trick they've seen.
                    self.next_step = Instant::now();
                }
            }
            3 => self.open_menu(x, y),
            _ => {}
        }
    }

    fn on_release(&mut self, button: u8, x: i32, y: i32) {
        if button != 1 || !std::mem::take(&mut self.button_down) {
            return;
        }
        if self.button_state().is_some_and(|b| b.rect.contains(x, y)) {
            self.press_button();
        }
    }

    fn on_motion(&mut self, x: i32, y: i32) {
        self.mouse = (x, y);
        if let Some(m) = &mut self.menu {
            let before = m.bounds();
            if m.hover(x, y, self.fb.rect()) {
                let after = self.menu.as_ref().unwrap().bounds();
                self.invalidate(before.union(&after));
            }
        }
    }

    fn move_focus(&mut self, sym: u32) {
        let n = self.game.hands[SOUTH].len();
        if n == 0 {
            return;
        }
        self.focus = Some(match (sym, self.focus) {
            (keys::HOME, _) => 0,
            (keys::END, _) => n - 1,
            (keys::LEFT, Some(f)) if f > 0 => f - 1,
            (keys::LEFT, _) => n - 1,
            (keys::RIGHT, Some(f)) if f + 1 < n => f + 1,
            (_, _) => 0,
        });
    }

    fn on_key(&mut self, sym: u32, mods: u16) {
        let ctrl = mods & CTRL != 0;
        let shift = mods & SHIFT != 0;
        if let Some((kind, _)) = &self.dialog {
            if *kind == DialogKind::ConfirmReset {
                if sym == keys::RETURN || sym == keys::KP_ENTER {
                    self.set.stats = Default::default();
                    self.set.save();
                    self.set_status("Statistics cleared.".into());
                    self.close_dialog();
                } else if sym == keys::ESCAPE {
                    self.close_dialog();
                }
            } else if ctrl && sym == 'q' as u32 {
                self.quit = true;
            } else if !matches!(sym, 0xffe1..=0xffee) {
                // Any key but a bare modifier dismisses it.
                self.close_dialog();
            }
            return;
        }
        if self.menu.is_some() {
            if sym == keys::ESCAPE || sym == keys::F10 || sym == keys::MENU {
                self.close_menu();
            }
            return;
        }
        let k = |c: char| sym == c as u32;
        let cmd = match () {
            _ if ctrl && k('q') => Some(Cmd::Quit),
            _ if ctrl && k('n') || sym == keys::F2 => Some(Cmd::NewGame),
            _ if ctrl && (k('y') || (k('z') && shift)) => Some(Cmd::Redo),
            _ if ctrl && k('z') => Some(Cmd::Undo),
            _ if ctrl && shift && k('s') => Some(Cmd::ScoreSheet),
            _ if ctrl && shift && k('t') => Some(Cmd::Statistics),
            _ if ctrl && (k('+') || k('=') || sym == keys::KP_ADD) => Some(Cmd::ZoomIn),
            _ if ctrl && (k('-') || k('_') || sym == keys::KP_SUB) => Some(Cmd::ZoomOut),
            _ if ctrl && (k('0') || k(')') || sym == keys::KP_0) => Some(Cmd::ZoomFit),
            _ if sym == keys::F11 || (ctrl && shift && k('f')) => Some(Cmd::Fullscreen),
            _ if sym == keys::F1 => Some(Cmd::Help),
            _ if ctrl || mods & ALT != 0 => None,
            _ if k('h') => Some(Cmd::Hint),
            _ if k('d') => Some(Cmd::Demo),
            _ => None,
        };
        if let Some(c) = cmd {
            self.run(c);
            return;
        }
        match sym {
            keys::F10 | keys::MENU => {
                let (x, y) = self.mouse;
                self.open_menu(x, y);
            }
            keys::ESCAPE => {
                if self.rain.is_some() {
                    self.rain = None;
                    self.invalidate_all();
                }
                if self.demo {
                    self.run(Cmd::Demo);
                }
                self.focus = None;
            }
            keys::LEFT | keys::RIGHT | keys::HOME | keys::END => self.move_focus(sym),
            keys::RETURN | keys::KP_ENTER => {
                self.snap();
                if self.button_state().is_some_and(|b| b.enabled) {
                    self.press_button();
                } else if let Some(f) = self.focus {
                    self.activate(f);
                } else if self.step == Step::Collect {
                    self.next_step = Instant::now();
                }
            }
            keys::SPACE => {
                self.snap();
                if let Some(f) = self.focus {
                    self.activate(f);
                } else if self.step == Step::Collect {
                    self.next_step = Instant::now();
                }
            }
            _ => {}
        }
    }

    fn on_resize(&mut self) {
        self.fb = Image::new(self.win.w, self.win.h);
        self.ui = self.win.dpi_scale;
        self.close_menu();
        self.bg_resized();
        self.snap();
        // Cards follow the window until you pick a size; re-render them
        // once resizing settles.
        if self.set.card_width == 0 && self.card_width_for_view() != self.cw {
            self.cards_due = Some(Instant::now() + Duration::from_millis(120));
        }
        self.relayout();
        if self.rain.as_ref().is_some_and(|r| r.finished) {
            self.rain = None;
        }
        if !self.win.fullscreen {
            self.set.win_w = self.win.w;
            self.set.win_h = self.win.h;
        }
    }

    // -------------------------------------------------------------- drawing

    fn current_labels(&self) -> Vec<Label> {
        let s = self.ui;
        let g = &self.game;
        (0..NPLAYERS)
            .map(|p| {
                let mut text = format!("{}  {}", NAMES[p], g.total[p]);
                if g.round_pts[p] > 0 {
                    text.push_str(&format!("  (+{})", g.round_pts[p]));
                }
                let w = text_width_bold(&text, s) + 2 * s;
                let (cx, top) = self.lay.label_pos(p);
                let turn = g.phase == Phase::Playing && g.turn == p && !g.trick_full();
                Label {
                    rect: Rect::new(cx - w / 2, top, w, (font::HEIGHT + 1) * s),
                    text,
                    color: if turn { 0xffffff00 } else { ui::WHITE },
                }
            })
            .collect()
    }

    fn compose_board(&mut self, clip: Rect) {
        let v = self.view();
        let clip = clip.intersect(&v);
        if clip.is_empty() {
            return;
        }
        self.fb.copy_from(&self.bg, 0, 0, clip);
        for c in self.paint_order() {
            let d = self.drawn_for(c as usize);
            if d.rect.intersect(&clip).is_empty() || d.squash <= 0 {
                continue;
            }
            let img = &self.cards[d.img as usize];
            self.fb
                .blit_squashed(img, d.rect.x, d.rect.y, d.squash, clip, d.dim);
            if d.focus {
                focus_rect(&mut self.fb, d.rect.inflate(-3 * self.ui), self.ui, clip);
            }
        }
        let s = self.ui;
        for l in &self.labels {
            if l.rect.intersect(&clip).is_empty() {
                continue;
            }
            self.fb
                .text_bold(&l.text, l.rect.x + s, l.rect.y + s, s, ui::BLACK, clip);
            self.fb
                .text_bold(&l.text, l.rect.x, l.rect.y, s, l.color, clip);
        }
        if let Some(b) = &self.button {
            ui::button(
                &mut self.fb,
                b.rect,
                s,
                &b.label,
                b.enabled,
                b.pressed,
                b.enabled,
                clip,
            );
        }
    }

    /// Cards in paint order (bottom first).
    fn paint_order(&self) -> Vec<u8> {
        let mut v: Vec<(bool, i32, u8)> = (0..52u8)
            .filter(|&c| self.sprites[c as usize].visible())
            .map(|c| {
                let s = &self.sprites[c as usize];
                (s.raised(), s.z, c)
            })
            .collect();
        v.sort_unstable();
        v.into_iter().map(|t| t.2).collect()
    }

    fn drawn_for(&self, c: usize) -> Drawn {
        let s = &self.sprites[c];
        if !s.visible() {
            return Drawn::default();
        }
        let face = s.flip >= 0.5;
        let side = !face && s.shown_side();
        let img = if face {
            c as u8
        } else if side {
            BACK_SIDEWAYS
        } else {
            BACK
        };
        let iw = self.cards[img as usize].w;
        let cos = (std::f32::consts::PI * s.flip).cos().abs();
        let squash = (cos * iw as f32).round() as i32;
        // A card seen nearly edge-on catches less light.
        let turn = ((1.0 - cos) * 90.0) as u8;
        let dim = if self.hint == Some(c as u8) {
            128
        } else {
            turn
        };
        let focus = self
            .focus
            .and_then(|f| self.game.hands[SOUTH].get(f))
            .is_some_and(|&x| x as usize == c)
            && s.anim.is_none();
        Drawn {
            rect: self.lay.rect_at(s.x, s.y, side),
            img,
            dim,
            squash: squash.max(1),
            focus,
        }
    }

    fn render(&mut self) {
        let v = self.view();
        if self.rain.is_none() {
            for c in 0..52 {
                let d = self.drawn_for(c);
                if d != self.drawn[c] {
                    self.dirty = self.dirty.union(&self.drawn[c].rect).union(&d.rect);
                    self.drawn[c] = d;
                }
            }
            let labels = self.current_labels();
            if labels != self.labels {
                for l in self.labels.iter().chain(&labels) {
                    self.dirty = self.dirty.union(&l.rect.inflate(self.ui));
                }
                self.labels = labels;
            }
            let b = self.button_state();
            if b != self.button {
                for r in self.button.iter().chain(&b).map(|b| b.rect) {
                    self.dirty = self.dirty.union(&r);
                }
                self.button = b;
            }
        }
        let dirty = self.dirty.intersect(&self.fb.rect());
        self.dirty = Rect::default();
        if dirty.is_empty() {
            return;
        }
        if let Some(r) = &self.rain {
            self.fb.copy_from(&r.buf, 0, 0, dirty.intersect(&v));
        } else {
            self.compose_board(dirty);
        }
        if self.set.status_bar {
            let sb = Rect::new(0, v.bottom(), self.win.w, self.win.h - v.bottom());
            if !sb.intersect(&dirty).is_empty() {
                let mut right = format!("Hand {}", self.game.round.max(1));
                if self.game.hearts_broken && self.game.phase == Phase::Playing {
                    right.push_str("   Hearts broken");
                }
                ui::status_bar(&mut self.fb, sb, self.ui, &self.status, &right, dirty);
            }
        }
        if let Some(m) = &self.menu {
            m.draw(&mut self.fb, dirty);
        }
        if let Some((_, d)) = &self.dialog {
            d.draw(&mut self.fb, self.ui, v, dirty);
        }
        if let Err(e) = self.win.present(&self.fb, dirty) {
            eprintln!("hearts-rs: present failed: {e}");
            self.quit = true;
        }
    }

    // ------------------------------------------------------------ main loop

    fn tick(&mut self) {
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        let mut anim = false;
        for s in self.sprites.iter_mut() {
            anim |= s.tick(now);
        }

        while let Ok((g, img)) = self.bg_rx.try_recv() {
            if g == self.bg_gen && img.w == self.view().w && img.h == self.view().h {
                self.bg = img;
                self.invalidate_all();
                if self.rain.is_some() {
                    self.rain_regrab = Some(now);
                }
            }
        }
        if self.bg_due.is_some_and(|t| now >= t) {
            self.start_bg_job();
        }
        if self.cards_due.is_some_and(|t| now >= t) {
            self.cards_due = None;
            if self.set.card_width == 0 {
                let w = self.card_width_for_view();
                self.set_card_width(w);
            }
        }

        if !anim && self.step != Step::Wait && now >= self.next_step && !self.paused() {
            self.step();
        }

        if self.rain_regrab.is_some_and(|t| now >= t) {
            self.rain_regrab = None;
            if self.rain.as_ref().is_some_and(|r| r.finished) {
                self.rain = None;
            } else if self.rain.is_some() {
                let snap = self.board_snapshot();
                let q = self.rain_queue();
                self.rain.as_mut().unwrap().regrab(snap, q);
            }
            self.invalidate_all();
        }
        if self.rain_regrab.is_none()
            && let Some(r) = &mut self.rain
        {
            let d = r.update(dt, &self.cards);
            self.dirty = self.dirty.union(&d);
        }
    }

    /// Menus and dialogs (other than the end-of-hand sheet) hold the game.
    fn paused(&self) -> bool {
        self.menu.is_some()
            || self
                .dialog
                .as_ref()
                .is_some_and(|(k, _)| *k != DialogKind::HandOver)
    }

    fn timeout_ms(&self) -> i32 {
        let busy = self.animating() || self.rain.as_ref().is_some_and(|r| !r.finished);
        if busy {
            // Animation runs off the compositor's frame callbacks; the
            // timer only covers frames that had nothing to show.
            return if self.win.frame_pending() {
                100
            } else {
                FRAME_MS as i32
            };
        }
        let now = Instant::now();
        let mut t = i32::MAX;
        let mut due =
            |at: Instant| t = t.min(at.saturating_duration_since(now).as_millis() as i32 + 1);
        if self.step != Step::Wait && !self.paused() {
            due(self.next_step);
        }
        if let Some(c) = self.cards_due {
            due(c);
        }
        if let Some(b) = self.bg_due {
            due(b);
        }
        if let Some(r) = self.rain_regrab {
            due(r);
        }
        if t == i32::MAX { -1 } else { t }
    }
}

/// Windows' dotted keyboard focus rectangle.
fn focus_rect(fb: &mut Image, r: Rect, s: i32, clip: Rect) {
    let mut i = 0;
    let dot = |fb: &mut Image, x: i32, y: i32, i: i32| {
        let c = if i % 2 == 0 { ui::BLACK } else { ui::WHITE };
        fb.fill(Rect::new(x, y, s, s).intersect(&clip), c);
    };
    let mut x = r.x;
    while x < r.right() {
        dot(fb, x, r.y, i);
        dot(fb, x, r.bottom() - s, i);
        x += s;
        i += 1;
    }
    let mut y = r.y;
    i = 0;
    while y < r.bottom() {
        dot(fb, r.x, y, i);
        dot(fb, r.right() - s, y, i);
        y += s;
        i += 1;
    }
}

fn usage() -> ! {
    println!(
        "hearts-rs {} - Hearts\n\nUsage: hearts-rs [--easy | --medium | --hard]\n\nCard decks are read from $XDG_DATA_DIRS/carddecks (KDE decks), backgrounds\nfrom $XDG_DATA_DIRS/kpat/themes. Settings live in ~/.config/hearts-rsrc.",
        env!("CARGO_PKG_VERSION")
    );
    std::process::exit(0)
}

fn main() {
    let mut set = Settings::load();
    for a in std::env::args().skip(1) {
        match a.as_str() {
            "--easy" => set.difficulty = 0,
            "--medium" => set.difficulty = 1,
            "--hard" => set.difficulty = 2,
            "--version" | "-V" => {
                println!("hearts-rs {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            _ => usage(),
        }
    }

    let decks = assets::find_decks();
    let themes = assets::find_themes();
    // Requested deck/theme, falling back to built-ins if they won't load.
    let mut deck_err = None;
    let deck = find_index(&decks, &set.deck)
        .and_then(|i| Deck::load(&decks[i]).map_err(|e| deck_err = Some(e)).ok())
        .unwrap_or_else(|| {
            set.deck = assets::BUILTIN.into();
            Deck::load(&decks[find_index(&decks, assets::BUILTIN).unwrap()]).expect("built-in deck")
        });
    let theme = find_index(&themes, &set.theme)
        .and_then(|i| Theme::load(&themes[i]).ok())
        .or_else(|| find_index(&themes, "greenblaze").and_then(|i| Theme::load(&themes[i]).ok()))
        .unwrap_or_else(|| {
            Theme::load(&themes[find_index(&themes, assets::BUILTIN).unwrap()])
                .expect("built-in theme")
        });

    let (w, h) = if set.win_w > 0 && set.win_h > 0 {
        (set.win_w, set.win_h)
    } else {
        (1000, 750)
    };
    let win = match Window::open("Hearts", w, h) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("hearts-rs: cannot open window: {e}");
            std::process::exit(1);
        }
    };
    let (bg_tx, bg_rx) = mpsc::channel();
    let ui_scale = win.dpi_scale;
    let rules = set.rules;
    let mut app = App {
        fb: Image::new(win.w, win.h),
        win,
        set,
        decks,
        themes,
        deck,
        theme,
        cards: Vec::new(),
        cw: 0,
        ch: 0,
        bg: Image::default(),
        bg_due: None,
        cards_due: None,
        bg_gen: 0,
        bg_tx,
        bg_rx,
        ui: ui_scale,
        game: Game::new(rules),
        undo: Vec::new(),
        redo: Vec::new(),
        selected: Vec::new(),
        focus: None,
        hint: None,
        sprites: [Sprite::default(); 52],
        owner: [u8::MAX; 52],
        lay: Layout::default(),
        step: Step::Wait,
        next_step: Instant::now(),
        demo: false,
        status: String::new(),
        rain: None,
        rain_regrab: None,
        last_frame: Instant::now(),
        menu: None,
        dialog: None,
        button_down: false,
        dirty: Rect::default(),
        drawn: [Drawn::default(); 52],
        labels: Vec::new(),
        button: None,
        last_press: (0, -100, -100),
        mouse: (0, 0),
        rng: Rng::new(),
        quit: false,
    };
    // Match the theme setting to what actually loaded.
    if find_index(&app.themes, &app.set.theme).is_none() {
        app.set.theme = if find_index(&app.themes, "greenblaze").is_some() {
            "greenblaze".into()
        } else {
            assets::BUILTIN.into()
        };
    }

    app.cw = app.card_width_for_view();
    app.render_cards();
    app.render_bg_now();
    let v = app.view();
    app.lay = Layout::new(v.w, v.h, app.cw, app.ch, app.label_h());
    app.new_game();
    if let Some(e) = deck_err {
        app.status = format!("Card deck failed to load ({e}); using the built-in deck.");
    }

    while !app.quit {
        loop {
            match app.win.poll() {
                Ok(Some(ev)) => match ev {
                    Ev::Expose => app.invalidate_all(),
                    Ev::Resize => app.on_resize(),
                    Ev::Press {
                        button,
                        x,
                        y,
                        time,
                        mods,
                    } => app.on_press(button, x, y, time, mods),
                    Ev::Release { button, x, y } => app.on_release(button, x, y),
                    Ev::Motion { x, y } => app.on_motion(x, y),
                    Ev::Key { sym, mods } => app.on_key(sym, mods),
                    Ev::Leave => {}
                    Ev::Close => app.quit = true,
                },
                Ok(None) => break,
                Err(e) => {
                    eprintln!("hearts-rs: Wayland connection lost: {e}");
                    app.quit = true;
                    break;
                }
            }
        }
        if app.quit {
            break;
        }
        app.tick();
        if !app.win.frame_pending() {
            app.render();
        }
        let t = app.timeout_ms();
        app.win.wait(t);
    }
    if !app.win.fullscreen {
        app.set.win_w = app.win.w;
        app.set.win_h = app.win.h;
    }
    app.set.save();
}
