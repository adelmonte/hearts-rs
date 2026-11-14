//! Where everything sits on the table, and the animated card sprites.
//! Positions are card centres, so sideways cards (West and East hold
//! theirs turned a quarter) share the same arithmetic.

use crate::gfx::Rect;
use std::time::Instant;

/// South's fan, as a fraction of the card width.
const SPREAD: f32 = 0.28;
/// The other hands are packed tighter.
const SPREAD_OTHERS: f32 = 0.17;
const MARGIN: f32 = 0.12;
const GAP: f32 = 0.22;
/// Passing cards stand this far out of South's hand (of the card height).
pub const RAISE: f32 = 0.22;

/// Card width that fits the whole table into the view.
pub fn fit_width(vw: i32, vh: i32, aspect: f32, label_h: i32) -> i32 {
    let k = 1.0 / aspect;
    let by_w = vw as f32 / (12.0 * SPREAD + 1.0 + 2.0 * k + 2.0 * (MARGIN + GAP) * k);
    let by_h = (vh - 2 * label_h) as f32 / (k * (3.84 + RAISE + 2.0 * MARGIN + 2.0 * GAP));
    (by_w.min(by_h) as i32).clamp(20, 2000)
}

#[derive(Clone, Copy, Default)]
pub struct Layout {
    pub cw: i32,
    pub ch: i32,
    /// Trick centre.
    pub cx: f32,
    pub cy: f32,
    south_y: f32,
    north_y: f32,
    west_x: f32,
    east_x: f32,
    sp: f32,
    osp: f32,
    side_sp: f32,
}

impl Layout {
    pub fn new(vw: i32, vh: i32, cw: i32, ch: i32, label_h: i32) -> Layout {
        let (cwf, chf, vwf, vhf) = (cw as f32, ch as f32, vw as f32, vh as f32);
        let lh = label_h as f32;
        let m = MARGIN * chf;
        let north_y = m + chf / 2.0;
        let south_y = vhf - m - lh - chf / 2.0;
        let north_bottom = north_y + chf / 2.0 + lh;
        let south_top = south_y - chf / 2.0 - RAISE * chf;
        let cy = ((north_bottom + south_top) / 2.0).round();
        let west_x = m + chf / 2.0;
        let east_x = vwf - m - chf / 2.0;
        // Squeeze the fans if the window is too small for them.
        let room = (east_x - west_x - chf - 2.0 * GAP * chf - cwf).max(0.0);
        let sp = (SPREAD * cwf).min(room / 12.0).max(2.0);
        let side_room = (south_top - north_bottom - cwf).max(0.0);
        let side_sp = (SPREAD_OTHERS * cwf).min(side_room / 12.0).max(2.0);
        Layout {
            cw,
            ch,
            cx: (vwf / 2.0).round(),
            cy,
            south_y: south_y.round(),
            north_y: north_y.round(),
            west_x: west_x.round(),
            east_x: east_x.round(),
            sp,
            osp: SPREAD_OTHERS * cwf,
            side_sp,
        }
    }

    pub fn sideways(p: usize) -> bool {
        p == 1 || p == 3
    }

    /// Centre of card `i` of `n` in player `p`'s hand.
    pub fn hand_pos(&self, p: usize, i: usize, n: usize) -> (f32, f32) {
        let k = i as f32 - (n.max(1) - 1) as f32 / 2.0;
        match p {
            0 => ((self.cx + k * self.sp).round(), self.south_y),
            1 => (self.west_x, (self.cy + k * self.side_sp).round()),
            2 => ((self.cx - k * self.osp).round(), self.north_y),
            _ => (self.east_x, (self.cy - k * self.side_sp).round()),
        }
    }

    pub fn raise(&self) -> f32 {
        (RAISE * self.ch as f32).round()
    }

    /// Where player `p`'s card lies in the trick.
    pub fn trick_pos(&self, p: usize) -> (f32, f32) {
        let (dx, dy) = match p {
            0 => (0.0, 0.42 * self.ch as f32),
            1 => (-0.82 * self.cw as f32, 0.0),
            2 => (0.0, -0.42 * self.ch as f32),
            _ => (0.82 * self.cw as f32, 0.0),
        };
        ((self.cx + dx).round(), (self.cy + dy).round())
    }

    pub fn seat(&self, p: usize) -> (f32, f32) {
        self.hand_pos(p, 0, 1)
    }

    /// Name plate anchor: (centre x, top y).
    pub fn label_pos(&self, p: usize) -> (i32, i32) {
        let (cw, ch) = (self.cw as f32, self.ch as f32);
        let (x, y) = match p {
            0 => (self.cx, self.south_y + ch / 2.0 + 2.0),
            1 => (self.west_x, self.cy + (12.0 * self.side_sp + cw) / 2.0 + 2.0),
            2 => (self.cx, self.north_y + ch / 2.0 + 2.0),
            _ => (self.east_x, self.cy + (12.0 * self.side_sp + cw) / 2.0 + 2.0),
        };
        (x as i32, y as i32)
    }

    pub fn rect_at(&self, x: f32, y: f32, side: bool) -> Rect {
        let (w, h) = if side {
            (self.ch, self.cw)
        } else {
            (self.cw, self.ch)
        };
        Rect::new(x.round() as i32 - w / 2, y.round() as i32 - h / 2, w, h)
    }
}

#[derive(Clone, Copy)]
pub struct Anim {
    from: (f32, f32),
    flip_from: f32,
    from_side: bool,
    start: Instant,
    dur: f32,
    turning: bool,
    progress: f32,
    pub raised: bool,
}

