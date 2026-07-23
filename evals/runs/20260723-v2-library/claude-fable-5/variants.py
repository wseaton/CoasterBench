import geom, layout7, json, pathlib
from build import hill, turn180, turn90
L = layout7

def body(pass_c, pass_d_extra_slalom):
    p = []
    p += ["begin_station","middle_station","middle_station","end_station"]
    p += L.spiral_lift(); p += L.first_drop(); p += ["flat"]*L.A1
    p += L.climb(L.SHELF); p += turn180("l", banked=True, mid=L.MID1)
    p += L.dive(L.SHELF); p += hill(3,3); p += hill(2,2); p += ["flat"]*L.B1
    p += L.climb(L.SHELF); p += turn180("r", banked=True, mid=L.MID2)
    p += L.dive(L.SHELF); p += pass_c(); p += ["flat"]*L.C1
    p += L.climb(L.SHELF); p += turn90("r", banked=True)
    p += L.dive(L.SHELF); p += ["flat"]*L.CORR; p += turn90("r", banked=True)
    p += hill(2,2)
    p += L.slalom("r") if pass_d_extra_slalom else ["flat"]*14
    p += ["flat"]*L.E1
    p += turn180("r", banked=True, mid=L.MID3)
    return p

def v1(): return body(lambda: hill(2,2)+hill(2,2), True)
def v2(): return body(lambda: hill(2,2)+hill(2,2), False)

def solve(fn, name):
    for A1 in range(0,16):
        for CORR in range(18,34):
            L.A1, L.CORR = A1, CORR
            p = fn(); r = geom.simulate(p,(58,62,0),0)
            if (r['x'],r['y'],r['z'],r['dir'],r['roll'],r['pitch'])!=(58,62,0,0,'none','none'): continue
            xs=[f[0] for f in r['foot']]; ys=[f[1] for f in r['foot']]
            if geom.collisions(r['foot']) or max(xs)>71 or min(xs)<26: continue
            if any(54<=f[1]<=76 and f[0]>=67 for f in r['foot']): continue
            print(name,'A1',A1,'CORR',CORR,'n',len(p),'bbox',(min(xs),max(xs),min(ys),max(ys)))
            pathlib.Path(f'/tmp/{name}.json').write_text(json.dumps({"ride_type":52,"start":{"x":58,"y":62,"dir":0},"pieces":p}))
            return True
    print(name,'no solution')

L.B1, L.C1, L.E1, L.MID3 = 0, 8, 4, 12
solve(v1,'v1'); solve(v2,'v2')
