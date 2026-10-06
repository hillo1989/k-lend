//! Angriffe auf contracts/vault_factory.sil. Jede Angriffs-Tx wird aus der
//! ehrlichen openVault-Tx abgeleitet (eine Stelle verbogen); erwartet wird,
//! dass genau der Factory-Input (0) scheitert. Die ehrliche Variante muss
//! durchgehen (Gegenprobe), damit ein Fehlschlag wirklich an der Regel liegt.
//! Nach dem Mutationstest vom 28.09.2026 hat jede Regel einen eigenen Fall.

use kaspa_consensus_core::Hash;
use kaspa_consensus_core::hashing::covenant_id::covenant_id;
use kaspa_consensus_core::tx::{CovenantBinding, ScriptPublicKey, TransactionOutpoint, TransactionOutput, UtxoEntry};
use kaspa_lending_protocol::contracts::*;
use kaspa_lending_protocol::math::E8;
use kaspa_lending_protocol::ops::{p2pk_spk, self, BRANCH_VALUE, Deployment, xonly};
use kaspa_lending_protocol::sim::{Sim, test_register_params};
use kaspa_lending_protocol::txb::{Draft, In, Unlock, build};
use kaspa_txscript::opcodes::codes::OpTrue;
use rand::{RngCore, thread_rng};
use secp256k1::{Keypair, Secp256k1, SecretKey};
use silverscript_abi::ArtifactValue;

fn key() -> Keypair {
    let secp = Secp256k1::new();
    let mut sk = [0u8; 32];
    loop {
        thread_rng().fill_bytes(&mut sk);
        if let Ok(s) = SecretKey::from_slice(&sk) {
            return Keypair::from_secret_key(&secp, &s);
        }
    }
}

struct Deployed {
    sim: Sim,
    deployer: Keypair,
    op: OracleParams,
    oracle: ops::Tracked<OracleState>,
    fp: FactoryParams,
    factory: ops::Tracked<FactoryState>,
    feed: kaspa_lending_protocol::sim::Feed,
}

fn deployed() -> Deployed {
    let mut sim = Sim::new();
    let deployer = key();
    sim.faucet(&deployer, 1_000 * E8 as u64);
    let net = sim.params.clone();
    let feed = sim.deploy_feed(&deployer, test_register_params(&deployer), SignerSet { keys: (0..5).map(|_| xonly(&key())).collect(), t: 3, t_rot: 3 }, 4_000_000, 0, 1_000_000_000).expect("Register und Orakel");
    let op = feed.oracle_params.clone();
    let oracle = feed.oracle.clone();
    let fp = FactoryParams { deployer: xonly(&deployer), ghost_tpl: ghost_template() };
    let (b, factory) = ops::deploy_factory(&fp, 100_000_000, &sim.funds(&deployer), &net).unwrap();
    sim.submit(&b).unwrap();
    Deployed { sim, deployer, op, oracle, fp, factory, feed }
}

fn initialized() -> (Sim, Deployment, Keypair) {
    let mut d = deployed();
    let net = d.sim.params.clone();
    let (b, f2, root, vp) = ops::init_factory(&d.op, &d.oracle, &d.fp, &d.factory, &d.deployer, (20_000, 15_000, 1_000, NO_DEBT_LIMIT), spk_bytes(&p2pk_spk(&xonly(&d.deployer))), 100_000_000, &d.sim.funds(&d.deployer), &net).unwrap();
    d.sim.submit(&b).unwrap();
    let dep = Deployment {
        network: "sim".into(),
        register_params: d.feed.register_params.clone(), register: d.feed.register.clone(), signer_set: d.feed.signer_set.clone(), fallback_set: None, rotation: None, old_tickets: vec![], foreign_change: None, signers_unknown: false, oracle_params: d.op,
        oracle: d.oracle,
        factory_params: d.fp,
        factory: f2,
        ghost_root: Some(root),
        vault_params: Some(vp),
        vaults: vec![],
        tokens: vec![],
        pool: None,
        pool_pending: None,
        lp_tokens: vec![],
        pool_unresolved: None,
    };
    (d.sim, dep, d.deployer)
}