#[derive(Clone, Copy)]
pub struct Sprite {
    /// Resting place and face.
    pub tx: f32,
    pub ty: f32,
    pub up: bool,
    pub side: bool,
    /// Taken in a trick: vanishes once it gets there.
    pub gone: bool,
    pub z: i32,
    pub anim: Option<Anim>,
    /// Current displayed position and flippedness (0 = back, 1 = face).
    pub x: f32,
    pub y: f32,
    pub flip: f32,
}

impl Default for Sprite {
    fn default() -> Self {
        Sprite {
            tx: 0.0,
            ty: 0.0,
            up: false,
            side: false,
            gone: false,
            z: 0,
            anim: None,
            x: 0.0,
            y: 0.0,
            flip: 0.0,
        }
    }
}

fn ease(t: f32) -> f32 {
    1.0 - (1.0 - t) * (1.0 - t)
}

pub struct Target {
    pub x: f32,
    pub y: f32,
    pub up: bool,
    pub side: bool,
    pub gone: bool,
}

impl Sprite {
    pub fn same(&self, t: &Target) -> bool {
        self.tx == t.x && self.ty == t.y && self.up == t.up && self.side == t.side && self.gone == t.gone
    }

    /// Move towards a new resting place over `ms` milliseconds, beginning
    /// at `start` (the sprite waits where it is until then).
    pub fn go(&mut self, t: &Target, ms: f32, raised: bool, start: Instant) {
        let from_side = self.shown_side();
        let target_flip = if t.up { 1.0 } else { 0.0 };
        let moving = (t.x - self.x).abs() > 1.0
            || (t.y - self.y).abs() > 1.0
            || self.flip != target_flip
            || from_side != t.side;
        self.tx = t.x;
        self.ty = t.y;
        self.up = t.up;
        self.side = t.side;
        self.gone = t.gone;
        if ms <= 0.0 || !moving {
            self.finish();
            return;
        }
        self.anim = Some(Anim {
            from: (self.x, self.y),
            flip_from: self.flip,
            from_side,
            start,
            dur: ms / 1000.0,
            turning: self.flip != target_flip,
            progress: 0.0,
            raised: raised || self.anim.is_some_and(|a| a.raised),
        });
    }

    /// Advance; returns true while still animating.
    pub fn tick(&mut self, now: Instant) -> bool {
        let Some(a) = &mut self.anim else {
            return false;
        };
        if now < a.start {
            return true;
        }
        let t = (now.saturating_duration_since(a.start).as_secs_f32() / a.dur).min(1.0);
        a.progress = t;
        let e = if a.turning {
            t * t * (3.0 - 2.0 * t)
        } else {
            ease(t)
        };
        let (from, flip_from) = (a.from, a.flip_from);
        self.x = from.0 + (self.tx - from.0) * e;
        self.y = from.1 + (self.ty - from.1) * e;
        let target = if self.up { 1.0 } else { 0.0 };
        self.flip = flip_from + (target - flip_from) * e;
        if t >= 1.0 {
            self.finish();
            return false;
        }
        true
    }

    pub fn finish(&mut self) {
        self.x = self.tx;
        self.y = self.ty;
        self.flip = if self.up { 1.0 } else { 0.0 };
        self.anim = None;
    }

    pub fn raised(&self) -> bool {
        self.anim.is_some_and(|a| a.raised)
    }

    /// Which way the card is turned right now: sideways cards straighten
    /// up (or turn) halfway through a move.
    pub fn shown_side(&self) -> bool {
        match self.anim {
            Some(a) if a.progress < 0.5 => a.from_side,
            _ => self.side,
        }
    }

    pub fn visible(&self) -> bool {
        !(self.gone && self.anim.is_none())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_fits_and_nothing_collides() {
        let label_h = 17;
        for &(vw, vh) in &[(1000, 730), (800, 580), (1920, 1060), (1395, 1683), (640, 460), (2560, 1400)] {
            for aspect in [71.0 / 96.0, 0.69, 0.75] {
                let cw = fit_width(vw, vh, aspect, label_h);
                let ch = (cw as f32 / aspect).round() as i32;
                let l = Layout::new(vw, vh, cw, ch, label_h);
                let view = Rect::new(0, 0, vw, vh);
                let inside = |r: Rect| r.intersect(&view) == r;
                let hand = |p: usize| {
                    (0..13).fold(Rect::default(), |a, i| {
                        let (x, mut y) = l.hand_pos(p, i, 13);
                        if p == 0 {
                            y -= l.raise();
                        }
                        let r = l.rect_at(x, y, Layout::sideways(p));
                        let r = if p == 0 { r.union(&l.rect_at(x, y + l.raise(), false)) } else { r };
                        a.union(&r)
                    })
                };
                let trick = (0..4).fold(Rect::default(), |a, p| {
                    let (x, y) = l.trick_pos(p);
                    a.union(&l.rect_at(x, y, false))
                });
                let ctx = format!("{vw}x{vh} aspect {aspect} cw {cw}");
                assert!(inside(trick), "{ctx}: trick {trick:?}");
                for p in 0..4 {
                    let h = hand(p);
                    assert!(inside(h), "{ctx}: hand {p} {h:?}");
                    assert!(h.intersect(&trick).is_empty(), "{ctx}: hand {p} {h:?} hits trick {trick:?}");
                    let (lx, ly) = l.label_pos(p);
                    let lr = Rect::new(lx - 40, ly, 80, label_h);
                    assert!(inside(lr), "{ctx}: label {p} {lr:?}");
                    assert!(lr.intersect(&trick).is_empty(), "{ctx}: label {p} hits trick");
                    for q in 0..p {
                        assert!(h.intersect(&hand(q)).is_empty(), "{ctx}: hands {p} {q} collide");
                    }
                }
            }
        }
    }
}
