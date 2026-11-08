//! Calydon: a memecoin you hunt together.
//!
//! A plain NEP-141 token plus a hunt. Holders `commit` tokens against the beast's HP before the
//! deadline; anyone calls `resolve` after it; each hunter calls `claim` once.
//! - Entry fee: each committed token pays `tribute_bps` plus `fill_fee_bps` x (how full the bar was
//!   when it went in), on top of the stake, into the hoard (see `fill_fee`).
//! - Slain (committed >= HP): stake back plus a share of the hoard, rounded down. Next HP x `hp_up_bps`.
//! - Escaped: stake back minus `burn_bps`, rounded up, and that part burns. Next HP x `hp_down_bps`.
//!
//! Launch values: rule `EarlyWeight`, tribute 1%, fill fee up to +10%, burn 10%, HP x1.1 / x0.7.
//! The supply is created once at `new`; nothing ever mints again, so it only falls (escape burns).
//! It starts in three places: a starter bucket (`starter` gives a fixed amount once per account),
//! the launch hoard, and the launch holder.
//! Every record the contract stores is paid in NEAR by the call that creates it: a commit that
//! creates an entry attaches `entry_deposit` (returned by `claim`); `resolve` and `starter` attach
//! `record_deposit` (kept: their records stay). Each such call checks that its storage growth costs
//! no more than what was attached.
//! CAL sent to the contract by a plain transfer is swept into the hoard by the next `resolve`.
//! Every parameter is fixed at `new` and readable through `config`. There is no owner, setter or pause.
use near_contract_standards::fungible_token::events::{FtBurn, FtMint, FtTransfer};
use near_contract_standards::fungible_token::metadata::{
    FT_METADATA_SPEC, FungibleTokenMetadata, FungibleTokenMetadataProvider,
};
use near_contract_standards::fungible_token::{
    FungibleToken, FungibleTokenCore, FungibleTokenResolver,
};
use near_contract_standards::storage_management::{
    StorageBalance, StorageBalanceBounds, StorageManagement,
};
use near_sdk::json_types::U128;
use near_sdk::serde_json::{Value, json};
use near_sdk::store::{LookupMap, LookupSet, Vector};
use near_sdk::{
    AccountId, BorshStorageKey, NearToken, PanicOnDefault, Promise, PromiseOrValue, env, near,
    require,
};

uint::construct_uint! {
    struct U256(4);
}

/// How a slain hunt's hoard is shared. Chosen once at launch.
#[near(serializers = [json, borsh])]
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Rule {
    /// Rule A: stake beyond the HP is refunded at commit and earns nothing; shares are pro rata to stake.
    HpCap,
    /// Rule B: no cap; a commit's weight is stake x the number of `step_blocks` steps left before
    /// the deadline, so the same stake committed earlier earns a larger share.
    EarlyWeight,
}

#[near(serializers = [json, borsh])]
#[derive(Clone)]
pub struct Config {
    pub name: String,
    pub symbol: String,
    pub decimals: u8,
    pub rule: Rule,
    pub first_hp: U128,
    pub hunt_blocks: u64,
    pub step_blocks: u64,
    /// Entry fee on `commit`, paid on top of the stake into the hoard, rounded up.
    pub tribute_bps: u32,
    /// Extra entry fee per token, rising linearly from 0 at an empty bar to this at a full one (and
    /// this past the HP); paid into the hoard with the tribute. See `fill_fee`.
    pub fill_fee_bps: u32,
    /// Part of an escaped stake that burns, rounded up.
    pub burn_bps: u32,
    pub hp_up_bps: u32,
    pub hp_down_bps: u32,
    /// Smallest amount a commit accepts, in yocto-CAL.
    pub min_stake: U128,
    /// yoctoNEAR attached to a commit that creates the caller's entry in a hunt (a later commit to the
    /// same hunt attaches 1 yoctoNEAR). It pays for the entry's storage; `claim` sends it back.
    pub entry_deposit: U128,
    /// yoctoNEAR attached to `resolve` and to `starter`. It pays for the record they add (the next
    /// hunt, the starter mark) and stays with the contract, as those records do.
    pub record_deposit: U128,
    /// yocto-CAL that `starter` gives an account, once, while the starter bucket lasts.
    pub starter_amount: U128,
}

#[near(serializers = [json, borsh])]
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Outcome {
    Slain,
    Escaped,
}

