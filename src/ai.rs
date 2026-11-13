//! Computer players, ported from qt-hearts: Easy plays loosely, Medium
//! ducks and dumps danger, Hard counts cards, watches seat position and
//! defends against a moon shot.

use crate::game::{CLUBS, DIAMONDS, HEARTS, NPLAYERS, QUEEN_SPADES, SPADES, card, points, rank, suit};
use crate::rain::Rng;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Difficulty {
    Easy,
    Medium,
    Hard,
}

impl Difficulty {
    pub fn from_index(i: u8) -> Difficulty {
        match i {
            0 => Difficulty::Easy,
            2 => Difficulty::Hard,
            _ => Difficulty::Medium,
        }
    }
    pub fn index(self) -> u8 {
        self as u8
    }
}

/// What everyone at the table has seen this hand.
#[derive(Clone, Default)]
pub struct Memory {
    played: u64,
    /// Bit per suit for each player known to be out of it.
    voids: [u8; NPLAYERS],
    pub queen_played: bool,
    suit_played: [u8; 4],
}

impl Memory {
    pub fn record(&mut self, c: u8, player: usize, lead: u8) {
        self.played |= 1 << c;
        self.suit_played[suit(c) as usize] += 1;
        if c == QUEEN_SPADES {
            self.queen_played = true;
        }
        if suit(c) != lead {
            self.voids[player] |= 1 << lead;
        }
    }
    pub fn is_void(&self, player: usize, s: u8) -> bool {
        self.voids[player] >> s & 1 != 0
    }
    fn count_played(&self, s: u8) -> u8 {
        self.suit_played[s as usize]
    }
}

pub struct Ctx {
    pub totals: [i32; NPLAYERS],
    pub round: [i32; NPLAYERS],
    pub moon_protection: bool,
    pub difficulty: Difficulty,
}

fn of_suit(cards: &[u8], s: u8) -> Vec<u8> {
    cards.iter().copied().filter(|&c| suit(c) == s).collect()
}

fn has_suit(cards: &[u8], s: u8) -> bool {
    cards.iter().any(|&c| suit(c) == s)
}

fn count_suit(cards: &[u8], s: u8) -> usize {
    cards.iter().filter(|&&c| suit(c) == s).count()
}

/// First card with the highest (or lowest) rank, as qt-hearts picks them.
fn highest(cards: &[u8]) -> Option<u8> {
    cards.iter().copied().fold(None, |b, c| match b {
        Some(b) if rank(c) <= rank(b) => Some(b),
        _ => Some(c),
    })
}

fn lowest(cards: &[u8]) -> Option<u8> {
    cards.iter().copied().fold(None, |b, c| match b {
        Some(b) if rank(c) >= rank(b) => Some(b),
        _ => Some(c),
    })
}

fn highest_of(cards: &[u8], s: u8) -> Option<u8> {
    highest(&of_suit(cards, s))
}

fn lowest_of(cards: &[u8], s: u8) -> Option<u8> {
    lowest(&of_suit(cards, s))
}

fn highest_below(cards: &[u8], r: u8) -> Option<u8> {
    let below: Vec<u8> = cards.iter().copied().filter(|&c| rank(c) < r).collect();
    highest(&below)
}

fn pick(cards: &[u8], rng: &mut Rng) -> u8 {
    cards[rng.below(cards.len() as u64) as usize]
}

fn coin(rng: &mut Rng) -> bool {
    rng.below(2) == 0
}

/// Who is winning the trick so far.
fn current_winner(trick: &[(usize, u8)]) -> Option<usize> {
    let lead = suit(trick.first()?.1);
    trick
        .iter()
        .filter(|&&(_, c)| suit(c) == lead)
        .fold(None::<(usize, u8)>, |b, &(p, c)| match b {
            Some((_, bc)) if rank(c) <= rank(bc) => b,
            _ => Some((p, c)),
        })
        .map(|(p, _)| p)
}

// ------------------------------------------------------------------ passing

