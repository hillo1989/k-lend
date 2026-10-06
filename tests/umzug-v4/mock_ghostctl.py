#!/usr/bin/env python3
# Attrappe für ./ghostctl (v4) und bin/ghostctl-v3 – kein Netz, keine echten Schlüssel.
# Vorbild: tests/umzug/mock_ghostctl.py (v2/v3). Nachgebildet nach
# protocol/src/bin/ghostctl.rs (v4, dieser Ordner) und kaspa-lending/protocol/src/bin/ghostctl.rs (v3):
#  - `keys --dir D`: liest D/*.json sortiert, "file" = D/<name> (wie dir.join),
#    Schlüsseldatei {"mock_xonly": "<64 hex>"} = Schlüssel, {"secrets": [...]} = Komitee
#    bzw. Unterzeichner ("signers" = Anzahl), alles andere wird übersprungen
#  - Befehle mit Zustandsabgleich übernehmen ein offenes Journal (<zustand ohne .json>.pending.json):
#    Ziel bekommt "next", Journal wird gelöscht (store::resolve_pending, Fall „angenommen“),
#    aber nur mit "journal": true in mock.json
#  - Besitzer eines neuen Vaults = x-only der Schlüsseldatei aus --key
#  - v4 lehnt wie old_version eine Zustandsdatei ohne "register" ab (außer keys,
#    price, committee-keygen); v3 lehnt eine Zustandsdatei mit "register" ab
#  - `committee-keygen <datei> --count N` (v4): legt {"secrets": [N×"x"]} an, wie
#    write_new nur, wenn die Datei noch fehlt
#  - `deploy` (v4): Zustandsdatei mit "register", --committee muss eine Datei mit
#    genau einem Schlüssel sein, --probe wird abgelehnt (der Umzug deployt ohne);
#    legt wie rate_restart <zustand>-zins.json neu an; bindet 4 KAS
# Welt (Guthaben, Vaults, Pool) in mock.json: {"v3": {...}, "v4": {...}}.
# Schalter je Version: "dup_keys": true (Liste doppelt) oder "diff" (doppelt, die
# Kopie mit anderem x-only), "xonly_as": "<hex>" (keys nennt für jeden Schlüssel
# dieses x-only), "open_owner", "status_broken", "fail": {befehl: meldung},
# "dry_fail": {befehl: meldung} (nur mit --dry-run), "fail_echt": {befehl: meldung}
# (nur ohne --dry-run: die Probe im Plan gelingt, das Senden scheitert vor der Wirkung).
# Wie ghostctl (confirm): Ohne --ja und ohne --dry-run fragt jeder Sendebefehl
# „MAINNET – wirklich senden? [j/N]“ auf stderr und liest eine Zeile von stdin
# (Byte für Byte); jede Rückfrage steht als „# Rückfrage“ in calls.log.
# --dry-run --json gibt eine Gebühr aus ("dry_fee", Vorgabe 0.0412 KAS je Probe).
# `price` nennt "price" (Vorgabe kasUsd der Welt). Das KAS-Guthaben eines
# Schlüssels ist in beiden Versionen dieselbe Adresse: Rückflüsse aus Version 3
# landen bei "v4", mal "kas_back" (Vorgabe 1).
# Version 3 rechnet Zins: Vaults dürfen "interestUsd" tragen (status --json wie
# v3); close zahlt ⌈Zins/Kurs⌉ KAS (ab 0,2 KAS) an die Zinskasse, withdraw
# verlangt Sicherheit·Kurs ≥ 200 % von Schuld + Zins (ops::withdraw).
# Version 4 zieht für deploy 4, open-vault Sicherheit + 3, mint 1, pool-open KAS + 3 ab.
# "move": {...} beim ersten echten Sendebefehl der Version; "move_after":
# {befehl|status#N: {...}} nach der Wirkung; "sleep"/"sleep_after": {befehl: s};
# "rm_after": {befehl: pfad} – nach der Wirkung wird die Datei entfernt (etwa die
# Unterzeichner-Datei verschwindet während Teil A); "write_after": {befehl: [pfad,
# inhalt]} – danach wird die Datei geschrieben; "move_other_after": {befehl: {...}}
# – danach bewegt sich die Welt der ANDEREN Version (etwa das Orakel von v4 während Teil A).
# Audit 16:
#  - Anteils-UTXOs: "lpUtxos": [a, b, …] je Schlüssel (sonst ein UTXO mit lpShares);
#    lpShares = Summe. pool-remove v3 zieht wie pool::own(…, 2) nur aus den ZWEI
#    GRÖSSTEN ab und meldet bei mehr als zweien „Hinweis: Anteile liegen auf mehr
#    als zwei UTXOs …“ auf stderr; --dry-run --json nennt die Auszahlung („kas“,
#    „ghost“, gekappt auf 1 KAS Mindestreserve) und prüft --min-kas/--min-ghost.
#  - Speichermasse: withdraw und close (v3) scheitern wie txb::build mit „Transaktion
#    zu groß für einen Block“, wenn der Ausgang an den Schlüssel unter 0,025 KAS
#    läge – auch mit --dry-run; ebenso die Mindestquote bei withdraw.
#  - v4-Orakel: "frozen": true / "freezeInMinutes" (Vorgabe 120) in status --json;
#    mint, pool-open und pool-add scheitern eingefroren; oracle-update (--committee
#    mit 1 Schlüssel) macht es frisch. mint prüft 200 % (Sicherheit·Kurs ≥ 2·Schuld).
#  - pool-open v4 in drei Schritten (Genesis, init mit Mindestliquidität 1 KAS,
#    Rest einlegen): "pool_open_fail": 2 bzw. 3 lässt Schritt 2 bzw. 3 scheitern;
#    nach Schritt 1 steht "pool_pending" (status zeigt noch keinen Pool, pool-open
#    setzt fort), nach Schritt 2 gibt es den Pool ohne eigene Anteile. pool-add legt
#    zum Verhältnis des Pools ein (1 % Toleranz) und gibt Anteile.
import json, os, sys
root = os.path.dirname(os.path.abspath(__file__))
if os.path.basename(root) == "bin":
    root = os.path.dirname(root)
