//! Vault v5 (contracts/stable_vault_v5.sil) mit Orakel v5 und GHOST in der
//! echten Skript-Engine: Zwei-Preis-Regel je Eintrag (A20e-1), sweep-Grenze
//! (A20a-3), Abwicklung bei dauerhaft eingefrorenem Orakel (A20e-2). Jede
//! Regel mit Angriff und ehrlicher Gegenprobe; geprüft wird, WELCHER Eingang
//! ablehnt (der Vault, Eingang 0).

mod v5common;

use kaspa_consensus_core::Hash;
use kaspa_consensus_core::tx::{CovenantBinding, TransactionOutput};
use kaspa_txscript::pay_to_script_hash_script;
use secp256k1::Keypair;
use silverscript_abi::{ArtifactValue, SilAbiArtifact};
use silverscript_lang::compiler::{CompileOptions, compile_to_sil_abi_artifact_with_options};
use std::collections::BTreeMap;
use v5common::common::{bytecode, compiled_template_parts_and_hash, execute_input_with_covenants, push_redeem_script};
use v5common::*;

const VAULT_SRC: &str = include_str!("../../contracts/stable_vault_v5.sil");
const KCC20_SRC: &str = include_str!("../../contracts/ghost_token.sil");
const GHOST_COV: Hash = Hash::from_bytes([0x0b; 32]);
const VAULT_COV: Hash = Hash::from_bytes([0x0c; 32]);

const MCR: i64 = 20_000;
const LIQ: i64 = 15_000;
const BONUS: i64 = 1_000;
const ID_PUBKEY: u8 = 0x00;
const ID_COV: u8 = 0x02;
const PRICE: i64 = 4_000_000; // 0,04 USD
const INDEX: i64 = 1_000_000_000;
const SETTLE_AFTER: i64 = 60 * DAY;
const SETTLE_FEE: i64 = 500;
const SWEEP_FEE: i64 = 10_000_000;
const DUST: i64 = 20_000_000;

/// Orakelzustand für Vault-Tests: aktueller Preis, Referenz, eingefroren, DAA
#[derive(Clone, Copy)]
struct O {
    kas: i64,
    refp: i64,
    frozen: bool,
    daa: i64,
}

impl O {
    fn honest(kas: i64) -> Self {
        Self { kas, refp: kas, frozen: false, daa: 1_000_000 }
    }
    fn st(self) -> OSt {
        OSt { kas_usd: self.kas, ref_usd: self.refp, frozen: self.frozen, daa: self.daa, index: INDEX, cand_usd: self.kas, ..OSTART }
    }
}

struct Env {
    owner: Keypair,
    treasury: Keypair,
    oracle_tpl: (Vec<u8>, Vec<u8>, Vec<u8>),
    ghost_tpl: (Vec<u8>, Vec<u8>, Vec<u8>),
}

#[derive(Clone, Copy, Debug)]
struct St {
    debt: i64,
    interest: i64,
}

fn p2pk(x: &[u8]) -> kaspa_consensus_core::tx::ScriptPublicKey {
    let mut s = vec![0x20];
    s.extend_from_slice(x);
    s.push(0xac);
    kaspa_consensus_core::tx::ScriptPublicKey::new(0, s.into())
}

fn spk_bytes(s: &kaspa_consensus_core::tx::ScriptPublicKey) -> Vec<u8> {
    let mut v = s.version().to_be_bytes().to_vec();
    v.extend_from_slice(s.script());
    v
}

