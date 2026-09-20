#!/usr/bin/env python3
"""Per session log: terminal status, verify count, max detector streak
(consecutive positive, non-decreasing author-diagnostic counts — the exact
rule in executor/src/governor/hard_fail.rs::check_verifier_persistence),
and the structural-code share of all diagnostics."""
import json, sys, glob, os
STRUCT = {"E0004","E0026","E0027","E0559","E0063"}
def streak(counts):
    best=cur=0; prev=None
    for c in counts:
        if c>0 and (prev is None or c>=prev): cur+=1
        elif c>0: cur=1
        else: cur=0
        best=max(best,cur); prev=c if c>0 else None
    return best
rows=[]
for f in sorted(glob.glob(sys.argv[1]+"/session-phase-*.jsonl"), key=os.path.getmtime):
    status="?"; counts=[]; codes={}; turns=0
    for line in open(f):
        try: r=json.loads(line)
        except: continue
        e=r.get("event",{}); t=e.get("event_type")
        if t=="verify":
            d=e.get("diagnostics",[]); counts.append(len(d))
            for x in d: codes[x.get("code")]=codes.get(x.get("code"),0)+1
        elif t=="session_end": status=e.get("status","?"); turns=e.get("turns",0)
    tot=sum(codes.values()); st=sum(v for k,v in codes.items() if k in STRUCT)
    rows.append((os.path.basename(f).replace("session-phase-","").replace(".jsonl",""),status,turns,len(counts),streak(counts),tot,st))
print(f"{'log':<16}{'status':<17}{'turns':>6}{'verifies':>9}{'maxstreak':>10}{'diags':>6}{'struct':>7}")
for r in rows: print(f"{r[0]:<16}{r[1]:<17}{r[2]:>6}{r[3]:>9}{r[4]:>10}{r[5]:>6}{r[6]:>7}")