fn cov_out(art: &Artifact, value: u64, auth: u16, cov: Hash) -> TransactionOutput {
    TransactionOutput { value, script_public_key: spk(art), covenant: Some(CovenantBinding { authorizing_input: auth, covenant_id: cov }) }
}

fn optrue() -> ScriptPublicKey {
    ScriptPublicKey::new(0, vec![OpTrue].into())
}

fn ok_or_factory_err(r: Result<(), String>) -> Result<(), String> {
    match r {
        Ok(()) => Ok(()),
        Err(e) if e.contains("Input 0") => Err(e),
        Err(e) => panic!("Tx scheitert, aber nicht an der Factory (Input 0): {e}"),
    }
}

// ---------------------------------------------------------------- init ----

#[test]
fn init_nur_durch_deployer() {
    let d = deployed();
    let thief = key();
    let r = ops::init_factory(&d.op, &d.oracle, &d.fp, &d.factory, &thief, (20_000, 15_000, 1_000, NO_DEBT_LIMIT), spk_bytes(&p2pk_spk(&xonly(&thief))), 100_000_000, &d.sim.funds(&d.deployer), &d.sim.params);
    assert!(ok_or_factory_err(r.map(|_| ())).is_err());
}

#[test]
fn init_nur_einmal_auch_nicht_durch_den_deployer() {
    // L72: Nach init hat auch der Deployer keine Rechte mehr (sonst könnte er
    // Vault-Template oder GHOST-ID austauschen).
    let (sim, dep, deployer) = initialized();
    let r = ops::init_factory(&dep.oracle_params, &dep.oracle, &dep.factory_params, &dep.factory, &deployer, (10_000, 10_000, 0, NO_DEBT_LIMIT), spk_bytes(&p2pk_spk(&xonly(&deployer))), 100_000_000, &sim.funds(&deployer), &sim.params);
    assert!(ok_or_factory_err(r.map(|_| ())).is_err());
}

// ----------------------------------------------------------- openVault ----

/// Stellschrauben gegenüber der ehrlichen openVault-Tx.
#[derive(Default)]
struct V {
    vault_art: Option<Artifact>,
    vault_value: Option<u64>,
    /// Vault-Ausgang ohne Covenant-Bindung
    vault_unbound: bool,
    /// Vault-Ausgang als Fortsetzung dieses (Angreifer-)Covenants statt Genesis
    vault_as_continuation_of: Option<(TransactionOutpoint, UtxoEntry)>,
    factory_value: Option<u64>,
    o0: Option<GhostTok>,
    o1: Option<GhostTok>,
    extra_ghost_out: Option<GhostTok>,
    /// beliebiger zusätzlicher Ausgang am Ende
    extra_out: Option<TransactionOutput>,
    /// Hintertür: zusätzliche OpTrue-Ausgabe in DERSELBEN Genesis-Gruppe wie der Vault
    backdoor: bool,
    /// KAS-Wert der Wurzel-Minter-Fortsetzung
    root_value: Option<u64>,
}

