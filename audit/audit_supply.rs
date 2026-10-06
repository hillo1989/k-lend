//! Auditor-Angriffe auf die Mengenintegrität (Supply). Wiederverwendet die
//! Bausteine aus vault_tests.rs, dupliziert nur das Nötigste. Jeder Input wird
//! ausgeführt; ein Angriff gilt als abgewehrt, wenn genau der Vault (Input 0)
//! ablehnt und wir zeigen, dass die reine Token-Seite den Angriff durchließe.

mod common;
use common::{bytecode, compiled_template_parts_and_hash, execute_input_with_covenants, push_redeem_script};
use kaspa_consensus_core::Hash;
use kaspa_consensus_core::hashing::sighash::{SigHashReusedValuesUnsync, calc_schnorr_signature_hash};
use kaspa_consensus_core::hashing::sighash_type::SIG_HASH_ALL;
use kaspa_consensus_core::tx::{CovenantBinding, MutableTransaction, Transaction, TransactionId, TransactionInput, TransactionOutpoint, TransactionOutput, UtxoEntry, VerifiableTransaction};
use kaspa_txscript::pay_to_script_hash_script;
use rand::{RngCore, thread_rng};
use secp256k1::{Keypair, Message, Secp256k1, SecretKey};
use silverscript_abi::{ArtifactValue, SilAbiArtifact, encode_contract_covenant_decl_sig_script, encode_contract_entry_sig_script};
use silverscript_lang::compiler::{CompileOptions, compile_to_sil_abi_artifact_with_options};
use std::collections::BTreeMap;

const VAULT_SRC: &str = include_str!("../../contracts/stable_vault.sil");
const ORACLE_SRC: &str = include_str!("../../contracts/risk_oracle.sil");
const KCC20_SRC: &str = include_str!("../../vendor/silverscript/silverscript-lang/tests/examples/kcc20.sil");
const ORACLE_COV: Hash = Hash::from_bytes([0x0a; 32]);
const GHOST_COV: Hash = Hash::from_bytes([0x0b; 32]);
const VAULT_COV: Hash = Hash::from_bytes([0x0c; 32]);
const MCR: i64 = 20_000; const LIQ: i64 = 15_000; const BONUS: i64 = 1_000;
const ID_PUBKEY: u8 = 0x00; const ID_COV: u8 = 0x02; const E8: i64 = 100_000_000;
const PRICE: i64 = 4_000_000; const INDEX: i64 = 1_050_000_000; const COLL: i64 = 10_000 * E8;

fn rk() -> Keypair { let s=Secp256k1::new(); let mut b=[0u8;32]; loop{ thread_rng().fill_bytes(&mut b); if let Ok(k)=SecretKey::from_slice(&b){return Keypair::from_secret_key(&s,&k);}}}
fn xonly(k:&Keypair)->Vec<u8>{k.x_only_public_key().0.serialize().to_vec()}
fn debt_of(sh:i64,ix:i64)->i64{(sh as u128*ix as u128).div_ceil(1_000_000_000)as i64}

