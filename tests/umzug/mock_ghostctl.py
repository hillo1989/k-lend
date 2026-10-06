#!/usr/bin/env python3
# Attrappe für ./ghostctl (v3) und bin/ghostctl-v2 – kein Netz, keine echten Schlüssel.
# Nachgebildet nach protocol/src/bin/ghostctl.rs und store.rs (Stand 8b6ee75):
#  - `keys --dir D`: liest D/*.json sortiert, "file" = D/<name> (wie dir.join),
#    Schlüsseldatei {"mock_xonly": "<64 hex>"} = Schlüssel, {"secrets": [...]} = Komitee,
#    alles andere wird übersprungen (wie eine unlesbare Schlüsseldatei)
#  - Befehle mit Zustandsabgleich übernehmen ein offenes Journal (<zustand ohne .json>.pending.json):
#    Ziel bekommt "next", Journal wird gelöscht (store::resolve_pending, Fall „angenommen“),
#    aber nur mit "journal": true in mock.json
#  - Besitzer eines neuen Vaults = x-only der Schlüsseldatei aus --key
#  - Symlinks und Hardlinks im Schlüsselordner werden wie Dateien gelistet
#    (wie ghostctl: read_dir, dann die Datei lesen)
# Welt (Guthaben, Vaults, Pool) in mock.json: {"v2": {...}, "v3": {...}}.
# Schalter je Version: "dup_keys": true (Liste doppelt) oder "diff" (doppelt, die
# Kopie mit anderem x-only), "xonly_as": "<hex>" (keys nennt für jeden Schlüssel
# dieses x-only, etwa eine andere Ableitung in Version 3), "open_owner",
# "status_broken", "fail": {befehl: meldung}, "dry_fail": {befehl: meldung} (nur mit
# --dry-run, wie ghostctl beim Zusammenführen verstreuter GHOST: echt geht es).
# Wie ghostctl (confirm in ghostctl.rs): Ohne --ja und ohne --dry-run fragt jeder
# Sendebefehl „MAINNET – wirklich senden? [j/N]“ auf stderr und liest eine Zeile
# von stdin (Byte für Byte, damit weitere Antworten für spätere Aufrufe bleiben);
# jede Rückfrage steht als „# Rückfrage“ in calls.log. --dry-run --json gibt eine
# Gebühr aus ("dry_fee", Vorgabe 0.0412 KAS je Probe). `price` nennt "price" (Vorgabe
# kasUsd der Welt). Das KAS-Guthaben eines Schlüssels ist in beiden Versionen dieselbe
# Adresse: Rückflüsse aus Version 2 (schließen, herausnehmen, Pool abziehen) landen
# auch bei "v3", mal "kas_back" (Vorgabe 1; kleiner = weniger KAS als geplant);
# "move" siehe unten (Welt ändert sich beim ersten echten Senden).
# Version 3 zieht für deploy 30, open-vault Sicherheit + 3, mint 1, pool-open KAS + 3 ab.
# Audit 13: "move_after": {befehl: {...}} – die Welt bewegt sich NACH der Wirkung
# dieses Sendebefehls (etwa ein fremder Vault verschwindet zwischen Tilgen und
# Schließen); Schlüssel "status#N": nach dem N-ten Status (--json, ohne --dry-run)
# dieser Version. "sleep": {befehl: s} – Sendebefehl dauert vor der Wirkung;
# "sleep_after": {befehl: s} – Wirkung gespeichert, ghostctl wartet noch
# (Bestätigung). pool-remove prüft --min-kas/--min-ghost wie pool::remove.
# `keys` nennt je Schlüssel eine Attrappen-Adresse "kaspa:attrappe<x-only[:16]>".
import json, os, sys
root = os.path.dirname(os.path.abspath(__file__))
if os.path.basename(root) == "bin":
    root = os.path.dirname(root)
ver = "v2" if os.path.basename(sys.argv[0]) == "ghostctl-v2" else "v3"
args = sys.argv[1:]
with open(os.path.join(root, "calls.log"), "a") as f:
    f.write(ver + " " + " ".join(args) + "\n")
M = json.load(open(os.path.join(root, "mock.json")))
def save():
    json.dump(M, open(os.path.join(root, "mock.json"), "w"), indent=1)
def log(s):
    with open(os.path.join(root, "calls.log"), "a") as f:
        f.write("   # " + s + "\n")
SENDS = {"deploy", "open-vault", "mint", "repay", "close", "withdraw", "pool-remove", "pool-open"}
VALUED = {"--network", "--state", "--key", "--vault", "--ghost", "--keep", "--kas", "--percent", "--committee", "--rate", "--dir", "--rpc", "--min-kas", "--min-ghost"}
def opt(name, default=None):
    return args[args.index(name) + 1] if name in args else default