fn open(sim: &Sim, dep: &Deployment, payer: &Keypair, v: V) -> Result<(), String> {
    let owner = xonly(payer);
    let vp = dep.vault_params.as_ref().unwrap();
    let root = dep.ghost_root.as_ref().unwrap();
    let f = &dep.factory;
    let fart = factory(&dep.factory_params, &f.state);
    let rart = root.state.artifact();
    let honest_vault = vault(vp, &owner, &VaultState::default());
    let vtpl = Template::of(&honest_vault);
    let vault_art = v.vault_art.unwrap_or(honest_vault);

    let mut inputs = vec![
        In {
            outpoint: f.outpoint,
            entry: UtxoEntry::new(f.value, spk(&fart), 0, false, Some(f.cov)),
            unlock: Unlock::Entry { art: fart.clone(), entry: "openVault", args: vec![], sig_at: None }, // Argumente unten
        },
        In { outpoint: root.outpoint, entry: UtxoEntry::new(root.value, spk(&rart), 0, false, Some(root.cov)), unlock: Unlock::Raw(vec![]) },
    ];

    // Vault-Ausgang (Index 1)
    let mut vault_out = TransactionOutput { value: v.vault_value.unwrap_or(1_000 * E8 as u64), script_public_key: spk(&vault_art), covenant: None };
    let vid = if v.vault_unbound {
        Hash::from_bytes([0; 32])
    } else if let Some((op, e)) = &v.vault_as_continuation_of {
        let x = e.covenant_id.unwrap();
        inputs.push(In { outpoint: *op, entry: e.clone(), unlock: Unlock::Raw(vec![]) });
        vault_out.covenant = Some(CovenantBinding { authorizing_input: (inputs.len() - 1) as u16, covenant_id: x });
        x
    } else if v.backdoor {
        // Index der Hintertür: nach Factory, Vault, 2 GHOST-Ausgängen und optionalen Extras
        let idx = 4 + v.extra_ghost_out.is_some() as u32 + v.extra_out.is_some() as u32;
        let door = TransactionOutput { value: E8 as u64, script_public_key: optrue(), covenant: None };
        let id = covenant_id(f.outpoint, [(1u32, &vault_out), (idx, &door)].into_iter());
        vault_out.covenant = Some(CovenantBinding { authorizing_input: 0, covenant_id: id });
        id
    } else {
        let id = covenant_id(f.outpoint, std::iter::once((1u32, &vault_out)));
        vault_out.covenant = Some(CovenantBinding { authorizing_input: 0, covenant_id: id });
        id
    };

    let o0 = v.o0.unwrap_or(root.state.clone());
    let o1 = v.o1.unwrap_or(GhostTok::minter_of(&vid));
    let mut ghost_outs = vec![o0, o1];
    if let Some(x) = v.extra_ghost_out {
        ghost_outs.push(x);
    }
    let states: Vec<ArtifactValue> = ghost_outs.iter().map(GhostTok::arg).collect();
    inputs[0].unlock = Unlock::Entry {
        art: fart.clone(),
        entry: "openVault",
        args: vec![
            ArtifactValue::Bytes(owner.clone()),
            ArtifactValue::Int(1),
            ArtifactValue::Bytes(vtpl.prefix),
            ArtifactValue::Bytes(vtpl.suffix),
            ArtifactValue::Array(states.clone()),
        ],
        sig_at: None,
    };
    inputs[1].unlock = Unlock::Leader { art: rart.clone(), new_states: states, signer: None };
    for (op, e) in &sim.funds(payer).utxos {
        inputs.push(In { outpoint: *op, entry: e.clone(), unlock: Unlock::P2pk { signer: (*payer).into() } });
    }
    let mut outputs = vec![cov_out(&fart, v.factory_value.unwrap_or(f.value), 0, f.cov), vault_out];
    for (n, t) in ghost_outs.iter().enumerate() {
        outputs.push(cov_out(&t.artifact(), if n == 0 { v.root_value.unwrap_or(root.value) } else { BRANCH_VALUE }, 1, root.cov));
    }
    if let Some(x) = v.extra_out {
        outputs.push(x);
    }
    if v.backdoor {
        outputs.push(TransactionOutput { value: E8 as u64, script_public_key: optrue(), covenant: Some(CovenantBinding { authorizing_input: 0, covenant_id: vid }) });
    }
    let d = Draft { inputs, outputs, change_spk: ops::p2pk_spk(&owner), lock_time: 0 };
    ok_or_factory_err(build(d, &sim.params).map(|_| ()))
}

fn setup() -> (Sim, Deployment, Keypair) {
    let (mut sim, dep, _) = initialized();
    let payer = key();
    sim.faucet(&payer, 5_000 * E8 as u64);
    (sim, dep, payer)
}

/// Angriff und Gegenprobe auf DEMSELBEN Deployment (setup() erzeugt jedes
/// Mal neue Schlüssel und IDs).
fn rejected(make: impl Fn(&Deployment, &Keypair) -> V) {
    let (sim, dep, payer) = setup();
    assert!(open(&sim, &dep, &payer, V::default()).is_ok(), "Gegenprobe: ehrliche Variante muss gehen");
    assert!(open(&sim, &dep, &payer, make(&dep, &payer)).is_err(), "Factory muss ablehnen");
}

#[test]
fn ehrliche_variante_geht() {
    let (sim, dep, payer) = setup();
    open(&sim, &dep, &payer, V::default()).expect("ehrlich");
}

