#!/usr/bin/env python3
"""Print, for each session log given, the maximal VerifierFailurePersistent
streak window: every verify in it (turn, count, code@file:line), how many
consecutive verifies had an identical diagnostic set, and the distinct files
patched inside the window. Usage: verifier-persistence-windows.py <log>..."""
import json,sys
def load(f):
    ev=[json.loads(l) for l in open(f) if l.strip()]
    return ev
for f in sys.argv[1:]:
    ev=load(f); name=f.split("session-phase-")[1].replace(".jsonl","")
    ver=[(r["turn"],r["event"]["diagnostics"]) for r in ev if r["event"].get("event_type")=="verify"]
    # find maximal streak window (positive, non-decreasing)
    best=(0,0,0); cur_start=None; prev=None; cur=0
    for i,(t,d) in enumerate(ver):
        c=len(d)
        if c>0 and (prev is None or c>=prev):
            if cur==0: cur_start=i
            cur+=1
        elif c>0: cur=1; cur_start=i
        else: cur=0
        if cur>best[0]: best=(cur,cur_start,i)
        prev=c if c>0 else None
    L,a,b=best
    if L==0: print(f"\n## {name}: no streak"); continue
    t0,t1=ver[a][0],ver[b][0]
    print(f"\n## {name}  streak={L}  verify turns {t0}..{t1}")
    sets=[]
    for t,d in ver[a:b+1]:
        s=sorted(f'{x["code"]}@{x["path"].split("/")[-1]}:{x["line"]}' for x in d)
        sets.append(tuple(s)); print(f"  v{t:>4} n={len(d)} {' '.join(s)}")
    same=sum(1 for i in range(1,len(sets)) if sets[i]==sets[i-1])
    patches=[(r["turn"],r["event"]["tool_call"]["arguments"].get("path")) for r in ev if r["event"].get("event_type")=="parsed" and r["event"]["tool_call"]["name"] in ("patch","patch_lines","write_file") and t0-1<=r["turn"]<=t1 and isinstance(r["event"]["tool_call"].get("arguments"),dict)]
    files=sorted(set(p for _,p in patches if p))
    print(f"  identical-set transitions: {same}/{len(sets)-1}   patches in window: {len(patches)}   distinct files: {len(files)}")
    for p in files: print(f"    {p}")