cmd = next(a for i, a in enumerate(args) if not a.startswith("--") and (i == 0 or args[i - 1] not in VALUED))
state = opt("--state", "deployments/mainnet.json")
dry = "--dry-run" in args
js = "--json" in args
w = M[ver]

def keyfile_xonly(path):
    try:
        d = json.load(open(path))
    except Exception:
        return None
    return d.get("mock_xonly") if isinstance(d, dict) else None

def resolve_pending():
    if not M.get("journal"):
        return
    p = os.path.splitext(state)[0] + ".pending.json"
    if not os.path.exists(p):
        return
    j = json.load(open(p))
    if j.get("target") and j.get("next") is not None:
        json.dump(j["next"], open(j["target"], "w"))
        log("Journal übernommen → " + j["target"])
    os.remove(p)

dfail = w.get("dry_fail", {}).get(cmd) if "--dry-run" in args else None
if dfail and "--json" in args:
    # wie ghostctl: mit --json steht der Fehler auf stdout, Exitcode 1
    print(json.dumps({"error": dfail, "ok": False, "transactions": []})); sys.exit(1)
fail = w.get("fail", {}).get(cmd) or dfail
if fail:
    print(fail, file=sys.stderr); sys.exit(1)

if cmd == "price":
    print(json.dumps({"ok": True, "median": w.get("price", w["kasUsd"]), "quotes": []}))
    sys.exit(0)

if cmd == "keys":
    d = opt("--dir", "keys")
    out = []
    names = sorted(n for n in os.listdir(d) if n.endswith(".json")) if os.path.isdir(d) else []
    for n in names:
        f = os.path.join(d, n)
        try:
            c = json.load(open(f))
        except Exception:
            continue
        if isinstance(c, dict) and "secrets" in c:
            out.append({"file": f, "type": "committee", "signers": len(c["secrets"])})
            continue
        x = keyfile_xonly(f)
        if not x:
            continue
        x = w.get("xonly_as") or x
        k = w.get("keys", {}).get(x, {})
        out.append({"file": f, "type": "key", "xonly": x, "address": "kaspa:attrappe" + x[:16], "kas": k.get("kas", 1), "ghost": k.get("ghost", 0), "lpShares": str(k.get("lpShares", "0")), "vaults": [i for i, v in enumerate(w["vaults"]) if v["owner"] == x]})
    if w.get("dup_keys") == "diff":
        out = out + [dict(k, xonly="9" * 64) if k["type"] == "key" else k for k in out]
    elif w.get("dup_keys"):
        out = out + out
    print(json.dumps({"ok": True, "keys": out, "offline": False, "transactions": []}))
    sys.exit(0)

if cmd == "status":
    if not os.path.exists(state):
        print(json.dumps({"network": "mainnet", "deployed": False})); sys.exit(0)
    resolve_pending()
    if w.get("status_broken"):
        print(json.dumps({"network": "mainnet", "deployed": False, "error": "Attrappe: Zustand nicht lesbar"})); sys.exit(0)
    if js:
        print(json.dumps({"network": "mainnet", "deployed": True, "oracle": {"kasUsd": w["kasUsd"]}, "vaults": [dict(v, index=i) for i, v in enumerate(w["vaults"])], "pool": w.get("pool")}))
        # nach dem N-ten Status bewegt sich die Welt (gelesen ist noch der alte Stand)
        if "move_after" in w:
            n = w["_n_status"] = w.get("_n_status", 0) + 1
            ma = w["move_after"].pop("status#%d" % n, None)
            if ma is not None:
                w.update(ma); log("Welt bewegt nach status#%d" % n)
            save()
    else:
        print("Vault (Attrappe) " + str(len(w["vaults"])))
    sys.exit(0)

if cmd == "balance":
    print("Guthaben (Attrappe)"); sys.exit(0)

resolve_pending()
key = opt("--key")
kx = keyfile_xonly(key) if key else None
if key and not kx:
    print("Schlüsseldatei " + str(key) + " nicht lesbar", file=sys.stderr); sys.exit(1)
if dry:
    if js:
        print(json.dumps({"ok": True, "dryRun": True, "transactions": [{"action": cmd, "feeKas": w.get("dry_fee", 0.0412)}]}))
    else:
        print("Probelauf " + cmd)
    sys.exit(0)
if cmd in SENDS and "--ja" not in args:
    sys.stderr.write("  MAINNET – wirklich senden? [j/N] "); sys.stderr.flush()
    log("Rückfrage " + cmd)
    a = b""
    while True:
        c = os.read(0, 1)
        if not c or c == b"\n":
            break
        a += c
    if a.decode().strip().lower() != "j":
        print("abgebrochen", file=sys.stderr); sys.exit(1)
