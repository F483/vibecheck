import sys, xml.etree.ElementTree as ET
a, b = ET.parse(sys.argv[1]).getroot(), ET.parse(sys.argv[2]).getroot()
def tracks(r):
    return {t.get("TrackID"): t for t in r.find("COLLECTION")}
ta, tb = tracks(a), tracks(b)
assert ta.keys() == tb.keys(), "track sets differ"
bad = 0
for k in ta:
    x, y = ta[k], tb[k]
    if x.attrib != y.attrib or [dict(c.attrib) for c in x] != [dict(c.attrib) for c in y] or [c.tag for c in x] != [c.tag for c in y]:
        bad += 1
        if bad <= 3:
            print("DIFF", k, x.attrib.get("Location")[-60:]); print(" py", x.attrib == y.attrib, [c.tag for c in x][:6]); print(" rs", [c.tag for c in y][:6])
print("tracks", len(ta), "differing", bad)
def nodes(n, d=0):
    out = [(d, n.tag, dict(n.attrib))]
    for c in n:
        out += nodes(c, d + 1)
    return out
pa, pb = nodes(a.find("PLAYLISTS")), nodes(b.find("PLAYLISTS"))
print("playlist nodes", len(pa), len(pb), "identical" if pa == pb else "DIFFER")
if pa != pb:
    for i, (x, y) in enumerate(zip(pa, pb)):
        if x != y:
            print(i, x, y); break
