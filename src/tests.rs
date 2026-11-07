//! I3: splitting a stake across accounts never pays more. With the hunt's total weight, the pot,
//! the timing and the outcome held fixed, shares round down (floor a + floor b <= floor(a + b)),
//! fees and burns round up (ceil a + ceil b >= ceil(a + b)).
use super::*;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u128) -> u128 {
        ((self.next() as u128) << 64 | self.next() as u128) % n
    }
}

/// Net result of one account's stake in one hunt: what it gets back minus what it paid in.
fn net(stake: u128, fee: u128, weight: u128, slain: bool, pot: u128, total_weight: u128) -> i128 {
    let back = if slain {
        stake + share(pot, weight, total_weight)
    } else {
        stake - ceil_bps(stake, 1_000)
    };
    back as i128 - (stake + fee) as i128
}

#[test]
fn splitting_never_pays_more() {
    // Variant R2: each part pays fill_fee(committed before it, part, hp); the parts of a split land
    // one after another and cover the same fill interval as the whole stake. Weights are unchanged.
    let mut rng = Rng(494);
    let mut strictly_less = 0;
    for case in 0..20_000 {
        // Small numbers make rounding bite; large ones cover 24-decimal amounts.
        let scale: u128 = if case % 2 == 0 { 1_000 } else { 10u128.pow(27) };
        let hp = 1 + rng.below(scale);
        let before = rng.below(hp + hp / 2);
        let stake = 1 + rng.below(scale * 2);
        let others = rng.below(scale);
        let steps = 1 + rng.below(12);
        let pot = rng.below(scale * 4);
        let fill = rng.below(5_001) as u32;
        let total_weight = (stake + others) * steps;
        let slain = rng.next() % 2 == 0;
        let parts = 2 + rng.below(7) as usize;
        let mut cuts: Vec<u128> = (0..parts - 1).map(|_| rng.below(stake + 1)).collect();
        cuts.extend([0, stake]);
        cuts.sort();
        let mut at = before;
        let split: i128 = cuts
            .windows(2)
            .map(|w| w[1] - w[0])
            .filter(|&s| s > 0)
            .map(|s| {
                let fee = fill_fee(at, s, hp, 100, fill);
                at += s;
                net(s, fee, s * steps, slain, pot, total_weight)
            })
            .sum();
        let whole = net(stake, fill_fee(before, stake, hp, 100, fill), stake * steps, slain, pot, total_weight);
        assert!(split <= whole, "case {case}: split {split} > whole {whole}");
        strictly_less += (split < whole) as u32;
    }
    assert!(strictly_less > 0, "rounding never bit; the test is not exercising it");
}

#[test]
fn fill_fees() {
    // Without the fill part it is the plain tribute.
    assert_eq!(fill_fee(500, 101, 1_000, 100, 0), ceil_bps(101, 100));
    // HP 1000, fill fee 10%: the first 600 tokens pay 1% + 10% x 300/1000 on average = 6 + 18.
    assert_eq!(fill_fee(0, 600, 1_000, 100, 1_000), 24);
    // The last 400 pay 1% + 10% x 800/1000 = 4 + 32; past the HP every token pays 11%.
    assert_eq!(fill_fee(600, 400, 1_000, 100, 1_000), 36);
    assert_eq!(fill_fee(1_000, 700, 1_000, 100, 1_000), 77);
    assert_eq!(fill_fee(900, 200, 1_000, 100, 1_000), 22); // 2 + 9.5 + 10, rounded up
    // Rounded up once.
    assert_eq!(fill_fee(0, 1, 3, 100, 1_000), 1);
    let big = 10u128.pow(32);
    assert_eq!(fill_fee(0, big, big, 100, 1_000), big / 100 + big / 20);
}

#[test]
fn m2_counterexample_is_the_safe_direction() {
    // Hoard 2, total stake 3: one account staking 2 gets floor(4/3) = 1; two accounts staking 1 get 0 each.
    assert_eq!(share(2, 2, 3), 1);
    assert_eq!(share(2, 1, 3) * 2, 0);
}

#[test]
fn rounding_helpers() {
    assert_eq!(ceil_bps(1, 100), 1);
    assert_eq!(ceil_bps(100, 100), 1);
    assert_eq!(ceil_bps(101, 100), 2);
    assert_eq!(ceil_bps(0, 1_000), 0);
    assert_eq!(floor_bps(5, 8_000), 4);
    assert_eq!(ceil_bps(u128::MAX / 2, 10_000), u128::MAX / 2);
}
