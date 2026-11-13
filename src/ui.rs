//! Windows 95 style chrome drawn by hand: popup menus, a status bar and
//! small dialogs, in the built-in bitmap font.

use crate::font;
use crate::gfx::{Image, Rect, text_width, text_width_bold};

const ROW: i32 = 16;

pub const FACE: u32 = 0xffc0c0c0;
pub const NAVY: u32 = 0xff000080;
pub const WHITE: u32 = 0xffffffff;
pub const BLACK: u32 = 0xff000000;
pub const GRAY: u32 = 0xff808080;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cmd {
    NewGame,
    Undo,
    Redo,
    Hint,
    Demo,
    ScoreSheet,
    Statistics,
    ResetStats,
    Difficulty(u8),
    Speed(u8),
    EndScore(i32),
    ToggleQueenBreaks,
    ToggleExactReset,
    ToggleMoonProtection,
    ToggleFullPolish,
    ToggleStatus,
    Deck(usize),
    Theme(usize),
    ZoomIn,
    ZoomOut,
    ZoomFit,
    Fullscreen,
    Help,
    Quit,
}

#[derive(Clone)]
pub enum Item {
    Cmd {
        label: String,
        key: &'static str,
        cmd: Cmd,
        check: Option<bool>,
        enabled: bool,
    },
    Sub {
        label: String,
        items: Vec<Item>,
    },
    Sep,
}

impl Item {
    pub fn cmd(label: &str, key: &'static str, cmd: Cmd) -> Item {
        Item::Cmd {
            label: label.into(),
            key,
            cmd,
            check: None,
            enabled: true,
        }
    }
    pub fn check(label: &str, key: &'static str, cmd: Cmd, on: bool) -> Item {
        Item::Cmd {
            label: label.into(),
            key,
            cmd,
            check: Some(on),
            enabled: true,
        }
    }
    pub fn enabled(mut self, e: bool) -> Item {
        if let Item::Cmd { enabled, .. } = &mut self {
            *enabled = e;
        }
        self
    }
}

struct Level {
    items: Vec<Item>,
    rect: Rect,
    hover: Option<usize>,
}

pub struct Menu {
    levels: Vec<Level>,
    s: i32,
}

impl Menu {
    fn row_h(&self) -> i32 {
        ROW * self.s
    }
    fn sep_h(&self) -> i32 {
        6 * self.s
    }

    fn measure(items: &[Item], s: i32) -> (i32, i32) {
        let mut w = 0;
        let mut h = 0;
        for it in items {
            match it {
                Item::Cmd { label, key, .. } => {
                    w = w.max(text_width(label, s) + text_width(key, s) + 40 * s);
                    h += ROW * s;
                }
                Item::Sub { label, .. } => {
                    w = w.max(text_width(label, s) + 40 * s);
                    h += ROW * s;
                }
                Item::Sep => h += 6 * s,
            }
        }
        (w.max(80 * s), h + 6 * s)
    }

    fn place(items: Vec<Item>, x: i32, y: i32, alt_x: i32, s: i32, bounds: Rect) -> Level {
        let (w, h) = Self::measure(&items, s);
        let x = if x + w > bounds.right() {
            (alt_x - w).max(0)
        } else {
            x
        };
        let y = y.min(bounds.bottom() - h).max(0);
        Level {
            items,
            rect: Rect::new(x, y, w, h),
            hover: None,
        }
    }

    pub fn open(items: Vec<Item>, x: i32, y: i32, s: i32, bounds: Rect) -> Menu {
        Menu {
            levels: vec![Self::place(items, x, y, x, s, bounds)],
            s,
        }
    }

    pub fn bounds(&self) -> Rect {
        self.levels
            .iter()
            .fold(Rect::default(), |a, l| a.union(&l.rect))
    }

    fn item_at(&self, li: usize, x: i32, y: i32) -> Option<usize> {
        let l = &self.levels[li];
        if !l.rect.contains(x, y) {
            return None;
        }
        let mut yy = l.rect.y + 3 * self.s;
        for (i, it) in l.items.iter().enumerate() {
            let h = if matches!(it, Item::Sep) {
                self.sep_h()
            } else {
                self.row_h()
            };
            if y >= yy && y < yy + h {
                return (!matches!(it, Item::Sep)).then_some(i);
            }
            yy += h;
        }
        None
    }