pub fn choose_pass(hand: &[u8], me: usize, ctx: &Ctx, rng: &mut Rng) -> Vec<u8> {
    let _ = rng;
    if ctx.difficulty == Difficulty::Hard {
        return pass_hard(hand, me, ctx);
    }
    let mut v = hand.to_vec();
    // Most dangerous first: Q♠, A♠/K♠, hearts high to low, then high cards.
    v.sort_by_key(|&c| {
        let high_spade = suit(c) == SPADES && rank(c) >= 13;
        std::cmp::Reverse((c == QUEEN_SPADES, high_spade, suit(c) == HEARTS, rank(c)))
    });
    v.truncate(3);
    v
}

fn pass_hard(hand: &[u8], me: usize, ctx: &Ctx) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    let counts: [usize; 4] = std::array::from_fn(|s| count_suit(hand, s as u8));
    let has = |c: u8| hand.contains(&c);
    let has_q = has(QUEEN_SPADES);
    let has_k = has(card(SPADES, 13));
    let has_a = has(card(SPADES, 14));

    let my = ctx.totals[me];
    let best_other = (0..NPLAYERS)
        .filter(|&p| p != me)
        .map(|p| ctx.totals[p])
        .min()
        .unwrap_or(0);
    let far_behind = my - best_other > 25;
    let hearts = of_suit(hand, HEARTS);
    let high_hearts = hearts.iter().filter(|&&c| rank(c) >= 11).count();
    let moon_hand = (hearts.len() >= 6 && high_hearts >= 3)
        || (hearts.len() >= 5 && high_hearts >= 4 && has_a);

    if far_behind && moon_hand && ctx.moon_protection {
        // Going for the moon: keep the hearts and high cards.
        let mut low: Vec<u8> = hand
            .iter()
            .copied()
            .filter(|&c| rank(c) <= 6 && suit(c) != HEARTS)
            .collect();
        low.sort_by_key(|&c| rank(c));
        out.extend(low.into_iter().take(3));
        if out.len() >= 3 {
            return out;
        }
    }

    let keep_q = has_q && ((has_k && has_a) || counts[SPADES as usize] >= 5);

    let mut shortest = CLUBS;
    let mut shortest_n = 14;
    for s in [CLUBS, DIAMONDS, HEARTS] {
        let n = counts[s as usize];
        if n > 0 && n < shortest_n {
            shortest_n = n;
            shortest = s;
        }
    }

    let mut danger = Vec::new();
    if has_q && !keep_q {
        danger.push(QUEEN_SPADES);
    }
    if !has_q {
        if has_a {
            danger.push(card(SPADES, 14));
        }
        if has_k {
            danger.push(card(SPADES, 13));
        }
    }
    let mut hh = hearts.clone();
    hh.sort_by_key(|&c| std::cmp::Reverse(rank(c)));
    danger.extend(hh.into_iter().filter(|&c| rank(c) >= 12));

    let push = |out: &mut Vec<u8>, c: u8| {
        if out.len() < 3 && !out.contains(&c) {
            out.push(c);
        }
    };
    for c in danger {
        push(&mut out, c);
    }
    // Make a void if what's left of the shortest suit fits.
    if out.len() < 3 && shortest_n <= 3 - out.len() {
        for c in of_suit(hand, shortest) {
            push(&mut out, c);
        }
    }
    if out.len() < 3 {
        let mut rest = hand.to_vec();
        rest.sort_by_key(|&c| {
            let spade_last = keep_q && suit(c) == SPADES;
            (spade_last, std::cmp::Reverse(rank(c)))
        });
        for c in rest {
            push(&mut out, c);
        }
    }
    out
}

// ------------------------------------------------------------------ playing