impl Env {
    fn new() -> Self {
        let mut e = Self { owner: random_keypair(), treasury: random_keypair(), oracle_tpl: Default::default(), ghost_tpl: Default::default() };
        e.oracle_tpl = compiled_template_parts_and_hash(&oracle_art(REG_COV, OCFG, OSTART));
        e.ghost_tpl = compiled_template_parts_and_hash(&e.ghost(vec![0; 32], ID_COV, 0, true));
        e
    }
    fn oracle(&self, o: O) -> SilAbiArtifact {
        oracle_art(REG_COV, OCFG, o.st())
    }
    fn ghost(&self, owner: Vec<u8>, typ: u8, amount: i64, minter: bool) -> SilAbiArtifact {
        compile_to_sil_abi_artifact_with_options(
            KCC20_SRC,
            &[ArtifactValue::Bytes(owner), ArtifactValue::Int(amount), ArtifactValue::Byte(typ), ArtifactValue::Bool(minter), ArtifactValue::Int(3), ArtifactValue::Int(2)],
            CompileOptions::default(),
        )
        .unwrap()
    }
    fn vault(&self, s: St) -> SilAbiArtifact {
        let (op, os, oh) = &self.oracle_tpl;
        let (kp, ks, kh) = &self.ghost_tpl;
        compile_to_sil_abi_artifact_with_options(
            VAULT_SRC,
            &[
                ArtifactValue::Bytes(ORACLE_COV.as_bytes().to_vec()),
                ArtifactValue::Int(op.len() as i64),
                ArtifactValue::Int(os.len() as i64),
                ArtifactValue::Bytes(oh.clone()),
                ArtifactValue::Bytes(GHOST_COV.as_bytes().to_vec()),
                ArtifactValue::Int(kp.len() as i64),
                ArtifactValue::Int(ks.len() as i64),
                ArtifactValue::Bytes(kh.clone()),
                ArtifactValue::Int(MCR),
                ArtifactValue::Int(LIQ),
                ArtifactValue::Int(BONUS),
                ArtifactValue::Int(1_000 * E8), // Höchstschuld je Vault (hier nicht Gegenstand)
                ArtifactValue::Bytes(spk_bytes(&p2pk(&xonly(&self.treasury)))),
                ArtifactValue::Int(SETTLE_AFTER),
                ArtifactValue::Int(SETTLE_FEE),
                ArtifactValue::Bytes(xonly(&self.owner)),
                ArtifactValue::Int(s.debt),
                ArtifactValue::Int(s.interest),
                ArtifactValue::Int(INDEX),
            ],
            CompileOptions::default(),
        )
        .expect("Vault v5 kompiliert")
    }
}

fn tok(owner: &[u8], typ: u8, amount: i64, minter: bool) -> ArtifactValue {
    BTreeMap::from([
        ("ownerIdentifier".to_string(), ArtifactValue::Bytes(owner.to_vec())),
        ("identifierType".to_string(), ArtifactValue::Byte(typ)),
        ("amount".to_string(), ArtifactValue::Int(amount)),
        ("isMinter".to_string(), ArtifactValue::Bool(minter)),
    ])
    .into()
}

fn cout(art: &SilAbiArtifact, value: i64, auth: u16, cov: Hash) -> TransactionOutput {
    TransactionOutput {
        value: value as u64,
        script_public_key: pay_to_script_hash_script(&bytecode(art)),
        covenant: Some(CovenantBinding { authorizing_input: auth, covenant_id: cov }),
    }
}