    fn item_y(&self, li: usize, idx: usize) -> i32 {
        let l = &self.levels[li];
        let mut yy = l.rect.y + 3 * self.s;
        for it in &l.items[..idx] {
            yy += if matches!(it, Item::Sep) {
                self.sep_h()
            } else {
                self.row_h()
            };
        }
        yy
    }

    /// Track the pointer; opens submenus on hover. Returns true if the
    /// menu changed and needs repainting.
    pub fn hover(&mut self, x: i32, y: i32, bounds: Rect) -> bool {
        let Some(li) = (0..self.levels.len())
            .rev()
            .find(|&i| self.levels[i].rect.contains(x, y))
        else {
            // Outside: keep open submenus, but drop the highlight of the
            // deepest level.
            let last = self.levels.last_mut().unwrap();
            let changed = last.hover.is_some();
            last.hover = None;
            return changed;
        };
        let idx = self.item_at(li, x, y);
        if self.levels[li].hover == idx && self.levels.len() == li + 1 + self.sub_open(li) as usize
        {
            return false;
        }
        self.levels.truncate(li + 1);
        self.levels[li].hover = idx;
        if let Some(i) = idx
            && let Item::Sub { items, .. } = &self.levels[li].items[i]
        {
            let r = self.levels[li].rect;
            let y = self.item_y(li, i) - 3 * self.s;
            let lvl = Self::place(
                items.clone(),
                r.right() - 3 * self.s,
                y,
                r.x + 3 * self.s,
                self.s,
                bounds,
            );
            self.levels.push(lvl);
        }
        true
    }

    fn sub_open(&self, li: usize) -> bool {
        self.levels.len() > li + 1
    }

    /// Click: Some(Some(cmd)) to run, Some(None) to keep the menu open,
    /// None to close it.
    pub fn click(&mut self, x: i32, y: i32, bounds: Rect) -> Option<Option<Cmd>> {
        let li = (0..self.levels.len())
            .rev()
            .find(|&i| self.levels[i].rect.contains(x, y))?;
        match self.item_at(li, x, y).map(|i| &self.levels[li].items[i]) {
            Some(Item::Cmd {
                cmd, enabled: true, ..
            }) => Some(Some(*cmd)),
            Some(Item::Sub { .. }) => {
                self.hover(x, y, bounds);
                Some(None)
            }
            _ => Some(None),
        }
    }

    pub fn draw(&self, fb: &mut Image, clip: Rect) {
        let s = self.s;
        for l in &self.levels {
            if l.rect.intersect(&clip).is_empty() {
                continue;
            }
            fb.fill(l.rect.intersect(&clip), FACE);
            fb.bevel(l.rect, s, true);
            let mut y = l.rect.y + 3 * s;
            for (i, it) in l.items.iter().enumerate() {
                match it {
                    Item::Sep => {
                        let yy = y + 2 * s;
                        fb.fill(
                            Rect::new(l.rect.x + 4 * s, yy, l.rect.w - 8 * s, s).intersect(&clip),
                            GRAY,
                        );
                        fb.fill(
                            Rect::new(l.rect.x + 4 * s, yy + s, l.rect.w - 8 * s, s)
                                .intersect(&clip),
                            WHITE,
                        );
                        y += self.sep_h();
                    }
                    _ => {
                        let (label, key, check, enabled, sub) = match it {
                            Item::Cmd {
                                label,
                                key,
                                check,
                                enabled,
                                ..
                            } => (label.as_str(), *key, *check, *enabled, false),
                            Item::Sub { label, .. } => (label.as_str(), "", None, true, true),
                            Item::Sep => unreachable!(),
                        };
                        let row = Rect::new(l.rect.x + 3 * s, y, l.rect.w - 6 * s, self.row_h());
                        let hot = l.hover == Some(i);
                        if hot {
                            fb.fill(row.intersect(&clip), NAVY);
                        }
                        let fg = if !enabled {
                            GRAY
                        } else if hot {
                            WHITE
                        } else {
                            BLACK
                        };
                        let ty = y + s;
                        if !enabled && !hot {
                            fb.text(label, row.x + 16 * s + s, ty + s, s, WHITE, clip);
                        }
                        fb.text(label, row.x + 16 * s, ty, s, fg, clip);
                        if !key.is_empty() {
                            fb.text(
                                key,
                                row.right() - text_width(key, s) - 8 * s,
                                ty,
                                s,
                                fg,
                                clip,
                            );
                        }
                        if check == Some(true) {
                            check_mark(fb, row.x + 4 * s, y + 4 * s, s, fg, clip);
                        }
                        if sub {
                            arrow(fb, row.right() - 9 * s, y + 5 * s, s, fg, clip);
                        }
                        y += self.row_h();
                    }
                }
            }
        }
    }
}