# "move": {...} – die Welt bewegt sich zwischen Plan und Senden (jemand anders
# tauscht, das Orakel läuft weiter): Beim ersten echten Sendebefehl dieser Version
# werden die Felder übernommen (etwa "pool", "kasUsd", "vaults"), bevor er wirkt.
if cmd in SENDS and "move" in w:
    w.update(w.pop("move"))
    log("Welt bewegt")
if cmd in SENDS and w.get("sleep", {}).get(cmd):
    import time; log("schläft " + cmd); time.sleep(w["sleep"][cmd]); log("fertig geschlafen " + cmd)
kw = w.setdefault("keys", {}).setdefault(kx, {"ghost": 0, "kas": 0, "lpShares": "0"}) if kx else {}
def kas3(delta):
    # dieselbe Adresse in beiden Versionen: Guthaben in "v3" (x-only wie dort gelistet)
    w3 = M["v3"]
    x3 = w3.get("xonly_as") or kx
    k3 = w3.setdefault("keys", {}).setdefault(x3, {"ghost": 0, "kas": 0, "lpShares": "0"})
    k3["kas"] = round(k3.get("kas", 0) + delta * (M["v2"].get("kas_back", 1) if delta > 0 else 1), 8)
vi = int(opt("--vault", "-1"))
if cmd == "deploy":
    json.dump({"network": "mainnet", "vault_params": {"treasury": "ab"}}, open(state, "w"))
    kas3(-30)
elif cmd == "open-vault":
    owner = w.get("open_owner") or kx
    w["vaults"].append({"owner": owner, "covenantId": "neu%d" % len(w["vaults"]), "debtGhost": 0, "collateralKas": float(opt("--kas"))})
    kas3(-float(opt("--kas")) - 3)
elif cmd in ("mint", "repay", "close", "withdraw"):
    v = w["vaults"][vi]
    if v["owner"] != kx:
        print("Vault %d gehört nicht diesem Schlüssel" % vi, file=sys.stderr); sys.exit(1)
    if cmd == "mint":
        v["debtGhost"] = round(v["debtGhost"] + float(opt("--ghost")), 8); kw["ghost"] = round(kw.get("ghost", 0) + float(opt("--ghost")), 8)
        kas3(-1)
    elif cmd == "repay":
        g = min(float(opt("--ghost", v["debtGhost"])), v["debtGhost"])
        if g > kw.get("ghost", 0) + 1e-12:
            print("zu wenig GHOST", file=sys.stderr); sys.exit(1)
        v["debtGhost"] = round(v["debtGhost"] - g, 8); kw["ghost"] = round(kw["ghost"] - g, 8)
    elif cmd == "close":
        if v["debtGhost"] != 0:
            print("Schuld offen", file=sys.stderr); sys.exit(1)
        w["vaults"].pop(vi)
        if ver == "v2":
            kas3(v["collateralKas"])
    else:
        if ver == "v2":
            kas3(v["collateralKas"] - float(opt("--keep")))
        v["collateralKas"] = float(opt("--keep"))
elif cmd == "pool-remove":
    p = w.get("pool") or {}
    if "shares" in p:
        # wie pool::remove: ganzzahlig abgerundet, KAS-Seite auf 1 KAS Mindestreserve gekappt
        m, sh, x, y = int(kw.get("lpShares", 0)), int(p["shares"]), int(p["kasSompi"]), int(p["ghostUnits"])
        dx, dy = max(0, min(x * m // sh, x - 100000000)), y * m // sh
        mk, mg = round(float(opt("--min-kas", 0)) * 1e8), round(float(opt("--min-ghost", 0)) * 1e8)
        if dx < mk or dy < mg:
            print("Der Pool hat sich verschoben: weniger Auszahlung als erwartet. Bitte neu prüfen.", file=sys.stderr)
            save(); sys.exit(1)
        kas3(dx / 1e8)
        kw["ghost"] = round(kw.get("ghost", 0) + dy / 1e8, 8)
    kw["lpShares"] = "0"
elif cmd == "pool-open":
    w["pool"] = {"kas": opt("--kas"), "ghost": opt("--ghost")}
    kas3(-float(opt("--kas")) - 3)
else:
    print("unbekannt: " + cmd, file=sys.stderr); sys.exit(2)
ma = w.get("move_after", {}).pop(cmd, None) if cmd in SENDS else None
if ma is not None:
    w.update(ma); log("Welt bewegt nach " + cmd)
save()
if w.get("sleep_after", {}).get(cmd) and not dry:
    import time; log("wirkt, wartet " + cmd); time.sleep(w["sleep_after"][cmd])
