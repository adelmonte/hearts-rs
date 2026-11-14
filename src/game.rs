//! Hearts rules and game state. Cards use the deck's numbering: suit * 13
//! + rank - 1, suits club, diamond, heart, spade, ace = rank 1.

use crate::ai::{self, Ctx, Difficulty, Memory};
use crate::rain::Rng;

pub const CLUBS: u8 = 0;
pub const DIAMONDS: u8 = 1;
pub const HEARTS: u8 = 2;
pub const SPADES: u8 = 3;

pub const SOUTH: usize = 0;
pub const NPLAYERS: usize = 4;

pub const TWO_CLUBS: u8 = 1;
pub const QUEEN_SPADES: u8 = SPADES * 13 + 11;

pub fn suit(c: u8) -> u8 {
    c / 13
}

/// 2..=14, ace high.
pub fn rank(c: u8) -> u8 {
    match c % 13 {
        0 => 14,
        r => r + 1,
    }
}

pub fn card(suit: u8, rank: u8) -> u8 {
    suit * 13 + if rank == 14 { 0 } else { rank - 1 }
}

pub fn points(c: u8) -> i32 {
    if c == QUEEN_SPADES {
        13
    } else if suit(c) == HEARTS {
        1
    } else {
        0
    }
}

pub fn is_point(c: u8) -> bool {
    points(c) > 0
}

pub fn name(c: u8) -> String {
    let r = match rank(c) {
        11 => "jack".to_string(),
        12 => "queen".to_string(),
        13 => "king".to_string(),
        14 => "ace".to_string(),
        n => n.to_string(),
    };
    let s = ["clubs", "diamonds", "hearts", "spades"][suit(c) as usize];
    format!("{r} of {s}")
}

/// Hands are kept in this order: clubs, diamonds, spades, hearts, each
/// low to high.
fn sort_key(c: u8) -> (u8, u8) {
    let s = [0, 1, 3, 2][suit(c) as usize];
    (s, rank(c))
}