#[near(serializers = [json, borsh])]
#[derive(Clone)]
pub struct Hunt {
    pub id: u64,
    pub hp: U128,
    pub deadline: u64,
    pub committed: U128,
    pub weight: U128,
    pub hunters: u32,
    pub claims: u32,
    pub outcome: Option<Outcome>,
    /// The hoard handed to this hunt's hunters when it was slain; `paid` of it has been claimed.
    pub pot: U128,
    pub paid: U128,
    /// "First Spear": the account whose commit opened this hunt's bar.
    pub first_spear: Option<AccountId>,
}

#[near(serializers = [json, borsh])]
#[derive(Clone, Copy, Default)]
pub struct Entry {
    pub stake: U128,
    pub weight: U128,
}

#[near(serializers = [json])]
pub struct Totals {
    pub supply: U128,
    /// The contract's own token balance: hoard + locked + unclaimed + starter_left, plus whatever was
    /// sent to the contract by plain transfer since the last `resolve` (which sweeps it into the hoard).
    pub held: U128,
    pub hoard: U128,
    pub locked: U128,
    pub unclaimed: U128,
    pub starter_left: U128,
    pub burned: U128,
}

#[derive(BorshStorageKey)]
#[near]
enum Key {
    Accounts,
    Hunts,
    Entries,
    Starters,
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct Calydon {
    token: FungibleToken,
    config: Config,
    hunts: Vector<Hunt>,
    entries: LookupMap<(u64, AccountId), Entry>,
    starters: LookupSet<AccountId>,
    hoard: u128,
    unclaimed: u128,
    starter_left: u128,
    burned: u128,
}

pub fn ceil_bps(amount: u128, bps: u32) -> u128 {
    let (q, r) = (U256::from(amount) * U256::from(bps)).div_mod(U256::from(10_000u32));
    q.as_u128() + !r.is_zero() as u128
}

pub fn floor_bps(amount: u128, bps: u32) -> u128 {
    (U256::from(amount) * U256::from(bps) / U256::from(10_000u32)).as_u128()
}

/// Entry fee on `stake` tokens committed when `before` was already committed against
/// `hp`: the token at fill level x pays tribute_bps + fill_fee_bps x min(x, hp) / hp, summed over
/// the tokens of the stake and rounded up once. The sum is additive over consecutive parts, so a
/// stake cut into parts that land one after another pays the same before rounding and never less after it.
pub fn fill_fee(before: u128, stake: u128, hp: u128, tribute_bps: u32, fill_fee_bps: u32) -> u128 {
    let a = U256::from(before.min(hp));
    let b = U256::from(before.saturating_add(stake).min(hp));
    let (s, h) = (U256::from(stake), U256::from(hp));
    let above = s - (b - a);
    let (t, f) = (U256::from(tribute_bps), U256::from(fill_fee_bps));
    let two = U256::from(2u8);
    let num: U256 = t * s * h * two + f * (b * b - a * a) + f * h * above * two;
    let (q, r) = num.div_mod(h * U256::from(20_000u32));
    q.as_u128() + !r.is_zero() as u128
}

/// A hunter's share of a slain hunt's pot: pot x weight / total weight, rounded down.
pub fn share(pot: u128, weight: u128, total_weight: u128) -> u128 {
    (U256::from(pot) * U256::from(weight) / U256::from(total_weight)).as_u128()
}

fn emit(event: &str, data: Value) {
    env::log_str(&format!(
        "EVENT_JSON:{}",
        json!({"standard": "calydon", "version": "1.1.0", "event": event, "data": [data]})
    ));
}

#[near]
impl Calydon {
    /// Mints, once: `supply` to `holder`, `hoard` into the hoard and `starter` into the starter
    /// bucket (both held by the contract). Opens hunt 0.
    #[init]
    pub fn new(holder: AccountId, supply: U128, hoard: U128, starter: U128, config: Config) -> Self {
        require!(config.hunt_blocks > 0 && config.step_blocks > 0, "BAD_CONFIG");
        require!(config.tribute_bps <= 10_000 && config.burn_bps <= 10_000, "BAD_CONFIG");
        require!(config.tribute_bps + config.fill_fee_bps <= 10_000, "BAD_CONFIG");
        require!(config.min_stake.0 > 0 && config.entry_deposit.0 > 0, "BAD_CONFIG");
        let mut this = Self {
            token: FungibleToken::new(Key::Accounts),
            config,
            hunts: Vector::new(Key::Hunts),
            entries: LookupMap::new(Key::Entries),
            starters: LookupSet::new(Key::Starters),
            hoard: hoard.0,
            unclaimed: 0,
            starter_left: starter.0,
            burned: 0,
        };
        let me = env::current_account_id();
        this.token.internal_register_account(&me);
        this.token.internal_register_account(&holder);
        this.token.internal_deposit(&holder, supply.0);
        this.token.internal_deposit(&me, hoard.0 + starter.0);
        FtMint::emit_many(&[
            FtMint { owner_id: &holder, amount: supply, memo: Some("launch holder") },
            FtMint { owner_id: &me, amount: hoard, memo: Some("launch hoard") },
            FtMint { owner_id: &me, amount: starter, memo: Some("starter bucket") },
        ]);
        this.open(this.config.first_hp.0);
        this
    }

