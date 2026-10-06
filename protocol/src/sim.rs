//! Lokale Kette im Speicher: prüft Transaktionen so, wie ein Node es für
//! unsere Zwecke täte (UTXO vorhanden, Skripte mit Compute-Budget,
//! Covenant-Genesis, Speichermasse, Mindestgebühr, Locktime, relative Sperre) und wendet sie an.
//! Ersetzt KEIN Testnetz: Mempool-Standardregeln, P2P und Blockmasse fehlen.
//!
//! Locktime wie im Konsens (rusty-kaspa a41a333,
//! consensus/src/processes/transaction_validator/tx_validation_in_header_context.rs):
//! - 0: final (Z. 59);
//! - unter LOCK_TIME_THRESHOLD = 5·10^11 (consensus/core/src/constants.rs:22):
//!   DAA-Score, final wenn lock_time < DAA (Z. 65, 79);
//! - ab LOCK_TIME_THRESHOLD: Unix-Millisekunden, final wenn lock_time <
//!   Past Median Time (Z. 67–68, 76, 79). Die PMT ist der Mittelwert der 11
//!   mittleren von 27 Zeitstempeln im Abstand von etwa 10 s
//!   (consensus/src/processes/past_median_time.rs:18–38,
//!   consensus/core/src/config/constants.rs:23–30), läuft der Uhr also etwa
//!   2¼ Minuten hinterher. Mempool (virtual_processor/processor.rs:1226–1230)
//!   und Block (body_validation_in_context.rs:31–39) prüfen gleich.
//! - Nicht final ist trotzdem gültig, wenn ALLE Eingänge sequence = u64::MAX
//!   haben (Z. 86–90). Genau das verbietet OP_CHECKLOCKTIMEVERIFY für den
//!   eigenen Eingang (crypto/txscript/src/opcodes/mod.rs:1014–1064, Sequence Z. 1057).
//!
//! `now_ms` ist die simulierte Past Median Time; `advance_time` stellt sie vor.

use crate::ops::{Funds, p2pk_spk, xonly};
use crate::txb::{Built, check_scripts, masses, min_fee};
use kaspa_consensus_core::config::params::{MAINNET_PARAMS, Params};
use kaspa_consensus_core::constants::{LOCK_TIME_THRESHOLD, SEQUENCE_LOCK_TIME_DISABLED, SEQUENCE_LOCK_TIME_MASK};
use kaspa_consensus_core::tx::{TransactionId, TransactionOutpoint, UtxoEntry};
use secp256k1::Keypair;
use std::collections::HashMap;

/// Startzeit des Simulators: 2027-01-01 00:00 UTC
pub const START_MS: u64 = 1_798_761_600_000;

pub struct Sim {
    pub utxos: HashMap<TransactionOutpoint, UtxoEntry>,
    pub daa: u64,
    /// simulierte Past Median Time (Unix-ms) für Zeit-Locktimes
    pub now_ms: u64,
    pub params: Params,
    next_fake: u8,
}

impl Default for Sim {
    fn default() -> Self {
        Self::new()
    }
}

impl Sim {
    pub fn new() -> Self {
        Self { utxos: HashMap::new(), daa: 1_000_000, now_ms: START_MS, params: MAINNET_PARAMS, next_fake: 1 }
    }

    /// Legt eine P2PK-UTXO für `key` an (wie ein Faucet).
    pub fn faucet(&mut self, key: &Keypair, value: u64) {
        let op = TransactionOutpoint { transaction_id: TransactionId::from_bytes([self.next_fake; 32]), index: 0 };
        self.next_fake += 1;
        self.utxos.insert(op, UtxoEntry::new(value, p2pk_spk(&xonly(key)), self.daa, false, None));
    }

    pub fn funds(&self, key: &Keypair) -> Funds {
        let spk = p2pk_spk(&xonly(key));
        let mut utxos: Vec<_> = self.utxos.iter().filter(|(_, e)| e.script_public_key == spk).map(|(o, e)| (*o, e.clone())).collect();
        utxos.sort_by_key(|(o, _)| (o.transaction_id, o.index));
        Funds::new(key, utxos)
    }

    /// DAA vorrücken (z. B. für den Mindestabstand zwischen Orakel-Updates)
    pub fn advance(&mut self, daa: u64) {
        self.daa += daa;
    }

