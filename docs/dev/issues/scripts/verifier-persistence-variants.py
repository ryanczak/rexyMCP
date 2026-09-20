#!/usr/bin/env python3
"""Replay every session log against the VerifierFailurePersistent rule
(executor/src/governor/hard_fail.rs::check_verifier_persistence) and four
candidate variants. Prints one row per log whose streak reaches 5 under any
rule, then fire counts at threshold 6 and the max streak among `complete` runs
(the false-positive floor).

  base    : the shipped rule — consecutive verifies with >0 author diagnostics
            and a non-decreasing count.
  struct  : a verify whose diagnostics are ALL structural codes (E0004 non-
            exhaustive match, E0026/E0027/E0559/E0063 struct-literal/pattern
            field mismatches) does not extend the streak.
  refile  : the streak extends only when the write that triggered the verify
            hit a FILE already written earlier in the same streak.
  retarget: as refile, but keyed on (file, first 40 chars of the patch's
            old_str) — the same region, not just the same file.
  sameset : the streak extends only when the diagnostic set (code@file:line)
            is identical to the previous verify's.
Usage: verifier-persistence-variants.py <sessions-dir>"""
import json,sys,glob,os
STRUCT={"E0004","E0026","E0027","E0559","E0063"}
def analyse(f):
    ev=[json.loads(l) for l in open(f) if l.strip()]
    status="?"
    for r in ev:
        if r["event"].get("event_type")=="session_end": status=r["event"].get("status","?")
    writes=[]
    for r in ev:
        e=r["event"]
        if e.get("event_type")=="parsed" and e["tool_call"]["name"] in ("patch","patch_lines","write_file"):
            a=e["tool_call"].get("arguments") or {}
            writes.append((r["turn"],a.get("path"),(a.get("path"),(a.get("old_str") or "")[:40])))
    ver=[(r["turn"],r["event"]["diagnostics"]) for r in ev if r["event"].get("event_type")=="verify"]
    def trig(t):
        c=[w for w in writes if w[0]<=t]; return c[-1] if c else (None,None,None)
    mx={k:0 for k in("base","struct","refile","retarget","sameset")}
    cur={k:0 for k in mx}; prev=None; prevset=None; files=set(); targets=set()
    for t,d in ver:
        n=len(d); nd=(n>0 and (prev is None or n>=prev))
        sset=tuple(sorted(f'{x.get("code")}@{x.get("path")}:{x.get("line")}' for x in d))
        allstruct=n>0 and all(x.get("code") in STRUCT for x in d)
        _,fpath,ftarget=trig(t)
        cur["base"]= cur["base"]+1 if nd else (1 if n>0 else 0)
        cur["struct"]= 0 if (n==0 or allstruct) else (cur["struct"]+1 if nd else 1)
        if n==0: cur["refile"]=0; files=set()
        elif nd and fpath in files: cur["refile"]+=1
        else: cur["refile"]=1; files={fpath}
        if nd: files.add(fpath)
        if n==0: cur["retarget"]=0; targets=set()
        elif nd and ftarget in targets: cur["retarget"]+=1
        else: cur["retarget"]=1; targets={ftarget}
        if nd: targets.add(ftarget)
        cur["sameset"]= 0 if n==0 else (cur["sameset"]+1 if (nd and sset==prevset) else 1)
        for k in mx: mx[k]=max(mx[k],cur[k])
        prev=n if n>0 else None; prevset=sset if n>0 else None
    return status,mx
rows=[]
for f in sorted(glob.glob(os.path.join(sys.argv[1],"session-phase-*.jsonl")),key=os.path.getmtime):
    st,mx=analyse(f); rows.append((os.path.basename(f)[14:-6],st,mx))
keys=["base","struct","refile","retarget","sameset"]
print(f"{'log':<16}{'status':<16}"+"".join(f"{k:>9}" for k in keys))
for n,st,mx in rows:
    if any(mx[k]>=5 for k in keys): print(f"{n:<16}{st:<16}"+"".join(f"{mx[k]:>9}" for k in keys))
print("\nlogs:",len(rows))
print("fires at threshold 6:", {k:sum(1 for r in rows if r[2][k]>=6) for k in keys})
print("max streak among complete runs:", {k:max((r[2][k] for r in rows if r[1]=="complete"),default=0) for k in keys})
print("runs with base streak >= 10:", sum(1 for r in rows if r[2]["base"]>=10))