#[allow(clippy::too_many_arguments)]
pub fn choose_play(
    hand: &[u8],
    valid: &[u8],
    me: usize,
    trick: &[(usize, u8)],
    hearts_broken: bool,
    mem: &Memory,
    ctx: &Ctx,
    rng: &mut Rng,
) -> u8 {
    if valid.len() == 1 {
        return valid[0];
    }
    let trick_cards: Vec<u8> = trick.iter().map(|&(_, c)| c).collect();
    let lead = trick.first().map(|&(_, c)| suit(c));
    let p = Player {
        hand,
        me,
        mem,
        ctx,
    };
    match (ctx.difficulty, lead) {
        (Difficulty::Easy, None) => {
            if coin(rng) {
                pick(valid, rng)
            } else {
                highest(valid).unwrap()
            }
        }
        (Difficulty::Easy, Some(l)) if has_suit(valid, l) => pick(valid, rng),
        (Difficulty::Easy, Some(_)) => {
            if coin(rng) {
                pick(valid, rng)
            } else {
                highest(valid).unwrap()
            }
        }
        (Difficulty::Medium, None) => p.lead_medium(valid, hearts_broken, rng),
        (Difficulty::Medium, Some(l)) if has_suit(valid, l) => p.follow_medium(valid, l, &trick_cards),
        (Difficulty::Medium, Some(_)) => p.slough_medium(valid),
        (Difficulty::Hard, None) => p.lead_hard(valid, hearts_broken),
        (Difficulty::Hard, Some(l)) if has_suit(valid, l) => p.follow_hard(valid, l, trick),
        (Difficulty::Hard, Some(_)) => p.slough_hard(valid, trick),
    }
}

struct Player<'a> {
    hand: &'a [u8],
    me: usize,
    mem: &'a Memory,
    ctx: &'a Ctx,
}

