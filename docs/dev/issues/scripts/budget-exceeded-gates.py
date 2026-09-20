import json, re, sys
MUT = {"patch","patch_lines","write_file"}
def run(path):
    recs=[json.loads(l) for l in open(path)]
    status="?"; seq=[]  # per turn: (turn, kind, extra)
    pending=None
    for r in recs:
        e=r["event"]; t=r["turn"]; et=e["event_type"]
        if et=="parsed":
            tc=e["tool_call"]; tool=tc["name"]; a=tc.get("arguments") or {}
            cmd=a.get("command","") if tool=="bash" else ""
            gate=bool(re.search(r'cargo (test|build|clippy)',cmd))
            pending=(t,tool,gate,cmd)
        elif et=="tool_result" and pending:
            t0,tool,gate,cmd=pending; pv=e.get("output_preview","") or ""
            if gate:
                red=bool(re.search(r'FAILED|^error|error\[|could not compile|warning: unused',pv,re.M))
                green=bool(re.search(r'test result: ok|Finished',pv)) and not red
                seq.append((t0,"gate","red" if red else ("green" if green else "?"),pv[:100].replace("\n"," ")))
            elif tool in MUT: seq.append((t0,"mut","",""))
            else: seq.append((t0,"read","",""))
            pending=None
        elif et=="session_end": status=e.get("status")
    # detector 1: mutations since last gate
    msg=0; msg_max=(0,None); cross={10:None,20:None,30:None}; last_gate=None
    for t,k,o,_ in seq:
        if k=="gate": msg=0; last_gate=t
        elif k=="mut":
            msg+=1
            if msg>msg_max[0]: msg_max=(msg,t)
            for th in cross:
                if msg==th and cross[th] is None: cross[th]=t
    # detector 2: consecutive red gates
    red=0; red_max=(0,None,None); rs=None; last_green=None; unk=0
    gates=[(t,o,pv) for t,k,o,pv in seq if k=="gate"]
    for t,o,pv in gates:
        if o=="green": red=0; rs=None; last_green=t
        elif o=="red":
            if rs is None: rs=t
            red+=1
            if red>red_max[0]: red_max=(red,rs,t)
        else: unk+=1
    print(f"\n=== {path.split('/')[-1]} status={status} ===")
    print(f"  D1 mutations-since-last-gate: max {msg_max[0]} (reached at turn {msg_max[1]}); crossed 10@{cross[10]} 20@{cross[20]} 30@{cross[30]}")
    print(f"  D2 gates: {len(gates)} total, {sum(1 for _,o,_ in gates if o=='red')} red, {sum(1 for _,o,_ in gates if o=='green')} green, {unk} unclassified")
    print(f"     longest consecutive-red run: {red_max[0]} (turns {red_max[1]}..{red_max[2]});  last GREEN gate at turn {last_green}")
    if gates:
        print(f"     sample gate previews (first red, first green):")
        fr=next((g for g in gates if g[1]=='red'),None); fg=next((g for g in gates if g[1]=='green'),None)
        if fr: print(f"       red   t{fr[0]}: {fr[2]}")
        if fg: print(f"       green t{fg[0]}: {fg[2]}")
for p in sys.argv[1:]: run(p)