ver = "v3" if os.path.basename(sys.argv[0]) == "ghostctl-v3" else "v4"
args = sys.argv[1:]
with open(os.path.join(root, "calls.log"), "a") as f:
    f.write(ver + " " + " ".join(args) + "\n")
M = json.load(open(os.path.join(root, "mock.json")))
def save():
    json.dump(M, open(os.path.join(root, "mock.json"), "w"), indent=1)
def log(s):
    with open(os.path.join(root, "calls.log"), "a") as f:
        f.write("   # " + s + "\n")
SENDS = {"deploy", "open-vault", "mint", "repay", "close", "withdraw", "pool-remove", "pool-open", "pool-add", "oracle-update"}
VALUED = {"--network", "--state", "--key", "--vault", "--ghost", "--keep", "--kas", "--percent", "--committee", "--rate", "--dir", "--rpc", "--min-kas", "--min-ghost", "--count", "--threshold"}
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

def err(msg, code=1):
    if js:
        print(json.dumps({"ok": False, "error": msg, "transactions": []}))
    else:
        print("Fehler: " + msg, file=sys.stderr)
    sys.exit(code)

dfail = w.get("dry_fail", {}).get(cmd) if dry else None
if dfail and js:
    print(json.dumps({"error": dfail, "ok": False, "transactions": []})); sys.exit(1)
fail = w.get("fail", {}).get(cmd) or dfail or (None if dry else w.get("fail_echt", {}).get(cmd))
if fail:
    print(fail, file=sys.stderr); sys.exit(1)

if cmd == "price":
    print(json.dumps({"ok": True, "median": w.get("price", w["kasUsd"]), "quotes": []}))
    sys.exit(0)