    /// Uhr (Past Median Time) vorstellen
    pub fn advance_time(&mut self, ms: u64) {
        self.now_ms += ms;
    }

    /// Uhr auf einen Zeitpunkt setzen (nur vorwärts)
    pub fn set_time(&mut self, ms: u64) {
        assert!(ms >= self.now_ms, "die Past Median Time läuft nicht rückwärts");
        self.now_ms = ms;
    }

    pub fn balance(&self, key: &Keypair) -> u64 {
        self.funds(key).utxos.iter().map(|(_, e)| e.amount).sum()
    }

    pub fn submit(&mut self, b: &Built) -> Result<(), String> {
        let tx = &b.tx;
        for (i, (input, e)) in tx.inputs.iter().zip(&b.entries).enumerate() {
            let have = self.utxos.get(&input.previous_outpoint).ok_or(format!("Input {i}: UTXO nicht vorhanden (schon ausgegeben?)"))?;
            if have.amount != e.amount || have.script_public_key != e.script_public_key || have.covenant_id != e.covenant_id {
                return Err(format!("Input {i}: UTXO weicht vom erwarteten Zustand ab"));
            }
        }
        // Konsens: final, wenn lock_time < DAA des Blocks (tx_validation_in_header_context.rs:79);
        // Test-Audit 5: der Simulator hatte auch lock_time == DAA angenommen
        if tx.lock_time >= LOCK_TIME_THRESHOLD {
            // Zeit: final erst, wenn lock_time < Past Median Time, außer alle Eingänge sind final
            if tx.lock_time >= self.now_ms && tx.inputs.iter().any(|i| i.sequence != u64::MAX) {
                return Err(format!("Locktime {} ms ≥ Past Median Time {} ms – Tx noch nicht final", tx.lock_time, self.now_ms));
            }
        } else if tx.lock_time != 0 && tx.lock_time >= self.daa {
            return Err(format!("Locktime {} ≥ DAA {} – Tx noch nicht final", tx.lock_time, self.daa));
        }
        // relative Sperre je Eingang wie check_sequence_lock (tx_validation_in_utxo_context.rs:136):
        // gesperrt, solange DAA der UTXO + Sequenz − 1 ≥ DAA des Blocks
        for (i, input) in tx.inputs.iter().enumerate() {
            if input.sequence & SEQUENCE_LOCK_TIME_DISABLED == 0 {
                let created = self.utxos[&input.previous_outpoint].block_daa_score as i64;
                if created + (input.sequence & SEQUENCE_LOCK_TIME_MASK) as i64 - 1 >= self.daa as i64 {
                    return Err(format!("Input {i}: relative Sperre ({} DAA) noch nicht abgelaufen", input.sequence & SEQUENCE_LOCK_TIME_MASK));
                }
            }
        }
        if tx.outputs.iter().any(|o| o.value == 0) {
            return Err("Ausgang mit 0 sompi (Konsens lehnt ab)".into());
        }
        check_scripts(tx, &b.entries)?;
        // Mempool-Standardregel: die Probe lief am 05.10.2026 im Mainnet hinein
        crate::txb::check_standard_sig_ops(tx, &b.entries)?;
        let (compute, transient, storage) = masses(tx, &b.entries, &self.params)?;
        crate::txb::check_block_limits(compute, transient, storage)?;
        if tx.storage_mass() != storage {
            return Err(format!("Speichermasse committet {} ≠ berechnet {storage}", tx.storage_mass()));
        }
        let total_in: u64 = b.entries.iter().map(|e| e.amount).sum();
        let total_out: u64 = tx.outputs.iter().map(|o| o.value).sum();
        let fee = total_in.checked_sub(total_out).ok_or("Ausgänge > Eingänge")?;
        if fee < min_fee(compute, transient) {
            return Err(format!("Gebühr {fee} < Mindestgebühr {}", min_fee(compute, transient)));
        }
        for input in &tx.inputs {
            self.utxos.remove(&input.previous_outpoint);
        }
        let id = tx.id();
        for (i, o) in tx.outputs.iter().enumerate() {
            self.utxos.insert(
                TransactionOutpoint { transaction_id: id, index: i as u32 },
                UtxoEntry::new(o.value, o.script_public_key.clone(), self.daa, false, o.covenant.map(|c| c.covenant_id)),
            );
        }
        // 10 Blöcke bei 10 BPS: eine Sekunde
        self.daa += 10;
        self.now_ms += 1_000;
        Ok(())
    }
}