struct Env{committee:Vec<Vec<u8>>,oracle_tpl:(Vec<u8>,Vec<u8>,Vec<u8>),ghost_tpl:(Vec<u8>,Vec<u8>,Vec<u8>),owner:Keypair}
impl Env{
 fn new()->Self{let committee:Vec<Vec<u8>>=(0..5).map(|_|xonly(&rk())).collect();
  let mut e=Self{committee,oracle_tpl:Default::default(),ghost_tpl:Default::default(),owner:rk()};
  e.oracle_tpl=compiled_template_parts_and_hash(&e.oracle_art()); e.ghost_tpl=compiled_template_parts_and_hash(&e.ghost(vec![0;32],ID_COV,0,true)); e}
 fn oracle_art(&self)->SilAbiArtifact{let mut a:Vec<ArtifactValue>=self.committee.iter().cloned().map(ArtifactValue::Bytes).collect();
  a.extend([3i64,1_000_000_000,PRICE,1_000_000,1,158_548_959,INDEX].map(ArtifactValue::Int));
  compile_to_sil_abi_artifact_with_options(ORACLE_SRC,&a,CompileOptions::default()).unwrap()}
 fn ghost(&self,o:Vec<u8>,t:u8,amt:i64,m:bool)->SilAbiArtifact{compile_to_sil_abi_artifact_with_options(KCC20_SRC,&[ArtifactValue::Bytes(o),ArtifactValue::Int(amt),ArtifactValue::Byte(t),ArtifactValue::Bool(m),ArtifactValue::Int(3),ArtifactValue::Int(2)],CompileOptions::default()).unwrap()}
 fn vault(&self,sh:i64)->SilAbiArtifact{let(op,os,oh)=&self.oracle_tpl;let(kp,ks,kh)=&self.ghost_tpl;
  compile_to_sil_abi_artifact_with_options(VAULT_SRC,&[ArtifactValue::Bytes(ORACLE_COV.as_bytes().to_vec()),ArtifactValue::Int(op.len()as i64),ArtifactValue::Int(os.len()as i64),ArtifactValue::Bytes(oh.clone()),ArtifactValue::Bytes(GHOST_COV.as_bytes().to_vec()),ArtifactValue::Int(kp.len()as i64),ArtifactValue::Int(ks.len()as i64),ArtifactValue::Bytes(kh.clone()),ArtifactValue::Int(MCR),ArtifactValue::Int(LIQ),ArtifactValue::Int(BONUS),ArtifactValue::Bytes(xonly(&self.owner)),ArtifactValue::Int(sh)],CompileOptions::default()).unwrap()}
}
fn gstate(o:&[u8],t:u8,a:i64,m:bool)->ArtifactValue{BTreeMap::from([("ownerIdentifier".to_string(),ArtifactValue::Bytes(o.to_vec())),("identifierType".to_string(),ArtifactValue::Byte(t)),("amount".to_string(),ArtifactValue::Int(a)),("isMinter".to_string(),ArtifactValue::Bool(m))]).into()}
fn cout(a:&SilAbiArtifact,v:i64,auth:u16,cov:Hash)->TransactionOutput{TransactionOutput{value:v as u64,script_public_key:pay_to_script_hash_script(&bytecode(a)),covenant:Some(CovenantBinding{authorizing_input:auth,covenant_id:cov})}}
fn putxo(a:&SilAbiArtifact,v:i64,cov:Hash)->UtxoEntry{UtxoEntry::new(v as u64,pay_to_script_hash_script(&bytecode(a)),0,false,Some(cov))}
fn plain(v:i64)->TransactionOutput{TransactionOutput{value:v as u64,script_public_key:kaspa_consensus_core::tx::ScriptPublicKey::new(0,vec![kaspa_txscript::opcodes::codes::OpTrue].into()),covenant:None}}
fn op(i:usize)->TransactionOutpoint{TransactionOutpoint{transaction_id:TransactionId::from_bytes([0x40+i as u8;32]),index:i as u32}}
fn sign(tx:&Transaction,e:&[UtxoEntry],i:usize,k:&Keypair)->Vec<u8>{let m=MutableTransaction::with_entries(tx.clone(),e.to_vec());let r=SigHashReusedValuesUnsync::new();let h=calc_schnorr_signature_hash(&m.as_verifiable(),i,SIG_HASH_ALL,&r);let mut s=k.sign_schnorr(Message::from_digest_slice(h.as_bytes().as_slice()).unwrap()).as_ref().to_vec();s.push(SIG_HASH_ALL.to_u8());s}
fn cname(a:&SilAbiArtifact)->String{a.contracts.keys().next().unwrap().clone()}

