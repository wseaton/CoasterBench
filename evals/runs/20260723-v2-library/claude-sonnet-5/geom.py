import sys
sys.path.insert(0, '.')
import sim

def advance(cur, pname):
    tid = sim.CAT[pname]
    ted = sim.TED[tid]
    rb, re_, zb, ze, cx, cy = ted['coords']
    d = cur['d']
    ox, oy = sim.rot(cx, cy, d)
    nx, ny = cur['x']+ox, cur['y']+oy
    nz = cur['z'] - zb + ze
    nd = (d + re_ - rb) & 3
    if (re_ & 4) == 0:
        nx += sim.DELTA[nd][0]
        ny += sim.DELTA[nd][1]
    return {'x':nx,'y':ny,'z':nz,'d':nd}

def run(pieces, start):
    cur = {'x':start['x']*32,'y':start['y']*32,'z':0,'d':start['dir']&3}
    hist=[cur]
    for p in pieces:
        t = p['t'] if isinstance(p, dict) else p
        cur = advance(cur, t)
        hist.append(cur)
    return cur, hist