// ------------------------------------------------------- Version 4 ----

use crate::contracts::{OracleParams, OracleState, RegisterParams, RegisterState, SignerSet};
use crate::ops::{self, CovValues, Deployment, Tracked};

/// Register und Orakel v4, angelegt und initialisiert
pub struct Feed {
    pub register_params: RegisterParams,
    pub register: Tracked<RegisterState>,
    pub signer_set: SignerSet,
    pub oracle_params: OracleParams,
    pub oracle: Tracked<OracleState>,
}

impl Feed {
    /// Deployment ohne Factory-Init (Factory, GHOST und Vault-Parameter folgen)
    pub fn deployment(self, factory_params: crate::contracts::FactoryParams, factory: Tracked<crate::contracts::FactoryState>) -> Deployment {
        Deployment {
            network: "sim".into(),
            register_params: self.register_params,
            register: self.register,
            signer_set: self.signer_set,
            fallback_set: None,
            rotation: None,
            old_tickets: vec![],
            foreign_change: None,
            signers_unknown: false,
            oracle_params: self.oracle_params,
            oracle: self.oracle,
            factory_params,
            factory,
            ghost_root: None,
            vault_params: None,
            vaults: vec![],
            tokens: vec![],
            pool: None,
            pool_pending: None,
            lp_tokens: vec![],
            pool_unresolved: None,
        }
    }
}

/// Testparameter: Grenzen 1-von-1, Wartezeiten 1 h, Einfrieren nach 2 h
pub fn test_register_params(deployer: &Keypair) -> RegisterParams {
    RegisterParams { deployer: xonly(deployer), min_signers: 1, min_threshold: 1, rot_delay_daa: 36_000, emerg_after_daa: 36_000, emerg_delay_daa: 36_000 }
}

impl Sim {
    /// Register-Genesis, Orakel-Genesis, Register-init (drei Tx, wie das
    /// Deployment). `kas_usd`/`rate` = Startpreis und -zins; der Zinsschritt
    /// ist für Tests nicht begrenzt (rate_step = max_rate).
    pub fn deploy_feed(&mut self, deployer: &Keypair, rp: RegisterParams, set: SignerSet, kas_usd: i64, rate: i64, max_rate: i64) -> Result<Feed, String> {
        self.deploy_feed_with(deployer, rp, set, kas_usd, rate, (max_rate, max_rate, 600))
    }

    /// Wie `deploy_feed` mit Zinsrahmen (max_rate, rate_step, rate_gap_daa)
    pub fn deploy_feed_with(&mut self, deployer: &Keypair, rp: RegisterParams, set: SignerSet, kas_usd: i64, rate: i64, rates: (i64, i64, i64)) -> Result<Feed, String> {
        let (max_rate, rate_step, rate_gap_daa) = rates;
        let net = self.params.clone();
        // wie das Deployment: je 1 KAS (Pool- und Vault-Tests prüfen damit die Speichermasse)
        let vals = CovValues::small();
        let rs = RegisterState::genesis(&set, None, 0, xonly(deployer).try_into().unwrap(), self.daa as i64);
        let (b, reg) = ops::deploy_register(&rp, rs, vals.register, &self.funds(deployer), &net)?;
        self.submit(&b)?;
        let op = OracleParams { reg_cov: reg.cov, max_rate, rate_step, rate_gap_daa, freeze_after_daa: 72_000 };
        let os = OracleState { kas_usd, oracle_daa: self.daa as i64, seq: 0, stable_rate: rate, stable_index: 1_000_000_000, frozen: false, last_rate_daa: self.daa as i64 };
        let (b, oracle) = ops::deploy_oracle(&op, os, vals.oracle, &self.funds(deployer), &net)?;
        self.submit(&b)?;
        let (b, reg) = ops::init_register(&rp, &reg, &set, deployer, &op, &oracle, &self.funds(deployer), &net)?;
        self.submit(&b)?;
        Ok(Feed { register_params: rp, register: reg, signer_set: set, oracle_params: op, oracle })
    }

    /// Komitee als Satz (t von n, tRot = t)
    pub fn signer_set(committee: &[Keypair], t: i64) -> SignerSet {
        SignerSet { keys: committee.iter().map(xonly).collect(), t, t_rot: t }
    }
}