fn check_mark(fb: &mut Image, x: i32, y: i32, s: i32, c: u32, clip: Rect) {
    for (dx, dy) in [(0, 3), (1, 4), (2, 5), (3, 4), (4, 3), (5, 2), (6, 1)] {
        fb.fill(
            Rect::new(x + dx * s, y + dy * s, s, 2 * s).intersect(&clip),
            c,
        );
    }
}

fn arrow(fb: &mut Image, x: i32, y: i32, s: i32, c: u32, clip: Rect) {
    for i in 0..4 {
        fb.fill(
            Rect::new(x + i * s, y + i * s, s, (7 - 2 * i) * s).intersect(&clip),
            c,
        );
    }
}

/// A Win95 dialog: title bar, lines of text and an optional OK button.
/// Tabs split a line into columns shared by all lines; in a `table` the
/// columns after the first are right-aligned.
pub struct Dialog {
    pub title: String,
    pub lines: Vec<String>,
    pub table: bool,
    pub button: Option<&'static str>,
}

impl Dialog {
    pub fn new(title: &str, lines: Vec<String>) -> Dialog {
        Dialog {
            title: title.into(),
            lines,
            table: false,
            button: Some("OK"),
        }
    }

    fn columns(&self, s: i32) -> Vec<i32> {
        let mut w: Vec<i32> = Vec::new();
        for l in &self.lines {
            if !l.contains('\t') {
                continue;
            }
            for (i, cell) in l.split('\t').enumerate() {
                let cw = text_width(cell, s);
                if i >= w.len() {
                    w.push(cw);
                } else {
                    w[i] = w[i].max(cw);
                }
            }
        }
        w
    }

    fn gap(&self, s: i32) -> i32 {
        if self.table { 16 * s } else { 12 * s }
    }

    pub fn rect(&self, s: i32, view: Rect) -> Rect {
        let mut w = text_width_bold(&self.title, s) + 40 * s;
        let cols = self.columns(s);
        let table_w: i32 = cols.iter().sum::<i32>() + self.gap(s) * (cols.len() as i32 - 1).max(0);
        for l in &self.lines {
            let lw = if l.contains('\t') { table_w } else { text_width(l, s) };
            w = w.max(lw + 24 * s);
        }
        w = w.max(160 * s);
        let h = 22 * s
            + self.lines.len() as i32 * font::HEIGHT * s
            + if self.button.is_some() { 31 * s } else { 0 }
            + 8 * s;
        Rect::new(view.x + (view.w - w) / 2, view.y + (view.h - h) / 2, w, h)
    }

    pub fn button_rect(&self, s: i32, view: Rect) -> Option<Rect> {
        let label = self.button?;
        let r = self.rect(s, view);
        let w = (text_width(label, s) + 24 * s).max(75 * s);
        let h = 23 * s;
        Some(Rect::new(r.x + (r.w - w) / 2, r.bottom() - h - 9 * s, w, h))
    }

