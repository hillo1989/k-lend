#!/usr/bin/env python3
# Attrappe für ghostctl (v2/v3) – Modell steht im Feld "_m" der Zustandsdatei
import json, sys, os
args = sys.argv[1:]
log = open("calls.log", "a")
prog = "v2" if __file__.endswith("ghostctl-v2") else "v3"
log.write(prog + " " + " ".join(args) + "\n"); log.close()
def opt(name, default=None):
    return args[args.index(name)+1] if name in args else default
state = opt("--state"); dry = "--dry-run" in args; js = "--json" in args
pos = [a for i,a in enumerate(args) if not a.startswith("--") and (i==0 or not args[i-1] in ("--network","--state","--key","--vault","--ghost","--keep","--kas","--percent","--committee","--rate"))]
cmd = pos[0] if pos else ""
OWN = "aa"*32
def load():
    return json.load(open(state))
def save(d):
    if not dry:
        json.dump(d, open(state,"w"))
if cmd == "keys":
    m = load()["_m"] if os.path.exists(state) else {"ghost":0,"lp":"0","kas":100}
    alt=[{"file":"keys/alt-mainnet-owner.json","type":"key","xonly":"cc"*32,"kas":1,"ghost":0,"lpShares":"0"}] if os.environ.get("STUB_ALTKEY") else []
    print(json.dumps({"keys":alt+[{"file":"keys/mainnet-committee.json","type":"committee"},{"file":"keys/mainnet-owner.json","type":"key","xonly":OWN,"kas":m.get("kas",100),"ghost":m["ghost"],"lpShares":m.get("lp","0")}]})); sys.exit(0)
if cmd == "deploy":
    save({"network":"mainnet","vault_params":{"treasury":"x"},"_m":{"vaults":[],"ghost":0,"lp":"0","kas":100,"pool":False,"kasUsd":0.05}}); sys.exit(0)
d = load(); m = d["_m"]
if cmd == "status":
    vs=[dict(v, index=i) for i,v in enumerate(m["vaults"])]
    print(json.dumps({"vaults":vs,"pool":m.get("pool") or None,"oracle":{"kasUsd":m["kasUsd"]}}) if js else "Vault ..."); sys.exit(0)
vi = int(opt("--vault","-1"))
if cmd == "repay":
    v = m["vaults"][vi]; g = float(opt("--ghost", v["debtGhost"]))
    if v["owner"]!=OWN: print("fremder Vault", file=sys.stderr); sys.exit(1)
    g=min(g, v["debtGhost"]); 
    if g>m["ghost"]+1e-12: print("zu wenig GHOST", file=sys.stderr); sys.exit(1)
    v["debtGhost"]=round(v["debtGhost"]-g,8); m["ghost"]=round(m["ghost"]-g,8)
elif cmd == "close":
    v = m["vaults"][vi]
    if v["debtGhost"]!=0: print("Schuld offen", file=sys.stderr); sys.exit(1)
    m["vaults"].pop(vi)
elif cmd == "withdraw":
    m["vaults"][vi]["collateralKas"]=float(opt("--keep"))
elif cmd == "pool-remove":
    m["lp"]="0"; m["pool"]=False
elif cmd == "open-vault":
    m["vaults"].append({"owner":OWN,"covenantId":"new","collateralKas":float(opt("--kas")),"debtGhost":0,"stale":False})
elif cmd == "mint":
    m["vaults"][vi]["debtGhost"]+=float(opt("--ghost")); m["ghost"]+=float(opt("--ghost"))
elif cmd == "pool-open":
    m["pool"]=True
elif cmd == "balance":
    print("Guthaben ..."); sys.exit(0)
save(d)