#[test]
fn falsches_vault_template() {
    rejected(|dep, payer| {
        let mut vp = dep.vault_params.clone().unwrap();
        vp.mcr_bps = 10_000;
        V { vault_art: Some(vault(&vp, &xonly(payer), &VaultState::default())), ..Default::default() }
    });
}

#[test]
fn vault_mit_anfangsschuld() {
    rejected(|dep, payer| V { vault_art: Some(vault(dep.vault_params.as_ref().unwrap(), &xonly(payer), &VaultState { debt: 5, ..Default::default() })), ..Default::default() });
}

#[test]
fn vault_ueber_maximaler_sicherheit() {
    // L93
    let (mut sim, dep, payer) = setup();
    sim.faucet(&payer, 110_000_000 * E8 as u64);
    assert!(open(&sim, &dep, &payer, V { vault_value: Some(100_000_000 * E8 as u64), ..Default::default() }).is_ok());
    assert!(open(&sim, &dep, &payer, V { vault_value: Some(100_000_000 * E8 as u64 + 1), ..Default::default() }).is_err());
}

#[test]
fn vault_ohne_covenant_bindung() {
    // L58: ID 0 = gar kein Covenant
    rejected(|_, _| V { vault_unbound: true, o1: Some(GhostTok::minter_of(&Hash::from_bytes([0; 32]))), ..Default::default() });
}

#[test]
fn vault_als_fortsetzung_eines_angreifer_covenants() {
    // L59: Der Angreifer legt vorher einen eigenen Covenant X mit ZWEI
    // OpTrue-UTXOs an. Eine davon wird zum "Vault", die andere bliebe als
    // Hintertür bestehen und könnte den Minter-Zweig von X jederzeit bedienen.
    let (mut sim, dep, payer) = setup();
    let first = sim.funds(&payer).utxos[0].0;
    let mut a = TransactionOutput { value: 10 * E8 as u64, script_public_key: optrue(), covenant: None };
    let mut b = a.clone();
    let x = covenant_id(first, [(0u32, &a), (1u32, &b)].into_iter());
    a.covenant = Some(CovenantBinding { authorizing_input: 0, covenant_id: x });
    b.covenant = Some(CovenantBinding { authorizing_input: 0, covenant_id: x });
    let fund = sim.funds(&payer);
    let inputs = fund.utxos.iter().map(|(op, e)| In { outpoint: *op, entry: e.clone(), unlock: Unlock::P2pk { signer: (payer).into() } }).collect();
    let bx = build(Draft { inputs, outputs: vec![a, b], change_spk: ops::p2pk_spk(&xonly(&payer)), lock_time: 0 }, &sim.params).unwrap();
    sim.submit(&bx).unwrap();
    let op0 = TransactionOutpoint { transaction_id: bx.tx.id(), index: 0 };
    let e0 = sim.utxos[&op0].clone();
    assert!(open(&sim, &dep, &payer, V::default()).is_ok(), "Gegenprobe");
    assert!(open(&sim, &dep, &payer, V { vault_as_continuation_of: Some((op0, e0)), ..Default::default() }).is_err());
}

#[test]
fn factory_kas_duerfen_nicht_abfliessen() {
    // L50
    rejected(|dep, _| V { factory_value: Some(dep.factory.value - 1), ..Default::default() });
}

// Wurzel-Minter-Fortsetzung (o0)

#[test]
fn wurzel_minter_an_fremden_covenant() {
    // L111: das Prägerecht der Factory an eine andere Covenant-ID
    rejected(|_, _| V { o0: Some(GhostTok { owner: vec![7; 32], typ: ID_COV, amount: 0, minter: true }), ..Default::default() });
}

#[test]
fn wurzel_minter_mit_falschem_typ() {
    // L110
    rejected(|dep, _| V { o0: Some(GhostTok { owner: dep.factory.cov.as_bytes().to_vec(), typ: ID_PUBKEY, amount: 0, minter: true }), ..Default::default() });
}

#[test]
fn wurzel_minter_verliert_minterrecht() {
    // L109
    rejected(|dep, _| V { o0: Some(GhostTok { owner: dep.factory.cov.as_bytes().to_vec(), typ: ID_COV, amount: 0, minter: false }), ..Default::default() });
}

