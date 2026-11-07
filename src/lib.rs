//! Calydon: a memecoin you hunt together.
//!
//! A plain NEP-141 token plus a hunt. Holders `commit` tokens against the beast's HP before the
//! deadline; anyone calls `resolve` after it; each hunter calls `claim` once.
//! - Slain (committed >= HP): stake back plus a share of the hoard, rounded down. Next HP x1.25.
//! - Escaped: stake back minus 10%, rounded up, and that part burns. Next HP x0.8.
//!
//! The supply is created once at `new`; nothing ever mints again, so it only falls (escape burns).
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
use near_sdk::store::{LookupMap, Vector};
use near_sdk::{
    AccountId, BorshStorageKey, NearToken, PanicOnDefault, PromiseOrValue, assert_one_yocto, env,
    near, require,
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
    /// Part of an escaped stake that burns, rounded up.
    pub burn_bps: u32,
    pub hp_up_bps: u32,
    pub hp_down_bps: u32,
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
    /// The contract's own token balance; equals hoard + locked + unclaimed.
    pub held: U128,
    pub hoard: U128,
    pub locked: U128,
    pub unclaimed: U128,
    pub burned: U128,
}

#[derive(BorshStorageKey)]
#[near]
enum Key {
    Accounts,
    Hunts,
    Entries,
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct Calydon {
    token: FungibleToken,
    config: Config,
    hunts: Vector<Hunt>,
    entries: LookupMap<(u64, AccountId), Entry>,
    hoard: u128,
    unclaimed: u128,
    burned: u128,
}

pub fn ceil_bps(amount: u128, bps: u32) -> u128 {
    let (q, r) = (U256::from(amount) * U256::from(bps)).div_mod(U256::from(10_000u32));
    q.as_u128() + !r.is_zero() as u128
}

pub fn floor_bps(amount: u128, bps: u32) -> u128 {
    (U256::from(amount) * U256::from(bps) / U256::from(10_000u32)).as_u128()
}

/// A hunter's share of a slain hunt's pot: pot x weight / total weight, rounded down.
pub fn share(pot: u128, weight: u128, total_weight: u128) -> u128 {
    (U256::from(pot) * U256::from(weight) / U256::from(total_weight)).as_u128()
}

fn emit(event: &str, data: Value) {
    env::log_str(&format!(
        "EVENT_JSON:{}",
        json!({"standard": "calydon", "version": "0.1.0", "event": event, "data": [data]})
    ));
}

#[near]
impl Calydon {
    /// Mints `supply` to `holder` and `hoard` into the hoard, once; opens hunt 0.
    #[init]
    pub fn new(holder: AccountId, supply: U128, hoard: U128, config: Config) -> Self {
        require!(config.hunt_blocks > 0 && config.step_blocks > 0, "BAD_CONFIG");
        require!(config.tribute_bps <= 10_000 && config.burn_bps <= 10_000, "BAD_CONFIG");
        let mut this = Self {
            token: FungibleToken::new(Key::Accounts),
            config,
            hunts: Vector::new(Key::Hunts),
            entries: LookupMap::new(Key::Entries),
            hoard: hoard.0,
            unclaimed: 0,
            burned: 0,
        };
        let me = env::current_account_id();
        this.token.internal_register_account(&me);
        this.token.internal_register_account(&holder);
        this.token.internal_deposit(&holder, supply.0);
        this.token.internal_deposit(&me, hoard.0);
        FtMint::emit_many(&[
            FtMint { owner_id: &holder, amount: supply, memo: Some("launch supply") },
            FtMint { owner_id: &me, amount: hoard, memo: Some("launch hoard") },
        ]);
        this.open(this.config.first_hp.0);
        this
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
    /// the tribute on top. Returns the stake actually locked.
    #[payable]
    pub fn commit(&mut self, amount: U128) -> U128 {
        assert_one_yocto();
        let hunter = env::predecessor_account_id();
        let height = env::block_height();
        let id = self.hunts.len() as u32 - 1;
        let mut hunt = self.hunts[id].clone();
        require!(height < hunt.deadline, "HUNT_CLOSED");
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
        let fee = ceil_bps(stake, self.config.tribute_bps);
        self.move_tokens(&hunter, &env::current_account_id(), stake + fee, "commit");
        self.hoard += fee;

        let key = (hunt.id, hunter.clone());
        let mut entry = self.entries.get(&key).copied().unwrap_or_default();
        if entry.stake.0 == 0 {
            hunt.hunters += 1;
        }
        entry.stake.0 += stake;
        entry.weight.0 += weight;
        self.entries.insert(key, entry);
        hunt.committed.0 += stake;
        hunt.weight.0 += weight;
        emit("commit", json!({"hunt": hunt.id, "hunter": hunter, "stake": U128(stake), "fee": U128(fee), "weight": U128(weight), "committed": hunt.committed}));
        self.hunts[id] = hunt;
        U128(stake)
    }

    /// Settles hunt `hunt` once its deadline has passed and opens the next one. Anyone may call it.
    pub fn resolve(&mut self, hunt: u64) -> Outcome {
        let current = self.hunts.len() as u64 - 1;
        require!(hunt <= current, "NO_SUCH_HUNT");
        require!(hunt == current, "ALREADY_RESOLVED");
        let mut h = self.hunts[current as u32].clone();
        require!(env::block_height() >= h.deadline, "BEFORE_DEADLINE");
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
        outcome
    }

    /// Pays the caller's settlement for a resolved hunt, exactly once. Returns the amount paid.
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

    pub fn totals(&self) -> Totals {
        Totals {
            supply: U128(self.token.total_supply),
            held: U128(self.token.internal_unwrap_balance_of(&env::current_account_id())),
            hoard: U128(self.hoard),
            locked: self.current_hunt().committed,
            unclaimed: U128(self.unclaimed),
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