    /// Flushes the buffered collections, then requires that the storage added since `before` costs
    /// no more than `attached` yoctoNEAR.
    fn pay_storage(&mut self, before: u64, attached: u128) {
        self.hunts.flush();
        self.entries.flush();
        let added = env::storage_usage().saturating_sub(before) as u128;
        require!(added * env::storage_byte_cost().as_yoctonear() <= attached, "STORAGE_NOT_COVERED");
    }

    fn require_attached(&self, amount: u128, error: &str) {
        if env::attached_deposit().as_yoctonear() != amount {
            env::panic_str(error);
        }
    }

    fn open(&mut self, hp: u128) {
        let id = self.hunts.len() as u64;
        self.hunts.push(Hunt {
            id,
            hp: U128(hp.max(1)),
            deadline: env::block_height() + self.config.hunt_blocks,
            committed: U128(0),
            weight: U128(0),
            hunters: 0,
            claims: 0,
            outcome: None,
            pot: U128(0),
            paid: U128(0),
            first_spear: None,
        });
        emit("open", json!({"hunt": id, "hp": U128(hp.max(1)), "deadline": env::block_height() + self.config.hunt_blocks}));
    }

    fn move_tokens(&mut self, from: &AccountId, to: &AccountId, amount: u128, memo: &str) {
        if amount == 0 {
            return;
        }
        self.token.internal_withdraw(from, amount);
        self.token.internal_deposit(to, amount);
        FtTransfer { old_owner_id: from, new_owner_id: to, amount: U128(amount), memo: Some(memo) }
            .emit();
    }

    /// Locks `amount` in the current hunt (under rule A, at most the HP still missing) and charges
    /// the entry fee on top. Attach `entry_deposit` if this creates your entry in the hunt, otherwise
    /// 1 yoctoNEAR. Returns the stake actually locked.
    #[payable]
    pub fn commit(&mut self, amount: U128) -> U128 {
        let before = env::storage_usage();
        let hunter = env::predecessor_account_id();
        let height = env::block_height();
        let id = self.hunts.len() as u32 - 1;
        let mut hunt = self.hunts[id].clone();
        require!(height < hunt.deadline, "HUNT_CLOSED");
        require!(amount.0 >= self.config.min_stake.0, "BELOW_MIN_STAKE");
        let key = (hunt.id, hunter.clone());
        let fresh = !self.entries.contains_key(&key);
        if fresh {
            self.require_attached(self.config.entry_deposit.0, "ATTACH_ENTRY_DEPOSIT");
        } else {
            self.require_attached(1, "ATTACH_ONE_YOCTO");
        }
        let (stake, weight) = match self.config.rule {
            Rule::HpCap => {
                let stake = amount.0.min(hunt.hp.0.saturating_sub(hunt.committed.0));
                (stake, stake)
            }
            Rule::EarlyWeight => {
                let steps = (hunt.deadline - height).div_ceil(self.config.step_blocks);
                (amount.0, amount.0 * steps as u128)
            }
        };
        require!(stake > 0, "NOTHING_TO_LOCK");
        let fee = fill_fee(hunt.committed.0, stake, hunt.hp.0, self.config.tribute_bps, self.config.fill_fee_bps);
        self.move_tokens(&hunter, &env::current_account_id(), stake + fee, "commit");
        self.hoard += fee;

        let mut entry = self.entries.get(&key).copied().unwrap_or_default();
        if fresh {
            hunt.hunters += 1;
        }
        if hunt.first_spear.is_none() {
            hunt.first_spear = Some(hunter.clone());
        }
        entry.stake.0 += stake;
        entry.weight.0 += weight;
        self.entries.insert(key, entry);
        hunt.committed.0 += stake;
        hunt.weight.0 += weight;
        emit("commit", json!({"hunt": hunt.id, "hunter": hunter, "stake": U128(stake), "fee": U128(fee), "weight": U128(weight), "committed": hunt.committed, "height": height}));
        self.hunts[id] = hunt;
        self.pay_storage(before, env::attached_deposit().as_yoctonear());
        U128(stake)
    }