impl Player<'_> {
    fn has(&self, c: u8) -> bool {
        self.hand.contains(&c)
    }

    fn queen_out(&self) -> bool {
        !self.mem.queen_played && !self.has(QUEEN_SPADES)
    }

    /// (my score, best other, worst other), counting this hand so far.
    fn standing(&self) -> (i32, i32, i32, usize) {
        let s = |p: usize| self.ctx.totals[p] + self.ctx.round[p];
        let mut best = i32::MAX;
        let mut worst = i32::MIN;
        let mut leader = 0;
        for p in (0..NPLAYERS).filter(|&p| p != self.me) {
            if s(p) < best {
                best = s(p);
                leader = p;
            }
            worst = worst.max(s(p));
        }
        (s(self.me), best, worst, leader)
    }

    fn moon_shooter(&self) -> Option<usize> {
        (0..NPLAYERS).find(|&p| p != self.me && self.ctx.round[p] >= 13)
    }

    fn lead_medium(&self, valid: &[u8], hearts_broken: bool, rng: &mut Rng) -> u8 {
        if self.queen_out()
            && let Some(ls) = lowest_of(valid, SPADES)
            && rank(ls) < 12
            && rng.below(5) < 2
        {
            return ls;
        }
        for s in [CLUBS, DIAMONDS] {
            if let Some(c) = lowest_of(valid, s) {
                return c;
            }
        }
        if let Some(c) = lowest_of(valid, SPADES).filter(|&c| rank(c) < 12) {
            return c;
        }
        if hearts_broken && let Some(c) = lowest_of(valid, HEARTS) {
            return c;
        }
        lowest(valid).unwrap()
    }

    fn follow_medium(&self, valid: &[u8], lead: u8, trick: &[u8]) -> u8 {
        let top = highest_of(trick, lead).map_or(0, rank);
        highest_below(&of_suit(valid, lead), top).unwrap_or_else(|| lowest(valid).unwrap())
    }

    fn slough_medium(&self, valid: &[u8]) -> u8 {
        if valid.contains(&QUEEN_SPADES) {
            return QUEEN_SPADES;
        }
        if self.queen_out() {
            for r in [14, 13] {
                if valid.contains(&card(SPADES, r)) {
                    return card(SPADES, r);
                }
            }
        }
        if let Some(c) = highest_of(valid, HEARTS) {
            return c;
        }
        if let Some(&c) = valid.iter().find(|&&c| suit(c) == SPADES && rank(c) >= 13) {
            return c;
        }
        highest(valid).unwrap()
    }

    fn lead_hard(&self, valid: &[u8], hearts_broken: bool) -> u8 {
        let (my, best, worst, _) = self.standing();
        let leading = my < best;
        let behind = my > worst;
        if self.queen_out()
            && let Some(ls) = lowest_of(valid, SPADES)
            && rank(ls) < 12
        {
            let threshold = if behind { 8 } else { 6 };
            if self.mem.count_played(SPADES) < threshold {
                return ls;
            }
        }
        if leading && best - my > 15 {
            return lowest(valid).unwrap();
        }
        for s in [CLUBS, DIAMONDS] {
            if count_suit(valid, s) >= 2
                && let Some(c) = lowest_of(valid, s).filter(|&c| rank(c) <= 7)
            {
                return c;
            }
        }
        if hearts_broken && let Some(c) = lowest_of(valid, HEARTS).filter(|&c| rank(c) <= 6) {
            return c;
        }
        for s in [CLUBS, DIAMONDS] {
            if let Some(c) = lowest_of(valid, s) {
                return c;
            }
        }
        if let Some(c) = lowest_of(valid, SPADES).filter(|&c| rank(c) < 11) {
            return c;
        }
        if hearts_broken && let Some(c) = lowest_of(valid, HEARTS) {
            return c;
        }
        lowest(valid).unwrap()
    }

    fn follow_hard(&self, valid: &[u8], lead: u8, trick: &[(usize, u8)]) -> u8 {
        let cards: Vec<u8> = trick.iter().map(|&(_, c)| c).collect();
        let top = highest_of(&cards, lead).map_or(0, rank);
        let in_suit = of_suit(valid, lead);

        // Someone going for the moon is winning this trick: take it from them.
        if let Some(s) = self.moon_shooter()
            && current_winner(trick) == Some(s)
            && let Some(c) = lowest(
                &in_suit
                    .iter()
                    .copied()
                    .filter(|&c| rank(c) > top)
                    .collect::<Vec<_>>(),
            )
        {
            return c;
        }
        // In every seat: duck under the top card if possible, otherwise the
        // lowest card loses the least.
        highest_below(&in_suit, top).unwrap_or_else(|| lowest(valid).unwrap())
    }

    fn slough_hard(&self, valid: &[u8], trick: &[(usize, u8)]) -> u8 {
        let pts: i32 = trick.iter().map(|&(_, c)| points(c)).sum();
        let (my, _, worst, leader) = self.standing();
        let behind = my > worst;
        let winner = current_winner(trick);

        if let Some(s) = self.moon_shooter()
            && winner == Some(s)
            && let Some(&c) = valid
                .iter()
                .find(|&&c| suit(c) != HEARTS && c != QUEEN_SPADES)
        {
            return c;
        }
        if pts > 0 {
            if valid.contains(&QUEEN_SPADES) {
                return QUEEN_SPADES;
            }
            if let Some(c) = highest_of(valid, HEARTS) {
                return c;
            }
        }
        if winner == Some(leader) && pts == 0 && behind {
            if valid.contains(&QUEEN_SPADES) {
                return QUEEN_SPADES;
            }
            if let Some(c) = highest_of(valid, HEARTS) {
                return c;
            }
        }
        let have_q = self.has(QUEEN_SPADES);
        if !self.mem.queen_played && !have_q {
            for r in [14, 13] {
                if valid.contains(&card(SPADES, r)) {
                    return card(SPADES, r);
                }
            }
        }
        if have_q && valid.contains(&QUEEN_SPADES) {
            return QUEEN_SPADES;
        }
        // Shed the longest side suit to work towards a void.
        let counts: [usize; 4] = std::array::from_fn(|s| count_suit(self.hand, s as u8));
        let mut longest = None;
        let mut longest_n = 0;
        for &c in valid {
            if suit(c) != HEARTS && counts[suit(c) as usize] > longest_n {
                longest_n = counts[suit(c) as usize];
                longest = Some(suit(c));
            }
        }
        if let Some(s) = longest
            && let Some(c) = highest_of(valid, s)
        {
            return c;
        }
        if let Some(c) = highest_of(valid, HEARTS) {
            return c;
        }
        highest(valid).unwrap()
    }
}