enum Call{Entry{art:SilAbiArtifact,entry:&'static str,args:Vec<ArtifactValue>,sig_by:Option<(usize,Keypair)>},Leader{art:SilAbiArtifact,ns:Vec<ArtifactValue>,sig:Option<Keypair>},Delegate{art:SilAbiArtifact,sig:Keypair}}
struct In{utxo:UtxoEntry,call:Call}
fn execute(inputs:Vec<In>,outputs:Vec<TransactionOutput>)->Vec<Result<(),String>>{
 let entries:Vec<UtxoEntry>=inputs.iter().map(|i|i.utxo.clone()).collect();
 let bare:Vec<TransactionInput>=(0..inputs.len()).map(|i|TransactionInput::new_with_compute_budget(op(i),vec![],0,0)).collect();
 let unsigned=Transaction::new(1,bare,outputs.clone(),0,Default::default(),0,vec![]);
 let mut fi=vec![];
 for(idx,input)in inputs.into_iter().enumerate(){
  let s=match input.call{
   Call::Entry{art,entry,mut args,sig_by}=>{if let Some((p,k))=sig_by{args.insert(p,ArtifactValue::Bytes(sign(&unsigned,&entries,idx,&k)));}let mut s=encode_contract_entry_sig_script(&art,&cname(&art),entry,&args).unwrap();s.extend_from_slice(&push_redeem_script(&bytecode(&art)));s}
   Call::Leader{art,ns,sig}=>{let sg=sig.map(|k|sign(&unsigned,&entries,idx,&k)).unwrap_or(vec![0;65]);let a=vec![ArtifactValue::Array(ns),ArtifactValue::Bytes(sg),ArtifactValue::Byte(0)];let mut s=encode_contract_covenant_decl_sig_script(&art,&cname(&art),"transfer",true,&a).unwrap();s.extend_from_slice(&push_redeem_script(&bytecode(&art)));s}
   Call::Delegate{art,sig}=>{let a=vec![ArtifactValue::Bytes(sign(&unsigned,&entries,idx,&sig)),ArtifactValue::Byte(0)];let mut s=encode_contract_covenant_decl_sig_script(&art,&cname(&art),"transfer",false,&a).unwrap();s.extend_from_slice(&push_redeem_script(&bytecode(&art)));s}
  };
  fi.push(TransactionInput::new_with_compute_budget(op(idx),s,0,0));
 }
 let tx=Transaction::new(1,fi,outputs,0,Default::default(),0,vec![]);
 (0..tx.inputs.len()).map(|i|execute_input_with_covenants(tx.clone(),entries.clone(),i).map_err(|e|format!("{e:?}"))).collect()
}

// ---- Angriff 1: repay mit DREI GHOST-Ausgängen (Minter + 2 Empfänger).
// Ziel: einen dritten GHOST-Ausgang an der Vault-Buchhaltung vorbeischmuggeln
// (Minter-Leader prüft keine Mengenerhaltung, MAX_GHOST_OUTS=2).
// Erwartung laut for-Schleifen-Guard require(nOut<=2): Vault lehnt ab.
#[test]
fn drei_ghost_ausgaenge_werden_am_schleifen_guard_abgelehnt(){
 let e=Env::new();
 let shares=100*E8; let payer=rk();
 let vault=e.vault(shares); let minter=e.ghost(VAULT_COV.as_bytes().to_vec(),ID_COV,0,true);
 let pay=e.ghost(xonly(&payer),ID_PUBKEY,50*E8,false);
 // drei Ausgänge: Minter(0) + zwei Empfänger => nOut=3
 let outs=vec![gstate(&VAULT_COV.as_bytes(),ID_COV,0,true),gstate(&xonly(&payer),ID_PUBKEY,20*E8,false),gstate(&xonly(&payer),ID_PUBKEY,20*E8,false)];
 let inputs=vec![
  In{utxo:putxo(&vault,COLL,VAULT_COV),call:Call::Entry{art:vault.clone(),entry:"repay",args:vec![ArtifactValue::Int(1),ArtifactValue::Array(outs.clone())],sig_by:None}},
  In{utxo:putxo(&e.oracle_art(),E8,ORACLE_COV),call:Call::Entry{art:e.oracle_art(),entry:"read",args:vec![],sig_by:None}},
  In{utxo:putxo(&minter,1_000,GHOST_COV),call:Call::Leader{art:minter.clone(),ns:outs,sig:None}},
  In{utxo:putxo(&pay,1_000,GHOST_COV),call:Call::Delegate{art:pay.clone(),sig:payer}},
 ];
 let vshares=shares-((40*E8 as i64)as u128*1_000_000_000u128/INDEX as u128)as i64;
 let outputs=vec![
  cout(&e.vault(vshares),COLL,0,VAULT_COV),
  cout(&e.oracle_art(),E8,1,ORACLE_COV),
  cout(&minter,1_000,2,GHOST_COV),
  cout(&e.ghost(xonly(&payer),ID_PUBKEY,20*E8,false),1_000,2,GHOST_COV),
  cout(&e.ghost(xonly(&payer),ID_PUBKEY,20*E8,false),1_000,2,GHOST_COV),
  plain(1),
 ];
 let r=execute(inputs,outputs);
 println!("drei_ausgaenge => {r:?}");
 assert!(r[0].is_err(),"Vault muss den 3. GHOST-Ausgang ablehnen (Schleifen-Guard): {r:?}");
}

// ---- Angriff 2: repay, bei dem der Minter-Zweig (ghost input 0) durch einen
// FREMDEN Vault-Minter ersetzt wird (owner != dieser Vault). Ziel: einen
// fremden Minter-Zweig kapern. Erwartung: ghostDelta verlangt owner==myCov.
#[test]
fn fremder_minter_zweig_wird_abgelehnt(){
 let e=Env::new();
 let other=Hash::from_bytes([0x0e;32]); // ID eines fremden Vaults
 let shares=100*E8; let payer=rk();
 let vault=e.vault(shares);
 let minter=e.ghost(other.as_bytes().to_vec(),ID_COV,0,true); // Minter gehoert FREMDEM Vault
 let pay=e.ghost(xonly(&payer),ID_PUBKEY,50*E8,false);
 let outs=vec![gstate(&other.as_bytes(),ID_COV,0,true),gstate(&xonly(&payer),ID_PUBKEY,20*E8,false)];
 let inputs=vec![
  In{utxo:putxo(&vault,COLL,VAULT_COV),call:Call::Entry{art:vault.clone(),entry:"repay",args:vec![ArtifactValue::Int(1),ArtifactValue::Array(outs.clone())],sig_by:None}},
  In{utxo:putxo(&e.oracle_art(),E8,ORACLE_COV),call:Call::Entry{art:e.oracle_art(),entry:"read",args:vec![],sig_by:None}},
  In{utxo:putxo(&minter,1_000,GHOST_COV),call:Call::Leader{art:minter.clone(),ns:outs,sig:None}},
  In{utxo:putxo(&pay,1_000,GHOST_COV),call:Call::Delegate{art:pay.clone(),sig:payer}},
 ];
 let vshares=shares-((30*E8 as i64)as u128*1_000_000_000u128/INDEX as u128)as i64;
 let outputs=vec![
  cout(&e.vault(vshares),COLL,0,VAULT_COV),
  cout(&e.oracle_art(),E8,1,ORACLE_COV),
  cout(&minter,1_000,2,GHOST_COV),
  cout(&e.ghost(xonly(&payer),ID_PUBKEY,20*E8,false),1_000,2,GHOST_COV),
  plain(1),
 ];
 let r=execute(inputs,outputs);
 println!("fremder_minter => {r:?}");
 assert!(r[0].is_err(),"Vault muss fremden Minter ablehnen: {r:?}");
}

// ---- Gegenprobe/Kontroll: ehrliches Voll-Repay geht durch (damit Ablehnungen
// oben wirklich an der Regel liegen).
#[test]
fn ehrliches_repay_geht_durch(){
 let e=Env::new();
 let shares=100*E8; let debt=debt_of(shares,INDEX); let payer=rk();
 let vault=e.vault(shares); let minter=e.ghost(VAULT_COV.as_bytes().to_vec(),ID_COV,0,true);
 let pay=e.ghost(xonly(&payer),ID_PUBKEY,debt,false);
 let outs=vec![gstate(&VAULT_COV.as_bytes(),ID_COV,0,true)];
 let inputs=vec![
  In{utxo:putxo(&vault,COLL,VAULT_COV),call:Call::Entry{art:vault.clone(),entry:"repay",args:vec![ArtifactValue::Int(1),ArtifactValue::Array(outs.clone())],sig_by:None}},
  In{utxo:putxo(&e.oracle_art(),E8,ORACLE_COV),call:Call::Entry{art:e.oracle_art(),entry:"read",args:vec![],sig_by:None}},
  In{utxo:putxo(&minter,1_000,GHOST_COV),call:Call::Leader{art:minter.clone(),ns:outs,sig:None}},
  In{utxo:putxo(&pay,1_000,GHOST_COV),call:Call::Delegate{art:pay.clone(),sig:payer}},
 ];
 let outputs=vec![
  cout(&e.vault(0),COLL,0,VAULT_COV),
  cout(&e.oracle_art(),E8,1,ORACLE_COV),
  cout(&minter,1_000,2,GHOST_COV),
  plain(1),
 ];
 let r=execute(inputs,outputs);
 println!("ehrlich => {r:?}");
 assert!(r.iter().all(|x|x.is_ok()),"{r:?}");
}