    /// Settles hunt `hunt` once its deadline has passed and opens the next one. Anyone may call it,
    /// attaching `record_deposit` for the next hunt's record. First sweeps into the hoard any CAL the
    /// contract holds beyond hoard + locked + unclaimed + starter bucket (plain transfers to it).
    #[payable]
    pub fn resolve(&mut self, hunt: u64) -> Outcome {
        let before = env::storage_usage();
        self.require_attached(self.config.record_deposit.0, "ATTACH_RECORD_DEPOSIT");
        let current = self.hunts.len() as u64 - 1;
        require!(hunt <= current, "NO_SUCH_HUNT");
        require!(hunt == current, "ALREADY_RESOLVED");
        let mut h = self.hunts[current as u32].clone();
        require!(env::block_height() >= h.deadline, "BEFORE_DEADLINE");
        let held = self.token.internal_unwrap_balance_of(&env::current_account_id());
        let surplus = held - (self.hoard + h.committed.0 + self.unclaimed + self.starter_left);
        if surplus > 0 {
            self.hoard += surplus;
            emit("sweep", json!({"hunt": hunt, "amount": U128(surplus)}));
        }
        let slain = h.committed.0 > 0 && h.committed.0 >= h.hp.0;
        let (outcome, next_bps) = if slain {
            h.pot = U128(self.hoard);
            self.hoard = 0;
            (Outcome::Slain, self.config.hp_up_bps)
        } else {
            (Outcome::Escaped, self.config.hp_down_bps)
        };
        self.unclaimed += h.committed.0 + h.pot.0;
        h.outcome = Some(outcome);
        let next_hp = floor_bps(h.hp.0, next_bps);
        emit("resolve", json!({"hunt": hunt, "outcome": outcome, "hp": h.hp, "committed": h.committed, "pot": h.pot, "next_hp": U128(next_hp.max(1))}));
        self.hunts[current as u32] = h;
        self.open(next_hp);
        self.pay_storage(before, self.config.record_deposit.0);
        outcome
    }

    /// Gives the caller `starter_amount` CAL from the starter bucket, once per account (the last one
    /// gets what is left). The caller must be registered (`storage_deposit`) and attach
    /// `record_deposit` for the mark that it has taken its starter.
    #[payable]
    pub fn starter(&mut self) -> U128 {
        let before = env::storage_usage();
        self.require_attached(self.config.record_deposit.0, "ATTACH_RECORD_DEPOSIT");
        let who = env::predecessor_account_id();
        require!(self.token.accounts.contains_key(&who), "NOT_REGISTERED");
        require!(!self.starters.contains(&who), "STARTER_TAKEN");
        let amount = self.config.starter_amount.0.min(self.starter_left);
        require!(amount > 0, "STARTER_EMPTY");
        self.starter_left -= amount;
        self.starters.insert(who.clone());
        let me = env::current_account_id();
        self.move_tokens(&me, &who, amount, "starter");
        emit("starter", json!({"account_id": who, "amount": U128(amount), "left": U128(self.starter_left)}));
        self.pay_storage(before, self.config.record_deposit.0);
        U128(amount)
    }

    /// Pays the caller's settlement for a resolved hunt, exactly once, and sends back the entry's
    /// `entry_deposit` in NEAR. Returns the CAL paid.
    pub fn claim(&mut self, hunt: u64) -> U128 {
        let hunter = env::predecessor_account_id();
        require!(hunt < self.hunts.len() as u64, "NO_SUCH_HUNT");
        let mut h = self.hunts[hunt as u32].clone();
        let outcome = h.outcome.unwrap_or_else(|| env::panic_str("NOT_RESOLVED"));
        let entry = self
            .entries
            .remove(&(hunt, hunter.clone()))
            .unwrap_or_else(|| env::panic_str("NOTHING_TO_CLAIM"));
        let stake = entry.stake.0;
        let (payout, burn) = match outcome {
            Outcome::Slain => {
                let s = share(h.pot.0, entry.weight.0, h.weight.0);
                h.paid.0 += s;
                (stake + s, 0)
            }
            Outcome::Escaped => {
                let burn = ceil_bps(stake, self.config.burn_bps);
                (stake - burn, burn)
            }
        };
        h.claims += 1;
        self.unclaimed -= payout + burn;
        if h.claims == h.hunters {
            // Rounding dust of the pot goes back to the hoard once the last hunter has claimed.
            let dust = h.pot.0 - h.paid.0;
            self.unclaimed -= dust;
            self.hoard += dust;
        }
        let me = env::current_account_id();
        self.move_tokens(&me, &hunter, payout, "claim");
        if burn > 0 {
            self.token.internal_withdraw(&me, burn);
            self.burned += burn;
            FtBurn { owner_id: &me, amount: U128(burn), memo: Some("escaped") }.emit();
        }
        emit("claim", json!({"hunt": hunt, "hunter": hunter, "outcome": outcome, "paid": U128(payout), "burned": U128(burn)}));
        self.hunts[hunt as u32] = h;
        Promise::new(hunter).transfer(NearToken::from_yoctonear(self.config.entry_deposit.0)).detach();
        U128(payout)
    }