#[test]
fn wurzel_minter_mit_guthaben() {
    // L112
    rejected(|dep, _| V { o0: Some(GhostTok { owner: dep.factory.cov.as_bytes().to_vec(), typ: ID_COV, amount: 5, minter: true }), ..Default::default() });
}

// Minter-Zweig des neuen Vaults (o1) — die ID ist erst in open() bekannt,
// daher werden hier nur Felder außer dem Besitzer verbogen.

fn o1_with(f: impl Fn(&mut GhostTok)) -> impl Fn(&Sim, &Deployment, &Keypair) -> Result<(), String> {
    move |sim, dep, payer| {
        // Vault-ID wie in open() berechnen
        let vp = dep.vault_params.as_ref().unwrap();
        let out = TransactionOutput { value: 1_000 * E8 as u64, script_public_key: spk(&vault(vp, &xonly(payer), &VaultState::default())), covenant: None };
        let vid = covenant_id(dep.factory.outpoint, std::iter::once((1u32, &out)));
        let mut t = GhostTok::minter_of(&vid);
        f(&mut t);
        open(sim, dep, payer, V { o1: Some(t), ..Default::default() })
    }
}

fn rejected_o1(f: impl Fn(&mut GhostTok)) {
    let (sim, dep, payer) = setup();
    assert!(o1_with(|_| {})(&sim, &dep, &payer).is_ok(), "Gegenprobe");
    assert!(o1_with(f)(&sim, &dep, &payer).is_err(), "Factory muss ablehnen");
}

#[test]
fn minter_zweig_fuer_fremde_id() {
    // L116
    rejected_o1(|t| t.owner = vec![7; 32]);
}

#[test]
fn minter_zweig_ohne_minterrecht() {
    // L114
    rejected_o1(|t| t.minter = false);
}

#[test]
fn minter_zweig_mit_falschem_typ() {
    // L115
    rejected_o1(|t| t.typ = ID_PUBKEY);
}

#[test]
fn minter_zweig_mit_vorab_guthaben() {
    // L117: ein Zweig mit Guthaben würde beim ersten Prägen mehr ausgeben,
    // als als Schuld gebucht wird (der Vault zählt nur die Differenz)
    rejected_o1(|t| t.amount = 1_000_000 * E8);
}

#[test]
fn dritter_ghost_ausgang_mit_minterrecht() {
    // L106 + L107 (sichern sich gegenseitig ab): ein zusätzlicher
    // Minter-Zweig für einen Angreifer
    rejected(|_, _| V { extra_ghost_out: Some(GhostTok { owner: xonly(&key()), typ: ID_PUBKEY, amount: 0, minter: true }), ..Default::default() });
}

#[test]
fn zweite_fortsetzung_der_factory() {
    // L48: zusätzliche Factory-UTXO mit anderem Vault-Template
    rejected(|dep, _| {
        let f = &dep.factory;
        let fake = FactoryState { ghost_cov: f.state.ghost_cov, vault_hash: vec![9; 32], initialized: true };
        V { extra_out: Some(cov_out(&factory(&dep.factory_params, &fake), E8 as u64, 0, f.cov)), ..Default::default() }
    });
}

#[test]
fn hintertuer_mit_derselben_genesis_id_scheitert() {
    // L64: Genesis-Gruppe {Vault, OpTrue-Ausgabe} mit gemeinsamer ID – der
    // Konsens akzeptiert das, die OpTrue-UTXO könnte später den Minter-Zweig
    // des Vaults bedienen. Die Factory muss ablehnen. (Test-Audit 5, 28.09.2026:
    // dieser Test war bei einer Überarbeitung verloren gegangen.)
    rejected(|_, _| V { backdoor: true, ..Default::default() });
}

#[test]
fn wurzel_minter_behaelt_seine_kas() {
    // Fix-Review N-11: vorher ließen sich beim openVault die 10 KAS des
    // Wurzel-Minters bis auf 1 sompi abziehen
    rejected(|dep, _| V { root_value: Some(dep.ghost_root.as_ref().unwrap().value - 1), ..Default::default() });
}