pub fn sort_hand(h: &mut [u8]) {
    h.sort_by_key(|&c| sort_key(c));
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pass {
    Left,
    Right,
    Across,
    Hold,
}

impl Pass {
    pub fn for_round(round: u32) -> Pass {
        [Pass::Left, Pass::Right, Pass::Across, Pass::Hold][(round.max(1) as usize - 1) % 4]
    }
    pub fn target(self, from: usize) -> usize {
        match self {
            Pass::Left => (from + 1) % NPLAYERS,
            Pass::Right => (from + 3) % NPLAYERS,
            Pass::Across => (from + 2) % NPLAYERS,
            Pass::Hold => from,
        }
    }
    pub fn word(self) -> &'static str {
        match self {
            Pass::Left => "Left",
            Pass::Right => "Right",
            Pass::Across => "Across",
            Pass::Hold => "",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rules {
    pub end_score: i32,
    /// Landing exactly on the end score resets you to 50.
    pub exact_reset: bool,
    pub queen_breaks_hearts: bool,
    /// A moon shooter takes -26 instead when giving +26 would lose them the game.
    pub moon_protection: bool,
    /// 99 points and taking 25 in a hand resets you to 98.
    pub full_polish: bool,
}

impl Default for Rules {
    fn default() -> Self {
        Rules {
            end_score: 100,
            exact_reset: false,
            queen_breaks_hearts: true,
            moon_protection: false,
            full_polish: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Choosing three cards to pass.
    Passing,
    /// Cards passed; the received ones are shown until the player is ready.
    Received,
    /// Hold hand, or passing over: waiting for the first lead.
    Ready,
    Playing,
    RoundOver,
    GameOver,
}

#[derive(Clone, Debug)]
pub struct Hand {
    pub points: [i32; NPLAYERS],
    pub moon: Option<usize>,
}

#[derive(Clone)]
pub struct Game {
    pub rules: Rules,
    pub round: u32,
    pub phase: Phase,
    pub hands: [Vec<u8>; NPLAYERS],
    pub round_pts: [i32; NPLAYERS],
    pub total: [i32; NPLAYERS],
    pub sheet: Vec<Hand>,
    pub trick: Vec<(usize, u8)>,
    pub turn: usize,
    pub hearts_broken: bool,
    pub first_trick: bool,
    pub mem: Memory,
    /// Who took each card this hand.
    pub taken_by: [Option<usize>; 52],
    /// Cards each player received in the pass.
    pub received: [Vec<u8>; NPLAYERS],
}

pub enum Played {
    Next,
    TrickFull,
}

impl Game {
    pub fn new(rules: Rules) -> Game {
        Game {
            rules,
            round: 0,
            phase: Phase::Ready,
            hands: Default::default(),
            round_pts: [0; NPLAYERS],
            total: [0; NPLAYERS],
            sheet: Vec::new(),
            trick: Vec::new(),
            turn: 0,
            hearts_broken: false,
            first_trick: true,
            mem: Memory::default(),
            taken_by: [None; 52],
            received: Default::default(),
        }
    }

    pub fn pass_dir(&self) -> Pass {
        Pass::for_round(self.round)
    }

    pub fn deal(&mut self, rng: &mut Rng) {
        self.round += 1;
        let mut deck: Vec<u8> = (0..52).collect();
        for i in (1..deck.len()).rev() {
            let j = rng.below(i as u64 + 1) as usize;
            deck.swap(i, j);
        }
        for (p, h) in self.hands.iter_mut().enumerate() {
            *h = deck[p * 13..(p + 1) * 13].to_vec();
            sort_hand(h);
        }
        self.round_pts = [0; NPLAYERS];
        self.trick.clear();
        self.hearts_broken = false;
        self.first_trick = true;
        self.mem = Memory::default();
        self.taken_by = [None; 52];
        self.received = Default::default();
        self.phase = if self.pass_dir() == Pass::Hold {
            Phase::Ready
        } else {
            Phase::Passing
        };
    }

    pub fn ctx(&self, diff: Difficulty) -> Ctx {
        Ctx {
            totals: self.total,
            round: self.round_pts,
            moon_protection: self.rules.moon_protection,
            difficulty: diff,
        }
    }

    /// Exchange cards: South gives `mine`, the others choose their own.
    pub fn pass(&mut self, mine: &[u8], diff: Difficulty, rng: &mut Rng) {
        let dir = self.pass_dir();
        let ctx = self.ctx(diff);
        let mut give: [Vec<u8>; NPLAYERS] = Default::default();
        give[SOUTH] = mine.to_vec();
        for (p, g) in give.iter_mut().enumerate().skip(1) {
            *g = ai::choose_pass(&self.hands[p], p, &ctx, rng);
        }
        for (p, g) in give.iter().enumerate() {
            self.hands[p].retain(|c| !g.contains(c));
        }
        for (p, g) in give.into_iter().enumerate() {
            let to = dir.target(p);
            self.hands[to].extend_from_slice(&g);
            sort_hand(&mut self.hands[to]);
            self.received[to] = g;
        }
        self.phase = Phase::Received;
    }

    pub fn start_play(&mut self) {
        self.phase = Phase::Playing;
        self.turn = (0..NPLAYERS)
            .find(|&p| self.hands[p].contains(&TWO_CLUBS))
            .unwrap_or(SOUTH);
    }

    pub fn lead_suit(&self) -> Option<u8> {
        self.trick.first().map(|&(_, c)| suit(c))
    }

    pub fn trick_full(&self) -> bool {
        self.trick.len() == NPLAYERS
    }

    /// Cards player `p` may play now, in hand order.
    pub fn valid(&self, p: usize) -> Vec<u8> {
        let hand = &self.hands[p];
        let Some(lead) = self.lead_suit() else {
            if self.first_trick && hand.contains(&TWO_CLUBS) {
                return vec![TWO_CLUBS];
            }
            if !self.hearts_broken && hand.iter().any(|&c| suit(c) != HEARTS) {
                return hand.iter().copied().filter(|&c| suit(c) != HEARTS).collect();
            }
            return hand.clone();
        };
        let follow: Vec<u8> = hand.iter().copied().filter(|&c| suit(c) == lead).collect();
        if !follow.is_empty() {
            return follow;
        }
        if self.first_trick {
            let safe: Vec<u8> = hand.iter().copied().filter(|&c| !is_point(c)).collect();
            if !safe.is_empty() {
                return safe;
            }
        }
        hand.clone()
    }

    /// Why South can't play `c` right now.
    pub fn why_not(&self, c: u8) -> &'static str {
        match self.lead_suit() {
            None if self.first_trick => "You must lead the two of clubs.",
            None => "Hearts have not been broken yet.",
            Some(l) if self.hands[SOUTH].iter().any(|&x| suit(x) == l) && suit(c) != l => {
                "You must follow suit."
            }
            Some(_) => "You can't play points on the first trick.",
        }
    }

    pub fn play(&mut self, c: u8) -> Played {
        let p = self.turn;
        self.hands[p].retain(|&x| x != c);
        self.trick.push((p, c));
        self.mem.record(c);
        if suit(c) == HEARTS || (c == QUEEN_SPADES && self.rules.queen_breaks_hearts) {
            self.hearts_broken = true;
        }
        if self.trick_full() {
            Played::TrickFull
        } else {
            self.turn = (p + 1) % NPLAYERS;
            Played::Next
        }
    }

    pub fn trick_winner(&self) -> usize {
        let lead = self.lead_suit().unwrap_or(CLUBS);
        self.trick
            .iter()
            .filter(|&&(_, c)| suit(c) == lead)
            .max_by_key(|&&(_, c)| rank(c))
            .map_or(self.turn, |&(p, _)| p)
    }

    /// Take the full trick; returns the winner and the points in it.
    pub fn collect(&mut self) -> (usize, i32) {
        let w = self.trick_winner();
        let pts: i32 = self.trick.iter().map(|&(_, c)| points(c)).sum();
        for &(_, c) in &self.trick {
            self.taken_by[c as usize] = Some(w);
        }
        self.round_pts[w] += pts;
        self.trick.clear();
        self.first_trick = false;
        self.turn = w;
        if self.hands.iter().all(|h| h.is_empty()) {
            self.phase = Phase::RoundOver;
        }
        (w, pts)
    }

    /// Score the finished hand. Returns true when the game is over.
    pub fn end_round(&mut self) -> bool {
        let mut pts = self.round_pts;
        if self.rules.full_polish {
            for p in 0..NPLAYERS {
                if self.total[p] == 99 && pts[p] == 25 {
                    pts[p] = -1;
                }
            }
        }
        let moon = (0..NPLAYERS).find(|&p| self.round_pts[p] == 26);
        if let Some(m) = moon {
            let others_lose = {
                let after: Vec<i32> = (0..NPLAYERS)
                    .filter(|&p| p != m)
                    .map(|p| self.total[p] + 26)
                    .collect();
                after.iter().any(|&t| t >= self.rules.end_score)
                    && after.iter().any(|&t| t < self.total[m])
            };
            if self.rules.moon_protection && others_lose {
                pts = [0; NPLAYERS];
                pts[m] = -26;
            } else {
                pts = [26; NPLAYERS];
                pts[m] = 0;
            }
        }
        for p in 0..NPLAYERS {
            self.total[p] += pts[p];
            if self.rules.exact_reset && self.total[p] == self.rules.end_score {
                self.total[p] = 50;
            }
        }
        self.sheet.push(Hand { points: pts, moon });
        self.round_pts = [0; NPLAYERS];
        let over = self.total.iter().any(|&t| t >= self.rules.end_score);
        self.phase = if over {
            Phase::GameOver
        } else {
            Phase::RoundOver
        };
        over
    }

    /// Lowest score wins; ties go to the first seat from South.
    pub fn winner(&self) -> usize {
        (0..NPLAYERS).min_by_key(|&p| self.total[p]).unwrap_or(SOUTH)
    }

    pub fn ai_play(&self, diff: Difficulty, rng: &mut Rng) -> u8 {
        let p = self.turn;
        let valid = self.valid(p);
        ai::choose_play(
            &self.hands[p],
            &valid,
            p,
            &self.trick,
            self.hearts_broken,
            &self.mem,
            &self.ctx(diff),
            rng,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn play_game(diff: Difficulty, rules: Rules, seed: u64) -> Game {
        let mut rng = Rng::seeded(seed);
        let mut g = Game::new(rules);
        loop {
            g.deal(&mut rng);
            let all: u64 = g
                .hands
                .iter()
                .flatten()
                .fold(0, |m, &c| m | 1u64 << c);
            assert_eq!(all, (1u64 << 52) - 1);
            if g.phase == Phase::Passing {
                let ctx = g.ctx(diff);
                let mine = ai::choose_pass(&g.hands[SOUTH], SOUTH, &ctx, &mut rng);
                assert_eq!(mine.len(), 3);
                g.pass(&mine, diff, &mut rng);
                assert!(g.hands.iter().all(|h| h.len() == 13));
            }
            g.start_play();
            assert!(g.hands[g.turn].contains(&TWO_CLUBS));
            while g.phase == Phase::Playing {
                let c = g.ai_play(diff, &mut rng);
                assert!(g.valid(g.turn).contains(&c), "illegal {}", name(c));
                if let Played::TrickFull = g.play(c) {
                    g.collect();
                }
            }
            assert_eq!(g.round_pts.iter().sum::<i32>(), 26);
            if g.end_round() {
                return g;
            }
        }
    }

    #[test]
    fn games_finish_legally() {
        for seed in 1..60 {
            for diff in [Difficulty::Easy, Difficulty::Medium, Difficulty::Hard] {
                let g = play_game(diff, Rules::default(), seed);
                assert!(g.total.iter().any(|&t| t >= 100));
                let sum: i32 = g.sheet.iter().map(|h| h.points.iter().sum::<i32>()).sum();
                assert_eq!(sum, g.total.iter().sum::<i32>());
            }
        }
    }

    #[test]
    fn variant_rules_finish() {
        let rules = Rules {
            end_score: 50,
            exact_reset: true,
            queen_breaks_hearts: false,
            moon_protection: true,
            full_polish: true,
        };
        for seed in 1..40 {
            play_game(Difficulty::Hard, rules, seed);
        }
    }

    #[test]
    fn moon_scoring() {
        let mut g = Game::new(Rules::default());
        g.round_pts = [0, 26, 0, 0];
        g.end_round();
        assert_eq!(g.total, [26, 0, 26, 26]);
        let mut g = Game::new(Rules {
            moon_protection: true,
            ..Rules::default()
        });
        g.total = [80, 90, 10, 10];
        g.round_pts = [26, 0, 0, 0];
        g.end_round();
        assert_eq!(g.total, [54, 90, 10, 10]);
    }

    #[test]
    fn leading_rules() {
        let mut g = Game::new(Rules::default());
        g.hands[0] = vec![card(HEARTS, 5), card(CLUBS, 9), card(HEARTS, 9)];
        sort_hand(&mut g.hands[0]);
        g.first_trick = false;
        g.turn = 0;
        assert_eq!(g.valid(0), vec![card(CLUBS, 9)]);
        g.hearts_broken = true;
        assert_eq!(g.valid(0).len(), 3);
    }
}