    pub fn config(&self) -> Config {
        self.config.clone()
    }

    pub fn current_hunt(&self) -> Hunt {
        self.hunts[self.hunts.len() - 1].clone()
    }

    pub fn hunt(&self, id: u64) -> Option<Hunt> {
        self.hunts.get(id as u32).cloned()
    }

    pub fn entry(&self, hunt: u64, account_id: AccountId) -> Option<Entry> {
        self.entries.get(&(hunt, account_id)).copied()
    }

    /// Whether `account_id` has taken its starter.
    pub fn starter_taken(&self, account_id: AccountId) -> bool {
        self.starters.contains(&account_id)
    }

    pub fn totals(&self) -> Totals {
        Totals {
            supply: U128(self.token.total_supply),
            held: U128(self.token.internal_unwrap_balance_of(&env::current_account_id())),
            hoard: U128(self.hoard),
            locked: self.current_hunt().committed,
            unclaimed: U128(self.unclaimed),
            starter_left: U128(self.starter_left),
            burned: U128(self.burned),
        }
    }
}

#[near]
impl FungibleTokenCore for Calydon {
    #[payable]
    fn ft_transfer(&mut self, receiver_id: AccountId, amount: U128, memo: Option<String>) {
        self.token.ft_transfer(receiver_id, amount, memo)
    }

    #[payable]
    fn ft_transfer_call(
        &mut self,
        receiver_id: AccountId,
        amount: U128,
        memo: Option<String>,
        msg: String,
    ) -> PromiseOrValue<U128> {
        self.token.ft_transfer_call(receiver_id, amount, memo, msg)
    }

    fn ft_total_supply(&self) -> U128 {
        self.token.ft_total_supply()
    }

    fn ft_balance_of(&self, account_id: AccountId) -> U128 {
        self.token.ft_balance_of(account_id)
    }
}

#[near]
impl FungibleTokenResolver for Calydon {
    #[private]
    fn ft_resolve_transfer(
        &mut self,
        sender_id: AccountId,
        receiver_id: AccountId,
        amount: U128,
    ) -> U128 {
        // The standard burns a refund only when the sender has unregistered, which
        // `storage_unregister` below never allows.
        self.token.internal_ft_resolve_transfer(&sender_id, receiver_id, amount).0.into()
    }
}

#[near]
impl StorageManagement for Calydon {
    #[payable]
    fn storage_deposit(
        &mut self,
        account_id: Option<AccountId>,
        registration_only: Option<bool>,
    ) -> StorageBalance {
        self.token.storage_deposit(account_id, registration_only)
    }

    #[payable]
    fn storage_withdraw(&mut self, amount: Option<NearToken>) -> StorageBalance {
        self.token.storage_withdraw(amount)
    }

    /// Never allowed: unregistering would burn tokens outside an escaped hunt.
    #[payable]
    fn storage_unregister(&mut self, force: Option<bool>) -> bool {
        let _ = force;
        env::panic_str("UNREGISTER_DISABLED")
    }

    fn storage_balance_bounds(&self) -> StorageBalanceBounds {
        self.token.storage_balance_bounds()
    }

    fn storage_balance_of(&self, account_id: AccountId) -> Option<StorageBalance> {
        self.token.storage_balance_of(account_id)
    }
}

#[near]
impl FungibleTokenMetadataProvider for Calydon {
    fn ft_metadata(&self) -> FungibleTokenMetadata {
        FungibleTokenMetadata {
            spec: FT_METADATA_SPEC.to_string(),
            name: self.config.name.clone(),
            symbol: self.config.symbol.clone(),
            icon: None,
            reference: None,
            reference_hash: None,
            decimals: self.config.decimals,
        }
    }
}

#[cfg(test)]
mod tests;