if cmd == "committee-keygen":
    if ver != "v4":
        err("unbekannt: committee-keygen (Attrappe v3)", 2)
    out = args[args.index(cmd) + 1]
    n = int(opt("--count", "1"))
    if os.path.exists(out):
        err(out + " existiert schon")
    json.dump({"secrets": ["x"] * n}, open(out, "w"))
    print("%d Unterzeichner-Schlüssel in %s (Attrappe)" % (n, out))
    wa = w.get("write_after", {}).pop(cmd, None)
    if wa is not None:
        open(os.path.join(root, wa[0]), "w").write(wa[1]); log("geschrieben nach " + cmd + ": " + wa[0]); save()
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
        lps = str(sum(int(u) for u in k["lpUtxos"])) if "lpUtxos" in k else str(k.get("lpShares", "0"))
        out.append({"file": f, "type": "key", "xonly": x, "address": "kaspa:attrappe" + x[:16], "kas": k.get("kas", 1), "ghost": k.get("ghost", 0), "lpShares": lps, "vaults": [i for i, v in enumerate(w["vaults"]) if v["owner"] == x]})
    if w.get("dup_keys") == "diff":
        out = out + [dict(k, xonly="9" * 64) if k["type"] == "key" else k for k in out]
    elif w.get("dup_keys"):
        out = out + out
    print(json.dumps({"ok": True, "keys": out, "offline": False, "transactions": []}))
    sys.exit(0)

# Zustandsdatei der falschen Version (old_version in v4; v3 kennt kein Register)
if os.path.exists(state):
    try:
        sd = json.load(open(state))
    except Exception:
        sd = None
    if isinstance(sd, dict):
        if ver == "v4" and "register" not in sd:
            err(state + " stammt von Version 3. Diese ghostctl-Version bedient nur Version 4 (Attrappe)")
        if ver == "v3" and "register" in sd:
            err(state + ": Zustandsdatei nicht lesbar (Version 4, Attrappe v3)")

if cmd == "status":
    if not os.path.exists(state):
        print(json.dumps({"network": "mainnet", "deployed": False})); sys.exit(0)
    resolve_pending()
    if w.get("status_broken"):
        print(json.dumps({"network": "mainnet", "deployed": False, "error": "Attrappe: Zustand nicht lesbar"})); sys.exit(0)
    if js:
        orc = {"kasUsd": w["kasUsd"]}
        if ver == "v4":
            orc.update(frozen=bool(w.get("frozen")), freezeInMinutes=w.get("freezeInMinutes", 120))
        print(json.dumps({"network": "mainnet", "deployed": True, "oracle": orc, "vaults": [dict(v, index=i) for i, v in enumerate(w["vaults"])], "pool": w.get("pool")}))
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
if cmd == "deploy":
    if ver != "v4":
        err("deploy mit Version 3 ist im Umzug nicht vorgesehen (Attrappe)", 2)
    if "--probe" in args:
        err("Attrappe: --probe ist im Umzug nicht erwartet", 2)
    if os.path.exists(state):
        err(state + " existiert schon – in diesem Netz ist bereits angelegt")
    try:
        com = json.load(open(opt("--committee")))
        assert len(com["secrets"]) == 1
    except Exception:
        err("Unterzeichner-Datei " + str(opt("--committee")) + " nicht lesbar oder nicht genau 1 Schlüssel (Attrappe)")
# Prüfungen, die ghostctl schon beim Bauen macht (also auch mit --dry-run)
kw0 = w.get("keys", {}).get(kx, {}) if kx else {}
extra = {}
def lp_list(k):
    return sorted((int(u) for u in k["lpUtxos"]), reverse=True) if "lpUtxos" in k else ([int(k.get("lpShares", 0))] if int(k.get("lpShares", 0)) else [])