    pub fn draw(&self, fb: &mut Image, s: i32, view: Rect, clip: Rect) {
        let r = self.rect(s, view);
        let cols = self.columns(s);
        fb.fill(r.intersect(&clip), FACE);
        fb.bevel(r, s, true);
        let title = Rect::new(r.x + 3 * s, r.y + 3 * s, r.w - 6 * s, 14 * s);
        fb.fill(title.intersect(&clip), NAVY);
        fb.text_bold(&self.title, title.x + 3 * s, title.y, s, WHITE, clip);
        let mut y = r.y + 22 * s;
        for l in &self.lines {
            if l.contains('\t') {
                let mut x = r.x + 12 * s;
                for (i, cell) in l.split('\t').enumerate() {
                    let cw = cols.get(i).copied().unwrap_or(0);
                    let tx = if self.table && i > 0 {
                        x + cw - text_width(cell, s)
                    } else {
                        x
                    };
                    fb.text(cell, tx, y, s, BLACK, clip);
                    x += cw + self.gap(s);
                }
            } else if l == "---" {
                let yy = y + font::HEIGHT * s / 2;
                fb.fill(Rect::new(r.x + 10 * s, yy, r.w - 20 * s, s).intersect(&clip), GRAY);
                fb.fill(Rect::new(r.x + 10 * s, yy + s, r.w - 20 * s, s).intersect(&clip), WHITE);
            } else {
                fb.text(l, r.x + 12 * s, y, s, BLACK, clip);
            }
            y += font::HEIGHT * s;
        }
        if let (Some(label), Some(b)) = (self.button, self.button_rect(s, view)) {
            button(fb, b, s, label, true, false, true, clip);
        }
    }
}

/// A Windows 95 push button.
#[allow(clippy::too_many_arguments)]
pub fn button(
    fb: &mut Image,
    r: Rect,
    s: i32,
    label: &str,
    enabled: bool,
    pressed: bool,
    default: bool,
    clip: Rect,
) {
    fb.fill(r.intersect(&clip), FACE);
    let mut inner = r;
    if default {
        // The default button has an extra black frame.
        for e in [
            Rect::new(r.x, r.y, r.w, s),
            Rect::new(r.x, r.bottom() - s, r.w, s),
            Rect::new(r.x, r.y, s, r.h),
            Rect::new(r.right() - s, r.y, s, r.h),
        ] {
            fb.fill(e.intersect(&clip), BLACK);
        }
        inner = r.inflate(-s);
    }
    if pressed {
        for e in [
            Rect::new(inner.x, inner.y, inner.w, s),
            Rect::new(inner.x, inner.y, s, inner.h),
        ] {
            fb.fill(e.intersect(&clip), GRAY);
        }
    } else {
        fb.bevel(inner, s, true);
    }
    let off = if pressed { s } else { 0 };
    let tx = r.x + (r.w - text_width(label, s)) / 2 + off;
    let ty = r.y + (r.h - font::HEIGHT * s) / 2 + off;
    if enabled {
        fb.text(label, tx, ty, s, BLACK, clip);
    } else {
        fb.text(label, tx + s, ty + s, s, WHITE, clip);
        fb.text(label, tx, ty, s, GRAY, clip);
    }
}

pub fn status_bar(fb: &mut Image, r: Rect, s: i32, left: &str, right: &str, clip: Rect) {
    fb.fill(r.intersect(&clip), FACE);
    fb.fill(Rect::new(r.x, r.y, r.w, s).intersect(&clip), WHITE);
    let rw = text_width(right, s) + 12 * s;
    let lbox = Rect::new(r.x + 2 * s, r.y + 3 * s, r.w - rw - 6 * s, r.h - 5 * s);
    let rbox = Rect::new(r.right() - rw - 2 * s, r.y + 3 * s, rw, r.h - 5 * s);
    fb.bevel(lbox, s, false);
    fb.fill(lbox.inflate(-2 * s).intersect(&clip), FACE);
    fb.bevel(rbox, s, false);
    fb.fill(rbox.inflate(-2 * s).intersect(&clip), FACE);
    let ty = r.y + 3 * s;
    fb.text(left, lbox.x + 5 * s, ty, s, BLACK, clip.intersect(&lbox));
    fb.text(right, rbox.x + 6 * s, ty, s, BLACK, clip.intersect(&rbox));
}

pub fn status_height(s: i32) -> i32 {
    20 * s
}