/// Eingänge mit Tx-Signatur (Besitzer bzw. Token-Delegate). Eigener Bau, weil
/// v5common nur Datensignaturen kennt.
enum C {
    Entry(SilAbiArtifact, &'static str, Vec<ArtifactValue>, Option<(usize, Keypair)>),
    Leader(SilAbiArtifact, Vec<ArtifactValue>),
    Delegate(SilAbiArtifact, Keypair),
}

fn run(inputs: Vec<(kaspa_consensus_core::tx::UtxoEntry, C)>, outputs: Vec<TransactionOutput>, lock: u64) -> Vec<Result<(), String>> {
    use kaspa_consensus_core::hashing::sighash::{SigHashReusedValuesUnsync, calc_schnorr_signature_hash};
    use kaspa_consensus_core::hashing::sighash_type::SIG_HASH_ALL;
    use kaspa_consensus_core::tx::{MutableTransaction, Transaction, TransactionId, TransactionInput, TransactionOutpoint};
    use silverscript_abi::{encode_contract_covenant_decl_sig_script, encode_contract_entry_sig_script};
    let entries: Vec<_> = inputs.iter().map(|(u, _)| u.clone()).collect();
    let op = |i: usize| TransactionOutpoint { transaction_id: TransactionId::from_bytes([0x40 + i as u8; 32]), index: i as u32 };
    let bare: Vec<_> = (0..inputs.len()).map(|i| TransactionInput::new_with_compute_budget(op(i), vec![], 0, 0)).collect();
    let unsigned = Transaction::new(1, bare, outputs.clone(), lock, Default::default(), 0, vec![]);
    let sign = |idx: usize, k: &Keypair| {
        let mtx = MutableTransaction::with_entries(unsigned.clone(), entries.clone());
        let h = calc_schnorr_signature_hash(&mtx.as_verifiable(), idx, SIG_HASH_ALL, &SigHashReusedValuesUnsync::new());
        let mut s = k.sign_schnorr(secp256k1::Message::from_digest_slice(h.as_bytes().as_slice()).unwrap()).as_ref().to_vec();
        s.push(SIG_HASH_ALL.to_u8());
        s
    };
    let mut fin = vec![];
    for (idx, (_, c)) in inputs.into_iter().enumerate() {
        let (art, mut s) = match c {
            C::Entry(art, entry, mut args, sig) => {
                if let Some((pos, k)) = sig {
                    args.insert(pos, ArtifactValue::Bytes(sign(idx, &k)));
                }
                let s = encode_contract_entry_sig_script(&art, &contract_name(&art), entry, &args).unwrap();
                (art, s)
            }
            C::Leader(art, states) => {
                let args = vec![ArtifactValue::Array(states), ArtifactValue::Bytes(vec![0; 65]), ArtifactValue::Byte(0)];
                let s = encode_contract_covenant_decl_sig_script(&art, &contract_name(&art), "transfer", true, &args).unwrap();
                (art, s)
            }
            C::Delegate(art, k) => {
                let args = vec![ArtifactValue::Bytes(sign(idx, &k)), ArtifactValue::Byte(0)];
                let s = encode_contract_covenant_decl_sig_script(&art, &contract_name(&art), "transfer", false, &args).unwrap();
                (art, s)
            }
        };
        s.extend_from_slice(&push_redeem_script(&bytecode(&art)));
        fin.push(TransactionInput::new_with_compute_budget(op(idx), s, 0, 0));
    }
    let tx = Transaction::new(1, fin, outputs, lock, Default::default(), 0, vec![]);
    (0..tx.inputs.len()).map(|i| execute_input_with_covenants(tx.clone(), entries.clone(), i).map_err(|e| format!("{e:?}"))).collect()
}

fn u(art: &SilAbiArtifact, v: i64, cov: Hash) -> kaspa_consensus_core::tx::UtxoEntry {
    cov_utxo(art, v, cov)
}

/// mint `amount` aus einem Vault mit Schuld `debt`
fn mint(e: &Env, debt: i64, amount: i64, coll: i64, o: O) -> Vec<Result<(), String>> {
    let rcpt = random_keypair();
    let outs = vec![tok(VAULT_COV.as_bytes().as_slice(), ID_COV, 0, true), tok(&xonly(&rcpt), ID_PUBKEY, amount, false)];
    let v = e.vault(St { debt, interest: 0 });
    let minter = e.ghost(VAULT_COV.as_bytes().to_vec(), ID_COV, 0, true);
    let oa = e.oracle(o);
    run(
        vec![
            (u(&v, coll, VAULT_COV), C::Entry(v.clone(), "mint", vec![i(amount), i(1), ArtifactValue::Array(outs.clone())], Some((3, e.owner)))),
            (u(&oa, E8, ORACLE_COV), C::Entry(oa.clone(), "read", vec![], None)),
            (u(&minter, 1_000, GHOST_COV), C::Leader(minter.clone(), outs)),
        ],
        vec![
            cout(&e.vault(St { debt: debt + amount, interest: 0 }), coll, 0, VAULT_COV),
            cout(&oa, E8, 1, ORACLE_COV),
            cout(&minter, 1_000, 2, GHOST_COV),
            cout(&e.ghost(xonly(&rcpt), ID_PUBKEY, amount, false), 1_000, 2, GHOST_COV),
        ],
        o.daa as u64,
    )
}

/// redeem/liquidate: `amount` GHOST verbrennen; Vault danach (Zustand, KAS) oder Ende
fn burn(e: &Env, entry: &'static str, debt: i64, amount: i64, coll: i64, after: Option<(i64, i64)>, o: O, lock: i64) -> Vec<Result<(), String>> {
    let payer = random_keypair();
    let v = e.vault(St { debt, interest: 0 });
    let minter = e.ghost(VAULT_COV.as_bytes().to_vec(), ID_COV, 0, true);
    let pay = e.ghost(xonly(&payer), ID_PUBKEY, amount, false);
    let outs = vec![tok(VAULT_COV.as_bytes().as_slice(), ID_COV, 0, true)];
    let oa = e.oracle(o);
    let mut outputs = vec![];
    if let Some((d, c)) = after {
        outputs.push(cout(&e.vault(St { debt: d, interest: 0 }), c, 0, VAULT_COV));
    }
    outputs.push(cout(&oa, E8, 1, ORACLE_COV));
    outputs.push(cout(&minter, 1_000, 2, GHOST_COV));
    outputs.push(plain_out(1));
    run(
        vec![
            (u(&v, coll, VAULT_COV), C::Entry(v.clone(), entry, vec![i(1), i(amount), ArtifactValue::Array(outs.clone())], None)),
            (u(&oa, E8, ORACLE_COV), C::Entry(oa.clone(), "read", vec![], None)),
            (u(&minter, 1_000, GHOST_COV), C::Leader(minter.clone(), outs)),
            (u(&pay, 1_000, GHOST_COV), C::Delegate(pay.clone(), payer)),
        ],
        outputs,
        lock as u64,
    )
}

fn withdraw(e: &Env, debt: i64, coll: i64, new_coll: i64, o: O) -> Vec<Result<(), String>> {
    let v = e.vault(St { debt, interest: 0 });
    let oa = e.oracle(o);
    run(
        vec![
            (u(&v, coll, VAULT_COV), C::Entry(v.clone(), "withdraw", vec![i(new_coll), i(1)], Some((2, e.owner)))),
            (u(&oa, E8, ORACLE_COV), C::Entry(oa.clone(), "read", vec![], None)),
        ],
        vec![cout(&e.vault(St { debt, interest: 0 }), new_coll, 0, VAULT_COV), cout(&oa, E8, 1, ORACLE_COV), plain_out(1)],
        0,
    )
}

fn sweep(e: &Env, interest: i64, coll: i64, to_treasury: i64, o: O) -> Vec<Result<(), String>> {
    let v = e.vault(St { debt: 0, interest });
    let oa = e.oracle(o);
    run(
        vec![(u(&v, coll, VAULT_COV), C::Entry(v.clone(), "sweep", vec![i(1)], None)), (u(&oa, E8, ORACLE_COV), C::Entry(oa.clone(), "read", vec![], None))],
        vec![
            cout(&oa, E8, 1, ORACLE_COV),
            TransactionOutput { value: to_treasury as u64, script_public_key: p2pk(&xonly(&e.treasury)), covenant: None },
            plain_out(1),
        ],
        0,
    )
}

fn value(coll: i64, p: i64) -> i64 {
    (coll as i128 * p as i128 / E8 as i128) as i64
}

/// Auszahlung einer Rücknahme in sompi: amount·(1 − fee) USD zum Preis p
fn paid(amount: i64, fee_bps: i64, p: i64) -> i64 {
    let usd = amount as i128 * (10_000 - fee_bps) as i128 / 10_000;
    (usd * E8 as i128 / p as i128) as i64
}

// ================================================================== mint ----

#[test]
fn mint_braucht_deckung_zu_beiden_preisen() {
    let e = Env::new();
    let coll = 10_000 * E8; // 400 USD bei 0,04
    // ehrlich: 200 GHOST bei 200 %
    assert!(all_ok(&mint(&e, 0, 200 * E8, coll, O::honest(PRICE))));
    assert!(mint(&e, 0, 200 * E8 + 1, coll, O::honest(PRICE))[0].is_err());
    // Angriff A20e-1: aktueller Preis gefälscht ×1,25, Referenz ehrlich –
    // mehr als die ehrlichen 200 GHOST gehen nicht
    let fake = O { kas: PRICE * 5 / 4, ..O::honest(PRICE) };
    assert!(mint(&e, 0, 200 * E8 + 1, coll, fake)[0].is_err());
    assert!(mint(&e, 0, 250 * E8, coll, fake)[0].is_err());
    assert!(all_ok(&mint(&e, 0, 200 * E8, coll, fake)));
    // Referenz gefälscht hoch, aktueller Preis ehrlich: ebenso
    let fake_ref = O { refp: PRICE * 5 / 4, ..O::honest(PRICE) };
    assert!(mint(&e, 0, 200 * E8 + 1, coll, fake_ref)[0].is_err());
    // beide hoch: dann (und erst dann) mehr – das Fenster des Wächters
    let both = O { kas: PRICE * 5 / 4, refp: PRICE * 5 / 4, ..O::honest(PRICE) };
    assert!(all_ok(&mint(&e, 0, 250 * E8, coll, both)));
}

#[test]
fn withdraw_mit_schuld_zu_beiden_preisen() {
    let e = Env::new();
    let coll = 10_000 * E8;
    let debt = 100 * E8; // 400 % ehrlich; Mindestquote erlaubt 5 000 KAS Rest
    assert!(all_ok(&withdraw(&e, debt, coll, 5_000 * E8, O::honest(PRICE))));
    assert!(withdraw(&e, debt, coll, 5_000 * E8 - 1, O::honest(PRICE))[0].is_err());
    let fake = O { kas: PRICE * 2, ..O::honest(PRICE) };
    assert!(withdraw(&e, debt, coll, 2_500 * E8, fake)[0].is_err(), "gefälschter hoher Preis gibt nichts frei");
    assert!(all_ok(&withdraw(&e, debt, coll, 5_000 * E8, fake)));
}

// ============================================================= liquidate ----

#[test]
fn liquidation_braucht_unterdeckung_zu_beiden_preisen() {
    // Beleg A20e-1 (v4): gesunder Vault 1 000 KAS / 20 GHOST bei 0,04335 USD
    // (217 %), Preis gefälscht auf 1/4 – in v4 sofort liquidierbar.
    let e = Env::new();
    let (coll, debt) = (1_000 * E8, 20 * E8);
    let honest = 4_335_083;
    let fake = O { kas: honest / 4, ..O::honest(honest) };
    // gesamte Sicherheit für 0,5 GHOST Burn (Bonus rechnet zum Fälschungspreis)
    let r = burn(&e, "liquidate", debt, 20 * E8, coll, None, fake, 0);
    assert!(r[0].is_err(), "v5: Referenz ehrlich, Vault gesund → keine Liquidation: {r:?}");
    // Gegenprobe: echter Absturz auf 1/4 (beide Preise) → liquidierbar
    let crash = O { kas: honest / 4, refp: honest / 4, ..O::honest(honest) };
    let r = burn(&e, "liquidate", debt, debt, coll, None, crash, 0);
    assert!(all_ok(&r), "{r:?}");
}

#[test]
fn liquidation_rechnet_zum_aktuellen_preis_ab() {
    // echter Sturz, Referenz zieht nach: unter 150 % zu beiden Preisen. Der
    // Liquidator bekommt Schuld + 10 % zum AKTUELLEN Preis (sonst stockte sie)
    let e = Env::new();
    let (coll, debt, burnv) = (1_000 * E8, 20 * E8, 5 * E8);
    let o = O { kas: 2_700_000, refp: 2_900_000, ..O::honest(PRICE) }; // 135 % / 145 %
    let claim = burnv * 11_000 / 10_000; // USD
    let seize = (claim as i128 * E8 as i128).div_euclid(o.kas as i128) as i64 + 1; // aufgerundet
    let r = burn(&e, "liquidate", debt, burnv, coll, Some((debt - burnv, coll - seize)), o, 0);
    assert!(all_ok(&r), "{r:?}");
    // zum höheren (Referenz-)Preis abgerechnet wäre es weniger: abgelehnt
    let seize_ref = (claim as i128 * E8 as i128 / o.refp as i128) as i64 + 1;
    assert!(burn(&e, "liquidate", debt, burnv, coll, Some((debt - burnv, coll - seize_ref)), o, 0)[0].is_err());
    // nur der aktuelle Preis unter der Schwelle (Referenz 155 %): nicht liquidierbar
    let o2 = O { kas: 2_700_000, refp: 3_100_000, ..O::honest(PRICE) };
    assert!(burn(&e, "liquidate", debt, burnv, coll, Some((debt - burnv, coll - seize)), o2, 0)[0].is_err());
}

// ================================================================ redeem ----

#[test]
fn ruecknahme_zahlt_zum_hoeheren_preis() {
    let e = Env::new();
    let (coll, debt, amt) = (1_000 * E8, 10 * E8, 2 * E8); // 400 %
    // Angriff: aktueller Preis gefälscht tief (−20 %) → mehr KAS je GHOST
    let o = O { kas: PRICE * 4 / 5, ..O::honest(PRICE) };
    let at_cur = paid(amt, 100, o.kas);
    let at_ref = paid(amt, 100, o.refp);
    assert!(at_cur > at_ref);
    assert!(burn(&e, "redeem", debt, amt, coll, Some((debt - amt, coll - at_cur)), o, 0)[0].is_err(), "zum Fälschungspreis");
    assert!(all_ok(&burn(&e, "redeem", debt, amt, coll, Some((debt - amt, coll - at_ref)), o, 0)), "zum höheren Preis");
}

#[test]
fn ruecknahme_braucht_schwelle_zu_beiden_preisen() {
    let e = Env::new();
    let (coll, debt, amt) = (1_000 * E8, 25 * E8, 2 * E8); // 160 % bei 0,04
    let o = O::honest(PRICE);
    assert!(all_ok(&burn(&e, "redeem", debt, amt, coll, Some((debt - amt, coll - paid(amt, 100, PRICE))), o, 0)));
    // Referenz tiefer (145 %): gesperrt
    let o2 = O { refp: 3_600_000, ..o };
    assert!(burn(&e, "redeem", debt, amt, coll, Some((debt - amt, coll - paid(amt, 100, PRICE))), o2, 0)[0].is_err());
}

// ================================================================= sweep ----

#[test]
fn a20a_3_sweep_nur_ueber_sweep_fee() {
    let e = Env::new();
    let o = O::honest(PRICE);
    // v4-Befund: Sicherheit 0,05 KAS, Zins 0,003 USD > Wert; Kasse 1 sompi
    let r = sweep(&e, 300_000, 5_000_000, 1, o);
    assert!(r[0].is_err(), "v5 lehnt ab: {r:?}");
    // auch genau SWEEP_FEE: nichts für die Kasse → abgelehnt
    assert!(sweep(&e, 300_000, SWEEP_FEE, 1, o)[0].is_err());
    // Gegenprobe 0,5 KAS: Kasse genau coll − SWEEP_FEE
    let coll = 50_000_000;
    let interest = value(coll, PRICE) + 1_000;
    assert!(all_ok(&sweep(&e, interest, coll, coll - SWEEP_FEE, o)));
    assert!(sweep(&e, interest, coll, coll - SWEEP_FEE - 1, o)[0].is_err());
}

#[test]
fn sweep_rechnet_zum_hoeheren_preis() {
    // gefälschter tiefer Preis ließe die Zinsgebühr die Sicherheit übersteigen
    let e = Env::new();
    let coll = 50_000_000;
    let interest = value(coll, PRICE) * 9 / 10; // 90 % der Sicherheit zum ehrlichen Preis
    let fake = O { kas: PRICE * 4 / 5, ..O::honest(PRICE) };
    assert!(sweep(&e, interest, coll, coll - SWEEP_FEE, fake)[0].is_err());
    let real = O { kas: PRICE * 4 / 5, refp: PRICE * 4 / 5, ..O::honest(PRICE) };
    assert!(all_ok(&sweep(&e, interest, coll, coll - SWEEP_FEE, real)));
}

// ============================================================ Abwicklung ----

fn frozen(daa: i64) -> O {
    O { frozen: true, daa, ..O::honest(PRICE) }
}

#[test]
fn abwicklung_erst_nach_settle_after_bei_eingefrorenem_orakel() {
    let e = Env::new();
    let (coll, debt, amt) = (1_000 * E8, 10 * E8, E8);
    let o = frozen(1_000_000);
    let pay = paid(amt, SETTLE_FEE, PRICE);
    let after = Some((debt - amt, coll - pay));
    assert!(burn(&e, "redeem", debt, amt, coll, after, o, o.daa + SETTLE_AFTER - 1)[0].is_err(), "eine DAA zu früh");
    let r = burn(&e, "redeem", debt, amt, coll, after, o, o.daa + SETTLE_AFTER);
    assert!(all_ok(&r), "{r:?}");
    // ohne Einfrieren: gewöhnliche Rücknahme (1 %), kein Abwicklungstarif
    let live = O { frozen: false, ..o };
    assert!(burn(&e, "redeem", debt, amt, coll, after, live, o.daa + SETTLE_AFTER)[0].is_err());
    assert!(all_ok(&burn(&e, "redeem", debt, amt, coll, Some((debt - amt, coll - paid(amt, 100, PRICE))), live, 0)));
    // vor der Frist ist eine Rücknahme bei eingefrorenem Orakel gesperrt (wie v4)
    assert!(burn(&e, "redeem", debt, amt, coll, Some((debt - amt, coll - paid(amt, 100, PRICE))), o, 0)[0].is_err());
}

#[test]
fn abwicklung_ohne_quotenpruefung_und_ab_kleinen_betraegen() {
    let e = Env::new();
    // 120 %: unter der Liquidationsschwelle, gewöhnliche Rücknahme ginge nicht
    let (coll, debt) = (300 * E8, 10 * E8);
    let o = frozen(1_000_000);
    let lock = o.daa + SETTLE_AFTER;
    for amt in [1_000_000, E8 / 2] {
        let r = burn(&e, "redeem", debt, amt, coll, Some((debt - amt, coll - paid(amt, SETTLE_FEE, PRICE))), o, lock);
        assert!(all_ok(&r), "{amt}: {r:?}");
    }
    assert!(burn(&e, "redeem", debt, 999_999, coll, Some((debt - 999_999, coll - paid(999_999, SETTLE_FEE, PRICE))), o, lock)[0].is_err());
    // zum Fälschungspreis tief (Referenz ehrlich) zahlt sie nicht mehr aus
    let fk = O { kas: PRICE * 4 / 5, ..o };
    assert!(burn(&e, "redeem", debt, E8, coll, Some((debt - E8, coll - paid(E8, SETTLE_FEE, fk.kas))), fk, lock)[0].is_err());
    assert!(all_ok(&burn(&e, "redeem", debt, E8, coll, Some((debt - E8, coll - paid(E8, SETTLE_FEE, PRICE))), fk, lock)));
}

#[test]
fn abwicklung_eines_unterdeckten_vaults_nur_im_ganzen() {
    let e = Env::new();
    // 80 %: Sicherheit 200 KAS = 8 USD, Schuld 10 GHOST
    let (coll, debt) = (200 * E8, 10 * E8);
    let o = frozen(1_000_000);
    let lock = o.daa + SETTLE_AFTER;
    assert!(burn(&e, "redeem", debt, E8, coll, Some((debt - E8, coll - paid(E8, SETTLE_FEE, PRICE))), o, lock)[0].is_err(), "Teil");
    let r = burn(&e, "redeem", debt, debt, coll, None, o, lock);
    assert!(all_ok(&r), "ganze Schuld gegen ganze Sicherheit: {r:?}");
    // gedeckter Vault endet nicht: ganze Schuld mit Vault-Ende abgelehnt
    assert!(burn(&e, "redeem", debt, debt, 1_000 * E8, None, o, lock)[0].is_err());
}
