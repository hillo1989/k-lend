#!/usr/bin/env python3
# Attrappe für ghostctl v4/v3 (Audit 15): Zustand in $SIM, Zeit in Stunden aus
# $SIM["now"]. Fehler lassen sich über $SIM["fail"][<befehl>] = Anzahl einspeisen;
# "status_stale" liefert einen Status mit fresh = false (gescheiterter Abgleich);
# $SIM["stale_at"] = n: genau der n-te Statusaufruf ist veraltet.
import json, os, sys
SIM = os.environ["SIM"]
s = json.load(open(SIM))
a = sys.argv[1:]
log = open(SIM + ".log", "a"); log.write(" ".join(a) + "\n"); log.close()
def save(): json.dump(s, open(SIM, "w"))
def opt(n, d=None):
    return a[a.index(n) + 1] if n in a else d
dry = "--dry-run" in a
s["calls"] = s.get("calls", 0) + 1
def failnow(tag):
    f = s.get("fail", {})
    if f.get(tag, 0) > 0:
        f[tag] -= 1; save(); print(f"FEHLER (injiziert: {tag})", file=sys.stderr); sys.exit(1)
cmds = ["keygen", "committee-keygen", "balance", "status", "deploy", "oracle-update", "open-vault", "mint", "oracle-freeze", "signers", "repay", "close", "send"]
cmd = next(x for x in a if x in cmds)
now = s["now"]
if cmd in ("keygen", "committee-keygen"):
    f = a[a.index(cmd) + 1]; open(f, "w").write("{}"); save(); sys.exit(0)
if cmd == "send" and os.environ.get("V3") == "1":
    failnow("v3send"); s["bal"][opt("--to")] = s["bal"].get(opt("--to"), 0) + float(opt("--kas")); save(); sys.exit(0)
failnow(cmd)
if cmd == "balance":
    print(json.dumps({"kas": s["bal"].get(opt("--key"), 0)})); save(); sys.exit(0)
if cmd == "status":
    f = s["fail"]
    if "status_skip" in f:
        if f["status_skip"] == 0:
            del f["status_skip"]; save(); print("Fehler: Sperre belegt (injiziert)", file=sys.stderr); sys.exit(1)
        f["status_skip"] -= 1
    s["status_calls"] = s.get("status_calls", 0) + 1
    stale = f.get("status_stale", 0) > 0 or s.get("stale_at") == s["status_calls"]
    if f.get("status_stale", 0) > 0:
        f["status_stale"] -= 1
    st = {"oracle": {"fresh": not stale, "seq": s["seq"], "frozen": s["frozen"], "freezeInMinutes": (s["last"] + 1 - now) * 60},
          "vaults": [{"debtGhost": v} for v in s["vaults"]],
          "signers": {"rotation": None if s["rot"] is None else {"readyInHours": s["rot"] - now, "valid": True}}}
    print(json.dumps(st)); save(); sys.exit(0)
if cmd == "deploy":
    if not dry:
        open(opt("--state"), "w").write('{"register":{}}'); s["bal"][opt("--key")] -= 4.1
    save(); sys.exit(0)
if cmd == "oracle-update":
    com = opt("--committee")
    if s["signer"] not in com:
        print("Fehler: nicht die nötigen Schlüssel – zu wenige der nötigen Schlüsseln"); sys.exit(1)
    if not dry:
        s["seq"] += 1; s["frozen"] = False; s["last"] = now
    save(); sys.exit(0)
if cmd == "open-vault":
    if not dry: s["vaults"].append(0.0); s["bal"][opt("--key")] -= 13
    save(); sys.exit(0)
if cmd == "mint":
    if s["frozen"]: print("Fehler: Orakel ist eingefroren"); sys.exit(1)
    if not dry: s["vaults"][int(opt("--vault"))] = float(opt("--ghost"))
    save(); sys.exit(0)
if cmd == "oracle-freeze":
    if now - s["last"] < 1: print("zu früh"); sys.exit(1)
    if not dry: s["frozen"] = True
    save(); sys.exit(0)
if cmd == "signers":
    sub = a[a.index("signers") + 1]
    if sub == "propose":
        if s["signer"] not in opt("--committee"): print("nötigen Schlüsseln"); sys.exit(1)
        if not dry: s["rot"] = now + 1.0; s["tickets"] = s.get("tickets", 0) + 1
        if s["fail"].get("propose_sent_err", 0) > 0:
            s["fail"]["propose_sent_err"] -= 1; save(); print("Tx nicht bestätigt (injiziert)"); sys.exit(1)
    elif sub == "activate":
        failnow("activate")
        if s["rot"] is None: print("Fehler: keine Ankündigung offen"); sys.exit(1)
        if now < s["rot"]: print("Fehler: Node lehnt ab: one of the transaction sequence locks conditions was not met"); sys.exit(1)
        if not dry: s["signer"] = "signer2"; s["rot"] = None
    else:
        print("signers show")
    save(); sys.exit(0)
if cmd == "repay":
    if not s["vaults"] or s["vaults"][int(opt("--vault"))] == 0: print("Fehler: keine Schuld / kein Vault"); sys.exit(1)
    if not dry: s["vaults"][int(opt("--vault"))] = 0.0
    save(); sys.exit(0)
if cmd == "close":
    if not s["vaults"]: print("Fehler: kein Vault"); sys.exit(1)
    if not dry: s["vaults"].pop(); s["bal"][opt("--key")] += 10
    save(); sys.exit(0)
if cmd == "send":
    if not dry: s["bal"][opt("--key")] -= float(opt("--kas"))
    save(); sys.exit(0)
