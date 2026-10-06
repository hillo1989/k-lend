# Entwurf: Beitrag für kas-smiths.org (Kategorie „Product Use Cases“)

Vom Nutzer selbst zu posten. Vorher prüfen: Abschnitt „KCC20 example“ erst
posten, wenn die SilverScript-Maintainer informiert sind (siehe Hinweis unten).

---

**Title:** GHOST / K.Lend – an oracle-based, over-collateralized USD stablecoin on L1 covenants (live on mainnet)

Hi smiths,

I'd like to share **K.Lend** (https://k-lend.com, code: https://github.com/hillo1989/k-lend), a CDP-style stablecoin (**GHOST**, soft-pegged to 1 USD) that runs entirely on Kaspa L1 covenants, written in SilverScript 1.0. It is live on mainnet (current contract version 4). This is a different design from the oracle-free KUSD thread here, so the two might be interesting to compare.

**Contracts (v4)**
- `stable_vault` – KAS collateral, min. 200 % to mint, liquidation below 150 % (partial, with bonus), redemption at 1 USD against vaults ≥ 150 % (1 % fee stays with the vault owner), interest accrues on an index and is paid in KAS on close.
- `ghost_token` – a KCC20 variant; every vault has its own minter branch bound to the vault's covenant id, so GHOST can only be minted through a healthy vault.
- `price_oracle` + `signer_register` – price attested by a signer set (t-of-n), with signer rotation behind a consensus-enforced relative sequence lock (14 days), an emergency path and an oracle freeze after 2 h of silence (mint/liquidate/redeem stop when frozen).
- `ghost_pool` – KAS/GHOST pool with 0.3 % fee and a ±3 % band around the oracle price.
- `standing_order` – "vault" for recurring payments: anyone may trigger a payment at the due time, only to the fixed recipient, only the owner can cancel.

**Things we learned that may help other builders**
1. **Standard sig-op limit per P2SH input is 15, counted statically** (every `OpCheckSig*` in the redeem script, including unrolled loops). Our first register with `checkQuorum` used three times for up to 9 signers had 28 sig-ops – consensus-valid, but rejected by the mempool. We now cap at 4-of-7 (13 sig-ops). Worth checking before deploying anything with multisig logic.
2. **Storage mass matters a lot for covenant outputs**: each new covenant output costs roughly 4·10¹² / value grams. 0.3 KAS token UTXOs put a vault tx at ~307 000 g (block limit 500 000); 1 KAS tokens / 3 KAS minter branches bring it down to 21 000–57 000 g.
3. **Wallet signing of covenant inputs works today**: KasWare `signPskt` and Kastle `signTx` sign covenant entry inputs and KCC20 leader/delegate inputs. Our server builds the unsigned tx, the wallet signs, and the server rebuilds the tx from its own state and requires it to be bit-identical before checking signatures and broadcasting – so no keys on the server and nothing taken from the browser at face value.
4. **Relative sequence locks are enforced by consensus** ("one of the transaction sequence locks conditions was not met") – usable for time-delayed governance in covenants.
5. *(see note below)* The KCC20 example only checked `totalIn == totalOut`; without requiring every new state's `amount >= 0`, one unit can be split into `+D` and `-(D-1)`, i.e. unlimited minting. We added that check.

**Status / honesty section**
- Experimental. Several AI-assisted audits (findings and fixes are in the repo under `audit/`), **no professional audit yet**. The oracle currently has a single signer; an emergency key set and a v5 with on-chain price-jump limits are in the works.
- Users sign everything in KasWare/Kastle; `.k` names (dotk.name) are resolved and verified against our own node before use.

Feedback on the design – especially the oracle/signer-register approach and the v5 plans (jump limits, guardian freeze, replay protection for price attestations) – is very welcome.

Andy

---

**Hinweis zu Punkt 5:** Das betrifft das Beispiel `kcc20.sil` im SilverScript-Repo
(kaspanet/silverscript, tests/examples). Bevor du das öffentlich schreibst, die
Maintainer vertraulich informieren (z. B. über ein privates Security-Advisory
auf GitHub oder den Core-R&D-Telegram https://t.me/kasparnd) und ihnen etwas
Zeit geben – andere Projekte könnten das Beispiel übernommen haben. Wenn du
das nicht willst, Punkt 5 vor dem Posten streichen.
