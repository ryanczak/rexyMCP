import json, re, sys
from collections import Counter, defaultdict

MUT = {"patch","patch_lines","write_file"}
def norm(tool, args):
    if tool == "bash":
        c = args.get("command","")
        c = re.sub(r'\d+', 'N', c)          # strip numbers (line ranges etc)
        c = re.sub(r'\s+', ' ', c).strip()
        return "bash:" + c[:90]
    p = args.get("path") or args.get("file_path") or ""
    return f"{tool}:{p.split('/')[-1]}"

def analyze(path):
    recs = [json.loads(l) for l in open(path)]
    calls = []   # (turn, tool, norm, is_mut, is_gate, is_reread)
    ctx = {}     # turn -> context_pct
    evict = Counter()
    status = "?"
    for r in recs:
        e = r["event"]; t = r["turn"]; et = e["event_type"]
        if et == "parsed":
            tc = e["tool_call"]; tool = tc["name"]; args = tc.get("arguments") or {}
            n = norm(tool, args)
            cmd = args.get("command","") if tool=="bash" else ""
            is_gate = bool(re.search(r'cargo (test|build|clippy|fmt)', cmd))
            calls.append((t, tool, n, tool in MUT, is_gate))
        elif et == "metrics":
            ctx[t] = e.get("context_pct")
        elif et == "read_evicted":
            evict[t//50*50] += 1
        elif et == "session_end":
            status = e.get("status")
    N = len(calls)
    # last mutation
    mut_turns = [t for t,_,_,m,_ in calls if m]
    last_mut = mut_turns[-1] if mut_turns else None
    # longest no-mutation streak
    best = (0,0,0); cur_start=None; cur=0
    for i,(t,_,_,m,_) in enumerate(calls):
        if m: cur=0; cur_start=None
        else:
            if cur_start is None: cur_start=t
            cur+=1
            if cur>best[0]: best=(cur,cur_start,t)
    # repetition
    rep = Counter(n for _,_,n,_,_ in calls)
    top = rep.most_common(5)
    # gates
    gate_turns=[t for t,_,_,_,g in calls if g]
    # no-mutation streak ending at the end (tail streak)
    tail=0
    for t,_,_,m,_ in reversed(calls):
        if m: break
        tail+=1
    # streak of non-mutating calls >=40 : first occurrence turn
    first40=None; cur=0; cs=None
    for t,_,_,m,_ in calls:
        if m: cur=0; cs=None
        else:
            if cs is None: cs=t
            cur+=1
            if cur==40 and first40 is None: first40=cs
    print(f"\n=== {path.split('/')[-1]}  status={status}  calls={N} ===")
    print(f"  mutations: {len(mut_turns)}  last mutation at turn {last_mut}  tail no-mut streak: {tail}")
    print(f"  longest no-mutation streak: {best[0]} calls, turns {best[1]}..{best[2]}")
    print(f"  first time a no-mutation streak reached 40: began at turn {first40}")
    print(f"  gate runs: {len(gate_turns)}  first at {gate_turns[0] if gate_turns else None}  last at {gate_turns[-1] if gate_turns else None}")
    print(f"  distinct normalized calls: {len(rep)} / {N}  ({len(rep)/N:.0%} novel)")
    print(f"  top repeats:")
    for n,c in top: print(f"     {c:4d}x  {n}")
    ks=sorted(ctx); 
    if ks:
        pts=[k for k in (50,100,200,300,400,500,600) if k in ctx]
        print("  context_pct: " + "  ".join(f"t{k}={ctx[k]:.2f}" for k in pts))
    if evict: print(f"  read_evicted by 50-turn bucket: {dict(sorted(evict.items()))}")

for p in sys.argv[1:]: analyze(p)