MIN_OUT = 0.025
if ver == "v3" and cmd == "pool-remove":
    p = w.get("pool") or {}
    lps = lp_list(kw0)
    if "shares" in p and lps:
        if len(lps) > 2:
            print("Hinweis: Anteile liegen auf mehr als zwei UTXOs; abgezogen wird aus den zwei größten.", file=sys.stderr)
        m = sum(lps[:2])
        sh, x, y = int(p["shares"]), int(p["kasSompi"]), int(p["ghostUnits"])
        dx, dy = max(0, min(x * m // sh, x - 100000000)), y * m // sh
        mk, mg = round(float(opt("--min-kas", 0)) * 1e8), round(float(opt("--min-ghost", 0)) * 1e8)
        if dx < mk or dy < mg:
            err("Der Pool hat sich verschoben: weniger Auszahlung als erwartet. Bitte neu prüfen.")
        extra = {"kas": dx / 1e8, "ghost": dy / 1e8, "_m": m, "_dx": dx, "_dy": dy}
    else:
        err("Dieser Schlüssel hat keine Pool-Anteile.")
if ver == "v3" and cmd in ("withdraw", "close") and 0 <= int(opt("--vault", "-1")) < len(w["vaults"]):
    v = w["vaults"][int(opt("--vault"))]
    if cmd == "withdraw":
        keep = float(opt("--keep"))
        if keep * w["kasUsd"] < 2 * (v["debtGhost"] + v.get("interestUsd", 0)) - 1e-12:
            err("Nach dem Abheben wäre die Mindestquote unterschritten")
        if v["collateralKas"] - keep < MIN_OUT:
            err("Transaktion zu groß für einen Block (Speichermasse, Attrappe)")
    elif v["debtGhost"] == 0:
        import math
        i = round(v.get("interestUsd", 0) * 1e8)
        f = min(math.ceil(i / w["kasUsd"]) if i > 0 else 0, round(v["collateralKas"] * 1e8))
        f = f if f >= 20000000 else 0
        if v["collateralKas"] - f / 1e8 < MIN_OUT:
            err("Transaktion zu groß für einen Block (Speichermasse, Attrappe)")
if ver == "v4" and cmd in ("mint", "pool-open", "pool-add") and w.get("frozen"):
    err("Das Orakel ist eingefroren – " + cmd + " gesperrt, bis wieder ein Preis kommt")
if ver == "v4" and cmd == "mint":
    v = w["vaults"][int(opt("--vault"))]
    if v["collateralKas"] * w["kasUsd"] < 2 * (v["debtGhost"] + float(opt("--ghost"))) - 1e-12:
        err("Nach dem Prägen wäre die Mindestquote unterschritten")
if ver == "v4" and cmd == "oracle-update":
    try:
        assert len(json.load(open(opt("--committee")))["secrets"]) == 1
    except Exception:
        err("Unterzeichner-Datei passt nicht (Attrappe)")
if dry:
    if js:
        print(json.dumps(dict({"ok": True, "dryRun": True, "transactions": [{"action": cmd, "feeKas": w.get("dry_fee", 0.0412)}]}, **{k: v for k, v in extra.items() if not k.startswith("_")})))
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
if cmd in SENDS and "move" in w:
    w.update(w.pop("move"))
    log("Welt bewegt")
if cmd in SENDS and w.get("sleep", {}).get(cmd):
    import time; log("schläft " + cmd); time.sleep(w["sleep"][cmd]); log("fertig geschlafen " + cmd)
kw = w.setdefault("keys", {}).setdefault(kx, {"ghost": 0, "kas": 0, "lpShares": "0"}) if kx else {}
def kas4(delta):
    # dieselbe Adresse in beiden Versionen: Guthaben in "v4" (x-only wie dort gelistet)
    w4 = M["v4"]
    x4 = w4.get("xonly_as") or kx
    k4 = w4.setdefault("keys", {}).setdefault(x4, {"ghost": 0, "kas": 0, "lpShares": "0"})
    k4["kas"] = round(k4.get("kas", 0) + delta * (M["v3"].get("kas_back", 1) if delta > 0 else 1), 8)
def zins_kas(v):
    # ⌈Zins/Kurs⌉ in KAS, höchstens die Sicherheit, unter 0,2 KAS erlassen (ops::close_fee)
    import math
    i = round(v.get("interestUsd", 0) * 1e8)
    f = min(math.ceil(i / w["kasUsd"]) if i > 0 else 0, round(v["collateralKas"] * 1e8))
    return f / 1e8 if f >= 20000000 else 0
vi = int(opt("--vault", "-1"))
if cmd == "deploy":
    json.dump({"network": "mainnet", "register": {"cov": "r4"}, "vault_params": {"treasury": "ab", "max_debt": 1}}, open(state, "w"))
    # rate_restart: die Zinsregel beginnt neu, in <zustand>-zins.json
    json.dump({"network": "mainnet", "attrappe": "v4"}, open(os.path.splitext(state)[0] + "-zins.json", "w"))
    kas4(-4)
elif cmd == "open-vault":
    owner = w.get("open_owner") or kx
    w["vaults"].append({"owner": owner, "covenantId": "neu%d" % len(w["vaults"]), "debtGhost": 0, "collateralKas": float(opt("--kas"))})
    kas4(-float(opt("--kas")) - 3)
elif cmd in ("mint", "repay", "close", "withdraw"):
    v = w["vaults"][vi]
    if v["owner"] != kx:
        print("Vault %d gehört nicht diesem Schlüssel" % vi, file=sys.stderr); sys.exit(1)
    if cmd == "mint":
        v["debtGhost"] = round(v["debtGhost"] + float(opt("--ghost")), 8); kw["ghost"] = round(kw.get("ghost", 0) + float(opt("--ghost")), 8)
        kas4(-1)
    elif cmd == "repay":
        g = min(float(opt("--ghost", v["debtGhost"])), v["debtGhost"])
        if g > kw.get("ghost", 0) + 1e-12:
            print("zu wenig GHOST", file=sys.stderr); sys.exit(1)
        v["debtGhost"] = round(v["debtGhost"] - g, 8); kw["ghost"] = round(kw["ghost"] - g, 8)
    elif cmd == "close":
        if v["debtGhost"] != 0:
            print("Schuld offen", file=sys.stderr); sys.exit(1)
        w["vaults"].pop(vi)
        if ver == "v3":
            z = zins_kas(v)
            if z:
                log("Zins an die Zinskasse %.8f KAS" % z)
            kas4(round(v["collateralKas"] - z, 8))
    else:
        keep = float(opt("--keep"))
        if ver == "v3" and keep * w["kasUsd"] < 2 * (v["debtGhost"] + v.get("interestUsd", 0)) - 1e-12:
            print("Nach dem Abheben wäre die Mindestquote unterschritten", file=sys.stderr); sys.exit(1)
        if ver == "v3":
            kas4(v["collateralKas"] - keep)
        v["collateralKas"] = keep
elif cmd == "pool-remove":
    p = w.get("pool") or {}
    if "shares" in p:
        # Mindestbeträge nach dem Bewegen der Welt erneut prüfen (wie beim Senden)
        lps = lp_list(kw)
        m = sum(lps[:2])
        sh, x, y = int(p["shares"]), int(p["kasSompi"]), int(p["ghostUnits"])
        dx, dy = max(0, min(x * m // sh, x - 100000000)), y * m // sh
        mk, mg = round(float(opt("--min-kas", 0)) * 1e8), round(float(opt("--min-ghost", 0)) * 1e8)
        if dx < mk or dy < mg:
            print("Der Pool hat sich verschoben: weniger Auszahlung als erwartet. Bitte neu prüfen.", file=sys.stderr)
            save(); sys.exit(1)
        kas4(dx / 1e8)
        kw["ghost"] = round(kw.get("ghost", 0) + dy / 1e8, 8)
        p.update(kasSompi=str(x - dx), ghostUnits=str(y - dy), shares=str(sh - m))
        rest = lps[2:]
        if "lpUtxos" in kw:
            kw["lpUtxos"] = rest
        kw["lpShares"] = str(sum(rest))
    else:
        kw["lpShares"] = "0"
elif cmd == "pool-open":
    if w.get("pool"):
        err("In diesem Netz gibt es schon einen Pool. Einlegen mit pool-add.")
    kas_u, gh_u = round(float(opt("--kas")) * 1e8), round(float(opt("--ghost")) * 1e8)
    g0 = max(1, gh_u * 100000000 // kas_u)
    if not w.get("pool_pending"):
        w["pool_pending"] = True; kas4(-1); log("pool-open 1/3 Genesis")
    else:
        log("pool-open setzt fort (Genesis besteht)")
    if w.get("pool_open_fail") == 2:
        save(); err("Pool initialisieren (2/3) fehlgeschlagen (Attrappe)")
    w["pool_pending"] = False
    w["pool"] = {"kas": opt("--kas"), "ghost": opt("--ghost"), "kasSompi": "100000000", "ghostUnits": str(g0), "shares": "100000000"}
    kw["ghost"] = round(kw.get("ghost", 0) - g0 / 1e8, 8); kas4(-2); log("pool-open 2/3 init")
    if w.get("pool_open_fail") == 3:
        save(); err("Liquidität einlegen (3/3) fehlgeschlagen (Attrappe) – Pool ist angelegt; den Rest später mit pool-add einlegen")
    rk, rg = kas_u - 100000000, gh_u - g0
    p = w["pool"]
    p.update(kasSompi=str(100000000 + rk), ghostUnits=str(g0 + rg), shares=str(100000000 + rk))
    kw["ghost"] = round(kw.get("ghost", 0) - rg / 1e8, 8); kw["lpShares"] = str(int(kw.get("lpShares", 0)) + rk)
    kas4(-rk / 1e8 - 1); log("pool-open 3/3 Rest eingelegt")
elif cmd == "pool-add":
    p = w.get("pool") or {}
    if "kasSompi" not in p:
        err("In diesem Netz gibt es noch keinen Pool.")
    x, y, sh = int(p["kasSompi"]), int(p["ghostUnits"]), int(p["shares"])
    ak, ag = round(float(opt("--kas")) * 1e8), round(float(opt("--ghost")) * 1e8)
    if abs(ak * y - ag * x) * 100 > ak * y:
        err("Der Kurs des Pools weicht um mehr als 1.0 % von deinem Verhältnis ab")
    if ag > round(kw.get("ghost", 0) * 1e8) + 1:
        err("zu wenig GHOST")
    m = sh * ag // y
    p.update(kasSompi=str(x + ak), ghostUnits=str(y + ag), shares=str(sh + m))
    kw["ghost"] = round(kw.get("ghost", 0) - ag / 1e8, 8); kw["lpShares"] = str(int(kw.get("lpShares", 0)) + m)
    kas4(-ak / 1e8 - 1)
elif cmd == "oracle-update":
    w["frozen"] = False; w["freezeInMinutes"] = 120
else:
    print("unbekannt: " + cmd, file=sys.stderr); sys.exit(2)
ma = w.get("move_after", {}).pop(cmd, None) if cmd in SENDS else None
if ma is not None:
    w.update(ma); log("Welt bewegt nach " + cmd)
mo = w.get("move_other_after", {}).pop(cmd, None) if cmd in SENDS else None
if mo is not None:
    M["v4" if ver == "v3" else "v3"].update(mo); log("andere Welt bewegt nach " + cmd)
wa = w.get("write_after", {}).pop(cmd, None) if cmd in SENDS else None
if wa is not None:
    open(os.path.join(root, wa[0]), "w").write(wa[1]); log("geschrieben nach " + cmd + ": " + wa[0])
rm = w.get("rm_after", {}).pop(cmd, None) if cmd in SENDS else None
if rm is not None:
    try:
        os.remove(os.path.join(root, rm)); log("entfernt nach " + cmd + ": " + rm)
    except OSError:
        pass
save()
if w.get("sleep_after", {}).get(cmd) and not dry:
    import time; log("wirkt, wartet " + cmd); time.sleep(w["sleep_after"][cmd])
